//! `quake_progs::builtins` over synthetic VMs (Phase 6 M7).
//!
//! Like the loader suite, this one has no whole-function C oracle, and for the
//! same structural reason: `pr_cmds.c` is not an oracle file. Its builtins
//! reach `SV_Move`, `SV_LinkEdict`, `SV_StartSound`, `WriteDest`, the PVS and
//! the client — most of the server — so compiling it under the differential
//! prelude would mean stubbing all of that out, and both sides would then be
//! driven through stubs rather than through the engine. That is why the
//! ROADMAP's flip mechanism for this file is a per-slot vtable flip rather
//! than a whole-file swap.
//!
//! What compares these builtins against C is `trace_diff.py`, and for builtins
//! it is unusually direct: the trace records every builtin call with its
//! arguments (`B`), its return value (`R`) and every global write it performs
//! (`W`), so a C-vs-Rust run over real game code compares each ported builtin
//! call by call. It is byte-identical over six map/game combinations.
//!
//! Two parts of the batch do get a real C oracle here: `PF_ftos` and
//! `PF_vtos`' formatting is checked against the platform `snprintf` through
//! the ADR-005 conformance seam, since that is where their compatibility
//! risk lives.
//!
//! The rest of this suite covers the domain gameplay does not reach —
//! infinities, NaN, the `int` range boundaries, cleared string handles and
//! the `restart` guard's edge cases.

use core::ffi::c_int;

use quake_ctest::c_snprintf_f;
use quake_ctest::c_snprintf_i32;
use quake_progs::arena::{VmRaw, EDICT_V_OFFSET};
use quake_progs::builtins::{self, BuiltinError, BuiltinSys, MsgKind, MsgValue};
use quake_types::progs::{
    DPrograms, Edict, EntVars, QcVm, OFS_PARM0, OFS_PARM1, OFS_PARM2, OFS_RETURN,
};

// ---------------------------------------------------------------------------

#[derive(Default)]
struct MockSys {
    /// Values `COM_Rand` hands out, consumed front to back.
    rand: Vec<c_int>,
    angle_vectors: Vec<[f32; 3]>,
    temp_strings: Vec<Vec<u8>>,
    var_string: Vec<u8>,
    cvar_values: Vec<(Vec<u8>, f32)>,
    cvar_sets: Vec<(Vec<u8>, Vec<u8>)>,
    cbuf: Vec<Vec<u8>>,
    changelevel: bool,
    printed: Vec<Vec<u8>>,
    dprinted: Vec<Vec<u8>>,
    /// Guarded seams: `Some(guard)` makes the next call to that seam report a
    /// caught jump, the way a real `Host_Error` inside it would.
    alloc_guard: Option<c_int>,
    allocated: c_int,
    freed: Vec<c_int>,
    banners: Vec<(Vec<u8>, c_int)>,
    maxclients: c_int,
    writes: Vec<(c_int, c_int, MsgKind, String)>,
}

impl BuiltinSys for MockSys {
    fn sqrt(&mut self, v: f64) -> f64 {
        v.sqrt()
    }
    fn atan2(&mut self, y: f64, x: f64) -> f64 {
        y.atan2(x)
    }
    fn floor(&mut self, v: f64) -> f64 {
        v.floor()
    }
    fn ceil(&mut self, v: f64) -> f64 {
        v.ceil()
    }
    fn fabs(&mut self, v: f64) -> f64 {
        v.abs()
    }

    fn sin(&mut self, v: f64) -> f64 {
        v.sin()
    }
    fn cos(&mut self, v: f64) -> f64 {
        v.cos()
    }
    fn tan(&mut self, v: f64) -> f64 {
        v.tan()
    }
    fn asin(&mut self, v: f64) -> f64 {
        v.asin()
    }
    fn acos(&mut self, v: f64) -> f64 {
        v.acos()
    }
    fn atan(&mut self, v: f64) -> f64 {
        v.atan()
    }
    fn pow(&mut self, a: f64, b: f64) -> f64 {
        a.powf(b)
    }
    fn log(&mut self, v: f64) -> f64 {
        v.ln()
    }

    fn atof(&mut self, s: &[u8]) -> f64 {
        core::str::from_utf8(s)
            .ok()
            .and_then(|t| t.trim().parse().ok())
            .unwrap_or(0.0)
    }
    fn atoi(&mut self, s: &[u8]) -> c_int {
        core::str::from_utf8(s)
            .ok()
            .and_then(|t| t.trim().parse().ok())
            .unwrap_or(0)
    }
    fn strtoul_hex(&mut self, s: &[u8]) -> u32 {
        let t = core::str::from_utf8(s).unwrap_or("");
        let t = t
            .trim_start()
            .trim_start_matches("0x")
            .trim_start_matches("0X");
        let end = t.find(|c: char| !c.is_ascii_hexdigit()).unwrap_or(t.len());
        u32::from_str_radix(&t[..end], 16).unwrap_or(0)
    }
    fn strcmp(&mut self, a: &[u8], b: &[u8], fold_case: bool) -> c_int {
        let (a, b) = (fold(a, fold_case), fold(b, fold_case));
        sign(a.cmp(&b))
    }
    fn strncmp(&mut self, a: &[u8], b: &[u8], len: c_int, fold_case: bool) -> c_int {
        let n = len.max(0) as usize;
        let (a, b) = (fold(a, fold_case), fold(b, fold_case));
        let a = &a[..a.len().min(n)];
        let b = &b[..b.len().min(n)];
        sign(a.cmp(b))
    }

    fn vector_vectors(&mut self, forward: [f32; 3]) {
        self.angle_vectors.push(forward);
    }
    fn vector_angles(&mut self, forward: [f32; 3], up: Option<[f32; 3]>) -> [f32; 3] {
        self.angle_vectors.push(forward);
        up.unwrap_or([1.0, 2.0, 3.0])
    }

    fn warn(&mut self, msg: &[u8]) {
        self.printed.push(msg.to_vec());
    }
    fn dwarn(&mut self, msg: &[u8]) {
        self.dprinted.push(msg.to_vec());
    }

    fn com_rand(&mut self) -> c_int {
        if self.rand.is_empty() {
            0
        } else {
            self.rand.remove(0)
        }
    }

    fn angle_vectors(&mut self, angles: [f32; 3]) {
        self.angle_vectors.push(angles);
    }

    fn store_temp_string(&mut self, bytes: &[u8]) -> c_int {
        // the engine truncates at STRINGTEMP_LENGTH - 1, like q_snprintf
        let n = bytes.len().min(builtins::STRINGTEMP_LENGTH - 1);
        self.temp_strings.push(bytes[..n].to_vec());
        -(self.temp_strings.len() as c_int)
    }

    fn var_string(&mut self, _first: c_int) -> Vec<u8> {
        self.var_string.clone()
    }

    fn cvar_value(&mut self, name: &[u8]) -> f32 {
        self.cvar_values
            .iter()
            .find(|(n, _)| n == name)
            .map_or(0.0, |(_, v)| *v)
    }

    fn cvar_set(&mut self, name: &[u8], value: &[u8]) {
        self.cvar_sets.push((name.to_vec(), value.to_vec()));
    }

    fn cbuf_add_text(&mut self, text: &[u8]) {
        self.cbuf.push(text.to_vec());
    }

    fn changelevel_issued(&mut self, set: bool) -> bool {
        let was = self.changelevel;
        if set {
            self.changelevel = true;
        }
        was
    }

    fn ed_alloc(&mut self) -> Result<c_int, BuiltinError> {
        if let Some(g) = self.alloc_guard.take() {
            return Err(BuiltinError::GuardCaught(g));
        }
        self.allocated += 1;
        Ok(self.allocated)
    }

    fn ed_free(&mut self, num: c_int) -> Result<(), BuiltinError> {
        self.freed.push(num);
        Ok(())
    }

    fn ed_print_with_banner(&mut self, banner: &[u8], num: c_int) -> Result<(), BuiltinError> {
        self.banners.push((banner.to_vec(), num));
        Ok(())
    }

    fn ed_print_num(&mut self, num: c_int) -> Result<(), BuiltinError> {
        self.banners.push((Vec::new(), num));
        Ok(())
    }

    fn maxclients(&mut self) -> c_int {
        self.maxclients
    }

    fn msg_write(&mut self, dest: c_int, entnum: c_int, kind: MsgKind, value: MsgValue) {
        let rendered = match value {
            MsgValue::Int(v) => format!("i:{v}"),
            MsgValue::Float(v) => format!("f:{}", v.to_bits()),
            MsgValue::Bytes(b) => format!("s:{}", String::from_utf8_lossy(b)),
        };
        self.writes.push((dest, entnum, kind, rendered));
    }

    fn print(&mut self, msg: &[u8]) {
        self.printed.push(msg.to_vec());
    }

    fn dprint(&mut self, msg: &[u8]) {
        self.dprinted.push(msg.to_vec());
    }
}

/// Lowercase-fold for the mock's compare seams.
fn fold(s: &[u8], on: bool) -> Vec<u8> {
    if on {
        s.iter().map(|b| b.to_ascii_lowercase()).collect()
    } else {
        s.to_vec()
    }
}

/// The *sign* of a comparison; the mock deliberately does not try to imitate
/// the platform's magnitude, which is why the tests below only assert signs.
fn sign(o: core::cmp::Ordering) -> c_int {
    match o {
        core::cmp::Ordering::Less => -1,
        core::cmp::Ordering::Equal => 0,
        core::cmp::Ordering::Greater => 1,
    }
}

const ENTITYFIELDS: c_int = 128;
const MAX_EDICTS: c_int = 32;

/// A VM with a globals block, an edict array and a strings blob, driven the
/// way `PR_ExecuteProgram` would drive a builtin.
struct Fixture {
    vm: Box<QcVm>,
    _header: Box<DPrograms>,
    globals: Vec<f32>,
    edicts: Vec<u8>,
    strings: Vec<u8>,
    sys: MockSys,
}

impl Fixture {
    fn new(strings: &[u8]) -> Self {
        // SAFETY: qcvm_t is a POD C struct; all-zeroes is the state sv/cl
        // start in.
        let mut vm: Box<QcVm> = unsafe { Box::new(core::mem::zeroed()) };
        let mut header = Box::new(zeroed_header());
        header.entityfields = ENTITYFIELDS;
        header.numstrings = strings.len() as c_int;

        // PR_LoadProgs' expression, rounding included: the arena asserts the
        // stride is pointer-aligned, because engine data lives in the edict.
        let align = core::mem::align_of::<*const u8>();
        let stride = (ENTITYFIELDS as usize * 4
            + (core::mem::size_of::<Edict>() - core::mem::size_of::<EntVars>())
            + align
            - 1)
            & !(align - 1);
        let mut globals = vec![0.0f32; 256];
        let mut edicts = vec![0u8; stride * MAX_EDICTS as usize];
        let mut blob = strings.to_vec();

        vm.progs = &mut *header;
        vm.globals = globals.as_mut_ptr();
        vm.edicts = edicts.as_mut_ptr().cast();
        vm.edict_size = stride as c_int;
        vm.max_edicts = MAX_EDICTS;
        vm.num_edicts = 1;
        vm.strings = blob.as_mut_ptr().cast();
        vm.stringssize = blob.len() as c_int;

        Self {
            vm,
            _header: header,
            globals,
            edicts,
            strings: blob,
            sys: MockSys::default(),
        }
    }

    fn raw(&mut self) -> VmRaw {
        let p: *mut QcVm = &mut *self.vm;
        // SAFETY: every lump a builtin touches is set by `new`, and the edict
        // array is `max_edicts * edict_size` bytes.
        unsafe { VmRaw::new(p) }
    }

    fn set_f32(&mut self, ofs: usize, v: f32) {
        self.globals[ofs] = v;
    }

    fn set_vec3(&mut self, ofs: usize, v: [f32; 3]) {
        self.globals[ofs..ofs + 3].copy_from_slice(&v);
    }

    fn get_f32(&self, ofs: usize) -> f32 {
        self.globals[ofs]
    }

    fn get_i32(&self, ofs: usize) -> i32 {
        self.globals[ofs].to_bits() as i32
    }

    fn set_i32(&mut self, ofs: usize, v: i32) {
        self.globals[ofs] = f32::from_bits(v as u32);
    }

    /// Mark edict `num` live (or free) and give its `field` word a string
    /// handle.
    fn edict_field(&mut self, num: c_int, free: bool, word_ofs: usize, handle: i32) {
        let stride = self.vm.edict_size as usize;
        let free_ofs = core::mem::offset_of!(Edict, free);
        self.edicts[num as usize * stride + free_ofs] = u8::from(free);
        let at = num as usize * stride + EDICT_V_OFFSET + word_ofs * 4;
        self.edicts[at..at + 4].copy_from_slice(&handle.to_le_bytes());
    }
}

fn zeroed_header() -> DPrograms {
    DPrograms {
        version: 0,
        crc: 0,
        ofs_statements: 0,
        numstatements: 0,
        ofs_globaldefs: 0,
        numglobaldefs: 0,
        ofs_fielddefs: 0,
        numfielddefs: 0,
        ofs_functions: 0,
        numfunctions: 0,
        ofs_strings: 0,
        numstrings: 0,
        ofs_globals: 0,
        numglobals: 0,
        entityfields: 0,
    }
}

// ---------------------------------------------------------------------------
// vector maths

#[test]
fn normalize_uses_double_intermediates() {
    // A vector where a float accumulation and a double one give different
    // results, so the test would fail if the port dropped the widening.
    let v = [1.0e-4f32, 3.0e-4, 7.0e-4];
    let mut f = Fixture::new(b"\0");
    f.set_vec3(OFS_PARM0, v);
    let mut sys = core::mem::take(&mut f.sys);
    builtins::pf_normalize(&mut f.raw(), &mut sys);

    let t = (f64::from(v[0]) * f64::from(v[0])
        + f64::from(v[1]) * f64::from(v[1])
        + f64::from(v[2]) * f64::from(v[2]))
    .sqrt();
    let inv = 1.0 / t;
    for (i, &vi) in v.iter().enumerate() {
        assert_eq!(
            f.get_f32(OFS_RETURN + i).to_bits(),
            ((f64::from(vi) * inv) as f32).to_bits(),
            "component {i}"
        );
    }
}

#[test]
fn normalize_of_the_zero_vector_is_zero_not_nan() {
    for v in [[0.0f32, 0.0, 0.0], [-0.0, 0.0, -0.0]] {
        let mut f = Fixture::new(b"\0");
        f.set_vec3(OFS_PARM0, v);
        let mut sys = core::mem::take(&mut f.sys);
        builtins::pf_normalize(&mut f.raw(), &mut sys);
        assert_eq!(f.get_f32(OFS_RETURN), 0.0);
        assert_eq!(f.get_f32(OFS_RETURN + 1), 0.0);
        assert_eq!(f.get_f32(OFS_RETURN + 2), 0.0);
    }
}

#[test]
fn vlen_matches_the_double_expression_including_overflow_to_inf() {
    for v in [
        [3.0f32, 4.0, 0.0],
        [0.0, 0.0, 0.0],
        [f32::MAX, f32::MAX, f32::MAX],
        [f32::MIN_POSITIVE, 0.0, 0.0],
    ] {
        let mut f = Fixture::new(b"\0");
        f.set_vec3(OFS_PARM0, v);
        let mut sys = core::mem::take(&mut f.sys);
        builtins::pf_vlen(&mut f.raw(), &mut sys);
        let expect = (f64::from(v[0]) * f64::from(v[0])
            + f64::from(v[1]) * f64::from(v[1])
            + f64::from(v[2]) * f64::from(v[2]))
        .sqrt() as f32;
        assert_eq!(f.get_f32(OFS_RETURN).to_bits(), expect.to_bits(), "{v:?}");
    }
    // The point of the widening: 1e20 squared overflows f32 but not f64, so a
    // float accumulation would return inf here and the C expression does not.
    let mut f = Fixture::new(b"\0");
    f.set_vec3(OFS_PARM0, [1e20, 1e20, 1e20]);
    let mut sys = core::mem::take(&mut f.sys);
    builtins::pf_vlen(&mut f.raw(), &mut sys);
    assert!(f.get_f32(OFS_RETURN).is_finite(), "double intermediates");
    assert!(
        (1e20f32 * 1e20f32).is_infinite(),
        "and a float one would not be"
    );
}

#[test]
fn vectoyaw_truncates_before_the_negative_wrap() {
    let cases: [([f32; 3], f32); 6] = [
        ([0.0, 0.0, 0.0], 0.0),
        ([0.0, 0.0, 5.0], 0.0), // x and y both zero: the early arm
        ([1.0, 0.0, 0.0], 0.0),
        ([0.0, 1.0, 0.0], 90.0),
        ([-1.0, 0.0, 0.0], 180.0),
        ([0.0, -1.0, 0.0], 270.0),
    ];
    for (v, expect) in cases {
        let mut f = Fixture::new(b"\0");
        f.set_vec3(OFS_PARM0, v);
        let mut sys = core::mem::take(&mut f.sys);
        builtins::pf_vectoyaw(&mut f.raw(), &mut sys);
        assert_eq!(f.get_f32(OFS_RETURN), expect, "{v:?}");
    }

    // an angle whose atan2 is not integral: the result is the *truncated*
    // degrees, never rounded
    let mut f = Fixture::new(b"\0");
    f.set_vec3(OFS_PARM0, [1.0, 0.9, 0.0]);
    let mut sys = core::mem::take(&mut f.sys);
    builtins::pf_vectoyaw(&mut f.raw(), &mut sys);
    let exact = 0.9f64.atan2(1.0) * 180.0 / core::f64::consts::PI;
    assert_eq!(f.get_f32(OFS_RETURN), exact as f32 as i32 as f32);
    assert!(exact.fract() != 0.0, "the fixture must exercise truncation");
}

#[test]
fn vectoangles_wraps_pitch_to_360_and_uses_270_for_straight_down() {
    let mut f = Fixture::new(b"\0");
    f.set_vec3(OFS_PARM0, [0.0, 0.0, -1.0]);
    let mut sys = core::mem::take(&mut f.sys);
    builtins::pf_vectoangles(&mut f.raw(), &mut sys);
    assert_eq!(f.get_f32(OFS_RETURN), 270.0, "pitch");
    assert_eq!(f.get_f32(OFS_RETURN + 1), 0.0, "yaw");
    assert_eq!(f.get_f32(OFS_RETURN + 2), 0.0, "roll is always zero");

    let mut f = Fixture::new(b"\0");
    f.set_vec3(OFS_PARM0, [0.0, 0.0, 1.0]);
    let mut sys = core::mem::take(&mut f.sys);
    builtins::pf_vectoangles(&mut f.raw(), &mut sys);
    assert_eq!(f.get_f32(OFS_RETURN), 90.0);

    // forward-and-up: atan2 (1, 1) is +45, so no wrap
    let mut f = Fixture::new(b"\0");
    f.set_vec3(OFS_PARM0, [1.0, 0.0, 1.0]);
    let mut sys = core::mem::take(&mut f.sys);
    builtins::pf_vectoangles(&mut f.raw(), &mut sys);
    assert_eq!(f.get_f32(OFS_RETURN), 45.0);
    assert_eq!(f.get_f32(OFS_RETURN + 1), 0.0);

    // forward-and-down: atan2 (-1, 1) is -45, and the +360 arm is what makes
    // this 315 rather than -45
    let mut f = Fixture::new(b"\0");
    f.set_vec3(OFS_PARM0, [1.0, 0.0, -1.0]);
    let mut sys = core::mem::take(&mut f.sys);
    builtins::pf_vectoangles(&mut f.raw(), &mut sys);
    assert_eq!(f.get_f32(OFS_RETURN), 315.0, "atan2 (-1, 1) = -45, wrapped");
}

#[test]
fn makevectors_forwards_the_argument_to_the_engine() {
    let mut f = Fixture::new(b"\0");
    f.set_vec3(OFS_PARM0, [10.0, 20.0, 30.0]);
    let mut sys = core::mem::take(&mut f.sys);
    builtins::pf_makevectors(&mut f.raw(), &mut sys);
    assert_eq!(sys.angle_vectors, vec![[10.0, 20.0, 30.0]]);
}

// ---------------------------------------------------------------------------
// scalar maths

#[test]
fn random_is_strictly_inside_the_open_unit_interval() {
    for r in [0, 1, 0x3fff, 0x7fff, -1, i32::MIN, i32::MAX] {
        let mut f = Fixture::new(b"\0");
        f.sys.rand = vec![r];
        let mut sys = core::mem::take(&mut f.sys);
        builtins::pf_random(&mut f.raw(), &mut sys);
        let v = f.get_f32(OFS_RETURN);
        assert!(v > 0.0 && v < 1.0, "COM_Rand {r} -> {v}");
    }
}

#[test]
fn random_scales_by_one_or_two_arguments() {
    // argc == 1: scaled by PARM0
    let mut f = Fixture::new(b"\0");
    f.sys.rand = vec![0x4000];
    f.vm.argc = 1;
    f.set_f32(OFS_PARM0, 10.0);
    let mut sys = core::mem::take(&mut f.sys);
    builtins::pf_random(&mut f.raw(), &mut sys);
    let base = (0x4000i32 & 0x7fff) as f32 / 0x8000 as f32 + (0.5 / 0x8000 as f32);
    assert_eq!(f.get_f32(OFS_RETURN), base * 10.0);

    // argc == 2: lerped between PARM0 and PARM1
    let mut f = Fixture::new(b"\0");
    f.sys.rand = vec![0x4000];
    f.vm.argc = 2;
    f.set_f32(OFS_PARM0, -5.0);
    f.set_f32(OFS_PARM1, 5.0);
    let mut sys = core::mem::take(&mut f.sys);
    builtins::pf_random(&mut f.raw(), &mut sys);
    assert_eq!(f.get_f32(OFS_RETURN), -5.0 + base * 10.0);

    // argc > 2 takes the same arm as argc == 2
    let mut f = Fixture::new(b"\0");
    f.sys.rand = vec![0x4000];
    f.vm.argc = 7;
    f.set_f32(OFS_PARM0, -5.0);
    f.set_f32(OFS_PARM1, 5.0);
    let mut sys = core::mem::take(&mut f.sys);
    builtins::pf_random(&mut f.raw(), &mut sys);
    assert_eq!(f.get_f32(OFS_RETURN), -5.0 + base * 10.0);
}

#[test]
fn rint_rounds_half_away_from_zero_by_truncation() {
    let cases: [(f32, f32); 10] = [
        (0.0, 0.0),
        (0.4, 0.0),
        (0.5, 1.0),
        (1.5, 2.0),
        (2.5, 3.0), // NOT banker's rounding: rint() would give 2
        (-0.4, 0.0),
        (-0.5, -1.0),
        (-1.5, -2.0),
        (-2.5, -3.0),
        (1e9, 1e9),
    ];
    for (input, expect) in cases {
        let mut f = Fixture::new(b"\0");
        f.set_f32(OFS_PARM0, input);
        let mut sys = core::mem::take(&mut f.sys);
        builtins::pf_rint(&mut f.raw(), &mut sys);
        assert_eq!(f.get_f32(OFS_RETURN), expect, "rint({input})");
    }
}

#[test]
fn floor_ceil_fabs_go_through_libm_and_round_back_to_f32() {
    for v in [0.0f32, -0.0, 1.5, -1.5, f32::MAX, f32::MIN, 1e-40] {
        let mut f = Fixture::new(b"\0");
        f.set_f32(OFS_PARM0, v);
        let mut sys = core::mem::take(&mut f.sys);
        builtins::pf_floor(&mut f.raw(), &mut sys);
        assert_eq!(
            f.get_f32(OFS_RETURN).to_bits(),
            (f64::from(v).floor() as f32).to_bits(),
            "floor({v})"
        );

        let mut sys = core::mem::take(&mut f.sys);
        builtins::pf_ceil(&mut f.raw(), &mut sys);
        assert_eq!(
            f.get_f32(OFS_RETURN).to_bits(),
            (f64::from(v).ceil() as f32).to_bits(),
            "ceil({v})"
        );

        let mut sys = core::mem::take(&mut f.sys);
        builtins::pf_fabs(&mut f.raw(), &mut sys);
        assert_eq!(
            f.get_f32(OFS_RETURN).to_bits(),
            (f64::from(v).abs() as f32).to_bits(),
            "fabs({v})"
        );
    }
}

// ---------------------------------------------------------------------------
// the string builtins -- against the platform snprintf (ADR-005 seam)

#[test]
fn ftos_matches_the_platform_snprintf_on_both_arms() {
    let cases: [f32; 14] = [
        0.0,
        -0.0,
        1.0,
        -1.0,
        0.5,
        -0.5,
        123.456,
        1e9,
        -1e9,
        f32::MAX,
        f32::MIN_POSITIVE,
        1e-40,
        2147483520.0,  // the largest f32 below 2^31
        -2147483648.0, // exactly INT_MIN, which round-trips through (int)
    ];
    for v in cases {
        let mut f = Fixture::new(b"\0");
        f.set_f32(OFS_PARM0, v);
        let mut sys = core::mem::take(&mut f.sys);
        builtins::pf_ftos(&mut f.raw(), &mut sys);

        let n = quake_progs::exec::c_cast_i32(v);
        let expect = if v == n as f32 {
            c_snprintf_i32("%d", n)
        } else {
            c_snprintf_f("%5.1f", f64::from(v))
        };
        assert_eq!(
            String::from_utf8_lossy(&sys.temp_strings[0]),
            expect,
            "ftos({v})"
        );
        assert_eq!(f.get_i32(OFS_RETURN), -1, "the returned engine handle");
    }
}

#[test]
fn ftos_of_a_nan_takes_the_float_arm() {
    let mut f = Fixture::new(b"\0");
    f.set_f32(OFS_PARM0, f32::NAN);
    let mut sys = core::mem::take(&mut f.sys);
    builtins::pf_ftos(&mut f.raw(), &mut sys);
    // NaN != (int)NaN whatever the cast produces, so the "%5.1f" arm runs
    assert_eq!(
        String::from_utf8_lossy(&sys.temp_strings[0]),
        c_snprintf_f("%5.1f", f64::from(f32::NAN))
    );
}

#[test]
fn vtos_matches_the_platform_snprintf() {
    for v in [
        [0.0f32, 0.0, 0.0],
        [1.25, -2.5, 3.75],
        [f32::INFINITY, f32::NEG_INFINITY, f32::NAN],
        [1e9, -1e9, 1e-9],
    ] {
        let mut f = Fixture::new(b"\0");
        f.set_vec3(OFS_PARM0, v);
        let mut sys = core::mem::take(&mut f.sys);
        builtins::pf_vtos(&mut f.raw(), &mut sys);
        let expect = format!(
            "'{} {} {}'",
            c_snprintf_f("%5.1f", f64::from(v[0])),
            c_snprintf_f("%5.1f", f64::from(v[1])),
            c_snprintf_f("%5.1f", f64::from(v[2]))
        );
        assert_eq!(
            String::from_utf8_lossy(&sys.temp_strings[0]),
            expect,
            "{v:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// edict iteration

#[test]
fn find_starts_after_the_given_entity_and_falls_back_to_world() {
    // strings blob: "" at 0, "target" at 1, "other" at 8
    let mut f = Fixture::new(b"\0target\0other\0");
    f.vm.num_edicts = 6;
    // edict 1 and 4 both carry "target"; 2 is free but also carries it
    f.edict_field(1, false, 0, 1);
    f.edict_field(2, true, 0, 1);
    f.edict_field(3, false, 0, 8);
    f.edict_field(4, false, 0, 1);
    f.edict_field(5, false, 0, 0);

    let stride = f.vm.edict_size;
    let mut probe = |start: c_int| -> c_int {
        f.set_i32(OFS_PARM0, start * stride);
        f.set_i32(OFS_PARM1, 0);
        f.set_i32(OFS_PARM2, 1);
        let mut sys = core::mem::take(&mut f.sys);
        builtins::pf_find(&mut f.raw(), &mut sys).unwrap();
        f.get_i32(OFS_RETURN) / stride
    };
    assert_eq!(probe(0), 1, "search starts at start + 1");
    assert_eq!(probe(1), 4, "edict 2 matches but is free, so it is skipped");
    assert_eq!(probe(4), 0, "nothing left: world");
    assert_eq!(probe(5), 0);
}

#[test]
fn a_cleared_string_handle_reports_prgetstrings_raise_not_an_empty_string() {
    // C's G_STRING goes through PR_GetString, which Host_Errors on a negative
    // handle whose slot has been cleared -- it does not yield "". Swallowing
    // that would silently change `find`, `cvar` and `localcmd` from raising
    // to searching for / setting the empty string.
    let mut f = Fixture::new(b"\0");
    f.vm.numknownstrings = 2;
    let mut slots: [*const core::ffi::c_char; 2] = [core::ptr::null(); 2];
    f.vm.knownstrings = slots.as_mut_ptr();

    f.set_i32(OFS_PARM2, -1);
    f.set_i32(OFS_PARM0, 0);
    f.set_i32(OFS_PARM1, 0);
    let mut sys = core::mem::take(&mut f.sys);
    assert_eq!(
        builtins::pf_find(&mut f.raw(), &mut sys),
        Err(BuiltinError::NonExistentString(-1))
    );

    f.set_i32(OFS_PARM0, -2);
    assert_eq!(
        builtins::pf_cvar(&mut f.raw(), &mut sys),
        Err(BuiltinError::NonExistentString(-2))
    );
    assert_eq!(
        builtins::pf_localcmd(&mut f.raw(), &mut sys),
        Err(BuiltinError::NonExistentString(-2))
    );
    assert!(sys.cbuf.is_empty(), "nothing reached the command buffer");
}

#[test]
fn nextent_skips_free_edicts_and_wraps_to_world() {
    let mut f = Fixture::new(b"\0");
    f.vm.num_edicts = 4;
    f.edict_field(1, true, 0, 0);
    f.edict_field(2, false, 0, 0);
    f.edict_field(3, true, 0, 0);

    let stride = f.vm.edict_size;
    let mut probe = |start: c_int| -> c_int {
        f.set_i32(OFS_PARM0, start * stride);
        builtins::pf_nextent(&mut f.raw());
        f.get_i32(OFS_RETURN) / stride
    };
    assert_eq!(probe(0), 2, "1 is free");
    assert_eq!(probe(2), 0, "3 is free, then num_edicts: world");
}

// ---------------------------------------------------------------------------
// console, cvars, commands

#[test]
fn localcmd_restart_guard_matches_by_prefix_after_control_bytes() {
    // control bytes count as leading whitespace: the scan is `*str2 <= ' '`
    for text in [
        &b"restart\n"[..],
        b"  restart\n",
        b"\x01\x02restart\n",
        b"restartfoo\n", // a prefix match, not a whole-word one
    ] {
        let mut blob = vec![0u8];
        blob.extend_from_slice(text);
        blob.push(0);
        let mut f = Fixture::new(&blob);
        f.set_i32(OFS_PARM0, 1);
        let mut sys = core::mem::take(&mut f.sys);

        builtins::pf_localcmd(&mut f.raw(), &mut sys).unwrap();
        assert_eq!(sys.cbuf, vec![text.to_vec()], "first {text:?}");
        assert!(sys.changelevel, "the guard latches");

        // second time through, the command is dropped
        sys.cbuf.clear();
        builtins::pf_localcmd(&mut f.raw(), &mut sys).unwrap();
        assert!(sys.cbuf.is_empty(), "second {text:?}");
    }
}

#[test]
fn localcmd_passes_anything_else_straight_through_twice() {
    let mut blob = vec![0u8];
    blob.extend_from_slice(b"map e1m1\n\0");
    let mut f = Fixture::new(&blob);
    f.set_i32(OFS_PARM0, 1);
    let mut sys = core::mem::take(&mut f.sys);
    builtins::pf_localcmd(&mut f.raw(), &mut sys).unwrap();
    builtins::pf_localcmd(&mut f.raw(), &mut sys).unwrap();
    assert_eq!(sys.cbuf.len(), 2);
    assert!(!sys.changelevel);
}

#[test]
fn cvar_names_reach_the_engine_as_raw_bytes() {
    // Quake strings carry high-bit bytes (the coloured-text charset); a lossy
    // UTF-8 round trip would look up a different cvar.
    let mut blob = vec![0u8];
    blob.extend_from_slice(b"sv_\xe1\xe2\0");
    let name_at = 1;
    blob.extend_from_slice(b"1.5\0");
    let value_at = blob.len() as i32 - 4;
    let mut f = Fixture::new(&blob);
    f.sys.cvar_values = vec![(b"sv_\xe1\xe2".to_vec(), 42.0)];

    f.set_i32(OFS_PARM0, name_at);
    let mut sys = core::mem::take(&mut f.sys);
    builtins::pf_cvar(&mut f.raw(), &mut sys).unwrap();
    assert_eq!(f.get_f32(OFS_RETURN), 42.0);

    f.set_i32(OFS_PARM0, name_at);
    f.set_i32(OFS_PARM1, value_at);
    builtins::pf_cvar_set(&mut f.raw(), &mut sys).unwrap();
    assert_eq!(
        sys.cvar_sets,
        vec![(b"sv_\xe1\xe2".to_vec(), b"1.5".to_vec())]
    );
}

#[test]
fn dprint_and_coredump_reach_the_right_console_channels() {
    let mut f = Fixture::new(b"\0");
    f.sys.var_string = b"hello \xe1world".to_vec();
    let mut sys = core::mem::take(&mut f.sys);
    builtins::pf_dprint(&mut sys);
    assert_eq!(sys.dprinted, vec![b"hello \xe1world".to_vec()]);
    assert!(sys.printed.is_empty(), "dprint is developer-only");

    builtins::pf_coredump(&mut sys);
    assert_eq!(sys.cbuf, vec![b"edicts\n".to_vec()]);
}

#[test]
fn traceon_traceoff_and_precache_file() {
    let mut f = Fixture::new(b"\0");
    builtins::pf_traceon(&mut f.raw());
    assert!(f.vm.trace);
    builtins::pf_traceoff(&mut f.raw());
    assert!(!f.vm.trace);

    // precache_file is a qcc-era no-op that returns its argument unchanged,
    // including a handle that does not resolve
    for handle in [0, 7, -3, i32::MIN] {
        f.set_i32(OFS_PARM0, handle);
        builtins::pf_precache_file(&mut f.raw());
        assert_eq!(f.get_i32(OFS_RETURN), handle);
    }
}

#[test]
fn changeyaw_turns_no_further_than_yaw_speed_and_wraps_the_short_way() {
    use quake_progs::arena::{entvars_ofs, EdictArena};

    // (current, ideal, speed, the move C selects). The expected angle is
    // `anglemod (anglemod (current) + move)` -- asserting the *move* is what
    // pins the logic; asserting a literal angle would only re-test anglemod.
    // `None` means the early return: current == ideal writes nothing at all.
    let cases: [(f32, f32, f32, Option<f32>); 6] = [
        (0.0, 0.0, 10.0, None),            // already there
        (0.0, 5.0, 10.0, Some(5.0)),       // within one step
        (0.0, 90.0, 10.0, Some(10.0)),     // clamped to yaw_speed
        (0.0, 350.0, 10.0, Some(-10.0)),   // wraps backwards, not 10 forward
        (350.0, 10.0, 10.0, Some(10.0)),   // wraps forwards across 0, clamped
        (0.0, 180.0, 400.0, Some(-180.0)), // exactly 180 takes the >= arm
    ];
    for (current, ideal, speed, expected_move) in cases {
        let mut f = Fixture::new(b"\0");
        f.vm.num_edicts = 2;
        let stride = f.vm.edict_size as usize;
        let put = |edicts: &mut Vec<u8>, ofs: usize, v: f32| {
            let at = stride + EDICT_V_OFFSET + ofs;
            edicts[at..at + 4].copy_from_slice(&v.to_le_bytes());
        };
        put(&mut f.edicts, entvars_ofs::ANGLES + 4, current);
        put(&mut f.edicts, entvars_ofs::IDEAL_YAW, ideal);
        put(&mut f.edicts, entvars_ofs::YAW_SPEED, speed);

        let self_ofs = core::mem::offset_of!(quake_types::progs::GlobalVars, self_) / 4;
        f.set_i32(self_ofs, f.vm.edict_size);

        let mut vm = f.raw();
        let count = vm.max_edicts().max(0) as usize;
        let base = vm.edicts_base();
        let ed_stride = vm.edict_stride() as usize;
        // SAFETY: the fixture's edict array is `max_edicts * edict_size`.
        let mut arena = unsafe { EdictArena::borrowed(base, ed_stride, count) };
        builtins::pf_changeyaw(&mut vm, &mut arena);

        let at = stride + EDICT_V_OFFSET + entvars_ofs::ANGLES + 4;
        let got = f32::from_le_bytes(f.edicts[at..at + 4].try_into().unwrap());
        let expect = match expected_move {
            None => current,
            Some(m) => builtins::anglemod(builtins::anglemod(current) + m),
        };
        assert_eq!(
            got, expect,
            "changeyaw({current} -> {ideal} @ {speed}), move {expected_move:?}"
        );
    }
}

#[test]
fn changeyaw_ignores_a_self_outside_the_edict_array() {
    // COMPAT (accepted divergence): C's PROG_TO_EDICT is unchecked in release
    // builds, so this would be a wild read there. `self` is engine-set on
    // every think, so it cannot happen for a valid VM.
    use quake_progs::arena::EdictArena;
    let mut f = Fixture::new(b"\0");
    let self_ofs = core::mem::offset_of!(quake_types::progs::GlobalVars, self_) / 4;
    f.set_i32(self_ofs, f.vm.edict_size * (MAX_EDICTS + 4));
    let mut vm = f.raw();
    let count = vm.max_edicts().max(0) as usize;
    let base = vm.edicts_base();
    let ed_stride = vm.edict_stride() as usize;
    // SAFETY: the fixture's edict array is `max_edicts * edict_size`.
    let mut arena = unsafe { EdictArena::borrowed(base, ed_stride, count) };
    builtins::pf_changeyaw(&mut vm, &mut arena);
    assert!(f.edicts.iter().all(|&b| b == 0), "nothing was written");
}

// ---------------------------------------------------------------------------
// M8: the builtins the guarded seams unblocked, and message writing

#[test]
fn spawn_returns_the_new_edicts_prog_offset() {
    let mut f = Fixture::new(b"\0");
    let mut sys = core::mem::take(&mut f.sys);
    builtins::pf_spawn(&mut f.raw(), &mut sys).unwrap();
    assert_eq!(f.get_i32(OFS_RETURN), f.vm.edict_size, "edict 1");
}

#[test]
fn a_guarded_seam_that_raises_comes_back_as_a_caught_jump() {
    // ED_Alloc's "no free edicts" Host_Error is caught by the seam's own
    // Host_Guard and re-issued by the C wrapper once this frame has returned
    // (ADR-009 rule 3). What the builtin must not do is swallow it.
    let mut f = Fixture::new(b"\0");
    f.sys.alloc_guard = Some(1);
    let mut sys = core::mem::take(&mut f.sys);
    assert_eq!(
        builtins::pf_spawn(&mut f.raw(), &mut sys),
        Err(BuiltinError::GuardCaught(1))
    );
    assert_eq!(f.get_i32(OFS_RETURN), 0, "and writes no return value");
}

#[test]
fn remove_frees_the_edict_the_prog_offset_names() {
    let mut f = Fixture::new(b"\0");
    f.set_i32(OFS_PARM0, f.vm.edict_size * 3);
    let mut sys = core::mem::take(&mut f.sys);
    builtins::pf_remove(&mut f.raw(), &mut sys).unwrap();
    assert_eq!(sys.freed, vec![3]);
}

#[test]
fn eprint_range_checks_the_way_num_for_edict_does() {
    // G_EDICTNUM goes through NUM_FOR_EDICT, whose `b < 0 || b >= num_edicts`
    // test raises in release builds too -- it is not one of the DEBUG-only
    // consistency checks.
    let mut f = Fixture::new(b"\0");
    f.vm.num_edicts = 4;
    let stride = f.vm.edict_size;
    let mut sys = core::mem::take(&mut f.sys);

    f.set_i32(OFS_PARM0, stride * 2);
    builtins::pf_eprint(&mut f.raw(), &mut sys).unwrap();
    assert_eq!(sys.banners, vec![(Vec::new(), 2)]);

    for bad in [-stride, stride * 4, stride * 100] {
        f.set_i32(OFS_PARM0, bad);
        assert_eq!(
            builtins::pf_eprint(&mut f.raw(), &mut sys),
            Err(BuiltinError::BadEdictPointer),
            "prog offset {bad}"
        );
    }
}

#[test]
fn error_banners_the_current_function_then_dumps_self_then_raises() {
    let mut f = Fixture::new(b"\0");
    f.sys.var_string = b"it broke".to_vec();
    let self_ofs = core::mem::offset_of!(quake_types::progs::GlobalVars, self_) / 4;
    f.set_i32(self_ofs, f.vm.edict_size * 2);
    let mut sys = core::mem::take(&mut f.sys);

    assert_eq!(
        builtins::pf_error(&mut f.raw(), &mut sys),
        Err(BuiltinError::ProgramError)
    );
    // the banner and the entity dump are one seam, so the console order is
    // C's: banner first, dump second
    assert_eq!(sys.banners.len(), 1);
    assert_eq!(sys.banners[0].1, 2, "self");
    assert!(
        sys.banners[0].0.starts_with(b"======SERVER ERROR in "),
        "{:?}",
        String::from_utf8_lossy(&sys.banners[0].0)
    );
    assert!(sys.banners[0].0.ends_with(b":\nit broke\n"));
    assert!(sys.printed.is_empty(), "nothing is printed separately");
}

#[test]
fn objerror_is_not_fatal_and_frees_self() {
    // pr_cmds.c comments the Host_Error out: "by design, this should not be
    // fatal". The level continues with self removed.
    let mut f = Fixture::new(b"\0");
    f.sys.var_string = b"bad entity".to_vec();
    let self_ofs = core::mem::offset_of!(quake_types::progs::GlobalVars, self_) / 4;
    f.set_i32(self_ofs, f.vm.edict_size * 5);
    let mut sys = core::mem::take(&mut f.sys);

    assert_eq!(builtins::pf_objerror(&mut f.raw(), &mut sys), Ok(()));
    assert!(sys.banners[0].0.starts_with(b"======OBJECT ERROR in "));
    assert_eq!(
        sys.freed,
        vec![5],
        "self is freed, the server is not killed"
    );
}

#[test]
fn write_dest_resolves_every_destination_and_rejects_the_rest() {
    let mut f = Fixture::new(b"\0");
    f.vm.num_edicts = 8;
    f.sys.maxclients = 4;
    let stride = f.vm.edict_size;
    let msg_entity = core::mem::offset_of!(quake_types::progs::GlobalVars, msg_entity) / 4;
    f.set_i32(msg_entity, stride * 2);
    let mut sys = core::mem::take(&mut f.sys);

    for dest in [0, 2, 3, 4, 5] {
        f.set_f32(OFS_PARM0, dest as f32);
        assert_eq!(
            builtins::write_dest(&mut f.raw(), &mut sys),
            Ok((dest, 0)),
            "dest {dest}"
        );
    }

    // MSG_ONE carries the client's edict number
    f.set_f32(OFS_PARM0, 1.0);
    assert_eq!(builtins::write_dest(&mut f.raw(), &mut sys), Ok((1, 2)));

    // a msg_entity outside 1..maxclients is "not a client"
    for bad in [0, 5, 7] {
        f.set_i32(msg_entity, stride * bad);
        assert_eq!(
            builtins::write_dest(&mut f.raw(), &mut sys),
            Err(BuiltinError::WriteDestNotAClient),
            "msg_entity {bad}"
        );
    }
    // and one outside the edict array raises in NUM_FOR_EDICT first
    f.set_i32(msg_entity, stride * 9);
    assert_eq!(
        builtins::write_dest(&mut f.raw(), &mut sys),
        Err(BuiltinError::BadEdictPointer)
    );

    for bad in [-1.0f32, 6.0, 1e9] {
        f.set_f32(OFS_PARM0, bad);
        assert_eq!(
            builtins::write_dest(&mut f.raw(), &mut sys),
            Err(BuiltinError::WriteDestBadDestination),
            "dest {bad}"
        );
    }
}

#[test]
fn the_integer_writers_convert_the_way_c_does_and_the_float_ones_do_not() {
    let mut f = Fixture::new(b"\0");
    f.vm.num_edicts = 8;
    f.sys.maxclients = 4;
    f.set_f32(OFS_PARM0, 0.0); // MSG_BROADCAST

    // MSG_WriteByte/Char/Short/Long take a C `int`, so the float argument
    // goes through the per-arch float->int conversion; Angle/Coord take a
    // `float` and get the value untouched.
    for (kind, v) in [
        (MsgKind::Byte, 3.9f32),
        (MsgKind::Char, -3.9),
        (MsgKind::Short, 1e9),
        (MsgKind::Long, -1e9),
    ] {
        f.set_f32(OFS_PARM1, v);
        let mut sys = core::mem::take(&mut f.sys);
        builtins::pf_sv_write(&mut f.raw(), &mut sys, kind).unwrap();
        let expect = quake_progs::exec::c_cast_i32(v);
        assert_eq!(
            sys.writes.last().unwrap().3,
            format!("i:{expect}"),
            "{kind:?}"
        );
        f.sys = sys;
    }

    for kind in [MsgKind::Angle, MsgKind::Coord] {
        for v in [3.9f32, -0.0, f32::NAN, f32::INFINITY] {
            f.set_f32(OFS_PARM1, v);
            let mut sys = core::mem::take(&mut f.sys);
            builtins::pf_sv_write(&mut f.raw(), &mut sys, kind).unwrap();
            assert_eq!(
                sys.writes.last().unwrap().3,
                format!("f:{}", v.to_bits()),
                "{kind:?} {v}"
            );
            f.sys = sys;
        }
    }
}

#[test]
fn writestring_raises_on_a_cleared_handle_and_writeentity_range_checks() {
    let mut blob = vec![0u8];
    blob.extend_from_slice(b"hello\0");
    let mut f = Fixture::new(&blob);
    f.vm.num_edicts = 4;
    f.sys.maxclients = 4;
    f.set_f32(OFS_PARM0, 0.0);

    f.set_i32(OFS_PARM1, 1);
    let mut sys = core::mem::take(&mut f.sys);
    builtins::pf_sv_write(&mut f.raw(), &mut sys, MsgKind::Str).unwrap();
    assert_eq!(sys.writes.last().unwrap().3, "s:hello");

    // a cleared engine handle: PR_GetString raises rather than yielding ""
    f.vm.numknownstrings = 1;
    let mut slots: [*const core::ffi::c_char; 1] = [core::ptr::null()];
    f.vm.knownstrings = slots.as_mut_ptr();
    f.set_i32(OFS_PARM1, -1);
    assert_eq!(
        builtins::pf_sv_write(&mut f.raw(), &mut sys, MsgKind::Str),
        Err(BuiltinError::NonExistentString(-1))
    );

    // WriteEntity goes through G_EDICTNUM, so it range-checks
    let stride = f.vm.edict_size;
    f.set_i32(OFS_PARM1, stride * 3);
    builtins::pf_sv_write(&mut f.raw(), &mut sys, MsgKind::Entity).unwrap();
    assert_eq!(sys.writes.last().unwrap().3, "i:3");
    f.set_i32(OFS_PARM1, stride * 4);
    assert_eq!(
        builtins::pf_sv_write(&mut f.raw(), &mut sys, MsgKind::Entity),
        Err(BuiltinError::BadEdictPointer)
    );
}

// ---------------------------------------------------------------------------
// M9: pr_ext.c batch 1
//
// Almost nothing here is exercised by id1, hipnotic, rogue or the re-release
// -- these are DarkPlaces/FTE extension builtins that only modern mods call,
// and the corpus's four mod entries are all skipped for want of the mod dirs.
// Trace parity therefore says very little about this batch, and these tests
// are the coverage rather than a supplement to it. Each one pins a quirk that
// a clean-room reimplementation would get wrong.

use quake_progs::ext;

fn ext_fixture(strings: &[u8]) -> Fixture {
    Fixture::new(strings)
}

#[test]
fn min_and_max_fold_from_the_first_argument_and_ignore_nan_after_it() {
    let mut f = ext_fixture(b"\0");
    f.vm.argc = 3;
    f.set_f32(OFS_PARM0, 5.0);
    f.set_f32(OFS_PARM1, f32::NAN);
    f.set_f32(OFS_PARM2, 2.0);
    ext::pf_min(&mut f.raw());
    assert_eq!(f.get_f32(OFS_RETURN), 2.0, "NaN never wins a `>` test");
    ext::pf_max(&mut f.raw());
    assert_eq!(f.get_f32(OFS_RETURN), 5.0, "nor a `<` one");

    // a NaN *first* argument is never replaced, because both comparisons
    // against it are false
    f.set_f32(OFS_PARM0, f32::NAN);
    f.set_f32(OFS_PARM1, 1.0);
    f.set_f32(OFS_PARM2, 2.0);
    ext::pf_min(&mut f.raw());
    assert!(f.get_f32(OFS_RETURN).is_nan());
}

#[test]
fn bound_clamps_to_the_maximum_first() {
    // bound (10, x, 5) returns 10, not 5: the max clamp runs before the min
    let mut f = ext_fixture(b"\0");
    for (lo, v, hi, expect) in [
        (0.0f32, 5.0f32, 10.0f32, 5.0f32),
        (0.0, -5.0, 10.0, 0.0),
        (0.0, 50.0, 10.0, 10.0),
        (10.0, 7.0, 5.0, 10.0),
    ] {
        f.set_f32(OFS_PARM0, lo);
        f.set_f32(OFS_PARM1, v);
        f.set_f32(OFS_PARM2, hi);
        ext::pf_bound(&mut f.raw());
        assert_eq!(f.get_f32(OFS_RETURN), expect, "bound({lo}, {v}, {hi})");
    }
}

#[test]
fn ext_anglemod_is_the_subtract_loop_not_the_mathlib_quantisation() {
    let mut f = ext_fixture(b"\0");
    for (input, expect) in [
        (0.0f32, 0.0f32),
        (359.5, 359.5),
        (360.0, 0.0),
        (720.25, 0.25),
        (-1.0, 359.0),
        (-360.0, 0.0),
    ] {
        f.set_f32(OFS_PARM0, input);
        ext::pf_anglemod(&mut f.raw());
        assert_eq!(f.get_f32(OFS_RETURN), expect, "anglemod({input})");
    }
    // and it is exact where mathlib's quantises to 1/65536 of a turn
    f.set_f32(OFS_PARM0, 359.5);
    ext::pf_anglemod(&mut f.raw());
    assert_ne!(f.get_f32(OFS_RETURN), builtins::anglemod(359.5));
}

#[test]
fn bitshift_converts_both_arguments_through_the_c_cast() {
    let mut f = ext_fixture(b"\0");
    for (mask, shift, expect) in [
        (1.0f32, 4.0f32, 16i32),
        (256.0, -4.0, 16),
        (-1.0, 1.0, -2),
        (-16.0, -2.0, -4), // arithmetic shift: the sign is kept
        (3.9, 1.9, 6),     // both truncate before the shift
    ] {
        f.set_f32(OFS_PARM0, mask);
        f.set_f32(OFS_PARM1, shift);
        ext::pf_bitshift(&mut f.raw());
        assert_eq!(
            f.get_f32(OFS_RETURN),
            expect as f32,
            "bitshift({mask}, {shift})"
        );
    }
}

#[test]
fn mod_uses_a_truncated_quotient_and_warns_on_zero() {
    let mut f = ext_fixture(b"\0");
    for (a, n, expect) in [
        (7.0f32, 3.0f32, 1.0f32),
        (-7.0, 3.0, -1.0), // truncation toward zero, not floor
        (7.0, -3.0, 1.0),
        (7.5, 2.0, 1.5), // "inherantly floaty": the remainder keeps a fraction
    ] {
        f.set_f32(OFS_PARM0, a);
        f.set_f32(OFS_PARM1, n);
        let mut sys = core::mem::take(&mut f.sys);
        ext::pf_mod(&mut f.raw(), &mut sys);
        assert_eq!(f.get_f32(OFS_RETURN), expect, "mod({a}, {n})");
        f.sys = sys;
    }

    f.set_f32(OFS_PARM1, 0.0);
    let mut sys = core::mem::take(&mut f.sys);
    ext::pf_mod(&mut f.raw(), &mut sys);
    assert_eq!(f.get_f32(OFS_RETURN), 0.0, "mod by zero is 0, not NaN");
    assert_eq!(sys.dprinted, vec![b"PF_mod: mod by zero\n".to_vec()]);
}

#[test]
fn crossproduct_matches_the_mathlib_expansion() {
    let mut f = ext_fixture(b"\0");
    f.set_vec3(OFS_PARM0, [1.0, 0.0, 0.0]);
    f.set_vec3(OFS_PARM1, [0.0, 1.0, 0.0]);
    ext::pf_crossproduct(&mut f.raw());
    assert_eq!(
        [
            f.get_f32(OFS_RETURN),
            f.get_f32(OFS_RETURN + 1),
            f.get_f32(OFS_RETURN + 2)
        ],
        [0.0, 0.0, 1.0]
    );
}

#[test]
fn vectoangles2_negates_the_pitch() {
    let mut f = ext_fixture(b"\0");
    f.vm.argc = 2;
    f.set_vec3(OFS_PARM0, [1.0, 0.0, 0.0]);
    f.set_vec3(OFS_PARM1, [7.0, 8.0, 9.0]);
    let mut sys = core::mem::take(&mut f.sys);
    ext::pf_ext_vectoangles(&mut f.raw(), &mut sys);
    // the mock's VectorAngles returns `up` verbatim, so the negation is what
    // is being observed here, not the mathlib result
    assert_eq!(f.get_f32(OFS_RETURN), -7.0);
    assert_eq!(f.get_f32(OFS_RETURN + 1), 8.0);
    assert_eq!(f.get_f32(OFS_RETURN + 2), 9.0);
}

#[test]
fn itos_and_htos_match_the_platform_snprintf() {
    let mut f = ext_fixture(b"\0");
    for v in [0i32, 1, -1, i32::MIN, i32::MAX, 255, -255] {
        f.set_i32(OFS_PARM0, v);
        let mut sys = core::mem::take(&mut f.sys);
        ext::pf_itos(&mut f.raw(), &mut sys);
        assert_eq!(
            String::from_utf8_lossy(&sys.temp_strings[0]),
            c_snprintf_i32("%i", v),
            "itos({v})"
        );

        let mut sys2 = MockSys::default();
        ext::pf_htos(&mut f.raw(), &mut sys2);
        assert_eq!(
            String::from_utf8_lossy(&sys2.temp_strings[0]),
            quake_ctest::c_snprintf_u32("%x", v as u32),
            "htos({v})"
        );
    }
}

#[test]
fn ftoe_range_checks_against_max_edicts_not_num_edicts() {
    // EDICT_NUM's test is `n < 0 || n >= qcvm->max_edicts`, and it raises in
    // release builds. num_edicts is deliberately lower here.
    let mut f = ext_fixture(b"\0");
    f.vm.num_edicts = 3;
    let stride = f.vm.edict_size;

    f.set_f32(OFS_PARM0, 10.0);
    assert_eq!(ext::pf_edict_for_num(&mut f.raw()), Ok(()));
    assert_eq!(f.get_i32(OFS_RETURN), stride * 10, "10 < max_edicts");

    for bad in [-1.0f32, MAX_EDICTS as f32, 1e9] {
        f.set_f32(OFS_PARM0, bad);
        assert!(matches!(
            ext::pf_edict_for_num(&mut f.raw()),
            Err(BuiltinError::BadEdictNum(_))
        ));
    }
}

#[test]
fn str2chr_never_rejects_index_zero_and_reads_the_terminator() {
    // COMPAT: the test is `ofs && (ofs < 0 || ofs > strlen)`, so index 0
    // bypasses it entirely and an index equal to the length reads the NUL.
    let mut blob = vec![0u8];
    blob.extend_from_slice(b"abc\0");
    let mut f = ext_fixture(&blob);
    f.vm.argc = 2;
    f.set_i32(OFS_PARM0, 1);

    for (idx, expect) in [
        (0.0f32, f32::from(b'a')),
        (1.0, f32::from(b'b')),
        (2.0, f32::from(b'c')),
        (3.0, 0.0), // exactly the length: the NUL, not the error arm
        (4.0, 0.0), // past it: the error arm, same value
        (-1.0, f32::from(b'c')),
        (-3.0, f32::from(b'a')),
    ] {
        f.set_f32(OFS_PARM1, idx);
        ext::pf_str2chr(&mut f.raw()).unwrap();
        assert_eq!(f.get_f32(OFS_RETURN), expect, "str2chr(\"abc\", {idx})");
    }

    // and on an empty string index 0 reads the terminator rather than failing
    let mut f = ext_fixture(b"\0");
    f.vm.argc = 1;
    f.set_i32(OFS_PARM0, 0);
    ext::pf_str2chr(&mut f.raw()).unwrap();
    assert_eq!(f.get_f32(OFS_RETURN), 0.0);
}

#[test]
fn substring_handles_negative_start_and_length_the_way_c_does() {
    let mut blob = vec![0u8];
    blob.extend_from_slice(b"abcdef\0");
    let mut f = ext_fixture(&blob);
    f.set_i32(OFS_PARM0, 1);

    for (start, length, expect) in [
        (0.0f32, 3.0f32, &b"abc"[..]),
        (2.0, 3.0, b"cde"),
        (2.0, 100.0, b"cdef"),
        (-2.0, 2.0, b"ef"),
        (-100.0, 3.0, b"abc"),  // start re-clamped to 0 after the adjustment
        (0.0, -1.0, b"abcdef"), // negative length: to the end
        (0.0, -3.0, b"abcd"),
        (10.0, 3.0, b""),   // start past the end
        (0.0, 0.0, b""),    // zero length
        (0.0, -100.0, b""), // length goes negative
    ] {
        f.set_f32(OFS_PARM1, start);
        f.set_f32(OFS_PARM2, length);
        let mut sys = core::mem::take(&mut f.sys);
        ext::pf_substring(&mut f.raw(), &mut sys).unwrap();
        assert_eq!(
            sys.temp_strings[0],
            expect.to_vec(),
            "substring(\"abcdef\", {start}, {length})"
        );
    }
}

#[test]
fn strncmp_offsets_only_the_first_string() {
    // COMPAT (bug preserved): C computes and clamps `bofs` and then never uses
    // it -- `strncmp (a + aofs, b, len)`.
    let mut blob = vec![0u8];
    blob.extend_from_slice(b"xxabc\0");
    let a_at = 1;
    blob.extend_from_slice(b"yyabc\0");
    let b_at = 7;
    let mut f = ext_fixture(&blob);
    f.set_i32(OFS_PARM0, a_at);
    f.set_i32(OFS_PARM1, b_at);
    f.vm.argc = 5;
    f.set_f32(OFS_PARM2, 3.0); // len
    f.set_f32(OFS_PARM0 + 9, 2.0); // aofs -> "abc"
    f.set_f32(OFS_PARM0 + 12, 2.0); // bofs -> would be "abc" if it were used

    let mut sys = core::mem::take(&mut f.sys);
    ext::pf_strncmp(&mut f.raw(), &mut sys).unwrap();
    assert_ne!(
        f.get_f32(OFS_RETURN),
        0.0,
        "if bofs were honoured this would compare abc to abc and be 0"
    );

    // with bofs 0 the comparison is "abc" vs "yya", still non-zero, and with
    // aofs 0 it is "xxa" vs "yya"
    f.set_f32(OFS_PARM0 + 9, 0.0);
    ext::pf_strncmp(&mut f.raw(), &mut sys).unwrap();
    assert_ne!(f.get_f32(OFS_RETURN), 0.0);

    // two-argument form falls through to a whole-string compare
    f.vm.argc = 2;
    f.set_i32(OFS_PARM1, a_at);
    ext::pf_strncmp(&mut f.raw(), &mut sys).unwrap();
    assert_eq!(f.get_f32(OFS_RETURN), 0.0);
}

#[test]
fn strncasecmp_folds_case_and_shares_the_offset_clamp() {
    let mut blob = vec![0u8];
    blob.extend_from_slice(b"ABC\0");
    blob.extend_from_slice(b"abc\0");
    let mut f = ext_fixture(&blob);
    f.set_i32(OFS_PARM0, 1);
    f.set_i32(OFS_PARM1, 5);
    f.vm.argc = 2;
    let mut sys = core::mem::take(&mut f.sys);
    ext::pf_strncasecmp(&mut f.raw(), &mut sys).unwrap();
    assert_eq!(f.get_f32(OFS_RETURN), 0.0);

    // the case-sensitive one does not fold
    ext::pf_strncmp(&mut f.raw(), &mut sys).unwrap();
    assert_ne!(f.get_f32(OFS_RETURN), 0.0);
}

#[test]
fn strstrofs_returns_a_byte_offset_or_minus_one() {
    let mut blob = vec![0u8];
    blob.extend_from_slice(b"hello world\0");
    blob.extend_from_slice(b"o w\0");
    blob.extend_from_slice(b"zzz\0");
    let mut f = ext_fixture(&blob);
    f.set_i32(OFS_PARM0, 1);

    f.set_i32(OFS_PARM1, 13);
    ext::pf_strstrofs(&mut f.raw()).unwrap();
    assert_eq!(f.get_f32(OFS_RETURN), 4.0);

    f.set_i32(OFS_PARM1, 17);
    ext::pf_strstrofs(&mut f.raw()).unwrap();
    assert_eq!(f.get_f32(OFS_RETURN), -1.0);

    // a start past the end reports -1; a start of exactly the length does not
    f.vm.argc = 3;
    f.set_i32(OFS_PARM1, 13);
    f.set_f32(OFS_PARM2, 100.0);
    ext::pf_strstrofs(&mut f.raw()).unwrap();
    assert_eq!(f.get_f32(OFS_RETURN), -1.0);
    f.set_f32(OFS_PARM2, 5.0);
    ext::pf_strstrofs(&mut f.raw()).unwrap();
    assert_eq!(f.get_f32(OFS_RETURN), -1.0, "the match is before the start");
}

#[test]
fn strtrim_strips_only_the_four_c_whitespace_bytes() {
    let mut blob = vec![0u8];
    blob.extend_from_slice(b" \t\r\nabc \t\r\n\0");
    blob.extend_from_slice(b"\x0b\x0cabc\x0b\x0c\0");
    let mut f = ext_fixture(&blob);

    f.set_i32(OFS_PARM0, 1);
    let mut sys = core::mem::take(&mut f.sys);
    ext::pf_strtrim(&mut f.raw(), &mut sys).unwrap();
    assert_eq!(sys.temp_strings[0], b"abc".to_vec());

    // vertical tab and form feed are q_isspace but are NOT in C's list here
    f.set_i32(OFS_PARM0, 13);
    let mut sys = MockSys::default();
    ext::pf_strtrim(&mut f.raw(), &mut sys).unwrap();
    assert_eq!(sys.temp_strings[0], b"\x0b\x0cabc\x0b\x0c".to_vec());
}

#[test]
fn strreplace_works_and_strireplace_is_crippled_by_the_sizeof_bug() {
    // COMPAT: pr_ext.c's strireplace bounds its loop with
    // `sizeof (resultbuf)` where resultbuf is a `char *`, so the capacity is
    // 8 bytes, not STRINGTEMP_LENGTH. strreplace next door uses the constant.
    let mut blob = vec![0u8];
    blob.extend_from_slice(b"a\0"); // search, at 1
    blob.extend_from_slice(b"X\0"); // replace, at 3
    blob.extend_from_slice(b"aaaaaaaaaaaaaaaa\0"); // subject, at 5
    let mut f = ext_fixture(&blob);
    f.set_i32(OFS_PARM0, 1);
    f.set_i32(OFS_PARM1, 3);
    f.set_i32(OFS_PARM2, 5);

    let mut sys = core::mem::take(&mut f.sys);
    ext::pf_strreplace(&mut f.raw(), &mut sys).unwrap();
    assert_eq!(sys.temp_strings[0], b"XXXXXXXXXXXXXXXX".to_vec());

    let mut sys = MockSys::default();
    ext::pf_strireplace(&mut f.raw(), &mut sys).unwrap();
    assert_eq!(
        sys.temp_strings[0],
        b"XXXXX".to_vec(),
        "8 - replacelen(1) - 2 = 5 bytes, and no more"
    );

    // with a six-byte replacement the case-insensitive variant produces
    // nothing at all
    blob.extend_from_slice(b"ABCDEF\0");
    let mut f = ext_fixture(&blob);
    f.set_i32(OFS_PARM0, 1);
    f.set_i32(OFS_PARM1, 22);
    f.set_i32(OFS_PARM2, 5);
    let mut sys = core::mem::take(&mut f.sys);
    ext::pf_strireplace(&mut f.raw(), &mut sys).unwrap();
    assert!(sys.temp_strings[0].is_empty());
}

#[test]
fn replace_with_an_empty_search_returns_the_subjects_own_handle() {
    // C hands PR_SetEngineString the subject pointer, which resolves back to
    // the same handle -- not a fresh temp string.
    let mut blob = vec![0u8];
    blob.extend_from_slice(b"\0"); // empty search at 1
    blob.extend_from_slice(b"X\0");
    blob.extend_from_slice(b"subject\0");
    let subject_at = 4;
    let mut f = ext_fixture(&blob);
    f.set_i32(OFS_PARM0, 1);
    f.set_i32(OFS_PARM1, 2);
    f.set_i32(OFS_PARM2, subject_at);

    let mut sys = core::mem::take(&mut f.sys);
    ext::pf_strreplace(&mut f.raw(), &mut sys).unwrap();
    assert_eq!(f.get_i32(OFS_RETURN), subject_at);
    assert!(sys.temp_strings.is_empty(), "no temp string is consumed");
}

#[test]
fn case_conversion_is_ascii_only_so_the_quake_charset_survives() {
    let mut blob = vec![0u8];
    blob.extend_from_slice(b"aBc\xe1\xc1\0");
    let mut f = ext_fixture(&blob);
    f.set_i32(OFS_PARM0, 1);

    let mut sys = core::mem::take(&mut f.sys);
    ext::pf_strtoupper(&mut f.raw(), &mut sys).unwrap();
    assert_eq!(sys.temp_strings[0], b"ABC\xe1\xc1".to_vec());

    let mut sys = MockSys::default();
    ext::pf_strtolower(&mut f.raw(), &mut sys).unwrap();
    assert_eq!(sys.temp_strings[0], b"abc\xe1\xc1".to_vec());
}

#[test]
fn chr2str_stops_six_bytes_short_of_the_buffer() {
    let mut f = ext_fixture(b"\0");
    f.vm.argc = 4;
    for (i, v) in [f32::from(b'a'), 0xe041 as f32, 0x20ac as f32, 200.0]
        .into_iter()
        .enumerate()
    {
        f.set_f32(OFS_PARM0 + i * 3, v);
    }
    let mut sys = core::mem::take(&mut f.sys);
    ext::pf_chr2str(&mut f.raw(), &mut sys);
    assert_eq!(
        sys.temp_strings[0],
        b"aA?\xc8".to_vec(),
        "0xe041 is the Quake charset; U+20AC is over 255 so it becomes '?'; \
         200 passes because pr_ext's qc_isascii is `u < 256`, not q_isascii"
    );
}

#[test]
fn strcat_truncates_and_warns_rather_than_dropping_everything() {
    let long = vec![b'x'; 600];
    let mut blob = vec![0u8];
    blob.extend_from_slice(&long);
    blob.push(0);
    let mut f = ext_fixture(&blob);
    f.vm.argc = 3;
    f.set_i32(OFS_PARM0, 1);
    f.set_i32(OFS_PARM0 + 3, 1);
    f.set_i32(OFS_PARM0 + 6, 1);

    let mut sys = core::mem::take(&mut f.sys);
    ext::pf_strcat(&mut f.raw(), &mut sys).unwrap();
    assert_eq!(
        sys.temp_strings[0].len(),
        builtins::STRINGTEMP_LENGTH - 1,
        "the first overflowing append still fills the buffer"
    );
    assert_eq!(
        sys.printed,
        vec![b"PF_strcat: overflow (string truncated)\n".to_vec()]
    );
}

#[test]
fn strpad_pads_left_for_a_negative_width_and_never_truncates_the_source() {
    let mut blob = vec![0u8];
    blob.extend_from_slice(b"ab\0");
    let mut f = ext_fixture(&blob);
    f.sys.var_string = b"ab".to_vec();

    f.set_f32(OFS_PARM0, 5.0);
    let mut sys = core::mem::take(&mut f.sys);
    ext::pf_strpad(&mut f.raw(), &mut sys);
    assert_eq!(sys.temp_strings[0], b"ab   ".to_vec());

    f.set_f32(OFS_PARM0, -5.0);
    let mut sys2 = MockSys {
        var_string: b"ab".to_vec(),
        ..MockSys::default()
    };
    ext::pf_strpad(&mut f.raw(), &mut sys2);
    assert_eq!(sys2.temp_strings[0], b"   ab".to_vec());

    // a source longer than the field simply gets no padding
    f.set_f32(OFS_PARM0, 2.0);
    let mut sys3 = MockSys {
        var_string: b"abcdef".to_vec(),
        ..MockSys::default()
    };
    ext::pf_strpad(&mut f.raw(), &mut sys3);
    assert_eq!(sys3.temp_strings[0], b"abcdef".to_vec());
}

/// Keeps the fixture's backing buffers alive for the whole test, which is what
/// makes the raw `qcvm_t` pointers valid.
#[test]
fn fixture_buffers_outlive_the_views() {
    let f = Fixture::new(b"\0abc\0");
    assert_eq!(f.strings.len(), 5);
    assert!(!f.edicts.is_empty() && !f.globals.is_empty());
}
