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
use quake_progs::builtins::{self, BuiltinError, BuiltinSys};
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

    fn print(&mut self, msg: &[u8]) {
        self.printed.push(msg.to_vec());
    }

    fn dprint(&mut self, msg: &[u8]) {
        self.dprinted.push(msg.to_vec());
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

/// Keeps the fixture's backing buffers alive for the whole test, which is what
/// makes the raw `qcvm_t` pointers valid.
#[test]
fn fixture_buffers_outlive_the_views() {
    let f = Fixture::new(b"\0abc\0");
    assert_eq!(f.strings.len(), 5);
    assert!(!f.edicts.is_empty() && !f.globals.is_empty());
}
