//! `ResampleSfx` (snd_mem.c): resample/decimate a loaded sfx to the DMA rate.
//!
//! Arithmetic is bit-compat-critical (ADR-010): the C truncates
//! `length / stepscale` through float, builds `fracstep` by float-to-int
//! truncation, and accumulates `samplefrac` in an `int64_t` -- all
//! reproduced exactly here.

/// The `sfxcache_t` header fields the resampler reads and rewrites.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SfxMeta {
    pub length: i32,
    pub loopstart: i32,
    pub speed: i32,
    pub width: i32,
    pub stereo: i32,
}

/// Port of `ResampleSfx`. `data` is the raw PCM at the wav's `dataofs`
/// (the full remaining file slice, exactly the bytes the C could address);
/// `out` is the destination PCM area (`len` bytes as computed by
/// `S_LoadSound`). Returns the rewritten header fields.
pub fn resample_sfx(
    mut meta: SfxMeta,
    inrate: i32,
    inwidth: i32,
    shm_speed: i32,
    loadas8bit: bool,
    data: &[u8],
    out: &mut [u8],
) -> SfxMeta {
    let stepscale = inrate as f32 / shm_speed as f32; // usually 0.5, 1, or 2

    let outcount = (meta.length as f32 / stepscale) as i32;
    meta.length = outcount;
    if meta.loopstart != -1 {
        meta.loopstart = (meta.loopstart as f32 / stepscale) as i32;
    }

    meta.speed = shm_speed;
    meta.width = if loadas8bit { 1 } else { inwidth };
    meta.stereo = 0;

    // resample / decimate to the current source rate
    // C loops are `for (i = 0; i < outcount; i++)` with a signed i: a
    // negative outcount (hostile headers) runs zero iterations
    let iters = outcount.max(0) as usize;
    if stepscale == 1.0 && inwidth == 1 && meta.width == 1 {
        // fast special case
        for i in 0..iters {
            out[i] = data[i].wrapping_sub(128);
        }
    } else {
        // general case
        // samplefrac can overflow 2**31 with very big sounds (C comment)
        let mut samplefrac: i64 = 0;
        let fracstep = (stepscale * 256.0) as i32;
        for i in 0..iters {
            let srcsample = (samplefrac >> 8) as i32 as usize;
            samplefrac += fracstep as i64;
            let sample: i32 = if inwidth == 2 {
                i16::from_le_bytes([data[srcsample * 2], data[srcsample * 2 + 1]]) as i32
            } else {
                // C: (unsigned int)((unsigned char)data[s] - 128) << 8, into int
                (((data[srcsample] as i32 - 128) as u32) << 8) as i32
            };
            if meta.width == 2 {
                let v = sample as i16;
                out[i * 2..i * 2 + 2].copy_from_slice(&v.to_le_bytes());
            } else {
                out[i] = (sample >> 8) as i8 as u8;
            }
        }
    }

    meta
}
