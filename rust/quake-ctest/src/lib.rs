//! Safe wrappers over the c_ref_* reference C symbols (see build.rs and
//! include/c_ref_prelude.h) so tests can differentially compare the original
//! C implementations against the Rust ports.

use core::ffi::{c_char, c_int, c_void};

mod ffi {
    use core::ffi::{c_char, c_int, c_uchar, c_uint, c_void};
    extern "C" {
        pub fn ctest_snprintf_f(buf: *mut c_char, n: usize, fmt: *const c_char, v: f64) -> c_int;
        pub fn ctest_snprintf_i32(buf: *mut c_char, n: usize, fmt: *const c_char, v: i32) -> c_int;
        pub fn ctest_snprintf_u32(buf: *mut c_char, n: usize, fmt: *const c_char, v: u32) -> c_int;
        pub fn ctest_snprintf_i64(buf: *mut c_char, n: usize, fmt: *const c_char, v: i64) -> c_int;
        pub fn ctest_snprintf_u64(buf: *mut c_char, n: usize, fmt: *const c_char, v: u64) -> c_int;
        pub fn ctest_snprintf_str(
            buf: *mut c_char,
            n: usize,
            fmt: *const c_char,
            v: *const c_char,
        ) -> c_int;
        pub fn c_ref_CRC_Init(crcvalue: *mut u16);
        pub fn c_ref_CRC_ProcessByte(crcvalue: *mut u16, data: u8);
        pub fn c_ref_CRC_Value(crcvalue: u16) -> u16;
        pub fn c_ref_CRC_Block(start: *const u8, count: c_int) -> u16;
        pub fn c_ref_Com_BlockChecksum(buffer: *mut c_void, length: c_int) -> c_uint;
        pub fn c_ref_Com_BlockFullChecksum(buffer: *mut c_void, len: c_int, outbuf: *mut c_uchar);
        pub fn c_ref_q_strlcpy(dst: *mut c_char, src: *const c_char, siz: usize) -> usize;
        pub fn c_ref_q_strlcat(dst: *mut c_char, src: *const c_char, siz: usize) -> usize;
    }
}

pub fn c_crc_init() -> u16 {
    let mut v = 0u16;
    // SAFETY: valid pointer to a local
    unsafe { ffi::c_ref_CRC_Init(&mut v) };
    v
}

pub fn c_crc_process_byte(crc: &mut u16, data: u8) {
    // SAFETY: valid pointer from a &mut
    unsafe { ffi::c_ref_CRC_ProcessByte(crc, data) };
}

pub fn c_crc_value(crc: u16) -> u16 {
    // SAFETY: no pointers involved
    unsafe { ffi::c_ref_CRC_Value(crc) }
}

pub fn c_crc_block(data: &[u8]) -> u16 {
    assert!(data.len() <= c_int::MAX as usize);
    // SAFETY: pointer/length come from a valid slice
    unsafe { ffi::c_ref_CRC_Block(data.as_ptr(), data.len() as c_int) }
}

pub fn c_block_checksum(data: &[u8]) -> u32 {
    assert!(data.len() <= c_int::MAX as usize);
    // SAFETY: pointer/length come from a valid slice; the C reads only
    unsafe { ffi::c_ref_Com_BlockChecksum(data.as_ptr() as *mut c_void, data.len() as c_int) }
}

pub fn c_block_full_checksum(data: &[u8]) -> [u8; 16] {
    assert!(data.len() <= c_int::MAX as usize);
    let mut out = [0u8; 16];
    // SAFETY: pointer/length come from a valid slice; out is 16 writable bytes
    unsafe {
        ffi::c_ref_Com_BlockFullChecksum(
            data.as_ptr() as *mut c_void,
            data.len() as c_int,
            out.as_mut_ptr(),
        )
    };
    out
}

// --- platform snprintf oracle (ADR-005 conformance) ---

const SNPRINTF_BUF: usize = 1024;

macro_rules! oracle {
    ($name:ident, $ffi:ident, $ty:ty) => {
        /// Platform `snprintf` with one argument of the given type. `fmt` must
        /// be NUL-free ASCII.
        pub fn $name(fmt: &str, v: $ty) -> String {
            let mut cfmt = fmt.as_bytes().to_vec();
            assert!(!cfmt.contains(&0));
            cfmt.push(0);
            let mut buf = vec![0u8; SNPRINTF_BUF];
            // SAFETY: buf/fmt are valid NUL-terminated buffers; the wrapper is
            // a plain C function compiled in build.rs
            let n = unsafe {
                ffi::$ffi(
                    buf.as_mut_ptr() as *mut c_char,
                    SNPRINTF_BUF,
                    cfmt.as_ptr() as *const c_char,
                    v,
                )
            };
            assert!(
                n >= 0 && (n as usize) < SNPRINTF_BUF,
                "oracle buffer too small"
            );
            buf.truncate(n as usize);
            String::from_utf8(buf).unwrap()
        }
    };
}

oracle!(c_snprintf_f, ctest_snprintf_f, f64);
oracle!(c_snprintf_i32, ctest_snprintf_i32, i32);
oracle!(c_snprintf_u32, ctest_snprintf_u32, u32);
oracle!(c_snprintf_i64, ctest_snprintf_i64, i64);
oracle!(c_snprintf_u64, ctest_snprintf_u64, u64);

/// Platform `snprintf` with one string argument.
pub fn c_snprintf_str(fmt: &str, v: &[u8]) -> String {
    let mut cfmt = fmt.as_bytes().to_vec();
    assert!(!cfmt.contains(&0));
    cfmt.push(0);
    let mut cv = v.to_vec();
    assert!(!cv.contains(&0));
    cv.push(0);
    let mut buf = vec![0u8; SNPRINTF_BUF];
    // SAFETY: all buffers valid and NUL-terminated
    let n = unsafe {
        ffi::ctest_snprintf_str(
            buf.as_mut_ptr() as *mut c_char,
            SNPRINTF_BUF,
            cfmt.as_ptr() as *const c_char,
            cv.as_ptr() as *const c_char,
        )
    };
    assert!(
        n >= 0 && (n as usize) < SNPRINTF_BUF,
        "oracle buffer too small"
    );
    buf.truncate(n as usize);
    String::from_utf8(buf).unwrap()
}

/// Runs the reference q_strlcpy on a copy of `dst`; returns (result, buffer).
/// `src` must not contain interior NULs.
pub fn c_strlcpy(dst: &[u8], src: &[u8], siz: usize) -> (usize, Vec<u8>) {
    assert!(siz <= dst.len());
    assert!(!src.contains(&0));
    let mut dst = dst.to_vec();
    let mut src_z = src.to_vec();
    src_z.push(0);
    // SAFETY: src_z is NUL-terminated; dst is valid for siz <= dst.len() bytes
    let ret = unsafe {
        ffi::c_ref_q_strlcpy(
            dst.as_mut_ptr() as *mut c_char,
            src_z.as_ptr() as *const c_char,
            siz,
        )
    };
    (ret, dst)
}

/// Runs the reference q_strlcat on a copy of `dst`; returns (result, buffer).
/// `src` must not contain interior NULs.
pub fn c_strlcat(dst: &[u8], src: &[u8], siz: usize) -> (usize, Vec<u8>) {
    assert!(siz <= dst.len());
    assert!(!src.contains(&0));
    let mut dst = dst.to_vec();
    let mut src_z = src.to_vec();
    src_z.push(0);
    // SAFETY: src_z is NUL-terminated; dst is valid for siz <= dst.len() bytes
    let ret = unsafe {
        ffi::c_ref_q_strlcat(
            dst.as_mut_ptr() as *mut c_char,
            src_z.as_ptr() as *const c_char,
            siz,
        )
    };
    (ret, dst)
}
