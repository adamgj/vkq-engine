//! WAV sfx fuzzer (Phase 4): drives the pure quake_snd wav-info parser and,
//! when it accepts, the resampler with S_LoadSound's own gating — asserting
//! no panics. The true C-vs-Rust differential is quake-ctest's
//! snd_mem_differential (the c_ref side Sys_Error-aborts on the bad-loop
//! path, so it cannot live inside libFuzzer; same design decision as the
//! Phase 3 format targets).

#![no_main]

use libfuzzer_sys::fuzz_target;
use quake_snd::resample::{resample_sfx, SfxMeta};
use quake_snd::wav::get_wavinfo;
use quake_types::sound::WavInfo;

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


fuzz_target!(|data: &[u8]| {
    let out = get_wavinfo(data);
    if out.bad_loop_length {
        return; // the engine Sys_Errors here
    }
    let info = out.info;

    // S_LoadSound's gates, in order
    if info.channels != 1 || (info.width != 1 && info.width != 2) || info.rate <= 0 {
        return;
    }
    for shm_speed in [11025i32, 44100] {
        let stepscale = info.rate as f32 / shm_speed as f32;
        let mut len = (info.samples as f32 / stepscale) as i32;
        len = len.wrapping_mul(info.width * info.channels);
        if info.samples == 0 || len == 0 || len < 0 {
            continue;
        }
        // the read-bounds clamp (snd_mem.c)
        if !resample_in_bounds(&info, shm_speed, data.len() as i64) {
            continue;
        }
        let pcm = &data[info.dataofs as usize..];
        let mut out_buf = vec![0u8; len as usize];
        let meta = SfxMeta {
            length: info.samples,
            loopstart: info.loopstart,
            speed: info.rate,
            width: info.width,
            stereo: info.channels,
        };
        for loadas8bit in [false, true] {
            let _ = resample_sfx(meta, info.rate, info.width, shm_speed, loadas8bit, pcm, &mut out_buf);
        }
    }
});
