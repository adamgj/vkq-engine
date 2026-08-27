//! `quake_progs::arena::StringTable` vs the C originals in `pr_edict_arena.c`
//! (Phase 6 M2).
//!
//! `string_t` handles are QC-visible values that reach savegames, so the
//! *numbers* this table hands out are compat-critical, not just the strings
//! they resolve to. The suite drives both sides through identical call
//! sequences and compares the handles and the resolved bytes, with dedicated
//! coverage for the three preserved quirks:
//!
//! * `PR_GetString`'s invalid-offset arm returns the empty string at the head
//!   of the blob and its `Host_Error` is dead code after a `return`;
//! * `PR_SetEngineString`'s in-blob test is `s <= strings + stringssize - 2`,
//!   two bytes short of the end;
//! * `PR_ClearEdictStrings` only resets `freeknownstrings` outside `_DEBUG`.

use core::ffi::{c_char, c_int, c_void};

use quake_ctest as _;
use quake_progs::arena::{Mem, StringError, StringTable};
use quake_types::progs::{QBoolean, QcVm};

extern "C" {
    fn ctest_progs_reset_vm(max_edicts: c_int, entityfields: c_int) -> *mut c_void;
    fn ctest_progs_set_strings(blob: *mut c_char, size: c_int, progsstrings: c_int);
    fn ctest_try_host(f: extern "C" fn(*mut c_void), arg: *mut c_void) -> c_int;

    fn c_ref_PR_GetString(num: c_int) -> *const c_char;
    fn c_ref_PR_SetEngineString(s: *const c_char) -> c_int;
    fn c_ref_PR_AllocString(size: c_int, ptr: *mut *mut c_char) -> c_int;
    fn c_ref_PR_ClearEngineString(num: c_int);
    fn c_ref_PR_ClearEdictStrings();

    static qcvm: *mut c_void;

    fn Mem_Alloc(size: usize) -> *mut c_void;
    fn Mem_Realloc(ptr: *mut c_void, size: usize) -> *mut c_void;
    fn Mem_Free(ptr: *const c_void);
}

static VM_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    VM_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn c_vm() -> *mut QcVm {
    // SAFETY: the fixture published a real qcvm_t here and progs_abi.rs proves
    // the mirror matches.
    unsafe { qcvm.cast() }
}

/// The engine allocator, exactly as the shipped shims will use it (ADR-013).
struct EngineMem {
    growth_notes: Vec<c_int>,
}

impl Mem for EngineMem {
    fn alloc(&mut self, size: usize) -> *mut u8 {
        // SAFETY: Mem_Alloc returns zeroed memory or aborts.
        unsafe { Mem_Alloc(size).cast() }
    }

    fn realloc(&mut self, ptr: *mut u8, size: usize) -> *mut u8 {
        // SAFETY: `ptr` is null or came from this allocator.
        unsafe { Mem_Realloc(ptr.cast(), size).cast() }
    }

    fn free(&mut self, ptr: *mut u8) {
        // SAFETY: `ptr` is null or came from this allocator; Mem_Free
        // tolerates null, like C's SAFE_FREE.
        unsafe { Mem_Free(ptr.cast()) }
    }

    fn note_slot_growth(&mut self, maxknownstrings: c_int) {
        self.growth_notes.push(maxknownstrings);
    }
}

/// A Rust-side string table with the same shape as the C one. Kept separate
/// from the C fixture so the two run independently and can be compared.
struct RustTable {
    knownstrings: *mut *const c_char,
    knownstringsowned: *mut QBoolean,
    maxknownstrings: c_int,
    numknownstrings: c_int,
    freeknownstrings: c_int,
    progsstrings: c_int,
    strings: *const c_char,
    stringssize: c_int,
    mem: EngineMem,
}

impl RustTable {
    fn new(blob: *const c_char, size: c_int, progsstrings: c_int) -> Self {
        Self {
            knownstrings: core::ptr::null_mut(),
            knownstringsowned: core::ptr::null_mut(),
            maxknownstrings: 0,
            numknownstrings: 0,
            freeknownstrings: 0,
            progsstrings,
            strings: blob,
            stringssize: size,
            mem: EngineMem {
                growth_notes: Vec::new(),
            },
        }
    }

    /// Splits `&mut self` into the table view and the allocator, which the
    /// StringTable API takes separately.
    fn parts(&mut self) -> (StringTable<'_>, &mut EngineMem) {
        let RustTable {
            knownstrings,
            knownstringsowned,
            maxknownstrings,
            numknownstrings,
            freeknownstrings,
            progsstrings,
            strings,
            stringssize,
            mem,
        } = self;
        (
            // SAFETY: every pointer addresses a field of `self`, which
            // outlives the returned table.
            unsafe {
                StringTable::from_parts(
                    *strings,
                    *stringssize,
                    knownstrings,
                    knownstringsowned,
                    maxknownstrings,
                    numknownstrings,
                    *progsstrings,
                    freeknownstrings,
                )
            },
            mem,
        )
    }

    fn set(&mut self, s: *const c_char) -> c_int {
        let (mut t, m) = self.parts();
        t.set_engine_string(s, m)
    }

    fn alloc(&mut self, size: c_int) -> (c_int, *mut c_char) {
        let (mut t, m) = self.parts();
        t.alloc_string(size, m)
    }

    fn clear(&mut self, num: c_int) {
        let (mut t, m) = self.parts();
        t.clear_engine_string(num, m);
    }

    fn clear_edict_strings(&mut self) {
        let (mut t, m) = self.parts();
        t.clear_edict_strings(m);
    }

    fn get(&mut self, num: c_int) -> Result<*const c_char, StringError> {
        let (t, _) = self.parts();
        t.get(num)
    }

    /// The bookkeeping counters C keeps in `qcvm_t`, for direct comparison.
    fn counters(&self) -> (c_int, c_int, c_int) {
        (
            self.maxknownstrings,
            self.numknownstrings,
            self.freeknownstrings,
        )
    }
}

fn c_counters() -> (c_int, c_int, c_int) {
    // SAFETY: see c_vm(); the caller holds VM_LOCK.
    unsafe {
        (
            (*c_vm()).maxknownstrings,
            (*c_vm()).numknownstrings,
            (*c_vm()).freeknownstrings,
        )
    }
}

/// A leaked progs string blob: `\0` then a few NUL-terminated strings, the
/// shape `PR_LoadProgs` produces.
fn make_blob() -> (*mut c_char, c_int) {
    let mut bytes: Vec<u8> = Vec::new();
    bytes.push(0); // the empty string PR_GetString's invalid arm returns
    for s in ["alpha", "beta", "gamma"] {
        bytes.extend_from_slice(s.as_bytes());
        bytes.push(0);
    }
    let size = bytes.len() as c_int;
    let leaked = Box::leak(bytes.into_boxed_slice());
    (leaked.as_mut_ptr().cast::<c_char>(), size)
}

fn cstr(p: *const c_char) -> String {
    // SAFETY: every pointer compared here is NUL-terminated by construction.
    unsafe { std::ffi::CStr::from_ptr(p) }
        .to_string_lossy()
        .into_owned()
}

fn setup(progsstrings: c_int) -> (RustTable, *mut c_char, c_int) {
    // SAFETY: the caller holds VM_LOCK.
    unsafe { ctest_progs_reset_vm(8, 105) };
    let (blob, size) = make_blob();
    // SAFETY: as above; the blob is leaked, so it outlives the fixture.
    unsafe { ctest_progs_set_strings(blob, size, progsstrings) };
    (
        RustTable::new(blob.cast_const(), size, progsstrings),
        blob,
        size,
    )
}

/// A leaked C string the engine could hand to `PR_SetEngineString`.
fn leak_cstr(s: &str) -> *const c_char {
    let boxed = std::ffi::CString::new(s).unwrap().into_boxed_c_str();
    Box::leak(boxed).as_ptr()
}

#[test]
fn positive_handles_index_the_blob_on_both_sides() {
    let _g = lock();
    let (mut rt, _blob, size) = setup(0);
    for num in [0, 1, 6, size - 1] {
        let r = rt.get(num).expect("in-blob handle");
        // SAFETY: the caller holds VM_LOCK.
        let c = unsafe { c_ref_PR_GetString(num) };
        assert_eq!(cstr(r), cstr(c), "PR_GetString({num})");
    }
}

/// COMPAT: out-of-range handles silently resolve to the empty string at the
/// head of the blob — the `Host_Error` in that arm sits after a `return` and
/// is unreachable. If C ever started raising, this test would catch it.
#[test]
fn invalid_handles_return_the_empty_string_and_never_raise() {
    let _g = lock();
    let (mut rt, _blob, size) = setup(0);
    for num in [size, size + 1, 100_000, -1, -50] {
        let r = rt.get(num).expect("invalid handles do not error");
        // SAFETY: the caller holds VM_LOCK.
        let c = unsafe { c_ref_PR_GetString(num) };
        assert_eq!(cstr(r), "", "PR_GetString({num}) should be empty");
        assert_eq!(cstr(r), cstr(c), "PR_GetString({num})");
    }
}

/// The one *live* error: a negative handle whose slot exists but is null.
#[test]
fn a_cleared_negative_handle_is_the_one_real_error() {
    let _g = lock();
    let (mut rt, _blob, _size) = setup(0);
    let s = leak_cstr("engine-owned");

    let rh = rt.set(s);
    // SAFETY: the caller holds VM_LOCK.
    let ch = unsafe { c_ref_PR_SetEngineString(s) };
    assert_eq!(rh, ch);

    rt.clear(rh);
    // SAFETY: as above.
    unsafe { c_ref_PR_ClearEngineString(ch) };

    assert_eq!(rt.get(rh), Err(StringError::NonExistent(rh)));

    extern "C" fn get_cleared(arg: *mut c_void) {
        // SAFETY: run under the armed Host_Error trap.
        unsafe { c_ref_PR_GetString(arg as c_int) };
    }
    // SAFETY: the trap is armed for exactly this call.
    let raised = unsafe { ctest_try_host(get_cleared, ch as *mut c_void) };
    assert_eq!(raised, 1, "C must Host_Error on a cleared engine string");
}

/// COMPAT: the in-blob shortcut is `s <= strings + stringssize - 2`, so a
/// pointer to the blob's *last* byte is NOT treated as an in-blob string and
/// gets a negative engine handle instead.
#[test]
fn set_engine_string_in_blob_test_is_off_by_two() {
    let _g = lock();
    let (mut rt, blob, size) = setup(0);

    for ofs in [0, 1, size - 3, size - 2, size - 1] {
        // SAFETY: 0 <= ofs < size, inside the blob.
        let p = unsafe { blob.add(ofs as usize).cast_const() };
        let r = rt.set(p);
        // SAFETY: the caller holds VM_LOCK.
        let c = unsafe { c_ref_PR_SetEngineString(p) };
        assert_eq!(r, c, "PR_SetEngineString(blob+{ofs}) handle");
        assert_eq!(rt.counters(), c_counters(), "counters after blob+{ofs}");
    }

    // the boundary itself: offsets up to size-2 are positive handles, and the
    // last byte falls through to the knownstrings path
    // SAFETY: inside the blob.
    let last = unsafe { blob.add(size as usize - 1).cast_const() };
    assert!(
        rt.set(last) < 0,
        "the final byte must not take the in-blob shortcut"
    );
}

#[test]
fn engine_string_identity_is_by_pointer_not_by_content() {
    let _g = lock();
    let (mut rt, _blob, _size) = setup(0);
    let a = leak_cstr("same text");
    let b = leak_cstr("same text");

    let ra = rt.set(a);
    let rb = rt.set(b);
    let ra2 = rt.set(a);
    // SAFETY: the caller holds VM_LOCK.
    let (ca, cb, ca2) = unsafe {
        (
            c_ref_PR_SetEngineString(a),
            c_ref_PR_SetEngineString(b),
            c_ref_PR_SetEngineString(a),
        )
    };
    assert_eq!((ra, rb, ra2), (ca, cb, ca2));
    assert_ne!(
        ra, rb,
        "equal content, different pointers -> different handles"
    );
    assert_eq!(
        ra, ra2,
        "the same pointer must resolve to its existing slot"
    );
    assert_eq!(rt.counters(), c_counters());
}

/// Null is handle 0 on both sides, and must not consume a slot.
#[test]
fn null_maps_to_handle_zero() {
    let _g = lock();
    let (mut rt, _blob, _size) = setup(0);
    let before = rt.counters();
    assert_eq!(rt.set(core::ptr::null()), 0);
    // SAFETY: the caller holds VM_LOCK.
    assert_eq!(unsafe { c_ref_PR_SetEngineString(core::ptr::null()) }, 0);
    assert_eq!(rt.counters(), before);
    assert_eq!(rt.counters(), c_counters());
}

/// `PR_AllocString(0, ...)` returns handle 0 and allocates nothing.
#[test]
fn zero_length_alloc_string_returns_zero() {
    let _g = lock();
    let (mut rt, _blob, _size) = setup(0);
    let (h, p) = rt.alloc(0);
    assert_eq!(h, 0);
    assert!(p.is_null());
    let mut cp: *mut c_char = core::ptr::null_mut();
    // SAFETY: the caller holds VM_LOCK.
    assert_eq!(unsafe { c_ref_PR_AllocString(0, &mut cp) }, 0);
    assert_eq!(rt.counters(), c_counters());
}

/// Slot reuse: clearing a handle lowers `freeknownstrings`, and the next
/// allocation must land in exactly the slot C picks — including growing past
/// PR_STRING_ALLOCSLOTS (256).
#[test]
fn slot_allocation_and_reuse_order_matches_through_a_growth() {
    let _g = lock();
    let (mut rt, _blob, _size) = setup(0);

    let mut handles = Vec::new();
    for i in 0..300 {
        let s = leak_cstr(&format!("s{i}"));
        let r = rt.set(s);
        // SAFETY: the caller holds VM_LOCK.
        let c = unsafe { c_ref_PR_SetEngineString(s) };
        assert_eq!(r, c, "handle {i}");
        assert_eq!(rt.counters(), c_counters(), "counters at {i}");
        handles.push(r);
    }
    // the growth happened, and both sides grew the same way
    assert!(rt.mem.growth_notes.len() >= 2, "{:?}", rt.mem.growth_notes);
    assert_eq!(rt.maxknownstrings, c_counters().0);

    // free a scattered set, then refill and check the reuse order
    for &h in &[handles[200], handles[7], handles[150], handles[6]] {
        rt.clear(h);
        // SAFETY: the caller holds VM_LOCK.
        unsafe { c_ref_PR_ClearEngineString(h) };
        assert_eq!(rt.counters(), c_counters(), "counters after clearing {h}");
    }
    for i in 0..6 {
        let s = leak_cstr(&format!("refill{i}"));
        let r = rt.set(s);
        // SAFETY: the caller holds VM_LOCK.
        let c = unsafe { c_ref_PR_SetEngineString(s) };
        assert_eq!(r, c, "refill handle {i}");
        assert_eq!(rt.counters(), c_counters(), "counters at refill {i}");
    }
}

/// `PR_AllocString` marks the slot owned, so `PR_ClearEdictStrings` frees it;
/// `PR_SetEngineString` slots are borrowed and only nulled. Slots below
/// `progsstrings` are untouched entirely.
#[test]
fn clear_edict_strings_respects_ownership_and_progsstrings() {
    let _g = lock();
    let progsstrings = 3;
    let (mut rt, _blob, _size) = setup(progsstrings);

    // three "progs-era" slots, then a mix of owned and borrowed
    for i in 0..progsstrings {
        let s = leak_cstr(&format!("progs{i}"));
        assert_eq!(
            rt.set(s),
            // SAFETY: the caller holds VM_LOCK.
            unsafe { c_ref_PR_SetEngineString(s) }
        );
    }
    for i in 0..5 {
        if i % 2 == 0 {
            let (rh, rp) = rt.alloc(16);
            let mut cp: *mut c_char = core::ptr::null_mut();
            // SAFETY: the caller holds VM_LOCK.
            let ch = unsafe { c_ref_PR_AllocString(16, &mut cp) };
            assert_eq!(rh, ch, "alloc_string handle {i}");
            assert!(!rp.is_null() && !cp.is_null());
        } else {
            let s = leak_cstr(&format!("borrowed{i}"));
            // SAFETY: the caller holds VM_LOCK.
            assert_eq!(rt.set(s), unsafe { c_ref_PR_SetEngineString(s) });
        }
    }
    assert_eq!(rt.counters(), c_counters(), "before clear");

    rt.clear_edict_strings();
    // SAFETY: the caller holds VM_LOCK.
    unsafe { c_ref_PR_ClearEdictStrings() };
    assert_eq!(rt.counters(), c_counters(), "after clear");

    // the progs-era slots survive; the owned ones above them are gone
    for h in [-1, -2, -3] {
        assert!(rt.get(h).is_ok(), "progs slot {h} must survive");
    }

    // and the next allocation lands where C puts it
    let s = leak_cstr("after-clear");
    // SAFETY: the caller holds VM_LOCK.
    assert_eq!(rt.set(s), unsafe { c_ref_PR_SetEngineString(s) });
    assert_eq!(rt.counters(), c_counters(), "after refill");
}
