//! C ABI shims for `Quake/crc.c` (declarations stay in `Quake/crc.h`).

use core::ffi::c_int;

/// C: `void CRC_Init (unsigned short *crcvalue);`
///
/// # Safety
/// `crcvalue` must be a valid, writable pointer.
#[no_mangle]
pub unsafe extern "C" fn CRC_Init(crcvalue: *mut u16) {
    // SAFETY: caller (engine C) passes a valid pointer per the crc.h contract
    unsafe { *crcvalue = quake_util::crc::crc_init() };
}

/// C: `void CRC_ProcessByte (unsigned short *crcvalue, byte data);`
///
/// # Safety
/// `crcvalue` must be a valid, writable pointer.
#[no_mangle]
pub unsafe extern "C" fn CRC_ProcessByte(crcvalue: *mut u16, data: u8) {
    // SAFETY: caller (engine C) passes a valid pointer per the crc.h contract
    unsafe { quake_util::crc::crc_process_byte(&mut *crcvalue, data) };
}

/// C: `unsigned short CRC_Value (unsigned short crcvalue);`
#[no_mangle]
pub extern "C" fn CRC_Value(crcvalue: u16) -> u16 {
    quake_util::crc::crc_value(crcvalue)
}

/// C: `unsigned short CRC_Block (const byte *start, int count);`
///
/// # Safety
/// `start` must point to at least `count` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn CRC_Block(start: *const u8, count: c_int) -> u16 {
    let data = if count > 0 {
        // SAFETY: caller (engine C) passes a buffer of at least `count` bytes;
        // a negative count is caller UB in C (runaway `count--`), mapped to empty
        unsafe { core::slice::from_raw_parts(start, count as usize) }
    } else {
        &[]
    };
    quake_util::crc::crc_block(data)
}
