//! Image_DecodePCX / Image_DecodeLMP shims (Quake/image_decode.c, Phase 3 M2)
//! and the Image_DecodeSTB shim (Quake/image_stb.c, Phase 3 M8).
//!
//! The C decoders stream from the Sys_File handle; these shims read the
//! whole resource (com_filesize bytes from the current position) in one
//! Sys_FileRead and hand the slice to the pure quake-image parsers, then
//! replicate the C originals' Mem_Alloc size arithmetic, Sys_Error
//! messages, and COM_CloseFile points exactly. COMPAT: for UB-only inputs
//! (RLE overrun/end-of-file, out-of-resource pak reads) the C originals
//! read or write out of bounds; these shims Sys_Error instead — see the
//! COMPAT notes in quake-image and the task plan amendment log.
//!
//! Image_DecodeSTB classifies the resource with the ported stb probe chain
//! (`quake_image::stb_sniff`) and dispatches per format; formats whose
//! crate/hand-ported decoder has not passed its ADR-012 gate route through
//! the always-compiled C `Image_DecodeSTBMem` (stb over the same bytes).

use core::ffi::{c_char, c_int};

use quake_c_sys as sys;
use quake_image::{jpeg_stb, lmp, pcx, png_stb, stb_sniff, tga};

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

/// The C failure path: `Con_Warning ("couldn't load %s (%s)\n", ...)`
/// (Quake/image_stb.c), format string byte-identical.
///
/// # Safety
/// `image_name` must be NUL-terminated and `reason` a NUL-terminated C
/// string (static or stb thread-local).
unsafe fn stb_warn(image_name: *const c_char, reason: *const c_char) {
    // SAFETY: caller contract; varargs match the C Con_Warning call site
    unsafe {
        sys::Con_Warning(c"couldn't load %s (%s)\n".as_ptr(), image_name, reason);
    }
}

/// Decode `file` with the C stb fallback (`Image_DecodeSTBMem`), emitting
/// the C's Con_Warning on failure. Returns the stb-allocated buffer (the
/// same Mem_Alloc plug the streaming C decoder uses) or NULL.
///
/// # Safety
/// Valid out-pointers; NUL-terminated `image_name`.
unsafe fn decode_stb_mem(
    file: &[u8],
    width: *mut c_int,
    height: *mut c_int,
    image_name: *const c_char,
) -> *mut u8 {
    let mut reason: *const c_char = core::ptr::null();
    // SAFETY: file is a live slice; len fits c_int (read_resource caps it at
    // com_filesize truncated to int); out-pointers valid per caller
    let data = unsafe {
        sys::Image_DecodeSTBMem(
            file.as_ptr(),
            file.len() as c_int,
            width,
            height,
            &mut reason,
        )
    };
    if data.is_null() {
        // SAFETY: stb's failure reason is a static/thread-local C string
        unsafe { stb_warn(image_name, reason) };
    }
    data
}

/// # Safety
/// C ABI contract of Image_DecodeSTB (Quake/image_stb.c): open `file_handle`
/// positioned at the resource start, valid `width`/`height` out-pointers,
/// NUL-terminated `image_name`. Like the C original this never `Sys_Error`s:
/// failure is a Con_Warning, COM_CloseFile and NULL, with the out-params
/// only written where the format's decoder writes them.
#[no_mangle]
pub unsafe extern "C" fn Image_DecodeSTB(
    file_handle: c_int,
    width: *mut c_int,
    height: *mut c_int,
    image_name: *const c_char,
) -> *mut u8 {
    // SAFETY: caller guarantees an open handle at the resource start
    let (file, _) = unsafe { read_resource(file_handle) };

    let data = match stb_sniff::classify(&file) {
        stb_sniff::Format::Unknown => {
            // C: stbi__load_main falls off the probe chain —
            // stbi__err("unknown image type", ...) into the Con_Warning
            // SAFETY: static reason string; image_name per contract
            unsafe { stb_warn(image_name, c"unknown image type".as_ptr()) };
            core::ptr::null_mut()
        }
        stb_sniff::Format::Tga => match tga::decode(&file) {
            Ok(t) => {
                // SAFETY: engine allocator; stb's final buffer is also a
                // Mem_Alloc'd allocation via the STBI_MALLOC plug
                let data = unsafe { sys::Mem_Alloc(t.rgba.len()) }.cast::<u8>();
                if !data.is_null() {
                    // SAFETY: Mem_Alloc returned rgba.len() valid bytes
                    unsafe {
                        core::ptr::copy_nonoverlapping(t.rgba.as_ptr(), data, t.rgba.len());
                    }
                }
                // SAFETY: valid out-pointers per the C ABI contract
                unsafe {
                    *width = t.width;
                    *height = t.height;
                }
                data
            }
            Err(e) => {
                // stb publishes *x/*y before some failures and not others;
                // the pure decoder reports which applies
                if let Some((w, h)) = e.dims {
                    // SAFETY: valid out-pointers per the C ABI contract
                    unsafe {
                        *width = w;
                        *height = h;
                    }
                }
                let reason = match e.reason {
                    tga::Reason::TooLarge => c"too large",
                    tga::Reason::BadFormat => c"bad format",
                    tga::Reason::BadPalette => c"bad palette",
                };
                // SAFETY: static reason string; image_name per contract
                unsafe { stb_warn(image_name, reason.as_ptr()) };
                core::ptr::null_mut()
            }
        },
        stb_sniff::Format::Png => match png_stb::decode(&file) {
            // CgBI (iPhone) PNGs stay on the C stb path wholesale
            // SAFETY: caller contract (out-pointers, image_name)
            Ok(png_stb::Png::Fallback) => unsafe {
                decode_stb_mem(&file, width, height, image_name)
            },
            Ok(png_stb::Png::Image {
                width: w,
                height: h,
                rgba,
            }) => {
                // SAFETY: engine allocator; stb's final buffer is likewise a
                // Mem_Alloc'd allocation via the STBI_MALLOC plug
                let data = unsafe { sys::Mem_Alloc(rgba.len()) }.cast::<u8>();
                if !data.is_null() {
                    // SAFETY: Mem_Alloc returned rgba.len() valid bytes
                    unsafe {
                        core::ptr::copy_nonoverlapping(rgba.as_ptr(), data, rgba.len());
                    }
                }
                // stb's PNG path writes the out-dims only on success
                // SAFETY: valid out-pointers per the C ABI contract
                unsafe {
                    *width = w;
                    *height = h;
                }
                data
            }
            Err(e) => {
                let owned;
                let reason: *const c_char = match &e {
                    png_stb::Error::Stb(r) => {
                        owned = std::ffi::CString::new(*r).expect("static stb reason");
                        owned.as_ptr()
                    }
                    // "XXXX PNG chunk not known" with the raw type bytes,
                    // truncated at the first NUL like the C char array
                    png_stb::Error::UnknownChunk(t) => {
                        let mut s = t.to_vec();
                        s.extend_from_slice(b" PNG chunk not known");
                        let nul = s.iter().position(|&b| b == 0).unwrap_or(s.len());
                        s.truncate(nul);
                        owned = std::ffi::CString::new(s).expect("truncated at NUL");
                        owned.as_ptr()
                    }
                    // COMPAT: the C prints whatever stale reason the stb
                    // thread-local held; the text is unspecified, only the
                    // reject decision is defined (masked in the differential)
                    png_stb::Error::StaleReason => c"corrupt PNG".as_ptr(),
                    // crate-side reject: same decision as stb, crate's own
                    // reason text (masked in the differential)
                    png_stb::Error::Crate(msg) => {
                        owned =
                            std::ffi::CString::new(msg.replace('\0', "?")).expect("NULs replaced");
                        owned.as_ptr()
                    }
                };
                // SAFETY: reason is NUL-terminated and outlives the call;
                // image_name per contract
                unsafe { stb_warn(image_name, reason) };
                core::ptr::null_mut()
            }
        },
        // COMPAT (owner decision, 2026-08-24): JPEG ships on zune-jpeg
        // under the relaxed gate — accept/reject + dims parity + bounded
        // pixel delta vs stb, not bit-exact (see quake_image::jpeg_stb)
        stb_sniff::Format::Jpeg => match jpeg_stb::decode(&file) {
            Ok(j) => {
                // SAFETY: engine allocator; stb's final buffer is likewise a
                // Mem_Alloc'd allocation via the STBI_MALLOC plug
                let data = unsafe { sys::Mem_Alloc(j.rgba.len()) }.cast::<u8>();
                if !data.is_null() {
                    // SAFETY: Mem_Alloc returned rgba.len() valid bytes
                    unsafe {
                        core::ptr::copy_nonoverlapping(j.rgba.as_ptr(), data, j.rgba.len());
                    }
                }
                // stb's JPEG path writes the out-dims only on success
                // SAFETY: valid out-pointers per the C ABI contract
                unsafe {
                    *width = j.width;
                    *height = j.height;
                }
                data
            }
            Err(e) => {
                let owned;
                let reason: *const c_char = match &e {
                    jpeg_stb::Error::TooLarge => c"too large".as_ptr(),
                    jpeg_stb::Error::OutOfMem => c"outofmem".as_ptr(),
                    // crate-side reject: same decision as stb, crate's own
                    // reason text (masked in the differential)
                    jpeg_stb::Error::Crate(msg) => {
                        owned =
                            std::ffi::CString::new(msg.replace('\0', "?")).expect("NULs replaced");
                        owned.as_ptr()
                    }
                };
                // SAFETY: reason is NUL-terminated and outlives the call;
                // image_name per contract
                unsafe { stb_warn(image_name, reason) };
                core::ptr::null_mut()
            }
        },
    };

    // SAFETY: engine call, closing the handle exactly where the C does
    // (image_stb.c closes once, after decode, on success and failure alike)
    unsafe { sys::COM_CloseFile(file_handle) };
    data
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
