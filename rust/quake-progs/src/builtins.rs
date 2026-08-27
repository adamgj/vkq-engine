//! The self-contained QuakeC builtins (`pr_cmds.c`, Phase 6 M7).
//!
//! These are the builtins whose behaviour is their own — vector maths with
//! `double` intermediates and `(int)` truncation, the `random()` expression,
//! the two string formatters and their temp-string ring, and the edict
//! iterators. Which slots flipped is recorded slot by slot in `pr_cmds.c`'s
//! tables, where each ported entry reads `PF_RS (name)`.
//!
//! # What deliberately stays C, and why
//!
//! Two rules decide the batch. First, a builtin that is a one-line wrapper
//! over server or client code moves no compatibility risk and just adds a
//! boundary crossing. Second, and load-bearing: **a ported builtin may not
//! call an engine seam that can `Host_Error`.** The `Host_Guard` the
//! interpreter wraps a builtin dispatch in (ADR-009, M3) sits *outside* the
//! builtin, so a raise from inside one now longjmps over a Rust frame — the
//! thing ADR-009 exists to forbid. That rules out `PF_Spawn` (`ED_Alloc`
//! raises "no free edicts"), `PF_Remove` (its free-list work belongs with the
//! arena flip anyway), `PF_eprint` (`ED_PrintNum` → `EDICT_NUM`), and
//! `PF_error`/`PF_objerror` (`ED_Print` from a Rust frame). `PF_break` stays C
//! because it deliberately writes through `(int *)-4` to trap into a debugger,
//! which is not expressible.
//!
//! `PF_Find` is in the batch even though it raises, because it raises by
//! *returning* [`BuiltinError::FindBadString`] and the C wrapper issues the
//! `PR_RunError` after the Rust frame has gone.
//!
//! # The engine seams
//!
//! Everything a builtin needs from code that has not moved — libm (ADR-010),
//! `COM_Rand`, `AngleVectors`, `PR_GetTempString`, `PF_VarString`, the cvar
//! and console layers — arrives through [`BuiltinSys`]. Two of those are
//! load-bearing rather than convenience:
//!
//! * `PR_GetTempString`'s ring is **process-global, not per-VM**
//!   (`pr_cmds.c`), shared by the server and client VMs; calling the engine's
//!   keeps that sharing exact.
//! * `PF_VarString` runs the localisation layer (`LOC_GetString`,
//!   `LOC_HasPlaceholders`, `LOC_Format`) and rate-limits its own overflow
//!   warning against `realtime`. It is a shared helper for `pr_ext.c` builtins
//!   too, so it stays one implementation.

use core::ffi::c_int;

use quake_types::progs::{OFS_PARM0, OFS_PARM1, OFS_PARM2, OFS_RETURN};
use quake_util::printf::{format, Arg};

use crate::arena::{entvars_ofs, EdictArena, EdictId, VmRaw};

/// `pr_cmds.c` `STRINGTEMP_LENGTH` — the temp-string buffers the two string
/// builtins format into, and therefore the width at which they truncate.
pub const STRINGTEMP_LENGTH: usize = 1024;

/// What a builtin needs from code that has not moved yet.
pub trait BuiltinSys {
    // ---- libm (ADR-010: the platform's results are the contract) ----
    fn sqrt(&mut self, v: f64) -> f64;
    fn atan2(&mut self, y: f64, x: f64) -> f64;
    fn floor(&mut self, v: f64) -> f64;
    fn ceil(&mut self, v: f64) -> f64;
    fn fabs(&mut self, v: f64) -> f64;
    fn sin(&mut self, v: f64) -> f64;
    fn cos(&mut self, v: f64) -> f64;
    fn tan(&mut self, v: f64) -> f64;
    fn asin(&mut self, v: f64) -> f64;
    fn acos(&mut self, v: f64) -> f64;
    fn atan(&mut self, v: f64) -> f64;
    fn pow(&mut self, a: f64, b: f64) -> f64;
    fn log(&mut self, v: f64) -> f64;

    // ---- the platform string/number conversions (ADR-010) ----

    /// `atof`, i.e. `strtod`.
    fn atof(&mut self, s: &[u8]) -> f64;
    /// `atoi`.
    fn atoi(&mut self, s: &[u8]) -> c_int;
    /// `strtoul (s, NULL, 16)`.
    fn strtoul_hex(&mut self, s: &[u8]) -> u32;
    /// `strcmp` / `q_strcasecmp`. The **raw** return value: QuakeC stores it
    /// into a float slot, so the platform's magnitude is observable.
    fn strcmp(&mut self, a: &[u8], b: &[u8], fold_case: bool) -> c_int;
    /// `strncmp` / `q_strncasecmp`, likewise raw.
    fn strncmp(&mut self, a: &[u8], b: &[u8], len: c_int, fold_case: bool) -> c_int;

    // ---- vector helpers that write engine globals ----

    /// `PF_vectorvectors`' whole body: it derives `v_right`/`v_up` from a
    /// forward vector and writes all three `pr_global_struct` vectors, so the
    /// normalisation and the destinations stay one seam.
    fn vector_vectors(&mut self, forward: [f32; 3]);
    /// `VectorAngles (fwd, up, out)` from `mathlib.c`.
    fn vector_angles(&mut self, forward: [f32; 3], up: Option<[f32; 3]>) -> [f32; 3];

    /// `COM_Rand` — the engine's own generator, so demo determinism holds
    /// (ADR-010).
    fn com_rand(&mut self) -> c_int;

    /// `AngleVectors (angles, pr_global_struct->v_forward, ->v_right, ->v_up)`.
    /// The destinations are engine globals, so the whole call stays one seam.
    fn angle_vectors(&mut self, angles: [f32; 3]);

    /// `PR_GetTempString`, then `q_snprintf`'s copy-and-truncate at
    /// [`STRINGTEMP_LENGTH`], then `PR_SetEngineString` — one seam so the
    /// **process-global** ring is stepped exactly once per call, the way C
    /// steps it.
    fn store_temp_string(&mut self, bytes: &[u8]) -> c_int;

    /// `PF_VarString (first)` — the localisation-aware argument joiner.
    fn var_string(&mut self, first: c_int) -> Vec<u8>;

    /// `Cvar_VariableValue`. Raw progs bytes, not `str`: Quake strings carry
    /// high-bit bytes (the coloured-text charset).
    fn cvar_value(&mut self, name: &[u8]) -> f32;
    /// `Cvar_Set`.
    fn cvar_set(&mut self, name: &[u8], value: &[u8]);
    /// `Cbuf_AddText`.
    fn cbuf_add_text(&mut self, text: &[u8]);
    /// `svs.changelevel_issued`, read and set by `PF_localcmd`'s `restart`
    /// guard. Returns the value *before* the set.
    fn changelevel_issued(&mut self, set: bool) -> bool;

    // ---- guarded seams (ADR-009) ----
    //
    // Each of these can `Host_Error`, so the glue runs it under its own
    // `Host_Guard` and hands the caught jump back as
    // [`BuiltinError::GuardCaught`]; the C wrapper re-issues it with
    // `Host_Reraise` once this Rust frame has returned. Without that, the
    // interpreter's guard — which sits *outside* the builtin dispatch — would
    // longjmp straight over these frames.

    /// `ED_Alloc`, returning the new edict's number. Raises
    /// `"ED_Alloc: no free edicts"`.
    fn ed_alloc(&mut self) -> Result<c_int, BuiltinError>;
    /// `ED_Free (EDICT_NUM_NO_CHECK (num))`.
    fn ed_free(&mut self, num: c_int) -> Result<(), BuiltinError>;
    /// `Con_Printf ("%s", banner)` then `ED_Print (EDICT_NUM_NO_CHECK (num))`,
    /// as one seam.
    ///
    /// The two halves cannot be split: `ED_Print` writes to the console too,
    /// so deferring the banner the way every other diagnostic here is deferred
    /// would print the dumped entity *before* the "======SERVER ERROR" line.
    /// Running both from the C frame keeps C's order and keeps the console
    /// off the Rust side entirely.
    fn ed_print_with_banner(&mut self, banner: &[u8], num: c_int) -> Result<(), BuiltinError>;
    /// `ED_PrintNum (num)` — whose `EDICT_NUM` raises for an out-of-range
    /// number.
    fn ed_print_num(&mut self, num: c_int) -> Result<(), BuiltinError>;

    // ---- message writing ----

    /// `svs.maxclients`, for `WriteDest`'s `MSG_ONE` range test.
    fn maxclients(&mut self) -> c_int;
    /// `MSG_Write<kind> (WriteDest (), ...)`. `dest`/`entnum` are the
    /// destination [`write_dest`] resolved; the glue turns them back into the
    /// `sizebuf_t *` and supplies `sv.protocolflags`/`sv_protocol_pext2`.
    fn msg_write(&mut self, dest: c_int, entnum: c_int, kind: MsgKind, value: MsgValue);

    /// `Con_Printf`, deferred: the console is not a leaf.
    fn print(&mut self, msg: &[u8]);
    /// `Con_DPrintf`, deferred.
    fn dprint(&mut self, msg: &[u8]);
    /// `Con_Warning`, deferred.
    fn warn(&mut self, msg: &[u8]);
    /// `Con_DWarning`, deferred.
    fn dwarn(&mut self, msg: &[u8]);
}

/// The raises a ported builtin can produce, reported so the jump happens in a
/// C frame (ADR-009).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinError {
    /// A guarded engine seam raised; the value is `Host_Guard`'s result, and
    /// the C wrapper re-issues the jump with `Host_Reraise` (ADR-009).
    GuardCaught(c_int),
    /// `PF_error`'s `Host_Error ("Program error")`.
    ProgramError,
    /// `WriteDest`'s `PR_RunError ("WriteDest: not a client")`.
    WriteDestNotAClient,
    /// `WriteDest`'s `PR_RunError ("WriteDest: bad destination")`.
    WriteDestBadDestination,
    /// `NUM_FOR_EDICT`'s `Host_Error ("NUM_FOR_EDICT: bad pointer")`, reached
    /// from `G_EDICTNUM`. Not a debug-only check.
    BadEdictPointer,
    /// `EDICT_NUM`'s `Host_Error ("EDICT_NUM: bad edict_num %i")`, reached
    /// from `PF_edict_for_num`. Also not debug-only.
    BadEdictNum(c_int),
    /// `PF_Find`'s `PR_RunError ("PF_Find: bad search string")`.
    ///
    /// COMPAT: unreachable in practice. C tests `if (!s)` after
    /// `G_STRING (OFS_PARM2)`, but `PR_GetString` never returns NULL — a
    /// cleared handle raises inside it instead, which is
    /// [`BuiltinError::NonExistentString`]. The arm is preserved anyway.
    FindBadString,
    /// `PR_GetString: attempt to get a non-existant string %d` reached from a
    /// builtin's string argument. C raises this *inside* `PR_GetString`; the
    /// port reports it so the jump happens in a C frame (ADR-009).
    NonExistentString(c_int),
}

// ---------------------------------------------------------------------------
// vector maths
//
// Every one of these computes in `double` and stores back to `float`. That is
// not incidental: `(double)v[0] * v[0] + ...` rounds once at the end where a
// float accumulation would round three times, and the state-hash goldens are
// sensitive to it.

/// `void() PF_normalize` — `vector normalize (vector)`
pub fn pf_normalize(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) {
    let v = vm.g_vec3(OFS_PARM0);
    let mut t = f64::from(v[0]) * f64::from(v[0])
        + f64::from(v[1]) * f64::from(v[1])
        + f64::from(v[2]) * f64::from(v[2]);
    t = sys.sqrt(t);

    let out = if t == 0.0 {
        [0.0, 0.0, 0.0]
    } else {
        let t = 1.0 / t;
        [
            (f64::from(v[0]) * t) as f32,
            (f64::from(v[1]) * t) as f32,
            (f64::from(v[2]) * t) as f32,
        ]
    };
    vm.set_g_vec3(OFS_RETURN, out);
}

/// `float vlen (vector)`
pub fn pf_vlen(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) {
    let v = vm.g_vec3(OFS_PARM0);
    let t = f64::from(v[0]) * f64::from(v[0])
        + f64::from(v[1]) * f64::from(v[1])
        + f64::from(v[2]) * f64::from(v[2]);
    vm.set_g_f32(OFS_RETURN, sys.sqrt(t) as f32);
}

/// `float vectoyaw (vector)`
///
/// COMPAT: the `(int)` cast truncates toward zero *before* the negative
/// wrap-around, so the result is always integral and the `< 0` test sees the
/// truncated value.
pub fn pf_vectoyaw(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) {
    let v = vm.g_vec3(OFS_PARM0);
    let yaw = if v[1] == 0.0 && v[0] == 0.0 {
        0.0
    } else {
        let mut yaw = crate::exec::c_cast_i32(
            (sys.atan2(f64::from(v[1]), f64::from(v[0])) * 180.0 / PI) as f32,
        ) as f32;
        if yaw < 0.0 {
            yaw += 360.0;
        }
        yaw
    };
    vm.set_g_f32(OFS_RETURN, yaw);
}

/// `vector vectoangles (vector)`
///
/// COMPAT: the pitch arm wraps to `+360` rather than staying negative, which
/// is why this differs from `mathlib.c`'s `VectorAngles`; and the "straight
/// up/down" arm yields 90/270 rather than ±90.
pub fn pf_vectoangles(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) {
    let v = vm.g_vec3(OFS_PARM0);
    let (pitch, yaw);
    if v[1] == 0.0 && v[0] == 0.0 {
        yaw = 0.0;
        pitch = if v[2] > 0.0 { 90.0 } else { 270.0 };
    } else {
        let mut y = crate::exec::c_cast_i32(
            (sys.atan2(f64::from(v[1]), f64::from(v[0])) * 180.0 / PI) as f32,
        ) as f32;
        if y < 0.0 {
            y += 360.0;
        }
        yaw = y;

        // COMPAT: `forward` is computed in `float`, unlike vlen/normalize
        // above -- `sqrt (v[0] * v[0] + v[1] * v[1])` promotes the float
        // product to double only at the call.
        let forward = sys.sqrt(f64::from(v[0] * v[0] + v[1] * v[1])) as f32;
        let mut p = crate::exec::c_cast_i32(
            (sys.atan2(f64::from(v[2]), f64::from(forward)) * 180.0 / PI) as f32,
        ) as f32;
        if p < 0.0 {
            p += 360.0;
        }
        pitch = p;
    }
    vm.set_g_f32(OFS_RETURN, pitch);
    vm.set_g_f32(OFS_RETURN + 1, yaw);
    vm.set_g_f32(OFS_RETURN + 2, 0.0);
}

/// `mathlib.h` `M_PI`, which `pr_cmds.c` divides by.
const PI: f64 = core::f64::consts::PI;

/// `void() makevectors` — writes `v_forward`/`v_right`/`v_up`.
pub fn pf_makevectors(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) {
    let angles = vm.g_vec3(OFS_PARM0);
    sys.angle_vectors(angles);
}

// ---------------------------------------------------------------------------
// scalar maths

/// `float random ()` — `0 < num < 1`, optionally scaled.
///
/// COMPAT: the comment in `pr_cmds.c` says "Returns a number from 0 <= num <
/// 1" and the code deliberately returns neither endpoint: 0 would break
/// `self.nextthink = time + random () * foo` in `walkmonster_start` (statue
/// monsters), and 1 would break `array[random () * array.length]`. Vanilla
/// could return 1; that is the bug this expression fixes, and it is *not*
/// reverted.
pub fn pf_random(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) {
    let r = sys.com_rand();
    let mut out = (r & 0x7fff) as f32 / 0x8000 as f32 + (0.5 / 0x8000 as f32);

    match vm.argc() {
        0 => {}
        1 => out *= vm.g_f32(OFS_PARM0),
        _ => {
            let lo = vm.g_f32(OFS_PARM0);
            out = lo + out * (vm.g_f32(OFS_PARM1) - lo);
        }
    }
    vm.set_g_f32(OFS_RETURN, out);
}

/// `float fabs (float)`
pub fn pf_fabs(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) {
    let v = vm.g_f32(OFS_PARM0);
    vm.set_g_f32(OFS_RETURN, sys.fabs(f64::from(v)) as f32);
}

/// `float floor (float)`
pub fn pf_floor(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) {
    let v = vm.g_f32(OFS_PARM0);
    vm.set_g_f32(OFS_RETURN, sys.floor(f64::from(v)) as f32);
}

/// `float ceil (float)`
pub fn pf_ceil(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) {
    let v = vm.g_f32(OFS_PARM0);
    vm.set_g_f32(OFS_RETURN, sys.ceil(f64::from(v)) as f32);
}

/// `float rint (float)`
///
/// COMPAT: this is *not* `rint`. It is round-half-away-from-zero through a
/// C `(int)` cast, so it truncates rather than rounding to even, and a value
/// outside `int` range takes the platform's float→int UB path — the same one
/// the interpreter's `OP_BITAND` emulates per arch.
pub fn pf_rint(vm: &mut VmRaw, _sys: &mut dyn BuiltinSys) {
    let f = vm.g_f32(OFS_PARM0);
    let out = if f > 0.0 {
        crate::exec::c_cast_i32(f + 0.5)
    } else {
        crate::exec::c_cast_i32(f - 0.5)
    };
    vm.set_g_f32(OFS_RETURN, out as f32);
}

/// `mathlib.c` `anglemod`, transliterated rather than shared: `quake-math`
/// depends on `quake-c-sys` and `quake-progs` deliberately does not.
///
/// COMPAT: the `& 65535` is applied to a C `int` cast of `a * (65536/360)`, so
/// the wrap is over the truncated integer, not the float.
#[must_use]
pub fn anglemod(a: f32) -> f32 {
    (360.0 / 65536.0) * ((crate::exec::c_cast_i32(a * (65536.0 / 360.0)) & 65535) as f32)
}

/// `void() changeyaw` — turns `self` toward `ideal_yaw` at `yaw_speed`.
pub fn pf_changeyaw(vm: &mut VmRaw, arena: &mut EdictArena) {
    let num = vm.from_prog_num(vm.global_self());
    // COMPAT (accepted divergence): C's PROG_TO_EDICT is unchecked in release
    // builds, so a `self` outside the array is a wild read there. `self` is
    // engine-set on every think, so this cannot fire for a valid VM.
    if num < 0 || num as usize >= arena.count() {
        return;
    }
    let id = EdictId(num as u32);

    let current = anglemod(arena.field_f32(id, entvars_ofs::ANGLES + 4));
    let ideal = arena.field_f32(id, entvars_ofs::IDEAL_YAW);
    let speed = arena.field_f32(id, entvars_ofs::YAW_SPEED);

    if current == ideal {
        return;
    }
    let mut move_ = ideal - current;
    if ideal > current {
        if move_ >= 180.0 {
            move_ -= 360.0;
        }
    } else if move_ <= -180.0 {
        move_ += 360.0;
    }
    if move_ > 0.0 {
        if move_ > speed {
            move_ = speed;
        }
    } else if move_ < -speed {
        move_ = -speed;
    }

    arena.set_field_f32(id, entvars_ofs::ANGLES + 4, anglemod(current + move_));
}

// ---------------------------------------------------------------------------
// string builtins

/// `string ftos (float)`
///
/// COMPAT: the integral test is `v == (int)v`, a C cast, so a value outside
/// `int` range takes the per-arch float→int path before the comparison — and
/// on x86_64 `INT_MIN` compares equal to `-2147483648.0` and prints as an
/// integer, while `1e30` does not.
pub fn pf_ftos(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) {
    let v = vm.g_f32(OFS_PARM0);
    let n = crate::exec::c_cast_i32(v);
    let bytes = if v == n as f32 {
        format(b"%d", &[Arg::I32(n)])
    } else {
        // "dodgy path", as pr_cmds.c calls it
        format(b"%5.1f", &[Arg::F64(f64::from(v))])
    };
    let handle = sys.store_temp_string(&bytes);
    vm.set_g_i32(OFS_RETURN, handle);
}

/// `string vtos (vector)`
pub fn pf_vtos(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) {
    let v = vm.g_vec3(OFS_PARM0);
    let bytes = format(
        b"'%5.1f %5.1f %5.1f'",
        &[
            Arg::F64(f64::from(v[0])),
            Arg::F64(f64::from(v[1])),
            Arg::F64(f64::from(v[2])),
        ],
    );
    let handle = sys.store_temp_string(&bytes);
    vm.set_g_i32(OFS_RETURN, handle);
}

// ---------------------------------------------------------------------------
// edict builtins

/// `entity find (entity start, .string field, string match)`
///
/// COMPAT: the search starts at `start + 1` and returns the *world* entity
/// when nothing matches, which is how QuakeC's `find` loops terminate. A
/// cleared string handle is skipped rather than compared.
///
/// The comparison is byte equality, not a `strcmp` call-through: C tests
/// `!strcmp (t, s)`, and for two NUL-terminated strings "compares equal" and
/// "same bytes" are the same predicate. (`OP_NE_S` in the interpreter *does*
/// call through, because it stores `strcmp`'s raw return value into a float
/// slot where QuakeC can read the platform-specific magnitude.)
pub fn pf_find(vm: &mut VmRaw, _sys: &mut dyn BuiltinSys) -> Result<(), BuiltinError> {
    let mut e = vm.from_prog_num(vm.g_i32(OFS_PARM0));
    let field = vm.g_i32(OFS_PARM1);
    let s = string_arg(vm, vm.g_i32(OFS_PARM2))?;

    e += 1;
    while e < vm.num_edicts() {
        if !vm.edict_free(e) {
            // COMPAT (accepted divergence): C reads the field word with no
            // bounds check, so a fielddef offset past `entityfields` is an
            // out-of-bounds read there; the port skips the edict instead.
            // Unreachable from a well-formed progs.
            if let Some(w) = vm.edict_field_words(e, field, 1) {
                if string_arg(vm, w[0])? == s {
                    vm.set_g_i32(OFS_RETURN, vm.to_prog_num(e));
                    return Ok(());
                }
            }
        }
        e += 1;
    }

    vm.set_g_i32(OFS_RETURN, vm.to_prog_num(0));
    Ok(())
}

/// `entity nextent (entity)` — the next non-free edict, or world at the end.
pub fn pf_nextent(vm: &mut VmRaw) {
    let mut i = vm.from_prog_num(vm.g_i32(OFS_PARM0));
    loop {
        i += 1;
        if i == vm.num_edicts() {
            vm.set_g_i32(OFS_RETURN, vm.to_prog_num(0));
            return;
        }
        if !vm.edict_free(i) {
            vm.set_g_i32(OFS_RETURN, vm.to_prog_num(i));
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// console, cvars and commands

/// `void dprint (string, ...)`
pub fn pf_dprint(sys: &mut dyn BuiltinSys) {
    let s = sys.var_string(0);
    sys.dprint(&s);
}

/// `float cvar (string)`
pub fn pf_cvar(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) -> Result<(), BuiltinError> {
    let name = string_arg(vm, vm.g_i32(OFS_PARM0))?;
    let value = sys.cvar_value(&name);
    vm.set_g_f32(OFS_RETURN, value);
    Ok(())
}

/// `void cvar_set (string var, string val)`
pub fn pf_cvar_set(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) -> Result<(), BuiltinError> {
    let var = string_arg(vm, vm.g_i32(OFS_PARM0))?;
    let val = string_arg(vm, vm.g_i32(OFS_PARM1))?;
    sys.cvar_set(&var, &val);
    Ok(())
}

/// `void localcmd (string)`
///
/// COMPAT: the `restart` guard skips leading bytes `<= ' '` (so control
/// characters count as whitespace) and matches by prefix, so `restartfoo`
/// trips it too.
pub fn pf_localcmd(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) -> Result<(), BuiltinError> {
    let text = string_arg(vm, vm.g_i32(OFS_PARM0))?;
    let start = text.iter().position(|&b| b > b' ').unwrap_or(text.len());
    if text[start..].starts_with(b"restart") && sys.changelevel_issued(true) {
        return Ok(());
    }
    sys.cbuf_add_text(&text);
    Ok(())
}

/// `void coredump ()` — queues the `edicts` console command.
pub fn pf_coredump(sys: &mut dyn BuiltinSys) {
    sys.cbuf_add_text(b"edicts\n");
}

/// `void traceon ()` — the single-step console debugger flag, not the
/// `-tracefile` oracle.
pub fn pf_traceon(vm: &mut VmRaw) {
    vm.set_trace_flag(true);
}

/// `void traceoff ()`
pub fn pf_traceoff(vm: &mut VmRaw) {
    vm.set_trace_flag(false);
}

/// `string precache_file (string)` — a qcc-era no-op that returns its
/// argument unchanged.
pub fn pf_precache_file(vm: &mut VmRaw) {
    let v = vm.g_i32(OFS_PARM0);
    vm.set_g_i32(OFS_RETURN, v);
}

// ---------------------------------------------------------------------------
// edict lifetime and diagnostics
//
// These four were held back from M7 because `ED_Alloc`, `ED_Free`, `ED_Print`
// and `ED_PrintNum` can all `Host_Error` and the interpreter's `Host_Guard`
// sits outside the builtin dispatch. They are here now because the seams
// guard themselves; see the `BuiltinSys` block.

/// `entity spawn ()`
pub fn pf_spawn(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) -> Result<(), BuiltinError> {
    let num = sys.ed_alloc()?;
    let prog = vm.to_prog_num(num);
    vm.set_g_i32(OFS_RETURN, prog);
    Ok(())
}

/// `void remove (entity)`
///
/// COMPAT (accepted divergence in reach, not in behaviour): C's `G_EDICT` is
/// `PROG_TO_EDICT`, which is unchecked in release builds, so a wild prog
/// offset hands `ED_Free` a wild pointer. The port passes the same number
/// through to `EDICT_NUM_NO_CHECK`, so the reachable behaviour is identical.
pub fn pf_remove(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) -> Result<(), BuiltinError> {
    let num = vm.from_prog_num(vm.g_i32(OFS_PARM0));
    sys.ed_free(num)
}

/// `void eprint (entity)`
pub fn pf_eprint(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) -> Result<(), BuiltinError> {
    // C writes `ED_PrintNum (G_EDICTNUM (OFS_PARM0))`, and G_EDICTNUM goes
    // through NUM_FOR_EDICT, whose range test raises in release builds too.
    let num = num_for_edict(vm, vm.g_i32(OFS_PARM0))?;
    sys.ed_print_num(num)
}

/// `void error (string, ...)` — terminal; kills the server.
pub fn pf_error(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) -> Result<(), BuiltinError> {
    let s = sys.var_string(0);
    let banner = error_banner(vm, b"======SERVER ERROR in ", &s);
    let num = vm.from_prog_num(vm.global_self());
    sys.ed_print_with_banner(&banner, num)?;
    Err(BuiltinError::ProgramError)
}

/// `void objerror (string, ...)`
///
/// COMPAT: deliberately **not** fatal — the `Host_Error` is commented out in
/// `pr_cmds.c` with "by design, this should not be fatal". `self` is freed and
/// the level continues.
pub fn pf_objerror(vm: &mut VmRaw, sys: &mut dyn BuiltinSys) -> Result<(), BuiltinError> {
    let s = sys.var_string(0);
    let banner = error_banner(vm, b"======OBJECT ERROR in ", &s);
    let num = vm.from_prog_num(vm.global_self());
    sys.ed_print_with_banner(&banner, num)?;
    sys.ed_free(num)
}

/// `"<banner><current function>:\n<message>\n"`, the shared first line of
/// `PF_error` and `PF_objerror`.
fn error_banner(vm: &VmRaw, banner: &[u8], message: &[u8]) -> Vec<u8> {
    let handle = vm
        .function_name_handle(vm.function_index(vm.xfunction()))
        .unwrap_or(0);
    let name = vm.get_string_bytes(handle).unwrap_or(b"");
    let mut out = banner.to_vec();
    out.extend_from_slice(name);
    out.extend_from_slice(b":\n");
    out.extend_from_slice(message);
    out.push(b'\n');
    out
}

/// `NUM_FOR_EDICT (PROG_TO_EDICT (prog))` including the range test, which is
/// **not** debug-only: `b < 0 || b >= qcvm->num_edicts` raises in release
/// builds too.
pub(crate) fn num_for_edict(vm: &VmRaw, prog: i32) -> Result<c_int, BuiltinError> {
    let num = vm.from_prog_num(prog);
    if num < 0 || num >= vm.num_edicts() {
        return Err(BuiltinError::BadEdictPointer);
    }
    Ok(num)
}

// ---------------------------------------------------------------------------
// message writing

/// Which `MSG_Write*` a `PF_sv_Write*` builtin performs. The glue owns the
/// actual call, so `sv.protocolflags` and `sv_protocol_pext2` never have to
/// cross the boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MsgKind {
    Byte,
    Char,
    Short,
    Long,
    Angle,
    Coord,
    /// The bytes are run through `LOC_GetString` by the glue first.
    Str,
    Entity,
}

/// The value a `MSG_Write*` takes, already converted the way C's implicit
/// argument conversion would convert it.
#[derive(Clone, Debug, PartialEq)]
pub enum MsgValue<'a> {
    /// A C `int` parameter: the float→int conversion is the per-arch one.
    Int(c_int),
    /// A C `float` parameter: no conversion at all.
    Float(f32),
    Bytes(&'a [u8]),
}

/// `server.h` message destinations.
pub const MSG_BROADCAST: c_int = 0;
pub const MSG_ONE: c_int = 1;
pub const MSG_ALL: c_int = 2;
pub const MSG_INIT: c_int = 3;
pub const MSG_EXT_MULTICAST: c_int = 4;
pub const MSG_EXT_ENTITY: c_int = 5;

/// `WriteDest` — which `sizebuf_t` a `PF_sv_Write*` writes into.
///
/// Returns the destination code plus, for `MSG_ONE`, the client's edict
/// number; the glue turns the pair back into the buffer. `MSG_EXT_ENTITY`
/// deliberately shares `MSG_EXT_MULTICAST`'s buffer, as it does in C.
///
/// COMPAT: `dest` comes from `(int)G_FLOAT (OFS_PARM0)`, so it is the per-arch
/// float→int conversion, and `NUM_FOR_EDICT` raises before the
/// `1 .. maxclients` test can run on a bad `msg_entity`.
pub fn write_dest(
    vm: &mut VmRaw,
    sys: &mut dyn BuiltinSys,
) -> Result<(c_int, c_int), BuiltinError> {
    let dest = crate::exec::c_cast_i32(vm.g_f32(OFS_PARM0));
    match dest {
        MSG_BROADCAST | MSG_ALL | MSG_INIT | MSG_EXT_MULTICAST | MSG_EXT_ENTITY => Ok((dest, 0)),
        MSG_ONE => {
            let entnum = num_for_edict(vm, vm.global_msg_entity())?;
            if entnum < 1 || entnum > sys.maxclients() {
                return Err(BuiltinError::WriteDestNotAClient);
            }
            Ok((dest, entnum))
        }
        _ => Err(BuiltinError::WriteDestBadDestination),
    }
}

/// The seven `PF_sv_Write*` builtins whose value is a plain number, plus
/// `PF_sv_WriteString` and `PF_sv_WriteEntity`.
///
/// COMPAT: `MSG_WriteByte`/`Char`/`Short`/`Long` take a C `int`, so
/// `G_FLOAT (OFS_PARM1)` goes through the per-arch float→int conversion;
/// `MSG_WriteAngle`/`Coord` take a `float` and get the value untouched.
pub fn pf_sv_write(
    vm: &mut VmRaw,
    sys: &mut dyn BuiltinSys,
    kind: MsgKind,
) -> Result<(), BuiltinError> {
    let (dest, entnum) = write_dest(vm, sys)?;
    match kind {
        MsgKind::Byte | MsgKind::Char | MsgKind::Short | MsgKind::Long => {
            let v = crate::exec::c_cast_i32(vm.g_f32(OFS_PARM1));
            sys.msg_write(dest, entnum, kind, MsgValue::Int(v));
        }
        MsgKind::Angle | MsgKind::Coord => {
            let v = vm.g_f32(OFS_PARM1);
            sys.msg_write(dest, entnum, kind, MsgValue::Float(v));
        }
        MsgKind::Str => {
            let bytes = string_arg(vm, vm.g_i32(OFS_PARM1))?;
            sys.msg_write(dest, entnum, kind, MsgValue::Bytes(&bytes));
        }
        MsgKind::Entity => {
            let num = num_for_edict(vm, vm.g_i32(OFS_PARM1))?;
            sys.msg_write(dest, entnum, kind, MsgValue::Int(num));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------

/// A progs string argument as raw bytes.
///
/// A cleared engine handle is an error rather than an empty string: C's
/// `G_STRING` goes through `PR_GetString`, which `Host_Error`s on one.
pub(crate) fn string_arg(vm: &VmRaw, handle: c_int) -> Result<Vec<u8>, BuiltinError> {
    vm.get_string_bytes(handle)
        .map(<[u8]>::to_vec)
        .map_err(|crate::arena::StringError::NonExistent(n)| BuiltinError::NonExistentString(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anglemod_matches_the_mathlib_expression() {
        for a in [0.0f32, 1.0, -1.0, 359.9, 360.0, 720.5, -720.5, 1e6] {
            let expected = (360.0 / 65536.0) * (((a * (65536.0 / 360.0)) as i32 & 65535) as f32);
            assert_eq!(anglemod(a), expected, "anglemod({a})");
        }
    }

    #[test]
    fn random_never_returns_zero_or_one() {
        // the two endpoints the expression is written to avoid
        let scaled = |r: i32| (r & 0x7fff) as f32 / 0x8000 as f32 + (0.5 / 0x8000 as f32);
        let lo = scaled(0);
        let hi = scaled(0x7fff);
        assert!(lo > 0.0, "0 would make walkmonster_start produce statues");
        assert!(hi < 1.0, "1 would break array[random () * array.length]");
    }
}
