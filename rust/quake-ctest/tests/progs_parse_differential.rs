//! `quake_progs::parse` vs `pr_edict_parse.c` (Phase 6 M5).
//!
//! The read side of ADR-019's gate 2. `save_diff.py` proves a whole savegame
//! survives the round trip; this suite drives `ED_ParseEpair` and
//! `ED_NewString` directly, over the literals a savegame can actually contain
//! — including the ones a well-behaved progs never writes.
//!
//! The numeric conversions are deliberately *not* reimplemented in Rust
//! (ADR-010): both sides call the same platform `atof`/`atoi`/`strtoll`/
//! `strtoull`, so what is under test is the dispatch, the truncation rules and
//! the entity-allocation side effects.

use core::ffi::{c_char, c_int, c_void, CStr};

use quake_ctest as _;
use quake_progs::alloc::AllocError;
use quake_progs::arena::{EdictArena, EdictId, Mem, VmRaw};
use quake_progs::parse::{self, ParseError, ParseSys};
use quake_types::progs::{etype, DDef, FreeList, QcVm, MAX_EDICTS};

extern "C" {
    fn ctest_progs_synth_vm(
        which: c_int,
        max_edicts: c_int,
        entityfields: c_int,
        numglobals: c_int,
        stmts: *const c_void,
        nstmts: c_int,
        funcs: *const c_void,
        nfuncs: c_int,
        strings: *const c_char,
        stringssize: c_int,
    ) -> *mut c_void;
    fn ctest_progs_select_vm(which: c_int);
    fn ctest_progs_vm(which: c_int) -> *mut c_void;
    fn ctest_progs_synth_free();
    fn ctest_progs_set_defs(
        which: c_int,
        fielddefs: *const DDef,
        numfielddefs: c_int,
        globaldefs: *const DDef,
        numglobaldefs: c_int,
        extfields_alpha: c_int,
    );
    fn c_ref_ED_ParseEpair(
        base: *mut c_void,
        key: *const DDef,
        s: *const c_char,
        zoned: bool,
    ) -> bool;
    fn c_ref_ED_NewString(s: *const c_char) -> c_int;

    fn Mem_Alloc(size: usize) -> *mut c_void;
    fn Mem_Realloc(ptr: *mut c_void, size: usize) -> *mut c_void;
    fn Mem_Free(ptr: *const c_void);
    fn atof(s: *const c_char) -> f64;
    fn atoi(s: *const c_char) -> c_int;
    fn strtoll(s: *const c_char, end: *mut *mut c_char, base: c_int) -> i64;
    fn strtoull(s: *const c_char, end: *mut *mut c_char, base: c_int) -> u64;
}

static VM_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    VM_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

const MAXE: c_int = 16;
/// At least `sizeof(entvars_t)/4` (105): `ED_Free`, which the `ev_entity` arm
/// reaches, resets fields as far out as `nextthink`. A real progs always
/// defines at least the engine block, so C never notices; a smaller fixture
/// would have C writing into the next edict while the arena's bounds check
/// catches it.
const ENTFIELDS: c_int = 128;
const NUMGLOBALS: c_int = 128;

/// Drives the Rust parser against fixture B, using the same libc entry points
/// the engine shim uses.
struct TestParse {
    unlinked: Vec<EdictId>,
    prints: Vec<String>,
}

impl Mem for TestParse {
    fn alloc(&mut self, size: usize) -> *mut u8 {
        // SAFETY: the ctest allocator, same as the engine's.
        unsafe { Mem_Alloc(size).cast() }
    }
    fn realloc(&mut self, ptr: *mut u8, size: usize) -> *mut u8 {
        // SAFETY: as above.
        unsafe { Mem_Realloc(ptr.cast(), size).cast() }
    }
    fn free(&mut self, ptr: *mut u8) {
        // SAFETY: as above; Mem_Free tolerates null.
        unsafe { Mem_Free(ptr.cast()) }
    }
    fn note_slot_growth(&mut self, _n: c_int) {}
}

impl ParseSys for TestParse {
    fn atof(&mut self, s: &CStr) -> f64 {
        // SAFETY: a leaf libc call on a NUL-terminated string.
        unsafe { atof(s.as_ptr()) }
    }
    fn atoi(&mut self, s: &CStr) -> c_int {
        // SAFETY: as above.
        unsafe { atoi(s.as_ptr()) }
    }
    fn strtoll(&mut self, s: &CStr) -> i64 {
        // SAFETY: as above.
        unsafe { strtoll(s.as_ptr(), core::ptr::null_mut(), 0) }
    }
    fn strtoull(&mut self, s: &CStr) -> u64 {
        // SAFETY: as above.
        unsafe { strtoull(s.as_ptr(), core::ptr::null_mut(), 0) }
    }
    fn find_field_ofs(&mut self, name: &CStr) -> Option<c_int> {
        let vm = vm_b();
        (0..vm.numfielddefs())
            .map(|i| vm.fielddef(i))
            .find(|d| vm.get_string_bytes(d.s_name).ok() == Some(name.to_bytes()))
            .map(|d| c_int::from(d.ofs))
    }
    fn find_function(&mut self, name: &CStr) -> Option<c_int> {
        let vm = vm_b();
        (0..vm.numfunctions()).find(|&i| {
            vm.function_name_handle(i)
                .and_then(|h| vm.get_string_bytes(h).ok())
                == Some(name.to_bytes())
        })
    }
    fn unlink_edict(&mut self, id: EdictId) {
        self.unlinked.push(id);
    }
    fn dprint(&mut self, msg: &str) {
        self.prints.push(msg.to_owned());
    }
    fn print(&mut self, msg: &str) {
        self.prints.push(msg.to_owned());
    }
    fn dwarn(&mut self, msg: &str) {
        self.prints.push(msg.to_owned());
    }
}

fn vm_b() -> VmRaw {
    // SAFETY: fixture B is live between setup() and teardown().
    unsafe { VmRaw::new(ctest_progs_vm(1).cast::<QcVm>()) }
}

fn arena_b() -> EdictArena {
    let vm = vm_b();
    // SAFETY: the fixture allocated max_edicts * edict_size bytes.
    unsafe {
        EdictArena::borrowed(
            vm.edicts_base(),
            vm.edict_size_for_test() as usize,
            MAXE as usize,
        )
    }
}

/// Both fixtures get the same defs and the same strings blob.
fn setup(fielddefs: &[DDef]) {
    let strings = b"\0classname\0targetname\0think\0\0".to_vec();
    for which in 0..2 {
        // SAFETY: the caller holds VM_LOCK; the slices outlive the copy.
        unsafe {
            ctest_progs_synth_vm(
                which,
                MAXE,
                ENTFIELDS,
                NUMGLOBALS,
                core::ptr::null(),
                0,
                core::ptr::null(),
                0,
                strings.as_ptr().cast::<c_char>(),
                strings.len() as c_int,
            );
            ctest_progs_set_defs(
                which,
                fielddefs.as_ptr(),
                fielddefs.len() as c_int,
                core::ptr::null(),
                0,
                -1,
            );
        }
    }
    // SAFETY: fixture A becomes the ambient VM the oracle dereferences.
    unsafe { ctest_progs_select_vm(0) };
}

fn teardown() {
    // SAFETY: the caller holds VM_LOCK and no view outlives this.
    unsafe { ctest_progs_synth_free() };
}

fn def(ty: c_int, ofs: u16, s_name: c_int) -> DDef {
    DDef {
        type_: ty as u16,
        ofs,
        s_name,
    }
}

fn new_free_list() -> Box<FreeList> {
    Box::new(FreeList {
        size: 0,
        head_index: 0,
        circular_buffer: [0u16; MAX_EDICTS],
    })
}

/// Runs one `ED_ParseEpair` on both sides over each fixture's globals block,
/// and compares the words written plus the return value.
fn epair_both(key: DDef, s: &str, zoned: bool) -> (bool, Vec<i32>, bool, Vec<i32>) {
    let cs = std::ffi::CString::new(s).unwrap();
    let words = 3usize;

    // --- C, over fixture A's globals ---
    // SAFETY: the caller holds VM_LOCK; fixture A is ambient.
    let c_ret = unsafe {
        ctest_progs_select_vm(0);
        let base = (*ctest_progs_vm(0).cast::<QcVm>()).globals.cast::<c_void>();
        c_ref_ED_ParseEpair(base, &key, cs.as_ptr(), zoned)
    };
    // SAFETY: reading back the words C wrote.
    let c_words: Vec<i32> = unsafe {
        let g = (*ctest_progs_vm(0).cast::<QcVm>()).globals.cast::<i32>();
        (0..words)
            .map(|i| g.add(usize::from(key.ofs) + i).read())
            .collect()
    };

    // --- Rust, over fixture B's globals ---
    let mut vm = vm_b();
    let mut arena = arena_b();
    let mut free_list = new_free_list();
    let mut sys = TestParse {
        unlinked: Vec::new(),
        prints: Vec::new(),
    };
    // SAFETY: fixture B's globals block has NUMGLOBALS words.
    let dest = unsafe {
        core::slice::from_raw_parts_mut(
            (*ctest_progs_vm(1).cast::<QcVm>())
                .globals
                .cast::<i32>()
                .add(usize::from(key.ofs)),
            words,
        )
    };
    let r_ret = parse::ed_parse_epair(
        &mut vm,
        &mut arena,
        &mut free_list,
        &mut sys,
        dest,
        c_int::from(key.type_),
        key.s_name,
        &cs,
        zoned,
    )
    .expect("ed_parse_epair");
    let r_words = dest.to_vec();

    (c_ret, c_words, r_ret, r_words)
}

fn assert_epair(key: DDef, s: &str, label: &str) {
    let (c_ret, c_words, r_ret, r_words) = epair_both(key, s, false);
    assert_eq!(r_ret, c_ret, "{label}: return value for {s:?}");
    assert_eq!(r_words, c_words, "{label}: written words for {s:?}");
}

/// Every scalar arm, over the literals a savegame can hold.
#[test]
fn scalar_arms_match() {
    let _g = lock();
    setup(&[def(etype::EV_VOID, 0, 0)]);

    let floats = [
        "0",
        "-0",
        "1",
        "-1",
        "0.1",
        "3.14159265358979",
        "1e30",
        "1e-30",
        "1e40",
        "-1e40",
        "0.30000001192092896",
        "  7.5",
        "7.5abc",
        "abc",
        "",
        "inf",
        "-inf",
        "nan",
        "16777217",
        "0.000000059604645",
    ];
    for s in floats {
        assert_epair(def(etype::EV_FLOAT, 20, 1), s, "ev_float");
        assert_epair(def(etype::EV_EXT_DOUBLE, 24, 1), s, "ev_ext_double");
    }

    let ints = [
        "0",
        "-1",
        "1",
        "2147483647",
        "-2147483648",
        "4294967295",
        "0x10",
        "010",
        "abc",
        "",
        "  42  ",
        "42abc",
        "99999999999999",
    ];
    for s in ints {
        assert_epair(def(etype::EV_EXT_INTEGER, 28, 1), s, "ev_ext_integer");
        // COMPAT: C's ev_ext_uint32 arm uses atoi, not an unsigned parse
        assert_epair(def(etype::EV_EXT_UINT32, 30, 1), s, "ev_ext_uint32");
        // base 0, so 0x/0 prefixes are honoured -- unlike the atoi arms
        assert_epair(def(etype::EV_EXT_SINT64, 32, 1), s, "ev_ext_sint64");
        assert_epair(def(etype::EV_EXT_UINT64, 36, 1), s, "ev_ext_uint64");
    }
    teardown();
}

/// The vector arm's space splitting, its 128-byte truncation, and the
/// short-literal zero fill.
#[test]
fn vector_arm_matches_including_short_and_overlong_literals() {
    let _g = lock();
    setup(&[def(etype::EV_VOID, 0, 0)]);

    let long = format!("{} 2 3", "1".repeat(200));
    let cases: Vec<&str> = vec![
        "1 2 3",
        "-1.5 0 2.25",
        "1 2",
        "1",
        "",
        "   ",
        "1  2  3",
        " 1 2 3",
        "1 2 3 4 5",
        "1e30 -1e30 0",
        &long,
    ];
    for s in cases {
        assert_epair(def(etype::EV_VECTOR, 40, 1), s, "ev_vector");
    }
    teardown();
}

/// `ev_field` and `ev_function` resolve by name and return false when the name
/// is unknown; the `sky`/`fog` names are suppressed silently.
#[test]
fn field_and_function_arms_match() {
    let _g = lock();
    setup(&[
        def(etype::EV_VOID, 0, 0),
        def(etype::EV_FLOAT, 5, 1),   // "classname"
        def(etype::EV_STRING, 6, 11), // "targetname"
    ]);
    for s in [
        "classname",
        "targetname",
        "nosuchfield",
        "sky",
        "skyfoo",
        "fog",
    ] {
        assert_epair(def(etype::EV_FIELD, 50, 1), s, "ev_field");
    }
    // no functions in the fixture, so every lookup fails
    for s in ["main", ""] {
        assert_epair(def(etype::EV_FUNCTION, 52, 1), s, "ev_function");
    }
    teardown();
}

/// `ev_string` allocates an engine string; the escape handling is
/// `ED_NewString`'s, compared directly.
#[test]
fn new_string_escape_handling_matches() {
    let _g = lock();
    setup(&[def(etype::EV_VOID, 0, 0)]);

    let cases = [
        "plain",
        "",
        "a\\nb",
        "a\\tb",
        "a\\\\b",
        "trailing\\",
        "\\n",
        "\\",
        "\\\\",
        "mixed \\n and \\x and \\\\ end",
    ];
    for s in cases {
        let cs = std::ffi::CString::new(s).unwrap();

        // SAFETY: the caller holds VM_LOCK; fixture A is ambient.
        let c_handle = unsafe {
            ctest_progs_select_vm(0);
            c_ref_ED_NewString(cs.as_ptr())
        };
        // SAFETY: reading back the string C allocated.
        let c_text = unsafe {
            let vm = VmRaw::new(ctest_progs_vm(0).cast::<QcVm>());
            vm.get_string_bytes(c_handle).unwrap().to_vec()
        };

        let mut vm = vm_b();
        let mut sys = TestParse {
            unlinked: Vec::new(),
            prints: Vec::new(),
        };
        let r_handle = parse::ed_new_string(&mut vm, &mut sys, &cs);
        let r_text = vm.get_string_bytes(r_handle).unwrap().to_vec();

        assert_eq!(r_handle, c_handle, "ED_NewString handle for {s:?}");
        assert_eq!(
            String::from_utf8_lossy(&r_text),
            String::from_utf8_lossy(&c_text),
            "ED_NewString text for {s:?}"
        );
    }
    teardown();
}

/// The `ev_entity` arm is the one with side effects: it extends
/// `num_edicts`, frees every edict it skipped over, and un-frees the target.
#[test]
fn entity_arm_extends_num_edicts_and_frees_the_gap() {
    let _g = lock();
    setup(&[def(etype::EV_VOID, 0, 0)]);

    // both fixtures start with num_edicts = max_edicts (the synth default);
    // wind them back so the extension path is exercised
    // SAFETY: the caller holds VM_LOCK.
    unsafe {
        (*ctest_progs_vm(0).cast::<QcVm>()).num_edicts = 2;
        (*ctest_progs_vm(1).cast::<QcVm>()).num_edicts = 2;
    }

    let key = def(etype::EV_ENTITY, 60, 1);
    let (c_ret, c_words, r_ret, r_words) = epair_both(key, "5", false);
    assert_eq!(r_ret, c_ret);
    assert_eq!(r_words, c_words, "the prog offset written");

    // SAFETY: as above.
    let (c_num, r_num) = unsafe {
        (
            (*ctest_progs_vm(0).cast::<QcVm>()).num_edicts,
            (*ctest_progs_vm(1).cast::<QcVm>()).num_edicts,
        )
    };
    assert_eq!(r_num, c_num, "num_edicts after the extension");
    assert_eq!(r_num, 6, "loaded_ent_num + 1");

    // edicts 2..5 were freed, edict 5 was taken
    let arena = arena_b();
    for n in 2..5 {
        assert!(arena.free(EdictId(n)), "edict {n} should be free");
    }
    assert!(!arena.free(EdictId(5)), "edict 5 should be allocated");
    teardown();
}

/// COMPAT: `etos` writes `entity N`, so the arm strips that prefix.
#[test]
fn entity_arm_strips_the_etos_prefix() {
    let _g = lock();
    setup(&[def(etype::EV_VOID, 0, 0)]);
    // SAFETY: the caller holds VM_LOCK.
    unsafe {
        (*ctest_progs_vm(0).cast::<QcVm>()).num_edicts = 8;
        (*ctest_progs_vm(1).cast::<QcVm>()).num_edicts = 8;
    }
    let key = def(etype::EV_ENTITY, 62, 1);
    for s in ["3", "entity 3", "entity3", " entity 3"] {
        let (c_ret, c_words, r_ret, r_words) = epair_both(key, s, false);
        assert_eq!(r_ret, c_ret, "{s:?}");
        assert_eq!(r_words, c_words, "{s:?}");
    }
    teardown();
}

/// An entity number at or past `max_edicts` is a `Host_Error` in C; the port
/// returns the condition so the raise happens in the C frame (ADR-009).
#[test]
fn entity_number_past_max_edicts_is_reported() {
    let _g = lock();
    setup(&[def(etype::EV_VOID, 0, 0)]);

    let mut vm = vm_b();
    let mut arena = arena_b();
    let mut free_list = new_free_list();
    let mut sys = TestParse {
        unlinked: Vec::new(),
        prints: Vec::new(),
    };
    let mut dest = [0i32; 3];
    let cs = std::ffi::CString::new(MAXE.to_string()).unwrap();
    let r = parse::ed_parse_epair(
        &mut vm,
        &mut arena,
        &mut free_list,
        &mut sys,
        &mut dest,
        etype::EV_ENTITY,
        1,
        &cs,
        false,
    );
    assert_eq!(
        r,
        Err(ParseError::EntityTooLarge {
            num: MAXE,
            max_edicts: MAXE
        })
    );
    let _ = AllocError::NoFreeEdicts { max_edicts: 0 };
    teardown();
}

/// An unhandled type is C's `default: break` — nothing written, `true`
/// returned.
#[test]
fn unhandled_types_write_nothing_and_return_true() {
    let _g = lock();
    setup(&[def(etype::EV_VOID, 0, 0)]);
    for ty in [etype::EV_VOID, etype::EV_POINTER, 99] {
        let (c_ret, c_words, r_ret, r_words) = epair_both(def(ty, 70, 1), "12345", false);
        assert!(c_ret && r_ret, "type {ty} should return true");
        assert_eq!(r_words, c_words, "type {ty} should write nothing");
    }
    teardown();
}
