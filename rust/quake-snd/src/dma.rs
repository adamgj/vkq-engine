//! Pure logic from snd_dma.c (Phase 4 M6): channel picking, spatialization,
//! and the raw-sample music scheduler. Float promotion points match the C
//! exactly (ADR-010); vector math is the bit-exact `quake-math` port.

use quake_math::mathlib::{dot_product, vector_normalize};
use quake_types::sound::{Channel, SamplePair, MAX_DYNAMIC_CHANNELS, MAX_RAW_SAMPLES, NUM_AMBIENTS};

/// `#define sound_nominal_clip_dist 1000.0` (snd_dma.c)
pub const SOUND_NOMINAL_CLIP_DIST: f64 = 1000.0;

/// SND_PickChannel: picks a channel based on priorities, empty slots, number
/// of channels. Returns the channel index (clearing its sfx like the C) or
/// None. `channels` must span at least NUM_AMBIENTS+MAX_DYNAMIC_CHANNELS.
pub fn pick_channel(
    channels: &mut [Channel],
    paintedtime: i32,
    cl_viewentity: i32,
    entnum: i32,
    entchannel: i32,
) -> Option<usize> {
    // Check for replacement sound, or find the best one to replace
    let mut first_to_die: i32 = -1;
    let mut life_left: i32 = 0x7fffffff;
    for ch_idx in NUM_AMBIENTS..NUM_AMBIENTS + MAX_DYNAMIC_CHANNELS {
        let ch = &channels[ch_idx];
        if entchannel != 0 // channel 0 never overrides
            && ch.entnum == entnum
            && (ch.entchannel == entchannel || entchannel == -1)
        {
            // always override sound from same entity
            first_to_die = ch_idx as i32;
            break;
        }

        // don't let monster sounds override player sounds
        if ch.entnum == cl_viewentity && entnum != cl_viewentity && !ch.sfx.is_null() {
            continue;
        }

        if ch.end.wrapping_sub(paintedtime) < life_left {
            life_left = ch.end.wrapping_sub(paintedtime);
            first_to_die = ch_idx as i32;
        }
    }

    if first_to_die == -1 {
        return None;
    }

    let i = first_to_die as usize;
    if !channels[i].sfx.is_null() {
        channels[i].sfx = core::ptr::null_mut();
    }
    Some(i)
}

/// SND_Spatialize: stereo separation and distance attenuation. The C
/// promotes through double at `1.0 + dot` / `(1.0 - dist) * rscale`, then
/// stores through float locals -- reproduced exactly.
pub fn spatialize(
    ch: &mut Channel,
    listener_origin: &[f32; 3],
    listener_right: &[f32; 3],
    cl_viewentity: i32,
    shm_channels: i32,
) {
    // anything coming from the view entity will always be full volume
    if ch.entnum == cl_viewentity {
        ch.leftvol = ch.master_vol;
        ch.rightvol = ch.master_vol;
        return;
    }

    // calculate stereo seperation and distance attenuation
    let mut source_vec = [
        ch.origin[0] - listener_origin[0],
        ch.origin[1] - listener_origin[1],
        ch.origin[2] - listener_origin[2],
    ];
    let dist = vector_normalize(&mut source_vec) * ch.dist_mult;
    let dot = dot_product(listener_right, &source_vec);

    let (rscale, lscale): (f32, f32) = if shm_channels == 1 {
        (1.0, 1.0)
    } else {
        ((1.0f64 + dot as f64) as f32, (1.0f64 - dot as f64) as f32)
    };

    // add in distance effect
    let scale = ((1.0f64 - dist as f64) * rscale as f64) as f32;
    ch.rightvol = (ch.master_vol as f32 * scale) as i32;
    if ch.rightvol < 0 {
        ch.rightvol = 0;
    }

    let scale = ((1.0f64 - dist as f64) * lscale as f64) as f32;
    ch.leftvol = (ch.master_vol as f32 * scale) as i32;
    if ch.leftvol < 0 {
        ch.leftvol = 0;
    }
}

/// S_RawSamples (from QuakeII): streaming music support. Expects data in
/// signed 16 bit, or unsigned 8 bit format; byte swapping is the codec's job.
#[allow(clippy::too_many_arguments)]
pub fn raw_samples(
    ring: &mut [SamplePair; MAX_RAW_SAMPLES],
    s_rawend: &mut i32,
    paintedtime: i32,
    samples: i32,
    rate: i32,
    width: i32,
    channels: i32,
    data: &[u8],
    volume: f32,
    shm_speed: i32,
) {
    if *s_rawend < paintedtime {
        *s_rawend = paintedtime;
    }

    let scale = rate as f32 / shm_speed as f32;
    let mut int_volume = (256.0 * volume) as i32;

    let s16 = |data: &[u8], idx: i32| -> i32 {
        let b = (idx as usize) * 2;
        i16::from_le_bytes([data[b], data[b + 1]]) as i32
    };

    if channels == 2 && width == 2 {
        let mut i = 0i32;
        loop {
            let src = (i as f32 * scale) as i32;
            if src >= samples {
                break;
            }
            let dst = (*s_rawend & (MAX_RAW_SAMPLES as i32 - 1)) as usize;
            *s_rawend += 1;
            ring[dst].left = s16(data, src * 2).wrapping_mul(int_volume);
            ring[dst].right = s16(data, src * 2 + 1).wrapping_mul(int_volume);
            i += 1;
        }
    } else if channels == 1 && width == 2 {
        let mut i = 0i32;
        loop {
            let src = (i as f32 * scale) as i32;
            if src >= samples {
                break;
            }
            let dst = (*s_rawend & (MAX_RAW_SAMPLES as i32 - 1)) as usize;
            *s_rawend += 1;
            ring[dst].left = s16(data, src).wrapping_mul(int_volume);
            ring[dst].right = s16(data, src).wrapping_mul(int_volume);
            i += 1;
        }
    } else if channels == 2 && width == 1 {
        int_volume = int_volume.wrapping_mul(256);
        let mut i = 0i32;
        loop {
            let src = (i as f32 * scale) as i32;
            if src >= samples {
                break;
            }
            let dst = (*s_rawend & (MAX_RAW_SAMPLES as i32 - 1)) as usize;
            *s_rawend += 1;
            ring[dst].left = (data[(src * 2) as usize] as i32 - 128).wrapping_mul(int_volume);
            ring[dst].right = (data[(src * 2 + 1) as usize] as i32 - 128).wrapping_mul(int_volume);
            i += 1;
        }
    } else if channels == 1 && width == 1 {
        int_volume = int_volume.wrapping_mul(256);
        let mut i = 0i32;
        loop {
            let src = (i as f32 * scale) as i32;
            if src >= samples {
                break;
            }
            let dst = (*s_rawend & (MAX_RAW_SAMPLES as i32 - 1)) as usize;
            *s_rawend += 1;
            let v = (data[src as usize] as i32 - 128).wrapping_mul(int_volume);
            ring[dst].left = v;
            ring[dst].right = v;
            i += 1;
        }
    }
}
