//! The progs interpreter (`pr_exec.c`).
//!
//! A near-transliteration, per PLAN.md §8 — idiomatisation waits until trace
//! parity holds. Every opcode's arithmetic is C's, including the quirks called
//! out in ADR-006: raw `OP_DIV_F` with no zero guard, C float→int truncation
//! in `OP_BITAND`/`OP_BITOR`, raw-int localstack copies, and the two different
//! readings of `dstatement_t`'s `short` operands (unsigned as global offsets,
//! signed as jump deltas).
//!
//! Errors are returned, never raised: `PR_RunError`/`Host_Error` happen in the
//! C frame that called us (ADR-009). Builtin dispatch goes through
//! [`ExecSys::call_builtin`], which the engine implements over `Host_Guard` so
//! that a builtin's `Host_Error` cannot `longjmp` across this frame.

use core::ffi::{c_char, c_int};
use core::mem::offset_of;

use quake_types::progs::{
    opcode as op, EntVars, LOCALSTACK_SIZE, MAX_STACK_DEPTH, OFS_PARM0, OFS_RETURN,
};

use crate::arena::{FuncRef, StringError, VmRaw};

/// COMPAT: `pr_exec.c` uses `0x1000000`, deliberately unlike vanilla's decimal
/// 100000 and QSS's `0x10000000`. The threshold is observable through the
/// error it produces.
const RUNAWAY_LIMIT: c_int = 0x0100_0000;

/// What C would have raised.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecError {
    /// `PR_RunError` — prints the current statement and a stack trace, zeroes
    /// `qcvm->depth`, then `Host_Error ("Program error")`.
    Run(RunError),
    /// A plain `Host_Error` with its own message, no statement dump.
    Host(HostError),
    /// A C builtin raised and `Host_Guard` caught the jump; the C frame must
    /// re-issue it with `Host_Reraise` once we have unwound (ADR-009).
    GuardCaught(c_int),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunError {
    /// `stack overflow`
    StackOverflow,
    /// `PR_ExecuteProgram: locals stack overflow`
    LocalsStackOverflow,
    /// `PR_ExecuteProgram: locals stack underflow`
    LocalsStackUnderflow,
    /// `runaway loop error`
    RunawayLoop,
    /// `NULL function`
    NullFunction,
    /// `assignment to world entity`
    AssignmentToWorld,
    /// `Bad opcode %i`
    BadOpcode(c_int),
    /// Not a C error: an edict field offset outside the edict array. C has no
    /// bounds check here at all — see [`VmRaw`]'s accepted-divergence note.
    FieldOutOfRange(i32),
    /// Not a C error either: `OP_CALL*` indexing `qcvm->functions` out of
    /// range. C reads past the lump; same accepted divergence.
    BadFunctionIndex(i32),
    /// `PR_GetString: attempt to get a non-existant string %d` reached from a
    /// string opcode.
    NonExistentString(c_int),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostError {
    /// `prog stack underflow`
    StackUnderflow,
}

/// The engine services the interpreter calls out to. Every method is a C
/// call; none may hold a Rust borrow of the VM across it.
pub trait ExecSys {
    /// Whether `-tracefile` is active (`pr_tracefile != NULL`). Read once per
    /// `PR_ExecuteProgram`, since it cannot change during execution.
    fn trace_enabled(&mut self) -> bool;

    fn trace_enter(&mut self, fnum: c_int);
    fn trace_leave(&mut self);
    fn trace_statement(&mut self, pc: c_int, op: c_int, a: c_int, b: c_int, c: c_int);
    fn trace_global_write(&mut self, ofs: c_int, values: &[i32]);
    fn trace_field_write(&mut self, byteofs: c_int, values: &[i32]);
    fn trace_builtin(&mut self, ordinal: c_int, argc: c_int, parms: &[i32]);
    fn trace_builtin_return(&mut self, ret: &[i32]);

    /// `PR_PrintStatement (st)`, the `qcvm->trace` single-step printer.
    fn print_statement(&mut self, pc: c_int);

    /// `qcvm->builtins[index] ()`, run under `Host_Guard`. Returns
    /// `HOST_GUARD_OK` (0) normally, or the caught jump.
    ///
    /// **On a non-zero return the VM must not be touched again.** `Host_Error`
    /// does its work *before* it jumps — `PR_SwitchQCVM (NULL)`,
    /// `SCR_EndLoadingPlaque`, `Host_ShutdownServer`, `CL_Disconnect`
    /// (`host.c`) — so by the time the guard hands control back here the
    /// ambient VM has been deselected and the server torn down. The lumps
    /// `VmRaw`/`EdictArena` point at survive that today (`Host_ShutdownServer`
    /// does not call `PR_ClearProgs`; only `Host_ClearMemory` does), but the
    /// interpreter must not rely on it: it returns immediately, without
    /// reading the VM or emitting a trace record.
    fn call_builtin(&mut self, index: c_int) -> c_int;

    /// `sv.state == ss_active` (the `OP_ADDRESS` world-entity guard).
    fn sv_active(&mut self) -> bool;

    /// The platform `strcmp`. Not reimplemented in Rust because `OP_NE_S`
    /// stores the *return value* into a float slot, where QuakeC can see it,
    /// and libc's exact non-zero result is part of the observable behaviour
    /// this port has to match on a given platform (ADR-010).
    fn strcmp(&mut self, a: *const c_char, b: *const c_char) -> c_int;
}

/// `PR_EnterFunction` — returns the new program counter (`first_statement - 1`,
/// which the loop's `st++` turns into `first_statement`).
fn enter_function(
    vm: &mut VmRaw,
    sys: &mut dyn ExecSys,
    tracing: bool,
    f: FuncRef,
) -> Result<c_int, ExecError> {
    if tracing {
        sys.trace_enter(vm.function_index(f));
    }

    let depth = vm.depth();
    // COMPAT: C writes stack[depth] *before* incrementing and checking, so on
    // the overflow path it would index stack[MAX_STACK_DEPTH]. That state is
    // unreachable — the previous entry already raised at MAX_STACK_DEPTH and
    // PR_RunError zeroes depth — so the bound is hoisted above the write here
    // rather than reproducing an out-of-bounds store.
    if depth < 0 || depth >= MAX_STACK_DEPTH as c_int {
        return Err(ExecError::Run(RunError::StackOverflow));
    }
    vm.push_stack(depth, vm.xstatement(), vm.xfunction());
    vm.set_depth(depth + 1);
    if vm.depth() >= MAX_STACK_DEPTH as c_int {
        return Err(ExecError::Run(RunError::StackOverflow));
    }

    let func = vm.function(f);

    // save off any locals that the new function steps on
    let c = func.locals;
    if vm.localstack_used() + c > LOCALSTACK_SIZE as c_int {
        return Err(ExecError::Run(RunError::LocalsStackOverflow));
    }
    for i in 0..c {
        let used = vm.localstack_used();
        let v = vm.g_i32((func.parm_start + i) as usize);
        vm.localstack_write(used + i, v);
    }
    vm.set_localstack_used(vm.localstack_used() + c);

    // copy parameters
    let mut o = func.parm_start;
    for i in 0..func.numparms {
        for j in 0..c_int::from(func.parm_size[i as usize]) {
            let v = vm.g_i32(OFS_PARM0 + (i * 3 + j) as usize);
            vm.set_g_i32(o as usize, v);
            o += 1;
        }
    }

    vm.set_xfunction(f);
    Ok(func.first_statement - 1)
}

/// `PR_LeaveFunction` — returns the caller's saved program counter.
fn leave_function(
    vm: &mut VmRaw,
    sys: &mut dyn ExecSys,
    tracing: bool,
) -> Result<c_int, ExecError> {
    if tracing {
        sys.trace_leave();
    }

    if vm.depth() <= 0 {
        return Err(ExecError::Host(HostError::StackUnderflow));
    }

    // Restore locals from the stack
    let xf = vm.xfunction();
    let c = vm.function(xf).locals;
    vm.set_localstack_used(vm.localstack_used() - c);
    if vm.localstack_used() < 0 {
        return Err(ExecError::Run(RunError::LocalsStackUnderflow));
    }

    let parm_start = vm.function(xf).parm_start;
    for i in 0..c {
        let used = vm.localstack_used();
        let v = vm.localstack_read(used + i);
        vm.set_g_i32((parm_start + i) as usize, v);
    }

    // up stack
    let depth = vm.depth() - 1;
    vm.set_depth(depth);
    let (s, f) = vm.stack_slot(depth);
    vm.set_xfunction(f);
    Ok(s)
}

/// `PR_ExecuteProgram`'s interpretation loop.
///
/// The `!fnum || fnum >= numfunctions` precheck stays in the C caller: it
/// wants `ED_Print (PROG_TO_EDICT (pr_global_struct->self))`, which is
/// `pr_edict.c` code that has not moved yet.
pub fn execute_program(vm: &mut VmRaw, sys: &mut dyn ExecSys, fnum: u32) -> Result<(), ExecError> {
    let tracing = sys.trace_enabled();

    let f = vm.function_ptr(fnum as c_int);

    vm.set_trace_flag(false);

    // make a stack frame
    let exitdepth = vm.depth();

    let mut pc = enter_function(vm, sys, tracing, f)?;
    let mut profile: c_int = 0;
    let mut startprofile: c_int = 0;

    loop {
        pc += 1; /* next statement */

        profile += 1;
        if profile > RUNAWAY_LIMIT {
            vm.set_xstatement(pc);
            return Err(ExecError::Run(RunError::RunawayLoop));
        }

        let st = vm.statement(pc);

        if vm.trace_flag() {
            sys.print_statement(pc);
        }
        if tracing {
            sys.trace_statement(
                pc,
                c_int::from(st.op),
                c_int::from(st.a),
                c_int::from(st.b),
                c_int::from(st.c),
            );
        }

        // OPA/OPB/OPC: the operands are reinterpreted as *unsigned* short here
        let a = st.a as u16 as usize;
        let b = st.b as u16 as usize;
        let c = st.c as u16 as usize;

        match st.op {
            op::OP_ADD_F => vm.set_g_f32(c, vm.g_f32(a) + vm.g_f32(b)),
            op::OP_ADD_V => {
                let (x, y) = (vm.g_vec3(a), vm.g_vec3(b));
                vm.set_g_vec3(c, [x[0] + y[0], x[1] + y[1], x[2] + y[2]]);
            }

            op::OP_SUB_F => vm.set_g_f32(c, vm.g_f32(a) - vm.g_f32(b)),
            op::OP_SUB_V => {
                let (x, y) = (vm.g_vec3(a), vm.g_vec3(b));
                vm.set_g_vec3(c, [x[0] - y[0], x[1] - y[1], x[2] - y[2]]);
            }

            op::OP_MUL_F => vm.set_g_f32(c, vm.g_f32(a) * vm.g_f32(b)),
            op::OP_MUL_V => {
                let (x, y) = (vm.g_vec3(a), vm.g_vec3(b));
                // COMPAT: the dot product's summation order is C's
                vm.set_g_f32(c, x[0] * y[0] + x[1] * y[1] + x[2] * y[2]);
            }
            op::OP_MUL_FV => {
                let (s, v) = (vm.g_f32(a), vm.g_vec3(b));
                vm.set_g_vec3(c, [s * v[0], s * v[1], s * v[2]]);
            }
            op::OP_MUL_VF => {
                let (v, s) = (vm.g_vec3(a), vm.g_f32(b));
                vm.set_g_vec3(c, [s * v[0], s * v[1], s * v[2]]);
            }

            // COMPAT (ADR-006): raw division, no zero guard
            op::OP_DIV_F => vm.set_g_f32(c, vm.g_f32(a) / vm.g_f32(b)),

            // COMPAT (ADR-006): C float->int truncation, which is UB for
            // out-of-range values and differs by architecture
            op::OP_BITAND => {
                let v = c_cast_i32(vm.g_f32(a)) & c_cast_i32(vm.g_f32(b));
                vm.set_g_f32(c, v as f32);
            }
            op::OP_BITOR => {
                let v = c_cast_i32(vm.g_f32(a)) | c_cast_i32(vm.g_f32(b));
                vm.set_g_f32(c, v as f32);
            }

            op::OP_GE => vm.set_g_f32(c, bool_f32(vm.g_f32(a) >= vm.g_f32(b))),
            op::OP_LE => vm.set_g_f32(c, bool_f32(vm.g_f32(a) <= vm.g_f32(b))),
            op::OP_GT => vm.set_g_f32(c, bool_f32(vm.g_f32(a) > vm.g_f32(b))),
            op::OP_LT => vm.set_g_f32(c, bool_f32(vm.g_f32(a) < vm.g_f32(b))),
            op::OP_AND => {
                vm.set_g_f32(c, bool_f32(vm.g_f32(a) != 0.0 && vm.g_f32(b) != 0.0));
            }
            op::OP_OR => {
                vm.set_g_f32(c, bool_f32(vm.g_f32(a) != 0.0 || vm.g_f32(b) != 0.0));
            }

            op::OP_NOT_F => vm.set_g_f32(c, bool_f32(vm.g_f32(a) == 0.0)),
            op::OP_NOT_V => {
                let v = vm.g_vec3(a);
                vm.set_g_f32(c, bool_f32(v[0] == 0.0 && v[1] == 0.0 && v[2] == 0.0));
            }
            op::OP_NOT_S => {
                // `!OPA->string || !*PR_GetString (OPA->string)`
                let h = vm.g_i32(a);
                let empty = if h == 0 {
                    true
                } else {
                    vm.string_is_empty(h)
                        .map_err(|StringError::NonExistent(n)| {
                            ExecError::Run(RunError::NonExistentString(n))
                        })?
                };
                vm.set_g_f32(c, bool_f32(empty));
            }
            op::OP_NOT_FNC => vm.set_g_f32(c, bool_f32(vm.g_i32(a) == 0)),
            op::OP_NOT_ENT => vm.set_g_f32(c, bool_f32(vm.is_world(vm.g_i32(a)))),

            op::OP_EQ_F => vm.set_g_f32(c, bool_f32(vm.g_f32(a) == vm.g_f32(b))),
            op::OP_EQ_V => {
                let (x, y) = (vm.g_vec3(a), vm.g_vec3(b));
                vm.set_g_f32(c, bool_f32(x[0] == y[0] && x[1] == y[1] && x[2] == y[2]));
            }
            op::OP_EQ_S => {
                let (x, y) = (string_of(vm, vm.g_i32(a))?, string_of(vm, vm.g_i32(b))?);
                vm.set_g_f32(c, bool_f32(sys.strcmp(x, y) == 0));
            }
            op::OP_EQ_E => vm.set_g_f32(c, bool_f32(vm.g_i32(a) == vm.g_i32(b))),
            op::OP_EQ_FNC => vm.set_g_f32(c, bool_f32(vm.g_i32(a) == vm.g_i32(b))),

            op::OP_NE_F => vm.set_g_f32(c, bool_f32(vm.g_f32(a) != vm.g_f32(b))),
            op::OP_NE_V => {
                let (x, y) = (vm.g_vec3(a), vm.g_vec3(b));
                vm.set_g_f32(c, bool_f32(x[0] != y[0] || x[1] != y[1] || x[2] != y[2]));
            }
            op::OP_NE_S => {
                // COMPAT: C stores strcmp's *raw* return value, not a boolean
                let (x, y) = (string_of(vm, vm.g_i32(a))?, string_of(vm, vm.g_i32(b))?);
                vm.set_g_f32(c, sys.strcmp(x, y) as f32);
            }
            op::OP_NE_E => vm.set_g_f32(c, bool_f32(vm.g_i32(a) != vm.g_i32(b))),
            op::OP_NE_FNC => vm.set_g_f32(c, bool_f32(vm.g_i32(a) != vm.g_i32(b))),

            op::OP_STORE_F
            | op::OP_STORE_ENT
            | op::OP_STORE_FLD
            | op::OP_STORE_S
            | op::OP_STORE_FNC => {
                let v = vm.g_i32(a);
                vm.set_g_i32(b, v);
                if tracing {
                    sys.trace_global_write(b as c_int, &[v]);
                }
            }
            op::OP_STORE_V => {
                let v = vm.g_vec3(a);
                vm.set_g_vec3(b, v);
                if tracing {
                    let w = vm.g_words(b, 3);
                    sys.trace_global_write(b as c_int, &w);
                }
            }

            op::OP_STOREP_F
            | op::OP_STOREP_ENT
            | op::OP_STOREP_FLD
            | op::OP_STOREP_S
            | op::OP_STOREP_FNC => {
                let byteofs = vm.g_i32(b);
                let v = vm.g_i32(a);
                vm.set_ed_i32(byteofs, v)
                    .ok_or(ExecError::Run(RunError::FieldOutOfRange(byteofs)))?;
                if tracing {
                    sys.trace_field_write(byteofs, &[v]);
                }
            }
            op::OP_STOREP_V => {
                let byteofs = vm.g_i32(b);
                let v = vm.g_vec3(a);
                vm.set_ed_vec3(byteofs, v)
                    .ok_or(ExecError::Run(RunError::FieldOutOfRange(byteofs)))?;
                if tracing {
                    // read back through the same offset C's `ptr` used
                    let w = vm.ed_words3(byteofs).unwrap_or([0; 3]);
                    sys.trace_field_write(byteofs, &w);
                }
            }

            op::OP_ADDRESS => {
                let prog = vm.g_i32(a);
                if vm.is_world(prog) && sys.sv_active() {
                    vm.set_xstatement(pc);
                    return Err(ExecError::Run(RunError::AssignmentToWorld));
                }
                vm.set_g_i32(c, vm.field_byte_offset(prog, vm.g_i32(b)));
            }

            op::OP_LOAD_F | op::OP_LOAD_FLD | op::OP_LOAD_ENT | op::OP_LOAD_S | op::OP_LOAD_FNC => {
                let byteofs = vm.field_byte_offset(vm.g_i32(a), vm.g_i32(b));
                let v = vm
                    .ed_i32(byteofs)
                    .ok_or(ExecError::Run(RunError::FieldOutOfRange(byteofs)))?;
                vm.set_g_i32(c, v);
            }
            op::OP_LOAD_V => {
                let byteofs = vm.field_byte_offset(vm.g_i32(a), vm.g_i32(b));
                let v = vm
                    .ed_vec3(byteofs)
                    .ok_or(ExecError::Run(RunError::FieldOutOfRange(byteofs)))?;
                vm.set_g_vec3(c, v);
            }

            // the jump deltas read the same fields as *signed*
            op::OP_IFNOT => {
                if vm.g_i32(a) == 0 {
                    pc += c_int::from(st.b) - 1;
                }
            }
            op::OP_IF => {
                if vm.g_i32(a) != 0 {
                    pc += c_int::from(st.b) - 1;
                }
            }
            op::OP_GOTO => pc += c_int::from(st.a) - 1,

            op::OP_CALL0..=op::OP_CALL8 => {
                let xf = vm.xfunction();
                vm.add_profile(xf, profile - startprofile);
                startprofile = profile;
                vm.set_xstatement(pc);
                vm.set_argc(c_int::from(st.op) - c_int::from(op::OP_CALL0));

                let fnum = vm.g_i32(a);
                if fnum == 0 {
                    return Err(ExecError::Run(RunError::NullFunction));
                }
                // C indexes qcvm->functions[OPA->function] with no upper
                // bound; same accepted divergence as the field offsets.
                if (fnum as u32) >= vm.numfunctions() as u32 {
                    return Err(ExecError::Run(RunError::BadFunctionIndex(fnum)));
                }
                let newf = vm.function_ptr(fnum);
                if vm.function(newf).first_statement < 0 {
                    // Built-in function
                    let mut i = -vm.function(newf).first_statement;
                    if i >= vm.numbuiltins() {
                        i = 0; // just invoke the fixme builtin.
                    }
                    let argc = vm.argc();
                    if tracing {
                        let parms = vm.g_words(OFS_PARM0, (argc.max(0) * 3) as usize);
                        sys.trace_builtin(i, argc, &parms);
                    }
                    let guard = sys.call_builtin(i);
                    if guard != 0 {
                        // The builtin raised. Return *without* touching the VM
                        // -- see ExecSys::call_builtin: Host_Error has already
                        // deselected the qcvm and shut the server down.
                        return Err(ExecError::GuardCaught(guard));
                    }
                    if tracing {
                        let ret = vm.g_words(OFS_RETURN, 3);
                        sys.trace_builtin_return(&ret);
                    }
                } else {
                    // Normal function
                    pc = enter_function(vm, sys, tracing, newf)?;
                }
            }

            op::OP_DONE | op::OP_RETURN => {
                let xf = vm.xfunction();
                vm.add_profile(xf, profile - startprofile);
                startprofile = profile;
                vm.set_xstatement(pc);
                for k in 0..3 {
                    let v = vm.g_i32(a + k);
                    vm.set_g_i32(OFS_RETURN + k, v);
                }
                pc = leave_function(vm, sys, tracing)?;
                if vm.depth() == exitdepth {
                    return Ok(());
                }
            }

            op::OP_STATE => {
                let prog = vm.global_self();
                // COMPAT: `time + 0.1` is evaluated in double (0.1 is a double
                // literal) and then narrowed on assignment to the float field.
                let nextthink = (f64::from(vm.global_time()) + 0.1) as f32;
                let frame = vm.g_f32(a);
                let think = vm.g_i32(b);
                set_entvar_f32(vm, prog, offset_of!(EntVars, nextthink), nextthink)?;
                set_entvar_f32(vm, prog, offset_of!(EntVars, frame), frame)?;
                set_entvar_i32(vm, prog, offset_of!(EntVars, think), think)?;
            }

            _ => {
                vm.set_xstatement(pc);
                return Err(ExecError::Run(RunError::BadOpcode(c_int::from(st.op))));
            }
        }
    }
}

/// C's `x ? 1 : 0` on a comparison result, stored into a float slot.
fn bool_f32(v: bool) -> f32 {
    if v {
        1.0
    } else {
        0.0
    }
}

/// COMPAT (ADR-006): C's `(int)someFloat`. Out-of-range and NaN inputs are UB
/// in C and the observed result differs per architecture, so the behaviour is
/// emulated per target rather than left to Rust's saturating cast.
#[must_use]
pub fn c_cast_i32(v: f32) -> i32 {
    #[cfg(target_arch = "x86_64")]
    {
        // cvttss2si yields INT_MIN ("integer indefinite") for anything it
        // cannot represent, NaN included.
        if v.is_nan() || v >= 2147483648.0 || v < -2147483648.0 {
            i32::MIN
        } else {
            v as i32
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        // AArch64 fcvtzs saturates, which is what Rust's `as` does.
        v as i32
    }
}

/// `PR_GetString`, with its one live error mapped into [`ExecError`] so the
/// raise happens in the C caller's frame (ADR-009).
fn string_of(vm: &VmRaw, handle: i32) -> Result<*const c_char, ExecError> {
    vm.get_string(handle)
        .map_err(|StringError::NonExistent(n)| ExecError::Run(RunError::NonExistentString(n)))
}

/// `ed->v.<field> = value` for `OP_STATE`, addressed the way C does: a byte
/// offset from the edict array base.
fn set_entvar_f32(vm: &mut VmRaw, prog: i32, byte_ofs: usize, v: f32) -> Result<(), ExecError> {
    let ofs = vm.field_byte_offset(prog, (byte_ofs / 4) as i32);
    vm.set_ed_i32(ofs, v.to_bits() as i32)
        .ok_or(ExecError::Run(RunError::FieldOutOfRange(ofs)))
}

fn set_entvar_i32(vm: &mut VmRaw, prog: i32, byte_ofs: usize, v: i32) -> Result<(), ExecError> {
    let ofs = vm.field_byte_offset(prog, (byte_ofs / 4) as i32);
    vm.set_ed_i32(ofs, v)
        .ok_or(ExecError::Run(RunError::FieldOutOfRange(ofs)))
}
