//! cbindgen-exported extern "C" shims; builds the libquake_rs staticlib.
//!
//! ADR-011: every `#[no_mangle] extern "C"` export of the workspace lives in
//! this crate, and each shim replicates its C original's signature exactly so
//! call sites keep compiling against the existing engine headers (crc.h,
//! strl_fn.h, ...). `scripts/harness/check_capi_signatures.sh` enforces that
//! by compiling the generated quake_rs.h against those headers in one TU.

// DO_USERDIRS adds per-user lookup steps to LOC_LoadFile that the Rust fs
// does not implement yet; fail loudly rather than silently diverge
#[cfg(all(feature = "fs", feature = "userdirs"))]
compile_error!(
    "the Rust filesystem does not implement DO_USERDIRS yet; build with -Duse_rust_fs=disabled"
);

#[cfg(feature = "engine-alloc")]
pub mod alloc;
#[cfg(feature = "snd")]
pub mod bgmusic;
pub mod cfgfile;
#[cfg(feature = "host")]
pub mod chase; // chase.c
#[cfg(feature = "host")]
pub mod cl_demo; // cl_demo.c
#[cfg(feature = "host")]
pub mod cl_input; // cl_input.c
#[cfg(feature = "host")]
pub mod cl_main; // cl_main.c
#[cfg(feature = "host")]
pub mod cl_parse; // cl_parse.c
#[cfg(feature = "host")]
pub mod cl_tent; // cl_tent.c
#[cfg(feature = "cvar")]
pub mod cmd; // cmd.c
pub mod crc;
#[cfg(feature = "cvar")]
pub mod cvar; // cvar.c
#[cfg(feature = "fs")]
pub mod fs;
#[cfg(feature = "fs")]
pub mod fs_stdio;
pub mod hash_map;
#[cfg(feature = "host")]
pub mod host; // host.c
#[cfg(feature = "host")]
pub mod host_cmd; // host_cmd.c
#[cfg(feature = "image")]
pub mod image_decode;
pub mod json;
#[cfg(feature = "fs")]
pub mod loc;
pub mod mathlib;
pub mod mdfour;
#[cfg(feature = "formats")]
pub mod model_parse;
#[cfg(feature = "net")]
pub mod net;
#[cfg(feature = "net")]
pub mod net_dgrm;
#[cfg(feature = "net")]
pub mod net_dgrm_orch; // net_dgrm.c orchestration half
#[cfg(feature = "net")]
pub mod net_loop;
#[cfg(feature = "net")]
pub mod net_main;
#[cfg(feature = "net")]
pub mod net_udp;
#[cfg(any(feature = "progs", feature = "progs-host"))]
pub mod pr_edict_arena; // pr_edict_arena.c (edict arena + progs string table)
#[cfg(feature = "progs")]
pub mod progs_builtins; // pr_cmds.c
#[cfg(all(feature = "host", feature = "progs-host"))]
pub mod progs_builtins_cl; // pr_cmds.c client-coupled builtins (Group F)
#[cfg(all(feature = "host", feature = "progs-host"))]
pub mod progs_builtins_filebuf; // pr_ext.c FRIK_FILE + strbuf builtins (M9f group C)
#[cfg(all(feature = "host", feature = "progs-host"))]
pub mod progs_builtins_particles; // pr_ext.c particle builtins (M9f group E)
#[cfg(all(feature = "host", feature = "progs-host"))]
pub mod progs_builtins_sprintf; // pr_ext.c sprintf group (M9f group A)
#[cfg(all(feature = "host", feature = "progs-host"))]
pub mod progs_builtins_strext; // pr_ext.c strconv/tokenizer/info group (M9f group B)
#[cfg(all(feature = "host", feature = "progs-host"))]
pub mod progs_builtins_sv; // pr_cmds.c / pr_ext.c server-coupled builtins
#[cfg(all(feature = "host", feature = "progs-host"))]
pub mod progs_builtins_sv_fx; // pr_cmds.c world-effect builtins (Group E)
#[cfg(all(feature = "host", feature = "progs-host"))]
pub mod progs_builtins_sv_msg; // pr_cmds.c / pr_ext.c message builtins
#[cfg(all(feature = "host", feature = "progs-host"))]
pub mod progs_builtins_te; // pr_ext.c temp-entity builtins (M9f group D)
#[cfg(all(feature = "host", feature = "progs-host"))]
pub mod progs_builtins_zone; // pr_ext.c strzone/strunzone + knownzone
#[cfg(all(feature = "host", feature = "progs-host"))]
pub mod progs_edict_dispatch; // pr_edict.c ED_Parse* key dispatchers
#[cfg(feature = "progs")]
pub mod progs_exec; // pr_exec.c
#[cfg(feature = "progs")]
pub mod progs_load; // pr_edict_load.c
#[cfg(any(feature = "progs", feature = "progs-host"))]
pub mod progs_parse; // pr_edict_parse.c
#[cfg(feature = "progs")]
pub mod progs_save; // pr_edict_save.c
#[cfg(feature = "snd")]
pub mod snd_codec;
#[cfg(feature = "snd")]
pub mod snd_dma;
#[cfg(feature = "snd")]
pub mod snd_mem;
#[cfg(feature = "snd")]
pub mod snd_mix;
#[cfg(all(feature = "snd", feature = "codec-mp3"))]
pub mod snd_mp3tag;
#[cfg(all(feature = "snd", feature = "sdl3"))]
pub mod snd_sdl;
#[cfg(all(feature = "snd", feature = "codec-umx"))]
pub mod snd_umx;
#[cfg(feature = "snd")]
pub mod snd_wave;
#[cfg(feature = "fs")]
pub mod steam;
pub mod strl;
#[cfg(feature = "host")]
pub mod sv_main; // sv_main.c
#[cfg(feature = "host")]
pub mod sv_move; // sv_move.c
#[cfg(feature = "host")]
pub mod sv_phys; // sv_phys.c
#[cfg(feature = "host")]
pub mod sv_send; // sv_send.c
#[cfg(feature = "host")]
pub mod sv_user; // sv_user.c
#[cfg(feature = "host")]
pub mod view; // view.c
pub mod wad;
#[cfg(feature = "host")]
pub mod world; // world.c

/// Phase 0 link probe: proves the staticlib is linked and its symbols
/// resolve from C. Returns the quake-capi crate ABI version.
#[no_mangle]
pub extern "C" fn QuakeRS_Version() -> u32 {
    0
}
