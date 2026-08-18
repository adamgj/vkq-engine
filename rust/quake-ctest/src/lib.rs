//! Safe wrappers over the c_ref_* reference C symbols (see build.rs and
//! include/c_ref_prelude.h) so tests can differentially compare the original
//! C implementations against the Rust ports.

use core::ffi::{c_char, c_int, c_void};

mod ffi {
    use core::ffi::{c_char, c_int, c_uchar, c_uint, c_void};
    extern "C" {
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
