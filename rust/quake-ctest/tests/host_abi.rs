//! ABI cross-check: the `quake_types::host` mirrors vs what the engine's own
//! headers (`server.h`, `client.h`, `render.h`, `protocol.h`) say on this
//! platform (Rust migration Phase 7).
//!
//! Under the Phase 7 host/server/client port, Rust reads and writes the
//! C-owned `sv`/`svs`/`cl`/`cls` globals directly, so mirror drift here is
//! silent memory corruption rather than a link error. Neither `server.h` nor
//! `client.h` is a bindgen-clean root (both pull `qcvm_t`, and `client.h`
//! additionally pulls the SDL/Vulkan-tainted `entity_t`), so the mirrors are
//! hand-written (ADR-011) and this probe, compiled from the engine's own
//! headers, is the per-platform gate.
//!
//! Neither header has any `DEBUG`/`_DEBUG`-conditional field (unlike
//! `progs.h`'s `edict_t`), so unlike `progs_abi.rs` this suite does not need
//! an `engine-debug` feature/build-profile agreement check.

use core::mem::{align_of, offset_of, size_of};

use quake_ctest as _;
use quake_types::host::{
    AmbientSound, CShift, Client, ClientState, ClientStatic, DeltaFrame, DeltaFrameEnt, Efrag,
    EntityNumState, EntityOpaque, ParticlePrecacheEntry, ScoreBoard, Server, ServerStatic,
    SvCustomStat, UserCmd, CA_CONNECTED, CA_DEDICATED, CA_DISCONNECTED, PRESPAWN_AMBIENTS,
    PRESPAWN_BASELINES, PRESPAWN_DONE, PRESPAWN_FLUSH, PRESPAWN_MODELS, PRESPAWN_PARTICLES,
    PRESPAWN_SIGNONMSG, PRESPAWN_SOUNDS, PRESPAWN_STATICS, SS_ACTIVE, SS_LOADING,
};

extern "C" {
    fn ctest_abi_host_lookup(key: *const core::ffi::c_char) -> usize;
}

fn c_abi(key: &str) -> usize {
    let cstr = std::ffi::CString::new(key).unwrap();
    // SAFETY: the probe only strcmp's the key against a compile-time table.
    let v = unsafe { ctest_abi_host_lookup(cstr.as_ptr()) };
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

#[test]
fn server_static_layout_matches_c() {
    check_size!(ServerStatic, "server_static_t");
    check_offset!(ServerStatic, maxclients, "server_static_t.maxclients");
    check_offset!(
        ServerStatic,
        maxclientslimit,
        "server_static_t.maxclientslimit"
    );
    check_offset!(ServerStatic, clients, "server_static_t.clients");
    check_offset!(ServerStatic, serverflags, "server_static_t.serverflags");
    check_offset!(
        ServerStatic,
        changelevel_issued,
        "server_static_t.changelevel_issued"
    );
    check_offset!(ServerStatic, serverinfo, "server_static_t.serverinfo");
}

#[test]
fn server_state_constants_match_c() {
    assert_eq!(SS_LOADING as usize, c_abi("const.ss_loading"));
    assert_eq!(SS_ACTIVE as usize, c_abi("const.ss_active"));
}

#[test]
fn ambient_sound_layout_matches_c() {
    check_size!(AmbientSound, "ambientsound_t");
    check_offset!(AmbientSound, origin, "ambientsound_t.origin");
    check_offset!(AmbientSound, soundindex, "ambientsound_t.soundindex");
    check_offset!(AmbientSound, volume, "ambientsound_t.volume");
    check_offset!(AmbientSound, attenuation, "ambientsound_t.attenuation");
}

#[test]
fn sv_custom_stat_layout_matches_c() {
    check_size!(SvCustomStat, "svcustomstat_t");
    check_offset!(SvCustomStat, idx, "svcustomstat_t.idx");
    check_offset!(SvCustomStat, r#type, "svcustomstat_t.type");
    check_offset!(SvCustomStat, fld, "svcustomstat_t.fld");
    check_offset!(SvCustomStat, ptr, "svcustomstat_t.ptr");
}

#[test]
fn server_layout_matches_c() {
    check_size!(Server, "server_t");
    check_offset!(Server, active, "server_t.active");
    check_offset!(Server, paused, "server_t.paused");
    check_offset!(Server, loadgame, "server_t.loadgame");
    check_offset!(Server, nomonsters, "server_t.nomonsters");
    check_offset!(Server, lastsave, "server_t.lastsave");
    check_offset!(Server, lastcheck, "server_t.lastcheck");
    check_offset!(Server, lastchecktime, "server_t.lastchecktime");
    check_offset!(Server, qcvm, "server_t.qcvm");
    check_offset!(Server, name, "server_t.name");
    check_offset!(Server, modelname, "server_t.modelname");
    check_offset!(Server, model_precache, "server_t.model_precache");
    check_offset!(Server, models, "server_t.models");
    check_offset!(Server, sound_precache, "server_t.sound_precache");
    check_offset!(Server, lightstyles, "server_t.lightstyles");
    check_offset!(Server, state, "server_t.state");
    check_offset!(Server, datagram, "server_t.datagram");
    check_offset!(Server, datagram_buf, "server_t.datagram_buf");
    check_offset!(Server, reliable_datagram, "server_t.reliable_datagram");
    check_offset!(
        Server,
        reliable_datagram_buf,
        "server_t.reliable_datagram_buf"
    );
    check_offset!(Server, signon, "server_t.signon");
    check_offset!(Server, signon_buf, "server_t.signon_buf");
    check_offset!(Server, protocol, "server_t.protocol");
    check_offset!(Server, protocolflags, "server_t.protocolflags");
    check_offset!(Server, multicast, "server_t.multicast");
    check_offset!(Server, multicast_buf, "server_t.multicast_buf");
    check_offset!(Server, particle_precache, "server_t.particle_precache");
    check_offset!(Server, static_entities, "server_t.static_entities");
    check_offset!(Server, num_statics, "server_t.num_statics");
    check_offset!(Server, max_statics, "server_t.max_statics");
    check_offset!(Server, ambientsounds, "server_t.ambientsounds");
    check_offset!(Server, num_ambients, "server_t.num_ambients");
    check_offset!(Server, max_ambients, "server_t.max_ambients");
    check_offset!(Server, customstats, "server_t.customstats");
    check_offset!(Server, numcustomstats, "server_t.numcustomstats");
    check_offset!(Server, effectsmask, "server_t.effectsmask");
}

#[test]
fn usercmd_layout_matches_c() {
    check_size!(UserCmd, "usercmd_t");
    check_offset!(UserCmd, servertime, "usercmd_t.servertime");
    check_offset!(UserCmd, seconds, "usercmd_t.seconds");
    check_offset!(UserCmd, viewangles, "usercmd_t.viewangles");
    check_offset!(UserCmd, forwardmove, "usercmd_t.forwardmove");
    check_offset!(UserCmd, sidemove, "usercmd_t.sidemove");
    check_offset!(UserCmd, upmove, "usercmd_t.upmove");
    check_offset!(
        UserCmd,
        forwardmove_accumulator,
        "usercmd_t.forwardmove_accumulator"
    );
    check_offset!(
        UserCmd,
        sidemove_accumulator,
        "usercmd_t.sidemove_accumulator"
    );
    check_offset!(UserCmd, upmove_accumulator, "usercmd_t.upmove_accumulator");
    check_offset!(UserCmd, buttons, "usercmd_t.buttons");
    check_offset!(UserCmd, impulse, "usercmd_t.impulse");
    check_offset!(UserCmd, sequence, "usercmd_t.sequence");
    check_offset!(UserCmd, weapon, "usercmd_t.weapon");
}

#[test]
fn prespawn_constants_match_c() {
    assert_eq!(PRESPAWN_DONE as usize, c_abi("const.PRESPAWN_DONE"));
    assert_eq!(PRESPAWN_FLUSH as usize, c_abi("const.PRESPAWN_FLUSH"));
    assert_eq!(PRESPAWN_MODELS as usize, c_abi("const.PRESPAWN_MODELS"));
    assert_eq!(PRESPAWN_SOUNDS as usize, c_abi("const.PRESPAWN_SOUNDS"));
    assert_eq!(
        PRESPAWN_PARTICLES as usize,
        c_abi("const.PRESPAWN_PARTICLES")
    );
    assert_eq!(
        PRESPAWN_BASELINES as usize,
        c_abi("const.PRESPAWN_BASELINES")
    );
    assert_eq!(PRESPAWN_STATICS as usize, c_abi("const.PRESPAWN_STATICS"));
    assert_eq!(PRESPAWN_AMBIENTS as usize, c_abi("const.PRESPAWN_AMBIENTS"));
    assert_eq!(
        PRESPAWN_SIGNONMSG as usize,
        c_abi("const.PRESPAWN_SIGNONMSG")
    );
}

#[test]
fn entity_num_state_layout_matches_c() {
    check_size!(EntityNumState, "entity_num_state_t");
    check_offset!(EntityNumState, num, "entity_num_state_t.num");
    check_offset!(EntityNumState, state, "entity_num_state_t.state");
}

#[test]
fn delta_frame_ent_size_matches_c() {
    // `struct deltaframe_s.ents` points at a truly anonymous C struct (no
    // tag), so it can only be checked by sizeof, not per-field offsetof.
    assert_eq!(
        size_of::<DeltaFrameEnt>(),
        c_abi("sizeof.deltaframe_ents_t")
    );
}

#[test]
fn delta_frame_layout_matches_c() {
    check_size!(DeltaFrame, "deltaframe_t");
    check_offset!(DeltaFrame, sequence, "deltaframe_t.sequence");
    check_offset!(DeltaFrame, timestamp, "deltaframe_t.timestamp");
    check_offset!(DeltaFrame, resendstatsnum, "deltaframe_t.resendstatsnum");
    check_offset!(DeltaFrame, resendstatsstr, "deltaframe_t.resendstatsstr");
    check_offset!(DeltaFrame, ents, "deltaframe_t.ents");
    check_offset!(DeltaFrame, numents, "deltaframe_t.numents");
    check_offset!(DeltaFrame, maxents, "deltaframe_t.maxents");
}

#[test]
fn client_layout_matches_c() {
    check_size!(Client, "client_t");
    check_offset!(Client, active, "client_t.active");
    check_offset!(Client, spawned, "client_t.spawned");
    check_offset!(Client, dropasap, "client_t.dropasap");
    check_offset!(Client, sendsignon, "client_t.sendsignon");
    check_offset!(Client, signonidx, "client_t.signonidx");
    check_offset!(Client, signon_sounds, "client_t.signon_sounds");
    check_offset!(Client, signon_models, "client_t.signon_models");
    check_offset!(Client, last_message, "client_t.last_message");
    check_offset!(Client, netconnection, "client_t.netconnection");
    check_offset!(Client, cmd, "client_t.cmd");
    check_offset!(Client, wishdir, "client_t.wishdir");
    check_offset!(Client, message, "client_t.message");
    check_offset!(Client, msgbuf, "client_t.msgbuf");
    check_offset!(Client, edict, "client_t.edict");
    check_offset!(Client, name, "client_t.name");
    check_offset!(Client, colors, "client_t.colors");
    check_offset!(Client, ping_times, "client_t.ping_times");
    check_offset!(Client, num_pings, "client_t.num_pings");
    check_offset!(Client, spawn_parms, "client_t.spawn_parms");
    check_offset!(Client, old_frags, "client_t.old_frags");
    check_offset!(Client, datagram, "client_t.datagram");
    check_offset!(Client, datagram_buf, "client_t.datagram_buf");
    check_offset!(Client, limit_entities, "client_t.limit_entities");
    check_offset!(Client, limit_unreliable, "client_t.limit_unreliable");
    check_offset!(Client, limit_reliable, "client_t.limit_reliable");
    check_offset!(Client, limit_models, "client_t.limit_models");
    check_offset!(Client, limit_sounds, "client_t.limit_sounds");
    check_offset!(Client, pextknown, "client_t.pextknown");
    check_offset!(Client, protocol_pext1, "client_t.protocol_pext1");
    check_offset!(Client, protocol_pext2, "client_t.protocol_pext2");
    check_offset!(Client, resendstatsnum, "client_t.resendstatsnum");
    check_offset!(Client, resendstatsstr, "client_t.resendstatsstr");
    check_offset!(Client, oldstats_i, "client_t.oldstats_i");
    check_offset!(Client, oldstats_f, "client_t.oldstats_f");
    check_offset!(Client, oldstats_s, "client_t.oldstats_s");
    check_offset!(Client, previousentities, "client_t.previousentities");
    check_offset!(Client, numpreviousentities, "client_t.numpreviousentities");
    check_offset!(Client, maxpreviousentities, "client_t.maxpreviousentities");
    check_offset!(Client, snapshotresume, "client_t.snapshotresume");
    check_offset!(
        Client,
        pendingentities_bits,
        "client_t.pendingentities_bits"
    );
    check_offset!(Client, numpendingentities, "client_t.numpendingentities");
    check_offset!(Client, frames, "client_t.frames");
    check_offset!(Client, numframes, "client_t.numframes");
    check_offset!(Client, lastacksequence, "client_t.lastacksequence");
    check_offset!(Client, lastmovemessage, "client_t.lastmovemessage");
    check_offset!(Client, lastmovetime, "client_t.lastmovetime");
    check_offset!(Client, knowntoqc, "client_t.knowntoqc");
    check_offset!(Client, userinfo, "client_t.userinfo");
}

#[test]
fn cactive_constants_match_c() {
    assert_eq!(CA_DEDICATED as usize, c_abi("const.CA_DEDICATED"));
    assert_eq!(CA_DISCONNECTED as usize, c_abi("const.CA_DISCONNECTED"));
    assert_eq!(CA_CONNECTED as usize, c_abi("const.CA_CONNECTED"));
}

#[test]
fn cshift_layout_matches_c() {
    check_size!(CShift, "cshift_t");
    check_offset!(CShift, destcolor, "cshift_t.destcolor");
    check_offset!(CShift, percent, "cshift_t.percent");
}

#[test]
fn scoreboard_layout_matches_c() {
    check_size!(ScoreBoard, "scoreboard_t");
    check_offset!(ScoreBoard, name, "scoreboard_t.name");
    check_offset!(ScoreBoard, entertime, "scoreboard_t.entertime");
    check_offset!(ScoreBoard, frags, "scoreboard_t.frags");
    check_offset!(ScoreBoard, colors, "scoreboard_t.colors");
    check_offset!(ScoreBoard, ping, "scoreboard_t.ping");
    check_offset!(ScoreBoard, translations, "scoreboard_t.translations");
    check_offset!(ScoreBoard, userinfo, "scoreboard_t.userinfo");
}

#[test]
fn client_static_layout_matches_c() {
    check_size!(ClientStatic, "client_static_t");
    check_offset!(ClientStatic, state, "client_static_t.state");
    check_offset!(ClientStatic, spawnparms, "client_static_t.spawnparms");
    check_offset!(ClientStatic, demonum, "client_static_t.demonum");
    check_offset!(ClientStatic, demos, "client_static_t.demos");
    check_offset!(ClientStatic, demorecording, "client_static_t.demorecording");
    check_offset!(ClientStatic, demoplayback, "client_static_t.demoplayback");
    check_offset!(ClientStatic, demopaused, "client_static_t.demopaused");
    check_offset!(ClientStatic, demoseeking, "client_static_t.demoseeking");
    check_offset!(ClientStatic, seektime, "client_static_t.seektime");
    check_offset!(ClientStatic, demospeed, "client_static_t.demospeed");
    check_offset!(
        ClientStatic,
        demo_prespawn_end,
        "client_static_t.demo_prespawn_end"
    );
    check_offset!(ClientStatic, timedemo, "client_static_t.timedemo");
    check_offset!(ClientStatic, forcetrack, "client_static_t.forcetrack");
    check_offset!(ClientStatic, demofile, "client_static_t.demofile");
    check_offset!(ClientStatic, td_lastframe, "client_static_t.td_lastframe");
    check_offset!(ClientStatic, td_startframe, "client_static_t.td_startframe");
    check_offset!(ClientStatic, td_starttime, "client_static_t.td_starttime");
    check_offset!(ClientStatic, signon, "client_static_t.signon");
    check_offset!(ClientStatic, netcon, "client_static_t.netcon");
    check_offset!(ClientStatic, message, "client_static_t.message");
    check_offset!(ClientStatic, userinfo, "client_static_t.userinfo");
}

#[test]
fn efrag_layout_matches_c() {
    check_size!(Efrag, "efrag_t");
    check_offset!(Efrag, leafnext, "efrag_t.leafnext");
    check_offset!(Efrag, entity, "efrag_t.entity");
}

#[test]
fn entity_opaque_size_and_align_match_c() {
    // `entity_t` is mirrored as an opaque blob (renderer state, later
    // phase); only its size/alignment matter here, checked against a local
    // field-verbatim shadow of `entity_t` compiled from `render.h` (see the
    // probe's Phase 7 section), not a guess.
    assert_eq!(size_of::<EntityOpaque>(), c_abi("sizeof.entity_t"));
    assert_eq!(align_of::<EntityOpaque>(), c_abi("alignof.entity_t"));
}

#[test]
fn particle_precache_entry_size_matches_c() {
    // `client_state_t::particle_precache`/`local_particle_precache` element
    // type is also a truly anonymous C struct -- sizeof-only, same as
    // `deltaframe_ents_t` above.
    assert_eq!(
        size_of::<ParticlePrecacheEntry>(),
        c_abi("sizeof.particle_precache_entry_t")
    );
}

#[test]
fn client_state_layout_matches_c() {
    check_size!(ClientState, "client_state_t");
    check_offset!(ClientState, movemessages, "client_state_t.movemessages");
    check_offset!(
        ClientState,
        ackedmovemessages,
        "client_state_t.ackedmovemessages"
    );
    check_offset!(ClientState, movecmds, "client_state_t.movecmds");
    check_offset!(ClientState, pendingcmd, "client_state_t.pendingcmd");
    check_offset!(ClientState, stats, "client_state_t.stats");
    check_offset!(ClientState, statsf, "client_state_t.statsf");
    check_offset!(ClientState, statss, "client_state_t.statss");
    check_offset!(ClientState, items, "client_state_t.items");
    check_offset!(ClientState, item_gettime, "client_state_t.item_gettime");
    check_offset!(ClientState, faceanimtime, "client_state_t.faceanimtime");
    check_offset!(ClientState, v_dmg_time, "client_state_t.v_dmg_time");
    check_offset!(ClientState, v_dmg_roll, "client_state_t.v_dmg_roll");
    check_offset!(ClientState, v_dmg_pitch, "client_state_t.v_dmg_pitch");
    check_offset!(ClientState, cshift_empty, "client_state_t.cshift_empty");
    check_offset!(ClientState, cshifts, "client_state_t.cshifts");
    check_offset!(ClientState, prev_cshifts, "client_state_t.prev_cshifts");
    check_offset!(ClientState, mviewangles, "client_state_t.mviewangles");
    check_offset!(ClientState, viewangles, "client_state_t.viewangles");
    check_offset!(ClientState, mvelocity, "client_state_t.mvelocity");
    check_offset!(ClientState, velocity, "client_state_t.velocity");
    check_offset!(ClientState, punchangle, "client_state_t.punchangle");
    check_offset!(ClientState, idealpitch, "client_state_t.idealpitch");
    check_offset!(ClientState, pitchvel, "client_state_t.pitchvel");
    check_offset!(ClientState, nodrift, "client_state_t.nodrift");
    check_offset!(ClientState, driftmove, "client_state_t.driftmove");
    check_offset!(ClientState, laststop, "client_state_t.laststop");
    check_offset!(ClientState, viewheight, "client_state_t.viewheight");
    check_offset!(ClientState, crouch, "client_state_t.crouch");
    check_offset!(ClientState, paused, "client_state_t.paused");
    check_offset!(ClientState, onground, "client_state_t.onground");
    check_offset!(ClientState, inwater, "client_state_t.inwater");
    check_offset!(ClientState, fixangle_time, "client_state_t.fixangle_time");
    check_offset!(ClientState, intermission, "client_state_t.intermission");
    check_offset!(ClientState, completed_time, "client_state_t.completed_time");
    check_offset!(ClientState, mtime, "client_state_t.mtime");
    check_offset!(ClientState, time, "client_state_t.time");
    check_offset!(ClientState, oldtime, "client_state_t.oldtime");
    check_offset!(
        ClientState,
        last_received_message,
        "client_state_t.last_received_message"
    );
    check_offset!(ClientState, model_precache, "client_state_t.model_precache");
    check_offset!(ClientState, sound_precache, "client_state_t.sound_precache");
    check_offset!(ClientState, mapname, "client_state_t.mapname");
    check_offset!(ClientState, levelname, "client_state_t.levelname");
    check_offset!(ClientState, viewentity, "client_state_t.viewentity");
    check_offset!(ClientState, maxclients, "client_state_t.maxclients");
    check_offset!(ClientState, gametype, "client_state_t.gametype");
    check_offset!(ClientState, worldmodel, "client_state_t.worldmodel");
    check_offset!(ClientState, free_efrags, "client_state_t.free_efrags");
    check_offset!(ClientState, num_efrags, "client_state_t.num_efrags");
    check_offset!(ClientState, efrag_allocs, "client_state_t.efrag_allocs");
    check_offset!(
        ClientState,
        num_efragallocs,
        "client_state_t.num_efragallocs"
    );
    check_offset!(ClientState, viewent, "client_state_t.viewent");
    check_offset!(ClientState, entities, "client_state_t.entities");
    check_offset!(ClientState, max_edicts, "client_state_t.max_edicts");
    check_offset!(ClientState, num_entities, "client_state_t.num_entities");
    check_offset!(
        ClientState,
        static_entities,
        "client_state_t.static_entities"
    );
    check_offset!(
        ClientState,
        max_static_entities,
        "client_state_t.max_static_entities"
    );
    check_offset!(ClientState, num_statics, "client_state_t.num_statics");
    check_offset!(ClientState, cdtrack, "client_state_t.cdtrack");
    check_offset!(ClientState, looptrack, "client_state_t.looptrack");
    check_offset!(ClientState, scores, "client_state_t.scores");
    check_offset!(ClientState, protocol, "client_state_t.protocol");
    check_offset!(ClientState, protocolflags, "client_state_t.protocolflags");
    check_offset!(ClientState, protocol_pext1, "client_state_t.protocol_pext1");
    check_offset!(ClientState, protocol_pext2, "client_state_t.protocol_pext2");
    check_offset!(
        ClientState,
        protocol_particles,
        "client_state_t.protocol_particles"
    );
    check_offset!(
        ClientState,
        particle_precache,
        "client_state_t.particle_precache"
    );
    check_offset!(
        ClientState,
        local_particle_precache,
        "client_state_t.local_particle_precache"
    );
    check_offset!(ClientState, ackframes, "client_state_t.ackframes");
    check_offset!(
        ClientState,
        ackframes_count,
        "client_state_t.ackframes_count"
    );
    check_offset!(ClientState, requestresend, "client_state_t.requestresend");
    check_offset!(ClientState, sendprespawn, "client_state_t.sendprespawn");
    check_offset!(ClientState, qcvm, "client_state_t.qcvm");
    check_offset!(ClientState, zoom, "client_state_t.zoom");
    check_offset!(ClientState, zoomdir, "client_state_t.zoomdir");
    check_offset!(ClientState, serverinfo, "client_state_t.serverinfo");
}
