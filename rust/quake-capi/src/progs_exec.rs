//! C-ABI shim for the progs interpreter (`pr_exec.c` → `quake-progs`).
//!
//! `Quake/pr_exec_glue.c` owns the C frame: it validates the entry function,
//! turns the status returned here back into the exact `PR_RunError`/
//! `Host_Error` the C interpreter produced, and re-issues any jump a builtin
//! raised (ADR-009). Nothing in this module may raise or panic.

use core::ffi::{c_char, c_int};

use quake_c_sys as c;
use quake_progs::arena::VmRaw;
use quake_progs::exec::{self, ExecError, ExecSys, HostError, RunError};
use quake_types::progs::QcVm;

/// Status codes shared with `Quake/pr_exec_glue.c` (keep in sync).
mod status {
    use core::ffi::c_int;

    pub const OK: c_int = 0;
    pub const ERR_STACK_OVERFLOW: c_int = 1;
    pub const ERR_LOCALS_OVERFLOW: c_int = 2;
    pub const ERR_LOCALS_UNDERFLOW: c_int = 3;
    pub const ERR_RUNAWAY: c_int = 4;
    pub const ERR_NULL_FUNCTION: c_int = 5;
    pub const ERR_WORLD_ASSIGN: c_int = 6;
    pub const ERR_BAD_OPCODE: c_int = 7;
    pub const ERR_FIELD_RANGE: c_int = 8;
    pub const ERR_BAD_FUNC_INDEX: c_int = 9;
    pub const ERR_NO_STRING: c_int = 10;
    pub const ERR_STACK_UNDERFLOW: c_int = 11;
    pub const GUARD_ABORTSERVER: c_int = 12;
    pub const GUARD_SCREEN_ERROR: c_int = 13;
}

/// Not a shared constant: any value the glue's switch does not recognise
/// lands in its `default:` "unknown status" arm.
const UNKNOWN_GUARD: c_int = 99;

/// The engine services, each a call straight back into `pr_exec_glue.c`.
struct EngineSys;

impl ExecSys for EngineSys {
    fn trace_enabled(&mut self) -> bool {
        // SAFETY: a leaf predicate in pr_exec_glue.c reading pr_tracefile.
        unsafe { c::PRExec_Glue_TraceEnabled() != 0 }
    }

    fn trace_enter(&mut self, fnum: c_int) {
        // SAFETY: the trace sink is an unlocked FILE * owned by pr_trace.c;
        // progs execution is single-threaded under -headless, which is the
        // only configuration whose traces are oracles (pr_trace.h).
        unsafe { c::PRExec_Glue_TraceEnter(fnum) }
    }

    fn trace_leave(&mut self) {
        // SAFETY: as above.
        unsafe { c::PRExec_Glue_TraceLeave() }
    }

    fn trace_statement(&mut self, pc: c_int, op: c_int, a: c_int, b: c_int, cc: c_int) {
        // SAFETY: as above.
        unsafe { c::PRExec_Glue_TraceStatement(pc, op, a, b, cc) }
    }

    fn trace_global_write(&mut self, ofs: c_int, values: &[i32]) {
        // SAFETY: as above; `values` is a live slice for the call's duration.
        unsafe {
            c::PRExec_Glue_TraceGlobalWrite(ofs, values.as_ptr(), values.len() as c_int);
        }
    }

    fn trace_field_write(&mut self, byteofs: c_int, values: &[i32]) {
        // SAFETY: as above.
        unsafe {
            c::PRExec_Glue_TraceFieldWrite(byteofs, values.as_ptr(), values.len() as c_int);
        }
    }

    fn trace_builtin(&mut self, ordinal: c_int, argc: c_int, parms: &[i32]) {
        // SAFETY: as above; the C side reads argc*3 words, which is what the
        // interpreter collected.
        unsafe { c::PRExec_Glue_TraceBuiltin(ordinal, argc, parms.as_ptr()) }
    }

    fn trace_builtin_return(&mut self, ret: &[i32]) {
        debug_assert_eq!(ret.len(), 3);
        // SAFETY: as above; the C side reads exactly three words.
        unsafe { c::PRExec_Glue_TraceBuiltinReturn(ret.as_ptr()) }
    }

    fn print_statement(&mut self, pc: c_int) {
        // SAFETY: pc indexes the loaded statements lump. This is the
        // qcvm->trace debug path; it reaches Con_Printf, so no Rust borrow of
        // the VM may be live -- the interpreter holds none across a callback.
        unsafe { c::PRExec_Glue_PrintStatement(pc) }
    }

    fn call_builtin(&mut self, index: c_int) -> c_int {
        // SAFETY: the interpreter clamped `index` into 0..numbuiltins. The
        // glue wraps the dispatch in Host_Guard, so a builtin's Host_Error
        // cannot longjmp across this Rust frame (ADR-009 rule 3).
        unsafe { c::PRExec_Glue_CallBuiltin(index) }
    }

    fn sv_active(&mut self) -> bool {
        // SAFETY: a leaf predicate reading sv.state.
        unsafe { c::PRExec_Glue_SvActive() != 0 }
    }

    fn strcmp(&mut self, a: *const c_char, b: *const c_char) -> c_int {
        // SAFETY: both pointers come from PR_GetString and are
        // NUL-terminated progs strings.
        unsafe { c::PRExec_Glue_Strcmp(a, b) }
    }
}

fn encode(err: ExecError, detail: &mut c_int) -> c_int {
    match err {
        ExecError::Run(RunError::StackOverflow) => status::ERR_STACK_OVERFLOW,
        ExecError::Run(RunError::LocalsStackOverflow) => status::ERR_LOCALS_OVERFLOW,
        ExecError::Run(RunError::LocalsStackUnderflow) => status::ERR_LOCALS_UNDERFLOW,
        ExecError::Run(RunError::RunawayLoop) => status::ERR_RUNAWAY,
        ExecError::Run(RunError::NullFunction) => status::ERR_NULL_FUNCTION,
        ExecError::Run(RunError::AssignmentToWorld) => status::ERR_WORLD_ASSIGN,
        ExecError::Run(RunError::BadOpcode(op)) => {
            *detail = op;
            status::ERR_BAD_OPCODE
        }
        ExecError::Run(RunError::FieldOutOfRange(ofs)) => {
            *detail = ofs;
            status::ERR_FIELD_RANGE
        }
        ExecError::Run(RunError::BadFunctionIndex(i)) => {
            *detail = i;
            status::ERR_BAD_FUNC_INDEX
        }
        ExecError::Run(RunError::NonExistentString(n)) => {
            *detail = n;
            status::ERR_NO_STRING
        }
        ExecError::Host(HostError::StackUnderflow) => status::ERR_STACK_UNDERFLOW,
        // HOST_GUARD_ABORTSERVER / HOST_GUARD_SCREEN_ERROR (quakedef.h).
        // Anything else falls through to a status pr_exec_glue.c's `default:`
        // arm reports, rather than being silently re-raised as the wrong jump.
        ExecError::GuardCaught(1) => status::GUARD_ABORTSERVER,
        ExecError::GuardCaught(2) => status::GUARD_SCREEN_ERROR,
        ExecError::GuardCaught(other) => {
            *detail = other;
            UNKNOWN_GUARD
        }
    }
}

/// `PR_ExecuteProgram`'s body. The caller has already validated `fnum`.
///
/// # Safety
///
/// Called only from `pr_exec_glue.c` with the ambient `qcvm` selected and
/// loaded (ADR-008: resolved exactly once, here).
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pr_execute_program(fnum: u32, detail: *mut c_int) -> c_int {
    // SAFETY: `detail` is a live int in the caller's frame.
    let detail = unsafe { &mut *detail };
    *detail = 0;

    // SAFETY: the ambient qcvm is the VM the host frame selected; the mirror
    // is verified against the C layout by quake-ctest/tests/progs_abi.rs.
    let vm_ptr = unsafe { c::qcvm }.cast::<QcVm>();
    // SAFETY: pr_exec_glue.c only calls us with a loaded VM, and the host
    // frame's PR_SwitchQCVM discipline gives exclusive access (ADR-007/008).
    let mut vm = unsafe { VmRaw::new(vm_ptr) };

    let mut sys = EngineSys;
    match exec::execute_program(&mut vm, &mut sys, fnum) {
        Ok(()) => status::OK,
        Err(e) => encode(e, detail),
    }
}
