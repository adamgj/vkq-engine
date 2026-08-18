//! C ABI shims for `Quake/mdfour.c` (declarations stay in `Quake/crc.h`).

use core::ffi::{c_int, c_uchar, c_uint, c_void};

// note: `void *buffer` (non-const) mirrors crc.h exactly — the declarations
// must stay compatible for the signature-parity check
fn slice_from<'a>(buffer: *mut c_void, length: c_int) -> &'a [u8] {
    if length > 0 {
        // SAFETY: callers (engine C) pass a buffer of at least `length` bytes;
        // negative lengths are caller UB in C and mapped to empty here
        unsafe { core::slice::from_raw_parts(buffer as *const u8, length as usize) }
    } else {
        &[]
    }
}

/// C: `unsigned Com_BlockChecksum (void *buffer, int length);`
///
/// # Safety
/// `buffer` must point to at least `length` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn Com_BlockChecksum(buffer: *mut c_void, length: c_int) -> c_uint {
    quake_util::mdfour::block_checksum(slice_from(buffer, length))
}

/// C: `void Com_BlockFullChecksum (void *buffer, int len, unsigned char *outbuf);`
///
/// # Safety
/// `buffer` must point to at least `len` readable bytes; `outbuf` must be
/// writable for 16 bytes.
#[no_mangle]
pub unsafe extern "C" fn Com_BlockFullChecksum(
    buffer: *mut c_void,
    len: c_int,
    outbuf: *mut c_uchar,
) {
    let digest = quake_util::mdfour::mdfour(slice_from(buffer, len));
    // SAFETY: outbuf is writable for 16 bytes per the crc.h contract
    unsafe { core::ptr::copy_nonoverlapping(digest.as_ptr(), outbuf, 16) };
}
