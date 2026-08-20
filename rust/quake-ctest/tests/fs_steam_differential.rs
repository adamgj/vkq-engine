//! Differential tests: store discovery — Steam_IsValidPath / Steam_FindGame /
//! Steam_ResolvePath (VDF/ACF parsing) and EGS_FindGame (launcher data +
//! *.item manifests) — the Rust shims vs the original steam.c compiled as
//! c_ref_*. Both sides read the same fixture trees through the same settable
//! Sys_Get*Dir stubs; results, packed steamgame_t buffers (bytes + subdir
//! offset) and Sys_Printf error lines are compared.

use core::ffi::{c_char, c_int, CStr};
use quake_c_sys::{steamgame_t, MAX_OSPATH};
use quake_ctest::fs as ctfs;
use quake_ctest::fs::{Side, BOTH};

fn zeroed_game() -> steamgame_t {
    steamgame_t {
        appid: 0,
        subdir: core::ptr::null_mut(),
        library: [0; MAX_OSPATH],
    }
}

/// Full observable steamgame_t state: return value, appid, the entire packed
/// library buffer, and the subdir pointer as an offset into it (or None).
#[derive(Debug, PartialEq, Eq)]
struct FindGameResult {
    ret: bool,
    appid: c_int,
    library: Vec<u8>,
    subdir_offset: Option<usize>,
    log: Vec<String>,
}

fn find_game(side: Side, appid: c_int) -> FindGameResult {
    let mut game = zeroed_game();
    ctfs::clear_logs();
    // SAFETY: `game` is a valid zeroed steamgame_t out-param
    let ret = unsafe { (ctfs::fns(side).steam_find_game)(&mut game, appid) };
    let library: Vec<u8> = game.library.iter().map(|&c| c as u8).collect();
    let subdir_offset = if game.subdir.is_null() {
        None
    } else {
        Some(game.subdir as usize - game.library.as_ptr() as usize)
    };
    FindGameResult {
        ret,
        appid: game.appid,
        library,
        subdir_offset,
        log: ctfs::con_log(),
    }
}

/// Runs Steam_FindGame on both sides, asserts parity, returns the C result
/// and the C-side raw game (for follow-up ResolvePath calls).
fn find_game_compare(appid: c_int, ctx: &str) -> FindGameResult {
    let c = find_game(Side::C, appid);
    let r = find_game(Side::Rust, appid);
    assert_eq!(c, r, "Steam_FindGame parity ({ctx})");
    c
}

fn resolve_path_compare(game: &steamgame_t, pathsize: usize, ctx: &str) {
    let mut results = Vec::new();
    for side in BOTH {
        let mut buf = vec![0x7fu8; pathsize.max(1)];
        // SAFETY: buf is writable for pathsize bytes (or unused for 0); game
        // is a valid steamgame_t
        let ret = unsafe {
            (ctfs::fns(side).steam_resolve_path)(buf.as_mut_ptr() as *mut c_char, pathsize, game)
        };
        results.push((ret, buf));
    }
    assert_eq!(
        results[0], results[1],
        "Steam_ResolvePath parity pathsize={pathsize} ({ctx})"
    );
}

fn is_valid_path_compare(path: &CStr, ctx: &str) -> bool {
    // SAFETY: NUL-terminated path
    let (c, r) = unsafe {
        (
            (ctfs::fns(Side::C).steam_is_valid_path)(path.as_ptr()),
            (ctfs::fns(Side::Rust).steam_is_valid_path)(path.as_ptr()),
        )
    };
    assert_eq!(c, r, "Steam_IsValidPath parity ({ctx})");
    c
}

fn egs_find_compare(
    nspace: &CStr,
    itemid: &CStr,
    appname: &CStr,
    pathsize: usize,
    ctx: &str,
) -> (bool, Vec<u8>) {
    let mut results = Vec::new();
    for side in BOTH {
        let mut buf = vec![0x7fu8; pathsize.max(1)];
        ctfs::clear_logs();
        // SAFETY: NUL-terminated ids; buf writable for pathsize bytes
        let ret = unsafe {
            (ctfs::fns(side).egs_find_game)(
                buf.as_mut_ptr() as *mut c_char,
                pathsize,
                nspace.as_ptr(),
                itemid.as_ptr(),
                appname.as_ptr(),
            )
        };
        results.push((ret, buf, ctfs::con_log()));
    }
    assert_eq!(results[0], results[1], "EGS_FindGame parity ({ctx})");
    let (ret, buf, _) = results.remove(0);
    (ret, buf)
}

/// Escapes a filesystem path for embedding in a VDF quoted string.
///
/// Windows paths contain backslashes and VDB_ParseString treats those as
/// escape introducers, so a raw path would be mangled (`C:\Users` loses the
/// `\U`). Real libraryfolders.vdf files double them for exactly this reason,
/// so the fixture does too — which also exercises the parser's `\\` case.
/// A no-op on Unix.
fn vdf_path(p: &std::path::Path) -> String {
    p.to_str().unwrap().replace('\\', "\\\\")
}

fn write(root: &std::path::Path, rel: &str, content: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

#[test]
fn steam_differential() {
    let _guard = ctfs::lock();
    let root = std::env::temp_dir().join(format!("quake-ctest-fssteam-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let steamroot = root.join("steam");
    let lib_a = root.join("lib-a");
    let lib_b = root.join("lib b"); // spaces + escapes exercised below
    std::fs::create_dir_all(&steamroot).unwrap();

    let vdf = format!(
        concat!(
            "\"libraryfolders\"\n",
            "{{\n",
            "\t\"0\"\n",
            "\t{{\n",
            "\t\t\"path\"\t\t\"{}\"\n",
            "\t\t\"label\"\t\t\"esc \\\"quoted\\\" \\t tab\"\n",
            "\t\t\"apps\"\n",
            "\t\t{{\n",
            "\t\t\t\"70\"\t\t\"123456\"\n",
            "\t\t}}\n",
            "\t}}\n",
            "\t\"1\"\n",
            "\t{{\n",
            "\t\t\"path\"\t\t\"{}\"\n",
            "\t\t\"apps\"\n",
            "\t\t{{\n",
            "\t\t\t\"2310\"\t\t\"999\"\n",
            "\t\t\t\"440\"\t\t\"1\"\n",
            "\t\t}}\n",
            "\t}}\n",
            "}}\n"
        ),
        vdf_path(&lib_a),
        vdf_path(&lib_b)
    );
    write(&steamroot, "config/libraryfolders.vdf", &vdf);
    write(
        &lib_b,
        "steamapps/appmanifest_2310.acf",
        concat!(
            "\"AppState\"\n",
            "{\n",
            "\t\"appid\"\t\t\"2310\"\n",
            "\t\"name\"\t\t\"Quake\"\n",
            "\t\"installdir\"\t\t\"Quake\"\n",
            "}\n"
        ),
    );

    // --- happy path: library resolved through both parsers -----------------
    ctfs::set_steam_dir(Some(steamroot.to_str().unwrap()));
    let found = find_game_compare(2310, "happy path");
    assert!(found.ret, "fixture must be found");
    assert_eq!(found.appid, 2310);
    let lib_str = lib_b.to_str().unwrap();
    assert_eq!(
        &found.library[..lib_str.len()],
        lib_str.as_bytes(),
        "library string"
    );
    assert_eq!(
        found.subdir_offset,
        Some(lib_str.len() + 1),
        "subdir packed right after the library NUL"
    );
    assert_eq!(
        &found.library[lib_str.len() + 1..lib_str.len() + 7],
        b"Quake\0",
        "packed subdir"
    );

    // ResolvePath: fits, truncates (still written), and tiny buffers
    let mut game = zeroed_game();
    // SAFETY: valid out-param; the fixture makes it succeed
    unsafe { (ctfs::fns(Side::C).steam_find_game)(&mut game, 2310) };
    for pathsize in [1024usize, 64, 16, 2, 1, 0] {
        resolve_path_compare(&game, pathsize, "resolved game");
    }
    // subdir == NULL: false without touching the buffer
    let empty = zeroed_game();
    for pathsize in [64usize, 0] {
        resolve_path_compare(&empty, pathsize, "unset game");
    }

    // --- Steam_IsValidPath ---------------------------------------------------
    assert!(is_valid_path_compare(
        &std::ffi::CString::new(steamroot.to_str().unwrap()).unwrap(),
        "steam root"
    ));
    assert!(!is_valid_path_compare(
        &std::ffi::CString::new(lib_b.to_str().unwrap()).unwrap(),
        "library dir"
    ));
    assert!(!is_valid_path_compare(
        &std::ffi::CString::new("a".repeat(1100)).unwrap(),
        "overlong path"
    ));

    // --- missing / malformed inputs -----------------------------------------
    // no Steam dir at all
    ctfs::set_steam_dir(None);
    let missing = find_game_compare(2310, "no steam dir");
    assert!(!missing.ret);
    assert!(
        missing
            .log
            .iter()
            .any(|l| l.contains("Steam library not found")),
        "got {:?}",
        missing.log
    );
    ctfs::set_steam_dir(Some(steamroot.to_str().unwrap()));

    // appid not present in any library
    let noapp = find_game_compare(9999, "unknown appid");
    assert!(!noapp.ret);
    assert!(
        noapp
            .log
            .iter()
            .any(|l| l.contains("Couldn't parse Steam library")),
        "got {:?}",
        noapp.log
    );

    // malformed VDF variants: parse aborts identically
    for (content, tag) in [
        ("\"libraryfolders\"\n{\n\t\"0\"\n\t{\n", "premature end"),
        ("\"libraryfolders\" \"x\" garbage", "trailing garbage"),
        ("\"a\" \"bad \\x escape\"", "unsupported escape"),
        ("noquotes", "no leading quote"),
        ("", "empty file"),
    ] {
        write(&steamroot, "config/libraryfolders.vdf", content);
        let r = find_game_compare(2310, tag);
        assert!(!r.ret, "{tag} must fail");
    }
    write(&steamroot, "config/libraryfolders.vdf", &vdf);

    // manifest missing
    std::fs::remove_file(lib_b.join("steamapps/appmanifest_2310.acf")).unwrap();
    let nomanifest = find_game_compare(2310, "missing manifest");
    assert!(!nomanifest.ret);
    assert!(
        nomanifest
            .log
            .iter()
            .any(|l| l.contains("Couldn't read Steam manifest")),
        "got {:?}",
        nomanifest.log
    );

    // manifest without installdir
    write(
        &lib_b,
        "steamapps/appmanifest_2310.acf",
        "\"AppState\"\n{\n\t\"appid\"\t\"2310\"\n}\n",
    );
    let noinstall = find_game_compare(2310, "manifest without installdir");
    assert!(!noinstall.ret);
    assert!(
        noinstall
            .log
            .iter()
            .any(|l| l.contains("Couldn't parse Steam manifest")),
        "got {:?}",
        noinstall.log
    );

    // library path so long the manifest path can't be formatted. Sized off
    // MAX_OSPATH (PATH_MAX, 260..4096 depending on the platform) rather than
    // a literal: the C's guard is `q_snprintf(...) >= sizeof (path)` with
    // path[MAX_OSPATH], so a library at least MAX_OSPATH long always
    // overflows once "/steamapps/appmanifest_2310.acf" is appended
    let long_lib = format!("/{}", "L".repeat(MAX_OSPATH));
    write(
        &steamroot,
        "config/libraryfolders.vdf",
        &format!(
            "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\"{}\"\n\t\t\"apps\"\n\t\t{{\n\t\t\t\"2310\"\t\"1\"\n\t\t}}\n\t}}\n}}\n",
            long_lib
        ),
    );
    let toolong = find_game_compare(2310, "manifest path too long");
    assert!(!toolong.ret);
    assert!(
        toolong.log.iter().any(|l| l.contains("path too long")),
        "got {:?}",
        toolong.log
    );

    ctfs::set_steam_dir(None);
}

#[test]
fn egs_differential() {
    let _guard = ctfs::lock();
    let root = std::env::temp_dir().join(format!("quake-ctest-fsegs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let manifests = root.join("Manifests");
    std::fs::create_dir_all(&manifests).unwrap();

    let ns = c"f57987ad149c43b3a7a66a7f10828f92";
    let item = c"19e3c0be6d6c4d4b84b1bc2248f94b43";
    let app = c"18161d3ef68e4166968036626d173f25";

    // --- phase 1: launcher data ---------------------------------------------
    let launcher_hit = format!(
        concat!(
            "{{\"InstallationList\": [",
            "{{\"NamespaceId\": \"other\", \"ItemId\": \"x\", \"AppName\": \"y\", \"InstallLocation\": \"/wrong\"}},",
            "{{\"NamespaceId\": \"{ns}\", \"ItemId\": \"{item}\", \"AppName\": \"{app}\", \"InstallLocation\": \"\"}},",
            "{{\"NamespaceId\": \"{ns}\", \"ItemId\": \"{item}\", \"AppName\": \"{app}\", \"InstallLocation\": \"/egs/QuakeRemaster\"}}",
            "]}}"
        ),
        ns = ns.to_str().unwrap(),
        item = item.to_str().unwrap(),
        app = app.to_str().unwrap()
    );

    ctfs::set_egs_manifest_dir(None);
    ctfs::set_egs_launcher_data(Some(&launcher_hit));
    let (ret, buf) = egs_find_compare(ns, item, app, 512, "launcher data hit");
    assert!(ret);
    assert!(buf.starts_with(b"/egs/QuakeRemaster\0"), "install location");

    // empty InstallLocation entries are skipped; no other match, no manifest
    // dir either -> false
    ctfs::set_egs_launcher_data(Some(
        "{\"InstallationList\": [{\"NamespaceId\": \"other\", \"ItemId\": \"x\", \"AppName\": \"y\", \"InstallLocation\": \"/wrong\"}]}",
    ));
    let (ret, _) = egs_find_compare(ns, item, app, 512, "launcher data no match");
    assert!(!ret);

    // corrupt launcher JSON falls through to the manifest scan
    ctfs::set_egs_launcher_data(Some("{ not json"));
    let (ret, _) = egs_find_compare(ns, item, app, 512, "corrupt launcher data, no manifests");
    assert!(!ret);

    // --- phase 2: *.item manifests ------------------------------------------
    ctfs::set_egs_launcher_data(None);
    ctfs::set_egs_manifest_dir(Some(manifests.to_str().unwrap()));

    // a directory with the .item suffix: skipped by the FA_DIRECTORY check
    std::fs::create_dir_all(manifests.join("aaa-dir.item")).unwrap();
    // wrong ids
    write(
        &root,
        "Manifests/bbb.item",
        "{\"CatalogNamespace\": \"other\", \"CatalogItemId\": \"x\", \"AppName\": \"y\", \"InstallLocation\": \"/wrong\"}",
    );
    // right ids but incomplete install
    write(
        &root,
        "Manifests/ccc.item",
        &format!(
            "{{\"CatalogNamespace\": \"{}\", \"CatalogItemId\": \"{}\", \"AppName\": \"{}\", \"InstallLocation\": \"/egs/incomplete\", \"bIsIncompleteInstall\": true}}",
            ns.to_str().unwrap(),
            item.to_str().unwrap(),
            app.to_str().unwrap()
        ),
    );
    // not JSON at all
    write(&root, "Manifests/ddd.item", "certainly { not json");
    // wrong extension: never scanned
    write(&root, "Manifests/eee.txt", "{}");
    // the winner
    write(
        &root,
        "Manifests/fff.item",
        &format!(
            "{{\"CatalogNamespace\": \"{}\", \"CatalogItemId\": \"{}\", \"AppName\": \"{}\", \"InstallLocation\": \"/egs/QuakeFromManifest\", \"bIsIncompleteInstall\": false}}",
            ns.to_str().unwrap(),
            item.to_str().unwrap(),
            app.to_str().unwrap()
        ),
    );

    let before = ctfs::open_handle_count();
    let (ret, buf) = egs_find_compare(ns, item, app, 512, "manifest scan hit");
    assert!(ret);
    assert!(
        buf.starts_with(b"/egs/QuakeFromManifest\0"),
        "manifest location"
    );
    assert_eq!(
        ctfs::open_handle_count(),
        before,
        "manifest scan leaks nothing"
    );

    // remove the winner: everything else is skipped for its own reason
    std::fs::remove_file(manifests.join("fff.item")).unwrap();
    let (ret, _) = egs_find_compare(ns, item, app, 512, "manifest scan no match");
    assert!(!ret);

    // no manifest dir either
    ctfs::set_egs_manifest_dir(None);
    let (ret, _) = egs_find_compare(ns, item, app, 512, "nothing configured");
    assert!(!ret);
}
