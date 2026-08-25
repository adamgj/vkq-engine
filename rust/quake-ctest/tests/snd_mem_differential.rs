//! Phase 4 M2: differential tests for the WAV-info parser and the sfx
//! resampler -- `quake_snd::wav::get_wavinfo` / `quake_snd::resample` vs the
//! c_ref-compiled `snd_mem.c` originals, over synthetic fixtures and (when
//! `QUAKE_GAME_DATA` is set) every WAV in the real id1 pak0.pak.

use core::ffi::{c_char, c_int, c_void};

use quake_ctest::fs as ctfs;
use quake_snd::resample::{resample_sfx, SfxMeta};
use quake_snd::wav::{get_wavinfo, Msg};
use quake_types::sound::WavInfo;

extern "C" {
    fn c_ref_GetWavinfo(name: *const c_char, wav: *mut u8, wavlength: c_int) -> WavInfo;
    fn ctest_snd_setup(shm_speed: c_int, loadas8bit_value: f32);
    fn ctest_resample_ref(
        length: c_int,
        loopstart: c_int,
        inrate: c_int,
        inwidth: c_int,
        stereo: c_int,
        data: *const u8,
        out: *mut u8,
        out_len: c_int,
        meta_out: *mut c_int,
    );
}

const NAME: &str = "testcase.wav";

fn render_msg(m: &Msg, name: &str) -> String {
    match m {
        Msg::BadChunkLen { name: chunk, len } => {
            format!("[dcon2] bad \"{chunk}\" chunk length ({len})\n")
        }
        Msg::MissingRiffWave => format!("[con] {name} missing RIFF/WAVE chunks\n"),
        Msg::MissingFmt => format!("[con] {name} is missing fmt chunk\n"),
        Msg::NotPcm => format!("[con] {name} is not Microsoft PCM format\n"),
        Msg::MissingData => format!("[con] {name} is missing data chunk\n"),
        Msg::LoopStartGeEnd => format!("[warn] {name} has loop start >= end\n"),
    }
}

/// Runs both sides on one buffer and asserts identical wavinfo_t, identical
/// console output, and matching Sys_Error behavior. Returns the (agreed)
/// parse result.
fn diff_wavinfo(tag: &str, data: &[u8]) -> (WavInfo, bool) {
    ctfs::clear_logs();
    let name = std::ffi::CString::new(NAME).unwrap();
    let mut c_info = WavInfo::default();
    let mut buf = data.to_vec();
    let err = ctfs::catch_sys_error(|| {
        // SAFETY: c_ref_GetWavinfo reads at most `len` bytes of `buf` (the
        // bounds clamps landed in snd_mem.c) and the name only for messages
        c_info = unsafe { c_ref_GetWavinfo(name.as_ptr(), buf.as_mut_ptr(), buf.len() as c_int) };
    });
    let c_log = ctfs::con_log();

    let r = get_wavinfo(data);
    let r_log: Vec<String> = r.messages.iter().map(|m| render_msg(m, NAME)).collect();

    match &err {
        Some(msg) => {
            assert!(r.bad_loop_length, "{tag}: C Sys_Error'd ({msg}), Rust did not");
            assert_eq!(msg, &format!("{NAME} has a bad loop length"), "{tag}");
        }
        None => {
            assert!(!r.bad_loop_length, "{tag}: Rust flagged bad loop length, C did not");
            assert_eq!(c_info, r.info, "{tag}: wavinfo_t mismatch");
        }
    }
    assert_eq!(c_log, r_log, "{tag}: console output mismatch");
    (r.info, r.bad_loop_length)
}

// ---------------------------------------------------------------------------
// fixture builders

fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(id);
    v.extend_from_slice(&(body.len() as u32).to_le_bytes());
    v.extend_from_slice(body);
    if body.len() % 2 == 1 {
        v.push(0); // RIFF pad byte
    }
    v
}

fn fmt_chunk(format: u16, channels: u16, rate: u32, bits: u16) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&format.to_le_bytes());
    b.extend_from_slice(&channels.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * channels as u32 * bits as u32 / 8).to_le_bytes());
    b.extend_from_slice(&((channels * bits / 8) as u16).to_le_bytes());
    b.extend_from_slice(&bits.to_le_bytes());
    chunk(b"fmt ", &b)
}

fn cue_chunk(loopstart: u32) -> Vec<u8> {
    // 1 cue point; the parser only reads the sample offset at data offset 24
    let mut b = vec![0u8; 28];
    b[0..4].copy_from_slice(&1u32.to_le_bytes());
    b[24..28].copy_from_slice(&loopstart.to_le_bytes());
    chunk(b"cue ", &b)
}

fn list_mark_chunk(loop_samples: u32) -> Vec<u8> {
    // cooledit loop-length marker: "mark" tag at data offset 20, the sample
    // count at data offset 16 (header offsets 28 and 24)
    let mut b = vec![0u8; 32];
    b[16..20].copy_from_slice(&loop_samples.to_le_bytes());
    b[20..24].copy_from_slice(b"mark");
    chunk(b"LIST", &b)
}

fn riff(inner: &[Vec<u8>]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(b"WAVE");
    for c in inner {
        body.extend_from_slice(c);
    }
    let mut v = Vec::new();
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(body.len() as u32).to_le_bytes());
    v.extend_from_slice(&body);
    v
}

fn pcm8(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 37 + 11) as u8).collect()
}

fn pcm16(n: usize) -> Vec<u8> {
    let mut v = Vec::new();
    for i in 0..n {
        v.extend_from_slice(&(((i as i32 * 2711 - 17000) % 32768) as i16).to_le_bytes());
    }
    v
}

// ---------------------------------------------------------------------------

#[test]
fn wavinfo_synthetic_fixtures() {
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty", Vec::new()),
        ("garbage", b"not a wav at all, nothing here".to_vec()),
        ("truncated-riff", b"RIFF\x04\x00".to_vec()),
        ("riff-short-chunk", {
            // RIFF chunk of 2 bytes: no room for "WAVE" (the OOB clamp case)
            let mut v = Vec::new();
            v.extend_from_slice(b"RIFF");
            v.extend_from_slice(&2u32.to_le_bytes());
            v.extend_from_slice(b"WA");
            v
        }),
        ("bad-chunk-len", {
            let mut v = Vec::new();
            v.extend_from_slice(b"RIFF");
            v.extend_from_slice(&0xffff_ffu32.to_le_bytes()); // longer than file
            v.extend_from_slice(b"WAVExxxx");
            v
        }),
        ("negative-chunk-len", {
            let mut v = Vec::new();
            v.extend_from_slice(b"RIFF");
            v.extend_from_slice(&0x8000_0000u32.to_le_bytes());
            v.extend_from_slice(b"WAVExxxx");
            v
        }),
        ("no-fmt", riff(&[chunk(b"data", &pcm8(16))])),
        ("short-fmt", {
            // fmt chunk of 8 bytes: the 16-byte field read clamp case
            riff(&[chunk(b"fmt ", &[1, 0, 1, 0, 0x11, 0x2b, 0, 0]), chunk(b"data", &pcm8(16))])
        }),
        ("not-pcm", riff(&[fmt_chunk(2, 1, 11025, 8), chunk(b"data", &pcm8(16))])),
        ("bad-width", riff(&[fmt_chunk(1, 1, 11025, 12), chunk(b"data", &pcm8(16))])),
        ("stereo", riff(&[fmt_chunk(1, 2, 11025, 8), chunk(b"data", &pcm8(16))])),
        ("no-data", riff(&[fmt_chunk(1, 1, 11025, 8)])),
        ("mono8", riff(&[fmt_chunk(1, 1, 11025, 8), chunk(b"data", &pcm8(64))])),
        ("mono8-odd-len", riff(&[fmt_chunk(1, 1, 11025, 8), chunk(b"data", &pcm8(63))])),
        ("mono16", riff(&[fmt_chunk(1, 1, 22050, 16), chunk(b"data", &pcm16(64))])),
        ("looped", riff(&[
            fmt_chunk(1, 1, 11025, 8),
            cue_chunk(16),
            chunk(b"data", &pcm8(64)),
        ])),
        ("looped-mark", riff(&[
            fmt_chunk(1, 1, 11025, 8),
            cue_chunk(16),
            list_mark_chunk(32),
            chunk(b"data", &pcm8(64)),
        ])),
        ("short-cue", {
            // cue chunk of 8 bytes: the 28-byte loopstart read clamp case
            riff(&[
                fmt_chunk(1, 1, 11025, 8),
                chunk(b"cue ", &[1, 0, 0, 0, 0, 0, 0, 0]),
                chunk(b"data", &pcm8(64)),
            ])
        }),
        ("loop-ge-end", riff(&[
            fmt_chunk(1, 1, 11025, 8),
            cue_chunk(64),
            chunk(b"data", &pcm8(64)),
        ])),
        ("bad-loop-length", riff(&[
            fmt_chunk(1, 1, 11025, 8),
            cue_chunk(16),
            list_mark_chunk(1000), // mark says more samples than data has
            chunk(b"data", &pcm8(64)),
        ])),
        ("list-no-mark", riff(&[
            fmt_chunk(1, 1, 11025, 8),
            cue_chunk(16),
            chunk(b"LIST", &[0u8; 32]),
            chunk(b"data", &pcm8(64)),
        ])),
        ("short-list", riff(&[
            fmt_chunk(1, 1, 11025, 8),
            cue_chunk(16),
            chunk(b"LIST", &[0u8; 16]),
            chunk(b"data", &pcm8(64)),
        ])),
    ];

    for (tag, data) in &cases {
        diff_wavinfo(tag, data);
    }
}

// ---------------------------------------------------------------------------
// resampler

/// S_LoadSound's allocation-size computation, reproduced exactly (float
/// truncations included) so both sides get identically sized buffers.
fn load_len(info: &WavInfo, shm_speed: i32) -> Option<i32> {
    if info.channels != 1 || (info.width != 1 && info.width != 2) || info.rate <= 0 {
        return None;
    }
    let stepscale = info.rate as f32 / shm_speed as f32;
    let mut len = (info.samples as f32 / stepscale) as i32;
    len = len.wrapping_mul(info.width * info.channels);
    if info.samples == 0 || len <= 0 {
        // len < 0: the engine's Mem_Alloc of a huge size fails and the sound
        // is rejected before the resampler runs
        return None;
    }
    Some(len)
}

/// snd_mem.c's read-bounds clamp, reproduced exactly: the last source index
/// the resampler touches must lie inside the loaded file
fn resample_in_bounds(info: &WavInfo, shm_speed: i32, file_len: i64) -> bool {
    let stepscale = info.rate as f32 / shm_speed as f32;
    let last_srcsample: i64 = if stepscale == 1.0 && info.width == 1 {
        info.samples as i64 - 1
    } else {
        let outcount = (info.samples as f32 / stepscale) as i32;
        let fracstep = (stepscale * 256.0) as i32;
        ((outcount as i64 - 1) * fracstep as i64) >> 8
    };
    info.dataofs as i64 + (last_srcsample + 1) * info.width as i64 <= file_len
}

fn diff_resample(tag: &str, info: &WavInfo, pcm: &[u8], shm_speed: i32, loadas8bit: bool) {
    let Some(len) = load_len(info, shm_speed) else {
        return;
    };
    // the read-bounds clamp in S_LoadSound
    if !resample_in_bounds(info, shm_speed, (info.dataofs as usize + pcm.len()) as i64) {
        return;
    }

    // SAFETY: setup writes stub-owned globals; single-threaded test
    unsafe { ctest_snd_setup(shm_speed, if loadas8bit { 1.0 } else { 0.0 }) };

    let mut c_out = vec![0u8; len as usize];
    let mut c_meta = [0 as c_int; 5];
    // SAFETY: out has len bytes, exactly S_LoadSound's alloc; data outlives
    // the call; meta_out points at 5 ints
    unsafe {
        ctest_resample_ref(
            info.samples,
            info.loopstart,
            info.rate,
            info.width,
            info.channels,
            pcm.as_ptr(),
            c_out.as_mut_ptr(),
            len,
            c_meta.as_mut_ptr(),
        )
    };

    let mut r_out = vec![0u8; len as usize];
    let meta = SfxMeta {
        length: info.samples,
        loopstart: info.loopstart,
        speed: info.rate,
        width: info.width,
        stereo: info.channels,
    };
    let r_meta = resample_sfx(meta, info.rate, info.width, shm_speed, loadas8bit, pcm, &mut r_out);

    assert_eq!(
        [r_meta.length, r_meta.loopstart, r_meta.speed, r_meta.width, r_meta.stereo],
        [c_meta[0], c_meta[1], c_meta[2], c_meta[3], c_meta[4]],
        "{tag}: sfxcache header mismatch (speed={shm_speed} loadas8bit={loadas8bit})"
    );
    assert_eq!(
        r_out, c_out,
        "{tag}: PCM mismatch (speed={shm_speed} loadas8bit={loadas8bit})"
    );
}

fn diff_both(tag: &str, file: &[u8]) {
    let (info, fatal) = diff_wavinfo(tag, file);
    if fatal || info.samples == 0 || info.dataofs < 0 {
        return;
    }
    let pcm = &file[info.dataofs as usize..];
    for &speed in &[11025, 22050, 44100] {
        for &as8 in &[false, true] {
            diff_resample(tag, &info, pcm, speed, as8);
        }
    }
}

#[test]
fn resample_synthetic_fixtures() {
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("mono8-11025", riff(&[fmt_chunk(1, 1, 11025, 8), chunk(b"data", &pcm8(1000))])),
        ("mono8-22050", riff(&[fmt_chunk(1, 1, 22050, 8), chunk(b"data", &pcm8(1000))])),
        ("mono8-44100", riff(&[fmt_chunk(1, 1, 44100, 8), chunk(b"data", &pcm8(1000))])),
        ("mono16-11025", riff(&[fmt_chunk(1, 1, 11025, 16), chunk(b"data", &pcm16(1000))])),
        ("mono16-44100", riff(&[fmt_chunk(1, 1, 44100, 16), chunk(b"data", &pcm16(1000))])),
        ("mono16-8012", riff(&[fmt_chunk(1, 1, 8012, 16), chunk(b"data", &pcm16(777))])),
        ("looped-16", riff(&[
            fmt_chunk(1, 1, 11025, 16),
            cue_chunk(100),
            chunk(b"data", &pcm16(500)),
        ])),
    ];
    for (tag, data) in &cases {
        diff_both(tag, data);
    }
}

// ---------------------------------------------------------------------------
// real assets (env-gated on QUAKE_GAME_DATA, like the Phase 3 suites)

/// minimal pak directory reader; enough to enumerate and extract *.wav
fn pak_entries(pak: &[u8]) -> Vec<(String, usize, usize)> {
    assert_eq!(&pak[0..4], b"PACK");
    let dirofs = i32::from_le_bytes(pak[4..8].try_into().unwrap()) as usize;
    let dirlen = i32::from_le_bytes(pak[8..12].try_into().unwrap()) as usize;
    let mut out = Vec::new();
    for e in pak[dirofs..dirofs + dirlen].chunks_exact(64) {
        let name_end = e.iter().position(|&b| b == 0).unwrap_or(56);
        let name = String::from_utf8_lossy(&e[..name_end]).into_owned();
        let pos = i32::from_le_bytes(e[56..60].try_into().unwrap()) as usize;
        let len = i32::from_le_bytes(e[60..64].try_into().unwrap()) as usize;
        out.push((name, pos, len));
    }
    out
}

#[test]
fn real_id1_wavs() {
    let Ok(root) = std::env::var("QUAKE_GAME_DATA") else {
        eprintln!("QUAKE_GAME_DATA not set; skipping real-asset wav corpus");
        return;
    };
    let mut ran = 0;
    for pakname in ["pak0.pak", "pak1.pak"] {
        let path = std::path::Path::new(&root).join("id1").join(pakname);
        let Ok(pak) = std::fs::read(&path) else { continue };
        for (name, pos, len) in pak_entries(&pak) {
            if !name.to_ascii_lowercase().ends_with(".wav") {
                continue;
            }
            diff_both(&name, &pak[pos..pos + len]);
            ran += 1;
        }
    }
    assert!(ran > 0, "no wavs found under {root}/id1");
    eprintln!("real wav corpus: {ran} files");
}

// keep the linked stubs' state helpers referenced so dead-code stripping
// never drops the c_ref fs the wav loader shares state with
#[allow(unused)]
unsafe fn _keep(_: *mut c_void) {}
