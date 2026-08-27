//! C-ABI shims for the ported QuakeC builtins (`pr_cmds.c` → `quake-progs`,
//! Phase 6 M7).
//!
//! One `#[no_mangle]` export per flipped slot. `Quake/pr_cmds_glue.c` wraps
//! each in a `builtin_t`-shaped function and owns the one `PR_RunError`
//! (ADR-009); `pr_cmds.c`'s tables name those wrappers through `PF_RS`.

use core::ffi::c_int;

use quake_c_sys as c;
use quake_progs::arena::{EdictArena, VmRaw};
use quake_progs::builtins::{self, BuiltinError, BuiltinSys, MsgKind, MsgValue};
use quake_types::progs::QcVm;

/// Status codes shared with `Quake/pr_cmds_glue.c` (keep in sync).
const PRBI_OK: c_int = 0;
const PRBI_ERR_FIND_BAD_STRING: c_int = 1;
const PRBI_ERR_NO_STRING: c_int = 2;
/// A guarded seam raised; `detail` carries `Host_Guard`'s result and the C
/// wrapper re-issues the jump with `Host_Reraise` (ADR-009).
const PRBI_ERR_GUARD: c_int = 3;
const PRBI_ERR_PROGRAM_ERROR: c_int = 4;
const PRBI_ERR_WRITEDEST_NOT_CLIENT: c_int = 5;
const PRBI_ERR_WRITEDEST_BAD_DEST: c_int = 6;
const PRBI_ERR_BAD_EDICT_POINTER: c_int = 7;

/// Deferred console output, for the same reason the parser and loader shims
/// defer theirs: `Con_Printf` is not a leaf.
struct EngineBuiltin {
    /// `(developer, bytes)`
    pending: Vec<(bool, Vec<u8>)>,
}

impl EngineBuiltin {
    fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    fn flush(&mut self) {
        for (developer, mut msg) in core::mem::take(&mut self.pending) {
            msg.retain(|&b| b != 0);
            msg.push(0);
            // SAFETY: NUL-terminated, and the console takes a plain `%s` so
            // progs bytes reach it unmodified.
            unsafe {
                if developer {
                    c::Con_DPrintf(c"%s".as_ptr(), msg.as_ptr());
                } else {
                    c::Con_Printf(c"%s".as_ptr(), msg.as_ptr());
                }
            }
        }
    }
}

/// A NUL-terminated copy of `bytes`, for the seams that take a `const char *`.
fn cstring(bytes: &[u8]) -> Vec<u8> {
    let mut v: Vec<u8> = bytes.iter().copied().filter(|&b| b != 0).collect();
    v.push(0);
    v
}

impl BuiltinSys for EngineBuiltin {
    fn sqrt(&mut self, v: f64) -> f64 {
        c::libm::sqrt(v)
    }

    fn atan2(&mut self, y: f64, x: f64) -> f64 {
        c::libm::atan2(y, x)
    }

    fn floor(&mut self, v: f64) -> f64 {
        c::libm::floor(v)
    }

    fn ceil(&mut self, v: f64) -> f64 {
        // quake-c-sys declares the libm subset the engine uses; `ceil` is only
        // reached from here, so it goes through the glue rather than growing
        // that list for one caller.
        // SAFETY: a libm leaf.
        unsafe { c::PRBI_Glue_Ceil(v) }
    }

    fn fabs(&mut self, v: f64) -> f64 {
        c::libm::fabs(v)
    }

    fn com_rand(&mut self) -> c_int {
        // SAFETY: the engine's own generator; a leaf.
        unsafe { c::COM_Rand() }
    }

    fn angle_vectors(&mut self, angles: [f32; 3]) {
        // SAFETY: `AngleVectors` writes the three pr_global_struct vectors,
        // which the glue supplies; a leaf.
        unsafe { c::PRBI_Glue_AngleVectors(angles.as_ptr()) }
    }

    fn store_temp_string(&mut self, bytes: &[u8]) -> c_int {
        // SAFETY: the glue copies at most `len` bytes and NUL-terminates
        // within the temp-string slot.
        unsafe { c::PRBI_Glue_StoreTempString(bytes.as_ptr().cast(), bytes.len() as c_int) }
    }

    fn var_string(&mut self, first: c_int) -> Vec<u8> {
        // SAFETY: PF_VarString returns its `static char out[1024]`, valid
        // until the next call; the bytes are copied out immediately.
        unsafe {
            let p = c::PRBI_Glue_VarString(first);
            if p.is_null() {
                Vec::new()
            } else {
                core::ffi::CStr::from_ptr(p).to_bytes().to_vec()
            }
        }
    }

    fn cvar_value(&mut self, name: &[u8]) -> f32 {
        let name = cstring(name);
        // SAFETY: NUL-terminated; Cvar_VariableValue is a leaf lookup.
        unsafe { c::PRBI_Glue_CvarValue(name.as_ptr().cast()) }
    }

    fn cvar_set(&mut self, name: &[u8], value: &[u8]) {
        let (name, value) = (cstring(name), cstring(value));
        // SAFETY: both NUL-terminated.
        unsafe { c::Cvar_Set(name.as_ptr().cast(), value.as_ptr().cast()) }
    }

    fn cbuf_add_text(&mut self, text: &[u8]) {
        let text = cstring(text);
        // SAFETY: NUL-terminated. Cbuf_AddText can Sys_Error on overflow,
        // which exits rather than longjmping, so no Rust frame is skipped.
        unsafe { c::Cbuf_AddText(text.as_ptr().cast()) }
    }

    fn ed_alloc(&mut self) -> Result<c_int, BuiltinError> {
        let mut num = 0;
        // SAFETY: the glue runs ED_Alloc under its own Host_Guard, so its
        // "no free edicts" raise never crosses this frame (ADR-009).
        let guard = unsafe { c::PRBI_Glue_EdAlloc(&mut num) };
        if guard != 0 {
            return Err(BuiltinError::GuardCaught(guard));
        }
        Ok(num)
    }

    fn ed_free(&mut self, num: c_int) -> Result<(), BuiltinError> {
        // SAFETY: guarded, as `ed_alloc`.
        let guard = unsafe { c::PRBI_Glue_EdFree(num) };
        if guard != 0 {
            return Err(BuiltinError::GuardCaught(guard));
        }
        Ok(())
    }

    fn ed_print_with_banner(&mut self, banner: &[u8], num: c_int) -> Result<(), BuiltinError> {
        let banner = cstring(banner);
        // SAFETY: NUL-terminated; guarded, as `ed_alloc`. Both halves run in
        // the C frame so the console order matches C's.
        let guard = unsafe { c::PRBI_Glue_EdPrintWithBanner(banner.as_ptr().cast(), num) };
        if guard != 0 {
            return Err(BuiltinError::GuardCaught(guard));
        }
        Ok(())
    }

    fn ed_print_num(&mut self, num: c_int) -> Result<(), BuiltinError> {
        // SAFETY: guarded, as `ed_alloc`.
        let guard = unsafe { c::PRBI_Glue_EdPrintNum(num) };
        if guard != 0 {
            return Err(BuiltinError::GuardCaught(guard));
        }
        Ok(())
    }

    fn maxclients(&mut self) -> c_int {
        // SAFETY: reads one int out of svs.
        unsafe { c::PRBI_Glue_MaxClients() }
    }

    fn msg_write(&mut self, dest: c_int, entnum: c_int, kind: MsgKind, value: MsgValue) {
        let kind = match kind {
            MsgKind::Byte => 0,
            MsgKind::Char => 1,
            MsgKind::Short => 2,
            MsgKind::Long => 3,
            MsgKind::Angle => 4,
            MsgKind::Coord => 5,
            MsgKind::Str => 6,
            MsgKind::Entity => 7,
        };
        let (i, f, bytes) = match value {
            MsgValue::Int(v) => (v, 0.0, None),
            MsgValue::Float(v) => (0, v, None),
            MsgValue::Bytes(b) => (0, 0.0, Some(cstring(b))),
        };
        let p = bytes
            .as_ref()
            .map_or(core::ptr::null(), |b| b.as_ptr().cast());
        // SAFETY: `p` is NUL-terminated when the kind is Str and unread
        // otherwise; the glue resolves `dest`/`entnum` back to the sizebuf
        // WriteDest chose. MSG_Write* can Sys_Error on overflow, which exits
        // rather than longjmping, so no Rust frame is skipped.
        unsafe { c::PRBI_Glue_MsgWrite(dest, entnum, kind, i, f, p) }
    }

    fn changelevel_issued(&mut self, set: bool) -> bool {
        // SAFETY: reads and conditionally sets one `qboolean` in `svs`.
        unsafe { c::PRBI_Glue_ChangelevelIssued(set) }
    }

    fn print(&mut self, msg: &[u8]) {
        self.pending.push((false, msg.to_vec()));
    }

    fn dprint(&mut self, msg: &[u8]) {
        self.pending.push((true, msg.to_vec()));
    }
}

/// The ambient VM, as a borrow-free view.
///
/// # Safety
///
/// A builtin only runs inside `PR_ExecuteProgram`, so the host frame has
/// already selected a loaded VM (ADR-007, ADR-008).
unsafe fn ambient_vm() -> VmRaw {
    // SAFETY: the caller's contract.
    unsafe { VmRaw::new(c::qcvm.cast::<QcVm>()) }
}

/// An arena over the ambient VM's edict array.
///
/// # Safety
///
/// As [`ambient_vm`]; the edict array is `max_edicts * edict_size` bytes.
unsafe fn ambient_arena(vm: &VmRaw) -> EdictArena {
    let stride = vm.edict_stride() as usize;
    let count = vm.max_edicts().max(0) as usize;
    // SAFETY: the caller's contract.
    unsafe { EdictArena::borrowed(vm.edicts_base(), stride, count) }
}

/// Run a builtin with a fresh view of the ambient VM and a deferred console,
/// flushing only after every view has gone out of scope, and encode its
/// outcome as a `PRBI_*` status.
///
/// Every export carries the status, not just the ones that can fail today, so
/// `pr_cmds_glue.c` has one wrapper shape and a builtin that grows a raise
/// later does not need its call site rewritten.
///
/// The exports below are written out one by one rather than generated by a
/// macro because cbindgen parses the source syntactically and never sees a
/// `macro_rules!` expansion — a macro would silently drop them from
/// `quake_rs.h`.
fn run(
    detail: *mut c_int,
    f: impl FnOnce(&mut VmRaw, &mut EngineBuiltin) -> Result<(), BuiltinError>,
) -> c_int {
    let mut sys = EngineBuiltin::new();
    let result = {
        // SAFETY: a builtin only runs inside PR_ExecuteProgram, so the host
        // frame has already selected a loaded VM.
        let mut vm = unsafe { ambient_vm() };
        f(&mut vm, &mut sys)
    };
    sys.flush();
    match result {
        Ok(()) => PRBI_OK,
        Err(BuiltinError::FindBadString) => PRBI_ERR_FIND_BAD_STRING,
        Err(BuiltinError::NonExistentString(n)) => {
            // SAFETY: the glue always passes a live `int`.
            unsafe { *detail = n };
            PRBI_ERR_NO_STRING
        }
        Err(BuiltinError::GuardCaught(g)) => {
            // SAFETY: as above.
            unsafe { *detail = g };
            PRBI_ERR_GUARD
        }
        Err(BuiltinError::ProgramError) => PRBI_ERR_PROGRAM_ERROR,
        Err(BuiltinError::WriteDestNotAClient) => PRBI_ERR_WRITEDEST_NOT_CLIENT,
        Err(BuiltinError::WriteDestBadDestination) => PRBI_ERR_WRITEDEST_BAD_DEST,
        Err(BuiltinError::BadEdictPointer) => PRBI_ERR_BAD_EDICT_POINTER,
    }
}

/// `PF_normalize`.
///
/// # Safety
///
/// Called only from the builtin table, i.e. from inside `PR_ExecuteProgram`,
/// with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_normalize(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| {
        builtins::pf_normalize(vm, sys);
        Ok(())
    })
}

/// `PF_vlen`.
///
/// # Safety
///
/// Called only from the builtin table, i.e. from inside `PR_ExecuteProgram`,
/// with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_vlen(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| {
        builtins::pf_vlen(vm, sys);
        Ok(())
    })
}

/// `PF_vectoyaw`.
///
/// # Safety
///
/// Called only from the builtin table, i.e. from inside `PR_ExecuteProgram`,
/// with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_vectoyaw(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| {
        builtins::pf_vectoyaw(vm, sys);
        Ok(())
    })
}

/// `PF_vectoangles`.
///
/// # Safety
///
/// Called only from the builtin table, i.e. from inside `PR_ExecuteProgram`,
/// with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_vectoangles(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| {
        builtins::pf_vectoangles(vm, sys);
        Ok(())
    })
}

/// `PF_makevectors`.
///
/// # Safety
///
/// Called only from the builtin table, i.e. from inside `PR_ExecuteProgram`,
/// with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_makevectors(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| {
        builtins::pf_makevectors(vm, sys);
        Ok(())
    })
}

/// `PF_random`.
///
/// # Safety
///
/// Called only from the builtin table, i.e. from inside `PR_ExecuteProgram`,
/// with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_random(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| {
        builtins::pf_random(vm, sys);
        Ok(())
    })
}

/// `PF_fabs`.
///
/// # Safety
///
/// Called only from the builtin table, i.e. from inside `PR_ExecuteProgram`,
/// with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_fabs(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| {
        builtins::pf_fabs(vm, sys);
        Ok(())
    })
}

/// `PF_floor`.
///
/// # Safety
///
/// Called only from the builtin table, i.e. from inside `PR_ExecuteProgram`,
/// with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_floor(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| {
        builtins::pf_floor(vm, sys);
        Ok(())
    })
}

/// `PF_ceil`.
///
/// # Safety
///
/// Called only from the builtin table, i.e. from inside `PR_ExecuteProgram`,
/// with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_ceil(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| {
        builtins::pf_ceil(vm, sys);
        Ok(())
    })
}

/// `PF_rint`.
///
/// # Safety
///
/// Called only from the builtin table, i.e. from inside `PR_ExecuteProgram`,
/// with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_rint(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| {
        builtins::pf_rint(vm, sys);
        Ok(())
    })
}

/// `PF_ftos`.
///
/// # Safety
///
/// Called only from the builtin table, i.e. from inside `PR_ExecuteProgram`,
/// with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_ftos(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| {
        builtins::pf_ftos(vm, sys);
        Ok(())
    })
}

/// `PF_vtos`.
///
/// # Safety
///
/// Called only from the builtin table, i.e. from inside `PR_ExecuteProgram`,
/// with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_vtos(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| {
        builtins::pf_vtos(vm, sys);
        Ok(())
    })
}

/// `PF_cvar`.
///
/// # Safety
///
/// Called only from the builtin table, i.e. from inside `PR_ExecuteProgram`,
/// with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_cvar(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| builtins::pf_cvar(vm, sys))
}

/// `PF_cvar_set`.
///
/// # Safety
///
/// Called only from the builtin table, i.e. from inside `PR_ExecuteProgram`,
/// with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_cvar_set(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| builtins::pf_cvar_set(vm, sys))
}

/// `PF_localcmd`.
///
/// # Safety
///
/// Called only from the builtin table, i.e. from inside `PR_ExecuteProgram`,
/// with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_localcmd(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| builtins::pf_localcmd(vm, sys))
}

/// `PF_Find`.
///
/// # Safety
///
/// Called only from the builtin table, i.e. from inside `PR_ExecuteProgram`,
/// with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_Find(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| builtins::pf_find(vm, sys))
}

/// `PF_nextent`.
///
/// # Safety
///
/// Called only from the builtin table, i.e. from inside `PR_ExecuteProgram`,
/// with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_nextent(detail: *mut c_int) -> c_int {
    run(detail, |vm, _sys| {
        builtins::pf_nextent(vm);
        Ok(())
    })
}

/// `PF_traceon`.
///
/// # Safety
///
/// Called only from the builtin table, i.e. from inside `PR_ExecuteProgram`,
/// with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_traceon(detail: *mut c_int) -> c_int {
    run(detail, |vm, _sys| {
        builtins::pf_traceon(vm);
        Ok(())
    })
}

/// `PF_traceoff`.
///
/// # Safety
///
/// Called only from the builtin table, i.e. from inside `PR_ExecuteProgram`,
/// with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_traceoff(detail: *mut c_int) -> c_int {
    run(detail, |vm, _sys| {
        builtins::pf_traceoff(vm);
        Ok(())
    })
}

/// `PF_precache_file`.
///
/// # Safety
///
/// Called only from the builtin table, i.e. from inside `PR_ExecuteProgram`,
/// with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_precache_file(detail: *mut c_int) -> c_int {
    run(detail, |vm, _sys| {
        builtins::pf_precache_file(vm);
        Ok(())
    })
}

/// `PF_dprint`.
///
/// # Safety
///
/// Called only from the builtin table, i.e. from inside `PR_ExecuteProgram`,
/// with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_dprint(detail: *mut c_int) -> c_int {
    run(detail, |_vm, sys| {
        builtins::pf_dprint(sys);
        Ok(())
    })
}

/// `PF_coredump`.
///
/// # Safety
///
/// Called only from the builtin table, i.e. from inside `PR_ExecuteProgram`,
/// with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_coredump(detail: *mut c_int) -> c_int {
    run(detail, |_vm, sys| {
        builtins::pf_coredump(sys);
        Ok(())
    })
}

/// `PF_Spawn`.
///
/// # Safety
///
/// Called only from the builtin table, with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_Spawn(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| builtins::pf_spawn(vm, sys))
}

/// `PF_Remove`.
///
/// # Safety
///
/// Called only from the builtin table, with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_Remove(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| builtins::pf_remove(vm, sys))
}

/// `PF_eprint`.
///
/// # Safety
///
/// Called only from the builtin table, with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_eprint(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| builtins::pf_eprint(vm, sys))
}

/// `PF_error`.
///
/// # Safety
///
/// Called only from the builtin table, with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_error(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| builtins::pf_error(vm, sys))
}

/// `PF_objerror`.
///
/// # Safety
///
/// Called only from the builtin table, with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_objerror(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| builtins::pf_objerror(vm, sys))
}

/// `PF_sv_WriteByte`.
///
/// # Safety
///
/// Called only from the builtin table, with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_WriteByte(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| {
        builtins::pf_sv_write(vm, sys, MsgKind::Byte)
    })
}

/// `PF_sv_WriteChar`.
///
/// # Safety
///
/// Called only from the builtin table, with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_WriteChar(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| {
        builtins::pf_sv_write(vm, sys, MsgKind::Char)
    })
}

/// `PF_sv_WriteShort`.
///
/// # Safety
///
/// Called only from the builtin table, with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_WriteShort(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| {
        builtins::pf_sv_write(vm, sys, MsgKind::Short)
    })
}

/// `PF_sv_WriteLong`.
///
/// # Safety
///
/// Called only from the builtin table, with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_WriteLong(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| {
        builtins::pf_sv_write(vm, sys, MsgKind::Long)
    })
}

/// `PF_sv_WriteAngle`.
///
/// # Safety
///
/// Called only from the builtin table, with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_WriteAngle(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| {
        builtins::pf_sv_write(vm, sys, MsgKind::Angle)
    })
}

/// `PF_sv_WriteCoord`.
///
/// # Safety
///
/// Called only from the builtin table, with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_WriteCoord(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| {
        builtins::pf_sv_write(vm, sys, MsgKind::Coord)
    })
}

/// `PF_sv_WriteString`.
///
/// # Safety
///
/// Called only from the builtin table, with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_WriteString(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| {
        builtins::pf_sv_write(vm, sys, MsgKind::Str)
    })
}

/// `PF_sv_WriteEntity`.
///
/// # Safety
///
/// Called only from the builtin table, with `detail` pointing at a live `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_sv_WriteEntity(detail: *mut c_int) -> c_int {
    run(detail, |vm, sys| {
        builtins::pf_sv_write(vm, sys, MsgKind::Entity)
    })
}

/// `PF_changeyaw`. Flipped in-file in `pr_cmds.c` rather than through the
/// table, because `sv_move.c`'s `SV_MoveToGoal` calls it directly as well and
/// a vtable-slot flip alone would leave the two callers on different
/// implementations.
///
/// # Safety
///
/// Called only with a loaded ambient VM.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pf_changeyaw() {
    // SAFETY: see ambient_vm().
    let mut vm = unsafe { ambient_vm() };
    // SAFETY: see ambient_arena(); a builtin runs on a loaded VM.
    let mut arena = unsafe { ambient_arena(&vm) };
    builtins::pf_changeyaw(&mut vm, &mut arena);
}
