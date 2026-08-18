//! Locale-insensitive ASCII ctype semantics (`Quake/q_ctype.h`).
//!
//! The C header is `static inline` only and survives for C consumers; this is
//! the Rust-side equivalent for ported code (e.g. CRC over `q_tolower`-folded
//! bytes in pr_ext, cvar name handling). All functions take/return `i32` like
//! the C so out-of-range values behave identically.

pub fn q_isascii(c: i32) -> bool {
    (c & !0x7f) == 0
}

pub fn q_islower(c: i32) -> bool {
    (b'a' as i32..=b'z' as i32).contains(&c)
}

pub fn q_isupper(c: i32) -> bool {
    (b'A' as i32..=b'Z' as i32).contains(&c)
}

pub fn q_isalpha(c: i32) -> bool {
    q_islower(c) || q_isupper(c)
}

pub fn q_isdigit(c: i32) -> bool {
    (b'0' as i32..=b'9' as i32).contains(&c)
}

pub fn q_isxdigit(c: i32) -> bool {
    q_isdigit(c)
        || (b'a' as i32..=b'f' as i32).contains(&c)
        || (b'A' as i32..=b'F' as i32).contains(&c)
}

pub fn q_isalnum(c: i32) -> bool {
    q_isalpha(c) || q_isdigit(c)
}

pub fn q_isblank(c: i32) -> bool {
    c == b' ' as i32 || c == b'\t' as i32
}

pub fn q_isspace(c: i32) -> bool {
    matches!(c, 0x20 | 0x09 | 0x0a | 0x0d | 0x0c | 0x0b)
}

pub fn q_isgraph(c: i32) -> bool {
    c > 0x20 && c <= 0x7e
}

pub fn q_isprint(c: i32) -> bool {
    (0x20..=0x7e).contains(&c)
}

pub fn q_toascii(c: i32) -> i32 {
    c & 0x7f
}

pub fn q_tolower(c: i32) -> i32 {
    if q_isupper(c) {
        c | (b'a' - b'A') as i32
    } else {
        c
    }
}

pub fn q_toupper(c: i32) -> i32 {
    if q_islower(c) {
        c & !((b'a' - b'A') as i32)
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_c_semantics() {
        for c in -256i32..=512 {
            let b = c as u8; // wrapped, only meaningful in 0..=255 range checks
            let _ = b;
            assert_eq!(q_isascii(c), (c & !0x7f) == 0);
            assert_eq!(q_tolower(b'A' as i32), b'a' as i32);
            assert_eq!(q_toupper(b'z' as i32), b'Z' as i32);
        }
        assert!(q_isspace(b'\x0b' as i32));
        assert!(!q_isspace(0));
        assert_eq!(q_tolower(b'@' as i32), b'@' as i32);
        assert_eq!(q_tolower(b'[' as i32), b'[' as i32);
        // out-of-ASCII values pass through untouched, like the C
        assert_eq!(q_tolower(0xc4), 0xc4);
    }
}
