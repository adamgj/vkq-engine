//! Phase 4 M4: mixer differential -- `quake_snd::mix::paint_channels` vs the
//! c_ref-compiled `snd_mix.c` over scripted channel schedules and fixed
//! decoder inputs (ADR-014: never live codec output), across the DMA format
//! matrix, filter qualities, underwater ramps, raw-music injection, and
//! pause-looping. Compares the per-block paintbuffer+DMA hash chain (via the
//! Harness_SndPaint seam both sides implement), the final DMA bytes, the
//! channel states, and paintedtime.

use core::ffi::c_int;

use quake_ctest as _;

use quake_snd::mix::{self, CacheView, MixerState, PaintParams, SfxSource};
use quake_types::sound::{Channel, SamplePair, Sfx, MAX_RAW_SAMPLES};

extern "C" {
    #[link_name = "c_ref_snd_channels"]
    static mut snd_channels: [Channel; 1024];
    #[link_name = "c_ref_total_channels"]
    static mut total_channels: c_int;
    #[link_name = "c_ref_paintedtime"]
    static mut paintedtime: c_int;
    #[link_name = "c_ref_s_rawend"]
    static mut s_rawend: c_int;
    #[link_name = "c_ref_s_rawsamples"]
    static mut s_rawsamples: [SamplePair; MAX_RAW_SAMPLES];

    fn c_ref_S_PaintChannels(endtime: c_int);
    fn c_ref_SND_InitScaletable();
    fn c_ref_S_SetUnderwaterIntensity(target: f32);

    fn ctest_snd_setup_dma(
        speed: c_int,
        samplebits: c_int,
        channels: c_int,
        signed8: c_int,
        samples: c_int,
        buffer: *mut u8,
    );
    fn ctest_snd_set_pause_state(
        cl_paused: c_int,
        sv_active: c_int,
        maxclients: c_int,
        keydest: c_int,
        frametime: f64,
    );
    fn ctest_snd_set_cvars(
        sfxvol: f32,
        sndspeed: f32,
        filterquality: f32,
        waterfx: f32,
        pauselooping: f32,
    );
    fn ctest_snd_block_reset();
    fn ctest_snd_block_get(count: *mut c_int) -> u64;
}

const FNV_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

fn fnv(mut h: u64, data: &[u8]) -> u64 {
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

// ---------------------------------------------------------------------------
// fixtures

/// A synthesized sfx cache: header meta plus PCM bytes, plus a boxed sfx_t
/// giving both sides a stable identity pointer.
struct SfxFixture {
    sfx: Box<Sfx>,
    /// full C-layout cache blob (header + data) the C mixer reads
    blob: Vec<u8>,
    length: i32,
    loopstart: i32,
    width: i32,
}

fn make_sfx(name: &str, length: i32, loopstart: i32, width: i32, seed: u32) -> SfxFixture {
    let data_len = (length * width) as usize;
    let mut blob = vec![0u8; 20 + data_len.max(4)];
    // sfxcache_t header: length, loopstart, speed, width, stereo
    blob[0..4].copy_from_slice(&length.to_le_bytes());
    blob[4..8].copy_from_slice(&loopstart.to_le_bytes());
    blob[8..12].copy_from_slice(&44100i32.to_le_bytes());
    blob[12..16].copy_from_slice(&width.to_le_bytes());
    blob[16..20].copy_from_slice(&0i32.to_le_bytes());
    let mut state = seed;
    for i in 0..data_len {
        state = state.wrapping_mul(1103515245).wrapping_add(12345);
        blob[20 + i] = (state >> 16) as u8;
    }

    let mut sfx = Box::new(Sfx {
        name: [0; 64],
        cache: core::ptr::null_mut(),
    });
    for (i, b) in name.bytes().take(63).enumerate() {
        sfx.name[i] = b as i8;
    }
    sfx.cache = blob.as_mut_ptr().cast();

    SfxFixture {
        sfx,
        blob,
        length,
        loopstart,
        width,
    }
}

struct TestLoader<'a> {
    sfxs: &'a [SfxFixture],
}

impl SfxSource for TestLoader<'_> {
    fn load(&mut self, sfxp: *mut Sfx) -> Option<CacheView<'_>> {
        self.sfxs
            .iter()
            .find(|f| core::ptr::addr_of!(*f.sfx) == sfxp.cast_const())
            .map(|f| CacheView {
                length: f.length,
                loopstart: f.loopstart,
                width: f.width,
                data: &f.blob[20..],
            })
    }
}

// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct ChanSetup {
    sfx_idx: usize,
    leftvol: i32,
    rightvol: i32,
    /// end relative to the scenario's starting paintedtime
    end: i32,
    pos: i32,
}

struct Scenario {
    tag: &'static str,
    shm: (i32, i32, i32, i32, i32), // speed, bits, channels, signed8, samples
    start_paintedtime: i32,
    sfxvolume: f32,
    sndspeed: f32,
    filterquality: f32,
    waterfx: f32,
    pauselooping: f32,
    pause_state: (i32, i32, i32, i32, f64),
    channels: Vec<ChanSetup>,
    /// (offset into raw ring, samples, value seed) for injected music
    raw: Option<(i32, u32)>,
    /// per paint call: (underwater target, endtime delta from current painted)
    paints: Vec<(f32, i32)>,
}

static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn run_scenario(sc: &Scenario, sfxs: &[SfxFixture]) {
    // the c_ref mixer's file statics (paintbuffer, filters, underwater) are
    // process-global: scenarios must not interleave
    let _guard = SERIAL.lock().unwrap();
    // drain the C underwater state left by an earlier scenario (the ramp
    // decays at 4/s; one huge frametime forces intensity back to 0, which is
    // the fresh Rust MixerState's value; alpha then equals exp(0) = 1 too)
    // SAFETY: single-threaded under the guard, stub-owned state
    unsafe {
        ctest_snd_set_pause_state(0, 0, 1, 0, 1.0e9);
        c_ref_S_SetUnderwaterIntensity(0.0);
    }
    let (speed, bits, chans, signed8, samples) = sc.shm;
    let bufbytes = (samples * bits / 8) as usize;

    // ---- C side ----
    let mut c_dma = vec![0u8; bufbytes];
    let mut init_channels = [Channel {
        sfx: core::ptr::null_mut(),
        leftvol: 0,
        rightvol: 0,
        end: 0,
        pos: 0,
        looping: 0,
        entnum: 0,
        entchannel: 0,
        origin: [0.0; 3],
        dist_mult: 0.0,
        master_vol: 0,
    }; 64];
    for (i, cs) in sc.channels.iter().enumerate() {
        let ch = &mut init_channels[i];
        ch.sfx = core::ptr::addr_of!(*sfxs[cs.sfx_idx].sfx).cast_mut();
        ch.leftvol = cs.leftvol;
        ch.rightvol = cs.rightvol;
        ch.end = sc.start_paintedtime + cs.end;
        ch.pos = cs.pos;
    }

    let mut raw_ring = [SamplePair::default(); MAX_RAW_SAMPLES];
    let mut rawend = 0;
    if let Some((extent, seed)) = sc.raw {
        rawend = sc.start_paintedtime + extent;
        let mut state = seed;
        for pair in raw_ring.iter_mut() {
            state = state.wrapping_mul(214013).wrapping_add(2531011);
            pair.left = ((state >> 8) & 0xffffff) as i32 - 0x800000;
            state = state.wrapping_mul(214013).wrapping_add(2531011);
            pair.right = ((state >> 8) & 0xffffff) as i32 - 0x800000;
        }
    }

    // SAFETY: single-threaded test writing stub-owned globals
    unsafe {
        ctest_snd_setup_dma(speed, bits, chans, signed8, samples, c_dma.as_mut_ptr());
        ctest_snd_set_cvars(
            sc.sfxvolume,
            sc.sndspeed,
            sc.filterquality,
            sc.waterfx,
            sc.pauselooping,
        );
        let (p, a, m, k, f) = sc.pause_state;
        ctest_snd_set_pause_state(p, a, m, k, f);
        paintedtime = sc.start_paintedtime;
        s_rawend = rawend;
        s_rawsamples = raw_ring;
        total_channels = sc.channels.len() as c_int;
        for (i, ch) in init_channels.iter().enumerate() {
            snd_channels[i] = *ch;
        }
        ctest_snd_block_reset();
        c_ref_SND_InitScaletable();
        let mut endtime = sc.start_paintedtime;
        for &(uw, delta) in &sc.paints {
            c_ref_S_SetUnderwaterIntensity(uw);
            endtime += delta;
            c_ref_S_PaintChannels(endtime);
        }
    }
    let mut c_blocks = 0;
    // SAFETY: plain accessor
    let c_hash = unsafe { ctest_snd_block_get(&mut c_blocks) };
    // SAFETY: reading back stub-owned state
    let (c_painted, c_channels): (i32, Vec<Channel>) =
        unsafe { (paintedtime, snd_channels[..sc.channels.len()].to_vec()) };

    // ---- Rust side ----
    let mut r_dma = vec![0u8; bufbytes];
    let mut r_channels = init_channels[..sc.channels.len()].to_vec();
    let mut st = MixerState::default();
    mix::init_scaletable(&mut st, sc.sfxvolume);
    let mut r_painted = sc.start_paintedtime;
    let mut r_hash = FNV_BASIS;
    let mut r_blocks = 0;

    let mut loader = TestLoader { sfxs };

    let mut endtime = sc.start_paintedtime;
    for &(uw, delta) in &sc.paints {
        mix::set_underwater_intensity(&mut st.underwater, uw, sc.waterfx, sc.pause_state.4);
        endtime += delta;
        let (p, a, m, k, _) = sc.pause_state;
        let pause_loops = sc.pauselooping != 0.0 && (p != 0 || (a != 0 && m == 1 && k != 0));
        let mut params = PaintParams {
            endtime,
            pause_loops,
            sfxvolume_value: sc.sfxvolume,
            sndspeed_value: sc.sndspeed,
            filterquality_value: sc.filterquality,
            shm_speed: speed,
            shm_samples: samples,
            shm_samplebits: bits,
            shm_channels: chans,
            shm_signed8: signed8,
            dma_buffer: &mut r_dma,
            s_rawend: rawend,
            raw_samples: &raw_ring,
        };
        mix::paint_channels(
            &mut st,
            &mut r_painted,
            &mut r_channels,
            &mut loader,
            &mut params,
            |painted, end, paint, dma| {
                let mut h = r_hash;
                h = fnv(h, &painted.to_le_bytes());
                h = fnv(h, &end.to_le_bytes());
                for pb in paint {
                    h = fnv(h, &pb.left.to_le_bytes());
                    h = fnv(h, &pb.right.to_le_bytes());
                }
                h = fnv(h, dma);
                r_hash = h;
                r_blocks += 1;
            },
        );
    }

    // ---- compare ----
    assert_eq!(c_painted, r_painted, "{}: paintedtime", sc.tag);
    assert_eq!(c_blocks, r_blocks, "{}: paint block count", sc.tag);
    assert_eq!(
        c_hash, r_hash,
        "{}: per-block paintbuffer+DMA hash chain",
        sc.tag
    );
    assert_eq!(c_dma, r_dma, "{}: final DMA buffer", sc.tag);
    for (i, (c, r)) in c_channels.iter().zip(r_channels.iter()).enumerate() {
        assert_eq!(
            c.sfx.is_null(),
            r.sfx.is_null(),
            "{}: ch{} sfx null",
            sc.tag,
            i
        );
        assert_eq!(c.pos, r.pos, "{}: ch{} pos", sc.tag, i);
        assert_eq!(c.end, r.end, "{}: ch{} end", sc.tag, i);
        assert_eq!(c.leftvol, r.leftvol, "{}: ch{} leftvol", sc.tag, i);
        assert_eq!(c.rightvol, r.rightvol, "{}: ch{} rightvol", sc.tag, i);
    }
}

fn base_channels() -> Vec<ChanSetup> {
    // end == length - pos, mirroring S_StartSound's `end = paintedtime +
    // sc->length` (schedules that overrun the cache are the dangling-channel
    // UB the engine never produces from a fresh start)
    vec![
        ChanSetup {
            sfx_idx: 0,
            leftvol: 200,
            rightvol: 100,
            end: 6000,
            pos: 0,
        },
        ChanSetup {
            sfx_idx: 1,
            leftvol: 120,
            rightvol: 250,
            end: 2400,
            pos: 100,
        },
        ChanSetup {
            sfx_idx: 2,
            leftvol: 400,
            rightvol: 90,
            end: 8000,
            pos: 0,
        }, // >255 clamp
        ChanSetup {
            sfx_idx: 3,
            leftvol: 255,
            rightvol: 255,
            end: 1000,
            pos: 4000,
        }, // 16-bit loud
        ChanSetup {
            sfx_idx: 0,
            leftvol: 0,
            rightvol: 0,
            end: 6000,
            pos: 0,
        }, // muted
    ]
}

fn sfx_set() -> Vec<SfxFixture> {
    vec![
        make_sfx("loop8", 6000, 1500, 1, 0xa5a5a5a5),
        make_sfx("oneshot8", 2500, -1, 1, 0x1234567),
        make_sfx("loop16", 8000, 0, 2, 0xdeadbeef),
        make_sfx("oneshot16", 5000, -1, 2, 0xcafef00d),
    ]
}

#[test]
fn mixer_format_matrix() {
    let sfxs = sfx_set();
    let formats = [
        ("st16", (44100, 16, 2, 0, 16384)),
        ("mono16", (44100, 16, 1, 0, 8192)),
        ("u8st", (44100, 8, 2, 0, 16384)),
        ("u8mono", (22050, 8, 1, 0, 8192)),
        ("s8mono", (11025, 8, 1, 1, 4096)),
    ];
    for (name, shm) in formats {
        let sc = Scenario {
            tag: name,
            shm,
            start_paintedtime: 100000,
            sfxvolume: 0.7,
            sndspeed: 44100.0,
            filterquality: 1.0,
            waterfx: 1.0,
            pauselooping: 0.0,
            pause_state: (0, 0, 1, 0, 1.0 / 72.0),
            channels: base_channels(),
            raw: None,
            paints: vec![(0.0, 613), (0.0, 612), (0.0, 4000), (0.0, 2500)],
        };
        run_scenario(&sc, &sfxs);
    }
}

#[test]
fn mixer_lowpass_filter_qualities() {
    let sfxs = sfx_set();
    for q in 1..=5 {
        let sc = Scenario {
            tag: "lowpass",
            shm: (44100, 16, 2, 0, 16384),
            start_paintedtime: 5000,
            sfxvolume: 1.0,
            sndspeed: 11025.0, // enables the lowpass path
            filterquality: q as f32,
            waterfx: 1.0,
            pauselooping: 0.0,
            pause_state: (0, 0, 1, 0, 1.0 / 72.0),
            channels: base_channels(),
            raw: None,
            paints: vec![(0.0, 613), (0.0, 612), (0.0, 3000), (0.0, 2048), (0.0, 100)],
        };
        run_scenario(&sc, &sfxs);
    }
}

#[test]
fn mixer_underwater_ramp() {
    let sfxs = sfx_set();
    let sc = Scenario {
        tag: "underwater",
        shm: (44100, 16, 2, 0, 16384),
        start_paintedtime: 200000,
        sfxvolume: 0.7,
        sndspeed: 44100.0,
        filterquality: 1.0,
        waterfx: 1.3,
        pauselooping: 0.0,
        pause_state: (0, 0, 1, 0, 1.0 / 72.0),
        channels: base_channels(),
        raw: None,
        paints: vec![
            (0.0, 613),
            (1.0, 612),
            (1.0, 613),
            (1.0, 2000),
            (0.5, 612),
            (0.0, 613),
            (0.0, 612),
        ],
    };
    run_scenario(&sc, &sfxs);
}

#[test]
fn mixer_raw_music_and_pause() {
    let sfxs = sfx_set();
    // raw music mixed in
    let sc = Scenario {
        tag: "rawmusic",
        shm: (44100, 16, 2, 0, 16384),
        start_paintedtime: 50000,
        sfxvolume: 0.7,
        sndspeed: 44100.0,
        filterquality: 1.0,
        waterfx: 1.0,
        pauselooping: 0.0,
        pause_state: (0, 0, 1, 0, 1.0 / 72.0),
        channels: base_channels(),
        raw: Some((4000, 0xbeef)),
        paints: vec![(0.0, 613), (0.0, 612), (0.0, 3000), (0.0, 2000)],
    };
    run_scenario(&sc, &sfxs);

    // pause_loops: looping channels skipped while paused
    let sc = Scenario {
        tag: "pauseloops",
        shm: (44100, 16, 2, 0, 16384),
        start_paintedtime: 4096,
        sfxvolume: 0.7,
        sndspeed: 44100.0,
        filterquality: 1.0,
        waterfx: 1.0,
        pauselooping: 1.0,
        pause_state: (1, 0, 1, 0, 1.0 / 72.0),
        channels: base_channels(),
        raw: None,
        paints: vec![(0.0, 613), (0.0, 4000)],
    };
    run_scenario(&sc, &sfxs);

    // single-player menu pause (sv.active, maxclients 1, key_dest != game)
    let sc = Scenario {
        tag: "menupause",
        shm: (44100, 16, 2, 0, 16384),
        start_paintedtime: 4096,
        sfxvolume: 0.7,
        sndspeed: 44100.0,
        filterquality: 1.0,
        waterfx: 1.0,
        pauselooping: 1.0,
        pause_state: (0, 1, 1, 3, 1.0 / 72.0),
        channels: base_channels(),
        raw: None,
        paints: vec![(0.0, 613), (0.0, 4000)],
    };
    run_scenario(&sc, &sfxs);
}

#[test]
fn mixer_volume_extremes() {
    let sfxs = sfx_set();
    for vol in [0.0f32, 0.15, 1.0, 4.0] {
        let sc = Scenario {
            tag: "volumes",
            shm: (44100, 16, 2, 0, 16384),
            start_paintedtime: 16000,
            sfxvolume: vol,
            sndspeed: 44100.0,
            filterquality: 1.0,
            waterfx: 1.0,
            pauselooping: 0.0,
            pause_state: (0, 0, 1, 0, 1.0 / 72.0),
            channels: base_channels(),
            raw: Some((2000, 77)),
            paints: vec![(0.0, 613), (0.0, 3000)],
        };
        run_scenario(&sc, &sfxs);
    }
}
