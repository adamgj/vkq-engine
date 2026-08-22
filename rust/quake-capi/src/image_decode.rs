//! Image_DecodePCX / Image_DecodeLMP shims (Quake/image_decode.c, Phase 3 M2)
//!
//! The C decoders stream from the Sys_File handle; these shims read the
//! whole resource (com_filesize bytes from the current position) in one
//! Sys_FileRead and hand the slice to the pure quake-image parsers, then
//! replicate the C originals' Mem_Alloc size arithmetic, Sys_Error
//! messages, and COM_CloseFile points exactly. COMPAT: for UB-only inputs
//! (RLE overrun/end-of-file, out-of-resource pak reads) the C originals
//! read or write out of bounds; these shims Sys_Error instead — see the
//! COMPAT notes in quake-image and the task plan amendment log.

use core::ffi::{c_char, c_int};

use quake_c_sys as sys;
use quake_image::{lmp, pcx};

/// Read the rest of the resource (the C originals' view of it: com_filesize
/// truncated to int, like their `const int file_size` locals).
///
/// # Safety
/// `file_handle` must be a handle COM_OpenFile just opened, positioned at
/// the resource start.
unsafe fn read_resource(file_handle: c_int) -> (Vec<u8>, c_int) {
    // COMPAT: both C decoders truncate com_filesize to int
    // SAFETY: reads the engine's thread-local resource size, no aliasing
    let file_size = unsafe { sys::COM_ThreadFileSize() } as c_int;
    let mut buf = vec![0u8; file_size.max(0) as usize];
    // SAFETY: buf is valid for file_size bytes; Sys_FileRead returns the
    // count actually read
    let got = unsafe { sys::Sys_FileRead(file_handle, buf.as_mut_ptr().cast(), file_size) };
    buf.truncate(got.clamp(0, file_size) as usize);
    (buf, file_size)
}

/// # Safety
/// C ABI contract of Image_DecodePCX: open `file_handle` positioned at the
/// resource start, valid `width`/`height` out-pointers, NUL-terminated
/// `image_name`.
#[no_mangle]
pub unsafe extern "C" fn Image_DecodePCX(
    file_handle: c_int,
    width: *mut c_int,
    height: *mut c_int,
    image_name: *const c_char,
) -> *mut u8 {
    // SAFETY: caller guarantees an open handle at the resource start
    let (file, _) = unsafe { read_resource(file_handle) };

    let header = match pcx::parse_header(&file) {
        Ok(h) => h,
        // SAFETY: Sys_Error is diverging; format strings and varargs match
        // the C call sites byte for byte
        Err(pcx::Error::NotValid) => unsafe {
            sys::Sys_Error(c"'%s' is not a valid PCX file".as_ptr(), image_name)
        },
        // SAFETY: as above
        Err(pcx::Error::BadVersion(v)) => unsafe {
            sys::Sys_Error(
                c"'%s' is version %i, should be 5".as_ptr(),
                image_name,
                c_int::from(v),
            )
        },
        // SAFETY: as above
        Err(pcx::Error::BadEncoding) => unsafe {
            sys::Sys_Error(c"'%s' has wrong encoding or bit depth".as_ptr(), image_name)
        },
    };

    // COMPAT: the C alloc size (w * h + 1) * 4 is an int converted to
    // size_t — sign-extended when the wrapped product is negative
    let alloc_size = header.alloc_size as isize as usize;
    // SAFETY: engine allocator, zero-initializing like the C's Mem_Alloc
    let data = unsafe { sys::Mem_Alloc(alloc_size) }.cast::<u8>();
    let out = if data.is_null() {
        // calloc failure: C would fault on the first pixel write (or return
        // NULL untouched when height <= 0, which decode reproduces by
        // writing nothing)
        &mut [][..]
    } else {
        // SAFETY: Mem_Alloc returned alloc_size valid, zeroed bytes
        unsafe { core::slice::from_raw_parts_mut(data, alloc_size) }
    };

    if pcx::decode(&file, &header, out).is_err() {
        // SAFETY: as above
        unsafe { sys::Sys_Error(c"'%s' is not a valid PCX file".as_ptr(), image_name) }
    }

    // SAFETY: engine call, closing the handle exactly where the C does
    unsafe { sys::COM_CloseFile(file_handle) };

    // SAFETY: valid out-pointers per the C ABI contract
    unsafe {
        *width = header.width;
        *height = header.height;
    }
    data
}

/// # Safety
/// C ABI contract of Image_DecodeLMP: open `file_handle` positioned at the
/// resource start, valid `width`/`height` out-pointers, NUL-terminated
/// `image_name`.
#[no_mangle]
pub unsafe extern "C" fn Image_DecodeLMP(
    file_handle: c_int,
    width: *mut c_int,
    height: *mut c_int,
    image_name: *const c_char,
) -> *mut u8 {
    // SAFETY: caller guarantees an open handle at the resource start
    let (file, file_size) = unsafe { read_resource(file_handle) };

    match lmp::decode(&file, file_size) {
        // SAFETY: diverging, message matches the C call sites
        Err(lmp::Error::NotValid) => unsafe {
            sys::Sys_Error(c"'%s' is not a valid LMP file".as_ptr(), image_name)
        },
        Ok(lmp::Lmp::SizeMismatch) => {
            // SAFETY: engine call, same point as the C early return
            unsafe { sys::COM_CloseFile(file_handle) };
            core::ptr::null_mut()
        }
        Ok(lmp::Lmp::Image {
            width: w,
            height: h,
            pixels,
        }) => {
            // SAFETY: engine allocator, same size expression as the C
            let data = unsafe { sys::Mem_Alloc(pixels.len()) }.cast::<u8>();
            if !data.is_null() {
                // SAFETY: Mem_Alloc returned pixels.len() valid bytes
                unsafe {
                    core::ptr::copy_nonoverlapping(pixels.as_ptr(), data, pixels.len());
                }
            }
            // SAFETY: engine call, closing the handle exactly where the C does
            unsafe { sys::COM_CloseFile(file_handle) };
            // SAFETY: valid out-pointers per the C ABI contract
            unsafe {
                *width = w as c_int;
                *height = h as c_int;
            }
            data
        }
    }
}
