//! Phase 7 M8 T8.1: characterization tests for Quake/host_cmd.c's savegame
//! stratum, driven through the C reference oracle in stubs/host_cmd_ref.c.
//!
//! There is no Rust port of host_cmd.c yet, so nothing here is a Rust-vs-C
//! comparison. These are the goldens the later port will be diffed against:
//! the savegame writer's whole output byte-for-byte, every early return in
//! its guard ladder, and the reader's guard ladder up to the point where it
//! needs a real BSP.
//!
//! Known gaps, stated so the port does not mistake this suite for coverage it
//! is not:
//!   * Host_Loadgame_f (host_cmd.c:1797) is only driven as far as the
//!     savegame-version check. Past that it calls CL_Disconnect_f and
//!     SV_SpawnServer (host_cmd.c:1913), which need a real map and progs.dat.
//!   * The rest of host_cmd.c -- the map/changelevel/restart commands, the
//!     client commands, the mod and demo file lists -- is out of scope for
//!     this milestone.
//!   * COMPAT: ADR-005. The extended block ends with Fog_GetFogCommand and
//!     Sky_GetSkyCommand (host_cmd.c:1685, :1689), whose real definitions
//!     (gl_fog.c:281, gl_sky.c:541) format with "%g". Those two calls pass
//!     always = true, so *every* savegame hits them. The Rust float formatter
//!     rejects %g, so a Rust Host_Savegame_f cannot route these through it.
//!     stubs.c reproduces both format strings verbatim so the goldens below
//!     pin the exact digits the C engine writes.

use std::ffi::{c_char, c_float, c_int, c_void, CStr, CString};
use std::sync::{Mutex, MutexGuard};

use quake_ctest as _; // links the cc-built c_ref_* archive

/// host_cmd.c:1510
const SAVEGAME_VERSION: i32 = 5;
/// quakedef.h:97
const SAVEGAME_COMMENT_LENGTH: usize = 39;
/// cmd.h:84 -- src_client = 0, src_command = 1
const SRC_CLIENT: c_int = 0;
const SRC_COMMAND: c_int = 1;
/// server.h -- MAX_LIGHTSTYLES
const MAX_LIGHTSTYLES: usize = 64;
/// quakedef.h -- NUM_BASIC_SPAWN_PARMS
const NUM_BASIC_SPAWN_PARMS: usize = 16;

extern "C" {
    fn ctest_hostcmd_savegame_comment(out: *mut c_char);
    fn ctest_hostcmd_savegame_f();
    fn ctest_hostcmd_loadgame_f();
    fn ctest_hostcmd_get_current_skill() -> c_int;
    fn ctest_hostcmd_setup_savegame(
        gamedir: *const c_char,
        levelname: *const c_char,
        mapname: *const c_char,
        monsters: c_int,
        totalmonsters: c_int,
        skill_value: c_int,
        qctime: c_float,
    );
    fn ctest_hostcmd_set_intermission(value: c_int);
    fn ctest_hostcmd_set_nomonsters(value: c_int);
    fn ctest_hostcmd_set_sv_active(value: c_int);
    fn ctest_hostcmd_set_maxclients(value: c_int);
    fn ctest_hostcmd_set_player_health(value: c_float);
    fn ctest_hostcmd_set_cmd_source(value: c_int);
    fn ctest_hostcmd_tokenize(text: *const c_char);
    fn ctest_hostcmd_get_lastsave() -> *const c_char;
    fn ctest_hostcmd_clear_qcvm();

    fn ctest_clear_con_log();
    fn ctest_con_log_len() -> c_int;
    fn ctest_con_log_get(index: c_int) -> *const c_char;
    fn ctest_try_host(f: extern "C" fn(*mut c_void), arg: *mut c_void) -> c_int;
    fn ctest_host_error_message() -> *const c_char;
    fn ctest_set_fog(
        density: c_float,
        red: c_float,
        green: c_float,
        blue: c_float,
        fade_done: c_int,
    );
    fn ctest_set_sky(name: *const c_char, name_worldspawn: *const c_char, skyfog: c_float);
    fn ctest_get_last_link_addr() -> *const c_char;
    fn ctest_scr_end_loading_plaque_count() -> c_int;
}

// The oracle is a pile of C globals; every test drives the same ones.
static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn cs(s: &str) -> CString {
    CString::new(s).unwrap()
}

fn con_log() -> Vec<String> {
    // SAFETY: ADR-004. Reading the harness console capture, which is plain C
    // storage owned by stubs.c and only mutated while TEST_LOCK is held.
    unsafe {
        (0..ctest_con_log_len())
            .map(|i| {
                CStr::from_ptr(ctest_con_log_get(i))
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }
}

/// A scratch directory standing in for com_gamedir. Tests are serialized, but
/// each still writes a distinct file so a leftover never masks a failure.
fn gamedir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("vkq_ctest_host_cmd");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn gamedir_str() -> String {
    gamedir().to_string_lossy().replace('\\', "/")
}

/// Puts the oracle into the one state Host_Savegame_f accepts: single-player,
/// active server, live player, `save <name>` already tokenized.
fn arm_savegame(savename: &str) {
    let dir = cs(&gamedir_str());
    let level = cs("The Slipgate Complex");
    let map = cs("e1m1");
    let empty = cs("");
    let cmd = cs(&format!("save {savename}"));
    // SAFETY: ADR-004. Fixture setup on the C oracle, TEST_LOCK held.
    unsafe {
        ctest_hostcmd_clear_qcvm();
        ctest_hostcmd_setup_savegame(dir.as_ptr(), level.as_ptr(), map.as_ptr(), 3, 10, 2, 12.5);
        ctest_set_fog(0.0, 0.0, 0.0, 0.0, 0);
        ctest_set_sky(empty.as_ptr(), empty.as_ptr(), 0.5);
        ctest_hostcmd_tokenize(cmd.as_ptr());
        ctest_clear_con_log();
    }
}

fn saved_bytes(savename: &str) -> Vec<u8> {
    std::fs::read(gamedir().join(format!("{savename}.sav"))).unwrap()
}

/// Independent transcription of the writer's format (host_cmd.c:1627-1692),
/// built from the same fixture values arm_savegame() installs.
fn expected_savegame(comment: &str, skill: i32, mapname: &str, time: f32) -> String {
    let mut out = String::new();
    out.push_str(&format!("{SAVEGAME_VERSION}\n"));
    out.push_str(&format!("{comment}\n"));
    for i in 0..NUM_BASIC_SPAWN_PARMS {
        out.push_str(&format!("{:.6}\n", i as f32));
    }
    out.push_str(&format!("{skill}\n"));
    out.push_str(&format!("{mapname}\n"));
    out.push_str(&format!("{time:.6}\n"));
    for i in 0..MAX_LIGHTSTYLES {
        out.push_str(match i {
            0 => "a\n",
            2 => "mmnn\n",
            _ => "m\n",
        });
    }
    // ED_WriteGlobals (pr_edict_save.c:162): only the DEF_SAVEGLOBAL def.
    out.push_str("{\n\"gsave\" \"3.500000\"\n}\n");
    // ED_Write (pr_edict_save.c:105) for edict 0 (all fields zero) and edict 1.
    out.push_str("{\n}\n");
    out.push_str("{\n\"health\" \"42.000000\"\n}\n");
    out.push_str("/*\n");
    out.push_str("// QuakeSpasm extended savegame\n");
    out.push_str("sv.model_precache 1 \"maps/ctest.bsp\"\n");
    out.push_str("sv.model_precache 2 \"progs/player.mdl\"\n");
    out.push_str("sv.sound_precache 1 \"weapons/r_exp3.wav\"\n");
    out.push_str("sv.particle_precache 1 \"tr_rocket\"\n");
    out.push_str("sv.serverflags 3\n");
    out.push_str("spawnparm 17 \"7.500000\"\n");
    // COMPAT: ADR-005 -- %g output, see the module comment.
    out.push_str("fog 0 0 0 0\n");
    out.push_str("sky \"\"\nskyfog 0.5\n");
    out.push_str("*/\n");
    out
}

fn savegame_comment() -> String {
    let mut buf = [0u8; SAVEGAME_COMMENT_LENGTH + 1];
    // SAFETY: ADR-004. buf is exactly the char[SAVEGAME_COMMENT_LENGTH + 1]
    // the C signature (host_cmd.c:1519) requires.
    unsafe { ctest_hostcmd_savegame_comment(buf.as_mut_ptr().cast()) };
    let end = buf.iter().position(|&b| b == 0).unwrap();
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

#[test]
fn savegame_comment_pads_the_level_name_and_underscores_the_blanks() {
    let _g = lock();
    arm_savegame("comment_short");
    let text = savegame_comment();

    assert_eq!(text.len(), SAVEGAME_COMMENT_LENGTH);
    // 22 columns of level name, then a fixed-width kills field (host_cmd.c:1538)
    assert_eq!(text, "The_Slipgate_Complex__kills:__3/_10____");
    assert!(!text.contains(' '));
}

#[test]
fn savegame_comment_truncates_the_level_name_at_twenty_two_columns() {
    let _g = lock();
    let dir = cs(&gamedir_str());
    let level = cs("A Level Name Far Longer Than Twenty Two");
    let map = cs("e1m1");
    // SAFETY: ADR-004. Fixture setup, TEST_LOCK held.
    unsafe {
        ctest_hostcmd_clear_qcvm();
        ctest_hostcmd_setup_savegame(dir.as_ptr(), level.as_ptr(), map.as_ptr(), 0, 0, 0, 0.0);
    }
    assert_eq!(
        savegame_comment(),
        "A_Level_Name_Far_Longekills:__0/__0____"
    );
}

#[test]
fn savegame_writes_the_whole_file() {
    let _g = lock();
    arm_savegame("golden");
    // SAFETY: ADR-004. Drives the C oracle, TEST_LOCK held.
    unsafe { ctest_hostcmd_savegame_f() };

    let text = String::from_utf8(saved_bytes("golden")).unwrap();
    // Sys_fopen opens in text mode (host_cmd.c:1621), so CRLF is a platform
    // artefact of stdio, not of the writer; the line sequence is the subject.
    assert_eq!(
        text.replace("\r\n", "\n"),
        expected_savegame("The_Slipgate_Complex__kills:__3/_10____", 2, "e1m1", 12.5)
    );
}

#[test]
fn savegame_reports_the_link_and_the_completion() {
    let _g = lock();
    arm_savegame("linkline");
    // SAFETY: ADR-004.
    unsafe { ctest_hostcmd_savegame_f() };

    let expected_path = format!("{}/linkline.sav", gamedir_str());
    // SAFETY: ADR-004. Static C buffer, TEST_LOCK held.
    let addr = unsafe {
        CStr::from_ptr(ctest_get_last_link_addr())
            .to_string_lossy()
            .into_owned()
    };
    assert_eq!(addr, expected_path);
    assert_eq!(
        con_log(),
        vec![
            // Con_SafePrintf reaches the capture log under the [safe] tag;
            // host_cmd.c:1616-1619 mixes Con_SafePrintf and Con_Printf.
            "[safe] Saving game to ".to_string(),
            format!("[link] {expected_path}"),
            "[safe] ...\n".to_string(),
            "[con] done.\n".to_string(),
        ]
    );
}

#[test]
fn savegame_remembers_the_name_for_autoload() {
    let _g = lock();
    arm_savegame("lastsave");
    // SAFETY: ADR-004.
    unsafe { ctest_hostcmd_savegame_f() };
    // SAFETY: ADR-004. Points at sv.lastsave, TEST_LOCK held.
    let last = unsafe {
        CStr::from_ptr(ctest_hostcmd_get_lastsave())
            .to_string_lossy()
            .into_owned()
    };
    assert_eq!(last, "lastsave");
}

#[test]
fn savegame_writes_the_fog_and_sky_commands_it_is_given() {
    let _g = lock();
    arm_savegame("fogsky");
    let night = cs("night");
    let empty = cs("");
    // SAFETY: ADR-004.
    unsafe {
        ctest_set_fog(0.05, 0.25, 0.5, 1.0, 0);
        ctest_set_sky(night.as_ptr(), empty.as_ptr(), 0.25);
        ctest_hostcmd_savegame_f();
    }

    let text = String::from_utf8(saved_bytes("fogsky"))
        .unwrap()
        .replace("\r\n", "\n");
    // COMPAT: ADR-005 -- these are %g digits, see the module comment.
    assert!(text.contains("\nfog 0.05 0.25 0.5 1\n"), "{text}");
    assert!(
        text.contains("\nsky \"night\"\nskyfog 0.25\n*/\n"),
        "{text}"
    );
}

/// Every guard in host_cmd.c:1560-1605 returns before any file is opened, so
/// the console line is the whole observable.
fn assert_refused(savename: &str, expected: &[&str]) {
    let path = gamedir().join(format!("{savename}.sav"));
    let _ = std::fs::remove_file(&path);
    // SAFETY: ADR-004.
    unsafe { ctest_hostcmd_savegame_f() };
    assert_eq!(con_log(), expected);
    assert!(!path.exists());
}

#[test]
fn savegame_ignores_a_command_that_did_not_come_from_the_console() {
    let _g = lock();
    arm_savegame("refused_src");
    // SAFETY: ADR-004.
    unsafe { ctest_hostcmd_set_cmd_source(SRC_CLIENT) };
    assert_refused("refused_src", &[]);
    // SAFETY: ADR-004. Leave the oracle in the shared default.
    unsafe { ctest_hostcmd_set_cmd_source(SRC_COMMAND) };
}

#[test]
fn savegame_refuses_when_no_server_is_running() {
    let _g = lock();
    arm_savegame("refused_inactive");
    // SAFETY: ADR-004.
    unsafe { ctest_hostcmd_set_sv_active(0) };
    assert_refused("refused_inactive", &["[con] Not playing a local game.\n"]);
}

#[test]
fn savegame_refuses_a_nomonsters_game() {
    let _g = lock();
    arm_savegame("refused_nomonsters");
    // SAFETY: ADR-004.
    unsafe { ctest_hostcmd_set_nomonsters(1) };
    assert_refused(
        "refused_nomonsters",
        &["[con] Can't save when using \"nomonsters\".\n"],
    );
}

#[test]
fn savegame_refuses_during_intermission() {
    let _g = lock();
    arm_savegame("refused_intermission");
    // SAFETY: ADR-004.
    unsafe { ctest_hostcmd_set_intermission(1) };
    assert_refused(
        "refused_intermission",
        &["[con] Can't save in intermission.\n"],
    );
}

#[test]
fn savegame_refuses_a_multiplayer_game() {
    let _g = lock();
    arm_savegame("refused_multiplayer");
    // SAFETY: ADR-004.
    unsafe { ctest_hostcmd_set_maxclients(2) };
    assert_refused(
        "refused_multiplayer",
        &["[con] Can't save multiplayer games.\n"],
    );
}

#[test]
fn savegame_requires_exactly_one_argument() {
    let _g = lock();
    arm_savegame("refused_argc");
    let cmd = cs("save");
    // SAFETY: ADR-004.
    unsafe {
        ctest_hostcmd_tokenize(cmd.as_ptr());
        ctest_clear_con_log();
    }
    assert_refused("refused_argc", &["[con] save <savename> : save a game\n"]);
}

#[test]
fn savegame_refuses_a_relative_pathname() {
    let _g = lock();
    arm_savegame("refused_relative");
    let cmd = cs("save ../escape");
    // SAFETY: ADR-004.
    unsafe {
        ctest_hostcmd_tokenize(cmd.as_ptr());
        ctest_clear_con_log();
    }
    assert_refused(
        "refused_relative",
        &["[con] Relative pathnames are not allowed.\n"],
    );
}

#[test]
fn savegame_refuses_a_dead_player() {
    let _g = lock();
    arm_savegame("refused_dead");
    // SAFETY: ADR-004.
    unsafe { ctest_hostcmd_set_player_health(0.0) };
    assert_refused(
        "refused_dead",
        &["[con] Can't savegame with a dead player\n"],
    );
}

extern "C" fn call_loadgame(_arg: *mut c_void) {
    // SAFETY: ADR-004. Trampoline for ctest_try_host; the oracle call itself.
    unsafe { ctest_hostcmd_loadgame_f() };
}

fn run_loadgame_trapped() -> Option<String> {
    // SAFETY: ADR-004. ctest_try_host (stubs.c:1415) arms the Host_Error trap
    // around the call and longjmps back out of it.
    unsafe {
        if ctest_try_host(call_loadgame, std::ptr::null_mut()) != 0 {
            Some(
                CStr::from_ptr(ctest_host_error_message())
                    .to_string_lossy()
                    .into_owned(),
            )
        } else {
            None
        }
    }
}

#[test]
fn loadgame_ignores_a_command_that_did_not_come_from_the_console() {
    let _g = lock();
    arm_savegame("load_src");
    let cmd = cs("load load_src");
    // SAFETY: ADR-004.
    unsafe {
        ctest_hostcmd_tokenize(cmd.as_ptr());
        ctest_hostcmd_set_cmd_source(SRC_CLIENT);
        ctest_clear_con_log();
    }
    assert_eq!(run_loadgame_trapped(), None);
    assert_eq!(con_log(), Vec::<String>::new());
    // SAFETY: ADR-004.
    unsafe { ctest_hostcmd_set_cmd_source(SRC_COMMAND) };
}

#[test]
fn loadgame_requires_exactly_one_argument() {
    let _g = lock();
    arm_savegame("load_argc");
    let cmd = cs("load");
    // SAFETY: ADR-004.
    unsafe {
        ctest_hostcmd_tokenize(cmd.as_ptr());
        ctest_clear_con_log();
    }
    assert_eq!(run_loadgame_trapped(), None);
    assert_eq!(con_log(), vec!["[con] load <savename> : load a game\n"]);
}

#[test]
fn loadgame_refuses_a_relative_pathname() {
    let _g = lock();
    arm_savegame("load_relative");
    let cmd = cs("load ../escape");
    // SAFETY: ADR-004.
    unsafe {
        ctest_hostcmd_tokenize(cmd.as_ptr());
        ctest_clear_con_log();
    }
    assert_eq!(run_loadgame_trapped(), None);
    assert_eq!(
        con_log(),
        vec!["[con] Relative pathnames are not allowed.\n"]
    );
}

#[test]
fn loadgame_reports_a_savegame_that_is_not_there() {
    let _g = lock();
    arm_savegame("load_missing");
    let _ = std::fs::remove_file(gamedir().join("load_missing.sav"));
    let cmd = cs("load load_missing");
    // SAFETY: ADR-004.
    unsafe {
        ctest_hostcmd_tokenize(cmd.as_ptr());
        ctest_clear_con_log();
    }
    // SAFETY: ADR-004.
    let before = unsafe { ctest_scr_end_loading_plaque_count() };
    assert_eq!(run_loadgame_trapped(), None);
    assert_eq!(con_log(), vec!["[con] ERROR: couldn't open.\n"]);
    // host_cmd.c:1866 ends the loading plaque before reporting the failure.
    // SAFETY: ADR-004.
    assert_eq!(unsafe { ctest_scr_end_loading_plaque_count() }, before + 1);
}

#[test]
fn loadgame_rejects_a_savegame_from_a_different_version() {
    let _g = lock();
    arm_savegame("load_version");
    std::fs::write(
        gamedir().join("load_version.sav"),
        format!("{}\ncomment\n", SAVEGAME_VERSION + 1),
    )
    .unwrap();
    let cmd = cs("load load_version");
    // SAFETY: ADR-004.
    unsafe {
        ctest_hostcmd_tokenize(cmd.as_ptr());
        ctest_clear_con_log();
    }
    assert_eq!(
        run_loadgame_trapped(),
        Some(format!(
            "Savegame is version {}, not {}",
            SAVEGAME_VERSION + 1,
            SAVEGAME_VERSION
        ))
    );
}

#[test]
fn loadgame_leaves_the_skill_alone_until_the_version_check_passes() {
    let _g = lock();
    arm_savegame("load_skill");
    std::fs::write(
        gamedir().join("load_skill.sav"),
        format!("{}\ncomment\n", SAVEGAME_VERSION + 1),
    )
    .unwrap();
    let cmd = cs("load load_skill");
    // SAFETY: ADR-004. arm_savegame installed current_skill = 2.
    unsafe {
        ctest_hostcmd_tokenize(cmd.as_ptr());
        ctest_clear_con_log();
    }
    assert!(run_loadgame_trapped().is_some());
    // SAFETY: ADR-004.
    assert_eq!(unsafe { ctest_hostcmd_get_current_skill() }, 2);
}
