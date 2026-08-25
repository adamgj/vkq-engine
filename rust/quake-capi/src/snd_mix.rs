//! S_PaintChannels / SND_InitScaletable / S_SetUnderwaterIntensity shims
//! (Quake/snd_mix.c, Phase 4 M5).
//!
//! The mixer's file-static state (paintbuffer, scaletable, filters,
//! underwater) lives Rust-side in a process-global `MixerState`; everything
//! else (channels, timing, cvars, the DMA block) stays C-owned and is read
//! through quake-c-sys exactly where the C read it. ADR-007: sound-globals
//! row -- storage C, mixer state Rust, main-thread access under the engine's
//! recursive snd_mutex (held by the S_Update_/S_ExtraUpdate callers).

use core::ffi::c_int;
use std::sync::{LazyLock, Mutex};

use quake_c_sys as sys;
use quake_snd::mix::{self, CacheView, MixerState, PaintParams, SfxSource};
use quake_types::sound::{Channel, SamplePair, SfxCache, MAX_RAW_SAMPLES};

extern "C" {
    // snd_glue.c (compiled only under -Duse_rust_snd)
    fn SND_Glue_PauseLoops() -> c_int;
}

static MIXER: LazyLock<Mutex<MixerState>> = LazyLock::new(|| Mutex::new(MixerState::default()));

/// The paint loop's S_LoadSound: calls our own exported loader (the same C
/// symbol the engine links) and views the returned cache.
struct EngineSfxSource {
    channels: *const Channel,
}

impl SfxSource for EngineSfxSource {
    fn load(&mut self, ch_index: usize) -> Option<CacheView<'_>> {
        // SAFETY: ch_index < total_channels; the paint loop already checked
        // ch->sfx non-null. S_LoadSound is our own export (same locking as C).
        unsafe {
            let sfxp = (*self.channels.add(ch_index)).sfx;
            let sc = crate::snd_mem::S_LoadSound(sfxp);
            if sc.is_null() {
                return None;
            }
            let sc: &SfxCache = &*sc;
            let data_len = (sc.length as i64 * sc.width as i64).max(0) as usize;
            Some(CacheView {
                length: sc.length,
                loopstart: sc.loopstart,
                width: sc.width,
                data: core::slice::from_raw_parts(sc.data.as_ptr(), data_len),
            })
        }
    }
}

/// C: `void S_PaintChannels (int endtime)`
///
/// # Safety
/// Called with snd_mutex held (S_Update_ / cvar paths), sound started
/// (`shm` valid, its buffer locked against the SDL callback).
#[no_mangle]
pub unsafe extern "C" fn S_PaintChannels(endtime: c_int) {
    let st = &mut *MIXER.lock().unwrap();

    // SAFETY: caller contract -- main thread under snd_mutex with the DMA
    // buffer locked; all globals read exactly where the C mixer read them
    unsafe {
        let shm = &*sys::shm;
        let dma_bytes = (shm.samples * (shm.samplebits / 8)).max(0) as usize;
        let dma_buffer = core::slice::from_raw_parts_mut(shm.buffer, dma_bytes);

        let total = sys::total_channels.clamp(0, 1024) as usize;
        let channels_ptr = core::ptr::addr_of_mut!(sys::snd_channels) as *mut Channel;
        let channels = core::slice::from_raw_parts_mut(channels_ptr, total);

        let raw_samples =
            &*(core::ptr::addr_of!(sys::s_rawsamples) as *const [SamplePair; MAX_RAW_SAMPLES]);

        let mut params = PaintParams {
            endtime,
            pause_loops: SND_Glue_PauseLoops() != 0,
            sfxvolume_value: sys::sfxvolume.value,
            sndspeed_value: sys::sndspeed.value,
            filterquality_value: sys::snd_filterquality.value,
            shm_speed: shm.speed,
            shm_samples: shm.samples,
            shm_samplebits: shm.samplebits,
            shm_channels: shm.channels,
            shm_signed8: shm.signed8,
            dma_buffer,
            s_rawend: sys::s_rawend,
            raw_samples,
        };

        let mut loader = EngineSfxSource {
            channels: channels_ptr,
        };

        let mut painted = sys::paintedtime;
        let sndhash = sys::harness_sndhash;
        mix::paint_channels(
            st,
            &mut painted,
            channels,
            &mut loader,
            &mut params,
            |block_start, block_end, paint, dma| {
                if sndhash {
                    // the -sndhash instrument point, identical to the C hook
                    sys::Harness_SndPaint(
                        block_start,
                        block_end,
                        paint.as_ptr().cast(),
                        dma.as_ptr(),
                        dma.len() as c_int,
                    );
                }
            },
        );
        sys::paintedtime = painted;
    }
}

/// C: `void SND_InitScaletable (void)`
///
/// # Safety
/// Reads the sfxvolume cvar; main thread.
#[no_mangle]
pub unsafe extern "C" fn SND_InitScaletable() {
    let st = &mut *MIXER.lock().unwrap();
    // SAFETY: main-thread cvar read
    let vol = unsafe { sys::sfxvolume.value };
    mix::init_scaletable(st, vol);
}

/// C: `void S_SetUnderwaterIntensity (float intensity)` (declared locally by
/// snd_dma.c; called from S_UpdateAmbientSounds)
///
/// # Safety
/// Reads snd_waterfx/host_frametime; main thread.
#[no_mangle]
pub unsafe extern "C" fn S_SetUnderwaterIntensity(target: f32) {
    let st = &mut *MIXER.lock().unwrap();
    // SAFETY: main-thread cvar/global reads
    let (waterfx, frametime) = unsafe { (sys::snd_waterfx.value, sys::host_frametime) };
    mix::set_underwater_intensity(&mut st.underwater, target, waterfx, frametime);
}
