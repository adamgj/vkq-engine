//! ADR-005 formatter conformance: quake_util::printf vs the platform
//! snprintf, on a stratified sample of f32/f64 bit patterns plus integer and
//! string spec coverage. The scheduled exhaustive 2^32 f32 sweep lives in
//! src/bin/snprintf_sweep.rs.
// The c_ref_* symbols are compiled C (build.rs), which Miri cannot execute;
// the shims themselves get Miri coverage in miri_capi.rs instead.
#![cfg(not(miri))]

use quake_ctest::*;
use quake_util::printf::{format, Arg};

fn rust_fmt(fmt: &str, arg: Arg) -> String {
    String::from_utf8(format(fmt.as_bytes(), &[arg])).unwrap()
}

/// Deterministic 64-bit LCG (fixed seed): reproducible sampling layer.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }
}

const FLOAT_SPECS: &[&str] = &[
    // the exact shapes the engine writers use (pr_edict.c, host_cmd.c, cvar.c)
    "%f",
    "% 5.0f  ",
    "% 7.1f",
    "% 13.0f  ",
    "% 15.1f",
    // spec-matrix coverage: flags x width x precision
    "%.0f",
    "%.1f",
    "%.2f",
    "%.3f",
    "%.9f",
    "%.17f",
    "%08.3f",
    "%-12.4f|",
    "%+f",
    "%#.0f",
    "%20f",
    "%020f",
    "% .5f",
];

fn check_f64(v: f64) {
    for spec in FLOAT_SPECS {
        let c = c_snprintf_f(spec, v);
        let r = rust_fmt(spec, Arg::F64(v));
        assert_eq!(
            r,
            c,
            "mismatch for {spec:?} of {v:?} (bits {:#018x})",
            v.to_bits()
        );
    }
}

#[test]
fn f32_stratified_sweep() {
    // every exponent x {edge, pattern, random} mantissas x both signs
    let mut lcg = Lcg(0x51c4a11e5);
    for exp in 0u32..=255 {
        let mut mantissas = vec![0u32, 1, 0x7fffff, 0x400000, 0x555555, 0x2aaaaa];
        for _ in 0..10 {
            mantissas.push((lcg.next() as u32) & 0x7fffff);
        }
        for m in mantissas {
            for sign in [0u32, 1] {
                let bits = (sign << 31) | (exp << 23) | m;
                check_f64(f32::from_bits(bits) as f64);
            }
        }
    }
}

#[test]
fn f64_stratified_sweep() {
    let mut lcg = Lcg(0xdefaced);
    // decimal-boundary neighborhoods, ties, subnormals, extremes
    let interesting: &[f64] = &[
        0.0,
        -0.0,
        0.5,
        1.5,
        2.5,
        0.25,
        0.75,
        0.05,
        0.15,
        0.1,
        1.0 / 3.0,
        9.9999995,
        0.9999999999,
        f64::MIN_POSITIVE,
        5e-324,
        1e-310,
        f64::MAX,
        1e308,
        1e21,
        4503599627370496.5,
        123456.789,
        -3.2,
        604.25,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ];
    for &v in interesting {
        check_f64(v);
    }
    // random f64 bit patterns (finite and not)
    for _ in 0..20000 {
        let v = f64::from_bits(lcg.next());
        check_f64(v);
    }
    // random values in the ranges savegames actually contain
    for _ in 0..20000 {
        let v = ((lcg.next() % 8_000_000) as f64 - 4_000_000.0) / 1000.0;
        check_f64(v);
    }
}

#[test]
fn nan_payloads() {
    // canonical, negative, payload and signaling forms; per-platform
    // spellings are encoded in the formatter and validated here
    let patterns: &[u64] = &[
        0x7ff8000000000000, // +qNaN canonical
        0xfff8000000000000, // -qNaN canonical (UCRT: -nan(ind))
        0x7ff8000000000001,
        0xfff800000000cafe,
        0x7ff0000000000001, // +sNaN
        0xfff0000000000001, // -sNaN
        0x7fffffffffffffff,
        0xffffffffffffffff,
    ];
    for &bits in patterns {
        check_f64(f64::from_bits(bits));
    }
}

#[test]
fn integer_conformance() {
    let mut lcg = Lcg(0x1234_5678);

    let i32_specs = [
        "%d", "%i", "%5d", "%-5d|", "%05d", "% d", "%+d", "%5.3d", "%.0d", "%c",
    ];
    let i32_edges = [0i32, 1, -1, 42, -42, i32::MIN, i32::MAX];
    for spec in i32_specs {
        if spec == "%c" {
            // %c only makes sense for printable bytes
            for v in [32i32, 65, 126] {
                assert_eq!(
                    rust_fmt(spec, Arg::I32(v)),
                    c_snprintf_i32(spec, v),
                    "spec {spec:?} of {v}"
                );
            }
            continue;
        }
        for v in i32_edges {
            assert_eq!(
                rust_fmt(spec, Arg::I32(v)),
                c_snprintf_i32(spec, v),
                "spec {spec:?} of {v}"
            );
        }
        for _ in 0..2000 {
            let v = lcg.next() as i32;
            assert_eq!(
                rust_fmt(spec, Arg::I32(v)),
                c_snprintf_i32(spec, v),
                "spec {spec:?} of {v}"
            );
        }
    }

    let u32_specs = [
        "%u", "%x", "%X", "%8x", "%08x", "%#x", "%#010x", "%.0x", "%-9u|",
    ];
    let u32_edges = [0u32, 1, 0xdeadbeef, u32::MAX];
    for spec in u32_specs {
        for v in u32_edges {
            assert_eq!(
                rust_fmt(spec, Arg::U32(v)),
                c_snprintf_u32(spec, v),
                "spec {spec:?} of {v}"
            );
        }
        for _ in 0..2000 {
            let v = lcg.next() as u32;
            assert_eq!(
                rust_fmt(spec, Arg::U32(v)),
                c_snprintf_u32(spec, v),
                "spec {spec:?} of {v}"
            );
        }
    }

    let i64_specs = ["%lld", "%lli", "%20lld", "%-20lld|", "%+lld"];
    for spec in i64_specs {
        for v in [0i64, 1, -1, i64::MIN, i64::MAX] {
            assert_eq!(
                rust_fmt(spec, Arg::I64(v)),
                c_snprintf_i64(spec, v),
                "spec {spec:?} of {v}"
            );
        }
        for _ in 0..2000 {
            let v = lcg.next() as i64;
            assert_eq!(
                rust_fmt(spec, Arg::I64(v)),
                c_snprintf_i64(spec, v),
                "spec {spec:?} of {v}"
            );
        }
    }

    let u64_specs = ["%llu", "%llx", "%llX", "%020llu"];
    for spec in u64_specs {
        for v in [0u64, 1, u64::MAX] {
            assert_eq!(
                rust_fmt(spec, Arg::U64(v)),
                c_snprintf_u64(spec, v),
                "spec {spec:?} of {v}"
            );
        }
        for _ in 0..2000 {
            let v = lcg.next();
            assert_eq!(
                rust_fmt(spec, Arg::U64(v)),
                c_snprintf_u64(spec, v),
                "spec {spec:?} of {v}"
            );
        }
    }
}

#[test]
fn string_conformance() {
    let specs = ["%s", "%.3s", "%.0s", "%6s", "%-6s|", "%10.5s"];
    let values: &[&[u8]] = &[b"", b"a", b"abc", b"abcdefghij", b"with space", b"\x01\x7f"];
    for spec in specs {
        for &v in values {
            assert_eq!(
                rust_fmt(spec, Arg::Str(v)),
                c_snprintf_str(spec, v),
                "spec {spec:?} of {v:?}"
            );
        }
    }
}
