//! `tasks.c` -- `parse_pinned_workers`: the `-pinnedworkers` argument.
//!
//! The argument is a comma list of core ids. `q_strsplit` drops empty tokens
//! (leading, doubled and trailing separators are all gobbled), the result is
//! capped at `MAX_WORKERS` entries, every entry must be all digits, and each
//! value is reduced modulo the logical core count. Any invalid entry
//! invalidates the whole list, in which case the engine keeps its default
//! worker count and pins nothing.

use crate::MAX_WORKERS;

/// Core ids to pin the workers to, or empty when the argument is invalid or
/// names no cores. `cores` is `QThread_NumLogicalCores()`; a value of zero is
/// treated as one so the modulo cannot fault.
pub fn parse(arg: &[u8], cores: usize) -> Vec<usize> {
    let cores = cores.max(1);
    let mut tokens = arg.split(|&c| c == b',').filter(|t| !t.is_empty());
    let mut result = Vec::new();
    // q_strsplit hands back one empty field when nothing was found, which the
    // digit check below then rejects.
    let mut first = true;
    loop {
        if result.len() >= MAX_WORKERS {
            break;
        }
        let field = match tokens.next() {
            Some(field) => field,
            None if first => &[][..],
            None => break,
        };
        first = false;
        if field.is_empty() || !field.iter().all(u8::is_ascii_digit) {
            return Vec::new();
        }
        result.push(strtol_saturating(field) % cores);
    }
    result
}

/// `strtol` on an all-digit field: saturates at `LONG_MAX` (which is
/// platform-sized, like the C `long`).
fn strtol_saturating(digits: &[u8]) -> usize {
    let mut value: core::ffi::c_long = 0;
    for &d in digits {
        let d = core::ffi::c_long::from(d - b'0');
        value = match value.checked_mul(10).and_then(|v| v.checked_add(d)) {
            Some(v) => v,
            None => return core::ffi::c_long::MAX as usize,
        };
    }
    value as usize
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::parse;

    #[test]
    fn plain_list() {
        assert_eq!(parse(b"0,1,2,3", 8), vec![0, 1, 2, 3]);
        assert_eq!(parse(b"7", 8), vec![7]);
    }

    #[test]
    fn empty_tokens_are_dropped() {
        assert_eq!(parse(b"1,,2,", 8), vec![1, 2]);
        assert_eq!(parse(b",3", 8), vec![3]);
    }

    #[test]
    fn empty_and_separator_only_are_invalid() {
        assert!(parse(b"", 8).is_empty());
        assert!(parse(b",,,", 8).is_empty());
    }

    #[test]
    fn non_digits_invalidate_everything() {
        assert!(parse(b"1a", 8).is_empty());
        assert!(parse(b"0,x", 8).is_empty());
        assert!(parse(b"-1", 8).is_empty());
        assert!(parse(b"1 ,2", 8).is_empty());
    }

    #[test]
    fn values_wrap_modulo_cores() {
        assert_eq!(parse(b"9", 8), vec![1]);
        assert_eq!(parse(b"1,2", 1), vec![0, 0]);
        assert_eq!(parse(b"5", 0), vec![0]);
    }

    #[test]
    fn capped_at_max_workers() {
        let arg = (0..40).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        let parsed = parse(arg.as_bytes(), 64);
        assert_eq!(parsed.len(), 32);
        assert_eq!(parsed, (0..32).collect::<Vec<_>>());
    }

    #[test]
    fn overflow_saturates_like_strtol() {
        let huge = b"99999999999999999999999999999";
        let expected = (core::ffi::c_long::MAX as usize) % 7;
        assert_eq!(parse(huge, 7), vec![expected]);
    }
}
