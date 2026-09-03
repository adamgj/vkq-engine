//! `Quake/r_part_fte_glue.c` declarations (Rust migration Phase 7 M10f-2).
//!
//! ADR-011: engine C symbols are declared only in this crate. `r_part_fte.c`
//! is a Pattern A whole-file swap that is split rather than moved wholesale.
//! The simulation half -- `r_part_fte.c:88-5583` less `P_LoadTexture`, plus
//! the deferred-queue/particle-update block at `:6325-6784` -- becomes
//! `quake-capi/src/r_part_fte.rs`. The rendering and emit half
//! (`:5585-6324`), `P_LoadTexture` (`:1146-1327`) and everything from
//! `PScript_UpdateParticleTypes` (`:6786`) down stay C in the glue: they are
//! Vulkan-typed throughout (`VkBuffer`, `vulkan_memory_t`, `gltexture_t`) and
//! belong to Phase 8 per `ROADMAP.md`.
//!
//! ## Why so much state stays C
//!
//! ADR-007 keeps shared state in C whenever a live C reader survives the
//! port, and this file's seam is *bidirectional*: the C tail
//! (`PScript_UpdateParticleTypes` and the three task entry points) reads
//! objects that the Rust half writes. Concretely, code that stays C touches
//! `p_kill_list`/`p_kill_first` (`:6800`), `part_run_list` (`:6806`,
//! `:7210`), `free_decals` (`:6838`, `:6852`), `type_emit_meta`
//! (`:6979`, `:7341`), `free_beams` (`:7088`), `deferred_queues` (`:7226`),
//! `free_particles` (`:7288`), `particletime` (`:7295`), `part_type` (as the
//! base of the `type - part_type` index arithmetic at `:6979` and `:7341`),
//! `numparticletypes`, `particle_updates`/`num_particle_updates`
//! (`:7333-7341`) and `p_frametime` (`:7320`). All of those therefore stay
//! in the glue and Rust reaches them through the externs below plus the
//! mirrors in this module.
//!
//! `particle_traces_used` is an `atomic_uint32_t`. `Quake/atomics.h`'s
//! accessors are `static inline` with compiler-specific barriers, so it is
//! reached through the glue shims rather than re-derived with Rust orderings
//! -- the same call `quake-capi/src/host_cmd.rs:470-499` makes. Its two
//! companions, `particle_trace_limit` and `particle_update_seed`, are plain
//! `uint32_t` read only by the simulation, so they moved to Rust.
//!
//! `r_trace_line_cache_counter` (`r_part_fte.c:170`) stays a plain C global
//! with external linkage: `glquake.h:133-137` increments it inline from a
//! macro that other C translation units expand, and `crate::view` already
//! declares it as an `extern static mut` that `crate::cl_parse` and
//! `crate::view` take the address of.
//!
//! The whole `cl_stris` family (`:526-535`), `current_buffer_index`, the
//! Vulkan buffer arrays (`:5579-5583`), `pright`/`pup`,
//! `PScript_LooksUseWBOIT` (`:515`, used only at `:6810`, `:6982`, `:7020`),
//! `r_showtris` (`:497`) and `r_particles` (`:498`) are read only by the C
//! half, so Rust never names them.
//!
//! Moving to Rust: `psintable`/`pcostable`, the four pool bases and their
//! counters, `r_plooksdirty`, `loadedconfigs`, `avelocities`, the six
//! trace-line-cache objects, `p_doflurry`, `pe_default`/`pe_size2`/
//! `pe_size3`/`pe_defaulttrail` and `legacynames`. The sixteen `cvar_t`
//! objects stay C-owned in the glue, the same call `Quake/chase_glue.c` and
//! `Quake/r_part_glue.c` make.
//!
//! `Mem_Alloc`, `Mem_Free`, `Mem_Realloc`, `COM_LoadFile`, `COM_Parse`,
//! `COM_FileBase`, `COM_Rand`, `COM_ThreadToken`, `Cvar_RegisterVariable`,
//! `Cvar_SetCallback`, `Cmd_AddCommand2`, `Cmd_Argc`, `Cmd_Argv`, `Con_*`
//! and `Sys_Error` come from the generated bindings, so none of them is
//! re-declared here.

use crate::cvar_t;
use core::ffi::{c_char, c_double, c_float, c_int, c_uint, c_ulong, c_void};

/// `r_part_fte.c:167` -- `#define SINTABLE_ENTRIES 128`.
pub const SINTABLE_ENTRIES: usize = 128;
/// `quakedef.h` -- `#define MAX_QPATH 64`.
pub const MAX_QPATH: usize = 64;
/// `tasks.h` -- `#define TASKS_MAX_WORKERS 32`, the length of the
/// `deferred_queues` array (`r_part_fte.c:6317`).
pub const TASKS_MAX_WORKERS: usize = 32;

/// `r_part_fte.c:454` -- `#define MAX_BEAMSEGS (1 << 11)`.
pub const MAX_BEAMSEGS: c_int = 1 << 11;
/// `r_part_fte.c:455` -- `#define MAX_PARTICLES (1 << 18)`.
pub const MAX_PARTICLES: c_int = 1 << 18;
/// `r_part_fte.c:456` -- `#define MAX_DECALS (1 << 18)`.
pub const MAX_DECALS: c_int = 1 << 18;
/// `r_part_fte.c:457` -- `#define MAX_TRAILSTATES (1 << 10)`.
pub const MAX_TRAILSTATES: c_int = 1 << 10;

/// `r_part_fte.c:157` -- `#define BEF_LINES 1`.
pub const BEF_LINES: c_int = 1;

/// `r_part_fte.c:224` -- `#define BS_LASTSEG 0x1`.
pub const BS_LASTSEG: c_int = 0x1;
/// `r_part_fte.c:225` -- `#define BS_DEAD 0x2`.
pub const BS_DEAD: c_int = 0x2;
/// `r_part_fte.c:226` -- `#define BS_NODRAW 0x4`.
pub const BS_NODRAW: c_int = 0x4;

/// `r_part_fte.c:6584` -- `#define PARTICLE_UPDATE_CHUNK_SIZE 1024`.
pub const PARTICLE_UPDATE_CHUNK_SIZE: c_int = 1024;

/// `r_part_fte.c:128-138` -- `blendmode_t`. Eight non-negative enumerators,
/// so the implementation type is `int`.
#[allow(non_camel_case_types)]
pub type blendmode_t = c_int;

/// `r_part_fte.c:130`
pub const BM_BLEND: blendmode_t = 0;
/// `r_part_fte.c:131`
pub const BM_BLENDCOLOUR: blendmode_t = 1;
/// `r_part_fte.c:132`
pub const BM_ADDA: blendmode_t = 2;
/// `r_part_fte.c:133`
pub const BM_ADDC: blendmode_t = 3;
/// `r_part_fte.c:134`
pub const BM_SUBTRACT: blendmode_t = 4;
/// `r_part_fte.c:135`
pub const BM_INVMODA: blendmode_t = 5;
/// `r_part_fte.c:136`
pub const BM_INVMODC: blendmode_t = 6;
/// `r_part_fte.c:137`
pub const BM_PREMUL: blendmode_t = 7;

/// `r_part_fte.c:265-275` -- the anonymous `type` enum inside `plooks_t`.
#[allow(non_camel_case_types)]
pub type plooks_type_t = c_int;

/// `r_part_fte.c:267`
pub const PT_NORMAL: plooks_type_t = 0;
/// `r_part_fte.c:268`
pub const PT_SPARK: plooks_type_t = 1;
/// `r_part_fte.c:269`
pub const PT_SPARKFAN: plooks_type_t = 2;
/// `r_part_fte.c:270`
pub const PT_TEXTUREDSPARK: plooks_type_t = 3;
/// `r_part_fte.c:271`
pub const PT_BEAM: plooks_type_t = 4;
/// `r_part_fte.c:272`
pub const PT_CDECAL: plooks_type_t = 5;
/// `r_part_fte.c:273`
pub const PT_UDECAL: plooks_type_t = 6;
/// `r_part_fte.c:274`
pub const PT_INVISIBLE: plooks_type_t = 7;

/// `r_part_fte.c:377-389` -- the anonymous `spawnmode` enum inside
/// `part_type_t`.
#[allow(non_camel_case_types)]
pub type spawnmode_t = c_int;

/// `r_part_fte.c:379`
pub const SM_BOX: spawnmode_t = 0;
/// `r_part_fte.c:380`
pub const SM_CIRCLE: spawnmode_t = 1;
/// `r_part_fte.c:381`
pub const SM_BALL: spawnmode_t = 2;
/// `r_part_fte.c:382`
pub const SM_SPIRAL: spawnmode_t = 3;
/// `r_part_fte.c:383`
pub const SM_TRACER: spawnmode_t = 4;
/// `r_part_fte.c:384`
pub const SM_TELEBOX: spawnmode_t = 5;
/// `r_part_fte.c:385`
pub const SM_LAVASPLASH: spawnmode_t = 6;
/// `r_part_fte.c:386`
pub const SM_UNICIRCLE: spawnmode_t = 7;
/// `r_part_fte.c:387`
pub const SM_FIELD: spawnmode_t = 8;
/// `r_part_fte.c:388`
pub const SM_DISTBALL: spawnmode_t = 9;
/// `r_part_fte.c:389`
pub const SM_MESHSURFACE: spawnmode_t = 10;

/// `r_part_fte.c:405-411` -- the anonymous `rampmode` enum inside
/// `part_type_t`.
#[allow(non_camel_case_types)]
pub type rampmode_t = c_int;

/// `r_part_fte.c:407`
pub const RAMP_NONE: rampmode_t = 0;
/// `r_part_fte.c:408`
pub const RAMP_DELTA: rampmode_t = 1;
/// `r_part_fte.c:409`
pub const RAMP_NEAREST: rampmode_t = 2;
/// `r_part_fte.c:410`
pub const RAMP_LERP: rampmode_t = 3;

/// `r_part_fte.c:421` -- has velocity modifiers.
pub const PT_VELOCITY: c_uint = 0x0001;
/// `r_part_fte.c:422` -- has friction modifiers.
pub const PT_FRICTION: c_uint = 0x0002;
/// `r_part_fte.c:423`
pub const PT_CHANGESCOLOUR: c_uint = 0x0004;
/// `r_part_fte.c:424` -- Q1-style tracer behaviour for colorindex.
pub const PT_CITRACER: c_uint = 0x0008;
/// `r_part_fte.c:425` -- apply inverse frametime to count.
pub const PT_INVFRAMETIME: c_uint = 0x0010;
/// `r_part_fte.c:426` -- average trail points from start to end.
pub const PT_AVERAGETRAIL: c_uint = 0x0020;
/// `r_part_fte.c:427` -- don't use trailstate for this emitter.
pub const PT_NOSTATE: c_uint = 0x0040;
/// `r_part_fte.c:428` -- don't randomise org/vel for the first particle.
pub const PT_NOSPREADFIRST: c_uint = 0x0080;
/// `r_part_fte.c:429` -- don't randomise org/vel for the last particle.
pub const PT_NOSPREADLAST: c_uint = 0x0100;
/// `r_part_fte.c:430` -- don't spawn if underwater.
pub const PT_TROVERWATER: c_uint = 0x0200;
/// `r_part_fte.c:431` -- don't spawn if overwater.
pub const PT_TRUNDERWATER: c_uint = 0x0400;
/// `r_part_fte.c:432` -- dlights from this effect don't cast shadows.
pub const PT_NODLSHADOW: c_uint = 0x0800;
/// `r_part_fte.c:433` -- effect has orgwrand or velwrand properties.
pub const PT_WORLDSPACERAND: c_uint = 0x1000;

/// `r_part_fte.c:437` -- particle type is currently in the execution list.
pub const PS_INRUNLIST: c_uint = 0x1;

/// `r_part_fte.c:731`
pub const FTECONTENTS_EMPTY: c_uint = 0;
/// `r_part_fte.c:732`
pub const FTECONTENTS_SOLID: c_uint = 1;
/// `r_part_fte.c:733`
pub const FTECONTENTS_WATER: c_uint = 2;
/// `r_part_fte.c:734`
pub const FTECONTENTS_SLIME: c_uint = 4;
/// `r_part_fte.c:735`
pub const FTECONTENTS_LAVA: c_uint = 8;
/// `r_part_fte.c:736`
pub const FTECONTENTS_SKY: c_uint = 16;
/// `r_part_fte.c:737` -- `FTECONTENTS_FLUID`.
pub const FTECONTENTS_FLUID: c_uint =
    FTECONTENTS_WATER | FTECONTENTS_SLIME | FTECONTENTS_LAVA | FTECONTENTS_SKY;
/// `r_part_fte.c:738`
pub const FTECONTENTS_PLAYERCLIP: c_uint = 0;

/// ADR-011 mirror of `trailstate_t` (`r_part_fte.c:139-153`). Both anonymous
/// unions hold nothing but a `float` (`lastdist`/`statetime` and
/// `laststop`/`emittime`), so they are flattened to a single `c_float` each:
/// the layout is identical and the alias is not observable.
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct trailstate_t {
    /// key to check if ts has been overwritten
    pub key: *mut *mut trailstate_t,
    /// assoc linked trail
    pub assoc: *mut trailstate_t,
    /// last beam pointer (flagged with `BS_LASTSEG`)
    pub lastbeam: *mut beamseg_t,
    /// `lastdist` / `statetime`
    pub state1: c_float,
    /// `laststop` / `emittime`
    pub state2: c_float,
}

/// ADR-011 mirror of `particle_t`'s anonymous `state` union
/// (`r_part_fte.c:203-207`). Unlike `trailstate_t`'s, this one genuinely
/// mixes widths -- `float` against a pointer -- so it stays a `union`.
#[repr(C)]
#[allow(non_camel_case_types)]
pub union particle_state_t {
    pub nextemit: c_float,
    pub trailstate: *mut trailstate_t,
}

/// ADR-011 mirror of the FTE `particle_t` (`r_part_fte.c:189-210`). Note
/// this is `fparticle_t`: `r_part_fte.c:39-40` `#define`s `particle_s`/
/// `particle_t` to `fparticle_s`/`fparticle_t` so it does not collide with
/// `glquake.h`'s classic particle. `PScript_ClearParticles`
/// (`r_part_fte.c:3532`) walks the pool by pointer arithmetic, so `size_of`
/// has to be right.
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct particle_t {
    pub next: *mut particle_t,
    pub die: c_float,

    // driver-usable fields
    pub org: [c_float; 3],
    pub rgba: [c_float; 4],
    pub scale: c_float,
    pub s1: c_float,
    pub t1: c_float,
    pub s2: c_float,
    pub t2: c_float,

    /// to throttle traces
    pub oldorg: [c_float; 3],
    /// renderer uses for sparks
    pub vel: [c_float; 3],
    pub angle: c_float,
    pub state: particle_state_t,
    // drivers never touch the following fields
    pub rotationspeed: c_float,
}

/// ADR-011 mirror of `clippeddecal_t` (`r_part_fte.c:212-222`). `model` is a
/// `qmodel_t *` kept opaque here: this crate has no `[dependencies]` and
/// cannot name `quake-types`, and the field is only ever compared and stored.
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct clippeddecal_t {
    pub next: *mut clippeddecal_t,
    pub die: c_float,

    /// `>0` is a lerpentity, `<0` is a csqc ent, 0 is the world
    pub entity: c_int,
    /// `qmodel_t *` -- just for paranoia
    pub model: *mut c_void,

    pub vertex: [[c_float; 3]; 3],
    pub texcoords: [[c_float; 2]; 3],
    pub valpha: [c_float; 3],

    pub rgba: [c_float; 4],
}

/// ADR-011 mirror of `beamseg_t` (`r_part_fte.c:228-236`).
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct beamseg_t {
    /// next in beamseg list
    pub next: *mut beamseg_t,

    pub p: *mut particle_t,
    /// flags for beamseg
    pub flags: c_int,
    pub dir: [c_float; 3],

    pub texture_s: c_float,
}

/// ADR-011 mirror of `skytris_t` (`r_part_fte.c:238-248`). `face` is a
/// `struct msurface_s *`, kept opaque for the same reason as
/// [`clippeddecal_t::model`].
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct skytris_t {
    pub next: *mut skytris_t,
    pub org: [c_float; 3],
    pub x: [c_float; 3],
    pub y: [c_float; 3],
    pub area: c_float,
    pub nexttime: c_double,
    pub ptype: c_int,
    /// `struct msurface_s *`
    pub face: *mut c_void,
}

/// ADR-011 mirror of `skytriblock_t` (`r_part_fte.c:250-255`).
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct skytriblock_t {
    pub next: *mut skytriblock_t,
    pub count: c_uint,
    pub tris: [skytris_t; 1024],
}

/// ADR-011 mirror of `plooks_t` (`r_part_fte.c:263-286`) -- the static render
/// state for a particle. `texture` is a `gltexture_t *`, written only by
/// `P_LoadTexture`, which stays C.
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct plooks_t {
    pub type_: plooks_type_t,

    pub blendmode: blendmode_t,
    /// `gltexture_t *`
    pub texture: *mut c_void,
    pub nearest: bool,

    pub scalefactor: c_float,
    pub invscalefactor: c_float,
    pub stretch: c_float,
    /// limits the particle's length to a multiple of its width
    pub minstretch: c_float,
    /// 0: direct rgba. 1: rgb*a,a (blend). 2: rgb*a,0 (add).
    pub premul: c_int,
}

/// ADR-011 mirror of `ramp_t` (`r_part_fte.c:289-295`).
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct ramp_t {
    pub rgb: [c_float; 3],
    pub alpha: c_float,
    pub scale: c_float,
    pub rotation: c_float,
}

/// ADR-011 mirror of `partsounds_t` (`r_part_fte.c:296-304`).
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct partsounds_t {
    pub name: [c_char; MAX_QPATH],
    pub vol: c_float,
    pub atten: c_float,
    pub delay: c_float,
    pub pitch: c_float,
    pub weight: c_float,
}

/// ADR-011 mirror of `part_type_t` (`r_part_fte.c:306-438`), transcribed in
/// declaration order. `P_GetParticleType` (`r_part_fte.c:928`) rebases
/// `part_run_list` and every `nexttorun` after a `Mem_Realloc` of this array,
/// and `type - part_type` recovers the index at `:6979` and `:7341`, so
/// `size_of` is part of the seam contract.
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct part_type_t {
    pub name: [c_char; MAX_QPATH],
    pub config: [c_char; MAX_QPATH],
    pub texname: [c_char; MAX_QPATH],

    pub numsounds: c_int,
    pub sounds: *mut partsounds_t,

    /// initial colour
    pub rgb: [c_float; 3],
    pub alpha: c_float,
    /// colour delta (per second)
    pub rgbchange: [c_float; 3],
    pub alphachange: c_float,
    /// random rgb colour to start with
    pub rgbrand: [c_float; 3],
    pub alpharand: c_float,
    /// get colour from a palette
    pub colorindex: c_int,
    /// and add up to this amount
    pub colorrand: c_int,
    /// colour stops changing at this time
    pub rgbchangetime: c_float,
    /// like `rgbrand`, but a single random value instead of separate
    pub rgbrandsync: [c_float; 3],
    /// initial scale
    pub scale: c_float,
    /// with up to this much extra
    pub scalerand: c_float,
    /// how long it lasts
    pub die: c_float,
    pub randdie: c_float,
    /// scale the incoming velocity by this much
    pub veladd: c_float,
    pub randomveladd: c_float,
    /// spawn the particle this far along its velocity direction
    pub orgadd: c_float,
    pub randomorgadd: c_float,
    /// spawn the particle with a velocity based upon its spawn type
    pub spawnvel: c_float,
    pub spawnvelvert: c_float,
    /// static 3d world-coord bias
    pub orgbias: [c_float; 3],
    pub velbias: [c_float; 3],
    /// 3d world-coord randomisation without relation to spawn mode
    pub orgwrand: [c_float; 3],
    /// 3d world-coord randomisation without relation to spawn mode
    pub velwrand: [c_float; 3],
    pub viewspacefrac: c_float,
    pub flurry: c_float,
    /// this decal only spawns on these surfaces
    pub surfflagmatch: c_int,
    /// this decal only spawns on these surfaces
    pub surfflagmask: c_int,

    // texture coords
    pub s1: c_float,
    pub t1: c_float,
    pub s2: c_float,
    pub t2: c_float,
    /// addition for s for each random slot
    pub texsstride: c_float,
    /// max times the stride can be added
    pub randsmax: c_int,

    /// shared looks, so state switches don't apply between particles so much
    pub slooks: *mut plooks_t,
    pub looks: plooks_t,

    /// time limit for trails
    pub spawntime: c_float,
    /// if `< 0`, particles might not spawn so many
    pub spawnchance: c_float,

    pub rotationstartmin: c_float,
    pub rotationstartrand: c_float,
    pub rotationmin: c_float,
    pub rotationrand: c_float,

    pub scaledelta: c_float,
    pub countextra: c_float,
    pub count: c_float,
    pub countrand: c_float,
    /// for trails
    pub countspacing: c_float,
    /// for badly-designed effects, instead of depending on trail state
    pub countoverflow: c_float,
    /// surface emitter multiplier
    pub rainfrequency: c_float,

    pub assoc: c_int,
    pub cliptype: c_int,
    pub inwater: c_int,
    pub clipcount: c_float,
    pub emit: c_int,
    pub emittime: c_float,
    pub emitrand: c_float,
    pub emitstart: c_float,

    pub areaspread: c_float,
    pub areaspreadvert: c_float,

    pub spawnparam1: c_float,
    pub spawnparam2: c_float,

    pub spawnmode: spawnmode_t,

    pub gravity: c_float,
    pub friction: [c_float; 3],
    pub clipbounce: c_float,
    pub stainonimpact: c_float,

    pub dl_rgb: [c_float; 3],
    pub dl_radius: [c_float; 2],
    pub dl_time: c_float,
    pub dl_decay: [c_float; 4],
    pub dl_corona_intensity: c_float,
    pub dl_corona_scale: c_float,
    pub dl_scales: [c_float; 3],
    pub dl_cubemapnum: c_int,

    pub rampmode: rampmode_t,
    pub rampindexes: c_int,
    pub ramp: *mut ramp_t,

    /// 0 if not loaded, 1 if automatically loaded, 2 if user loaded
    pub loaded: c_int,
    pub particles: *mut particle_t,
    pub clippeddecals: *mut clippeddecal_t,
    pub beams: *mut beamseg_t,
    pub nexttorun: *mut part_type_t,

    pub flags: c_uint,
    pub fluidmask: c_uint,

    pub state: c_uint,
}

/// ADR-011 mirror of `pcfg_t` (`r_part_fte.c:443-447`). The `name[1]` tail is
/// a C flexible-array idiom: `P_LoadParticleSet` allocates
/// `sizeof (pcfg_t) + strlen (name)` and copies the name in, so Rust treats
/// the field as the first byte of a variable-length string.
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct pcfg_t {
    pub next: *mut pcfg_t,
    pub name: [c_char; 1],
}

/// ADR-011 mirror of `deferred_effect_t` (`r_part_fte.c:6254-6259`).
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct deferred_effect_t {
    pub org: [c_float; 3],
    pub dir: [c_float; 3],
    pub count: c_float,
    pub type_: c_int,
}

/// ADR-011 mirror of `deferred_trail_t` (`r_part_fte.c:6261-6267`).
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct deferred_trail_t {
    pub start: [c_float; 3],
    pub end: [c_float; 3],
    pub type_: c_int,
    pub tsk: *mut *mut trailstate_t,
}

/// ADR-011 mirror of `deferred_decal_t` (`r_part_fte.c:6269-6276`).
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct deferred_decal_t {
    pub type_: *mut part_type_t,
    pub entity: c_int,
    pub center: [c_float; 3],
    pub normal: [c_float; 3],
    pub scale: c_float,
}

/// ADR-011 mirror of `deferred_dlight_t` (`r_part_fte.c:6278-6286`).
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct deferred_dlight_t {
    pub key: c_int,
    pub org: [c_float; 3],
    pub radius: c_float,
    pub die: c_float,
    pub decay: c_float,
    pub rgb: [c_float; 3],
}

/// ADR-011 mirror of `decalctx_t` (`r_part_fte.c:3928-3944`), the context
/// `Mod_ClipDecal` hands back to `PScript_AddDecals`. The decal clipper
/// family (`r_part_fte.c:3928-4308`) stays C in the glue -- every object it
/// touches (`free_decals`, `ptype->clippeddecals`, `part_run_list`,
/// `particletime`, `d_8to24table`) is already C-owned on the render side of
/// the bidirectional seam -- so Rust only ever builds this struct and passes
/// it through `FtePart_Glue_ClipDecal`.
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct decalctx_t {
    pub ptype: *mut part_type_t,
    pub entity: c_int,
    pub model: *mut c_void,
    pub center: [c_float; 3],
    pub normal: [c_float; 3],
    pub tangent1: [c_float; 3],
    pub tangent2: [c_float; 3],

    pub scale0: c_float,
    pub scale1: c_float,
    pub scale2: c_float,

    pub bias1: c_float,
    pub bias2: c_float,
}

/// ADR-011 mirror of `particle_update_t` (`r_part_fte.c:6288-6292`).
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct particle_update_t {
    pub p: *mut particle_t,
    pub type_: *mut part_type_t,
}

/// ADR-011 mirror of `deferred_queues_t` (`r_part_fte.c:6305-6316`), the
/// per-worker deferral buffers. The array lives in the glue because
/// `PScript_UpdateParticleTypes` (`:7226`) drains it from C.
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct deferred_queues_t {
    pub effects: *mut deferred_effect_t,
    pub num_effects: c_int,
    pub max_effects: c_int,

    pub trails: *mut deferred_trail_t,
    pub num_trails: c_int,
    pub max_trails: c_int,

    pub decals: *mut deferred_decal_t,
    pub num_decals: c_int,
    pub max_decals: c_int,

    pub dlights: *mut deferred_dlight_t,
    pub num_dlights: c_int,
    pub max_dlights: c_int,
}

/// ADR-011 mirror of `particle_emit_meta_t` (`r_part_fte.c:6420-6427`). The
/// `emit_core` member is a pointer to one of the emit functions that stay C,
/// so Rust only ever zeroes or copies it and keeps it opaque.
#[repr(C)]
#[allow(non_camel_case_types)]
pub struct particle_emit_meta_t {
    pub start: c_int,
    pub count: c_int,
    pub first_stri: c_int,
    pub vpp: c_int,
    pub ipp: c_int,
    pub ppb: c_int,
    /// `void (*emit_core) (scenetris_t *, particle_t *, plooks_t *, unsigned, unsigned)`
    pub emit_core: *mut c_void,
}

extern "C" {
    /* ---------------------------------------------------------------------
     * Glue-owned storage. Every one of these has a live C reader in the
     * rendering/emit half or in `PScript_UpdateParticleTypes` (ADR-007);
     * the module docs above cite the line for each.
     */

    /// `r_part_fte.c:168` -- `static float psintable[SINTABLE_ENTRIES]`.
    /// Glue-owned (ADR-007): the render half that stays C reaches it through
    /// the same `sin` macro (`r_part_fte.c:186`) at `:6068` and `:6159`.
    pub static mut psintable: [c_float; SINTABLE_ENTRIES];
    /// `r_part_fte.c:169` -- `static float pcostable[SINTABLE_ENTRIES]`, read
    /// by the C render half through the `cos` macro at `:6069` and `:6160`.
    pub static mut pcostable: [c_float; SINTABLE_ENTRIES];

    /// `r_part_fte.c:761` -- `static part_type_t *part_type`, the
    /// `Mem_Realloc`'d type array. `type - part_type` recovers the index at
    /// `:6979` and `:7341`.
    pub static mut part_type: *mut part_type_t;
    /// `r_part_fte.c:760` -- `static int numparticletypes`.
    pub static mut numparticletypes: c_int;
    /// `r_part_fte.c:762` -- `static part_type_t *part_run_list`, the
    /// `PS_INRUNLIST` chain drained by `PScript_UpdateParticleTypes`
    /// (`:6806`, `:7210`).
    pub static mut part_run_list: *mut part_type_t;
    /// `r_part_fte.c:500` -- `static float particletime`, advanced by the C
    /// tail at `:7295`.
    pub static mut particletime: c_float;

    /// `r_part_fte.c:459` -- head of the particle free list, refilled by the
    /// C tail at `:7288`. The glue renames it (`#define free_particles
    /// fte_free_particles`, mirroring `r_part_fte.c:40-41`'s `#define
    /// particle_t fparticle_t`) because `Quake/r_part_glue.c:75` already
    /// defines an unrelated external `free_particles`.
    #[link_name = "fte_free_particles"]
    pub static mut free_particles: *mut particle_t;
    /// `r_part_fte.c:465` -- head of the beamseg free list (`:7088`).
    pub static mut free_beams: *mut beamseg_t;
    /// `r_part_fte.c:469` -- head of the decal free list (`:6838`, `:6852`).
    pub static mut free_decals: *mut clippeddecal_t;

    /// `r_part_fte.c:6586` -- `static particle_t *p_kill_list`, read by the C
    /// tail at `:6800`.
    pub static mut p_kill_list: *mut particle_t;
    /// `r_part_fte.c:6587` -- `static particle_t *p_kill_first`.
    pub static mut p_kill_first: *mut particle_t;
    /// `r_part_fte.c:6585` -- `static float p_frametime`, the frametime the C
    /// tail hands `PScript_UpdateParticleTypes` at `:7320`.
    pub static mut p_frametime: c_float;

    /// `r_part_fte.c:6428` -- `static particle_emit_meta_t *type_emit_meta`,
    /// indexed by `type - part_type` at `:6979` and `:7341`.
    pub static mut type_emit_meta: *mut particle_emit_meta_t;
    /// `r_part_fte.c:6428` -- `static int num_type_emit_meta`.
    pub static mut num_type_emit_meta: c_int;

    /// `r_part_fte.c:6321` -- `static particle_update_t *particle_updates`,
    /// walked by `PScript_EmitParticlesTask` (`:7333-7341`).
    pub static mut particle_updates: *mut particle_update_t;
    /// `r_part_fte.c:6322` -- `static int num_particle_updates`.
    pub static mut num_particle_updates: c_int;
    /// `r_part_fte.c:6322` -- `static int max_particle_updates`.
    pub static mut max_particle_updates: c_int;

    /// `r_part_fte.c:6317` --
    /// `static deferred_queues_t deferred_queues[TASKS_MAX_WORKERS]`, drained
    /// by the C tail at `:7226`.
    pub static mut deferred_queues: [deferred_queues_t; TASKS_MAX_WORKERS];

    /// `r_part_fte.c:5701` -- `static vec3_t pright, pup`, the 1.5-scaled
    /// view axes. Written by `PScript_UpdateParticlesSetupTask` on the Rust
    /// side and read by every sprite emitter in the C half, so the storage
    /// stays glue-owned (ADR-007).
    pub static mut pright: [c_float; 3];
    /// See [`pright`].
    pub static mut pup: [c_float; 3];

    /// `r_part_fte.c:6323` -- `static atomic_uint32_t particle_traces_used`.
    /// Only ever touched through [`FtePart_Glue_AtomicIncrementU32`] and
    /// [`FtePart_Glue_AtomicStoreU32`]; see the module docs.
    pub static mut particle_traces_used: c_uint;

    /// `r_part_fte.c:31` -- `cvar_t r_fteparticles`, external linkage,
    /// declared in `glquake.h`.
    pub static mut r_fteparticles: cvar_t;
    /// `r_part_fte.c:485` -- `cvar_t r_particledesc`, external linkage. The
    /// glue owns it because `PScript_Shutdown`'s `Cvar_SetCallback` and the
    /// callback registration both cross the seam.
    pub static mut r_particledesc: cvar_t;

    /* The fourteen remaining file-static cvars (`r_part_fte.c:482-496`).
     * They stay C-owned in the glue for the reason `Quake/chase_glue.c` and
     * `Quake/r_part_glue.c` keep theirs there: `cvar_t` is a C ABI object
     * that `Cvar_RegisterVariable` links into the engine's hash chain and
     * mutates thereafter, so its storage has to be the C definition the
     * registration walked. Every read below is from the ported half.
     */

    /// `r_part_fte.c:482` -- `static cvar_t r_bouncysparks`.
    pub static mut r_bouncysparks: cvar_t;
    /// `r_part_fte.c:483` -- `static cvar_t r_part_rain`.
    pub static mut r_part_rain: cvar_t;
    /// `r_part_fte.c:484` -- `static cvar_t r_decal_noperpendicular`.
    pub static mut r_decal_noperpendicular: cvar_t;
    /// `r_part_fte.c:486` -- `static cvar_t r_part_rain_quantity`.
    pub static mut r_part_rain_quantity: cvar_t;
    /// `r_part_fte.c:487` -- `static cvar_t r_particle_tracelimit`.
    pub static mut r_particle_tracelimit: cvar_t;
    /// `r_part_fte.c:488` -- `static cvar_t r_part_sparks`.
    pub static mut r_part_sparks: cvar_t;
    /// `r_part_fte.c:489` -- `static cvar_t r_part_sparks_trifan`.
    pub static mut r_part_sparks_trifan: cvar_t;
    /// `r_part_fte.c:490` -- `static cvar_t r_part_sparks_textured`.
    pub static mut r_part_sparks_textured: cvar_t;
    /// `r_part_fte.c:491` -- `static cvar_t r_part_beams`.
    pub static mut r_part_beams: cvar_t;
    /// `r_part_fte.c:492` -- `static cvar_t r_part_contentswitch`.
    pub static mut r_part_contentswitch: cvar_t;
    /// `r_part_fte.c:493` -- `static cvar_t r_part_density`.
    pub static mut r_part_density: cvar_t;
    /// `r_part_fte.c:494` -- `static cvar_t r_part_maxparticles`.
    pub static mut r_part_maxparticles: cvar_t;
    /// `r_part_fte.c:495` -- `static cvar_t r_part_maxdecals`.
    pub static mut r_part_maxdecals: cvar_t;
    /// `r_part_fte.c:496` -- `static cvar_t r_lightflicker`.
    pub static mut r_lightflicker: cvar_t;

    /// `r_part_fte.c:3301` -- `extern qmodel_t mod_known[]`'s length, walked
    /// by `PScript_ClearAllSurfaceParticles`.
    pub static mut mod_numknown: c_int;

    /// `host.c` -- `double realtime`, sampled by `PScript_EffectSpawned`
    /// (`r_part_fte.c:3880`) to drive the dynamic-light flicker cadence.
    pub static mut realtime: c_double;

    /* ---------------------------------------------------------------------
     * `Host_Guard` trampolines (ADR-009 rule 3). Each wraps a callee that
     * can itself re-raise, so no C `longjmp` crosses a Rust frame.
     */

    /// `Quake/r_part_fte_glue.c` -- `Host_Guard` over
    /// `Cvar_RegisterVariable`, called sixteen times by
    /// `PScript_InitParticles` (`r_part_fte.c:3266-3281`).
    /// `cvar_cmd_glue.c:300` makes it a `Host_Reraise` wrapper.
    pub fn FtePart_Glue_RegisterVariable(var: *mut cvar_t) -> c_int;

    /// `Quake/r_part_fte_glue.c` -- `Host_Guard` over `CL_ClearTrailStates`
    /// (`r_part_fte.c:3315`, `:3532`). `cl_main_glue.c:839` makes it a
    /// `ClMain_Raise` wrapper.
    pub fn FtePart_Glue_ClearTrailStates() -> c_int;

    /// `Quake/r_part_fte_glue.c` -- `Host_Guard` over `CL_RegisterParticles`
    /// (`r_part_fte.c:3644`, `:6645`). `cl_parse_glue.c:625` makes it a
    /// `ClParse_Raise` wrapper.
    pub fn FtePart_Glue_RegisterParticles() -> c_int;

    /// `Quake/r_part_fte_glue.c` -- `Host_Guard` over `CL_EntityNum`
    /// (`r_part_fte.c:4458`). `cl_parse_glue.c:596` makes it a `ClParse_Raise`
    /// wrapper. On success the entity pointer lands in `*out`; on a non-zero
    /// return `*out` is untouched and the caller must not read it (ADR-009's
    /// post-guard invariant).
    pub fn FtePart_Glue_EntityNum(num: c_int, out: *mut *mut c_void) -> c_int;

    /* ---------------------------------------------------------------------
     * Glue shims over the half that stays C.
     */

    /// `Quake/r_part_fte_glue.c` -- external-linkage wrapper over
    /// `r_part_fte.c:1146`'s `static void P_LoadTexture (part_type_t *,
    /// qboolean)`, which is `gltexture_t`-typed and stays C. Called from
    /// `FinishParticleType` (`:2719`) and `PScript_Startup`'s reload path
    /// (`:3520`).
    pub fn FtePart_Glue_LoadTexture(ptype: *mut part_type_t, warn: bool);

    /// `Quake/r_part_fte_glue.c` -- stable C function pointer for
    /// `r_part_fte.c:857`'s `P_PartRedirect_f`, so a raise out of the command
    /// unwinds through C rather than a Rust frame.
    pub fn FtePart_Glue_PartRedirect_f();
    /// `Quake/r_part_fte_glue.c` -- ditto for `r_part_fte.c:2629`'s
    /// `P_PartInfo_f`.
    pub fn FtePart_Glue_PartInfo_f();
    /// `Quake/r_part_fte_glue.c` -- ditto for `r_part_fte.c:2595`'s
    /// `P_BeamInfo_f`.
    pub fn FtePart_Glue_BeamInfo_f();
    /// `Quake/r_part_fte_glue.c` -- ditto for `r_part_fte.c:3622`'s
    /// `R_ParticleDesc_Callback`, handed to `Cvar_SetCallback`.
    pub fn FtePart_Glue_ParticleDesc_Callback(var: *mut cvar_t);

    /// `Quake/r_part_fte_glue.c` -- wrapper over `Mod_ClipDecal`
    /// (`r_part_fte.c:4271`). Both call sites (`:4506` in Rust, `:7279` in
    /// the C tail) pass the same `PScript_AddDecals` callback, so the
    /// function pointer is not a real FFI parameter: the glue supplies it.
    #[allow(clippy::too_many_arguments)]
    pub fn FtePart_Glue_ClipDecal(
        model: *mut c_void,
        center: *mut c_float,
        normal: *mut c_float,
        tangent1: *mut c_float,
        tangent2: *mut c_float,
        size: c_float,
        surfflagmask: c_uint,
        surfflagmatch: c_uint,
        ctx: *mut c_void,
    );

    /// `Quake/r_part_fte_glue.c` -- `&mod_known[i]` as a `void *`.
    /// `PScript_ClearAllSurfaceParticles` (`r_part_fte.c:3301`) walks the
    /// model cache; going through the glue keeps the port free of any
    /// dependency on `sizeof (qmodel_t)`.
    pub fn FtePart_Glue_ModKnown(i: c_int) -> *mut c_void;

    /// `Quake/r_part_fte_glue.c` -- `Atomic_IncrementUInt32` over
    /// `particle_traces_used` (`r_part_fte.c:6539`).
    pub fn FtePart_Glue_AtomicIncrementU32(atomic: *mut c_void) -> c_uint;
    /// `Quake/r_part_fte_glue.c` -- `Atomic_StoreUInt32` over
    /// `particle_traces_used` (`r_part_fte.c:6761`).
    pub fn FtePart_Glue_AtomicStoreU32(atomic: *mut c_void, desired: c_uint);

    /* ---------------------------------------------------------------------
     * Everything else the simulation half reaches. None of these re-raises
     * (M10f-2 recon section 4), so ADR-009 rule 4 calls them straight
     * through.
     */

    /// `cmd.h` -- `void Cmd_TokenizeString (const char *text)`.
    pub fn Cmd_TokenizeString(text: *const c_char);
    /// `common.h` -- `const char *va (const char *format, ...)`.
    pub fn va(format: *const c_char, ...) -> *mut c_char;
    /// `common.c:633` -- `int q_snprintf (char *, size_t, const char *, ...)`.
    pub fn q_snprintf(str_: *mut c_char, size: usize, format: *const c_char, ...) -> c_int;
    /// `common.c` -- `size_t q_strlcpy (char *dst, const char *src, size_t siz)`.
    pub fn q_strlcpy(dst: *mut c_char, src: *const c_char, siz: usize) -> usize;

    /// `sound.h` -- `void S_StartSound (int entnum, int entchannel, sfx_t *sfx, vec3_t origin, float fvol, float attenuation)`.
    pub fn S_StartSound(
        entnum: c_int,
        entchannel: c_int,
        sfx: *mut c_void,
        origin: *mut c_float,
        fvol: c_float,
        attenuation: c_float,
    );
    /// `sound.h` -- `sfx_t *S_PrecacheSound (const char *sample)`.
    pub fn S_PrecacheSound(sample: *const c_char) -> *mut c_void;

    /// `server.h` -- `int SV_HullPointContents (hull_t *hull, int num, vec3_t p)`.
    pub fn SV_HullPointContents(hull: *mut c_void, num: c_int, p: *mut c_float) -> c_int;
    /// `world.h` -- `qboolean Q1BSP_RecursiveHullTrace (hull_t *hull, int num, float p1f, float p2f, vec3_t p1, vec3_t p2, struct trace_s *trace)`.
    pub fn Q1BSP_RecursiveHullTrace(
        hull: *mut c_void,
        num: c_int,
        p1f: c_float,
        p2f: c_float,
        p1: *mut c_float,
        p2: *mut c_float,
        trace: *mut c_void,
    ) -> bool;

    /// `tasks.h` -- `int Tasks_GetWorkerIndex (void)`.
    pub fn Tasks_GetWorkerIndex() -> c_int;
    /// `tasks.h` -- `int Tasks_NumWorkers (void)`.
    pub fn Tasks_NumWorkers() -> c_int;
    /// `tasks.h` -- `qboolean Tasks_IsWorker (void)`.
    pub fn Tasks_IsWorker() -> bool;

    /// libc `int atoi (const char *)`.
    pub fn atoi(s: *const c_char) -> c_int;
    /// libc `double atof (const char *)`.
    pub fn atof(s: *const c_char) -> c_double;

    /// `strl_fn.h` -- `size_t q_strlcat (char *dst, const char *src, size_t siz)`.
    pub fn q_strlcat(dst: *mut c_char, src: *const c_char, siz: usize) -> usize;
    /// `q_ctype.h` -- `int q_strcasecmp (const char *, const char *)`.
    pub fn q_strcasecmp(s1: *const c_char, s2: *const c_char) -> c_int;
    /// `q_ctype.h` -- `int q_strncasecmp (const char *, const char *, size_t)`.
    pub fn q_strncasecmp(s1: *const c_char, s2: *const c_char, n: usize) -> c_int;

    /// libc `int strcmp (const char *, const char *)`.
    pub fn strcmp(s1: *const c_char, s2: *const c_char) -> c_int;
    /// libc `int strncmp (const char *, const char *, size_t)`.
    pub fn strncmp(s1: *const c_char, s2: *const c_char, n: usize) -> c_int;
    /// libc `char *strcpy (char *, const char *)`.
    pub fn strcpy(dst: *mut c_char, src: *const c_char) -> *mut c_char;
    /// libc `size_t strlen (const char *)`.
    pub fn strlen(s: *const c_char) -> usize;
    /// libc `char *strstr (const char *, const char *)`.
    pub fn strstr(haystack: *const c_char, needle: *const c_char) -> *mut c_char;
    /// libc `char *strchr (const char *, int)`.
    pub fn strchr(s: *const c_char, c: c_int) -> *mut c_char;
    /// libc `char *strrchr (const char *, int)`.
    pub fn strrchr(s: *const c_char, c: c_int) -> *mut c_char;
    /// libc `void *memcpy (void *, const void *, size_t)`.
    pub fn memcpy(dst: *mut c_void, src: *const c_void, n: usize) -> *mut c_void;
    /// libc `void *memmove (void *, const void *, size_t)`.
    pub fn memmove(dst: *mut c_void, src: *const c_void, n: usize) -> *mut c_void;
    /// libc `void *memset (void *, int, size_t)`.
    pub fn memset(s: *mut c_void, c: c_int, n: usize) -> *mut c_void;
    /// libc `unsigned long strtoul (const char *, char **, int)`.
    pub fn strtoul(s: *const c_char, end: *mut *mut c_char, base: c_int) -> c_ulong;
    /// libc `double strtod (const char *, char **)`.
    pub fn strtod(s: *const c_char, end: *mut *mut c_char) -> f64;

    /// `gl_texmgr.h:103` -- `extern unsigned int d_8to24table[256]`, the
    /// palette the effect parser reads `palrgba` bytes out of.
    pub static mut d_8to24table: [c_uint; 256];
}
