//! Differential tests: gamedir / searchpath bookkeeping — COM_GetGameNames,
//! COM_GameDirMatches, COM_ResetGameDirectories quirks (dedupe, id1 and
//! empty-token handling), mission-pack flags, path_id doubling, multi-root
//! precedence (is_main/is_user) and COM_InitFilesystem — Rust shims vs the
//! c_ref_* C reference over identical fixtures and stubs.

use core::ffi::CStr;
use quake_ctest::fs as ctfs;
use quake_ctest::fs::{Side, BOTH};

fn game_names(side: Side, full: bool) -> String {
    // SAFETY: fs state owned under FS_LOCK; the returned pointer is copied
    // out before any further engine call (va-buffer lifetime)
    unsafe {
        CStr::from_ptr((ctfs::fns(side).get_game_names)(full))
            .to_string_lossy()
            .into_owned()
    }
}

fn dir_matches(side: Side, tdirs: &CStr) -> bool {
    // SAFETY: NUL-terminated input; fs state owned under FS_LOCK
    unsafe { (ctfs::fns(side).game_dir_matches)(tdirs.as_ptr()) }
}

/// Mounts `dirs` on both sides over `roots` and asserts snapshot parity;
/// returns the shared snapshot.
fn mount_compare(
    roots: &[&std::path::Path],
    main_idx: usize,
    dirs: &CStr,
    ctx: &str,
) -> ctfs::FsSnapshot {
    let mut snaps = Vec::new();
    for side in BOTH {
        ctfs::clear_logs();
        let err = ctfs::catch_sys_error(|| ctfs::setup(side, roots, main_idx, dirs));
        assert_eq!(err, None, "unexpected Sys_Error mounting {dirs:?} ({ctx})");
        snaps.push((ctfs::snapshot(side), ctfs::con_log()));
    }
    assert_eq!(snaps[0], snaps[1], "mount parity for {dirs:?} ({ctx})");
    snaps.remove(0).0
}

#[test]
fn gamedir_differential() {
    let _guard = ctfs::lock();
    ctfs::set_host_dirs("/nonexistent-host-basedir", None);
    ctfs::set_registered(1.0);
    ctfs::set_developer(0.0);

    let root = std::env::temp_dir().join(format!("quake-ctest-fsgd-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for d in [
        "id1", "tg", "other", "hipnotic", "quoth", "rogue", "a", "b", "c",
    ] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }

    // --- game names + matching over a mission-pack style mount --------------
    {
        mount_compare(&[&root], 0, c"hipnotic;quoth", "names");
        assert_eq!(game_names(Side::C, false), "hipnotic;quoth");
        assert_eq!(game_names(Side::Rust, false), "hipnotic;quoth");
        assert_eq!(game_names(Side::C, true), "id1;hipnotic;quoth");
        assert_eq!(game_names(Side::Rust, true), "id1;hipnotic;quoth");

        for tdirs in [
            c"hipnotic;quoth",
            c"id1;hipnotic;quoth",
            c"qw;hipnotic;quoth",
            c"id1;qw;hipnotic;quoth",
            c"qw",
            c"id1",
            c"",
            c"hipnotic",
            c"quoth;hipnotic",
            c"HIPNOTIC;QUOTH",
            // the leading id1/qw strip is strncmp, i.e. case-SENSITIVE, and
            // tdirs is server-controlled (MSG_ReadString in cl_parse.c), so
            // these must NOT be stripped
            c"ID1;hipnotic;quoth",
            c"Id1;hipnotic;quoth",
            c"QW;hipnotic;quoth",
            c"ID1",
            c"QW",
        ] {
            assert_eq!(
                dir_matches(Side::C, tdirs),
                dir_matches(Side::Rust, tdirs),
                "COM_GameDirMatches({tdirs:?}) with hipnotic;quoth mounted"
            );
        }
    }

    // --- matching with nothing mounted --------------------------------------
    {
        mount_compare(&[&root], 0, c"", "bare");
        for tdirs in [
            c"", c"id1", c"qw", c"qw;id1", c"id1;qw", c"tg", c"ID1", c"QW",
        ] {
            assert_eq!(
                dir_matches(Side::C, tdirs),
                dir_matches(Side::Rust, tdirs),
                "COM_GameDirMatches({tdirs:?}) with nothing mounted"
            );
        }
    }

    // --- ResetGameDirectories token quirks ----------------------------------
    for dirs in [
        c"tg",
        c"tg;tg",      // duplicate: second skipped
        c"TG;tg",      // case-insensitive duplicate
        c"id1;tg",     // id1 always filtered
        c"ID1;tg",     // ... case-insensitively
        c"tg;other",   // two mounts, path_id doubles
        c"a;b;c",      // three mounts: ids 1, 2, 4
        c";;tg;;",     // empty tokens
        c"missingdir", // is_main root mounts even without the directory
        c"",           // nothing
    ] {
        let snap = mount_compare(&[&root], 0, dirs, "token quirks");
        if dirs == c"a;b;c" {
            let ids: Vec<u32> = snap.searchpaths.iter().map(|s| s.path_id).collect();
            assert_eq!(ids, vec![4, 2, 1], "path_id doubling (head-first)");
        }
        if dirs == c"tg;tg" || dirs == c"TG;tg" {
            assert_eq!(
                snap.searchpaths.len(),
                1,
                "duplicate gamedir must mount once: {snap:?}"
            );
        }
    }

    // --- mission pack flag setting ------------------------------------------
    for (dirs, rogue, hipnotic) in [
        (c"rogue", true, false),
        (c"ROGUE", true, false),
        (c"hipnotic", false, true),
        (c"quoth", false, true),
        (c"rogue;hipnotic", true, true),
        (c"tg", false, false),
    ] {
        let snap = mount_compare(&[&root], 0, dirs, "mission packs");
        assert_eq!(snap.rogue, rogue, "rogue flag for {dirs:?}");
        assert_eq!(snap.hipnotic, hipnotic, "hipnotic flag for {dirs:?}");
        assert_eq!(
            snap.standard_quake,
            !(rogue || hipnotic),
            "standard_quake for {dirs:?}"
        );
    }

    // --- base marker: gamedirs stacked on a base keep doubling path_id ------
    {
        for side in BOTH {
            ctfs::setup(side, &[&root], 0, c"tg");
            ctfs::mark_base(side);
            // SAFETY: fs mounted under FS_LOCK; NUL-terminated dirs
            unsafe { (ctfs::fns(side).reset_game_directories)(c"a;b".as_ptr()) };
        }
        let c_snap = ctfs::snapshot(Side::C);
        assert_eq!(c_snap, ctfs::snapshot(Side::Rust), "base + stacked mount");
        let ids: Vec<u32> = c_snap.searchpaths.iter().map(|s| s.path_id).collect();
        assert_eq!(ids, vec![4, 2, 1], "stacked path_ids over base");
        assert_eq!(c_snap.above_base, 2, "two nodes above the base marker");
        // resetting to base frees only the stack above it
        for side in BOTH {
            // SAFETY: as above
            unsafe { (ctfs::fns(side).reset_game_directories)(c"".as_ptr()) };
        }
        let c_snap = ctfs::snapshot(Side::C);
        assert_eq!(c_snap, ctfs::snapshot(Side::Rust), "reset back to base");
        assert_eq!(c_snap.searchpaths.len(), 1, "base survives the reset");
        assert_eq!(c_snap.above_base, 0);
    }

    // --- multi-root precedence: extras below the main basedir ---------------
    {
        let extra = root.join("extra-root");
        let main = root.join("main-root");
        for r in [&extra, &main] {
            std::fs::create_dir_all(r.join("mg")).unwrap();
        }
        std::fs::create_dir_all(main.join("onlymain")).unwrap();
        std::fs::write(extra.join("mg/both.txt"), b"extra copy").unwrap();
        std::fs::write(main.join("mg/both.txt"), b"main copy").unwrap();
        std::fs::write(extra.join("mg/extraonly.txt"), b"extra only").unwrap();

        let snap = mount_compare(&[&extra, &main], 1, c"mg", "two roots");
        // both roots mount (head-first: main on top), same path_id
        assert_eq!(snap.searchpaths.len(), 2);
        assert_eq!(snap.searchpaths[0].path_id, snap.searchpaths[1].path_id);
        assert!(snap.searchpaths[0]
            .filename
            .starts_with(main.to_str().unwrap()));

        // the main root shadows the extra on conflicts; extras still reachable
        let (bytes, _, _, _) = ctfs::load_file(Side::C, c"both.txt").unwrap();
        assert_eq!(bytes, b"main copy");
        let r = ctfs::load_file(Side::Rust, c"both.txt").unwrap();
        assert_eq!(r.0, b"main copy");
        assert_eq!(
            ctfs::load_file(Side::C, c"extraonly.txt"),
            ctfs::load_file(Side::Rust, c"extraonly.txt")
        );

        // a dir missing from the (non-main) extra root is skipped there
        let snap = mount_compare(&[&extra, &main], 1, c"onlymain", "extra-missing dir");
        assert_eq!(snap.searchpaths.len(), 1, "extra root skipped: {snap:?}");
    }

    // --- userdir root: mkdir + mounted last as the write target -------------
    {
        let main = root.join("main-root");
        let user = root.join("user-root");
        std::fs::create_dir_all(&user).unwrap();
        ctfs::set_host_dirs(main.to_str().unwrap(), Some(user.to_str().unwrap()));
        assert!(!user.join("ug").exists());
        let snap = mount_compare(&[&main, &user], 0, c"ug", "userdir");
        assert!(
            user.join("ug").is_dir(),
            "userdir gamedir must be created via Sys_mkdir"
        );
        // both mounted (main is_main, user is_user), user on top
        assert_eq!(snap.searchpaths.len(), 2);
        assert!(snap.searchpaths[0]
            .filename
            .starts_with(user.to_str().unwrap()));
        assert!(snap.gamedir.starts_with(user.to_str().unwrap()));
        ctfs::set_host_dirs("/nonexistent-host-basedir", None);
    }

    // --- COM_ModForbiddenChars ----------------------------------------------
    for p in [
        c"",
        c".",
        c"..",
        c"a..b",
        c"a/b",
        c"a\\b",
        c"a:b",
        c"a\"b",
        c"a;b",
        c"normal",
        c"norm-al_1",
        c".hidden",
        c"a.b",
    ] {
        // SAFETY: NUL-terminated inputs, no state touched
        let (c_v, r_v) = unsafe {
            (
                (ctfs::fns(Side::C).mod_forbidden_chars)(p.as_ptr()),
                (ctfs::fns(Side::Rust).mod_forbidden_chars)(p.as_ptr()),
            )
        };
        assert_eq!(c_v, r_v, "COM_ModForbiddenChars({p:?})");
    }

    for side in BOTH {
        ctfs::reset(side);
    }
}

/// Full COM_InitFilesystem runs: -basedir/-rogue/-game/-basegame argument
/// handling, registration/command logging, base searchpath marking and the
/// store-detection skip, compared via con+cvar logs and state snapshots.
#[test]
fn init_filesystem_differential() {
    let _guard = ctfs::lock();
    ctfs::set_host_dirs("/nonexistent-host-basedir", None);
    ctfs::set_registered(1.0);
    ctfs::set_developer(0.0);

    let root = std::env::temp_dir().join(format!("quake-ctest-fsinit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for d in ["id1", "rogue", "mg1", "bg1", "bg2"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    std::fs::write(root.join("id1/gfx.txt"), b"id1 payload").unwrap();
    std::fs::write(root.join("mg1/mod.txt"), b"mod payload").unwrap();
    let root_str = root.to_str().unwrap().to_string();
    let root_slash = format!("{}/", root_str);

    let arg_sets: Vec<(Vec<&str>, bool, &str)> = vec![
        (vec!["quake", "-basedir", &root_str], false, "plain id1"),
        (
            vec!["quake", "-basedir", &root_slash, "-rogue", "-game", "mg1"],
            false,
            "trailing slash + rogue + -game",
        ),
        (
            vec![
                "quake",
                "-basedir",
                &root_str,
                "-basegame",
                "bg1",
                "-basegame",
                "bg2",
            ],
            false,
            "-basegame replaces id1",
        ),
        (
            vec!["quake", "-basedir", &root_str, "-hipnotic", "-quoth"],
            false,
            "both hipnotic-style packs",
        ),
        (
            vec!["quake", "-basedir", &root_str, "-game", "bad/dir"],
            true,
            "forbidden -game chars fatal",
        ),
        (vec!["quake", "-basedir", ""], true, "empty -basedir fatal"),
    ];

    for (args, expect_error, ctx) in arg_sets {
        let _own = ctfs::set_args(&args);
        let mut outputs = Vec::new();
        for side in BOTH {
            ctfs::reset(side);
            ctfs::clear_logs();
            let err = ctfs::catch_sys_error(|| {
                // SAFETY: startup call over freshly reset state under FS_LOCK
                unsafe { (ctfs::fns(side).init_filesystem)() }
            });
            outputs.push((err, ctfs::con_log(), ctfs::cvar_log(), ctfs::snapshot(side)));
        }
        assert_eq!(outputs[0], outputs[1], "COM_InitFilesystem parity: {ctx}");
        assert_eq!(
            outputs[0].0.is_some(),
            expect_error,
            "error expectation: {ctx}"
        );
        if !expect_error {
            assert!(
                outputs[0]
                    .1
                    .iter()
                    .any(|l| l.contains("COM_CheckRegistered")),
                "COM_CheckRegistered must run: {ctx}"
            );
        }
    }

    for side in BOTH {
        ctfs::reset(side);
    }
}
