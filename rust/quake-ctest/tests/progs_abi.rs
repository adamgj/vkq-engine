//! ABI cross-check: the `quake_types::progs` mirrors vs what the engine's own
//! headers (`pr_comp.h`, `progdefs.q1`, `progs.h`) say on this platform and
//! build profile (Phase 6).
//!
//! Under `-Duse_rust_progs` the Rust VM shares the C-owned `qcvm_t` embedded
//! in `sv`/`cl`, strides the C-allocated edict array by `qcvm->edict_size`,
//! and calls C builtins out of `qcvm->builtins[]`, so mirror drift is silent
//! memory corruption rather than a link error. `progs.h` is not a
//! bindgen-clean root, so the mirrors are hand-written (ADR-011) and this
//! probe, compiled from the engine's own headers, is the per-platform gate.
//!
//! `edict_t`'s layout forks on `DEBUG`/`_DEBUG`, so the suite first asserts
//! that the Rust `engine-debug` feature agrees with how the probe TU was
//! compiled — otherwise every edict offset below would be checked against the
//! wrong C layout.

use core::mem::{offset_of, size_of};

use quake_ctest as _;
use quake_types::progs::{
    self, AreaNode, BuiltinT, DDef, DFunction, DPrograms, DStatement, Edict, EntVars, EntityState,
    FreeList, GlobalVars, Link, PrExtFields, PrExtFuncs, PrExtGlobals, PrStack, QcVm,
};

extern "C" {
    fn ctest_abi_progs_lookup(key: *const core::ffi::c_char) -> usize;
}

fn c_abi(key: &str) -> usize {
    let cstr = std::ffi::CString::new(key).unwrap();
    // SAFETY: the probe only strcmp's the key against a compile-time table.
    let v = unsafe { ctest_abi_progs_lookup(cstr.as_ptr()) };
    assert_ne!(v, usize::MAX, "key {key:?} missing from the C probe table");
    v
}

macro_rules! check_size {
    ($rust:ty, $ctag:literal) => {
        assert_eq!(
            size_of::<$rust>(),
            c_abi(concat!("sizeof.", $ctag)),
            concat!("sizeof ", $ctag)
        );
    };
}

macro_rules! check_offset {
    ($rust:ty, $field:ident, $ckey:literal) => {
        assert_eq!(offset_of!($rust, $field), c_abi($ckey), $ckey);
    };
}

/// The `engine-debug` cargo feature must track the C `DEBUG`/`_DEBUG`, or the
/// edict header is three fields out and every later assertion is meaningless.
#[test]
fn engine_debug_feature_matches_the_c_build() {
    let c_debug = c_abi("const.ENGINE_DEBUG") == 1;
    let rust_debug = progs::ENGINE_DEBUG;
    assert_eq!(
        rust_debug, c_debug,
        "quake-types `engine-debug` feature ({rust_debug}) disagrees with the C \
         DEBUG/_DEBUG state of the probe TU ({c_debug}); edict_t's layout forks on it"
    );
}

#[test]
fn progs_file_format_mirrors_match_engine_headers() {
    check_size!(DStatement, "dstatement_t");
    check_offset!(DStatement, op, "dstatement_t.op");
    check_offset!(DStatement, a, "dstatement_t.a");
    check_offset!(DStatement, b, "dstatement_t.b");
    check_offset!(DStatement, c, "dstatement_t.c");

    check_size!(DDef, "ddef_t");
    check_offset!(DDef, type_, "ddef_t.type");
    check_offset!(DDef, ofs, "ddef_t.ofs");
    check_offset!(DDef, s_name, "ddef_t.s_name");

    check_size!(DFunction, "dfunction_t");
    check_offset!(DFunction, first_statement, "dfunction_t.first_statement");
    check_offset!(DFunction, parm_start, "dfunction_t.parm_start");
    check_offset!(DFunction, locals, "dfunction_t.locals");
    check_offset!(DFunction, profile, "dfunction_t.profile");
    check_offset!(DFunction, s_name, "dfunction_t.s_name");
    check_offset!(DFunction, s_file, "dfunction_t.s_file");
    check_offset!(DFunction, numparms, "dfunction_t.numparms");
    check_offset!(DFunction, parm_size, "dfunction_t.parm_size");

    check_size!(DPrograms, "dprograms_t");
    check_offset!(DPrograms, version, "dprograms_t.version");
    check_offset!(DPrograms, crc, "dprograms_t.crc");
    check_offset!(DPrograms, ofs_statements, "dprograms_t.ofs_statements");
    check_offset!(DPrograms, numstatements, "dprograms_t.numstatements");
    check_offset!(DPrograms, ofs_globaldefs, "dprograms_t.ofs_globaldefs");
    check_offset!(DPrograms, numglobaldefs, "dprograms_t.numglobaldefs");
    check_offset!(DPrograms, ofs_fielddefs, "dprograms_t.ofs_fielddefs");
    check_offset!(DPrograms, numfielddefs, "dprograms_t.numfielddefs");
    check_offset!(DPrograms, ofs_functions, "dprograms_t.ofs_functions");
    check_offset!(DPrograms, numfunctions, "dprograms_t.numfunctions");
    check_offset!(DPrograms, ofs_strings, "dprograms_t.ofs_strings");
    check_offset!(DPrograms, numstrings, "dprograms_t.numstrings");
    check_offset!(DPrograms, ofs_globals, "dprograms_t.ofs_globals");
    check_offset!(DPrograms, numglobals, "dprograms_t.numglobals");
    check_offset!(DPrograms, entityfields, "dprograms_t.entityfields");
}

/// Every field, not a spot check: `globalvars_t` and `entvars_t` are almost
/// entirely same-typed words, so a transposition of two neighbours would leave
/// the struct size unchanged and slip past a size-only assertion — while
/// silently rewiring which QuakeC variable the engine reads.
#[test]
fn progdefs_mirrors_match_engine_headers() {
    check_size!(GlobalVars, "globalvars_t");
    check_offset!(GlobalVars, pad, "globalvars_t.pad");
    check_offset!(GlobalVars, self_, "globalvars_t.self");
    check_offset!(GlobalVars, other, "globalvars_t.other");
    check_offset!(GlobalVars, world, "globalvars_t.world");
    check_offset!(GlobalVars, time, "globalvars_t.time");
    check_offset!(GlobalVars, frametime, "globalvars_t.frametime");
    check_offset!(GlobalVars, force_retouch, "globalvars_t.force_retouch");
    check_offset!(GlobalVars, mapname, "globalvars_t.mapname");
    check_offset!(GlobalVars, deathmatch, "globalvars_t.deathmatch");
    check_offset!(GlobalVars, coop, "globalvars_t.coop");
    check_offset!(GlobalVars, teamplay, "globalvars_t.teamplay");
    check_offset!(GlobalVars, serverflags, "globalvars_t.serverflags");
    check_offset!(GlobalVars, total_secrets, "globalvars_t.total_secrets");
    check_offset!(GlobalVars, total_monsters, "globalvars_t.total_monsters");
    check_offset!(GlobalVars, found_secrets, "globalvars_t.found_secrets");
    check_offset!(GlobalVars, killed_monsters, "globalvars_t.killed_monsters");
    check_offset!(GlobalVars, parm1, "globalvars_t.parm1");
    check_offset!(GlobalVars, parm2, "globalvars_t.parm2");
    check_offset!(GlobalVars, parm3, "globalvars_t.parm3");
    check_offset!(GlobalVars, parm4, "globalvars_t.parm4");
    check_offset!(GlobalVars, parm5, "globalvars_t.parm5");
    check_offset!(GlobalVars, parm6, "globalvars_t.parm6");
    check_offset!(GlobalVars, parm7, "globalvars_t.parm7");
    check_offset!(GlobalVars, parm8, "globalvars_t.parm8");
    check_offset!(GlobalVars, parm9, "globalvars_t.parm9");
    check_offset!(GlobalVars, parm10, "globalvars_t.parm10");
    check_offset!(GlobalVars, parm11, "globalvars_t.parm11");
    check_offset!(GlobalVars, parm12, "globalvars_t.parm12");
    check_offset!(GlobalVars, parm13, "globalvars_t.parm13");
    check_offset!(GlobalVars, parm14, "globalvars_t.parm14");
    check_offset!(GlobalVars, parm15, "globalvars_t.parm15");
    check_offset!(GlobalVars, parm16, "globalvars_t.parm16");
    check_offset!(GlobalVars, v_forward, "globalvars_t.v_forward");
    check_offset!(GlobalVars, v_up, "globalvars_t.v_up");
    check_offset!(GlobalVars, v_right, "globalvars_t.v_right");
    check_offset!(GlobalVars, trace_allsolid, "globalvars_t.trace_allsolid");
    check_offset!(
        GlobalVars,
        trace_startsolid,
        "globalvars_t.trace_startsolid"
    );
    check_offset!(GlobalVars, trace_fraction, "globalvars_t.trace_fraction");
    check_offset!(GlobalVars, trace_endpos, "globalvars_t.trace_endpos");
    check_offset!(
        GlobalVars,
        trace_plane_normal,
        "globalvars_t.trace_plane_normal"
    );
    check_offset!(
        GlobalVars,
        trace_plane_dist,
        "globalvars_t.trace_plane_dist"
    );
    check_offset!(GlobalVars, trace_ent, "globalvars_t.trace_ent");
    check_offset!(GlobalVars, trace_inopen, "globalvars_t.trace_inopen");
    check_offset!(GlobalVars, trace_inwater, "globalvars_t.trace_inwater");
    check_offset!(GlobalVars, msg_entity, "globalvars_t.msg_entity");
    check_offset!(GlobalVars, main, "globalvars_t.main");
    check_offset!(GlobalVars, StartFrame, "globalvars_t.StartFrame");
    check_offset!(GlobalVars, PlayerPreThink, "globalvars_t.PlayerPreThink");
    check_offset!(GlobalVars, PlayerPostThink, "globalvars_t.PlayerPostThink");
    check_offset!(GlobalVars, ClientKill, "globalvars_t.ClientKill");
    check_offset!(GlobalVars, ClientConnect, "globalvars_t.ClientConnect");
    check_offset!(
        GlobalVars,
        PutClientInServer,
        "globalvars_t.PutClientInServer"
    );
    check_offset!(
        GlobalVars,
        ClientDisconnect,
        "globalvars_t.ClientDisconnect"
    );
    check_offset!(GlobalVars, SetNewParms, "globalvars_t.SetNewParms");
    check_offset!(GlobalVars, SetChangeParms, "globalvars_t.SetChangeParms");

    check_size!(EntVars, "entvars_t");
    check_offset!(EntVars, modelindex, "entvars_t.modelindex");
    check_offset!(EntVars, absmin, "entvars_t.absmin");
    check_offset!(EntVars, absmax, "entvars_t.absmax");
    check_offset!(EntVars, ltime, "entvars_t.ltime");
    check_offset!(EntVars, movetype, "entvars_t.movetype");
    check_offset!(EntVars, solid, "entvars_t.solid");
    check_offset!(EntVars, origin, "entvars_t.origin");
    check_offset!(EntVars, oldorigin, "entvars_t.oldorigin");
    check_offset!(EntVars, velocity, "entvars_t.velocity");
    check_offset!(EntVars, angles, "entvars_t.angles");
    check_offset!(EntVars, avelocity, "entvars_t.avelocity");
    check_offset!(EntVars, punchangle, "entvars_t.punchangle");
    check_offset!(EntVars, classname, "entvars_t.classname");
    check_offset!(EntVars, model, "entvars_t.model");
    check_offset!(EntVars, frame, "entvars_t.frame");
    check_offset!(EntVars, skin, "entvars_t.skin");
    check_offset!(EntVars, effects, "entvars_t.effects");
    check_offset!(EntVars, mins, "entvars_t.mins");
    check_offset!(EntVars, maxs, "entvars_t.maxs");
    check_offset!(EntVars, size, "entvars_t.size");
    check_offset!(EntVars, touch, "entvars_t.touch");
    check_offset!(EntVars, r#use, "entvars_t.use");
    check_offset!(EntVars, think, "entvars_t.think");
    check_offset!(EntVars, blocked, "entvars_t.blocked");
    check_offset!(EntVars, nextthink, "entvars_t.nextthink");
    check_offset!(EntVars, groundentity, "entvars_t.groundentity");
    check_offset!(EntVars, health, "entvars_t.health");
    check_offset!(EntVars, frags, "entvars_t.frags");
    check_offset!(EntVars, weapon, "entvars_t.weapon");
    check_offset!(EntVars, weaponmodel, "entvars_t.weaponmodel");
    check_offset!(EntVars, weaponframe, "entvars_t.weaponframe");
    check_offset!(EntVars, currentammo, "entvars_t.currentammo");
    check_offset!(EntVars, ammo_shells, "entvars_t.ammo_shells");
    check_offset!(EntVars, ammo_nails, "entvars_t.ammo_nails");
    check_offset!(EntVars, ammo_rockets, "entvars_t.ammo_rockets");
    check_offset!(EntVars, ammo_cells, "entvars_t.ammo_cells");
    check_offset!(EntVars, items, "entvars_t.items");
    check_offset!(EntVars, takedamage, "entvars_t.takedamage");
    check_offset!(EntVars, chain, "entvars_t.chain");
    check_offset!(EntVars, deadflag, "entvars_t.deadflag");
    check_offset!(EntVars, view_ofs, "entvars_t.view_ofs");
    check_offset!(EntVars, button0, "entvars_t.button0");
    check_offset!(EntVars, button1, "entvars_t.button1");
    check_offset!(EntVars, button2, "entvars_t.button2");
    check_offset!(EntVars, impulse, "entvars_t.impulse");
    check_offset!(EntVars, fixangle, "entvars_t.fixangle");
    check_offset!(EntVars, v_angle, "entvars_t.v_angle");
    check_offset!(EntVars, idealpitch, "entvars_t.idealpitch");
    check_offset!(EntVars, netname, "entvars_t.netname");
    check_offset!(EntVars, enemy, "entvars_t.enemy");
    check_offset!(EntVars, flags, "entvars_t.flags");
    check_offset!(EntVars, colormap, "entvars_t.colormap");
    check_offset!(EntVars, team, "entvars_t.team");
    check_offset!(EntVars, max_health, "entvars_t.max_health");
    check_offset!(EntVars, teleport_time, "entvars_t.teleport_time");
    check_offset!(EntVars, armortype, "entvars_t.armortype");
    check_offset!(EntVars, armorvalue, "entvars_t.armorvalue");
    check_offset!(EntVars, waterlevel, "entvars_t.waterlevel");
    check_offset!(EntVars, watertype, "entvars_t.watertype");
    check_offset!(EntVars, ideal_yaw, "entvars_t.ideal_yaw");
    check_offset!(EntVars, yaw_speed, "entvars_t.yaw_speed");
    check_offset!(EntVars, aiment, "entvars_t.aiment");
    check_offset!(EntVars, goalentity, "entvars_t.goalentity");
    check_offset!(EntVars, spawnflags, "entvars_t.spawnflags");
    check_offset!(EntVars, target, "entvars_t.target");
    check_offset!(EntVars, targetname, "entvars_t.targetname");
    check_offset!(EntVars, dmg_take, "entvars_t.dmg_take");
    check_offset!(EntVars, dmg_save, "entvars_t.dmg_save");
    check_offset!(EntVars, dmg_inflictor, "entvars_t.dmg_inflictor");
    check_offset!(EntVars, owner, "entvars_t.owner");
    check_offset!(EntVars, movedir, "entvars_t.movedir");
    check_offset!(EntVars, message, "entvars_t.message");
    check_offset!(EntVars, sounds, "entvars_t.sounds");
    check_offset!(EntVars, noise, "entvars_t.noise");
    check_offset!(EntVars, noise1, "entvars_t.noise1");
    check_offset!(EntVars, noise2, "entvars_t.noise2");
    check_offset!(EntVars, noise3, "entvars_t.noise3");
}

#[test]
fn edict_mirror_matches_engine_headers() {
    check_size!(Link, "link_t");
    check_offset!(Link, prev, "link_t.prev");
    check_offset!(Link, next, "link_t.next");

    check_size!(EntityState, "entity_state_t");
    check_offset!(EntityState, origin, "entity_state_t.origin");
    check_offset!(EntityState, effects, "entity_state_t.effects");
    check_offset!(EntityState, velocity, "entity_state_t.velocity");
    check_offset!(EntityState, colormod, "entity_state_t.colormod");
    check_offset!(EntityState, solidsize, "entity_state_t.solidsize");

    // NB: size_of::<Edict>() is the *header* size only; the real array stride
    // is the runtime qcvm->edict_size, which extends past `v` (ADR-006).
    check_size!(Edict, "edict_t");
    // The three DEBUG/_DEBUG bookkeeping fields exist on the mirror only under
    // the feature, so this has to be a compile-time gate. All three are 8-byte
    // members: a mis-ordering would leave sizeof and every later offset
    // intact, so each needs its own check.
    // engine_debug_feature_matches_the_c_build() proves the feature and the
    // probe's TU agree, which is what makes these keys present.
    #[cfg(feature = "engine-debug")]
    {
        check_offset!(Edict, edict_ptr, "edict_t.edict_ptr");
        check_offset!(Edict, qcvm_owner, "edict_t.qcvm_owner");
        check_offset!(Edict, edict_num, "edict_t.edict_num");
    }
    check_offset!(Edict, area, "edict_t.area");
    check_offset!(Edict, num_leafs, "edict_t.num_leafs");
    check_offset!(Edict, leafnums, "edict_t.leafnums");
    check_offset!(Edict, baseline, "edict_t.baseline");
    check_offset!(Edict, alpha, "edict_t.alpha");
    check_offset!(Edict, sendinterval, "edict_t.sendinterval");
    check_offset!(Edict, sendinterval_default, "edict_t.sendinterval_default");
    check_offset!(Edict, oldframe, "edict_t.oldframe");
    check_offset!(Edict, oldthinktime, "edict_t.oldthinktime");
    check_offset!(Edict, predthinkpos, "edict_t.predthinkpos");
    check_offset!(Edict, lastthink, "edict_t.lastthink");
    check_offset!(Edict, freetime, "edict_t.freetime");
    check_offset!(Edict, free, "edict_t.free");
    check_offset!(Edict, v, "edict_t.v");
}

#[test]
fn qcvm_mirror_matches_engine_headers() {
    check_size!(PrStack, "prstack_t");
    check_offset!(PrStack, s, "prstack_t.s");
    check_offset!(PrStack, f, "prstack_t.f");

    check_size!(FreeList, "freelist_t");
    check_offset!(FreeList, size, "freelist_t.size");
    check_offset!(FreeList, head_index, "freelist_t.head_index");
    check_offset!(FreeList, circular_buffer, "freelist_t.circular_buffer");

    check_size!(AreaNode, "areanode_t");
    check_offset!(AreaNode, axis, "areanode_t.axis");
    check_offset!(AreaNode, dist, "areanode_t.dist");
    check_offset!(AreaNode, children, "areanode_t.children");
    check_offset!(AreaNode, trigger_edicts, "areanode_t.trigger_edicts");
    check_offset!(AreaNode, solid_edicts, "areanode_t.solid_edicts");

    check_size!(PrExtGlobals, "pr_extglobals_s");
    check_offset!(PrExtGlobals, time, "pr_extglobals_s.time");
    check_offset!(PrExtGlobals, physics_mode, "pr_extglobals_s.physics_mode");
    check_offset!(
        PrExtGlobals,
        player_localentnum,
        "pr_extglobals_s.player_localentnum"
    );

    check_size!(PrExtFuncs, "pr_extfuncs_s");
    check_offset!(PrExtFuncs, game_command, "pr_extfuncs_s.GameCommand");
    check_offset!(PrExtFuncs, csqc_draw_hud, "pr_extfuncs_s.CSQC_DrawHud");
    check_offset!(
        PrExtFuncs,
        csqc_parse_print,
        "pr_extfuncs_s.CSQC_Parse_Print"
    );

    check_size!(PrExtFields, "pr_extfields_s");
    check_offset!(PrExtFields, alpha, "pr_extfields_s.alpha");
    check_offset!(PrExtFields, customphysics, "pr_extfields_s.customphysics");
    check_offset!(PrExtFields, send_flags, "pr_extfields_s.SendFlags");

    assert_eq!(
        size_of::<BuiltinT>(),
        c_abi("sizeof.builtin_t"),
        "builtin_t: Option<extern \"C\" fn()> must be null-pointer-optimised \
         to a bare function pointer"
    );

    check_size!(QcVm, "qcvm_t");
    check_offset!(QcVm, progs, "qcvm_t.progs");
    check_offset!(QcVm, functions, "qcvm_t.functions");
    check_offset!(QcVm, function_map, "qcvm_t.function_map");
    check_offset!(QcVm, statements, "qcvm_t.statements");
    check_offset!(QcVm, globals, "qcvm_t.globals");
    check_offset!(QcVm, fielddefs, "qcvm_t.fielddefs");
    check_offset!(QcVm, fielddefs_map, "qcvm_t.fielddefs_map");
    check_offset!(QcVm, edict_size, "qcvm_t.edict_size");
    check_offset!(QcVm, builtins, "qcvm_t.builtins");
    check_offset!(QcVm, numbuiltins, "qcvm_t.numbuiltins");
    check_offset!(QcVm, argc, "qcvm_t.argc");
    check_offset!(QcVm, trace, "qcvm_t.trace");
    check_offset!(QcVm, xfunction, "qcvm_t.xfunction");
    check_offset!(QcVm, xstatement, "qcvm_t.xstatement");
    check_offset!(QcVm, progscrc, "qcvm_t.progscrc");
    check_offset!(QcVm, progshash, "qcvm_t.progshash");
    check_offset!(QcVm, progssize, "qcvm_t.progssize");
    check_offset!(QcVm, extglobals, "qcvm_t.extglobals");
    check_offset!(QcVm, extfuncs, "qcvm_t.extfuncs");
    check_offset!(QcVm, extfields, "qcvm_t.extfields");
    check_offset!(QcVm, strings, "qcvm_t.strings");
    check_offset!(QcVm, stringssize, "qcvm_t.stringssize");
    check_offset!(QcVm, knownstrings, "qcvm_t.knownstrings");
    check_offset!(QcVm, knownstringsowned, "qcvm_t.knownstringsowned");
    check_offset!(QcVm, maxknownstrings, "qcvm_t.maxknownstrings");
    check_offset!(QcVm, numknownstrings, "qcvm_t.numknownstrings");
    check_offset!(QcVm, progsstrings, "qcvm_t.progsstrings");
    check_offset!(QcVm, freeknownstrings, "qcvm_t.freeknownstrings");
    check_offset!(QcVm, globaldefs, "qcvm_t.globaldefs");
    check_offset!(QcVm, globaldefs_map, "qcvm_t.globaldefs_map");
    check_offset!(QcVm, knownzone, "qcvm_t.knownzone");
    check_offset!(QcVm, knownzonesize, "qcvm_t.knownzonesize");
    check_offset!(QcVm, stack, "qcvm_t.stack");
    check_offset!(QcVm, depth, "qcvm_t.depth");
    check_offset!(QcVm, localstack, "qcvm_t.localstack");
    check_offset!(QcVm, localstack_used, "qcvm_t.localstack_used");
    check_offset!(QcVm, time, "qcvm_t.time");
    check_offset!(QcVm, num_edicts, "qcvm_t.num_edicts");
    check_offset!(QcVm, reserved_edicts, "qcvm_t.reserved_edicts");
    check_offset!(QcVm, max_edicts, "qcvm_t.max_edicts");
    check_offset!(QcVm, edicts, "qcvm_t.edicts");
    check_offset!(QcVm, free_list, "qcvm_t.free_list");
    check_offset!(QcVm, worldmodel, "qcvm_t.worldmodel");
    check_offset!(QcVm, get_model, "qcvm_t.GetModel");
    check_offset!(QcVm, areanodes, "qcvm_t.areanodes");
    check_offset!(QcVm, numareanodes, "qcvm_t.numareanodes");
}

#[test]
fn progs_consts_match_engine_headers() {
    assert_eq!(progs::PROG_VERSION as usize, c_abi("const.PROG_VERSION"));
    assert_eq!(
        progs::PROGHEADER_CRC as usize,
        c_abi("const.PROGHEADER_CRC")
    );
    assert_eq!(progs::MAX_PARMS, c_abi("const.MAX_PARMS"));
    assert_eq!(
        usize::from(progs::DEF_SAVEGLOBAL),
        c_abi("const.DEF_SAVEGLOBAL")
    );
    assert_eq!(progs::OFS_RETURN, c_abi("const.OFS_RETURN"));
    assert_eq!(progs::OFS_PARM0, c_abi("const.OFS_PARM0"));
    assert_eq!(progs::OFS_PARM7, c_abi("const.OFS_PARM7"));
    assert_eq!(progs::RESERVED_OFS, c_abi("const.RESERVED_OFS"));
    assert_eq!(progs::MAX_ENT_LEAFS, c_abi("const.MAX_ENT_LEAFS"));
    assert_eq!(progs::MAX_EDICTS, c_abi("const.MAX_EDICTS"));
    assert_eq!(progs::MAX_AREA_DEPTH, c_abi("const.MAX_AREA_DEPTH"));
    assert_eq!(progs::AREA_NODES, c_abi("const.AREA_NODES"));
    assert_eq!(progs::MAX_STACK_DEPTH, c_abi("const.MAX_STACK_DEPTH"));
    assert_eq!(progs::LOCALSTACK_SIZE, c_abi("const.LOCALSTACK_SIZE"));
    assert_eq!(progs::STRINGTEMP_BUFFERS, c_abi("const.STRINGTEMP_BUFFERS"));
    assert_eq!(progs::STRINGTEMP_LENGTH, c_abi("const.STRINGTEMP_LENGTH"));
    assert_eq!(progs::MIN_EDICTS, c_abi("const.MIN_EDICTS"));
    // the two age thresholds are floats; the probe casts them to size_t,
    // which is exact for 2.0
    assert_eq!(
        progs::MIN_EDICT_AGE_FOR_REUSE as usize,
        c_abi("const.MIN_EDICT_AGE_FOR_REUSE")
    );
    assert_eq!(
        progs::MAX_EDICT_FREETIME_ALWAYS_REUSE as usize,
        c_abi("const.MAX_EDICT_FREETIME_ALWAYS_REUSE")
    );
}
