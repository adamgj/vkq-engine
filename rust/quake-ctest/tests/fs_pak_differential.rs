//! Differential tests: pak mounting + the COM_FindFile family — the Rust fs
//! shims (quake_rs `fs` feature) vs the original common_fs.c compiled as
//! c_ref_*. Both sides mount the same synthetic pak fixtures through the same
//! stub Sys_File* layer and the full observable state is compared: the
//! searchpath list shape, the flag globals, the con/Sys_Printf log, the
//! Sys_Error messages for fatal inputs, and the find/open/load results.

use core::ffi::{c_int, c_uint, CStr};
use quake_ctest::fs as ctfs;
use quake_ctest::fs::{Side, BOTH};

const PAK0_CRC_V106: u16 = 32981;

/// Builds a PACK image from (name, payload) entries.
fn build_pak(entries: &[(&[u8], &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"PACK");
    let mut payload_of = Vec::new();
    let mut ofs = 12usize;
    for (_, data) in entries {
        payload_of.push(ofs);
        ofs += data.len();
    }
    out.extend_from_slice(&(ofs as i32).to_le_bytes()); // dirofs
    out.extend_from_slice(&((entries.len() * 64) as i32).to_le_bytes()); // dirlen
    for (_, data) in entries {
        out.extend_from_slice(data);
    }
    for (i, (name, data)) in entries.iter().enumerate() {
        let mut n = [0u8; 56];
        let len = name.len().min(55);
        n[..len].copy_from_slice(&name[..len]);
        out.extend_from_slice(&n);
        out.extend_from_slice(&(payload_of[i] as i32).to_le_bytes());
        out.extend_from_slice(&(data.len() as i32).to_le_bytes());
    }
    out
}

struct Fixture {
    root: std::path::PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("quake-ctest-fspak-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("tg")).unwrap();
        Fixture { root }
    }

    fn write(&self, rel: &str, bytes: &[u8]) {
        let path = self.root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    /// Mounts "tg" on one side; returns Sys_Error message if the mount
    /// fataled, plus the con-log lines it produced.
    fn mount(&self, side: Side) -> (Option<String>, Vec<String>) {
        ctfs::clear_logs();
        let err = ctfs::catch_sys_error(|| ctfs::setup(side, &[&self.root], 0, c"tg"));
        (err, ctfs::con_log())
    }

    /// Mounts on both sides and asserts every observable output matches.
    /// Returns the (identical) snapshot.
    ///
    /// A fatal input longjmps out of the mount. That is legal for the c_ref
    /// side (pure C frames) but UB across the Rust shim's, so when the C
    /// fatals the Rust side is probed in a child process and only the
    /// messages are compared — there is no post-fatal state to diff anyway.
    /// See fs::catch_sys_error / PLAN.md §4.
    fn mount_both_and_compare(&self, ctx: &str) -> (Option<String>, ctfs::FsSnapshot) {
        let (c_err, c_log) = self.mount(Side::C);
        let c_snap = ctfs::snapshot(Side::C);

        if let Some(c_msg) = &c_err {
            let r_msg = ctfs::rust_fatal_in_child(
                "rust_fatal_child",
                "mount",
                &[("CTEST_FATAL_ROOT", self.root.to_str().unwrap())],
            )
            .unwrap_or_else(|| panic!("the Rust shim must Sys_Error too: {ctx}"));
            assert_eq!(*c_msg, r_msg, "Sys_Error message parity: {ctx}");
            return (c_err, c_snap);
        }

        let (r_err, r_log) = self.mount(Side::Rust);
        let r_snap = ctfs::snapshot(Side::Rust);
        assert_eq!(c_err, r_err, "Sys_Error parity: {ctx}");
        assert_eq!(c_log, r_log, "con log parity: {ctx}");
        assert_eq!(c_snap, r_snap, "state parity: {ctx}");
        (c_err, c_snap)
    }
}

fn find_compare(name: &CStr, ctx: &str) {
    let mut results = Vec::new();
    for side in BOTH {
        let f = ctfs::fns(side);
        // COM_FileExists
        let mut exists_path_id: c_uint = 12345;
        // SAFETY: NUL-terminated name, valid out-param; fs mounted under lock
        let exists = unsafe { (f.file_exists)(name.as_ptr(), &mut exists_path_id) };

        // COM_OpenFile + full read through the handle + COM_CloseFile
        let mut handle: c_int = -2;
        let mut open_path_id: c_uint = 12345;
        // SAFETY: as above
        let open_len = unsafe { (f.open_file)(name.as_ptr(), &mut handle, &mut open_path_id) };
        let open_meta = (
            open_len,
            ctfs::thread_file_size(),
            ctfs::thread_file_from_pak(),
        );
        let mut open_content = None;
        if handle != -1 {
            let mut buf = vec![0u8; open_len.max(0) as usize];
            // SAFETY: handle is open; buf sized for the read
            unsafe {
                quake_c_sys::Sys_FileRead(handle, buf.as_mut_ptr().cast(), buf.len() as c_int);
                (f.close_file)(handle);
            }
            open_content = Some(buf);
        }

        // COM_FOpenFile + stdio read
        let mut file: *mut quake_c_sys::FILE = core::ptr::null_mut();
        let mut fopen_path_id: c_uint = 12345;
        // SAFETY: as above
        let fopen_len = unsafe { (f.fopen_file)(name.as_ptr(), &mut file, &mut fopen_path_id) };
        let fopen_meta = (
            fopen_len,
            ctfs::thread_file_size(),
            ctfs::thread_file_from_pak(),
        );
        let mut fopen_content = None;
        if !file.is_null() {
            let mut buf = vec![0u8; fopen_len.max(0) as usize];
            // SAFETY: file is an open stream; buf sized for the read
            unsafe {
                quake_c_sys::stdio::fread(buf.as_mut_ptr().cast(), 1, buf.len(), file);
                quake_c_sys::stdio::fclose(file);
            }
            fopen_content = Some(buf);
        }

        // COM_LoadFile
        let loaded = ctfs::load_file(side, name);

        results.push((
            exists,
            exists_path_id,
            open_meta,
            open_path_id,
            handle == -1,
            open_content,
            fopen_meta,
            fopen_path_id,
            fopen_content,
            loaded,
        ));
    }
    assert_eq!(
        results[0], results[1],
        "find/open/load parity for {name:?} ({ctx})"
    );
}

#[test]
fn pak_differential() {
    let _guard = ctfs::lock();
    ctfs::set_host_dirs("/nonexistent-host-basedir", None);
    ctfs::set_registered(1.0);
    ctfs::set_developer(0.0);

    // --- case 1: healthy pak0 + pak1, loose files, embedded pak parity -----
    {
        let fx = Fixture::new("healthy");
        fx.write(
            "tg/pak0.pak",
            &build_pak(&[
                (b"shadowed.txt", b"pak version"),
                (b"maps/inpak.bsp", b"pak map payload"),
                (b"sound/misc/water1.wav", b"WAVE"),
            ]),
        );
        fx.write(
            "tg/pak1.pak",
            &build_pak(&[(b"shadowed.txt", b"pak1 wins"), (b"pak1only.txt", b"1")]),
        );
        fx.write("tg/shadowed.txt", b"dir version");
        fx.write("tg/loose.txt", b"loose content");
        fx.write("tg/maps/loosemap.txt", b"loose subdir content");

        let (err, snap) = fx.mount_both_and_compare("healthy pak0+pak1");
        assert_eq!(err, None);
        // dir node + pak0 + embedded vkquake.pak + pak1, head-first reversed:
        // pak1 on top, then vkquake.pak, then pak0, then the dir
        let names: Vec<Option<&str>> = snap
            .searchpaths
            .iter()
            .map(|s| {
                s.pack
                    .as_ref()
                    .map(|p| p.filename.rsplit('/').next().unwrap())
            })
            .collect();
        assert_eq!(
            names,
            vec![
                Some("pak1.pak"),
                Some("vkquake.pak"),
                Some("pak0.pak"),
                None
            ],
            "searchpath order"
        );
        assert!(snap.modified, "pak counts differ from retail: modified");

        for name in [
            c"shadowed.txt",
            c"maps/inpak.bsp",
            c"sound/misc/water1.wav",
            c"pak1only.txt",
            c"loose.txt",
            c"maps/loosemap.txt",
            c"embedded/marker.txt",
            c"missing.txt",
            c"maps/missing.bsp",
        ] {
            find_compare(name, "healthy, registered");
        }

        // pak shadowing: pak1 beats pak0 beats the loose file
        let (bytes, _, from_pak, _) = ctfs::load_file(Side::C, c"shadowed.txt").unwrap();
        assert_eq!(bytes, b"pak1 wins");
        assert_eq!(from_pak, 1);
        // the embedded pak content round-trips through both inflators
        let (bytes, _, _, _) = ctfs::load_file(Side::Rust, c"embedded/marker.txt").unwrap();
        assert_eq!(bytes, b"embedded vkquake.pak content marker\n");

        // shareware gate: loose files with path separators are skipped, pak
        // entries are not
        ctfs::set_registered(0.0);
        for name in [
            c"maps/inpak.bsp",
            c"maps/loosemap.txt",
            c"loose.txt",
            c"shadowed.txt",
            c"embedded/marker.txt",
        ] {
            find_compare(name, "healthy, shareware");
        }
        let mut path_id: c_uint = 0;
        // SAFETY: fs mounted; NUL-terminated name and valid out-param
        let gated = unsafe {
            (ctfs::fns(Side::C).file_exists)(c"maps/loosemap.txt".as_ptr(), &mut path_id)
        };
        assert!(!gated, "shareware gate must hide loose subdir files");
        ctfs::set_registered(1.0);

        // developer > 1: the miss also prints
        ctfs::set_developer(2.0);
        ctfs::clear_logs();
        assert!(ctfs::load_file(Side::C, c"missing.txt").is_none());
        let c_log = ctfs::con_log();
        ctfs::clear_logs();
        assert!(ctfs::load_file(Side::Rust, c"missing.txt").is_none());
        assert_eq!(c_log, ctfs::con_log(), "developer miss log");
        assert!(
            c_log.iter().any(|l| l.contains("can't find missing.txt")),
            "expected FindFile miss line, got {c_log:?}"
        );
        ctfs::set_developer(0.0);

        // COM_CloseFile: a mounted pak's own handle is never really closed,
        // while the dup handle from COM_OpenFile is
        for side in BOTH {
            let f = ctfs::fns(side);
            let paks = ctfs::pak_handles(side);
            assert_eq!(paks.len(), 3);
            let before = ctfs::open_handle_count();
            let mut handle: c_int = -1;
            // SAFETY: fs mounted; valid out-params
            unsafe {
                (f.open_file)(c"pak1only.txt".as_ptr(), &mut handle, core::ptr::null_mut());
            }
            assert_ne!(handle, -1);
            assert_eq!(
                ctfs::open_handle_count(),
                before + 1,
                "dup handle allocated"
            );
            // SAFETY: handle from COM_OpenFile; pak handles from the list walk
            unsafe {
                (f.close_file)(handle);
                assert_eq!(ctfs::open_handle_count(), before, "dup handle closed");
                for pak_handle in &paks {
                    (f.close_file)(*pak_handle);
                }
            }
            assert_eq!(
                ctfs::open_handle_count(),
                before,
                "pak handles must survive COM_CloseFile ({side:?})"
            );
        }
    }

    // --- case 2: zero-entry pak: warning, ignored, scan stops --------------
    // Mounted as the *second* gamedir (path_id == 2) so the embedded
    // vkquake.pak does not get added and the scan really stops; the
    // path_id == 1 variant (where the embedded pak keeps the scan going)
    // lives in zero_entry_pak0_with_embedded_divergence below.
    {
        let fx = Fixture::new("empty");
        std::fs::create_dir_all(fx.root.join("tg0")).unwrap();
        fx.write("tg/pak0.pak", &build_pak(&[]));
        fx.write(
            "tg/pak1.pak",
            &build_pak(&[(b"never.txt", b"never scanned")]),
        );

        let mut outputs = Vec::new();
        for side in BOTH {
            ctfs::clear_logs();
            let err = ctfs::catch_sys_error(|| ctfs::setup(side, &[&fx.root], 0, c"tg0;tg"));
            outputs.push((err, ctfs::con_log(), ctfs::snapshot(side)));
        }
        assert_eq!(outputs[0], outputs[1], "zero-entry pak0 parity");
        let (err, log, snap) = &outputs[0];
        assert_eq!(*err, None);
        assert!(
            log.iter().any(|l| l.contains("has no files, ignored")),
            "expected the empty-pak warning, got {log:?}"
        );
        assert_eq!(
            snap.searchpaths.iter().filter(|s| s.pack.is_some()).count(),
            0,
            "no pak mounted"
        );
        find_compare(c"never.txt", "zero-entry pak0 stops the scan");
    }

    // --- case 3: corrupt magic fatals identically --------------------------
    {
        let fx = Fixture::new("badmagic");
        let mut pak = build_pak(&[(b"x.txt", b"x")]);
        pak[0..4].copy_from_slice(b"PAKC");
        fx.write("tg/pak0.pak", &pak);
        let (err, _snap) = fx.mount_both_and_compare("corrupt magic");
        let msg = err.expect("corrupt magic must Sys_Error");
        assert!(
            msg.contains("is not a packfile") && msg.contains("tg/pak0.pak"),
            "unexpected message: {msg}"
        );
        for side in BOTH {
            ctfs::reset(side); // clear the partial mount
        }
    }

    // --- case 4: negative dirlen / dirofs fatal identically -----------------
    for (dirofs, dirlen, tag) in [
        (-8i32, 64i32, "negofs"),
        (12, -64, "neglen"),
        (-4, -4, "negboth"),
    ] {
        let fx = Fixture::new(tag);
        let mut pak = Vec::new();
        pak.extend_from_slice(b"PACK");
        pak.extend_from_slice(&dirofs.to_le_bytes());
        pak.extend_from_slice(&dirlen.to_le_bytes());
        pak.extend_from_slice(&[0u8; 64]);
        fx.write("tg/pak0.pak", &pak);
        let (err, _snap) = fx.mount_both_and_compare(tag);
        let msg = err.expect("negative dir must Sys_Error");
        assert!(
            msg.contains("Invalid packfile")
                && msg.contains(&format!("dirlen: {}, dirofs: {}", dirlen, dirofs)),
            "unexpected message: {msg}"
        );
        for side in BOTH {
            ctfs::reset(side);
        }
    }

    // --- case 5: 2048 entries mounts, 2049 fatals ---------------------------
    {
        let names: Vec<Vec<u8>> = (0..2049)
            .map(|i| format!("f{:04}.dat", i).into_bytes())
            .collect();
        let full: Vec<(&[u8], &[u8])> = names.iter().map(|n| (n.as_slice(), &b"d"[..])).collect();

        let fx = Fixture::new("max2048");
        fx.write("tg/pak0.pak", &build_pak(&full[..2048]));
        let (err, snap) = fx.mount_both_and_compare("2048 entries");
        assert_eq!(err, None);
        let pak0 = snap.searchpaths[1].pack.as_ref().expect("pak0 mounted");
        assert_eq!(pak0.numfiles, 2048);
        find_compare(c"f2047.dat", "2048-entry pak");

        let fx = Fixture::new("over2048");
        fx.write("tg/pak0.pak", &build_pak(&full));
        let (err, _snap) = fx.mount_both_and_compare("2049 entries");
        let msg = err.expect("2049 entries must Sys_Error");
        assert!(msg.contains("has 2049 files"), "unexpected message: {msg}");
        for side in BOTH {
            ctfs::reset(side);
        }
    }

    // --- case 6: retail directory CRC + count keeps com_modified false ------
    {
        // 339 zero-length entries = PAK0_COUNT; brute-force two padding bytes
        // in the last entry's name field (after its NUL, invisible to the
        // parse) until the directory CRC matches the v1.06 retail value
        let names: Vec<Vec<u8>> = (0..339)
            .map(|i| format!("retail{:03}.lmp", i).into_bytes())
            .collect();
        let entries: Vec<(&[u8], &[u8])> = names.iter().map(|n| (n.as_slice(), &b""[..])).collect();
        let mut pak = build_pak(&entries);
        let dirofs = 12usize;
        let dirlen = 339 * 64;
        // padding bytes 54/55 of the last entry's 56-byte name field
        let tweak = dirofs + dirlen - 64 + 54;

        let mut found = false;
        'search: for a in 0..=255u8 {
            for b in 0..=255u8 {
                pak[tweak] = a;
                pak[tweak + 1] = b;
                if quake_ctest::c_crc_block(&pak[dirofs..dirofs + dirlen]) == PAK0_CRC_V106 {
                    found = true;
                    break 'search;
                }
            }
        }
        assert!(found, "no 2-byte tweak reaches the retail CRC");

        let fx = Fixture::new("retailcrc");
        fx.write("tg/pak0.pak", &pak);
        let (err, snap) = fx.mount_both_and_compare("retail CRC + count");
        assert_eq!(err, None);
        assert!(
            !snap.modified,
            "retail count + CRC must keep com_modified false (embedded pak restores it)"
        );

        // flip one padding byte: same entries, non-retail CRC, modified
        pak[tweak] ^= 0xff;
        let fx = Fixture::new("badcrc");
        fx.write("tg/pak0.pak", &pak);
        let (err, snap) = fx.mount_both_and_compare("non-retail CRC");
        assert_eq!(err, None);
        assert!(snap.modified, "non-retail CRC must set com_modified");
    }

    for side in BOTH {
        ctfs::reset(side);
    }
}

/// FIXED DIVERGENCE (found by this suite; this test now guards the fix):
///
/// Input: main basedir gamedir mounted as the first game directory
/// (path_id == 1, embedded pak eligible) containing a ZERO-ENTRY pak0.pak
/// and a valid pak1.pak.
///
/// C (common_fs.c COM_AddGameDirectoryRoot): COM_LoadPackFile returns NULL
/// for the empty pak0, but the embedded-pak block then reassigns the same
/// `pak` variable (`pak = COM_LoadPackFile ("vkquake.pak", packhandle)`), so
/// the `if (!pak) break;` check passes and the scan CONTINUES to pak1.pak —
/// pak1 gets mounted (and its non-retail count sets com_modified).
///
/// Rust used to bind `let pak_ptr = ...` inside the embedded block, shadowing
/// the outer binding: the outer pak_ptr stayed null, the loop broke, and
/// pak1.pak was never scanned. add_game_directory_root now reassigns the
/// OUTER pak_ptr, so this test guards that bug-compatible C behavior.
#[test]
fn zero_entry_pak0_with_embedded_divergence() {
    let _guard = ctfs::lock();
    ctfs::set_host_dirs("/nonexistent-host-basedir", None);
    ctfs::set_registered(1.0);

    let fx = Fixture::new("empty-embedded");
    fx.write("tg/pak0.pak", &build_pak(&[]));
    fx.write(
        "tg/pak1.pak",
        &build_pak(&[(b"scanned.txt", b"pak1 reached through the embedded pak")]),
    );
    let (err, snap) = fx.mount_both_and_compare("zero-entry pak0 at path_id 1");
    assert_eq!(err, None);
    // the C behavior both sides must show: the embedded pak keeps the loop
    // alive past the ignored pak0, so pak1 IS mounted
    let pak_names: Vec<&str> = snap
        .searchpaths
        .iter()
        .filter_map(|s| s.pack.as_ref())
        .map(|p| p.filename.rsplit('/').next().unwrap())
        .collect();
    assert_eq!(pak_names, vec!["pak1.pak", "vkquake.pak"], "pak1 mounted");
    find_compare(c"scanned.txt", "pak1 reached past the empty pak0");

    for side in BOTH {
        ctfs::reset(side);
    }
}

/// Child-process half of the Rust-side fatal probes (see
/// `ctfs::rust_fatal_in_child`). A no-op unless CTEST_FATAL_CASE selects a
/// scenario, so it costs nothing in a normal run.
///
/// The trap is deliberately NOT armed here: the stub's `Sys_Error` prints
/// `Sys_Error: <msg>` to stderr and aborts, which is what the parent reads
/// back. Nothing longjmps through a Rust frame. The fixture is the one the
/// parent already wrote, reached through CTEST_FATAL_ROOT.
#[test]
fn rust_fatal_child() {
    let Some(case) = ctfs::fatal_child_case() else {
        return;
    };
    assert_eq!(case, "mount", "unknown fatal case");

    let root = std::path::PathBuf::from(std::env::var("CTEST_FATAL_ROOT").expect("root"));
    ctfs::set_host_dirs("/nonexistent-host-basedir", None);
    ctfs::set_registered(1.0);
    ctfs::set_developer(0.0);
    ctfs::setup(Side::Rust, &[&root], 0, c"tg");
    panic!(
        "expected the Rust pak mount to Sys_Error for {}",
        root.display()
    );
}
