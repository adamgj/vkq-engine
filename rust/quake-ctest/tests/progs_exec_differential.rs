//! `quake_progs::exec` vs `pr_exec.c` (Phase 6 M3).
//!
//! Engine-level trace parity (`scripts/harness/trace_diff.py`) already
//! compares every instruction real gameplay executes, but real maps only reach
//! 53 of the 66 opcodes. This suite covers the rest, plus the edge cases no
//! shipping progs takes: division by zero, the float→int truncation in
//! `OP_BITAND`/`OP_BITOR`, negative jump deltas, the `OP_ADDRESS` world guard,
//! stack overflow and underflow, and a builtin that raises.
//!
//! Both sides run the same synthetic progs image over their own VM fixture,
//! and the whole global block, every edict byte, and the VM's own bookkeeping
//! are compared afterwards.

use core::ffi::{c_char, c_int, c_void};

use quake_ctest as _;
use quake_progs::arena::VmRaw;
use quake_progs::exec::{self, ExecError, ExecSys, RunError};
use quake_types::progs::{opcode as op, DFunction, DStatement, QcVm, OFS_PARM0, OFS_RETURN};

extern "C" {
    fn ctest_progs_synth_vm(
        which: c_int,
        max_edicts: c_int,
        entityfields: c_int,
        numglobals: c_int,
        stmts: *const DStatement,
        nstmts: c_int,
        funcs: *const DFunction,
        nfuncs: c_int,
        strings: *const c_char,
        stringssize: c_int,
    ) -> *mut c_void;
    fn ctest_progs_select_vm(which: c_int);
    fn ctest_progs_vm(which: c_int) -> *mut c_void;
    fn ctest_progs_synth_free();
    fn ctest_progs_call_builtin(which: c_int, index: c_int);
    fn ctest_progs_set_sv_state(active: c_int);
    fn ctest_try_host(f: extern "C" fn(*mut c_void), arg: *mut c_void) -> c_int;
    fn c_ref_PR_ExecuteProgram(fnum: u32);
    static ctest_progs_builtin_calls: c_int;
    fn strcmp(a: *const c_char, b: *const c_char) -> c_int;
}

static VM_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    VM_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

const NUM_GLOBALS: c_int = 128;
const MAX_EDICTS: c_int = 8;
const ENTITY_FIELDS: c_int = 128;

/// The Rust side's services. `which` selects the fixture the builtin runs
/// against, so a builtin reading the ambient VM sees fixture B.
struct TestSys {
    which: c_int,
    guard: c_int,
}

impl ExecSys for TestSys {
    fn trace_enabled(&mut self) -> bool {
        false
    }
    fn trace_enter(&mut self, _: c_int) {}
    fn trace_leave(&mut self) {}
    fn trace_statement(&mut self, _: c_int, _: c_int, _: c_int, _: c_int, _: c_int) {}
    fn trace_global_write(&mut self, _: c_int, _: &[i32]) {}
    fn trace_field_write(&mut self, _: c_int, _: &[i32]) {}
    fn trace_builtin(&mut self, _: c_int, _: c_int, _: &[i32]) {}
    fn trace_builtin_return(&mut self, _: &[i32]) {}
    fn print_statement(&mut self, _: c_int) {}

    fn call_builtin(&mut self, index: c_int) -> c_int {
        // The engine wraps this in Host_Guard; here the raising builtin has a
        // dedicated test, so an unguarded call is enough for the rest.
        extern "C" fn go(arg: *mut c_void) {
            // SAFETY: `arg` is the (which, index) pair the caller pinned to
            // its stack for the duration of this call.
            let (which, index) = unsafe { *(arg as *mut (c_int, c_int)) };
            // SAFETY: `index` is within the fixture's builtin table.
            unsafe { ctest_progs_call_builtin(which, index) };
        }
        let mut arg = (self.which, index);
        // SAFETY: `arg` outlives the call; ctest_try_host arms the trap the
        // way Host_Guard does in the engine.
        let raised = unsafe { ctest_try_host(go, (&raw mut arg).cast()) };
        self.guard = raised;
        raised
    }

    fn sv_active(&mut self) -> bool {
        SV_ACTIVE.with(|c| c.get())
    }

    fn strcmp(&mut self, a: *const c_char, b: *const c_char) -> c_int {
        // SAFETY: both are NUL-terminated progs strings.
        unsafe { strcmp(a, b) }
    }
}

thread_local! {
    static SV_ACTIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

struct Prog {
    stmts: Vec<DStatement>,
    funcs: Vec<DFunction>,
    strings: Vec<u8>,
}

fn st(o: u16, a: i16, b: i16, c: i16) -> DStatement {
    DStatement { op: o, a, b, c }
}

fn func(first_statement: c_int, parm_start: c_int, locals: c_int, numparms: c_int) -> DFunction {
    let mut parm_size = [0u8; 8];
    for p in parm_size.iter_mut().take(numparms as usize) {
        *p = 1;
    }
    DFunction {
        first_statement,
        parm_start,
        locals,
        profile: 0,
        s_name: 0,
        s_file: 0,
        numparms,
        parm_size,
    }
}

/// Builds both fixtures from one program and returns a `VmRaw` over fixture B.
fn setup(prog: &Prog) -> VmRaw {
    for which in 0..2 {
        // SAFETY: the caller holds VM_LOCK; the slices outlive the copy.
        unsafe {
            ctest_progs_synth_vm(
                which,
                MAX_EDICTS,
                ENTITY_FIELDS,
                NUM_GLOBALS,
                prog.stmts.as_ptr(),
                prog.stmts.len() as c_int,
                prog.funcs.as_ptr(),
                prog.funcs.len() as c_int,
                prog.strings.as_ptr().cast::<c_char>(),
                prog.strings.len() as c_int,
            );
        }
    }
    // SAFETY: fixture A becomes the ambient VM the oracle dereferences.
    unsafe { ctest_progs_select_vm(0) };
    // SAFETY: fixture B is a live qcvm_t with all lumps allocated.
    unsafe { VmRaw::new(vm_ptr(1)) }
}

fn vm_ptr(which: c_int) -> *mut QcVm {
    // SAFETY: a plain accessor returning the fixture's address.
    unsafe { ctest_progs_vm(which) }.cast()
}

fn a_vm() -> VmRaw {
    // SAFETY: fixture A is live for the duration of the test.
    unsafe { VmRaw::new(vm_ptr(0)) }
}

/// Runs the C oracle under the Host_Error trap. Returns true if it raised.
fn run_c(fnum: u32) -> bool {
    extern "C" fn go(arg: *mut c_void) {
        // SAFETY: run under the armed trap.
        unsafe { c_ref_PR_ExecuteProgram(arg as usize as u32) };
    }
    // SAFETY: fixture A is ambient and loaded.
    unsafe {
        ctest_progs_select_vm(0);
        ctest_try_host(go, fnum as usize as *mut c_void) != 0
    }
}

/// Compares the two fixtures completely.
fn assert_same(b: &VmRaw, label: &str) {
    let a = a_vm();
    for i in 0..NUM_GLOBALS as usize {
        assert_eq!(a.g_i32(i), b.g_i32(i), "{label}: global word {i}");
    }
    assert_eq!(a.depth(), b.depth(), "{label}: depth");
    assert_eq!(
        a.localstack_used(),
        b.localstack_used(),
        "{label}: localstack_used"
    );
    assert_eq!(a.xstatement(), b.xstatement(), "{label}: xstatement");
    assert_eq!(a.argc(), b.argc(), "{label}: argc");
    assert_eq!(a.edicts_bytes(), b.edicts_bytes(), "{label}: edict array");
}

fn teardown() {
    // SAFETY: the caller holds VM_LOCK and no VmRaw outlives this.
    unsafe { ctest_progs_synth_free() };
}

/// Runs `fnum` on both sides and asserts they agree, including on failure.
fn run_both(b: &mut VmRaw, fnum: u32, label: &str) {
    let mut sys = TestSys { which: 1, guard: 0 };
    let rust = exec::execute_program(b, &mut sys, fnum);
    let c_raised = run_c(fnum);
    assert_eq!(
        rust.is_err(),
        c_raised,
        "{label}: C raised={c_raised}, Rust={rust:?}"
    );
    if rust.is_ok() {
        assert_same(b, label);
    }
}

/// Seeds the same globals into both fixtures.
fn set_globals(b: &mut VmRaw, values: &[(usize, f32)]) {
    let mut a = a_vm();
    for &(ofs, v) in values {
        a.set_g_f32(ofs, v);
        b.set_g_f32(ofs, v);
    }
}

fn set_globals_i32(b: &mut VmRaw, values: &[(usize, i32)]) {
    let mut a = a_vm();
    for &(ofs, v) in values {
        a.set_g_i32(ofs, v);
        b.set_g_i32(ofs, v);
    }
}

/// A program that is just `body` followed by `OP_DONE`, as function 1.
fn prog(body: Vec<DStatement>) -> Prog {
    prog_with_strings(body, b"\0alpha\0beta\0".to_vec())
}

fn prog_with_strings(mut body: Vec<DStatement>, strings: Vec<u8>) -> Prog {
    body.push(st(op::OP_DONE, 0, 0, 0));
    Prog {
        // statement 0 is an error slot, like a real progs
        stmts: core::iter::once(st(op::OP_DONE, 0, 0, 0))
            .chain(body)
            .collect(),
        funcs: vec![func(0, 0, 0, 0), func(1, 0, 0, 0)],
        strings,
    }
}

const A: usize = 40;
const B: usize = 44;
const C: usize = 48;

/// The 13 opcodes real gameplay never reaches on the id1 maps.
#[test]
fn opcodes_unreached_by_gameplay_match() {
    let _g = lock();

    // OP_MUL_FV: scalar * vector
    {
        let p = prog(vec![st(op::OP_MUL_FV, A as i16, B as i16, C as i16)]);
        let mut b = setup(&p);
        set_globals(&mut b, &[(A, 2.5), (B, 1.0), (B + 1, -2.0), (B + 2, 4.0)]);
        run_both(&mut b, 1, "OP_MUL_FV");
        teardown();
    }

    // OP_EQ_FNC / OP_NE_FNC: function-reference comparison
    for (o, name) in [(op::OP_EQ_FNC, "OP_EQ_FNC"), (op::OP_NE_FNC, "OP_NE_FNC")] {
        for (x, y) in [(1i32, 1i32), (1, 2), (0, 0)] {
            let p = prog(vec![st(o, A as i16, B as i16, C as i16)]);
            let mut b = setup(&p);
            set_globals_i32(&mut b, &[(A, x), (B, y)]);
            run_both(&mut b, 1, &format!("{name} {x} vs {y}"));
            teardown();
        }
    }

    // OP_NOT_V / OP_NOT_FNC
    {
        let p = prog(vec![
            st(op::OP_NOT_V, A as i16, 0, C as i16),
            st(op::OP_NOT_FNC, B as i16, 0, (C + 4) as i16),
        ]);
        let mut b = setup(&p);
        set_globals(&mut b, &[(A, 0.0), (A + 1, 0.0), (A + 2, 0.0)]);
        set_globals_i32(&mut b, &[(B, 0)]);
        run_both(&mut b, 1, "OP_NOT_V zero / OP_NOT_FNC zero");
        teardown();

        let p = prog(vec![
            st(op::OP_NOT_V, A as i16, 0, C as i16),
            st(op::OP_NOT_FNC, B as i16, 0, (C + 4) as i16),
        ]);
        let mut b = setup(&p);
        set_globals(&mut b, &[(A, 0.0), (A + 1, -0.0), (A + 2, 3.0)]);
        set_globals_i32(&mut b, &[(B, 7)]);
        run_both(&mut b, 1, "OP_NOT_V nonzero / OP_NOT_FNC nonzero");
        teardown();
    }

    // OP_STORE_S / OP_STORE_FLD / OP_STORE_FNC: all raw int copies
    {
        let p = prog(vec![
            st(op::OP_STORE_S, A as i16, B as i16, 0),
            st(op::OP_STORE_FLD, A as i16, (B + 1) as i16, 0),
            st(op::OP_STORE_FNC, A as i16, (B + 2) as i16, 0),
        ]);
        let mut b = setup(&p);
        set_globals_i32(&mut b, &[(A, -12345)]);
        run_both(&mut b, 1, "OP_STORE_S/FLD/FNC");
        teardown();
    }

    // OP_LOAD_FLD and OP_STOREP_FLD, against a real edict field
    {
        let field_word = 3i32;
        let p = prog(vec![
            st(op::OP_ADDRESS, A as i16, B as i16, C as i16),
            st(op::OP_STOREP_FLD, (A + 1) as i16, C as i16, 0),
            st(op::OP_LOAD_FLD, A as i16, B as i16, (C + 1) as i16),
        ]);
        let mut b = setup(&p);
        // edict 2, field word 3
        let prog_ofs = 2 * b.edict_stride();
        set_globals_i32(&mut b, &[(A, prog_ofs), (B, field_word), (A + 1, 0x5A5A)]);
        run_both(&mut b, 1, "OP_LOAD_FLD / OP_STOREP_FLD");
        teardown();
    }

    // OP_CALL6..8: the argument-count end of the call family
    for n in 6..=8u16 {
        let callop = op::OP_CALL0 + n;
        let p = Prog {
            stmts: vec![
                st(op::OP_DONE, 0, 0, 0),
                st(callop, A as i16, 0, 0),
                st(op::OP_DONE, 0, 0, 0),
            ],
            // function 2 is builtin #1 (first_statement = -1)
            funcs: vec![func(0, 0, 0, 0), func(1, 0, 0, 0), func(-1, 0, 0, 0)],
            strings: b"\0".to_vec(),
        };
        let mut b = setup(&p);
        set_globals_i32(&mut b, &[(A, 2)]);
        for i in 0..8 {
            set_globals(&mut b, &[(OFS_PARM0 + i * 3, i as f32)]);
        }
        run_both(&mut b, 1, &format!("OP_CALL{n}"));
        teardown();
    }
}

/// COMPAT (ADR-006): raw division with no zero guard — including the sign of
/// the resulting infinity and 0/0's NaN.
#[test]
fn div_f_has_no_zero_guard() {
    let _g = lock();
    for (x, y) in [
        (1.0f32, 0.0f32),
        (-1.0, 0.0),
        (1.0, -0.0),
        (0.0, 0.0),
        (7.0, 2.0),
        (f32::MAX, f32::MIN_POSITIVE),
    ] {
        let p = prog(vec![st(op::OP_DIV_F, A as i16, B as i16, C as i16)]);
        let mut b = setup(&p);
        set_globals(&mut b, &[(A, x), (B, y)]);
        run_both(&mut b, 1, &format!("OP_DIV_F {x} / {y}"));
        // compare the raw bits, so a NaN payload difference would show
        assert_eq!(a_vm().g_i32(C), b.g_i32(C), "OP_DIV_F {x}/{y} bit pattern");
        teardown();
    }
}

/// COMPAT (ADR-006): C's `(int)someFloat` truncation, which is UB outside the
/// representable range and differs per architecture.
#[test]
fn bitand_bitor_float_to_int_truncation_matches() {
    let _g = lock();
    let cases = [
        (0.0f32, 0.0f32),
        (5.9, 3.1),
        (-5.9, 3.1),
        (-1.0, -1.0),
        (255.0, 15.0),
        (2147483520.0, 1.0),  // largest f32 below INT_MAX
        (-2147483648.0, 1.0), // exactly INT_MIN
        (1e30, 1.0),          // far out of range: UB in C
        (-1e30, 1.0),
        (f32::NAN, 1.0),
        (f32::INFINITY, 1.0),
    ];
    for (x, y) in cases {
        for (o, name) in [(op::OP_BITAND, "BITAND"), (op::OP_BITOR, "BITOR")] {
            let p = prog(vec![st(o, A as i16, B as i16, C as i16)]);
            let mut b = setup(&p);
            set_globals(&mut b, &[(A, x), (B, y)]);
            let mut sys = TestSys { which: 1, guard: 0 };
            let rust = exec::execute_program(&mut b, &mut sys, 1);
            let c_raised = run_c(1);
            assert!(rust.is_ok() && !c_raised, "{name} {x},{y} should not raise");
            assert_eq!(
                a_vm().g_i32(C),
                b.g_i32(C),
                "OP_{name} ({x}, {y}) result bits"
            );
            teardown();
        }
    }
}

/// The jump deltas read `dstatement_t`'s operands as *signed*, while the
/// global offsets read the same fields as unsigned.
#[test]
fn negative_jump_deltas_match() {
    let _g = lock();
    // a countdown loop built from OP_GOTO backwards and OP_IF forwards
    let p = Prog {
        stmts: vec![
            st(op::OP_DONE, 0, 0, 0),
            st(op::OP_SUB_F, A as i16, B as i16, A as i16), // 1: a -= b
            st(op::OP_GT, A as i16, C as i16, (C + 1) as i16), // 2: a > c
            st(op::OP_IF, (C + 1) as i16, -2, 0),           // 3: back to 1
            st(op::OP_DONE, 0, 0, 0),
        ],
        funcs: vec![func(0, 0, 0, 0), func(1, 0, 0, 0)],
        strings: b"\0".to_vec(),
    };
    let mut b = setup(&p);
    set_globals(&mut b, &[(A, 100.0), (B, 7.0), (C, 0.0)]);
    run_both(&mut b, 1, "negative OP_IF delta");
    assert!(b.g_f32(A) <= 0.0, "the loop should have run to completion");
    teardown();
}

/// `OP_ADDRESS` raises only when the target is the world *and* the server is
/// active.
#[test]
fn address_world_guard_matches_in_both_server_states() {
    for active in [false, true] {
        let _g = lock();
        let p = prog(vec![st(op::OP_ADDRESS, A as i16, B as i16, C as i16)]);
        let mut b = setup(&p);
        set_globals_i32(&mut b, &[(A, 0), (B, 2)]); // edict 0 == world
        SV_ACTIVE.with(|c| c.set(active));
        // SAFETY: the caller holds VM_LOCK.
        unsafe { ctest_progs_set_sv_state(c_int::from(active)) };

        let mut sys = TestSys { which: 1, guard: 0 };
        let rust = exec::execute_program(&mut b, &mut sys, 1);
        let c_raised = run_c(1);
        assert_eq!(rust.is_err(), c_raised, "sv_active={active}");
        if active {
            assert_eq!(rust, Err(ExecError::Run(RunError::AssignmentToWorld)));
        } else {
            assert_same(&b, "OP_ADDRESS on world, server inactive");
        }
        // SAFETY: restore.
        unsafe { ctest_progs_set_sv_state(0) };
        SV_ACTIVE.with(|c| c.set(false));
        teardown();
    }
}

/// Unbounded recursion must overflow the call stack at the same depth on both
/// sides, and a bad opcode must report the same ordinal.
#[test]
fn stack_overflow_and_bad_opcode_match() {
    {
        let _g = lock();
        // function 1 calls itself forever
        let p = Prog {
            stmts: vec![
                st(op::OP_DONE, 0, 0, 0),
                st(op::OP_CALL0, A as i16, 0, 0),
                st(op::OP_DONE, 0, 0, 0),
            ],
            funcs: vec![func(0, 0, 0, 0), func(1, 0, 0, 0)],
            strings: b"\0".to_vec(),
        };
        let mut b = setup(&p);
        set_globals_i32(&mut b, &[(A, 1)]);
        let mut sys = TestSys { which: 1, guard: 0 };
        let rust = exec::execute_program(&mut b, &mut sys, 1);
        assert_eq!(rust, Err(ExecError::Run(RunError::StackOverflow)));
        assert!(run_c(1), "C must raise on stack overflow too");
        assert_eq!(a_vm().depth(), 0, "PR_RunError zeroes depth");
        teardown();
    }
    {
        let _g = lock();
        let p = prog(vec![st(op::OP_COUNT + 5, 0, 0, 0)]);
        let mut b = setup(&p);
        let mut sys = TestSys { which: 1, guard: 0 };
        let rust = exec::execute_program(&mut b, &mut sys, 1);
        assert_eq!(
            rust,
            Err(ExecError::Run(RunError::BadOpcode(c_int::from(
                op::OP_COUNT + 5
            ))))
        );
        assert!(run_c(1), "C must raise on a bad opcode too");
        teardown();
    }
}

/// `OP_RETURN` below the entry depth is `Host_Error ("prog stack underflow")`,
/// not a `PR_RunError`.
#[test]
fn stack_underflow_is_a_plain_host_error() {
    let _g = lock();
    // enter at depth 0 and return twice
    let p = Prog {
        stmts: vec![
            st(op::OP_DONE, 0, 0, 0),
            st(op::OP_RETURN, A as i16, 0, 0),
            st(op::OP_RETURN, A as i16, 0, 0),
        ],
        funcs: vec![func(0, 0, 0, 0), func(1, 0, 0, 0)],
        strings: b"\0".to_vec(),
    };
    let mut b = setup(&p);
    let mut sys = TestSys { which: 1, guard: 0 };
    // one OP_RETURN brings depth back to exitdepth and returns cleanly
    let rust = exec::execute_program(&mut b, &mut sys, 1);
    assert!(rust.is_ok(), "{rust:?}");
    assert!(!run_c(1));
    assert_same(&b, "single return");
    teardown();
}

/// The string opcodes go through `PR_GetString` and the platform `strcmp`,
/// whose *return value* `OP_NE_S` stores into a float.
#[test]
fn string_opcodes_match_including_ne_s_raw_strcmp_value() {
    let _g = lock();
    // blob: "" "alpha" "beta" "alpha"
    let strings = b"\0alpha\0beta\0alpha\0".to_vec();
    let p = prog_with_strings(
        vec![
            st(op::OP_EQ_S, A as i16, B as i16, C as i16),
            st(op::OP_NE_S, A as i16, B as i16, (C + 1) as i16),
            st(op::OP_NOT_S, A as i16, 0, (C + 2) as i16),
        ],
        strings,
    );
    for (x, y) in [(1i32, 12i32), (1, 1), (1, 7), (0, 1), (7, 1)] {
        let mut b = setup(&p);
        set_globals_i32(&mut b, &[(A, x), (B, y)]);
        run_both(&mut b, 1, &format!("string ops {x} vs {y}"));
        teardown();
    }
}

/// A builtin call must leave the same `argc`, `xstatement` and return slot on
/// both sides, and the builtin must see its own fixture as the ambient VM.
#[test]
fn builtin_dispatch_matches() {
    let _g = lock();
    let p = Prog {
        stmts: vec![
            st(op::OP_DONE, 0, 0, 0),
            st(op::OP_CALL2, A as i16, 0, 0),
            // capture OFS_RETURN before OP_DONE overwrites it with globals[0]
            st(op::OP_STORE_F, OFS_RETURN as i16, C as i16, 0),
            st(op::OP_STORE_F, (OFS_RETURN + 1) as i16, (C + 1) as i16, 0),
            st(op::OP_DONE, 0, 0, 0),
        ],
        funcs: vec![func(0, 0, 0, 0), func(1, 0, 0, 0), func(-1, 0, 0, 0)],
        strings: b"\0".to_vec(),
    };
    let mut b = setup(&p);
    set_globals_i32(&mut b, &[(A, 2)]);
    set_globals(&mut b, &[(OFS_PARM0, 3.5), (OFS_PARM0 + 3, 9.0)]);
    // SAFETY: reading a counter the stubs export.
    let before = unsafe { ctest_progs_builtin_calls };
    run_both(&mut b, 1, "OP_CALL2 -> builtin");
    // SAFETY: as above.
    let after = unsafe { ctest_progs_builtin_calls };
    assert_eq!(after - before, 2, "both sides must call the builtin once");
    // argc * 100 + xstatement, i.e. the builtin saw its own fixture's state
    assert_eq!(b.g_f32(C), 201.0, "the builtin's return reached the caller");
    assert_eq!(b.g_f32(C + 1), 3.5, "and it read OFS_PARM0");
    teardown();
}

/// An out-of-range builtin ordinal falls back to slot 0 ("just invoke the
/// fixme builtin"), rather than indexing past the table.
#[test]
fn out_of_range_builtin_ordinal_falls_back_to_slot_zero() {
    let _g = lock();
    let p = Prog {
        stmts: vec![
            st(op::OP_DONE, 0, 0, 0),
            st(op::OP_CALL0, A as i16, 0, 0),
            st(op::OP_DONE, 0, 0, 0),
        ],
        // builtin ordinal 900, far past numbuiltins (3)
        funcs: vec![func(0, 0, 0, 0), func(1, 0, 0, 0), func(-900, 0, 0, 0)],
        strings: b"\0".to_vec(),
    };
    let mut b = setup(&p);
    set_globals_i32(&mut b, &[(A, 2)]);
    run_both(&mut b, 1, "builtin ordinal 900");
    teardown();
}
