//! OpenBSD `strlcpy`/`strlcat` semantics (`Quake/strlcpy.c`, `Quake/strlcat.c`).
//!
//! The pure cores work on byte slices; the pointer-level `q_strlcpy` /
//! `q_strlcat` shims live in quake-capi. Return-value semantics are relied on
//! by ~41 engine files: `strlcpy` returns `strlen(src)`, `strlcat` returns
//! `strlen(src) + min(siz, strlen(initial dst))`; a return >= siz means
//! truncation.

/// Copy `src` (NUL excluded) into `dst` (the full destination buffer,
/// `siz` = `dst.len()`). At most `siz - 1` bytes are copied; `dst` is always
/// NUL-terminated unless `siz == 0`. Returns `strlen(src)`.
pub fn strlcpy(dst: &mut [u8], src: &[u8]) -> usize {
    let siz = dst.len();
    if siz != 0 {
        let n = src.len().min(siz - 1);
        dst[..n].copy_from_slice(&src[..n]);
        dst[n] = 0;
    }
    src.len()
}

/// Append `src` (NUL excluded) to the C string in `dst` (the full destination
/// buffer, `siz` = `dst.len()`). The existing-string scan is bounded by `siz`,
/// exactly like the C: if no NUL is found within `siz` bytes, nothing is
/// written and `siz + strlen(src)` is returned.
pub fn strlcat(dst: &mut [u8], src: &[u8]) -> usize {
    let siz = dst.len();
    let dlen = dst.iter().take(siz).position(|&b| b == 0).unwrap_or(siz);
    let n = siz - dlen;
    if n == 0 {
        return dlen + src.len();
    }
    let copy = src.len().min(n - 1);
    dst[dlen..dlen + copy].copy_from_slice(&src[..copy]);
    dst[dlen + copy] = 0;
    dlen + src.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strlcpy_fits() {
        let mut dst = [0xffu8; 8];
        assert_eq!(strlcpy(&mut dst, b"abc"), 3);
        assert_eq!(&dst[..4], b"abc\0");
    }

    #[test]
    fn strlcpy_truncates_and_reports_full_length() {
        let mut dst = [0xffu8; 4];
        assert_eq!(strlcpy(&mut dst, b"abcdef"), 6);
        assert_eq!(&dst, b"abc\0");
    }

    #[test]
    fn strlcpy_zero_size_writes_nothing() {
        let mut dst = [0u8; 0];
        assert_eq!(strlcpy(&mut dst, b"abc"), 3);
    }

    #[test]
    fn strlcat_appends() {
        let mut dst = [0u8; 8];
        dst[..3].copy_from_slice(b"ab\0");
        assert_eq!(strlcat(&mut dst, b"cd"), 4);
        assert_eq!(&dst[..5], b"abcd\0");
    }

    #[test]
    fn strlcat_truncates() {
        let mut dst = [0u8; 5];
        dst[..3].copy_from_slice(b"ab\0");
        assert_eq!(strlcat(&mut dst, b"cdef"), 6);
        assert_eq!(&dst, b"abcd\0");
    }

    #[test]
    fn strlcat_unterminated_dst_returns_siz_plus_srclen() {
        let mut dst = [b'x'; 4];
        assert_eq!(strlcat(&mut dst, b"yz"), 6);
        assert_eq!(&dst, b"xxxx"); // untouched
    }
}
