//! C-printf-compatible formatter.
//!
//! // COMPAT: ADR-005 — savegames, config.cfg and console output are written
//! with C printf semantics and the compatibility bar is byte-identical output.
//! Rust's `format!` (shortest-roundtrip floats) must never be used for
//! compat-relevant output; ported writers call this module instead.
//!
//! Covered conversions (the set the engine actually uses): `%f`/`%F` with
//! flags/width/precision, `%d`/`%i`/`%u`/`%x`/`%X` (with `l`/`ll`/`z` length
//! modifiers for 64-bit args), `%s`, `%c`, `%%`. `%g`/`%e` have no user in the
//! ported code and are deliberately not implemented (panics; adding them
//! requires extending the conformance suite first).
//!
//! `%f` produces the exact decimal expansion of the binary double with
//! IEEE-754 round-to-nearest, ties-to-even at the requested precision — this
//! matches glibc, Apple libc and UCRT (all correctly-rounding since C99/VS2015;
//! the conformance suite in quake-ctest is the arbiter, per-platform).
//!
//! inf/NaN spellings differ per platform and are `cfg(target_os)`-gated:
//! Apple libc never prints a sign or honors +/space for NaN; glibc treats NaN
//! like a signed value; UCRT distinguishes the indeterminate form
//! (`-nan(ind)`) and signaling NaNs (`nan(snan)`). The Windows table is
//! validated by the conformance CI job.
//!
//! Panics on malformed/unsupported format strings or argument-type mismatches:
//! format strings in the engine are compile-time constants, so these are
//! programming errors (ADR-009: panics indicate engine bugs).

/// A typed vararg. C's default argument promotions are the caller's job:
/// `float` promotes to `F64`, `char`/`short` to `I32`.
#[derive(Debug, Clone, Copy)]
pub enum Arg<'a> {
    I32(i32),
    U32(u32),
    I64(i64),
    U64(u64),
    F64(f64),
    Str(&'a [u8]),
}

#[derive(Debug, Default, Clone, Copy)]
struct Flags {
    minus: bool,
    plus: bool,
    space: bool,
    zero: bool,
    alt: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Length {
    None,
    Long,
    LongLong,
    Size,
}

/// Format `fmt` (a C printf format string, without the trailing NUL) with
/// `args`. Returns the formatted bytes.
pub fn format(fmt: &[u8], args: &[Arg]) -> Vec<u8> {
    let mut out = Vec::with_capacity(fmt.len() + 16);
    let mut rest = fmt;
    let mut args = args.iter();

    while let Some(pos) = rest.iter().position(|&b| b == b'%') {
        out.extend_from_slice(&rest[..pos]);
        rest = &rest[pos + 1..];

        if rest.first() == Some(&b'%') {
            out.push(b'%');
            rest = &rest[1..];
            continue;
        }

        // flags
        let mut flags = Flags::default();
        loop {
            match rest.first() {
                Some(b'-') => flags.minus = true,
                Some(b'+') => flags.plus = true,
                Some(b' ') => flags.space = true,
                Some(b'0') => flags.zero = true,
                Some(b'#') => flags.alt = true,
                _ => break,
            }
            rest = &rest[1..];
        }
        // width
        let mut width = 0usize;
        while let Some(&d @ b'0'..=b'9') = rest.first() {
            width = width * 10 + (d - b'0') as usize;
            rest = &rest[1..];
        }
        // precision
        let mut precision = None;
        if rest.first() == Some(&b'.') {
            rest = &rest[1..];
            let mut p = 0usize;
            while let Some(&d @ b'0'..=b'9') = rest.first() {
                p = p * 10 + (d - b'0') as usize;
                rest = &rest[1..];
            }
            precision = Some(p);
        }
        // length modifier
        let length = match rest.first() {
            Some(b'l') => {
                rest = &rest[1..];
                if rest.first() == Some(&b'l') {
                    rest = &rest[1..];
                    Length::LongLong
                } else {
                    Length::Long
                }
            }
            Some(b'z') => {
                rest = &rest[1..];
                Length::Size
            }
            Some(b'h') => panic!("printf: 'h' length modifier unsupported (no engine user)"),
            _ => Length::None,
        };

        let conv = *rest
            .first()
            .expect("printf: format string ends inside a conversion");
        rest = &rest[1..];

        let mut arg = || *args.next().expect("printf: not enough arguments");
        match conv {
            b'd' | b'i' => {
                let v = int_arg_signed(arg(), length);
                pad_number(
                    &mut out,
                    &fmt_int(v.unsigned_abs(), 10, false, precision),
                    v < 0,
                    flags,
                    width,
                    precision,
                );
            }
            b'u' => {
                let v = int_arg_unsigned(arg(), length);
                pad_number(
                    &mut out,
                    &fmt_int(v, 10, false, precision),
                    false,
                    no_sign(flags),
                    width,
                    precision,
                );
            }
            b'x' => {
                let v = int_arg_unsigned(arg(), length);
                let mut f = no_sign(flags);
                f.alt = flags.alt && v != 0;
                pad_hex(
                    &mut out,
                    &fmt_int(v, 16, false, precision),
                    f,
                    width,
                    precision,
                    false,
                );
            }
            b'X' => {
                let v = int_arg_unsigned(arg(), length);
                let mut f = no_sign(flags);
                f.alt = flags.alt && v != 0;
                pad_hex(
                    &mut out,
                    &fmt_int(v, 16, true, precision),
                    f,
                    width,
                    precision,
                    true,
                );
            }
            b'f' | b'F' => {
                let v = match arg() {
                    Arg::F64(v) => v,
                    other => panic!("printf: %f expects F64, got {other:?}"),
                };
                fmt_float(
                    &mut out,
                    v,
                    flags,
                    width,
                    precision.unwrap_or(6),
                    conv == b'F',
                );
            }
            b's' => {
                let s = match arg() {
                    Arg::Str(s) => s,
                    other => panic!("printf: %s expects Str, got {other:?}"),
                };
                let s = match precision {
                    Some(p) if p < s.len() => &s[..p],
                    _ => s,
                };
                pad_bytes(&mut out, s, flags, width);
            }
            b'c' => {
                let v = match arg() {
                    Arg::I32(v) => v,
                    other => panic!("printf: %c expects I32, got {other:?}"),
                };
                pad_bytes(&mut out, &[v as u8], flags, width);
            }
            other => panic!("printf: conversion '%{}' unsupported", other as char),
        }
    }
    out.extend_from_slice(rest);
    out
}

fn no_sign(mut f: Flags) -> Flags {
    // C ignores '+'/' ' for unsigned conversions
    f.plus = false;
    f.space = false;
    f
}

fn int_arg_signed(arg: Arg, length: Length) -> i64 {
    match (arg, length) {
        (Arg::I32(v), Length::None | Length::Long) => v as i64,
        (Arg::I64(v), Length::Long | Length::LongLong | Length::Size) => v,
        (a, l) => panic!("printf: signed int conversion with modifier {l:?} got {a:?}"),
    }
}

fn int_arg_unsigned(arg: Arg, length: Length) -> u64 {
    match (arg, length) {
        (Arg::U32(v), Length::None | Length::Long) => v as u64,
        (Arg::U64(v), Length::Long | Length::LongLong | Length::Size) => v,
        (a, l) => panic!("printf: unsigned int conversion with modifier {l:?} got {a:?}"),
    }
}

/// Digits of `v` in `base`, at least `precision` digits (zero-padded); the
/// C rule: value 0 with explicit precision 0 prints no digits.
fn fmt_int(v: u64, base: u64, upper: bool, precision: Option<usize>) -> Vec<u8> {
    let mut digits = Vec::new();
    let mut v = v;
    while v > 0 {
        let d = (v % base) as u8;
        digits.push(match d {
            0..=9 => b'0' + d,
            _ if upper => b'A' + d - 10,
            _ => b'a' + d - 10,
        });
        v /= base;
    }
    if digits.is_empty() && precision != Some(0) {
        digits.push(b'0');
    }
    while digits.len() < precision.unwrap_or(0) {
        digits.push(b'0');
    }
    digits.reverse();
    digits
}

/// Common width/sign/zero-pad assembly for decimal integers.
fn pad_number(
    out: &mut Vec<u8>,
    digits: &[u8],
    negative: bool,
    flags: Flags,
    width: usize,
    precision: Option<usize>,
) {
    let sign: &[u8] = if negative {
        b"-"
    } else if flags.plus {
        b"+"
    } else if flags.space {
        b" "
    } else {
        b""
    };
    // '0' flag is ignored when a precision is given (C rule), and when '-' set
    let zero_pad = flags.zero && !flags.minus && precision.is_none();
    let content = sign.len() + digits.len();
    let pad = width.saturating_sub(content);
    if flags.minus {
        out.extend_from_slice(sign);
        out.extend_from_slice(digits);
        out.extend(std::iter::repeat_n(b' ', pad));
    } else if zero_pad {
        out.extend_from_slice(sign);
        out.extend(std::iter::repeat_n(b'0', pad));
        out.extend_from_slice(digits);
    } else {
        out.extend(std::iter::repeat_n(b' ', pad));
        out.extend_from_slice(sign);
        out.extend_from_slice(digits);
    }
}

/// Hex assembly: like pad_number but with an optional 0x/0X prefix instead of
/// a sign (flags.alt has already been cleared for zero values).
fn pad_hex(
    out: &mut Vec<u8>,
    digits: &[u8],
    flags: Flags,
    width: usize,
    precision: Option<usize>,
    upper: bool,
) {
    let prefix: &[u8] = if flags.alt {
        if upper {
            b"0X"
        } else {
            b"0x"
        }
    } else {
        b""
    };
    let zero_pad = flags.zero && !flags.minus && precision.is_none();
    let content = prefix.len() + digits.len();
    let pad = width.saturating_sub(content);
    if flags.minus {
        out.extend_from_slice(prefix);
        out.extend_from_slice(digits);
        out.extend(std::iter::repeat_n(b' ', pad));
    } else if zero_pad {
        out.extend_from_slice(prefix);
        out.extend(std::iter::repeat_n(b'0', pad));
        out.extend_from_slice(digits);
    } else {
        out.extend(std::iter::repeat_n(b' ', pad));
        out.extend_from_slice(prefix);
        out.extend_from_slice(digits);
    }
}

fn pad_bytes(out: &mut Vec<u8>, s: &[u8], flags: Flags, width: usize) {
    let pad = width.saturating_sub(s.len());
    if flags.minus {
        out.extend_from_slice(s);
        out.extend(std::iter::repeat_n(b' ', pad));
    } else {
        out.extend(std::iter::repeat_n(b' ', pad));
        out.extend_from_slice(s);
    }
}

// ---------------------------------------------------------------------------
// %f: exact fixed-point decimal expansion of a binary64
// ---------------------------------------------------------------------------

/// Fixed-capacity little-endian bignum. 18 × 64 = 1152 bits, enough for
/// 10 × 2^1074 (the largest intermediate: a subnormal's fraction × 10).
#[derive(Clone, Copy)]
struct Big {
    limbs: [u64; 18],
    len: usize,
}

impl Big {
    fn from_u64(v: u64) -> Self {
        let mut b = Big {
            limbs: [0; 18],
            len: 0,
        };
        b.limbs[0] = v;
        b.len = usize::from(v != 0);
        b
    }

    fn is_zero(&self) -> bool {
        self.len == 0
    }

    fn shl(&mut self, bits: usize) {
        if self.is_zero() || bits == 0 {
            return;
        }
        let limb_shift = bits / 64;
        let bit_shift = bits % 64;
        let mut new = [0u64; 18];
        for i in (0..self.len).rev() {
            let v = self.limbs[i];
            new[i + limb_shift] |= v << bit_shift;
            if bit_shift != 0 && i + limb_shift + 1 < 18 {
                new[i + limb_shift + 1] |= v >> (64 - bit_shift);
            }
        }
        self.limbs = new;
        self.len = (self.len + limb_shift + 1).min(18);
        self.trim();
    }

    fn mul_small(&mut self, m: u64) {
        let mut carry = 0u128;
        for i in 0..self.len {
            let prod = self.limbs[i] as u128 * m as u128 + carry;
            self.limbs[i] = prod as u64;
            carry = prod >> 64;
        }
        while carry != 0 {
            self.limbs[self.len] = carry as u64;
            self.len += 1;
            carry = 0;
        }
    }

    /// self / 10 in place; returns the remainder digit.
    fn divmod10(&mut self) -> u8 {
        let mut rem = 0u64;
        for i in (0..self.len).rev() {
            let cur = ((rem as u128) << 64) | self.limbs[i] as u128;
            self.limbs[i] = (cur / 10) as u64;
            rem = (cur % 10) as u64;
        }
        self.trim();
        rem as u8
    }

    /// Extract bits at and above `shift` (must fit in u64), clearing them.
    fn extract_high(&mut self, shift: usize) -> u64 {
        let limb = shift / 64;
        let bit = shift % 64;
        let mut hi = 0u64;
        if limb < 18 {
            hi = self.limbs[limb] >> bit;
            if bit != 0 && limb + 1 < 18 {
                hi |= self.limbs[limb + 1] << (64 - bit);
            }
            // clear extracted bits
            if bit != 0 {
                self.limbs[limb] &= (1u64 << bit) - 1;
            } else {
                self.limbs[limb] = 0;
            }
            for l in self.limbs.iter_mut().take(self.len).skip(limb + 1) {
                *l = 0;
            }
        }
        self.trim();
        hi
    }

    /// Compare `2 * self` with `2^shift`.
    fn cmp2_pow2(&self, shift: usize) -> std::cmp::Ordering {
        // 2*self vs 2^shift  <=>  self vs 2^(shift-1)
        debug_assert!(shift >= 1);
        let target = shift - 1;
        let limb = target / 64;
        let bit = target % 64;
        for i in (0..self.len.max(limb + 1)).rev() {
            let v = self.limbs.get(i).copied().unwrap_or(0);
            let t = if i == limb { 1u64 << bit } else { 0 };
            if v != t {
                return v.cmp(&t);
            }
        }
        std::cmp::Ordering::Equal
    }

    fn trim(&mut self) {
        while self.len > 0 && self.limbs[self.len - 1] == 0 {
            self.len -= 1;
        }
    }

    /// Decimal digits, most significant first ("0" for zero).
    fn to_decimal(mut self) -> Vec<u8> {
        if self.is_zero() {
            return vec![b'0'];
        }
        let mut digits = Vec::new();
        while !self.is_zero() {
            digits.push(b'0' + self.divmod10());
        }
        digits.reverse();
        digits
    }
}

fn fmt_float(out: &mut Vec<u8>, v: f64, flags: Flags, width: usize, precision: usize, upper: bool) {
    let bits = v.to_bits();
    let negative = bits >> 63 == 1;
    let biased_exp = ((bits >> 52) & 0x7ff) as i32;
    let mantissa = bits & 0xf_ffff_ffff_ffff;

    if biased_exp == 0x7ff {
        fmt_nonfinite(out, negative, mantissa, flags, width, precision, upper);
        return;
    }

    // value = m53 * 2^e2
    let (m53, e2) = if biased_exp == 0 {
        (mantissa, -1074)
    } else {
        (mantissa | (1 << 52), biased_exp - 1075)
    };

    let mut int_part;
    let mut frac_digits = Vec::with_capacity(precision);
    let mut round_up = false;

    if e2 >= 0 {
        int_part = Big::from_u64(m53);
        int_part.shl(e2 as usize);
        frac_digits.resize(precision, b'0');
    } else {
        let shift = (-e2) as usize;
        int_part = if shift < 64 {
            Big::from_u64(m53 >> shift)
        } else {
            Big::from_u64(0)
        };
        let mut frac = Big::from_u64(m53);
        // keep only the fractional bits
        frac.extract_high(shift);
        for _ in 0..precision {
            frac.mul_small(10);
            let digit = frac.extract_high(shift);
            debug_assert!(digit < 10);
            frac_digits.push(b'0' + digit as u8);
        }
        // round to nearest, ties to even (correct rounding — matches the
        // platform C libraries; the conformance suite is the arbiter)
        if !frac.is_zero() {
            match frac.cmp2_pow2(shift) {
                std::cmp::Ordering::Greater => round_up = true,
                std::cmp::Ordering::Equal => {
                    let last = frac_digits.last().copied().unwrap_or_else(|| {
                        // tie with precision 0: parity comes from the integer part
                        *int_part.to_decimal().last().unwrap()
                    });
                    round_up = (last - b'0') % 2 == 1;
                }
                std::cmp::Ordering::Less => {}
            }
        }
    }

    let mut int_digits = int_part.to_decimal();
    if round_up {
        // propagate the carry through the fraction digits into the int part
        let mut carried = true;
        for d in frac_digits.iter_mut().rev() {
            if *d == b'9' {
                *d = b'0';
            } else {
                *d += 1;
                carried = false;
                break;
            }
        }
        if carried {
            for d in int_digits.iter_mut().rev() {
                if *d == b'9' {
                    *d = b'0';
                } else {
                    *d += 1;
                    carried = false;
                    break;
                }
            }
            if carried {
                int_digits.insert(0, b'1');
            }
        }
    }

    let mut body = int_digits;
    if precision > 0 || flags.alt {
        body.push(b'.');
        body.extend_from_slice(&frac_digits);
    }
    pad_number(out, &body, negative, flags, width, None);
}

/// UCRT %#.0f of a non-finite value: the decimal point is inserted at index
/// 1 of the sign-included string ("i.nf", "-.inf", "-.nan(ind)").
#[cfg(target_os = "windows")]
fn ucrt_hash_dot(out: &mut Vec<u8>, body: &[u8], negative: bool, flags: Flags, width: usize) {
    let mut signed: Vec<u8> = Vec::with_capacity(body.len() + 2);
    if negative {
        signed.push(b'-');
    } else if flags.plus {
        signed.push(b'+');
    } else if flags.space {
        signed.push(b' ');
    }
    signed.extend_from_slice(body);
    signed.insert(1, b'.');
    pad_bytes(out, &signed, flags, width);
}

fn fmt_nonfinite(
    out: &mut Vec<u8>,
    negative: bool,
    mantissa: u64,
    flags: Flags,
    width: usize,
    precision: usize,
    upper: bool,
) {
    // '0' flag is ignored for inf/nan on every platform
    let mut f = flags;
    f.zero = false;
    #[cfg(not(target_os = "windows"))]
    let _ = precision;

    if mantissa == 0 {
        let body: &[u8] = if upper { b"INF" } else { b"inf" };
        // UCRT's %#.0f forces the decimal point into index 1 of the *signed*
        // spelling: "i.nf", "-.inf" (observed on CI; per-platform per ADR-005)
        #[cfg(target_os = "windows")]
        if f.alt && precision == 0 {
            ucrt_hash_dot(out, body, negative, f, width);
            return;
        }
        pad_number(out, body, negative, f, width, None);
        return;
    }

    // NaN spellings are per-platform (validated by the conformance CI job):
    // - Apple libc: always "nan", never a sign, '+'/' ' ignored
    // - glibc: sign/'+'/' ' applied like a number, always "nan"
    // - UCRT: "-nan(ind)" for the indeterminate form (sign + quiet bit only),
    //   "nan(snan)" for signaling NaNs, plain "nan" otherwise
    #[cfg(target_os = "macos")]
    {
        let _ = mantissa;
        f.plus = false;
        f.space = false;
        let body: &[u8] = if upper { b"NAN" } else { b"nan" };
        pad_number(out, body, false, f, width, None);
    }
    #[cfg(target_os = "windows")]
    {
        let quiet = mantissa & (1 << 51) != 0;
        let body: Vec<u8> = if negative && mantissa == (1 << 51) {
            b"nan(ind)".to_vec()
        } else if !quiet {
            b"nan(snan)".to_vec()
        } else {
            b"nan".to_vec()
        };
        let body = if upper {
            body.to_ascii_uppercase()
        } else {
            body
        };
        // UCRT's %#.0f decimal-point insertion, as for inf: "-.nan(ind)"
        if f.alt && precision == 0 {
            ucrt_hash_dot(out, &body, negative, f, width);
            return;
        }
        pad_number(out, &body, negative, f, width, None);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = mantissa;
        let body: &[u8] = if upper { b"NAN" } else { b"nan" };
        pad_number(out, body, negative, f, width, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(fmt: &str, args: &[Arg]) -> String {
        String::from_utf8(format(fmt.as_bytes(), args)).unwrap()
    }

    #[test]
    fn basic_floats() {
        assert_eq!(f("%f", &[Arg::F64(0.0)]), "0.000000");
        assert_eq!(f("%f", &[Arg::F64(-0.0)]), "-0.000000");
        assert_eq!(f("%f", &[Arg::F64(1.5)]), "1.500000");
        assert_eq!(f("%f", &[Arg::F64(0.1)]), "0.100000");
        assert_eq!(f("%.20f", &[Arg::F64(0.1)]), "0.10000000000000000555");
        assert_eq!(f("%.0f", &[Arg::F64(0.5)]), "0");
        assert_eq!(f("%.0f", &[Arg::F64(1.5)]), "2");
        assert_eq!(f("%.0f", &[Arg::F64(2.5)]), "2");
        assert_eq!(f("%.1f", &[Arg::F64(0.25)]), "0.2");
        assert_eq!(f("%.1f", &[Arg::F64(0.75)]), "0.8");
        assert_eq!(f("%.0f", &[Arg::F64(-0.4)]), "-0");
        assert_eq!(f("%#.0f", &[Arg::F64(1.0)]), "1.");
        assert_eq!(f("%05.1f", &[Arg::F64(-3.2)]), "-03.2");
        assert_eq!(f("% 7.1f", &[Arg::F64(12.34)]), "   12.3");
        assert_eq!(f("% 5.0f  ", &[Arg::F64(3.0)]), "    3  ");
        assert_eq!(f("%.3f", &[Arg::F64(1e-10)]), "0.000");
        assert_eq!(f("%f", &[Arg::F64(1e21)]), "1000000000000000000000.000000");
        assert_eq!(f("%.0f", &[Arg::F64(9.99)]), "10");
        assert_eq!(f("%.1f", &[Arg::F64(9.99)]), "10.0");
    }

    #[test]
    fn nonfinite() {
        assert_eq!(f("%f", &[Arg::F64(f64::INFINITY)]), "inf");
        assert_eq!(f("%f", &[Arg::F64(f64::NEG_INFINITY)]), "-inf");
        assert_eq!(f("%07.1f", &[Arg::F64(f64::NEG_INFINITY)]), "   -inf");
        assert_eq!(f("%+f", &[Arg::F64(f64::INFINITY)]), "+inf");
        assert_eq!(f("% f", &[Arg::F64(f64::INFINITY)]), " inf");
        #[cfg(target_os = "macos")]
        {
            assert_eq!(f("%f", &[Arg::F64(f64::NAN)]), "nan");
            assert_eq!(f("%f", &[Arg::F64(-f64::NAN)]), "nan");
            assert_eq!(f("%+f", &[Arg::F64(f64::NAN)]), "nan");
            assert_eq!(f("% 7.1f", &[Arg::F64(f64::NAN)]), "    nan");
        }
    }

    #[test]
    fn integers() {
        assert_eq!(f("%i", &[Arg::I32(-42)]), "-42");
        assert_eq!(f("%d", &[Arg::I32(0)]), "0");
        assert_eq!(f("%u", &[Arg::U32(4294967295)]), "4294967295");
        assert_eq!(f("%x", &[Arg::U32(0xdeadbeef)]), "deadbeef");
        assert_eq!(f("%X", &[Arg::U32(0xdeadbeef)]), "DEADBEEF");
        assert_eq!(f("%x", &[Arg::U32(0)]), "0");
        assert_eq!(f("%.0x", &[Arg::U32(0)]), "");
        assert_eq!(f("%5.3d", &[Arg::I32(42)]), "  042");
        assert_eq!(f("%-5d|", &[Arg::I32(-42)]), "-42  |");
        assert_eq!(f("%05d", &[Arg::I32(-42)]), "-0042");
        assert_eq!(f("% d", &[Arg::I32(42)]), " 42");
        assert_eq!(f("%+d", &[Arg::I32(42)]), "+42");
        assert_eq!(f("%lld", &[Arg::I64(i64::MIN)]), "-9223372036854775808");
        assert_eq!(f("%llu", &[Arg::U64(u64::MAX)]), "18446744073709551615");
        assert_eq!(f("%#x", &[Arg::U32(255)]), "0xff");
        assert_eq!(f("%#x", &[Arg::U32(0)]), "0");
    }

    #[test]
    fn strings_and_literals() {
        assert_eq!(f("hello %s!", &[Arg::Str(b"world")]), "hello world!");
        assert_eq!(f("%.3s", &[Arg::Str(b"abcdef")]), "abc");
        assert_eq!(f("%6s", &[Arg::Str(b"abc")]), "   abc");
        assert_eq!(f("%-6s|", &[Arg::Str(b"abc")]), "abc   |");
        assert_eq!(f("100%%", &[]), "100%");
        assert_eq!(
            f("\"%s\" \"%s\"\n", &[Arg::Str(b"a"), Arg::Str(b"b")]),
            "\"a\" \"b\"\n"
        );
    }

    #[test]
    fn savegame_shapes() {
        // the exact format strings the savegame/config writers use
        assert_eq!(
            f("%f %f %f", &[Arg::F64(1.0), Arg::F64(-2.5), Arg::F64(0.0)]),
            "1.000000 -2.500000 0.000000"
        );
        assert_eq!(f("%i\n", &[Arg::I32(5)]), "5\n");
        assert_eq!(
            f("spawnparm %i \"%f\"\n", &[Arg::I32(1), Arg::F64(0.5)]),
            "spawnparm 1 \"0.500000\"\n"
        );
    }
}
