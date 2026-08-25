//! The SDL3 audio backend (Quake/snd_sdl3.c, Phase 4 M9, ADR-017): the six
//! SNDDMA_* entry points over SDL_OpenAudioDeviceStream's callback model,
//! through the `sdl3` crate's sys layer so every call keeps the C shape.
//!
//! `paint_audio` runs on the SDL audio thread: it only copies out of
//! `shm->buffer` and advances `samplepos`, under the stream lock the mixer
//! holds across paints (SNDDMA_LockBuffer/Submit) -- the same discipline as
//! the C. It is panic-free and allocation-free.

use core::ffi::{c_char, c_int, c_void};

use quake_c_sys as sys;
use sdl3::sys::audio::{
    SDL_AudioSpec, SDL_AudioStream, SDL_DestroyAudioStream, SDL_GetAudioDeviceName,
    SDL_GetAudioStreamDevice, SDL_GetCurrentAudioDriver, SDL_LockAudioStream,
    SDL_OpenAudioDeviceStream, SDL_PauseAudioStreamDevice, SDL_PutAudioStreamData,
    SDL_ResumeAudioStreamDevice, SDL_UnlockAudioStream, SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK,
    SDL_AUDIO_S16, SDL_AUDIO_U8,
};
use sdl3::sys::error::SDL_GetError;
use sdl3::sys::init::{SDL_InitSubSystem, SDL_QuitSubSystem, SDL_INIT_AUDIO};

// SAFETY invariant: written on the main thread in init/shutdown, read by the
// audio callback only while the device is running -- the same lifetime the C
// static had.
static mut AUDIO_STREAM: *mut SDL_AudioStream = core::ptr::null_mut();
static mut BUFFERSIZE: c_int = 0;

/// mathlib.h Q_nextPow2 (result == N when N is a power of 2)
fn next_pow2(val: u32) -> u32 {
    if val > 1 {
        1u32 << (31 - (val - 1).leading_zeros() + 1)
    } else {
        1
    }
}

/// the SDL audio callback (C: paint_audio)
unsafe extern "C" fn paint_audio(
    _userdata: *mut c_void,
    stream: *mut SDL_AudioStream,
    additional_amount: c_int,
    _total_amount: c_int,
) {
    // SAFETY: shm points at snd_glue.c's sn while sound runs; the engine
    // holds the stream lock whenever it touches buffer/samplepos, and SDL
    // holds it around this callback
    unsafe {
        if sys::shm.is_null() || additional_amount <= 0 {
            return;
        }
        let shm = &mut *sys::shm;

        let mut pos = shm.samplepos * (shm.samplebits / 8);
        if pos >= BUFFERSIZE {
            shm.samplepos = 0;
            pos = 0;
        }

        let tobufend = BUFFERSIZE - pos;
        let mut len1 = additional_amount;
        let mut len2 = 0;

        if len1 > tobufend {
            len1 = tobufend;
            len2 = additional_amount - len1;
        }

        SDL_PutAudioStreamData(stream, shm.buffer.add(pos as usize).cast(), len1);

        if len2 > 0 {
            SDL_PutAudioStreamData(stream, shm.buffer.cast(), len2);
            shm.samplepos = len2 / (shm.samplebits / 8);
        } else {
            shm.samplepos += len1 / (shm.samplebits / 8);
        }

        if shm.samplepos >= shm.samples {
            shm.samplepos = 0;
        }
    }
}

/// C: `qboolean SNDDMA_Init (dma_t *dma)`
///
/// # Safety
/// Main thread; `dma` is snd_glue.c's `sn`.
pub unsafe fn snddma_init(dma: *mut sys::dma_t) -> bool {
    // SAFETY: mirrors snd_sdl3.c's init sequence and messages exactly
    unsafe {
        if !SDL_InitSubSystem(SDL_INIT_AUDIO) {
            sys::Con_Printf(c"Couldn't init SDL audio: %s\n".as_ptr(), SDL_GetError());
            return false;
        }

        // Set up the desired format
        let spec = SDL_AudioSpec {
            freq: sys::snd_mixspeed.value as c_int,
            format: if sys::loadas8bit.value != 0.0 {
                SDL_AUDIO_U8
            } else {
                SDL_AUDIO_S16
            },
            channels: 2,
        };

        // Open the audio device with callback
        AUDIO_STREAM = SDL_OpenAudioDeviceStream(
            SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK,
            &spec,
            Some(paint_audio),
            core::ptr::null_mut(),
        );
        if AUDIO_STREAM.is_null() {
            sys::Con_Printf(c"Couldn't open SDL audio: %s\n".as_ptr(), SDL_GetError());
            SDL_QuitSubSystem(SDL_INIT_AUDIO);
            return false;
        }

        core::ptr::write_bytes(dma, 0, 1);
        sys::shm = dma;
        let shm = &mut *sys::shm;

        // Fill the audio DMA information block
        // (SDL_AUDIO_BITSIZE: low byte of the format value)
        shm.samplebits = (spec.format.0 as c_int) & 0xFF;
        shm.signed8 = 0; // spec.format is U8 or S16, never S8
        shm.speed = spec.freq;
        shm.channels = spec.channels;

        // Calculate buffer size - aim for ~100ms of audio
        let num_samples = (spec.channels * spec.freq) / 10;
        let num_samples = next_pow2(num_samples as u32) as c_int;

        shm.samples = num_samples;
        shm.samplepos = 0;
        shm.submission_chunk = 1;

        sys::Con_Printf(
            c"SDL audio spec  : %d Hz, %d channels\n".as_ptr(),
            spec.freq,
            spec.channels as c_int,
        );
        let driver = SDL_GetCurrentAudioDriver();
        let device = SDL_GetAudioDeviceName(SDL_GetAudioStreamDevice(AUDIO_STREAM));
        BUFFERSIZE = shm.samples * (shm.samplebits / 8);
        sys::Con_Printf(
            c"SDL audio driver: %s - %s, %d bytes buffer\n".as_ptr(),
            if driver.is_null() {
                c"(UNKNOWN)".as_ptr()
            } else {
                driver
            },
            if device.is_null() {
                c"(UNKNOWN)".as_ptr()
            } else {
                device
            },
            BUFFERSIZE,
        );

        shm.buffer = sys::Mem_Alloc(BUFFERSIZE as usize).cast();
        if shm.buffer.is_null() {
            SDL_DestroyAudioStream(AUDIO_STREAM);
            AUDIO_STREAM = core::ptr::null_mut();
            SDL_QuitSubSystem(SDL_INIT_AUDIO);
            sys::shm = core::ptr::null_mut();
            sys::Con_Printf(c"Failed allocating memory for SDL audio\n".as_ptr());
            return false;
        }

        SDL_ResumeAudioStreamDevice(AUDIO_STREAM);

        sys::Con_Printf(
            c"SDL audio initialized: samples=%d, samplebits=%d, channels=%d\n".as_ptr(),
            shm.samples,
            shm.samplebits,
            shm.channels,
        );

        true
    }
}

/// C: `int SNDDMA_GetDMAPos (void)`
///
/// # Safety
/// Sound started (`shm` valid); called under the stream lock.
pub unsafe fn snddma_get_dma_pos() -> c_int {
    // SAFETY: caller contract
    unsafe { (*sys::shm).samplepos }
}

/// C: `void SNDDMA_Shutdown (void)`
///
/// # Safety
/// Main thread.
pub unsafe fn snddma_shutdown() {
    // SAFETY: mirrors the C shutdown; the callback is destroyed before the
    // buffer is freed
    unsafe {
        if !sys::shm.is_null() {
            sys::Con_Printf(c"Shutting down SDL sound\n".as_ptr());
            SDL_DestroyAudioStream(AUDIO_STREAM);
            AUDIO_STREAM = core::ptr::null_mut();
            SDL_QuitSubSystem(SDL_INIT_AUDIO);
            let shm = &mut *sys::shm;
            if !shm.buffer.is_null() {
                sys::Mem_Free(shm.buffer.cast());
            }
            shm.buffer = core::ptr::null_mut();
            sys::shm = core::ptr::null_mut();
        }
    }
}

/// C: `void SNDDMA_LockBuffer (void)`
///
/// # Safety
/// Main thread.
pub unsafe fn snddma_lock_buffer() {
    // SAFETY: NULL-checked stream lock, like the C
    unsafe {
        if !AUDIO_STREAM.is_null() {
            SDL_LockAudioStream(AUDIO_STREAM);
        }
    }
}

/// C: `void SNDDMA_Submit (void)` (in the callback model, unlock is all)
///
/// # Safety
/// Main thread, after SNDDMA_LockBuffer.
pub unsafe fn snddma_submit() {
    // SAFETY: as above
    unsafe {
        if !AUDIO_STREAM.is_null() {
            SDL_UnlockAudioStream(AUDIO_STREAM);
        }
    }
}

/// C: `void SNDDMA_BlockSound (void)`
///
/// # Safety
/// Main thread.
pub unsafe fn snddma_block_sound() {
    // SAFETY: as above
    unsafe {
        if !AUDIO_STREAM.is_null() {
            SDL_PauseAudioStreamDevice(AUDIO_STREAM);
        }
    }
}

/// C: `void SNDDMA_UnblockSound (void)`
///
/// # Safety
/// Main thread.
pub unsafe fn snddma_unblock_sound() {
    // SAFETY: as above
    unsafe {
        if !AUDIO_STREAM.is_null() {
            SDL_ResumeAudioStreamDevice(AUDIO_STREAM);
        }
    }
}

// keep c_char referenced (message pointers above are c_char)
#[allow(unused)]
fn _sig(_: *const c_char) {}
