//! Rust-side engine-global stand-ins for the quake-capi `snd` shims in the
//! test binaries (Phase 4 M7).
//!
//! In the engine these symbols come from snd_glue.c / common.c; here the
//! c_ref sound subsystem owns its *renamed* copies (see c_ref_prelude.h), so
//! the unrenamed names the Rust shims import are defined here. Only the
//! codec framework and tag-skipping paths are exercised through the shims in
//! tests, so most of these are inert storage.

#![allow(non_upper_case_globals, missing_docs)]

use core::ffi::{c_char, c_int, c_void};

use quake_c_sys::{cvar_t, dma_t, qboolean, qmutex_t, snd_codec_t};
use quake_types::sound::{Channel, SamplePair, MAX_CHANNELS, MAX_RAW_SAMPLES};

// snd_glue.c storage stand-ins
#[no_mangle]
pub static mut snd_channels: [Channel; MAX_CHANNELS] = [Channel {
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
}; MAX_CHANNELS];
#[no_mangle]
pub static mut total_channels: c_int = 0;
#[no_mangle]
pub static mut soundtime: c_int = 0;
#[no_mangle]
pub static mut paintedtime: c_int = 0;
#[no_mangle]
pub static mut s_rawend: c_int = 0;
#[no_mangle]
pub static mut s_rawsamples: [SamplePair; MAX_RAW_SAMPLES] =
    [SamplePair { left: 0, right: 0 }; MAX_RAW_SAMPLES];
#[no_mangle]
pub static mut listener_origin: [f32; 3] = [0.0; 3];
#[no_mangle]
pub static mut listener_forward: [f32; 3] = [0.0; 3];
#[no_mangle]
pub static mut listener_right: [f32; 3] = [0.0; 3];
#[no_mangle]
pub static mut listener_up: [f32; 3] = [0.0; 3];
#[no_mangle]
pub static mut snd_mutex: *mut qmutex_t = core::ptr::null_mut();
#[no_mangle]
pub static mut shm: *mut dma_t = core::ptr::null_mut();
#[no_mangle]
pub static mut sn: dma_t = dma_t {
    channels: 0,
    samples: 0,
    submission_chunk: 0,
    samplepos: 0,
    samplebits: 0,
    signed8: 0,
    speed: 0,
    buffer: core::ptr::null_mut(),
};
#[no_mangle]
pub static mut safemode: c_int = 0;

const fn cvar(name: &'static core::ffi::CStr, string: &'static core::ffi::CStr) -> cvar_t {
    cvar_t {
        name: name.as_ptr(),
        string: string.as_ptr(),
        flags: 0,
        value: 0.0,
        default_string: core::ptr::null(),
        callback: None,
        completion: None,
        next: core::ptr::null_mut(),
    }
}

#[no_mangle]
pub static mut bgmvolume: cvar_t = cvar(c"bgmvolume", c"1");
#[no_mangle]
pub static mut sfxvolume: cvar_t = cvar(c"volume", c"0.7");
#[no_mangle]
pub static mut precache: cvar_t = cvar(c"precache", c"1");
#[no_mangle]
pub static mut loadas8bit: cvar_t = cvar(c"loadas8bit", c"0");
#[no_mangle]
pub static mut sndspeed: cvar_t = cvar(c"sndspeed", c"11025");
#[no_mangle]
pub static mut snd_mixspeed: cvar_t = cvar(c"snd_mixspeed", c"44100");
#[no_mangle]
pub static mut snd_waterfx: cvar_t = cvar(c"snd_waterfx", c"1");
#[no_mangle]
pub static mut snd_pauselooping: cvar_t = cvar(c"snd_pauselooping", c"1");
#[no_mangle]
pub static mut snd_filterquality: cvar_t = cvar(c"snd_filterquality", c"1");
#[no_mangle]
pub static mut nosound: cvar_t = cvar(c"nosound", c"0");
#[no_mangle]
pub static mut ambient_level: cvar_t = cvar(c"ambient_level", c"0.3");
#[no_mangle]
pub static mut ambient_fade: cvar_t = cvar(c"ambient_fade", c"100");
#[no_mangle]
pub static mut snd_noextraupdate: cvar_t = cvar(c"snd_noextraupdate", c"0");
#[no_mangle]
pub static mut snd_show: cvar_t = cvar(c"snd_show", c"0");
#[no_mangle]
pub static mut _snd_mixahead: cvar_t = cvar(c"_snd_mixahead", c"0.1");

// snd_glue.c accessors
#[no_mangle]
pub extern "C" fn SND_Glue_PauseLoops() -> qboolean {
    false
}
#[no_mangle]
pub extern "C" fn SND_Glue_ClientConnected() -> qboolean {
    false
}
#[no_mangle]
pub extern "C" fn SND_Glue_ViewEntity() -> c_int {
    0
}
#[no_mangle]
pub extern "C" fn SND_Glue_Worldmodel() -> *mut c_void {
    core::ptr::null_mut()
}
/// # Safety
/// stub; never dereferences p
#[no_mangle]
pub unsafe extern "C" fn SND_Glue_PointInLeaf(_p: *mut f32) -> *mut c_void {
    core::ptr::null_mut()
}

// A dummy mp3 decoder vtable for the framework differential: the Rust
// registry (codec-mp3 feature) references `mp3_codec`, and the stub table in
// stubs.c provides the identical c_ref_mp3_codec for the C side.
unsafe extern "C" fn dummy_init() -> qboolean {
    true
}
unsafe extern "C" fn dummy_shutdown() {}
unsafe extern "C" fn dummy_open(_s: *mut quake_c_sys::snd_stream_t) -> qboolean {
    false
}
unsafe extern "C" fn dummy_read(
    _s: *mut quake_c_sys::snd_stream_t,
    _b: c_int,
    _buf: *mut c_void,
) -> c_int {
    0
}
unsafe extern "C" fn dummy_rewind(_s: *mut quake_c_sys::snd_stream_t) -> c_int {
    -1
}
unsafe extern "C" fn dummy_close(_s: *mut quake_c_sys::snd_stream_t) {}

#[no_mangle]
pub static mut mp3_codec: snd_codec_t = snd_codec_t {
    type_: 1 << 4, // CODECTYPE_MP3
    initialized: true,
    ext: {
        const E: &core::ffi::CStr = c"mp3";
        E.as_ptr()
    },
    initialize: Some(dummy_init),
    shutdown: Some(dummy_shutdown),
    codec_open: Some(dummy_open),
    codec_read: Some(dummy_read),
    codec_rewind: Some(dummy_rewind),
    codec_jump: None,
    codec_close: Some(dummy_close),
    next: core::ptr::null_mut(),
};

// referenced so the module is never dead-stripped wholesale
pub fn _touch(_: *const c_char) {}
