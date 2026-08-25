//! The main sound-engine control surface (Quake/snd_dma.c, Phase 4 M6b).
//!
//! Storage stays C (snd_glue.c defines the cvars, snd_channels, shm/sn,
//! timing globals and snd_mutex -- the compat surface menu.c/cl_demo.c read
//! directly); the logic lives here, function for function. Internal state
//! that no C code ever touched (known_sfx, ambient bookkeeping, the DMA
//! wrap counters) moves into Rust statics. ADR-007 sound row: main-thread
//! access under the engine's recursive snd_mutex, exactly the C discipline.

use core::ffi::{c_char, c_int, c_void};

use quake_c_sys as sys;
use quake_snd::dma;
use quake_types::model_mem::MLeaf;
use quake_types::sound::{
    Channel, Sfx, MAX_CHANNELS, MAX_DYNAMIC_CHANNELS, MAX_QPATH, MAX_SOUNDS, NUM_AMBIENTS,
};

const AMBIENT_WATER: usize = 0;
const AMBIENT_SKY: usize = 1;

// ---------------------------------------------------------------------------
// file-internal state (was static in snd_dma.c; never visible to C)

struct SndState {
    snd_blocked: i32,
    snd_initialized: bool,
    sound_started: bool,
    known_sfx: Vec<Sfx>, // MAX_SOUNDS*2 entries, allocated once at init
    num_sfx: i32,
    ambient_sfx: [*mut Sfx; NUM_AMBIENTS],
    // GetSoundtime statics
    buffers: i32,
    oldsamplepos: i32,
    // S_UpdateAmbientSounds statics
    levels: [f32; NUM_AMBIENTS],
    // S_Play / S_PlayVol statics
    play_hash: i32,
    playvol_hash: i32,
}

// SAFETY-relevant invariant: accessed only on the main thread / under
// snd_mutex, the same discipline the C file-statics had (ADR-007).
static mut SND: SndState = SndState {
    snd_blocked: 0,
    snd_initialized: false,
    sound_started: false,
    known_sfx: Vec::new(),
    num_sfx: 0,
    ambient_sfx: [core::ptr::null_mut(); NUM_AMBIENTS],
    buffers: 0,
    oldsamplepos: 0,
    levels: [0.0; NUM_AMBIENTS],
    play_hash: 345,
    playvol_hash: 543,
};

#[allow(static_mut_refs)]
fn state() -> &'static mut SndState {
    // SAFETY: main-thread/snd_mutex access discipline (see SND above); the
    // engine never re-enters these entry points concurrently
    unsafe { &mut SND }
}

/// Base pointer to snd_glue.c's `channel_t snd_channels[MAX_CHANNELS]`
/// (layout pinned by quake-ctest/tests/snd_abi.rs). A raw pointer, not a
/// reference: several paths hold a `*mut Channel` into the array while
/// scanning it, so exclusive references are formed only in tight scopes.
fn channels_base() -> *mut Channel {
    // taking the extern static's address only; main-thread/snd_mutex
    // discipline governs the accesses derived from it
    core::ptr::addr_of_mut!(sys::snd_channels) as *mut Channel
}

fn lock() {
    // SAFETY: NULL-tolerant recursive engine mutex
    unsafe { sys::QMutex_Lock(sys::snd_mutex) };
}

fn unlock() {
    // SAFETY: as above
    unsafe { sys::QMutex_Unlock(sys::snd_mutex) };
}

fn name_bytes(name: *const c_char) -> &'static [u8] {
    // SAFETY: engine strings are NUL-terminated
    unsafe { core::ffi::CStr::from_ptr(name).to_bytes() }
}

// ---------------------------------------------------------------------------
// startup / shutdown

/// C: `void S_Startup (void)`
///
/// # Safety
/// Engine entry point; main thread.
#[no_mangle]
pub unsafe extern "C" fn S_Startup() {
    let st = state();
    if !st.snd_initialized {
        return;
    }

    // SAFETY: sn/shm are snd_glue.c storage; the harness clock replaces the
    // SDL backend under -sndhash exactly like the C
    st.sound_started = unsafe {
        if sys::harness_sndhash {
            sys::Harness_SNDDMA_Init(core::ptr::addr_of_mut!(sys::sn).cast())
        } else {
            sys::SNDDMA_Init(core::ptr::addr_of_mut!(sys::sn))
        }
    };

    // SAFETY: message varargs match snd_dma.c
    unsafe {
        if !st.sound_started {
            sys::Con_Printf(c"Failed initializing sound\n".as_ptr());
        } else {
            let shm = &*sys::shm;
            sys::Con_Printf(
                c"Audio: %d bit, %s, %d Hz\n".as_ptr(),
                shm.samplebits,
                if shm.channels == 2 {
                    c"stereo".as_ptr()
                } else {
                    c"mono".as_ptr()
                },
                shm.speed,
            );
        }
    }
}

/// C: `void S_Init (void)`
///
/// # Safety
/// Engine entry point; main thread, once.
#[no_mangle]
pub unsafe extern "C" fn S_Init() {
    let st = state();

    // SAFETY: all calls below mirror snd_dma.c's S_Init line for line; the
    // cvar_t storage lives in snd_glue.c
    unsafe {
        if st.snd_initialized {
            sys::Con_Printf(c"Sound is already initialized\n".as_ptr());
            return;
        }

        sys::snd_mutex = sys::QMutex_Create();

        sys::Cvar_RegisterVariable(core::ptr::addr_of_mut!(sys::nosound));
        sys::Cvar_RegisterVariable(core::ptr::addr_of_mut!(sys::sfxvolume));
        sys::Cvar_RegisterVariable(core::ptr::addr_of_mut!(sys::precache));
        sys::Cvar_RegisterVariable(core::ptr::addr_of_mut!(sys::loadas8bit));
        sys::Cvar_RegisterVariable(core::ptr::addr_of_mut!(sys::bgmvolume));
        sys::Cvar_RegisterVariable(core::ptr::addr_of_mut!(sys::ambient_level));
        sys::Cvar_RegisterVariable(core::ptr::addr_of_mut!(sys::ambient_fade));
        sys::Cvar_RegisterVariable(core::ptr::addr_of_mut!(sys::snd_noextraupdate));
        sys::Cvar_RegisterVariable(core::ptr::addr_of_mut!(sys::snd_show));
        sys::Cvar_RegisterVariable(core::ptr::addr_of_mut!(sys::_snd_mixahead));
        sys::Cvar_RegisterVariable(core::ptr::addr_of_mut!(sys::sndspeed));
        sys::Cvar_RegisterVariable(core::ptr::addr_of_mut!(sys::snd_mixspeed));
        sys::Cvar_RegisterVariable(core::ptr::addr_of_mut!(sys::snd_filterquality));
        sys::Cvar_RegisterVariable(core::ptr::addr_of_mut!(sys::snd_waterfx));
        sys::Cvar_RegisterVariable(core::ptr::addr_of_mut!(sys::snd_pauselooping));

        if sys::safemode != 0 || sys::COM_CheckParm(c"-nosound".as_ptr()) != 0 {
            return;
        }

        sys::Con_Printf(c"\nSound Initialization\n".as_ptr());

        sys::Cmd_AddCommand2(
            c"play".as_ptr(),
            Some(s_play_f),
            sys::cmd_source_t_src_command,
            false,
        );
        sys::Cmd_AddCommand2(
            c"playvol".as_ptr(),
            Some(s_playvol_f),
            sys::cmd_source_t_src_command,
            false,
        );
        sys::Cmd_AddCommand2(
            c"stopsound".as_ptr(),
            Some(s_stopallsounds_f),
            sys::cmd_source_t_src_command,
            false,
        );
        sys::Cmd_AddCommand2(
            c"soundlist".as_ptr(),
            Some(s_soundlist_f),
            sys::cmd_source_t_src_command,
            false,
        );
        sys::Cmd_AddCommand2(
            c"soundinfo".as_ptr(),
            Some(s_soundinfo_f),
            sys::cmd_source_t_src_command,
            false,
        );

        let i = sys::COM_CheckParm(c"-sndspeed".as_ptr());
        if i != 0 && i < sys::com_argc - 1 {
            sys::Cvar_SetQuick(
                core::ptr::addr_of_mut!(sys::sndspeed),
                *sys::com_argv.add(i as usize + 1),
            );
        }

        let i = sys::COM_CheckParm(c"-mixspeed".as_ptr());
        if i != 0 && i < sys::com_argc - 1 {
            sys::Cvar_SetQuick(
                core::ptr::addr_of_mut!(sys::snd_mixspeed),
                *sys::com_argv.add(i as usize + 1),
            );
        }

        sys::Cvar_SetCallback(
            core::ptr::addr_of_mut!(sys::sfxvolume),
            Some(callback_sfxvolume),
        );
        sys::Cvar_SetCallback(
            core::ptr::addr_of_mut!(sys::snd_filterquality),
            Some(callback_snd_filterquality),
        );

        crate::snd_mix::SND_InitScaletable();
        st.num_sfx = 0;
        st.known_sfx = (0..MAX_SOUNDS * 2)
            .map(|_| Sfx {
                name: [0; MAX_QPATH],
                cache: core::ptr::null_mut(),
            })
            .collect();

        st.snd_initialized = true;

        S_Startup();
        if !st.sound_started {
            return;
        }

        st.ambient_sfx[AMBIENT_WATER] = S_PrecacheSound(c"ambience/water1.wav".as_ptr());
        st.ambient_sfx[AMBIENT_SKY] = S_PrecacheSound(c"ambience/wind2.wav".as_ptr());

        crate::snd_codec::S_CodecInit();

        S_StopAllSounds(true, false);
    }
}

/// C: `void S_Shutdown (void)`
///
/// # Safety
/// Engine entry point; main thread.
#[no_mangle]
pub unsafe extern "C" fn S_Shutdown() {
    let st = state();
    if !st.sound_started {
        return;
    }

    st.sound_started = false;
    st.snd_blocked = 0;

    // SAFETY: mirrors the C shutdown order
    unsafe {
        crate::snd_codec::S_CodecShutdown();
        if sys::harness_sndhash {
            sys::Harness_SNDDMA_Shutdown();
        } else {
            sys::SNDDMA_Shutdown();
        }
        sys::shm = core::ptr::null_mut();
    }
}

// ---------------------------------------------------------------------------
// sfx registry

/// S_FindName (file-internal in C)
fn find_name(st: &mut SndState, name: *const c_char) -> *mut Sfx {
    // SAFETY: diverging engine error calls, exactly the C's
    unsafe {
        if name.is_null() {
            sys::Sys_Error(c"S_FindName: NULL".as_ptr());
        }
        if name_bytes(name).len() >= MAX_QPATH {
            sys::Sys_Error(c"Sound name too long: %s".as_ptr(), name);
        }
    }
    let nameb = name_bytes(name);

    // see if already loaded
    for i in 0..st.num_sfx as usize {
        let sfx_name = &st.known_sfx[i].name;
        let len = sfx_name.iter().position(|&c| c == 0).unwrap_or(MAX_QPATH);
        let bytes: &[u8] =
            // SAFETY: c_char and u8 have the same layout
            unsafe { core::slice::from_raw_parts(sfx_name.as_ptr().cast(), len) };
        if bytes == nameb {
            return &mut st.known_sfx[i];
        }
    }

    let mut i = st.num_sfx as usize;
    if st.num_sfx as usize == st.known_sfx.len() {
        flush_oldest_sounds(st);
        debug_assert!(st.num_sfx as usize == MAX_SOUNDS);
        i = MAX_SOUNDS;
    }

    let sfx = &mut st.known_sfx[i];
    // q_strlcpy into name[MAX_QPATH] (length already validated < MAX_QPATH)
    sfx.name = [0; MAX_QPATH];
    for (dst, &src) in sfx.name.iter_mut().zip(nameb.iter()) {
        *dst = src as c_char;
    }

    st.num_sfx += 1;
    sfx
}

/// S_FlushOldestSounds (file-internal in C): free the first MAX_SOUNDS
/// caches and slide the most recent half down. The C memmove leaves the
/// upper half's stale entries in place -- reproduced (they are overwritten
/// as num_sfx grows again).
fn flush_oldest_sounds(st: &mut SndState) {
    for i in 0..MAX_SOUNDS {
        let cache = st.known_sfx[i].cache;
        if !cache.is_null() {
            // SAFETY: caches are Mem_Alloc'd by S_LoadSound (ADR-013)
            unsafe { sys::Mem_Free(cache.cast()) };
            st.known_sfx[i].cache = core::ptr::null_mut();
        }
    }
    for i in 0..MAX_SOUNDS {
        let src = &st.known_sfx[MAX_SOUNDS + i];
        let copy = Sfx {
            name: src.name,
            cache: src.cache,
        };
        st.known_sfx[i] = copy;
    }
    st.num_sfx = MAX_SOUNDS as i32;
}

/// C: `void S_TouchSound (const char *name)`
///
/// # Safety
/// `name` NUL-terminated; main thread.
#[no_mangle]
pub unsafe extern "C" fn S_TouchSound(name: *const c_char) {
    let st = state();
    if !st.sound_started {
        return;
    }
    find_name(st, name);
}

/// C: `sfx_t *S_PrecacheSound (const char *name)`
///
/// # Safety
/// `name` NUL-terminated; main thread.
#[no_mangle]
pub unsafe extern "C" fn S_PrecacheSound(name: *const c_char) -> *mut Sfx {
    let st = state();
    // SAFETY: cvar read, main thread
    if !st.sound_started || unsafe { sys::nosound.value } != 0.0 {
        return core::ptr::null_mut();
    }

    let sfx = find_name(st, name);

    // cache it in
    // SAFETY: cvar read; S_LoadSound is our own export
    unsafe {
        if sys::precache.value != 0.0 {
            crate::snd_mem::S_LoadSound(sfx);
        }
    }

    sfx
}

// ---------------------------------------------------------------------------
// channel allocation / spatialization

/// C: `channel_t *SND_PickChannel (int entnum, int entchannel)`
///
/// # Safety
/// Main thread under snd_mutex (callers hold it, as in C).
#[no_mangle]
pub unsafe extern "C" fn SND_PickChannel(entnum: c_int, entchannel: c_int) -> *mut Channel {
    // SAFETY: paintedtime is glue storage; viewentity via the glue accessor;
    // the slice borrow is exclusive for the duration of pick_channel only
    unsafe {
        let (painted, viewentity) = (sys::paintedtime, sys::SND_Glue_ViewEntity());
        let picked = {
            let chans = core::slice::from_raw_parts_mut(channels_base(), MAX_CHANNELS);
            dma::pick_channel(chans, painted, viewentity, entnum, entchannel)
        };
        match picked {
            Some(i) => channels_base().add(i),
            None => core::ptr::null_mut(),
        }
    }
}

/// C: `void SND_Spatialize (channel_t *ch)`
///
/// # Safety
/// `ch` valid; main thread under snd_mutex.
#[no_mangle]
pub unsafe extern "C" fn SND_Spatialize(ch: *mut Channel) {
    // SAFETY: listener vectors and shm are glue storage set by S_Update
    unsafe {
        let origin: [f32; 3] = sys::listener_origin;
        let right: [f32; 3] = sys::listener_right;
        let shm_channels = if sys::shm.is_null() {
            2
        } else {
            (*sys::shm).channels
        };
        dma::spatialize(
            &mut *ch,
            &origin,
            &right,
            sys::SND_Glue_ViewEntity(),
            shm_channels,
        );
    }
}

// ---------------------------------------------------------------------------
// starting / stopping sounds

/// C: `void S_StartSound (int entnum, int entchannel, sfx_t *sfx, vec3_t origin, float fvol, float attenuation)`
///
/// # Safety
/// `origin` points at 3 floats; `sfx` NULL or valid; any thread that C
/// allowed (the mutex serializes).
#[no_mangle]
pub unsafe extern "C" fn S_StartSound(
    entnum: c_int,
    entchannel: c_int,
    sfx: *mut Sfx,
    origin: *mut f32,
    fvol: f32,
    attenuation: f32,
) {
    lock();
    let st = state();

    'unlock: {
        // SAFETY: cvar read under lock
        if !st.sound_started || sfx.is_null() || unsafe { sys::nosound.value } != 0.0 {
            break 'unlock;
        }

        // pick a channel to play on
        // SAFETY: our own export, lock is recursive
        let target_chan = unsafe { SND_PickChannel(entnum, entchannel) };
        if target_chan.is_null() {
            break 'unlock;
        }

        // spatialize
        // SAFETY: target_chan points into snd_channels; origin has 3 floats;
        // all channel accesses go through raw pointers (target_chan and the
        // scan below alias the same array)
        unsafe {
            core::ptr::write(target_chan, core::mem::zeroed());
            (*target_chan).origin = [*origin, *origin.add(1), *origin.add(2)];
            (*target_chan).dist_mult = (attenuation as f64 / dma::SOUND_NOMINAL_CLIP_DIST) as f32;
            (*target_chan).master_vol = (fvol * 255.0) as i32;
            (*target_chan).entnum = entnum;
            (*target_chan).entchannel = entchannel;
            SND_Spatialize(target_chan);

            if (*target_chan).leftvol == 0 && (*target_chan).rightvol == 0 {
                break 'unlock;
            }

            // new channel
            let sc = crate::snd_mem::S_LoadSound(sfx);
            if sc.is_null() {
                (*target_chan).sfx = core::ptr::null_mut();
                break 'unlock; // couldn't load the sound's data
            }
            let sc = &*sc;

            (*target_chan).sfx = sfx;
            (*target_chan).pos = 0;
            (*target_chan).end = sys::paintedtime + sc.length;

            // if an identical sound has also been started this frame, offset
            // the pos a bit to keep it from just making the first one louder
            let base = channels_base();
            for ch_idx in NUM_AMBIENTS..NUM_AMBIENTS + MAX_DYNAMIC_CHANNELS {
                let check = base.add(ch_idx);
                if core::ptr::eq(check, target_chan) {
                    continue;
                }
                if (*check).sfx == sfx && (*check).pos == 0 {
                    // LordHavoc: fixed skip calculations
                    let mut skip = (0.1f64 * (*sys::shm).speed as f64) as i32;
                    if skip > sc.length {
                        skip = sc.length;
                    }
                    if skip > 0 {
                        skip = sys::COM_Rand() % skip;
                    }
                    (*target_chan).pos += skip;
                    (*target_chan).end -= skip;
                    break;
                }
            }
        }
    }
    unlock();
}

/// C: `void S_StopSound (int entnum, int entchannel)`
///
/// # Safety
/// Engine entry point.
#[no_mangle]
pub unsafe extern "C" fn S_StopSound(entnum: c_int, entchannel: c_int) {
    lock();
    // SAFETY: exclusive access for this scope (main thread under snd_mutex)
    unsafe {
        let chans = core::slice::from_raw_parts_mut(channels_base(), MAX_DYNAMIC_CHANNELS);
        for ch in chans.iter_mut() {
            if ch.entnum == entnum && ch.entchannel == entchannel {
                ch.end = 0;
                ch.sfx = core::ptr::null_mut();
                break;
            }
        }
    }
    unlock();
}

/// C: `void S_StopAllSounds (qboolean clear, qboolean keep_statics)`
///
/// # Safety
/// Engine entry point.
#[no_mangle]
pub unsafe extern "C" fn S_StopAllSounds(clear: bool, keep_statics: bool) {
    let st = state();
    if !st.snd_initialized {
        return;
    }

    lock();
    'unlock: {
        if !st.sound_started {
            break 'unlock;
        }

        if !keep_statics {
            // SAFETY: glue storage under lock
            unsafe { sys::total_channels = (MAX_DYNAMIC_CHANNELS + NUM_AMBIENTS) as c_int };
        }

        for i in 0..MAX_CHANNELS {
            // the C calls S_LoadSound up to three times per channel; keep the
            // call pattern (each is a cheap cache hit after the first)
            // SAFETY: our own exports under the recursive lock
            unsafe {
                let ch = channels_base().add(i);
                let reset = !keep_statics
                    || (*ch).entnum != 0
                    || (*ch).sfx.is_null()
                    || crate::snd_mem::S_LoadSound((*ch).sfx).is_null()
                    || (*crate::snd_mem::S_LoadSound((*ch).sfx)).loopstart == -1;
                if reset {
                    *ch = core::mem::zeroed();
                } else {
                    (*ch).pos = 0;
                    (*ch).end = sys::paintedtime + (*crate::snd_mem::S_LoadSound((*ch).sfx)).length;
                }
            }
        }

        if clear {
            // SAFETY: our own export; lock is recursive
            unsafe { S_ClearBuffer() };
        }
    }
    unlock();
}

unsafe extern "C" fn s_stopallsounds_f() {
    // SAFETY: command trampoline on the main thread
    unsafe { S_StopAllSounds(true, false) };
}

/// C: `void S_ClearBuffer (void)`
///
/// # Safety
/// Engine entry point.
#[no_mangle]
pub unsafe extern "C" fn S_ClearBuffer() {
    lock();
    let st = state();
    'unlock: {
        // SAFETY: glue storage / DMA under lock, mirrors the C exactly
        unsafe {
            if !st.sound_started || sys::shm.is_null() {
                break 'unlock;
            }
            sys::SNDDMA_LockBuffer();
            let shm = &*sys::shm;
            if shm.buffer.is_null() {
                break 'unlock;
            }

            sys::s_rawend = 0;

            let clear: u8 = if shm.samplebits == 8 && shm.signed8 == 0 {
                0x80
            } else {
                0
            };
            core::ptr::write_bytes(
                shm.buffer,
                clear,
                (shm.samples * shm.samplebits / 8).max(0) as usize,
            );

            sys::SNDDMA_Submit();
        }
    }
    unlock();
}

/// C: `void S_StaticSound (sfx_t *sfx, vec3_t origin, int vol, float attenuation)`
///
/// # Safety
/// `origin` points at 3 floats; `sfx` NULL or valid.
#[no_mangle]
pub unsafe extern "C" fn S_StaticSound(
    sfx: *mut Sfx,
    origin: *mut f32,
    vol: c_int,
    attenuation: f32,
) {
    if sfx.is_null() {
        return;
    }

    lock();
    'unlock: {
        // SAFETY: glue storage under lock; messages match snd_dma.c
        unsafe {
            if sys::total_channels == MAX_CHANNELS as c_int {
                sys::Con_Printf(c"total_channels == MAX_CHANNELS\n".as_ptr());
                break 'unlock;
            }

            let ss_idx = sys::total_channels as usize;
            sys::total_channels += 1;

            let sc = crate::snd_mem::S_LoadSound(sfx);
            if sc.is_null() {
                break 'unlock;
            }
            let sc = &*sc;

            if sc.loopstart == -1 {
                sys::Con_Printf(c"Sound %s not looped\n".as_ptr(), (*sfx).name.as_ptr());
                break 'unlock;
            }

            let ss = channels_base().add(ss_idx);
            (*ss).sfx = sfx;
            (*ss).origin = [*origin, *origin.add(1), *origin.add(2)];
            (*ss).master_vol = vol;
            (*ss).dist_mult = ((attenuation / 64.0) as f64 / dma::SOUND_NOMINAL_CLIP_DIST) as f32;
            (*ss).end = sys::paintedtime + sc.length;

            SND_Spatialize(ss);
        }
    }
    unlock();
}

// ---------------------------------------------------------------------------
// per-frame update

fn underwater_intensity_for_contents(contents: i32) -> f32 {
    // CONTENTS_WATER -3, CONTENTS_SLIME -4, CONTENTS_LAVA -5 (bspfile.h)
    match contents {
        -5..=-3 => 1.0,
        _ => 0.0,
    }
}

/// S_UpdateAmbientSounds (file-internal in C)
fn update_ambient_sounds(st: &mut SndState) {
    lock();
    'unlock: {
        // SAFETY: glue accessors/storage under lock; float promotion points
        // match the C exactly
        unsafe {
            // no ambients when disconnected
            if !sys::SND_Glue_ClientConnected() {
                crate::snd_mix::S_SetUnderwaterIntensity(0.0);
                break 'unlock;
            }
            // calc ambient sound levels
            if sys::SND_Glue_Worldmodel().is_null() {
                break 'unlock;
            }

            let leaf =
                sys::SND_Glue_PointInLeaf(core::ptr::addr_of_mut!(sys::listener_origin).cast())
                    as *mut MLeaf;
            crate::snd_mix::S_SetUnderwaterIntensity(if leaf.is_null() {
                0.0
            } else {
                underwater_intensity_for_contents((*leaf).contents)
            });
            if leaf.is_null() || sys::ambient_level.value == 0.0 {
                for i in 0..NUM_AMBIENTS {
                    (*channels_base().add(i)).sfx = core::ptr::null_mut();
                }
                break 'unlock;
            }
            let leaf = &*leaf;

            for ambient_channel in 0..NUM_AMBIENTS {
                let chan = channels_base().add(ambient_channel);
                (*chan).sfx = st.ambient_sfx[ambient_channel];

                // C: static float vol = (int)(ambient_level.value * level)
                let vol = (sys::ambient_level.value
                    * leaf.ambient_sound_level[ambient_channel] as f32)
                    as i32 as f32;
                let vol = if vol < 8.0 { 0.0 } else { vol };

                // don't adjust volume too fast
                let level = &mut st.levels[ambient_channel];
                if *level < vol {
                    *level = (*level as f64 + sys::host_frametime * sys::ambient_fade.value as f64)
                        as f32;
                    if *level > vol {
                        *level = vol;
                    }
                } else if (*chan).master_vol as f32 > vol {
                    *level = (*level as f64 - sys::host_frametime * sys::ambient_fade.value as f64)
                        as f32;
                    if *level < vol {
                        *level = vol;
                    }
                }

                let l = *level as i32;
                (*chan).leftvol = l;
                (*chan).rightvol = l;
                (*chan).master_vol = l;
            }
        }
    }
    unlock();
}

/// C: `void S_RawSamples (int samples, int rate, int width, int channels, byte *data, float volume)`
///
/// # Safety
/// `data` valid for the format's extent.
#[no_mangle]
pub unsafe extern "C" fn S_RawSamples(
    samples: c_int,
    rate: c_int,
    width: c_int,
    chans: c_int,
    data: *mut u8,
    volume: f32,
) {
    // SAFETY: glue storage; data extent per caller contract (the codecs pass
    // consistent sample counts, like the C)
    unsafe {
        let ring = &mut *(core::ptr::addr_of_mut!(sys::s_rawsamples)
            as *mut [quake_types::sound::SamplePair; quake_types::sound::MAX_RAW_SAMPLES]);
        let mut rawend = sys::s_rawend;
        let byte_len = (samples.max(0) as usize)
            .saturating_mul(chans.max(1) as usize)
            .saturating_mul(width.max(1) as usize);
        let data = core::slice::from_raw_parts(data, byte_len);
        dma::raw_samples(
            ring,
            &mut rawend,
            sys::paintedtime,
            samples,
            rate,
            width,
            chans,
            data,
            volume,
            (*sys::shm).speed,
        );
        sys::s_rawend = rawend;
    }
}

/// C: `void S_Update (vec3_t origin, vec3_t forward, vec3_t right, vec3_t up)`
///
/// # Safety
/// Vectors point at 3 floats each; main thread.
#[no_mangle]
pub unsafe extern "C" fn S_Update(
    origin: *mut f32,
    forward: *mut f32,
    right: *mut f32,
    up: *mut f32,
) {
    lock();
    let st = state();
    'unlock: {
        if !st.sound_started || st.snd_blocked > 0 {
            break 'unlock;
        }

        // SAFETY: glue storage under lock; combine logic matches C exactly
        unsafe {
            let copy3 = |dst: *mut [f32; 3], src: *mut f32| {
                (*dst) = [*src, *src.add(1), *src.add(2)];
            };
            copy3(core::ptr::addr_of_mut!(sys::listener_origin), origin);
            copy3(core::ptr::addr_of_mut!(sys::listener_forward), forward);
            copy3(core::ptr::addr_of_mut!(sys::listener_right), right);
            copy3(core::ptr::addr_of_mut!(sys::listener_up), up);

            // update general area ambient sound sources
            update_ambient_sounds(st);

            let total = sys::total_channels as usize;
            let base = channels_base();
            let mut combine: Option<usize> = None;

            // update spatialization for static and dynamic sounds
            // (raw pointers throughout: SND_Spatialize takes a pointer into
            // the same array the combine pass reads and writes)
            for i in NUM_AMBIENTS..total {
                let ch = base.add(i);
                if (*ch).sfx.is_null() {
                    continue;
                }
                SND_Spatialize(ch); // respatialize channel
                if (*ch).leftvol == 0 && (*ch).rightvol == 0 {
                    continue;
                }

                // try to combine static sounds with a previous channel of the
                // same sound effect so we don't mix five torches every frame
                if i >= MAX_DYNAMIC_CHANNELS + NUM_AMBIENTS {
                    // see if it can just use the last one
                    if let Some(c) = combine {
                        let cc = base.add(c);
                        if (*cc).sfx == (*ch).sfx {
                            (*cc).leftvol += (*ch).leftvol;
                            (*cc).rightvol += (*ch).rightvol;
                            (*ch).leftvol = 0;
                            (*ch).rightvol = 0;
                            continue;
                        }
                    }
                    // search for one
                    let mut j = MAX_DYNAMIC_CHANNELS + NUM_AMBIENTS;
                    while j < i {
                        if (*base.add(j)).sfx == (*ch).sfx {
                            break;
                        }
                        j += 1;
                    }

                    // C quirk kept: the miss test compares against
                    // total_channels, not the loop bound i
                    if j == total {
                        combine = None;
                    } else {
                        if j != i {
                            let cj = base.add(j);
                            (*cj).leftvol += (*ch).leftvol;
                            (*cj).rightvol += (*ch).rightvol;
                            (*ch).leftvol = 0;
                            (*ch).rightvol = 0;
                        }
                        combine = Some(j);
                        continue;
                    }
                }
            }

            // debugging output
            if sys::snd_show.value != 0.0 {
                let mut dbg_total = 0;
                for i in 0..total {
                    let ch = base.add(i);
                    if !(*ch).sfx.is_null() && ((*ch).leftvol != 0 || (*ch).rightvol != 0) {
                        dbg_total += 1;
                    }
                }
                sys::Con_Printf(c"----(%i)----\n".as_ptr(), dbg_total as c_int);
            }

            // mix some sound
            s_update_mix(st);
        }
    }
    unlock();
}

/// GetSoundtime (file-internal in C)
fn get_soundtime(st: &mut SndState) {
    // SAFETY: glue storage/DMA position under lock
    unsafe {
        let shm = &*sys::shm;
        let fullsamples = shm.samples / shm.channels;

        // it is possible to miscount buffers if it has wrapped twice between
        // calls to S_Update. Oh well.
        let samplepos = if sys::harness_sndhash {
            sys::Harness_SNDDMA_GetDMAPos()
        } else {
            sys::SNDDMA_GetDMAPos()
        };

        if samplepos < st.oldsamplepos {
            st.buffers += 1; // buffer wrapped

            if sys::paintedtime > 0x40000000 {
                // time to chop things off to avoid 32 bit limits
                st.buffers = 0;
                sys::paintedtime = fullsamples;
                S_StopAllSounds(true, true);
            }
        }
        st.oldsamplepos = samplepos;

        sys::soundtime = st.buffers * fullsamples + samplepos / shm.channels;
    }
}

/// C: `void S_ExtraUpdate (void)`
///
/// # Safety
/// Engine entry point; main thread.
#[no_mangle]
pub unsafe extern "C" fn S_ExtraUpdate() {
    // SAFETY: cvar read
    if unsafe { sys::snd_noextraupdate.value } != 0.0 {
        return; // don't pollute timings
    }
    let st = state();
    s_update_mix(st);
}

/// S_Update_ (file-internal in C)
fn s_update_mix(st: &mut SndState) {
    if !st.snd_initialized {
        return;
    }

    lock();
    'unlock: {
        if !st.sound_started || st.snd_blocked > 0 {
            break 'unlock;
        }

        // SAFETY: DMA lock + glue storage, mirroring S_Update_ exactly
        unsafe {
            sys::SNDDMA_LockBuffer();
            if (*sys::shm).buffer.is_null() {
                break 'unlock;
            }

            // Updates DMA time
            get_soundtime(st);

            // check to make sure that we haven't overshot
            if sys::paintedtime < sys::soundtime {
                sys::paintedtime = sys::soundtime;
            }

            // mix ahead of current position
            let shm = &*sys::shm;
            let mut endtime =
                sys::soundtime as u32 + (sys::_snd_mixahead.value * shm.speed as f32) as u32;
            let samps = (shm.samples >> (shm.channels - 1)) as u32;
            endtime = endtime.min(sys::soundtime as u32 + samps);

            crate::snd_mix::S_PaintChannels(endtime as c_int);

            sys::SNDDMA_Submit();
        }
    }
    unlock();
}

// ---------------------------------------------------------------------------
// blocking

/// C: `void S_BlockSound (void)`
///
/// # Safety
/// Engine entry point.
#[no_mangle]
pub unsafe extern "C" fn S_BlockSound() {
    lock();
    let st = state();
    if st.sound_started && st.snd_blocked == 0 {
        st.snd_blocked = 1;
        // SAFETY: our own export + DMA block, mirroring the C
        unsafe {
            S_ClearBuffer();
            if !sys::shm.is_null() {
                sys::SNDDMA_BlockSound();
            }
        }
    }
    unlock();
}

/// C: `void S_UnblockSound (void)`
///
/// # Safety
/// Engine entry point.
#[no_mangle]
pub unsafe extern "C" fn S_UnblockSound() {
    lock();
    let st = state();
    'unlock: {
        if !st.sound_started || st.snd_blocked == 0 {
            break 'unlock;
        }
        if st.snd_blocked == 1 {
            st.snd_blocked = 0;
            // SAFETY: DMA unblock + our own export, mirroring the C
            unsafe {
                sys::SNDDMA_UnblockSound();
                S_ClearBuffer();
            }
        }
    }
    unlock();
}

/// C: `void S_ClearAll (void)`
///
/// # Safety
/// Engine entry point.
#[no_mangle]
pub unsafe extern "C" fn S_ClearAll() {
    lock();
    let st = state();
    for i in 0..st.num_sfx as usize {
        let cache = st.known_sfx[i].cache;
        if !cache.is_null() {
            // SAFETY: Mem-owned caches (ADR-013)
            unsafe { sys::Mem_Free(cache.cast()) };
            st.known_sfx[i].cache = core::ptr::null_mut();
        }
    }
    unlock();
}

// ---------------------------------------------------------------------------
// console commands

unsafe extern "C" fn callback_sfxvolume(_var: *mut sys::cvar_s) {
    // SAFETY: main-thread cvar callback
    unsafe { crate::snd_mix::SND_InitScaletable() };
}

unsafe extern "C" fn callback_snd_filterquality(_var: *mut sys::cvar_s) {
    // SAFETY: main-thread cvar callback; message and default match snd_dma.c
    unsafe {
        if sys::snd_filterquality.value < 1.0 || sys::snd_filterquality.value > 5.0 {
            sys::Con_Printf(c"snd_filterquality must be between 1 and 5\n".as_ptr());
            let default = if cfg!(windows) { c"5" } else { c"1" };
            sys::Cvar_SetQuick(
                core::ptr::addr_of_mut!(sys::snd_filterquality),
                default.as_ptr(),
            );
        }
    }
}

/// shared body of the `play` / `playvol` commands
unsafe fn play_cmd(with_vol: bool) {
    let st = state();
    // SAFETY: command context on the main thread; matches S_Play/S_PlayVol
    unsafe {
        let mut i = 1;
        while i < sys::Cmd_Argc() {
            let arg = sys::Cmd_Argv(i);
            let argb = name_bytes(arg);
            let mut name = [0u8; 256];
            let n = argb.len().min(255);
            name[..n].copy_from_slice(&argb[..n]);
            if !argb.contains(&b'.') {
                // C: strrchr(arg, '.') == NULL -> q_strlcat(name, ".wav", 256):
                // append at the current end, truncating at the 255-byte cap
                for (k, &b) in b".wav".iter().enumerate() {
                    if n + k < 255 {
                        name[n + k] = b;
                    }
                }
            }
            let sfx = S_PrecacheSound(name.as_ptr().cast());
            let (hash, vol) = if with_vol {
                let v = quake_c_sys::libm::strtod(name_bytes(sys::Cmd_Argv(i + 1))) as f32;
                let h = st.playvol_hash;
                st.playvol_hash += 1;
                (h, v)
            } else {
                let h = st.play_hash;
                st.play_hash += 1;
                (h, 1.0)
            };
            let mut origin: [f32; 3] = sys::listener_origin;
            S_StartSound(hash, 0, sfx, origin.as_mut_ptr(), vol, 1.0);
            i += if with_vol { 2 } else { 1 };
        }
    }
}

unsafe extern "C" fn s_play_f() {
    // SAFETY: command trampoline
    unsafe { play_cmd(false) };
}

unsafe extern "C" fn s_playvol_f() {
    // SAFETY: command trampoline
    unsafe { play_cmd(true) };
}

unsafe extern "C" fn s_soundlist_f() {
    let st = state();
    // SAFETY: prints match S_SoundList byte for byte
    unsafe {
        let mut total: usize = 0;
        for i in 0..st.num_sfx as usize {
            let sfx = &st.known_sfx[i];
            if sfx.cache.is_null() {
                continue;
            }
            let sc = &*sfx.cache;
            let size = sc.length * sc.width * (sc.stereo + 1);
            total += size as usize;
            if sc.loopstart >= 0 {
                sys::Con_SafePrintf(c"L".as_ptr());
            } else {
                sys::Con_SafePrintf(c" ".as_ptr());
            }
            sys::Con_SafePrintf(
                c"(%2db) %9i : %s\n".as_ptr(),
                sc.width * 8,
                size,
                sfx.name.as_ptr(),
            );
        }
        sys::Con_Printf(
            c"%i sounds, %lu bytes\n".as_ptr(),
            st.num_sfx,
            total as core::ffi::c_ulong,
        );
    }
}

unsafe extern "C" fn s_soundinfo_f() {
    let st = state();
    // SAFETY: prints match S_SoundInfo_f byte for byte
    unsafe {
        if !st.sound_started || sys::shm.is_null() {
            sys::Con_Printf(c"sound system not started\n".as_ptr());
            return;
        }
        let shm = &*sys::shm;
        sys::Con_Printf(
            c"%d bit, %s, %d Hz\n".as_ptr(),
            shm.samplebits,
            if shm.channels == 2 {
                c"stereo".as_ptr()
            } else {
                c"mono".as_ptr()
            },
            shm.speed,
        );
        sys::Con_Printf(c"%5d samples\n".as_ptr(), shm.samples);
        sys::Con_Printf(c"%5d samplepos\n".as_ptr(), shm.samplepos);
        sys::Con_Printf(c"%5d submission_chunk\n".as_ptr(), shm.submission_chunk);
        sys::Con_Printf(c"%5d total_channels\n".as_ptr(), sys::total_channels);
        sys::Con_Printf(c"%p dma buffer\n".as_ptr(), shm.buffer as *const c_void);
    }
}

/// C: `void S_LocalSound (const char *name)`
///
/// # Safety
/// `name` NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn S_LocalSound(name: *const c_char) {
    lock();
    let st = state();
    'unlock: {
        // SAFETY: cvar reads and our own exports under the recursive lock
        unsafe {
            if sys::nosound.value != 0.0 || !st.sound_started {
                break 'unlock;
            }

            let sfx = S_PrecacheSound(name);
            if sfx.is_null() {
                sys::Con_Printf(c"S_LocalSound: can't cache %s\n".as_ptr(), name);
                break 'unlock;
            }
            let mut origin = [0.0f32; 3]; // vec3_origin
            S_StartSound(
                sys::SND_Glue_ViewEntity(),
                -1,
                sfx,
                origin.as_mut_ptr(),
                1.0,
                1.0,
            );
        }
    }
    unlock();
}

/// C no-ops kept for the header surface
///
/// # Safety
/// Trivial.
#[no_mangle]
pub unsafe extern "C" fn S_ClearPrecache() {}

/// # Safety
/// Trivial.
#[no_mangle]
pub unsafe extern "C" fn S_BeginPrecaching() {}

/// # Safety
/// Trivial.
#[no_mangle]
pub unsafe extern "C" fn S_EndPrecaching() {}
