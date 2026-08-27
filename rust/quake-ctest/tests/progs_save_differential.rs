//! `quake_progs::save` vs `pr_edict_save.c` (Phase 6 M4).
//!
//! `save_diff.py` already byte-compares whole savegames end-to-end, which is
//! the real gate. This suite is the microscope: it drives both writers over
//! synthetic def tables so every `PR_UglyValueString` arm, every skip rule and
//! the manual-alpha fallback are exercised on values a shipping progs never
//! produces — negative zero, subnormals, infinities, the 64-bit types, and
//! strings that are cleared engine handles.

use core::ffi::{c_int, c_void};

use quake_ctest as _;
use quake_progs::arena::VmRaw;
use quake_progs::save::{self, SaveSys};
use quake_types::progs::{etype, DDef, QcVm, DEF_SAVEGLOBAL};

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
        strings: *const i8,
        stringssize: c_int,
    ) -> *mut c_void;
    fn ctest_progs_select_vm(which: c_int);
    fn ctest_try_host(f: unsafe extern "C" fn(*mut c_void), arg: *mut c_void) -> c_int;
    fn ctest_host_error_message() -> *const i8;
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
    fn ctest_progs_capture_ed_write(edict_num: c_int, out: *mut u8, out_max: c_int) -> c_int;
    fn ctest_progs_capture_ed_write_globals(out: *mut u8, out_max: c_int) -> c_int;
    fn c_ref_PR_UglyValueString(ty: c_int, val: *const i32) -> *const i8;
}

static VM_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    VM_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

struct EngineSave;

impl SaveSys for EngineSave {
    fn field_at_ofs(&mut self, ofs: c_int) -> Option<DDef> {
        // the same linear search ED_FieldAtOfs does, over the Rust fixture
        let vm = vm_b();
        (0..vm.numfielddefs())
            .map(|i| vm.fielddef(i))
            .find(|d| c_int::from(d.ofs) == ofs)
    }
}

/// Both fixtures carry the same def tables and the same strings blob, so the
/// two writers see identical inputs.
fn setup(fielddefs: &[DDef], globaldefs: &[DDef], extfields_alpha: c_int) {
    let strings = b"\0alpha\0beta\0some string value\0".to_vec();
    for which in 0..2 {
        // SAFETY: the caller holds VM_LOCK; the slices outlive the copy.
        unsafe {
            ctest_progs_synth_vm(
                which,
                8,
                64,
                128,
                core::ptr::null(),
                0,
                core::ptr::null(),
                0,
                strings.as_ptr().cast::<i8>(),
                strings.len() as c_int,
            );
            ctest_progs_set_defs(
                which,
                fielddefs.as_ptr(),
                fielddefs.len() as c_int,
                globaldefs.as_ptr(),
                globaldefs.len() as c_int,
                extfields_alpha,
            );
        }
    }
    // SAFETY: fixture A becomes the ambient VM the oracle dereferences.
    unsafe { ctest_progs_select_vm(0) };
}

fn vm_b() -> VmRaw {
    // SAFETY: fixture B is live between setup() and teardown().
    unsafe { VmRaw::new(ctest_progs_vm(1).cast::<QcVm>()) }
}

fn vm_a() -> VmRaw {
    // SAFETY: as above.
    unsafe { VmRaw::new(ctest_progs_vm(0).cast::<QcVm>()) }
}

fn teardown() {
    // SAFETY: the caller holds VM_LOCK and no VmRaw outlives this.
    unsafe { ctest_progs_synth_free() };
}

fn def(ty: c_int, ofs: u16, s_name: c_int) -> DDef {
    DDef {
        type_: ty as u16,
        ofs,
        s_name,
    }
}

fn c_ugly(ty: c_int, words: &[i32]) -> String {
    let mut buf = [0i32; 4];
    buf[..words.len().min(4)].copy_from_slice(&words[..words.len().min(4)]);
    // SAFETY: the caller holds VM_LOCK, fixture A is ambient, and buf has the
    // four words the widest arm reads.
    let p = unsafe { c_ref_PR_UglyValueString(ty, buf.as_ptr()) };
    // SAFETY: PR_UglyValueString returns its NUL-terminated static buffer.
    unsafe { std::ffi::CStr::from_ptr(p) }
        .to_string_lossy()
        .into_owned()
}

fn rust_ugly(ty: c_int, words: &[i32]) -> String {
    let mut buf = [0i32; 4];
    buf[..words.len().min(4)].copy_from_slice(&words[..words.len().min(4)]);
    let vm = vm_b();
    let mut sys = EngineSave;
    let bytes = save::ugly_value_string(&vm, &mut sys, ty, &buf).expect("ugly_value_string");
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Every `PR_UglyValueString` arm, including values a shipping progs never
/// produces. This is the ADR-005 float formatter under the microscope: `%f`
/// with the default precision of 6.
#[test]
fn ugly_value_string_matches_every_arm() {
    let _g = lock();
    setup(&[def(etype::EV_FLOAT, 0, 1)], &[], -1);

    let floats: &[f32] = &[
        0.0,
        -0.0,
        1.0,
        -1.0,
        0.1,
        1.0 / 3.0,
        1e-30,
        1e30,
        f32::MIN_POSITIVE,
        f32::from_bits(1), // smallest subnormal
        f32::MAX,
        -f32::MAX,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NAN,
        16777216.0,
        123_456.79,
        -0.000123,
    ];
    for &f in floats {
        let w = [f.to_bits() as i32];
        assert_eq!(
            rust_ugly(etype::EV_FLOAT, &w),
            c_ugly(etype::EV_FLOAT, &w),
            "ev_float {f:?} ({:#010x})",
            f.to_bits()
        );
    }

    for &v in &[0i32, 1, -1, i32::MIN, i32::MAX, 12345] {
        assert_eq!(
            rust_ugly(etype::EV_EXT_INTEGER, &[v]),
            c_ugly(etype::EV_EXT_INTEGER, &[v]),
            "ev_ext_integer {v}"
        );
        assert_eq!(
            rust_ugly(etype::EV_EXT_UINT32, &[v]),
            c_ugly(etype::EV_EXT_UINT32, &[v]),
            "ev_ext_uint32 {v}"
        );
    }

    // the Q_ALIGN(4) 64-bit types, read as two words
    for &v in &[0i64, 1, -1, i64::MIN, i64::MAX, 0x0123_4567_89AB_CDEF] {
        let w = [v as u64 as u32 as i32, ((v as u64) >> 32) as u32 as i32];
        assert_eq!(
            rust_ugly(etype::EV_EXT_SINT64, &w),
            c_ugly(etype::EV_EXT_SINT64, &w),
            "ev_ext_sint64 {v}"
        );
        assert_eq!(
            rust_ugly(etype::EV_EXT_UINT64, &w),
            c_ugly(etype::EV_EXT_UINT64, &w),
            "ev_ext_uint64 {v}"
        );
    }
    for &d in &[0.0f64, -0.0, 1.0 / 3.0, 1e-300, 1e300, f64::NAN] {
        let b = d.to_bits();
        let w = [b as u32 as i32, (b >> 32) as u32 as i32];
        assert_eq!(
            rust_ugly(etype::EV_EXT_DOUBLE, &w),
            c_ugly(etype::EV_EXT_DOUBLE, &w),
            "ev_ext_double {d:?}"
        );
    }

    // vectors, strings, void, and the bad-type fallback
    let v3 = [1.5f32.to_bits() as i32, (-0.0f32).to_bits() as i32, 0i32];
    assert_eq!(
        rust_ugly(etype::EV_VECTOR, &v3),
        c_ugly(etype::EV_VECTOR, &v3),
        "ev_vector"
    );
    for &h in &[0i32, 1, 7, 12] {
        assert_eq!(
            rust_ugly(etype::EV_STRING, &[h]),
            c_ugly(etype::EV_STRING, &[h]),
            "ev_string handle {h}"
        );
    }
    assert_eq!(
        rust_ugly(etype::EV_VOID, &[0]),
        c_ugly(etype::EV_VOID, &[0])
    );
    assert_eq!(rust_ugly(99, &[0]), c_ugly(99, &[0]), "bad type");

    teardown();
}

/// `DEF_SAVEGLOBAL` must be masked off before the type switch, on both sides.
#[test]
fn save_global_bit_is_masked_before_the_type_switch() {
    let _g = lock();
    setup(&[def(etype::EV_FLOAT, 0, 1)], &[], -1);
    let w = [2.5f32.to_bits() as i32];
    let tagged = etype::EV_FLOAT | c_int::from(DEF_SAVEGLOBAL);
    assert_eq!(rust_ugly(tagged, &w), c_ugly(tagged, &w));
    assert_eq!(rust_ugly(tagged, &w), rust_ugly(etype::EV_FLOAT, &w));
    teardown();
}

/// Runs `f` with the C `Host_Error` trap armed, returning the message if it
/// fired. `f` must wrap exactly one C seam call: C frames only under the
/// longjmp.
fn c_host_try(f: &mut dyn FnMut()) -> Option<String> {
    unsafe extern "C" fn trampoline(arg: *mut c_void) {
        // SAFETY: arg is the &mut &mut dyn FnMut passed below, alive for the
        // whole call.
        let f = unsafe { &mut *arg.cast::<&mut dyn FnMut()>() };
        f();
    }
    let mut f = f;
    let arg = (&raw mut f).cast::<c_void>();
    // SAFETY: the trampoline receives the pointer to `f` built just above.
    let hit = unsafe { ctest_try_host(trampoline, arg) };
    if hit == 0 {
        return None;
    }
    // SAFETY: the stub's message buffer is static and NUL-terminated.
    let msg = unsafe { std::ffi::CStr::from_ptr(ctest_host_error_message()) };
    Some(msg.to_string_lossy().into_owned())
}

/// `PR_UglyValueString`'s `ev_entity` arm goes through `NUM_FOR_EDICT`, whose
/// `b < 0 || b >= qcvm->num_edicts` test raises in *release* builds too. Both
/// sides must refuse the same prog offsets rather than emitting a number.
#[test]
fn entity_values_out_of_range_raise_on_both_sides() {
    let _g = lock();
    setup(&[def(etype::EV_FLOAT, 0, 1)], &[], -1);
    let stride = vm_b().edict_stride();
    let num_edicts = vm_b().num_edicts();
    assert!(num_edicts > 0, "fixture must have live edicts");

    for &n in &[-1, num_edicts, num_edicts + 7] {
        let w = [n * stride];

        let mut c_msg = None;
        c_host_try(&mut || {
            c_msg = Some(c_ugly(etype::EV_ENTITY, &w));
        });
        // Armed traps report through the message buffer, not the return value.
        let c_raised = c_msg.is_none();

        let vm = vm_b();
        let mut sys = EngineSave;
        let mut buf = [0i32; 4];
        buf[0] = w[0];
        let rust = save::ugly_value_string(&vm, &mut sys, etype::EV_ENTITY, &buf);

        assert!(c_raised, "C should raise for prog offset {n} edicts");
        assert_eq!(
            rust,
            Err(save::SaveError::BadEdictPointer),
            "Rust should raise for prog offset {n} edicts"
        );
    }
    teardown();
}

/// `ev_entity` writes the edict *number*, derived from the byte offset.
#[test]
fn entity_values_write_the_edict_number() {
    let _g = lock();
    setup(&[def(etype::EV_FLOAT, 0, 1)], &[], -1);
    let stride = vm_b().edict_stride();
    for n in 0..6 {
        let w = [n * stride];
        assert_eq!(
            rust_ugly(etype::EV_ENTITY, &w),
            c_ugly(etype::EV_ENTITY, &w),
            "ev_entity {n}"
        );
    }
    teardown();
}

fn c_ed_write(num: c_int) -> Vec<u8> {
    let mut buf = vec![0u8; 64 * 1024];
    // SAFETY: the caller holds VM_LOCK; buf is large enough for the fixtures.
    let n = unsafe { ctest_progs_capture_ed_write(num, buf.as_mut_ptr(), buf.len() as c_int) };
    assert!(n >= 0, "capture failed");
    buf.truncate(n as usize);
    buf
}

fn rust_ed_write(num: c_int) -> Vec<u8> {
    let vm = vm_b();
    let mut sys = EngineSave;
    let mut out = Vec::new();
    save::ed_write(&vm, &mut sys, num, &mut out).expect("ed_write");
    out
}

/// The full `ED_Write` record, with every skip rule in play.
#[test]
fn ed_write_matches_including_every_skip_rule() {
    let _g = lock();
    // s_name handles: 1 "alpha", 7 "beta", 12 "some string value"
    let fielddefs = vec![
        def(etype::EV_VOID, 0, 0),  // index 0: never written
        def(etype::EV_FLOAT, 1, 7), // normal float
        def(etype::EV_FLOAT | c_int::from(DEF_SAVEGLOBAL), 2, 7), // skipped: SAVEGLOBAL
        def(99, 3, 7),              // skipped: type >= NUM_TYPE_SIZES
        def(etype::EV_VECTOR, 4, 12), // vector
        def(etype::EV_STRING, 8, 12), // string
        def(etype::EV_ENTITY, 9, 1), // entity
    ];
    setup(&fielddefs, &[], -1);

    // seed field words on both fixtures identically
    for mut vm in [vm_a(), vm_b()] {
        let stride = vm.edict_stride();
        let base = 3 * stride;
        vm.set_ed_i32(vm.field_byte_offset(base, 1), 1.25f32.to_bits() as i32);
        vm.set_ed_i32(vm.field_byte_offset(base, 2), 9.0f32.to_bits() as i32);
        vm.set_ed_i32(vm.field_byte_offset(base, 3), 5);
        vm.set_ed_i32(vm.field_byte_offset(base, 4), 1.0f32.to_bits() as i32);
        vm.set_ed_i32(vm.field_byte_offset(base, 5), 0);
        vm.set_ed_i32(vm.field_byte_offset(base, 6), (-2.0f32).to_bits() as i32);
        vm.set_ed_i32(vm.field_byte_offset(base, 8), 7);
        vm.set_ed_i32(vm.field_byte_offset(base, 9), 2 * stride);
    }

    assert_eq!(
        String::from_utf8_lossy(&rust_ed_write(3)),
        String::from_utf8_lossy(&c_ed_write(3)),
        "ED_Write with mixed field types"
    );

    // an all-zero edict writes no field lines at all
    assert_eq!(
        String::from_utf8_lossy(&rust_ed_write(5)),
        String::from_utf8_lossy(&c_ed_write(5)),
        "ED_Write over an untouched edict"
    );
    teardown();
}

/// A freed edict is written as an empty record, short-circuiting everything.
#[test]
fn ed_write_of_a_free_edict_is_an_empty_record() {
    let _g = lock();
    setup(
        &[def(etype::EV_VOID, 0, 0), def(etype::EV_FLOAT, 1, 7)],
        &[],
        -1,
    );
    for mut vm in [vm_a(), vm_b()] {
        let stride = vm.edict_stride();
        vm.set_ed_i32(vm.field_byte_offset(2 * stride, 1), 3.0f32.to_bits() as i32);
    }
    // mark edict 2 free on both sides through the arena
    for which in 0..2 {
        // SAFETY: the caller holds VM_LOCK; the fixture has 8 edicts.
        let mut arena = unsafe {
            quake_progs::arena::EdictArena::borrowed(
                (*ctest_progs_vm(which).cast::<QcVm>()).edicts.cast::<u8>(),
                vm_b().edict_stride() as usize,
                8,
            )
        };
        arena.set_free(quake_progs::arena::EdictId(2), true);
    }
    let rust = rust_ed_write(2);
    assert_eq!(rust, b"{\n}\n", "free edicts write an empty record");
    assert_eq!(rust, c_ed_write(2));
    teardown();
}

/// johnfitz's manual-alpha fallback: written only when the progs has no
/// `alpha` field *and* the entity's alpha is non-default.
#[test]
fn manual_alpha_fallback_matches() {
    for (extfields_alpha, label) in [(-1, "progs has no alpha field"), (4, "progs defines alpha")] {
        for alpha in [0u8, 1, 2, 128, 255] {
            let _g = lock();
            setup(&[def(etype::EV_VOID, 0, 0)], &[], extfields_alpha);
            // set ed->alpha on both fixtures
            for which in 0..2 {
                // SAFETY: the caller holds VM_LOCK; the fixture has 8 edicts.
                let mut arena = unsafe {
                    quake_progs::arena::EdictArena::borrowed(
                        (*ctest_progs_vm(which).cast::<QcVm>()).edicts.cast::<u8>(),
                        vm_b().edict_stride() as usize,
                        8,
                    )
                };
                arena.set_alpha(quake_progs::arena::EdictId(1), alpha);
            }
            assert_eq!(
                String::from_utf8_lossy(&rust_ed_write(1)),
                String::from_utf8_lossy(&c_ed_write(1)),
                "{label}, alpha={alpha}"
            );
            teardown();
        }
    }
}

/// `ED_WriteGlobals` keeps only `DEF_SAVEGLOBAL` defs of a savegame-legal
/// type; everything else is skipped.
#[test]
fn ed_write_globals_matches_including_the_type_filter() {
    let _g = lock();
    let sg = c_int::from(DEF_SAVEGLOBAL);
    let globaldefs = vec![
        def(etype::EV_FLOAT, 30, 7),            // skipped: no SAVEGLOBAL bit
        def(etype::EV_FLOAT | sg, 31, 7),       // kept
        def(etype::EV_STRING | sg, 32, 12),     // kept
        def(etype::EV_ENTITY | sg, 33, 1),      // kept
        def(etype::EV_VECTOR | sg, 34, 12),     // skipped: type filter
        def(etype::EV_FUNCTION | sg, 37, 7),    // skipped: type filter
        def(etype::EV_EXT_INTEGER | sg, 38, 7), // kept
        def(etype::EV_EXT_UINT64 | sg, 39, 7),  // kept
    ];
    setup(&[def(etype::EV_VOID, 0, 0)], &globaldefs, -1);

    for mut vm in [vm_a(), vm_b()] {
        vm.set_g_i32(31, (-0.5f32).to_bits() as i32);
        vm.set_g_i32(32, 7);
        vm.set_g_i32(33, 2 * vm.edict_stride());
        vm.set_g_i32(38, -42);
        vm.set_g_i32(39, -1);
        vm.set_g_i32(40, 0);
    }

    let vm = vm_b();
    let mut sys = EngineSave;
    let mut rust = Vec::new();
    save::ed_write_globals(&vm, &mut sys, &mut rust).expect("ed_write_globals");

    let mut buf = vec![0u8; 64 * 1024];
    // SAFETY: the caller holds VM_LOCK.
    let n = unsafe { ctest_progs_capture_ed_write_globals(buf.as_mut_ptr(), buf.len() as c_int) };
    assert!(n >= 0);
    buf.truncate(n as usize);

    assert_eq!(
        String::from_utf8_lossy(&rust),
        String::from_utf8_lossy(&buf),
        "ED_WriteGlobals"
    );
    teardown();
}

/// COMPAT: an out-of-range string handle resolves to the empty string at the
/// head of the blob rather than raising — `PR_GetString`'s `Host_Error` for
/// that case sits after a `return` and is dead code. A savegame written from
/// such a handle therefore contains an empty value on both sides.
#[test]
fn out_of_range_string_handles_write_empty_on_both_sides() {
    let _g = lock();
    setup(&[def(etype::EV_VOID, 0, 0)], &[], -1);
    // numknownstrings is 0 here, so every negative handle is out of range,
    // as is any offset past the blob
    for h in [-1i32, -50, 10_000] {
        assert_eq!(
            rust_ugly(etype::EV_STRING, &[h]),
            c_ugly(etype::EV_STRING, &[h]),
            "ev_string handle {h}"
        );
        assert_eq!(rust_ugly(etype::EV_STRING, &[h]), "", "handle {h}");
    }
    // and the writer surfaces it as a value, not an error
    let vm = vm_b();
    let mut sys = EngineSave;
    assert_eq!(
        save::ugly_value_string(&vm, &mut sys, etype::EV_STRING, &[-1, 0, 0, 0]),
        Ok(Vec::new())
    );
    teardown();
}

/// COMPAT: `PR_UglyValueString` formats into a `static char line[1024]` with
/// `q_snprintf`, so a value longer than 1023 bytes is **truncated** — and
/// `ED_Write` writes whatever survives. A mod storing a long `strzone`d string
/// in an entity field reaches this; id1 never does, which is why the
/// end-to-end `save_diff.py` gate is blind to it.
///
/// Caught by the M5 compatibility review: the port formatted into an unbounded
/// `Vec`, so the Rust build wrote the full string where C wrote 1023 bytes —
/// a savegame byte divergence and a cross-load divergence.
#[test]
fn long_string_values_are_truncated_at_cs_buffer() {
    let _g = lock();

    // a strings blob holding one very long value, addressed by handle 1
    let mut blob: Vec<u8> = vec![0];
    let long: Vec<u8> = std::iter::repeat_n(b'A', 4000).collect();
    blob.extend_from_slice(&long);
    blob.push(0);

    for which in 0..2 {
        // SAFETY: the caller holds VM_LOCK; the slices outlive the copy.
        unsafe {
            ctest_progs_synth_vm(
                which,
                8,
                64,
                128,
                core::ptr::null(),
                0,
                core::ptr::null(),
                0,
                blob.as_ptr().cast::<i8>(),
                blob.len() as c_int,
            );
            let fielddefs = [def(etype::EV_VOID, 0, 0), def(etype::EV_STRING, 1, 1)];
            ctest_progs_set_defs(which, fielddefs.as_ptr(), 2, core::ptr::null(), 0, -1);
        }
    }
    // SAFETY: fixture A becomes the ambient VM the oracle dereferences.
    unsafe { ctest_progs_select_vm(0) };

    // the value formatter itself truncates
    let r = rust_ugly(etype::EV_STRING, &[1]);
    let c = c_ugly(etype::EV_STRING, &[1]);
    assert_eq!(r.len(), 1023, "C's line[1024] cap, minus the NUL");
    assert_eq!(r, c, "PR_UglyValueString over a 4000-byte string");

    // and so does the record ED_Write produces
    for mut vm in [vm_a(), vm_b()] {
        let stride = vm.edict_stride();
        vm.set_ed_i32(vm.field_byte_offset(stride, 1), 1);
    }
    assert_eq!(
        String::from_utf8_lossy(&rust_ed_write(1)),
        String::from_utf8_lossy(&c_ed_write(1)),
        "ED_Write over a long string field"
    );
    teardown();
}
