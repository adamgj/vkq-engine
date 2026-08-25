//! The software mixer (snd_mix.c), ported paint block for paint block.
//!
//! Bit-compat-critical throughout (ADR-010): C truncating divisions
//! (`/ 256`, never `>> 8`), the deliberate overflow form in
//! SND_PaintChannelFrom16, the lowpass filter's 4-lane partial float sums in
//! source order, and double-vs-float promotion points are all reproduced
//! exactly. Transcendentals go through `quake_c_sys::libm` (the platform
//! libm the C build links) -- // COMPAT: ADR-010.

use quake_c_sys::libm;
use quake_types::sound::{Channel, SamplePair, MAX_RAW_SAMPLES, PAINTBUFFER_SIZE};

const M_PI: f64 = core::f64::consts::PI;

/// `filter_t` (snd_mix.c). kernel/memory are Vec instead of Mem_Alloc blocks:
/// only their values feed the output, never their addresses.
#[derive(Default)]
pub struct Filter {
    memory: Vec<f32>,
    kernel: Vec<f32>,
    kernelsize: i32,
    m: i32,
    parity: i32,
    f_c: f32,
}

pub struct Underwater {
    pub intensity: f32,
    pub alpha: f32,
    accum: [f32; 2],
}

impl Default for Underwater {
    fn default() -> Self {
        // C initializer: {0.f, 1.f, {0.f, 0.f}}
        Underwater {
            intensity: 0.0,
            alpha: 1.0,
            accum: [0.0, 0.0],
        }
    }
}

/// All the file-static mixer state of snd_mix.c.
pub struct MixerState {
    pub paintbuffer: Box<[SamplePair; PAINTBUFFER_SIZE]>,
    pub scaletable: Box<[[i32; 256]; 32]>,
    filter_l: Filter,
    filter_r: Filter,
    pub underwater: Underwater,
    snd_vol: i32,
}

impl Default for MixerState {
    fn default() -> Self {
        MixerState {
            paintbuffer: Box::new([SamplePair::default(); PAINTBUFFER_SIZE]),
            scaletable: Box::new([[0; 256]; 32]),
            filter_l: Filter::default(),
            filter_r: Filter::default(),
            underwater: Underwater::default(),
            snd_vol: 0,
        }
    }
}

/// One loaded sfx cache as the mixer sees it (`sfxcache_t` view).
pub struct CacheView<'a> {
    pub length: i32,
    pub loopstart: i32,
    pub width: i32,
    /// the PCM bytes after the header (length*width valid bytes)
    pub data: &'a [u8],
}

/// S_LoadSound for the paint loop: maps a channel's sfx to its cache. The
/// pointer is the one the caller just read from the channel, so the loader
/// never touches the channels array itself (no aliasing with the paint
/// loop's `&mut` borrow).
pub trait SfxSource {
    fn load(&mut self, sfx: *mut quake_types::sound::Sfx) -> Option<CacheView<'_>>;
}

/// The engine state one S_PaintChannels call reads (dma format, cvar values,
/// raw music stream); `dma_buffer` is written through the transfer stage.
pub struct PaintParams<'a> {
    pub endtime: i32,
    pub pause_loops: bool,
    pub sfxvolume_value: f32,
    pub sndspeed_value: f32,
    pub filterquality_value: f32,
    pub shm_speed: i32,
    pub shm_samples: i32,
    pub shm_samplebits: i32,
    pub shm_channels: i32,
    pub shm_signed8: i32,
    pub dma_buffer: &'a mut [u8],
    pub s_rawend: i32,
    pub raw_samples: &'a [SamplePair; MAX_RAW_SAMPLES],
}

fn snd_write_linear_blast_stereo16(
    paint: &[SamplePair],
    out: &mut [u8],
    out_off: usize,
    count: usize,
    paint_off: usize,
) {
    // C walks paintbuffer as an int* two lanes at a time; here per pair
    for i in (0..count).step_by(2) {
        let pair = paint[paint_off + i / 2];
        for (lane, v) in [pair.left, pair.right].into_iter().enumerate() {
            let val = (v / 256).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            let idx = out_off + (i + lane) * 2;
            out[idx..idx + 2].copy_from_slice(&val.to_le_bytes());
        }
    }
}

fn transfer_stereo16(st: &mut MixerState, p: &mut PaintParams, paintedtime: i32, endtime: i32) {
    let mut lpaintedtime = paintedtime;
    let mut paint_pairs_consumed = 0usize; // C advances snd_p across iterations

    while lpaintedtime < endtime {
        // handle recirculating buffer issues
        let lpos = lpaintedtime & ((p.shm_samples >> 1) - 1);
        let out_off = (lpos << 1) as usize * 2; // short index -> byte offset

        let mut snd_linear_count = (p.shm_samples >> 1) - lpos;
        if lpaintedtime + snd_linear_count > endtime {
            snd_linear_count = endtime - lpaintedtime;
        }
        let snd_linear_count = (snd_linear_count << 1) as usize;

        snd_write_linear_blast_stereo16(
            &st.paintbuffer[..],
            p.dma_buffer,
            out_off,
            snd_linear_count,
            paint_pairs_consumed,
        );

        paint_pairs_consumed += snd_linear_count / 2;
        lpaintedtime += (snd_linear_count >> 1) as i32;
    }
}

fn transfer_paint_buffer(st: &mut MixerState, p: &mut PaintParams, paintedtime: i32, endtime: i32) {
    if p.shm_samplebits == 16 && p.shm_channels == 2 {
        transfer_stereo16(st, p, paintedtime, endtime);
        return;
    }

    let count = (endtime - paintedtime) * p.shm_channels;
    let out_mask = p.shm_samples - 1;
    let mut out_idx = (paintedtime * p.shm_channels) & out_mask;
    let step = 3 - p.shm_channels;

    // C walks paintbuffer as int*: lane index over left/right pairs
    let mut lane = 0usize;
    let read = |st: &MixerState, lane: usize| -> i32 {
        let pair = st.paintbuffer[lane / 2];
        if lane.is_multiple_of(2) {
            pair.left
        } else {
            pair.right
        }
    };

    if p.shm_samplebits == 16 {
        for _ in 0..count.max(0) {
            let val = (read(st, lane) / 256).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            lane += step as usize;
            let idx = out_idx as usize * 2;
            p.dma_buffer[idx..idx + 2].copy_from_slice(&val.to_le_bytes());
            out_idx = (out_idx + 1) & out_mask;
        }
    } else if p.shm_samplebits == 8 && p.shm_signed8 == 0 {
        for _ in 0..count.max(0) {
            let val = (read(st, lane) / 256).clamp(i16::MIN as i32, i16::MAX as i32);
            lane += step as usize;
            p.dma_buffer[out_idx as usize] = ((val / 256) + 128) as u8;
            out_idx = (out_idx + 1) & out_mask;
        }
    } else if p.shm_samplebits == 8 {
        // S8 format, e.g. with Amiga AHI
        for _ in 0..count.max(0) {
            let val = (read(st, lane) / 256).clamp(i16::MIN as i32, i16::MAX as i32);
            lane += step as usize;
            p.dma_buffer[out_idx as usize] = (val / 256) as i8 as u8;
            out_idx = (out_idx + 1) & out_mask;
        }
    }
}

/// Makes a lowpass filter kernel, from equation 16-4 in "The Scientist and
/// Engineer's Guide to Digital Signal Processing" (exact C float/double
/// promotion points).
fn make_blackman_window_kernel(kernel: &mut [f32], m: i32, f_c: f32) {
    // COMPAT: ADR-010 -- platform libm sin/cos, double math, float stores
    for i in 0..=m {
        if i == m / 2 {
            kernel[i as usize] = (2.0 * M_PI * f_c as f64) as f32;
        } else {
            let x = i as f64;
            let md = m as f64;
            kernel[i as usize] =
                ((libm::sin(2.0 * M_PI * f_c as f64 * (x - md / 2.0)) / (x - md / 2.0))
                    * (0.42 - 0.5 * libm::cos(2.0 * M_PI * x / md)
                        + 0.08 * libm::cos(4.0 * M_PI * x / md))) as f32;
        }
    }

    // normalize the kernel so all of the values sum to 1
    let mut sum: f32 = 0.0;
    for i in 0..=m {
        sum += kernel[i as usize];
    }
    for i in 0..=m {
        kernel[i as usize] /= sum;
    }
}

fn update_filter(filter: &mut Filter, m: i32, f_c: f32) {
    if filter.f_c != f_c || filter.m != m {
        filter.m = m;
        filter.f_c = f_c;

        filter.parity = 0;
        // M + 1 rounded up to the next multiple of 16
        filter.kernelsize = (m + 1) + 16 - ((m + 1) % 16);
        filter.memory = vec![0.0; filter.kernelsize as usize];
        filter.kernel = vec![0.0; filter.kernelsize as usize];

        make_blackman_window_kernel(&mut filter.kernel, m, f_c);
    }
}

/// Lowpass-filter one lane (`stride`-spaced ints) of 44100Hz audio, exactly
/// as S_ApplyFilter: decimated convolution with 4-lane partial float sums.
fn apply_filter(filter: &mut Filter, paint: &mut [SamplePair], lane: usize, count: usize) {
    let kernelsize = filter.kernelsize as usize;

    let read = |paint: &[SamplePair], i: usize| -> i32 {
        if lane == 0 {
            paint[i].left
        } else {
            paint[i].right
        }
    };

    let mut input = vec![0.0f32; kernelsize + count];

    // memory holds the previous kernelsize samples of input
    input[..kernelsize].copy_from_slice(&filter.memory);

    for i in 0..count {
        // C: data[i*stride] / (32768.0 * 256.0) -- double division, float store
        input[kernelsize + i] = (read(paint, i) as f64 / (32768.0 * 256.0)) as f32;
    }

    // copy out the last kernelsize samples to memory for next time
    filter
        .memory
        .copy_from_slice(&input[count..count + kernelsize]);

    // apply the filter
    let mut parity = filter.parity;
    let kernel = &filter.kernel;

    for i in 0..count {
        let input_plus_i = &input[i..];
        let mut val = [0.0f32; 4];

        let mut j = ((4 - parity) % 4) as usize;
        while j < kernelsize {
            val[0] += kernel[j] * input_plus_i[j];
            val[1] += kernel[j + 4] * input_plus_i[j + 4];
            val[2] += kernel[j + 8] * input_plus_i[j + 8];
            val[3] += kernel[j + 12] * input_plus_i[j + 12];
            j += 16;
        }

        // 4.0 factor is to increase volume by 12 dB (C: float sum promoted to
        // double by the constant, then truncated to int)
        let out = ((val[0] + val[1] + val[2] + val[3]) as f64 * (32768.0 * 256.0 * 4.0)) as i32;
        if lane == 0 {
            paint[i].left = out;
        } else {
            paint[i].right = out;
        }

        parity = (parity + 1) % 4;
    }

    filter.parity = parity;
}

/// S_LowpassFilter: M/bw table keyed on snd_filterquality.
fn lowpass_filter(
    filterquality_value: f32,
    paint: &mut [SamplePair],
    lane: usize,
    count: usize,
    filter: &mut Filter,
) {
    let (m, bw): (i32, f64) = match filterquality_value as i32 {
        1 => (126, 0.900),
        2 => (150, 0.915),
        3 => (174, 0.930),
        4 => (198, 0.945),
        _ => (222, 0.960),
    };

    // C: f_c = (bw * 11025 / 2.0) / 44100.0 (double), float store
    let f_c = ((bw * 11025.0 / 2.0) / 44100.0) as f32;

    update_filter(filter, m, f_c);
    apply_filter(filter, paint, lane, count);
}

/// S_SetUnderwaterIntensity (exact double promotion points).
pub fn set_underwater_intensity(
    uw: &mut Underwater,
    mut target: f32,
    snd_waterfx_value: f32,
    host_frametime: f64,
) {
    target *= snd_waterfx_value.clamp(0.0, 2.0);
    if uw.intensity < target {
        // C: intensity += host_frametime * 4.f (double), float store
        uw.intensity = (uw.intensity as f64 + host_frametime * 4.0) as f32;
        uw.intensity = uw.intensity.min(target);
    } else if uw.intensity > target {
        uw.intensity = (uw.intensity as f64 - host_frametime * 4.0) as f32;
        uw.intensity = uw.intensity.max(target);
    }
    // COMPAT: ADR-010 -- platform libm exp/log; 12.f promotes to double
    uw.alpha = libm::exp(-uw.intensity as f64 * libm::log(12.0)) as f32;
}

fn underwater_filter(uw: &mut Underwater, paint: &mut [SamplePair], endtime: usize) {
    if uw.intensity == 0.0 {
        if endtime > 0 {
            uw.accum[0] = paint[endtime - 1].left as f32;
            uw.accum[1] = paint[endtime - 1].right as f32;
        }
        return;
    }
    for pb in paint.iter_mut().take(endtime) {
        uw.accum[0] += uw.alpha * (pb.left as f32 - uw.accum[0]);
        uw.accum[1] += uw.alpha * (pb.right as f32 - uw.accum[1]);
        pb.left = uw.accum[0] as i32;
        pb.right = uw.accum[1] as i32;
    }
}

/// SND_InitScaletable (float truncation preserved).
pub fn init_scaletable(st: &mut MixerState, sfxvolume_value: f32) {
    for i in 0..32usize {
        // C: scale = i * 8 * 256 * sfxvolume.value (int*float -> float -> int)
        let scale = ((i * 8 * 256) as f32 * sfxvolume_value) as i32;
        for j in 0..256usize {
            let sj = if j < 128 { j as i32 } else { j as i32 - 256 };
            st.scaletable[i][j] = sj.wrapping_mul(scale);
        }
    }
}

fn paint_channel_from8(
    st: &mut MixerState,
    ch: &mut Channel,
    sc: &CacheView,
    count: i32,
    paintbufferstart: i32,
) {
    if ch.leftvol > 255 {
        ch.leftvol = 255;
    }
    if ch.rightvol > 255 {
        ch.rightvol = 255;
    }

    let lscale = &st.scaletable[(ch.leftvol >> 3) as usize];
    let rscale = &st.scaletable[(ch.rightvol >> 3) as usize];
    let base = ch.pos as i64;

    for i in 0..count.max(0) {
        // COMPAT: on stale channels the C reads past the cache (UB); we read 0
        let data = sc
            .data
            .get((base + i as i64) as usize)
            .copied()
            .unwrap_or(0) as usize;
        let pb = &mut st.paintbuffer[(paintbufferstart + i) as usize];
        pb.left = pb.left.wrapping_add(lscale[data]);
        pb.right = pb.right.wrapping_add(rscale[data]);
    }

    ch.pos += count;
}

fn paint_channel_from16(
    st: &mut MixerState,
    ch: &mut Channel,
    sc: &CacheView,
    count: i32,
    paintbufferstart: i32,
) {
    // moved >>8 to the volumes to avoid the observed overflow (C comment)
    let leftvol = ch.leftvol.wrapping_mul(st.snd_vol) / 256;
    let rightvol = ch.rightvol.wrapping_mul(st.snd_vol) / 256;
    let base = ch.pos as i64;

    for i in 0..count.max(0) {
        let idx = ((base + i as i64) * 2) as usize;
        // COMPAT: on stale channels the C reads past the cache (UB); we read 0
        let data = match (sc.data.get(idx), sc.data.get(idx + 1)) {
            (Some(&lo), Some(&hi)) => i16::from_le_bytes([lo, hi]) as i32,
            _ => 0,
        };
        let pb = &mut st.paintbuffer[(paintbufferstart + i) as usize];
        pb.left = pb.left.wrapping_add(data.wrapping_mul(leftvol));
        pb.right = pb.right.wrapping_add(data.wrapping_mul(rightvol));
    }

    ch.pos += count;
}

/// S_PaintChannels. `on_transfer` fires after each block's transfer with
/// (block_start, block_end, painted paintbuffer region, dma buffer) -- the
/// -sndhash instrument point, mirroring the C hook exactly.
pub fn paint_channels<S: SfxSource>(
    st: &mut MixerState,
    paintedtime: &mut i32,
    channels: &mut [Channel],
    sfx: &mut S,
    p: &mut PaintParams,
    mut on_transfer: impl FnMut(i32, i32, &[SamplePair], &[u8]),
) {
    let endtime = p.endtime;
    // C: snd_vol = sfxvolume.value * 256 (float -> int truncation)
    st.snd_vol = (p.sfxvolume_value * 256.0) as i32;

    while *paintedtime < endtime {
        // if paintbuffer is smaller than DMA buffer
        let mut end = endtime;
        if endtime - *paintedtime > PAINTBUFFER_SIZE as i32 {
            end = *paintedtime + PAINTBUFFER_SIZE as i32;
        }

        // clear the paint buffer
        let block = (end - *paintedtime) as usize;
        st.paintbuffer[..block].fill(SamplePair::default());

        // paint in the channels
        for ch in channels.iter_mut() {
            if ch.sfx.is_null() {
                continue;
            }
            if ch.leftvol == 0 && ch.rightvol == 0 {
                continue;
            }
            let Some(sc) = sfx.load(ch.sfx) else {
                continue;
            };
            if sc.loopstart >= 0 && p.pause_loops {
                continue;
            }

            let mut ltime = *paintedtime;

            while ltime < end {
                // paint up to end
                let count = if ch.end < end {
                    ch.end - ltime
                } else {
                    end - ltime
                };

                if count > 0 {
                    if sc.width == 1 {
                        paint_channel_from8(st, ch, &sc, count, ltime - *paintedtime);
                    } else {
                        paint_channel_from16(st, ch, &sc, count, ltime - *paintedtime);
                    }
                    ltime += count;
                }

                // if at end of loop, restart
                if ltime >= ch.end {
                    if sc.loopstart >= 0 {
                        ch.pos = sc.loopstart;
                        ch.end = ltime + sc.length - ch.pos;
                    } else {
                        // channel just stopped
                        ch.sfx = core::ptr::null_mut();
                        break;
                    }
                }
            }
        }

        // clip each sample to 0dB, then reduce by 6dB (headroom for the
        // lowpass filter and the music); the lowpass smooths the clipping
        for pb in st.paintbuffer[..block].iter_mut() {
            pb.left = pb.left.clamp(-32768 * 256, 32767 * 256) / 2;
            pb.right = pb.right.clamp(-32768 * 256, 32767 * 256) / 2;
        }

        // apply a lowpass filter
        if p.sndspeed_value == 11025.0 && p.shm_speed == 44100 {
            lowpass_filter(
                p.filterquality_value,
                &mut st.paintbuffer[..],
                0,
                block,
                &mut st.filter_l,
            );
            lowpass_filter(
                p.filterquality_value,
                &mut st.paintbuffer[..],
                1,
                block,
                &mut st.filter_r,
            );
        }

        underwater_filter(&mut st.underwater, &mut st.paintbuffer[..], block);

        // paint in the music
        if p.s_rawend >= *paintedtime {
            // copy from the streaming sound source
            let stop = if end < p.s_rawend { end } else { p.s_rawend };
            let mut i = *paintedtime;
            while i < stop {
                let s = (i & (MAX_RAW_SAMPLES as i32 - 1)) as usize;
                let pb = &mut st.paintbuffer[(i - *paintedtime) as usize];
                // lower music by 6db to match sfx
                pb.left = pb.left.wrapping_add(p.raw_samples[s].left / 2);
                pb.right = pb.right.wrapping_add(p.raw_samples[s].right / 2);
                i += 1;
            }
        }

        // transfer out according to DMA format
        transfer_paint_buffer(st, p, *paintedtime, end);
        on_transfer(*paintedtime, end, &st.paintbuffer[..block], p.dma_buffer);
        *paintedtime = end;
    }
}
