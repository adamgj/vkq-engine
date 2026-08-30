//! cvar/cmd/alias registry with C-callable callbacks
//!
//! Rust migration Phase 7 M2 (ROADMAP.md): the pointer-free halves of
//! `Quake/cvar.c` and `Quake/cmd.c`. The registries themselves are linked
//! lists of C-layout nodes and stay in `quake-capi`; only the two pieces of
//! logic that are pure functions over bytes live here, where they can be
//! unit-tested against the C originals' quirks.

#![forbid(unsafe_code)] // ADR-004: pure crate

use quake_util::printf::{format, Arg};

/// `Cvar_SetValueQuick`/`Cvar_SetValue`'s float-to-string conversion
/// (cvar.c:541-558 and cvar.c:584-601 -- the two bodies are identical).
///
/// Returns the 32-byte stack buffer the C originals use, NUL-terminated.
///
/// // COMPAT: ADR-005 -- `%f` goes through the engine printf formatter, not
/// Rust's `{}`; config.cfg and the userinfo wire strings are byte-compared.
/// // COMPAT: the `q_snprintf` truncation at 32 bytes is reproduced, and the
/// trailing-zero kill loop runs over the *truncated* text exactly as the C
/// does (it stops when the preceding byte is `.`, so `1.000000` becomes
/// `1.0`, and a truncated integral-looking expansion loses real digits).
/// // COMPAT: `(int)value` on an out-of-range float is UB in C; `as i32`
/// saturates. The comparison `value == (float)(int)value` then fails on both
/// sides for such values, so the selected branch matches in practice.
pub fn value_string(value: f32) -> [u8; 32] {
    let mut out = [0u8; 32];

    let integral = value == (value as i32) as f32;
    let formatted = if integral {
        format(b"%i", &[Arg::I32(value as i32)])
    } else {
        // C promotes the float argument to double before printf sees it
        format(b"%f", &[Arg::F64(value as f64)])
    };

    let n = formatted.len().min(out.len() - 1);
    out[..n].copy_from_slice(&formatted[..n]);

    if !integral {
        // kill trailing zeroes -- `while (--ptr > val && *ptr == '0' && ptr[-1] != '.')`
        let mut p = n;
        loop {
            if p == 0 {
                break;
            }
            p -= 1;
            if p > 0 && out[p] == b'0' && out[p - 1] != b'.' {
                out[p] = 0;
            } else {
                break;
            }
        }
    }

    out
}

/// The `Cbuf_Execute` line-break scan (cmd.c:176-188). Returns the index of
/// the terminating `;`/`\n`, or `cursize` when the whole buffer is one line.
///
/// `text` is the command buffer; the scan is bounded by `cursize`, but the
/// `//` test peeks at `text[i + 1]`, so callers pass a slice one byte longer
/// than `cursize` where the allocation permits.
///
/// // COMPAT: the peek reads one byte past `cursize` at `i == cursize - 1`.
/// The C does the same (inside the 256 KB `SZ_Alloc`), and it is observable:
/// a buffer whose last byte is `/` latches the comment flag off whatever
/// stale byte follows it. A missing byte is treated as 0.
/// // COMPAT: `comment` is never cleared inside the loop, so once `//` is
/// seen a `;` no longer splits the line until a `\n`.
pub fn line_break(text: &[u8], cursize: usize) -> usize {
    let mut quotes: u32 = 0;
    let mut comment = false;
    let mut i = 0usize;

    while i < cursize {
        let c = text[i];
        if c == b'"' {
            quotes = quotes.wrapping_add(1);
        }
        if c == b'/' && text.get(i + 1).copied().unwrap_or(0) == b'/' {
            comment = true;
        }
        if (quotes & 1) == 0 && !comment && c == b';' {
            break;
        }
        if c == b'\n' {
            break;
        }
        i += 1;
    }

    i
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(buf: &[u8; 32]) -> &[u8] {
        let n = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        &buf[..n]
    }

    #[test]
    fn integral_values_use_percent_i() {
        assert_eq!(s(&value_string(0.0)), b"0");
        assert_eq!(s(&value_string(1.0)), b"1");
        assert_eq!(s(&value_string(-42.0)), b"-42");
        // 123456789.0f is not representable; f32 rounds it to 123456792
        assert_eq!(s(&value_string(123456789.0)), b"123456792");
    }

    #[test]
    fn fractional_values_kill_trailing_zeroes() {
        assert_eq!(s(&value_string(0.5)), b"0.5");
        assert_eq!(s(&value_string(0.03125)), b"0.03125");
        assert_eq!(s(&value_string(-0.5)), b"-0.5");
    }

    #[test]
    fn tiny_values_keep_one_zero_after_the_point() {
        // %f of 1e-5 is "0.000010"; the loop stops at the byte after '.'
        assert_eq!(s(&value_string(1e-5)), b"0.00001");
    }

    #[test]
    fn value_string_truncates_at_32_bytes() {
        let out = value_string(1.0e30_f32 + 0.5);
        assert!(out[31] == 0);
        assert!(s(&out).len() <= 31);
    }

    #[test]
    fn line_break_splits_on_semicolon_and_newline() {
        assert_eq!(line_break(b"echo a;echo b", 13), 6);
        assert_eq!(line_break(b"echo a\necho b", 13), 6);
        assert_eq!(line_break(b"echo a", 6), 6);
    }

    #[test]
    fn line_break_ignores_semicolons_inside_quotes() {
        assert_eq!(line_break(b"say \"a;b\";x", 11), 9);
    }

    #[test]
    fn comment_latch_is_never_reset_within_a_line() {
        // once `//` is seen the `;` no longer splits, but `\n` still does
        assert_eq!(line_break(b"// a;b", 6), 6);
        assert_eq!(line_break(b"// a;b\nc", 8), 6);
    }

    #[test]
    fn line_break_peeks_one_byte_past_cursize() {
        // cursize 5: the final '/' peeks at the sixth byte
        assert_eq!(line_break(b"abcd//;x", 6), 6);
        // the peeked byte is outside cursize but still read
        assert_eq!(line_break(b"abcd/", 5), 5);
    }
}
