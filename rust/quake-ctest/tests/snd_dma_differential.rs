//! Phase 4 M6a: differential sweeps for the pure snd_dma.c logic --
//! `quake_snd::dma::{pick_channel, spatialize, raw_samples}` vs the
//! c_ref-compiled originals over randomized channel states, spatial grids,
//! and the raw-music format matrix.

use core::ffi::c_int;

use quake_ctest as _;
use quake_snd::dma;
use quake_types::sound::{Channel, SamplePair, MAX_RAW_SAMPLES};

extern "C" {
    #[link_name = "c_ref_snd_channels"]
    static mut snd_channels: [Channel; 1024];
    #[link_name = "c_ref_paintedtime"]
    static mut paintedtime: c_int;
    #[link_name = "c_ref_s_rawend"]
    static mut s_rawend: c_int;
    #[link_name = "c_ref_s_rawsamples"]
    static mut s_rawsamples: [SamplePair; MAX_RAW_SAMPLES];

    fn c_ref_SND_PickChannel(entnum: c_int, entchannel: c_int) -> *mut Channel;
    fn c_ref_SND_Spatialize(ch: *mut Channel);
    fn c_ref_S_RawSamples(samples: c_int, rate: c_int, width: c_int, channels: c_int, data: *mut u8, volume: f32);

    fn ctest_snd_setup_dma(speed: c_int, samplebits: c_int, channels: c_int, signed8: c_int, samples: c_int, buffer: *mut u8);
    fn ctest_snd_set_listener(origin: *const f32, right: *const f32);
    fn ctest_set_cl_viewentity(viewentity: c_int);
}

static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lcg(state: &mut u32) -> u32 {
    *state = state.wrapping_mul(1664525).wrapping_add(1013904223);
    *state
}

fn blank_channel() -> Channel {
    Channel {
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
    }
}

#[test]
fn pick_channel_sweep() {
    let _g = SERIAL.lock().unwrap();
    let mut seed = 0x12345u32;
    let dummy_sfx = Box::new([0u8; 8]); // any non-null identity
    for round in 0..500 {
        let painted = (round * 977) % 100000;
        let viewentity = (lcg(&mut seed) % 8) as i32;
        let mut channels = vec![blank_channel(); 1024];
        for ch in channels.iter_mut().take(200) {
            let r = lcg(&mut seed);
            ch.entnum = (r % 8) as i32;
            ch.entchannel = ((r >> 4) % 5) as i32 - 1;
            ch.end = painted + ((r >> 8) % 5000) as i32 - 1000;
            ch.sfx = if r & 0x8000 != 0 {
                dummy_sfx.as_ptr() as *mut _
            } else {
                core::ptr::null_mut()
            };
        }
        let entnum = (lcg(&mut seed) % 8) as i32;
        let entchannel = (lcg(&mut seed) % 5) as i32 - 1;

        // C side
        // SAFETY: serialized test writing c_ref globals
        let c_idx = unsafe {
            paintedtime = painted;
            ctest_set_cl_viewentity(viewentity);
            for (i, ch) in channels.iter().enumerate() {
                snd_channels[i] = *ch;
            }
            let p = c_ref_SND_PickChannel(entnum, entchannel);
            if p.is_null() {
                None
            } else {
                Some(p.offset_from(core::ptr::addr_of!(snd_channels) as *const Channel) as usize)
            }
        };
        let c_state: Vec<(bool, i32)> = unsafe {
            snd_channels[..200].iter().map(|c| (c.sfx.is_null(), c.end)).collect()
        };

        // Rust side
        let mut r_channels = channels.clone();
        let r_idx = dma::pick_channel(&mut r_channels, painted, viewentity, entnum, entchannel);
        let r_state: Vec<(bool, i32)> = r_channels[..200].iter().map(|c| (c.sfx.is_null(), c.end)).collect();

        assert_eq!(c_idx, r_idx, "round {round}: picked channel");
        assert_eq!(c_state, r_state, "round {round}: channel side effects");
    }
}

#[test]
fn spatialize_sweep() {
    let _g = SERIAL.lock().unwrap();
    let mut buf = vec![0u8; 64];
    let mut seed = 0xbeefu32;
    for &shm_channels in &[1i32, 2] {
        // SAFETY: serialized; sets c_ref shm format
        unsafe { ctest_snd_setup_dma(44100, 16, shm_channels, 0, 16384, buf.as_mut_ptr()) };
        for round in 0..2000 {
            let f = |s: &mut u32| (lcg(s) as i32 % 4000) as f32 / 3.0;
            let origin = [f(&mut seed), f(&mut seed), f(&mut seed)];
            let listener = [f(&mut seed), f(&mut seed), f(&mut seed)];
            let mut right = [f(&mut seed) / 100.0, f(&mut seed) / 100.0, f(&mut seed) / 100.0];
            if round % 7 == 0 {
                right = [0.0, 1.0, 0.0];
            }
            let viewentity = 1i32;
            let mut ch = blank_channel();
            ch.entnum = if round % 11 == 0 { 1 } else { 2 };
            ch.origin = if round % 13 == 0 { listener } else { origin };
            ch.dist_mult = ((lcg(&mut seed) % 4) as f32 + 0.1) / dma::SOUND_NOMINAL_CLIP_DIST as f32;
            ch.master_vol = (lcg(&mut seed) % 300) as i32;

            // SAFETY: serialized; c_ref reads listener + cl.viewentity + shm
            let c_ch = unsafe {
                ctest_snd_set_listener(listener.as_ptr(), right.as_ptr());
                ctest_set_cl_viewentity(viewentity);
                let mut c = ch;
                c_ref_SND_Spatialize(&mut c);
                c
            };
            let mut r_ch = ch;
            dma::spatialize(&mut r_ch, &listener, &right, viewentity, shm_channels);

            assert_eq!(
                (c_ch.leftvol, c_ch.rightvol),
                (r_ch.leftvol, r_ch.rightvol),
                "round {round} shm_channels {shm_channels}: vols (origin {:?})",
                ch.origin
            );
        }
    }
}

#[test]
fn raw_samples_sweep() {
    let _g = SERIAL.lock().unwrap();
    let mut buf = vec![0u8; 64];
    let mut seed = 0xfeedu32;
    for &(channels, width) in &[(2i32, 2i32), (1, 2), (2, 1), (1, 1)] {
        for &rate in &[11025i32, 22050, 44100, 48000, 8000] {
            // SAFETY: serialized; c_ref shm speed feeds the scale
            unsafe { ctest_snd_setup_dma(44100, 16, 2, 0, 16384, buf.as_mut_ptr()) };
            let painted = 30000;
            let mut data = vec![0u8; 4096 * channels as usize * width as usize];
            for b in data.iter_mut() {
                *b = (lcg(&mut seed) >> 16) as u8;
            }
            let samples = 4096 / 4; // keep runs bounded
            let volume = [0.0f32, 0.5, 1.0, 1.7][(lcg(&mut seed) % 4) as usize];

            // C side: two consecutive submissions (ring continuity + wrap)
            // SAFETY: serialized c_ref state
            let (c_end, c_ring) = unsafe {
                s_rawend = painted - 100; // force the < paintedtime clamp
                paintedtime = painted;
                s_rawsamples = [SamplePair::default(); MAX_RAW_SAMPLES];
                c_ref_S_RawSamples(samples, rate, width, channels, data.as_mut_ptr(), volume);
                c_ref_S_RawSamples(samples, rate, width, channels, data.as_mut_ptr(), volume);
                (s_rawend, s_rawsamples)
            };

            let mut r_ring = [SamplePair::default(); MAX_RAW_SAMPLES];
            let mut r_end = painted - 100;
            dma::raw_samples(&mut r_ring, &mut r_end, painted, samples, rate, width, channels, &data, volume, 44100);
            dma::raw_samples(&mut r_ring, &mut r_end, painted, samples, rate, width, channels, &data, volume, 44100);

            assert_eq!(c_end, r_end, "ch{channels} w{width} rate{rate}: s_rawend");
            assert_eq!(&c_ring[..], &r_ring[..], "ch{channels} w{width} rate{rate}: ring");
        }
    }
}
