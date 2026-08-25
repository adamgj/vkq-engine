//! Background music control (Quake/bgmusic.c, Phase 4 M8): the handler
//! table (exact order, MOD entries kept and permanently unavailable -- no
//! MOD codec exists, per ADR-018), stream open/CD-rip resolution, and the
//! 16 KiB staging loop feeding S_RawSamples. bgm_extmusic's cvar_t storage
//! lives in snd_glue.c (menu compat surface).

use core::ffi::{c_char, c_int, c_uint};

use quake_c_sys as sys;
use sys::{qboolean, snd_stream_t};

extern "C" {
    // cdaudio.h; cd_null.c stays compiled either way
    fn CDAudio_Play(track: u8, looping: qboolean) -> c_int;
    // snd_glue.c storage
    static mut bgm_extmusic: sys::cvar_t;
}

#[allow(dead_code)]
const CODECTYPE_NONE: c_uint = 0;
const CODECTYPE_MOD: c_uint = 1 << 1;
const CODECTYPE_FLAC: c_uint = 1 << 2;
const CODECTYPE_WAV: c_uint = 1 << 3;
const CODECTYPE_MP3: c_uint = 1 << 4;
const CODECTYPE_VORBIS: c_uint = 1 << 5;
const CODECTYPE_OPUS: c_uint = 1 << 6;
const CODECTYPE_UMX: c_uint = 1 << 7;

const ANY_CODECTYPE: c_uint = 0xFFFFFFFF;

const MAX_RAW_SAMPLES: c_int = 8192;

struct Handler {
    type_: c_uint,
    /// -1 means not present (C is_available)
    is_available: c_int,
    ext: &'static core::ffi::CStr,
}

struct BgmState {
    bgmloop: bool,
    no_extmusic: bool,
    old_volume: f32,
    /// wanted_handlers with runtime availability; `active` holds the indices
    /// the C linked into music_handlers (order preserved)
    handlers: [Handler; 10],
    active: Vec<usize>,
    initialized: bool,
    stream: *mut snd_stream_t,
}

// SAFETY invariant: main-thread access only, like the C file statics.
static mut BGM: BgmState = BgmState {
    bgmloop: false,
    no_extmusic: false,
    old_volume: -1.0,
    handlers: [
        Handler {
            type_: CODECTYPE_VORBIS,
            is_available: -1,
            ext: c"ogg",
        },
        Handler {
            type_: CODECTYPE_OPUS,
            is_available: -1,
            ext: c"opus",
        },
        Handler {
            type_: CODECTYPE_MP3,
            is_available: -1,
            ext: c"mp3",
        },
        Handler {
            type_: CODECTYPE_FLAC,
            is_available: -1,
            ext: c"flac",
        },
        Handler {
            type_: CODECTYPE_WAV,
            is_available: -1,
            ext: c"wav",
        },
        Handler {
            type_: CODECTYPE_MOD,
            is_available: -1,
            ext: c"it",
        },
        Handler {
            type_: CODECTYPE_MOD,
            is_available: -1,
            ext: c"s3m",
        },
        Handler {
            type_: CODECTYPE_MOD,
            is_available: -1,
            ext: c"xm",
        },
        Handler {
            type_: CODECTYPE_MOD,
            is_available: -1,
            ext: c"mod",
        },
        Handler {
            type_: CODECTYPE_UMX,
            is_available: -1,
            ext: c"umx",
        },
    ],
    active: Vec::new(),
    initialized: false,
    stream: core::ptr::null_mut(),
};

#[allow(static_mut_refs)]
fn state() -> &'static mut BgmState {
    // SAFETY: main-thread discipline (see BGM)
    unsafe { &mut BGM }
}

fn eq_ignore_ascii(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|(x, y)| x.eq_ignore_ascii_case(y))
}

/// C: `qboolean BGM_Init (void)`
///
/// # Safety
/// Engine entry point; main thread, after S_Init.
#[no_mangle]
pub unsafe extern "C" fn BGM_Init() -> qboolean {
    let st = state();

    // SAFETY: registration mirrors bgmusic.c; cvar storage in snd_glue.c
    unsafe {
        sys::Cvar_RegisterVariable(core::ptr::addr_of_mut!(bgm_extmusic));
        sys::Cmd_AddCommand2(
            c"music".as_ptr(),
            Some(bgm_play_f),
            sys::cmd_source_t_src_command,
            false,
        );
        sys::Cmd_AddCommand2(
            c"music_pause".as_ptr(),
            Some(bgm_pause_f),
            sys::cmd_source_t_src_command,
            false,
        );
        sys::Cmd_AddCommand2(
            c"music_resume".as_ptr(),
            Some(bgm_resume_f),
            sys::cmd_source_t_src_command,
            false,
        );
        sys::Cmd_AddCommand2(
            c"music_loop".as_ptr(),
            Some(bgm_loop_f),
            sys::cmd_source_t_src_command,
            false,
        );
        sys::Cmd_AddCommand2(
            c"music_stop".as_ptr(),
            Some(bgm_stop_f),
            sys::cmd_source_t_src_command,
            false,
        );
        sys::Cmd_AddCommand2(
            c"music_jump".as_ptr(),
            Some(bgm_jump_f),
            sys::cmd_source_t_src_command,
            false,
        );

        if sys::COM_CheckParm(c"-noextmusic".as_ptr()) != 0 {
            st.no_extmusic = true;
        }

        st.bgmloop = true;

        st.active.clear();
        for i in 0..st.handlers.len() {
            // all entries are BGM_STREAMER; MIDIDRV is not supported in quake
            st.handlers[i].is_available =
                crate::snd_codec::S_CodecIsAvailable(st.handlers[i].type_);
            if st.handlers[i].is_available != -1 {
                st.active.push(i);
            }
        }
        st.initialized = true;
    }
    true
}

/// C: `void BGM_Shutdown (void)`
///
/// # Safety
/// Engine entry point.
#[no_mangle]
pub unsafe extern "C" fn BGM_Shutdown() {
    // SAFETY: forwarded
    unsafe { BGM_Stop() };
    // sever our connections to snd_codec
    let st = state();
    st.active.clear();
    st.initialized = false;
}

/// BGM_Play_noext (file-internal in C)
unsafe fn play_noext(filename: &[u8], allowed_types: c_uint) {
    let st = state();
    // SAFETY: open calls + message match the C
    unsafe {
        for &i in st.active.iter() {
            let h = &st.handlers[i];
            if h.type_ & allowed_types == 0 {
                continue;
            }
            if h.is_available == 0 {
                continue;
            }
            // q_snprintf (tmp, MAX_QPATH, "%s/%s.%s", dir, filename, ext)
            let mut tmp = [0u8; 64];
            let ext = h.ext.to_bytes();
            for (n, &b) in b"music/"
                .iter()
                .chain(filename.iter())
                .chain(b".".iter())
                .chain(ext.iter())
                .take(63)
                .enumerate()
            {
                tmp[n] = b;
            }
            st.stream =
                crate::snd_codec::S_CodecOpenStreamType(tmp.as_ptr().cast(), h.type_, st.bgmloop);
            if !st.stream.is_null() {
                return; // success
            }
        }
        sys::Con_Printf(
            c"Couldn't handle music file %s\n".as_ptr(),
            filename.as_ptr(),
        );
    }
}

/// C: `void BGM_Play (const char *filename)`
///
/// # Safety
/// `filename` NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn BGM_Play(filename: *const c_char) {
    let st = state();
    // SAFETY: mirrors bgmusic.c incl. messages
    unsafe {
        BGM_Stop();

        if st.active.is_empty() {
            return;
        }

        if filename.is_null() || *filename == 0 {
            sys::Con_DPrintf(c"null music file name\n".as_ptr());
            return;
        }
        let nameb = core::ffi::CStr::from_ptr(filename).to_bytes();

        let ext = sys::COM_FileGetExtension(filename);
        if *ext == 0 {
            // try all things
            play_noext(nameb, ANY_CODECTYPE);
            return;
        }
        let extb = core::ffi::CStr::from_ptr(ext).to_bytes();

        let mut found: Option<usize> = None;
        for &i in st.active.iter() {
            let h = &st.handlers[i];
            if h.is_available != 0 && eq_ignore_ascii(extb, h.ext.to_bytes()) {
                found = Some(i);
                break;
            }
        }
        let Some(i) = found else {
            sys::Con_Printf(c"Unhandled extension for %s\n".as_ptr(), filename);
            return;
        };
        let mut tmp = [0u8; 64];
        for (n, &b) in b"music/".iter().chain(nameb.iter()).take(63).enumerate() {
            tmp[n] = b;
        }
        st.stream = crate::snd_codec::S_CodecOpenStreamType(
            tmp.as_ptr().cast(),
            st.handlers[i].type_,
            st.bgmloop,
        );
        if !st.stream.is_null() {
            return; // success
        }

        sys::Con_Printf(c"Couldn't handle music file %s\n".as_ptr(), filename);
    }
}

/// C: `void BGM_PlayCDtrack (byte track, qboolean looping)`
///
/// # Safety
/// Engine entry point.
#[no_mangle]
pub unsafe extern "C" fn BGM_PlayCDtrack(track: u8, looping: qboolean) {
    let st = state();
    // SAFETY: mirrors bgmusic.c (searchpath-priority cdrip resolution)
    unsafe {
        BGM_Stop();
        if CDAudio_Play(track, looping) == 0 {
            return; // success
        }

        if st.active.is_empty() {
            return;
        }

        if st.no_extmusic || bgm_extmusic.value == 0.0 {
            return;
        }

        let mut prev_id: c_uint = 0;
        let mut type_: c_uint = 0;
        let mut ext: Option<&'static core::ffi::CStr> = None;
        for &i in st.active.iter() {
            let h = &st.handlers[i];
            if h.is_available == 0 {
                continue;
            }
            let name = format!(
                "music/track{:02}.{}\0",
                track as c_int,
                h.ext.to_str().unwrap()
            );
            let mut path_id: c_uint = 0;
            if !sys::COM_FileExists(name.as_ptr().cast(), &mut path_id) {
                continue;
            }
            if path_id > prev_id {
                prev_id = path_id;
                type_ = h.type_;
                ext = Some(h.ext);
            }
        }
        match ext {
            None => {
                sys::Con_Printf(
                    c"Couldn't find a cdrip for track %d\n".as_ptr(),
                    track as c_int,
                );
            }
            Some(e) => {
                let name = format!("music/track{:02}.{}\0", track as c_int, e.to_str().unwrap());
                st.stream = crate::snd_codec::S_CodecOpenStreamType(
                    name.as_ptr().cast(),
                    type_,
                    st.bgmloop,
                );
                if st.stream.is_null() {
                    sys::Con_Printf(c"Couldn't handle music file %s\n".as_ptr(), name.as_ptr());
                }
            }
        }
    }
}

/// C: `void BGM_Stop (void)`
///
/// # Safety
/// Engine entry point.
#[no_mangle]
pub unsafe extern "C" fn BGM_Stop() {
    let st = state();
    if !st.stream.is_null() {
        // SAFETY: open stream owned by this module
        unsafe {
            (*st.stream).status = sys::stream_status_t_STREAM_NONE;
            crate::snd_codec::S_CodecCloseStream(st.stream);
            st.stream = core::ptr::null_mut();
            sys::s_rawend = 0;
        }
    }
}

/// C: `void BGM_Pause (void)`
///
/// # Safety
/// Engine entry point.
#[no_mangle]
pub unsafe extern "C" fn BGM_Pause() {
    let st = state();
    if !st.stream.is_null() {
        // SAFETY: stream owned by this module
        unsafe {
            if (*st.stream).status == sys::stream_status_t_STREAM_PLAY {
                (*st.stream).status = sys::stream_status_t_STREAM_PAUSE;
            }
        }
    }
}

/// C: `void BGM_Resume (void)`
///
/// # Safety
/// Engine entry point.
#[no_mangle]
pub unsafe extern "C" fn BGM_Resume() {
    let st = state();
    if !st.stream.is_null() {
        // SAFETY: stream owned by this module
        unsafe {
            if (*st.stream).status == sys::stream_status_t_STREAM_PAUSE {
                (*st.stream).status = sys::stream_status_t_STREAM_PLAY;
            }
        }
    }
}

/// BGM_UpdateStream (file-internal in C): the 16 KiB staging loop.
unsafe fn update_stream() {
    let st = state();
    // SAFETY: stream/raw-buffer plumbing mirrors bgmusic.c byte for byte
    unsafe {
        let mut did_rewind = false;
        let mut raw = [0u8; 16384];

        if (*st.stream).status != sys::stream_status_t_STREAM_PLAY {
            return;
        }

        // don't bother playing anything if musicvolume is 0
        if sys::bgmvolume.value <= 0.0 {
            return;
        }

        // see how many samples should be copied into the raw buffer
        if sys::s_rawend < sys::paintedtime {
            sys::s_rawend = sys::paintedtime;
        }

        while sys::s_rawend < sys::paintedtime + MAX_RAW_SAMPLES {
            let info = (*st.stream).info;
            let buffer_samples = MAX_RAW_SAMPLES - (sys::s_rawend - sys::paintedtime);

            // decide how much data needs to be read from the file
            let mut file_samples = buffer_samples * info.rate / (*sys::shm).speed;
            if file_samples == 0 {
                return;
            }

            // our max buffer size
            let mut file_bytes = file_samples * (info.width * info.channels);
            if file_bytes > raw.len() as c_int {
                file_bytes = raw.len() as c_int;
                file_samples = file_bytes / (info.width * info.channels);
            }

            // Read
            let res =
                crate::snd_codec::S_CodecReadStream(st.stream, file_bytes, raw.as_mut_ptr().cast());
            if res < file_bytes {
                file_samples = res / (info.width * info.channels);
            }

            if res > 0 {
                // data: add to raw buffer
                crate::snd_dma::S_RawSamples(
                    file_samples,
                    info.rate,
                    info.width,
                    info.channels,
                    raw.as_mut_ptr(),
                    sys::bgmvolume.value,
                );
                did_rewind = false;
            } else if res == 0 {
                // EOF
                if st.bgmloop {
                    if did_rewind {
                        sys::Con_Printf(c"Stream keeps returning EOF.\n".as_ptr());
                        BGM_Stop();
                        return;
                    }

                    let res = crate::snd_codec::S_CodecRewindStream(st.stream);
                    if res != 0 {
                        sys::Con_Printf(c"Stream seek error (%i), stopping.\n".as_ptr(), res);
                        BGM_Stop();
                        return;
                    }
                    did_rewind = true;
                } else {
                    BGM_Stop();
                    return;
                }
            } else {
                // res < 0: some read error
                sys::Con_Printf(c"Stream read error (%i), stopping.\n".as_ptr(), res);
                BGM_Stop();
                return;
            }
        }
    }
}

/// C: `void BGM_Update (void)`
///
/// # Safety
/// Engine entry point; main thread each frame.
#[no_mangle]
pub unsafe extern "C" fn BGM_Update() {
    let st = state();
    // SAFETY: cvar clamp + stream update, mirroring the C
    unsafe {
        if st.old_volume != sys::bgmvolume.value {
            if sys::bgmvolume.value < 0.0 {
                sys::Cvar_SetQuick(core::ptr::addr_of_mut!(sys::bgmvolume), c"0".as_ptr());
            } else if sys::bgmvolume.value > 1.0 {
                sys::Cvar_SetQuick(core::ptr::addr_of_mut!(sys::bgmvolume), c"1".as_ptr());
            }
            st.old_volume = sys::bgmvolume.value;
        }
        if !st.stream.is_null() {
            update_stream();
        }
    }
}

// ---------------------------------------------------------------------------
// console commands

unsafe extern "C" fn bgm_play_f() {
    // SAFETY: command trampoline
    unsafe {
        if sys::Cmd_Argc() == 2 {
            BGM_Play(sys::Cmd_Argv(1));
        } else {
            sys::Con_Printf(c"music <musicfile>\n".as_ptr());
        }
    }
}

unsafe extern "C" fn bgm_pause_f() {
    // SAFETY: command trampoline
    unsafe { BGM_Pause() };
}

unsafe extern "C" fn bgm_resume_f() {
    // SAFETY: command trampoline
    unsafe { BGM_Resume() };
}

unsafe extern "C" fn bgm_loop_f() {
    let st = state();
    // SAFETY: command trampoline; string compares are ASCII
    unsafe {
        if sys::Cmd_Argc() == 2 {
            let arg = core::ffi::CStr::from_ptr(sys::Cmd_Argv(1)).to_bytes();
            if eq_ignore_ascii(arg, b"0") || eq_ignore_ascii(arg, b"off") {
                st.bgmloop = false;
            } else if eq_ignore_ascii(arg, b"1") || eq_ignore_ascii(arg, b"on") {
                st.bgmloop = true;
            } else if eq_ignore_ascii(arg, b"toggle") {
                st.bgmloop = !st.bgmloop;
            }

            if !st.stream.is_null() {
                (*st.stream).loop_ = st.bgmloop;
            }
        }

        if st.bgmloop {
            sys::Con_Printf(c"Music will be looped\n".as_ptr());
        } else {
            sys::Con_Printf(c"Music will not be looped\n".as_ptr());
        }
    }
}

unsafe extern "C" fn bgm_stop_f() {
    // SAFETY: command trampoline
    unsafe { BGM_Stop() };
}

/// C `atoi`: optional whitespace and sign, then leading decimal digits only
/// (no exponent or hex forms -- unlike strtod)
fn c_atoi(b: &[u8]) -> core::ffi::c_int {
    let mut i = 0;
    while i < b.len() && (b[i] == b' ' || (0x09..=0x0d).contains(&b[i])) {
        i += 1;
    }
    let mut neg = false;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        neg = b[i] == b'-';
        i += 1;
    }
    let mut v: core::ffi::c_int = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        v = v
            .wrapping_mul(10)
            .wrapping_add((b[i] - b'0') as core::ffi::c_int);
        i += 1;
    }
    if neg {
        v.wrapping_neg()
    } else {
        v
    }
}

unsafe extern "C" fn bgm_jump_f() {
    let st = state();
    // SAFETY: command trampoline
    unsafe {
        if sys::Cmd_Argc() != 2 {
            sys::Con_Printf(c"music_jump <ordernum>\n".as_ptr());
        } else if !st.stream.is_null() {
            let arg = core::ffi::CStr::from_ptr(sys::Cmd_Argv(1)).to_bytes();
            crate::snd_codec::S_CodecJumpToOrder(st.stream, c_atoi(arg));
        }
    }
}
