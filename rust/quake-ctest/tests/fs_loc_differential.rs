//! Differential tests: localization (LOC_*) and the FNV hashes — the Rust
//! shims (quake_rs loc + quake-fs zipdir for .kpf) vs the original
//! common_fs.c LOCALIZATION block (with its in-TU miniz) compiled as
//! c_ref_*. Both sides acquire the same fixtures through the same stubs:
//! searchpath text file, then basedir QuakeEX.kpf (store + deflate entries),
//! plus the failure paths; lookups, format calls and console lines are
//! compared. An env-gated golden run covers the real rerelease QuakeEX.kpf.

use core::ffi::{c_char, c_int, c_void, CStr};
use quake_ctest::fs as ctfs;
use quake_ctest::fs::{Side, BOTH};

// ---------------------------------------------------------------------------
// zip fixture builder (format mirrors quake-fs's zipdir unit-test fixtures)

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    !crc
}

struct ZEntry {
    name: &'static str,
    payload: Vec<u8>,
    method: u16,
    crc: u32,
    uncomp: u32,
}

impl ZEntry {
    fn stored(name: &'static str, data: &[u8]) -> Self {
        ZEntry {
            name,
            payload: data.to_vec(),
            method: 0,
            crc: crc32(data),
            uncomp: data.len() as u32,
        }
    }

    fn deflated(name: &'static str, data: &[u8]) -> Self {
        ZEntry {
            name,
            payload: miniz_oxide::deflate::compress_to_vec(data, 6),
            method: 8,
            crc: crc32(data),
            uncomp: data.len() as u32,
        }
    }
}

fn build_zip(entries: &[ZEntry]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut local_ofs = Vec::new();
    for e in entries {
        local_ofs.push(out.len() as u32);
        out.extend_from_slice(&0x04034b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // bit flag
        out.extend_from_slice(&e.method.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // time+date
        out.extend_from_slice(&e.crc.to_le_bytes());
        out.extend_from_slice(&(e.payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&e.uncomp.to_le_bytes());
        out.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra len
        out.extend_from_slice(e.name.as_bytes());
        out.extend_from_slice(&e.payload);
    }
    let cdir_ofs = out.len() as u32;
    for (e, &lofs) in entries.iter().zip(&local_ofs) {
        out.extend_from_slice(&0x02014b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version made by
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // bit flag
        out.extend_from_slice(&e.method.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // time+date
        out.extend_from_slice(&e.crc.to_le_bytes());
        out.extend_from_slice(&(e.payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&e.uncomp.to_le_bytes());
        out.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
        out.extend_from_slice(&[0u8; 8]); // extra/comment lens, disk, int attr
        out.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        out.extend_from_slice(&lofs.to_le_bytes());
        out.extend_from_slice(e.name.as_bytes());
    }
    let cdir_size = out.len() as u32 - cdir_ofs;
    out.extend_from_slice(&0x06054b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // this disk
    out.extend_from_slice(&0u16.to_le_bytes()); // cdir disk
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&cdir_size.to_le_bytes());
    out.extend_from_slice(&cdir_ofs.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment len
    out
}

// ---------------------------------------------------------------------------

/// Runs LOC_Init on both sides and asserts the console lines match; returns
/// them.
fn loc_init_compare(ctx: &str) -> Vec<String> {
    let mut logs = Vec::new();
    for side in BOTH {
        ctfs::clear_logs();
        // SAFETY: fs mounted under FS_LOCK; single-threaded init like the C
        unsafe { (ctfs::fns(side).loc_init)() };
        logs.push(ctfs::con_log());
    }
    assert_eq!(logs[0], logs[1], "LOC_Init console parity: {ctx}");
    logs.remove(0)
}

/// Compares LOC_GetRawString + LOC_GetString for one key: null-ness, bytes,
/// and the miss-identity of GetString.
fn lookup_compare(key: &CStr, ctx: &str) -> Option<Vec<u8>> {
    let mut results = Vec::new();
    for side in BOTH {
        let f = ctfs::fns(side);
        // SAFETY: NUL-terminated key; loc data owned by the side until the
        // next LOC_Init/Shutdown, both under FS_LOCK
        unsafe {
            let raw = (f.loc_get_raw_string)(key.as_ptr());
            let get = (f.loc_get_string)(key.as_ptr());
            let raw_bytes = if raw.is_null() {
                None
            } else {
                Some(CStr::from_ptr(raw).to_bytes().to_vec())
            };
            let get_is_input = core::ptr::eq(get, key.as_ptr());
            let get_bytes = CStr::from_ptr(get).to_bytes().to_vec();
            results.push((raw_bytes, get_is_input, get_bytes));
        }
    }
    assert_eq!(results[0], results[1], "lookup parity for {key:?} ({ctx})");
    let (raw, miss_identity, _) = results.remove(0);
    assert_eq!(
        raw.is_none(),
        miss_identity,
        "GetString identity on miss only"
    );
    raw
}

const ARGS: [&CStr; 6] = [
    c"ARG-ZERO",
    c"argument-one-long",
    c"x",
    c"",
    c"four",
    c"55555555",
];

unsafe extern "C" fn getarg(idx: c_int, _userdata: *mut c_void) -> *const c_char {
    ARGS.get(idx.max(0) as usize).unwrap_or(&c"").as_ptr()
}

fn format_compare(fmt: &CStr, out_len: usize, ctx: &str) {
    let mut results = Vec::new();
    for side in BOTH {
        let f = ctfs::fns(side);
        let mut out = vec![0x7fu8; out_len.max(1)];
        ctfs::clear_logs();
        // SAFETY: NUL-terminated format; out is writable for out_len bytes
        // (or unused when out_len == 0, matching the C contract); getarg
        // returns valid static strings for every index
        let written = unsafe {
            (f.loc_format)(
                fmt.as_ptr(),
                Some(getarg),
                core::ptr::null_mut(),
                if out_len == 0 {
                    core::ptr::null_mut()
                } else {
                    out.as_mut_ptr().cast()
                },
                out_len,
            )
        };
        results.push((written, out, ctfs::con_log()));
    }
    assert_eq!(
        results[0], results[1],
        "LOC_Format parity for {fmt:?} len={out_len} ({ctx})"
    );
}

fn loc_corpus() -> Vec<u8> {
    let mut c: Vec<u8> = Vec::new();
    c.extend_from_slice(b"\xEF\xBB\xBF// BOM + header comment\n");
    c.extend_from_slice(b"/ malformed comment line\n");
    c.extend_from_slice(b"/\n");
    c.extend_from_slice(b"KEY_SIMPLE=hello world\n");
    c.extend_from_slice(b"KEY_QUOTED=\"quoted value\"\n");
    c.extend_from_slice(b"KEY_ESCAPES=\"a\\nb\\tc\\vd\\be\\ff quote\\\" apos\\' end\"\n");
    c.extend_from_slice(b"KEY_UNKNOWN_ESC=\"weird\\z\\qstuff\"\n");
    c.extend_from_slice(b"KEY_TRAILQ=\"text\"garbage after close\n");
    c.extend_from_slice(b"KEY_EMPTY=\n");
    c.extend_from_slice(b"KEY_EMPTYQ=\"\"\n");
    c.extend_from_slice(b"   KEY_LEADWS=indented line\n");
    c.extend_from_slice(b"KEY_PLACE={0} killed {1}\n");
    c.extend_from_slice(b"KEY_BRACES={}{ }{2}\n");
    c.extend_from_slice(b"KEY_CRLF=crlf line\r\n");
    c.extend_from_slice(b"NOEQUALS just a line\n");
    c.extend_from_slice(b"=value with empty key\n");
    c.extend_from_slice(b"KEY_HIGH=high\x80\xfebytes\n");
    // filler keys force modulo collisions in the numindices probe (2x load)
    for i in 0..50 {
        c.extend_from_slice(
            format!("KEY_F{:03}=filler {}\n", i, i)
                .into_bytes()
                .as_slice(),
        );
    }
    c.extend_from_slice(b"KEY_LAST=no trailing newline\n");
    // C quirk (replicated by the Rust port, verified by lookup parity): an
    // UNQUOTED value with trailing whitespace makes the trailing-trim loop
    // write its first NUL exactly over the line's newline (value_dst ==
    // cursor), so `while (*cursor)` ends and EVERYTHING after this line is
    // silently ignored — keep it last, with one sacrificial key after it.
    c.extend_from_slice(b"KEY_TRIM   =    spaced out value   \n");
    c.extend_from_slice(b"KEY_AFTER_TRIM=never parsed\n");
    c
}

fn corpus_keys() -> Vec<&'static CStr> {
    vec![
        c"$KEY_SIMPLE",
        c"$KEY_QUOTED",
        c"$KEY_ESCAPES",
        c"$KEY_UNKNOWN_ESC",
        c"$KEY_TRIM",
        c"$KEY_TRAILQ",
        c"$KEY_EMPTY",
        c"$KEY_EMPTYQ",
        c"$KEY_LEADWS",
        c"$KEY_PLACE",
        c"$KEY_BRACES",
        c"$KEY_CRLF",
        c"$KEY_HIGH",
        c"$KEY_F000",
        c"$KEY_F031",
        c"$KEY_F049",
        c"$KEY_LAST",
        c"$KEY_AFTER_TRIM",
        c"$KEY_MISSING",
        c"$",
        c"",
        c"KEY_SIMPLE", // no $ prefix
        c"$NOEQUALS",
        c"$key_simple", // case-sensitive lookup
    ]
}

#[test]
fn loc_differential() {
    let _guard = ctfs::lock();
    ctfs::set_host_dirs("/nonexistent-host-basedir", None);
    ctfs::set_registered(1.0);
    ctfs::set_developer(0.0);

    let root = std::env::temp_dir().join(format!("quake-ctest-fsloc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    // --- text-file corpus through the searchpath (COM_LoadFile) path --------
    {
        std::fs::create_dir_all(root.join("lg/localization")).unwrap();
        std::fs::write(root.join("lg/localization/loc_english.txt"), loc_corpus()).unwrap();
        for side in BOTH {
            ctfs::setup(side, &[&root], 0, c"lg");
        }

        let log = loc_init_compare("text corpus");
        assert!(
            log.iter()
                .any(|l| l.contains("Loaded") && l.contains("strings")),
            "expected a Loaded-N-strings line, got {log:?}"
        );
        assert!(
            log.iter()
                .any(|l| l.contains("unrecognized escape sequence")),
            "expected escape warnings, got {log:?}"
        );
        assert!(
            log.iter().any(|l| l.contains("malformed comment")),
            "expected malformed-comment warnings, got {log:?}"
        );

        for key in corpus_keys() {
            lookup_compare(key, "text corpus");
        }
        // spot-check the parse itself (both sides already proven equal)
        assert_eq!(
            lookup_compare(c"$KEY_ESCAPES", "escape content"),
            Some(b"a\nb\tc\x0bd\x08e\x0cf quote\" apos' end".to_vec())
        );
        // the C's trailing trim zeroes *value_dst BEFORE decrementing, so the
        // first trailing blank survives ("value " not "value"); the Rust port
        // replicates the off-by-one bug-compatibly
        assert_eq!(
            lookup_compare(c"$KEY_TRIM", "trim content"),
            Some(b"spaced out value ".to_vec())
        );
        // ... and the same trim NUL lands on the newline, ending the parse:
        // nothing after the trailing-whitespace line is ever loaded
        assert_eq!(
            lookup_compare(c"$KEY_AFTER_TRIM", "post-trim truncation"),
            None
        );
        assert_eq!(
            lookup_compare(c"$KEY_TRAILQ", "trailing quote content"),
            Some(b"text".to_vec())
        );
        assert_eq!(
            lookup_compare(c"$KEY_CRLF", "CR kept in unquoted value"),
            Some(b"crlf line\r".to_vec())
        );

        // placeholders with data loaded
        for s in [c"{0}", c"none", c"{12}", c"{x}", c"{", c"{}", c"a{1}b"] {
            // SAFETY: NUL-terminated inputs; loc data loaded
            let (c_v, r_v) = unsafe {
                (
                    (ctfs::fns(Side::C).loc_has_placeholders)(s.as_ptr()),
                    (ctfs::fns(Side::Rust).loc_has_placeholders)(s.as_ptr()),
                )
            };
            assert_eq!(c_v, r_v, "LOC_HasPlaceholders({s:?})");
        }

        // format vectors (incl. overflow and {N} args)
        for fmt in [
            c"{0} and {1}",
            c"a{}b",
            c"{1}{0}{1}",
            c"{5}",
            c"{9}",
            c"no placeholders at all",
            c"{unterminated",
            c"trailing {",
            c"{0}{1}{2}{3}{4}{5} overflowing tail text",
        ] {
            for out_len in [64usize, 8, 4, 1, 0] {
                format_compare(fmt, out_len, "format vectors");
            }
        }
    }

    // --- kpf acquisition path (store + deflate entries) ---------------------
    {
        let root2 = std::env::temp_dir().join(format!("quake-ctest-fskpf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root2);
        std::fs::create_dir_all(&root2).unwrap();

        let corpus = loc_corpus();
        let kpf = build_zip(&[
            ZEntry::stored("other/readme.txt", b"stored filler"),
            ZEntry::deflated("localization/loc_english.txt", &corpus),
        ]);
        std::fs::write(root2.join("QuakeEX.kpf"), &kpf).unwrap();
        for side in BOTH {
            ctfs::setup(side, &[&root2], 0, c"lgx");
        }

        let log = loc_init_compare("kpf deflate entry");
        assert!(
            log.iter()
                .any(|l| l.contains("Loaded") && l.contains("strings")),
            "kpf load must succeed, got {log:?}"
        );
        for key in corpus_keys() {
            lookup_compare(key, "kpf deflate entry");
        }

        // stored-entry variant
        let kpf = build_zip(&[ZEntry::stored("localization/loc_english.txt", &corpus)]);
        std::fs::write(root2.join("QuakeEX.kpf"), &kpf).unwrap();
        let log = loc_init_compare("kpf stored entry");
        assert!(
            log.iter().any(|l| l.contains("Loaded")),
            "stored kpf: {log:?}"
        );
        lookup_compare(c"$KEY_SIMPLE", "kpf stored entry");

        // kpf without the loc entry: both fail with the same console line
        let kpf = build_zip(&[ZEntry::stored("other/readme.txt", b"filler")]);
        std::fs::write(root2.join("QuakeEX.kpf"), &kpf).unwrap();
        let log = loc_init_compare("kpf missing entry");
        assert!(
            log.iter().any(|l| l.contains("Couldn't load")),
            "missing entry must fail, got {log:?}"
        );
        lookup_compare(c"$KEY_SIMPLE", "kpf missing entry (nothing loaded)");

        // corrupt kpf: reader init fails identically, no handle leaks
        let before = ctfs::open_handle_count();
        std::fs::write(root2.join("QuakeEX.kpf"), b"this is not a zip archive").unwrap();
        let log = loc_init_compare("corrupt kpf");
        assert!(
            log.iter().any(|l| l.contains("Couldn't load")),
            "corrupt kpf must fail, got {log:?}"
        );
        assert_eq!(
            ctfs::open_handle_count(),
            before,
            "kpf failure leaks no handles"
        );

        // no loc file, no kpf at all
        std::fs::remove_file(root2.join("QuakeEX.kpf")).unwrap();
        let log = loc_init_compare("nothing to load");
        assert!(log.iter().any(|l| l.contains("Couldn't load")), "{log:?}");
    }

    // --- LOC_Shutdown drops everything on both sides ------------------------
    for side in BOTH {
        // SAFETY: shutdown path, main thread under FS_LOCK
        unsafe { (ctfs::fns(side).loc_shutdown)() };
    }
    lookup_compare(c"$KEY_SIMPLE", "after shutdown");
    // SAFETY: NUL-terminated input; nothing loaded
    let empty = unsafe { (ctfs::fns(Side::C).loc_has_placeholders)(c"{0}".as_ptr()) };
    assert!(!empty, "no placeholders reported with nothing loaded");

    for side in BOTH {
        ctfs::reset(side);
    }
}

#[test]
fn hash_differential() {
    let _guard = ctfs::lock();

    let strings: [&CStr; 8] = [
        c"",
        c"a",
        c"abc",
        c"KEY_SIMPLE",
        c"key_simple",
        c"\x7f",
        c"\u{00ff}", // multi-byte UTF-8: exercises high-bit chars
        c"a slightly longer string with spaces and 1234567890",
    ];
    for s in strings {
        // SAFETY: NUL-terminated inputs, pure functions
        let (c_v, r_v) = unsafe {
            (
                (ctfs::fns(Side::C).hash_string)(s.as_ptr()),
                (ctfs::fns(Side::Rust).hash_string)(s.as_ptr()),
            )
        };
        assert_eq!(c_v, r_v, "COM_HashString({s:?})");
    }

    // Raw high-bit bytes (not expressible in a c"" literal, which must be
    // UTF-8). `hash ^= *str++` reads a plain `char`, whose signedness is
    // implementation-defined: signed on x86-64/Apple arm64, unsigned on
    // AArch64 Linux. These vectors are the ones that diverge if the Rust
    // port hardcodes one signedness instead of following c_char.
    for raw in [
        vec![0x80u8],
        vec![0xffu8],
        vec![0xc3, 0xbf],
        vec![b'k', 0x80, b'y'],
        (0x80u8..=0xbf).collect::<Vec<u8>>(),
    ] {
        let s = std::ffi::CString::new(raw.clone()).unwrap();
        // SAFETY: NUL-terminated input, pure functions
        let (c_v, r_v) = unsafe {
            (
                (ctfs::fns(Side::C).hash_string)(s.as_ptr()),
                (ctfs::fns(Side::Rust).hash_string)(s.as_ptr()),
            )
        };
        assert_eq!(c_v, r_v, "COM_HashString(raw {raw:02x?})");
    }

    let blocks: [&[u8]; 6] = [
        b"",
        b"\x00",
        b"abc\x00def",
        b"\xff\xfe\x80\x7f",
        &[0xaa; 1024],
        b"block data with text",
    ];
    for b in blocks {
        // SAFETY: pointer/length from a valid slice, pure functions
        let (c_v, r_v) = unsafe {
            (
                (ctfs::fns(Side::C).hash_block)(b.as_ptr().cast(), b.len()),
                (ctfs::fns(Side::Rust).hash_block)(b.as_ptr().cast(), b.len()),
            )
        };
        assert_eq!(c_v, r_v, "COM_HashBlock({b:?})");
    }
    // NULL with size 0 never dereferences on either side
    // SAFETY: size 0: no bytes are read
    let (c_v, r_v) = unsafe {
        (
            (ctfs::fns(Side::C).hash_block)(core::ptr::null(), 0),
            (ctfs::fns(Side::Rust).hash_block)(core::ptr::null(), 0),
        )
    };
    assert_eq!(c_v, r_v, "COM_HashBlock(NULL, 0)");
}

/// Env-gated golden: the real rerelease QuakeEX.kpf (data_subdir "rerelease",
/// see Misc/harness/corpus.json). Extracts localization/loc_english.txt with
/// quake_fs::zipdir directly, then runs LOC_Init on both engine sides over
/// the same basedir and three-way-compares every key. Skips silently when
/// QUAKE_GAME_DATA is absent.
#[test]
fn loc_rerelease_kpf_golden() {
    let Ok(data) = std::env::var("QUAKE_GAME_DATA") else {
        eprintln!("QUAKE_GAME_DATA not set; skipping the rerelease kpf golden test");
        return;
    };
    let rerelease = std::path::Path::new(&data).join("rerelease");
    let kpf_path = rerelease.join("QuakeEX.kpf");
    if !kpf_path.is_file() {
        eprintln!(
            "{} not found; skipping the rerelease kpf golden test",
            kpf_path.display()
        );
        return;
    }

    let _guard = ctfs::lock();
    ctfs::set_host_dirs("/nonexistent-host-basedir", None);
    ctfs::set_registered(1.0);

    // the pure-Rust reference extraction + parse
    let kpf = std::fs::read(&kpf_path).unwrap();
    let archive = quake_fs::zipdir::ZipArchive::open(&kpf).expect("kpf must open");
    let text = archive
        .extract(b"localization/loc_english.txt")
        .expect("loc_english.txt must extract");
    let parsed = quake_fs::loc::parse(&text, &mut |_| {}).expect("loc must parse");
    assert!(!parsed.entries().is_empty(), "rerelease loc has entries");

    // both engine sides load from the same basedir (kpf or the loose file,
    // identically); a bogus gamedir keeps the searchpath walk a miss
    for side in BOTH {
        ctfs::setup(side, &[&rerelease], 0, c"ctest-none");
    }
    let log = loc_init_compare("rerelease kpf");
    let loaded_line = log
        .iter()
        .find(|l| l.contains("Loaded") && l.contains("strings"))
        .expect("rerelease loc must load");
    assert!(
        loaded_line.contains(&format!("Loaded {} strings", parsed.entries().len())),
        "engine count vs zipdir parse: {loaded_line:?} vs {}",
        parsed.entries().len()
    );

    for entry in parsed.entries() {
        let key = parsed.str_at(entry.key);
        let value = parsed.str_at(entry.value);
        let mut query = Vec::with_capacity(key.len() + 1);
        query.push(b'$');
        query.extend_from_slice(key);
        let query = std::ffi::CString::new(query).unwrap();
        let engine = lookup_compare(&query, "rerelease kpf golden");
        assert_eq!(
            engine.as_deref(),
            Some(value),
            "engine value vs zipdir bytes for key {:?}",
            String::from_utf8_lossy(key)
        );
    }

    for side in BOTH {
        // SAFETY: shutdown/teardown under FS_LOCK
        unsafe { (ctfs::fns(side).loc_shutdown)() };
        ctfs::reset(side);
    }
    eprintln!(
        "rerelease kpf golden: {} keys compared across c_ref miniz / quake_fs zipdir",
        parsed.entries().len()
    );
}
