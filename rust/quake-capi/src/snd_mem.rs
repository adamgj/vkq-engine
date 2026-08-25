//! S_LoadSound / GetWavinfo shims (Quake/snd_mem.c, Phase 4 M3).
//!
//! The shims replicate the C originals exactly: the same snd_mutex locking,
//! COM_LoadFile/Mem_Free lifecycle, console messages (byte-identical format
//! strings), the Sys_Error on a bad loop length (diverging call, no unwind
//! -- ADR-009), and Mem_Alloc size arithmetic (ADR-013: the sfxcache_t
//! crosses the boundary and C frees it with SAFE_FREE). Parsing and
//! resampling are the pure `quake-snd` ports.

use core::ffi::{c_char, c_int};

use quake_c_sys as sys;
use quake_snd::resample::{resample_sfx, SfxMeta};
use quake_snd::wav::{get_wavinfo, Msg, WavParse};
use quake_types::sound::{Sfx, SfxCache, WavInfo};

/// Replay the console messages the C GetWavinfo would print, in order, with
/// byte-identical format strings; then the Sys_Error, if any.
///
/// # Safety
/// `name` must be a NUL-terminated C string.
unsafe fn replay_messages(parse: &WavParse, name: *const c_char) {
    for m in &parse.messages {
        // SAFETY: varargs match the C call sites in snd_mem.c exactly
        unsafe {
            match m {
                Msg::BadChunkLen { name: chunk, len } => {
                    // the chunk names are 4-char literals; NUL-terminate
                    let mut buf = [0u8; 5];
                    buf[..4].copy_from_slice(chunk.as_bytes());
                    sys::Con_DPrintf2(
                        c"bad \"%s\" chunk length (%d)\n".as_ptr(),
                        buf.as_ptr(),
                        *len,
                    );
                }
                Msg::MissingRiffWave => {
                    sys::Con_Printf(c"%s missing RIFF/WAVE chunks\n".as_ptr(), name);
                }
                Msg::MissingFmt => {
                    sys::Con_Printf(c"%s is missing fmt chunk\n".as_ptr(), name);
                }
                Msg::NotPcm => {
                    sys::Con_Printf(c"%s is not Microsoft PCM format\n".as_ptr(), name);
                }
                Msg::MissingData => {
                    sys::Con_Printf(c"%s is missing data chunk\n".as_ptr(), name);
                }
                Msg::LoopStartGeEnd => {
                    sys::Con_Warning(c"%s has loop start >= end\n".as_ptr(), name);
                }
            }
        }
    }
    if parse.bad_loop_length {
        // SAFETY: diverging C call; never returns, so no Rust frame unwinds
        unsafe { sys::Sys_Error(c"%s has a bad loop length".as_ptr(), name) };
    }
}

/// C: `wavinfo_t GetWavinfo (const char *name, byte *wav, int wavlength)`
///
/// # Safety
/// `name` NUL-terminated; `wav` NULL or valid for `wavlength` bytes.
#[no_mangle]
pub unsafe extern "C" fn GetWavinfo(
    name: *const c_char,
    wav: *mut u8,
    wavlength: c_int,
) -> WavInfo {
    if wav.is_null() {
        return WavInfo::default();
    }
    // SAFETY: caller contract; a negative length yields an empty slice,
    // matching the C's iff_end < wav "missing RIFF" outcome
    let data = unsafe { core::slice::from_raw_parts(wav, wavlength.max(0) as usize) };
    let parse = get_wavinfo(data);
    // SAFETY: name per caller contract
    unsafe { replay_messages(&parse, name) };
    parse.info
}

/// snd_mem.c's read-bounds clamp: the last source index the resampler
/// touches must lie inside the loaded file (exact float arithmetic).
fn resample_in_bounds(info: &WavInfo, stepscale: f32, file_len: i64) -> bool {
    let last_srcsample: i64 = if stepscale == 1.0 && info.width == 1 {
        info.samples as i64 - 1
    } else {
        let outcount = (info.samples as f32 / stepscale) as i32;
        let fracstep = (stepscale * 256.0) as i32;
        ((outcount as i64 - 1) * fracstep as i64) >> 8
    };
    info.dataofs as i64 + (last_srcsample + 1) * info.width as i64 <= file_len
}

/// C: `sfxcache_t *S_LoadSound (sfx_t *s)`
///
/// # Safety
/// `s` must point at a live sfx_t whose name is NUL-terminated; called on
/// the main thread (or with snd_mutex protecting the sfx cache, as in C).
#[no_mangle]
pub unsafe extern "C" fn S_LoadSound(s: *mut Sfx) -> *mut SfxCache {
    // SAFETY: NULL-tolerant recursive engine mutex, exactly the C locking
    unsafe { sys::QMutex_Lock(sys::snd_mutex) };

    let mut sc: *mut SfxCache = core::ptr::null_mut();
    let mut data: *mut u8 = core::ptr::null_mut();

    'done: {
        // SAFETY: caller contract
        let sfx = unsafe { &mut *s };

        // see if still in memory
        if !sfx.cache.is_null() {
            sc = sfx.cache;
            break 'done;
        }

        // load it in: "sound/" + name, q_strlcpy/q_strlcat over a 256 buffer
        let mut namebuffer = [0u8; 256];
        namebuffer[..6].copy_from_slice(b"sound/");
        let name_len = sfx.name.iter().position(|&c| c == 0).unwrap_or(64);
        let take = name_len.min(256 - 6 - 1);
        for (i, &c) in sfx.name[..take].iter().enumerate() {
            namebuffer[6 + i] = c as u8;
        }

        // SAFETY: namebuffer is NUL-terminated
        data = unsafe { sys::COM_LoadFile(namebuffer.as_ptr().cast(), core::ptr::null_mut()) };
        if data.is_null() {
            // SAFETY: byte-identical to the C message
            unsafe { sys::Con_Printf(c"Couldn't load %s\n".as_ptr(), namebuffer.as_ptr()) };
            break 'done;
        }

        // SAFETY: the engine's com_filesize is THREAD_LOCAL; the accessor is
        // the only portable way to read it from Rust (truncated to int like C)
        let filesize = unsafe { sys::COM_ThreadFileSize() } as c_int;
        // SAFETY: COM_LoadFile returned filesize readable bytes
        let file = unsafe { core::slice::from_raw_parts(data, filesize.max(0) as usize) };
        let parse = get_wavinfo(file);
        // SAFETY: sfx.name is NUL-terminated per caller contract
        unsafe { replay_messages(&parse, sfx.name.as_ptr()) };
        let info = parse.info;

        // SAFETY: message varargs match snd_mem.c
        unsafe {
            if info.channels != 1 {
                sys::Con_Printf(c"%s is a stereo sample\n".as_ptr(), sfx.name.as_ptr());
                break 'done;
            }
            if info.width != 1 && info.width != 2 {
                sys::Con_Printf(c"%s is not 8 or 16 bit\n".as_ptr(), sfx.name.as_ptr());
                break 'done;
            }
            if info.rate <= 0 {
                sys::Con_Printf(
                    c"%s has an invalid sample rate\n".as_ptr(),
                    sfx.name.as_ptr(),
                );
                break 'done;
            }
        }

        // SAFETY: shm is set before sounds load (S_Startup); read like C
        let shm_speed = unsafe { (*sys::shm).speed };
        let stepscale = info.rate as f32 / shm_speed as f32;
        let mut len = (info.samples as f32 / stepscale) as i32;
        len = len.wrapping_mul(info.width * info.channels);

        if info.samples == 0 || len == 0 {
            // SAFETY: byte-identical message
            unsafe { sys::Con_Printf(c"%s has zero samples\n".as_ptr(), sfx.name.as_ptr()) };
            break 'done;
        }

        if !resample_in_bounds(&info, stepscale, filesize as i64) {
            // SAFETY: byte-identical message
            unsafe { sys::Con_Printf(c"%s has a bad data length\n".as_ptr(), sfx.name.as_ptr()) };
            break 'done;
        }

        // SAFETY: engine mimalloc (ADR-013); size arithmetic identical to C
        // (a negative len goes through the same huge-size_t failing alloc)
        let alloc = unsafe {
            sys::Mem_Alloc((len as isize as usize).wrapping_add(core::mem::size_of::<SfxCache>()))
        };
        if alloc.is_null() {
            break 'done;
        }
        sc = alloc.cast();
        // SAFETY: alloc is at least sizeof(sfxcache_t), zero-initialized
        let cache = unsafe { &mut *sc };
        cache.length = info.samples;
        cache.loopstart = info.loopstart;
        cache.speed = info.rate;
        cache.width = info.width;
        cache.stereo = info.channels;

        sfx.cache = sc;

        // ResampleSfx (s, sc->speed, sc->width, data + info.dataofs): the
        // source is the rest of the loaded file (reads bounded by the clamp
        // above), the destination the cache's data area
        let meta = SfxMeta {
            length: cache.length,
            loopstart: cache.loopstart,
            speed: cache.speed,
            width: cache.width,
            stereo: cache.stereo,
        };
        let pcm = &file[(info.dataofs as usize).min(file.len())..];
        // SAFETY: the allocation holds len bytes after the header
        let out = unsafe {
            core::slice::from_raw_parts_mut(cache.data.as_mut_ptr(), len.max(0) as usize)
        };
        // SAFETY: main-thread cvar read under snd_mutex
        let loadas8bit = unsafe { sys::loadas8bit.value != 0.0 };
        let new_meta = resample_sfx(meta, info.rate, info.width, shm_speed, loadas8bit, pcm, out);
        cache.length = new_meta.length;
        cache.loopstart = new_meta.loopstart;
        cache.speed = new_meta.speed;
        cache.width = new_meta.width;
        cache.stereo = new_meta.stereo;
    }

    // SAFETY: Mem_Free is NULL-tolerant, mirroring the C epilogue exactly
    unsafe {
        sys::Mem_Free(data.cast());
        sys::QMutex_Unlock(sys::snd_mutex);
    }
    sc
}
