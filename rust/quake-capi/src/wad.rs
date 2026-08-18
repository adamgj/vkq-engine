//! C ABI shims for `Quake/wad.c` (declarations stay in `Quake/wad.h`).
//!
//! The wad decision logic is pure (quake_fs::wad); this shim owns the engine
//! globals (`wad_numlumps`/`wad_lumps`/`wad_base`), performs the same
//! in-place edits of the loaded file image the C did (byteswaps, name
//! cleanup, SwapPic — the renderer does pointer arithmetic against
//! `wad_base`), and keeps every allocation on Mem_Alloc/Mem_Free since the
//! buffers and wad_t lists cross the FFI boundary (ADR-013).

use core::ffi::{c_char, c_int, c_void, CStr};
use quake_fs::wad::{
    check_add_wad_header, cleanup_name, header_extends_beyond, repair_lump, wad2_id_ok,
    AddWadVerdict, LumpProblem,
};
use quake_types::wad::{LumpInfo, QPic, WadInfo, TYP_QPIC};

/// wad_t (wad.h): embeds the engine fshandle_t; C callers seek/read through
/// `wad->fh` (gl_model.c), so the layout is ABI.
#[repr(C)]
pub struct Wad {
    pub name: [c_char; 64], // MAX_QPATH
    pub id: c_int,
    pub fh: quake_c_sys::fshandle_t,
    pub numlumps: c_int,
    pub lumps: *mut LumpInfo,
    pub next: *mut Wad,
}

const _: () = assert!(std::mem::size_of::<quake_c_sys::fshandle_t>() == 40);
const _: () = assert!(std::mem::size_of::<Wad>() == 136);
const _: () = assert!(std::mem::offset_of!(Wad, id) == 64);
const _: () = assert!(std::mem::offset_of!(Wad, fh) == 72);
const _: () = assert!(std::mem::offset_of!(Wad, numlumps) == 112);
const _: () = assert!(std::mem::offset_of!(Wad, lumps) == 120);
const _: () = assert!(std::mem::offset_of!(Wad, next) == 128);

#[no_mangle]
pub static mut wad_numlumps: c_int = 0;
#[no_mangle]
pub static mut wad_lumps: *mut LumpInfo = core::ptr::null_mut();
#[no_mangle]
pub static mut wad_base: *mut u8 = core::ptr::null_mut();

/// Reads a lump name (up to 16 bytes, stopping at NUL) from C memory.
unsafe fn read_name(p: *const c_char) -> Vec<u8> {
    let mut out = Vec::with_capacity(16);
    for i in 0..16 {
        // SAFETY: lumpinfo_t names are 16 bytes; we never read past that
        let b = unsafe { *p.add(i) } as u8;
        if b == 0 {
            break;
        }
        out.push(b);
    }
    out
}

/// In-place `W_CleanupName` on a 16-byte lump name.
unsafe fn cleanup_name_in_place(p: *mut c_char) {
    // SAFETY: p points to a 16-byte name field per the wad.h layout
    let raw = unsafe { read_name(p) };
    let cleaned = cleanup_name(&raw);
    // SAFETY: as above; cleanup_name always yields exactly 16 bytes
    unsafe { core::ptr::copy_nonoverlapping(cleaned.as_ptr() as *const c_char, p, 16) };
}

/// C: `void W_CleanupName (const char *in, char *out);`
///
/// # Safety
/// `in_` must be a NUL-terminated string or at least 16 readable bytes;
/// `out` must be writable for 16 bytes. In-place calls are fine, like C.
#[no_mangle]
pub unsafe extern "C" fn W_CleanupName(in_: *const c_char, out: *mut c_char) {
    // SAFETY: caller contract above
    let raw = unsafe { read_name(in_) };
    let cleaned = cleanup_name(&raw);
    // SAFETY: caller contract above; the read is already complete, so in-place
    // calls (in_ == out) stay well-defined, like C
    unsafe { core::ptr::copy_nonoverlapping(cleaned.as_ptr() as *const c_char, out, 16) };
}

/// The WAD2 magic at the head of a loaded image.
///
/// Split from `read_wad_counts` to mirror wad.c's read order: the id first,
/// the counts only once the id checks out. That narrows but does not close the
/// short-file window — `COM_LoadFile` allocates `len + 1` and pads to nothing,
/// so a gfx.wad whose entire contents are `WAD2` still reaches the 12-byte
/// header read. wad.c gates that read on the id alone too, and staying
/// bug-for-bug with it is deliberate.
///
/// # Safety
/// `base` must point to at least 4 readable bytes.
unsafe fn read_wad_ident(base: *const u8) -> [u8; 4] {
    // SAFETY: caller contract above; the read is unaligned-safe
    let ident = unsafe { core::ptr::addr_of!((*(base as *const WadInfo)).identification) }
        .cast::<[u8; 4]>();
    // SAFETY: as above
    unsafe { ident.read_unaligned() }
}

/// The lump-directory location from a WAD2 header: `(numlumps, infotableofs)`.
///
/// # Safety
/// `base` must point to a full readable `wadinfo_t`.
unsafe fn read_wad_counts(base: *const u8) -> (c_int, i32) {
    // SAFETY: caller contract above; reads are unaligned-safe, and the file
    // data is little-endian on disk
    unsafe {
        let header = base as *const WadInfo;
        (
            i32::from_le(core::ptr::addr_of!((*header).numlumps).read_unaligned()),
            i32::from_le(core::ptr::addr_of!((*header).infotableofs).read_unaligned()),
        )
    }
}

/// Shared per-lump fixup: byteswaps, bounds repair (+warning), name cleanup.
/// `warn` receives the pre-formatted problem. Returns nothing; edits in place.
unsafe fn fixup_lump(
    lump: *mut LumpInfo,
    file_len: i64,
    cleanup_first: bool,
    warn: impl Fn(*const c_char, LumpProblem),
) {
    // SAFETY: `lump` points at a lumpinfo_t inside a readable/writable buffer,
    // so `name` is its 16-byte field
    let name = unsafe { core::ptr::addr_of_mut!((*lump).name) as *mut c_char };
    if cleanup_first {
        // SAFETY: as above
        unsafe { cleanup_name_in_place(name) };
    }

    // SAFETY: as above; the reads are unaligned-safe and the on-disk fields
    // are little-endian
    let (mut filepos, mut size, disksize) = unsafe {
        (
            i32::from_le(core::ptr::addr_of!((*lump).filepos).read_unaligned()),
            i32::from_le(core::ptr::addr_of!((*lump).size).read_unaligned()),
            i32::from_le(core::ptr::addr_of!((*lump).disksize).read_unaligned()),
        )
    };

    if let Some(problem) = repair_lump(&mut filepos, &mut size, disksize, file_len) {
        warn(name as *const c_char, problem);
    }

    // SAFETY: as above; the writes go back into the fields just read
    unsafe {
        core::ptr::addr_of_mut!((*lump).filepos).write_unaligned(filepos);
        core::ptr::addr_of_mut!((*lump).size).write_unaligned(size);
    }
    if !cleanup_first {
        // SAFETY: as above
        unsafe { cleanup_name_in_place(name) };
    }
}

/// C: `void W_LoadWadFile (void);` — loads the global gfx.wad.
///
/// # Safety
/// Engine single-threaded init path, like the C original.
#[no_mangle]
pub unsafe extern "C" fn W_LoadWadFile() {
    let filename = c"gfx.wad";

    // SAFETY: wad_base ownership is this function's, exactly as in the C
    // original, and the globals are only touched on the init path
    let base = unsafe {
        if !wad_base.is_null() {
            quake_c_sys::Mem_Free(wad_base as *const c_void);
        }
        wad_base = quake_c_sys::COM_LoadFile(filename.as_ptr(), core::ptr::null_mut());
        wad_base
    };
    if base.is_null() {
        // SAFETY: engine C API; the format and both arguments are
        // NUL-terminated strings
        unsafe {
            quake_c_sys::Sys_Error(
                c"W_LoadWadFile: couldn't load %s\n\nBasedir is: %s\n\nCheck that this has an id1 subdirectory containing pak0.pak and pak1.pak, or use the -basedir command-line option to specify another directory."
                    .as_ptr(),
                filename.as_ptr(),
                core::ptr::addr_of!(quake_c_sys::manual::com_basedir) as *const c_char,
            );
        }
    }
    // SAFETY: engine C API, reporting on the file COM_LoadFile just read
    let file_len = unsafe { quake_c_sys::COM_ThreadFileSize() };

    // SAFETY: COM_LoadFile returned a non-NULL image. It guarantees only
    // `len + 1` bytes, so a gfx.wad under 4 bytes reads out of bounds here —
    // exactly where wad.c's unchecked `header->identification[0..4]` does.
    let ident = unsafe { read_wad_ident(base) };

    let mut infotableofs: i32 = 0;
    if !wad2_id_ok(&ident) {
        // SAFETY: engine C API; NUL-terminated format and argument
        unsafe {
            quake_c_sys::Con_Printf(
                c"Wad file %s doesn't have WAD2 id\n".as_ptr(),
                filename.as_ptr(),
            );
            wad_numlumps = 0;
        }
    } else {
        // SAFETY: wad.c gates the numlumps/infotableofs reads on the WAD2 id
        // alone, and so does this — a 4-byte "WAD2" file reads 8 bytes past
        // the COM_LoadFile block, bug-for-bug with the C. The global is only
        // written on the init path.
        let (numlumps, ofs) = unsafe { read_wad_counts(base) };
        infotableofs = ofs;
        // SAFETY: the global is only written on the init path
        unsafe { wad_numlumps = numlumps };
    }

    // wrapping_offset, not offset: infotableofs is untrusted file data and
    // this runs *before* the bounds check below (the C leaves wad_lumps
    // pointing at the bogus address too, so it cannot be reordered).
    // `offset` is getelementptr inbounds -- immediate UB out of range --
    // whereas the C is a plain integer add.
    let lumps = base.wrapping_offset(infotableofs as isize) as *mut LumpInfo;
    // SAFETY: the global is only written on the init path
    unsafe { wad_lumps = lumps };

    // `wad_numlumps` stays the single source of truth for the count, written
    // at exactly the points the C writes it; the locals below are snapshots of
    // it, read on the single-threaded init path.
    // SAFETY: the global was just settled by the id check above
    let numlumps = unsafe { wad_numlumps };
    if header_extends_beyond(infotableofs, numlumps, file_len) {
        // SAFETY: engine C API; NUL-terminated format and argument
        unsafe {
            quake_c_sys::Con_Printf(
                c"Wad file %s header extends beyond end of file\n".as_ptr(),
                filename.as_ptr(),
            );
            wad_numlumps = 0;
        }
    }
    // SAFETY: as above — re-read because the bounds check may have zeroed it
    let numlumps = unsafe { wad_numlumps };

    let warn = |name: *const c_char, problem: LumpProblem| {
        // SAFETY: engine C API; `name` is the lump's 16-byte name field, which
        // the "%.16s" conversion never reads past
        unsafe {
            match problem {
                LumpProblem::BeginsBeyond { over } => quake_c_sys::Con_Printf(
                    c"Wad file %s lump \"%.16s\" begins %lld bytes beyond end of wad\n".as_ptr(),
                    filename.as_ptr(),
                    name,
                    over,
                ),
                LumpProblem::ExtendsBeyond { over, size } => quake_c_sys::Con_Printf(
                    c"Wad file %s lump \"%.16s\" extends %lld bytes beyond end of wad (lump size: %u)\n"
                        .as_ptr(),
                    filename.as_ptr(),
                    name,
                    over,
                    size,
                ),
            }
        }
    };

    for i in 0..numlumps.max(0) {
        // SAFETY: the directory holds `numlumps` entries within the image,
        // which header_extends_beyond just confirmed
        let lump = unsafe { lumps.add(i as usize) };
        // SAFETY: `lump` points at a lumpinfo_t inside the loaded image
        unsafe { fixup_lump(lump, file_len, false, warn) };
        // SAFETY: as above
        let type_ = unsafe { core::ptr::addr_of!((*lump).type_).read_unaligned() };
        if type_ == TYP_QPIC {
            // SAFETY: as above; wrapping_offset because filepos is untrusted
            // file data, exactly as at the directory offset above
            unsafe {
                let filepos = core::ptr::addr_of!((*lump).filepos).read_unaligned();
                SwapPic(base.wrapping_offset(filepos as isize) as *mut QPic);
            }
        }
    }
}

unsafe fn get_lumpinfo(
    lumps: *mut LumpInfo,
    numlumps: c_int,
    name: *const c_char,
) -> *mut LumpInfo {
    // SAFETY: `name` is a NUL-terminated string or a 16-byte name field
    let raw = unsafe { read_name(name) };
    let clean = cleanup_name(&raw);

    for i in 0..numlumps.max(0) {
        // SAFETY: lumps/numlumps describe a valid lump array
        let lump = unsafe { lumps.add(i as usize) };
        // SAFETY: the lump's `name` is a 16-byte field
        let lump_name: [u8; 16] =
            unsafe { *(core::ptr::addr_of!((*lump).name) as *const [u8; 16]) };
        // both sides are cleaned (zero-filled) 16-byte names, so C's
        // strcmp equality is exactly 16-byte equality
        if clean == lump_name {
            return lump;
        }
    }
    core::ptr::null_mut()
}

/// C: `void *W_GetLumpName (const char *name, lumpinfo_t **out_info);`
///
/// # Safety
/// `name` NUL-terminated; `out_info` writable. Only written when found, like C.
#[no_mangle]
pub unsafe extern "C" fn W_GetLumpName(
    name: *const c_char,
    out_info: *mut *mut LumpInfo,
) -> *mut c_void {
    // SAFETY: the globals describe the loaded gfx.wad after W_LoadWadFile;
    // `name` per the caller contract above
    let lump = unsafe { get_lumpinfo(wad_lumps, wad_numlumps, name) };
    if lump.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: `out_info` is writable per the contract above; the lump header
    // is inside the loaded image, and filepos is untrusted file data (hence
    // wrapping_offset, as in W_LoadWadFile)
    unsafe {
        *out_info = lump;
        let filepos = core::ptr::addr_of!((*lump).filepos).read_unaligned();
        wad_base.wrapping_offset(filepos as isize) as *mut c_void
    }
}

/// C: `void SwapPic (qpic_t *pic);`
///
/// # Safety
/// `pic` must point to a writable qpic_t (possibly unaligned, inside a wad).
#[no_mangle]
pub unsafe extern "C" fn SwapPic(pic: *mut QPic) {
    // SAFETY: caller contract; unaligned-safe accesses
    unsafe {
        let w = core::ptr::addr_of_mut!((*pic).width);
        w.write_unaligned(i32::from_le(w.read_unaligned()));
        let h = core::ptr::addr_of_mut!((*pic).height);
        h.write_unaligned(i32::from_le(h.read_unaligned()));
    }
}

unsafe fn open_wad_file(filename: *const c_char, fh: *mut quake_c_sys::fshandle_t) -> bool {
    let mut f: *mut quake_c_sys::FILE = core::ptr::null_mut();
    // SAFETY: engine C API; `filename` is NUL-terminated and `f` is a writable
    // out-parameter
    let length = unsafe { quake_c_sys::COM_FOpenFile(filename, &mut f, core::ptr::null_mut()) };
    if length == -1 {
        return false;
    }
    // SAFETY: `fh` points to a writable fshandle_t and `f` is the file just
    // opened, which is also what COM_ThreadFileFromPak reports on
    unsafe {
        (*fh).file = f;
        (*fh).start = quake_c_sys::Sys_ftell(f);
        (*fh).pos = 0;
        (*fh).length = length;
        (*fh).pak = quake_c_sys::COM_ThreadFileFromPak() != 0;
    }
    true
}

unsafe fn add_wad_file(name: *const c_char, fh: *mut quake_c_sys::fshandle_t) -> *mut Wad {
    let mut header = WadInfo {
        identification: [0; 4],
        numlumps: 0,
        infotableofs: 0,
    };
    // SAFETY: `header` is a writable wadinfo_t of exactly that size and `fh`
    // is an open handle; a short read leaves the zeroed fields, like the C
    unsafe {
        quake_c_sys::FS_fread(
            core::ptr::addr_of_mut!(header) as *mut c_void,
            1,
            core::mem::size_of::<WadInfo>(),
            fh,
        );
    }

    let id = i32::from_le_bytes([
        header.identification[0] as u8,
        header.identification[1] as u8,
        header.identification[2] as u8,
        header.identification[3] as u8,
    ]);
    let numlumps = i32::from_le(header.numlumps);
    let infotableofs = i32::from_le(header.infotableofs);

    match check_add_wad_header(id, numlumps, infotableofs) {
        AddWadVerdict::BadId => {
            // SAFETY: engine C API; NUL-terminated format and argument
            unsafe { quake_c_sys::Con_Warning(c"%s is not a valid WAD\n".as_ptr(), name) };
            return core::ptr::null_mut();
        }
        AddWadVerdict::BadCounts => {
            // SAFETY: engine C API; NUL-terminated format and argument
            unsafe {
                quake_c_sys::Con_Warning(
                    c"%s is not a valid WAD (%i lumps, %i info table offset)\n".as_ptr(),
                    name,
                    numlumps,
                    infotableofs,
                );
            }
            return core::ptr::null_mut();
        }
        AddWadVerdict::Empty => {
            // SAFETY: engine C API; NUL-terminated format and argument
            unsafe {
                quake_c_sys::Con_DPrintf2(c"WAD file %s has no lumps, ignored\n".as_ptr(), name);
            }
            return core::ptr::null_mut();
        }
        AddWadVerdict::Ok => {}
    }

    let table_bytes = numlumps as usize * core::mem::size_of::<LumpInfo>();
    // SAFETY: the verdict above is Ok, so numlumps is positive and the table
    // size fits; the block stays on Mem_Alloc because the lump array crosses
    // the FFI boundary (ADR-013), and it is exactly what FS_fread is told to
    // fill
    let lumps = unsafe {
        let lumps = quake_c_sys::Mem_Alloc(table_bytes) as *mut LumpInfo;
        quake_c_sys::FS_fseek(fh, infotableofs as i64, 0 /* SEEK_SET */);
        quake_c_sys::FS_fread(lumps as *mut c_void, 1, table_bytes, fh);
        lumps
    };
    // SAFETY: `fh` is an open handle, so its length field is readable
    let file_len = unsafe { (*fh).length };

    let warn = |lump_name: *const c_char, problem: LumpProblem| {
        // SAFETY: engine C API; `lump_name` is the lump's 16-byte name field,
        // which the "%.16s" conversion never reads past
        unsafe {
            match problem {
                LumpProblem::BeginsBeyond { over } => quake_c_sys::Con_Warning(
                    c"WAD file %s lump \"%.16s\" begins %lld bytes beyond end of WAD\n".as_ptr(),
                    name,
                    lump_name,
                    over,
                ),
                LumpProblem::ExtendsBeyond { over, size } => quake_c_sys::Con_Warning(
                    c"WAD file %s lump \"%.16s\" extends %lld bytes beyond end of WAD (lump size is %i)\n"
                        .as_ptr(),
                    name,
                    lump_name,
                    over,
                    size,
                ),
            }
        }
    };

    for i in 0..numlumps {
        // SAFETY: i < numlumps, the array just allocated
        let lump = unsafe { lumps.add(i as usize) };
        // SAFETY: `lump` points at a lumpinfo_t in that array
        unsafe { fixup_lump(lump, file_len, true, warn) };
    }

    // SAFETY: the block is sized for a wad_t and stays on Mem_Alloc because
    // the node crosses the FFI boundary (ADR-013); `name` is NUL-terminated,
    // so the truncated copy plus its terminator fit the 64-byte field
    let wad = unsafe {
        let wad = quake_c_sys::Mem_Alloc(core::mem::size_of::<Wad>()) as *mut Wad;
        let name_bytes = CStr::from_ptr(name).to_bytes();
        let name_field = core::ptr::addr_of_mut!((*wad).name) as *mut u8;
        let n = name_bytes.len().min(63);
        core::ptr::copy_nonoverlapping(name_bytes.as_ptr(), name_field, n);
        *name_field.add(n) = 0;
        (*wad).id = id;
        (*wad).fh = *fh;
        (*wad).numlumps = numlumps;
        (*wad).lumps = lumps;
        wad
    };

    // SAFETY: engine C API; NUL-terminated format and argument
    unsafe { quake_c_sys::Con_DPrintf(c"%s\n".as_ptr(), name) };
    wad
}

/// C: `wad_t *W_LoadWadList (const char *names);` — `;`-separated list from
/// the worldspawn "wad" key; wads are prepended, so the returned list is in
/// reverse order.
///
/// # Safety
/// `names` must be a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn W_LoadWadList(names: *const c_char) -> *mut Wad {
    // SAFETY: NUL-terminated per the wad.h contract
    let all = unsafe { CStr::from_ptr(names) }.to_bytes();
    let mut wads: *mut Wad = core::ptr::null_mut();

    let mut rest = all;
    loop {
        // C: `for (name = newnames; name && *name;)` — an empty remainder
        // ends the loop; an empty middle segment is still processed
        // (yielding COM_FileBase's "?model?")
        if rest.is_empty() {
            break;
        }
        let (seg, next) = match rest.iter().position(|&b| b == b';') {
            Some(p) => (&rest[..p], Some(&rest[p + 1..])),
            None => (rest, None),
        };

        // remove all of the leading garbage left by the map editor
        let mut seg_z: Vec<u8> = seg.to_vec();
        seg_z.push(0);
        let mut filename = [0 as c_char; 64]; // MAX_QPATH

        // SAFETY: engine C API; `seg_z` is NUL-terminated and `filename` is
        // writable for the 64 bytes both calls are told about
        unsafe {
            quake_c_sys::COM_FileBase(seg_z.as_ptr() as *const c_char, filename.as_mut_ptr(), 64);
            quake_c_sys::COM_AddExtension(filename.as_mut_ptr(), c".wad".as_ptr(), 64);
        }

        // SAFETY: fshandle_t is plain C data, so an all-zero value is a
        // valid (closed) handle; the C leaves this local uninitialised and
        // only reads it once open_wad_file has filled it in
        let mut fh: quake_c_sys::fshandle_t = unsafe { core::mem::zeroed() };
        // SAFETY: `filename` is NUL-terminated and `fh` is writable
        let mut opened = unsafe { open_wad_file(filename.as_ptr(), &mut fh) };
        if !opened {
            // try the "gfx" directory: C memmoves the name up 4 bytes and
            // stamps "gfx/", re-terminating the last byte
            // both pointers come from one `as_mut_ptr`: spelling this as
            // copy(filename.as_ptr(), filename.as_mut_ptr().add(4), ..) reads
            // through a tag the second retag has already invalidated, which
            // Miri/Stacked Borrows rejects
            let p = filename.as_mut_ptr();
            // SAFETY: source and destination are both inside `filename` and
            // 4 + 60 == 64; `copy` is the overlap-tolerant memmove
            unsafe { core::ptr::copy(p, p.add(4), 60) };
            filename[0] = b'g' as c_char;
            filename[1] = b'f' as c_char;
            filename[2] = b'x' as c_char;
            filename[3] = b'/' as c_char;
            filename[63] = 0;
            // SAFETY: as above
            opened = unsafe { open_wad_file(filename.as_ptr(), &mut fh) };
        }

        if opened {
            // SAFETY: `fh` is now an open handle and `filename` NUL-terminated
            let wad = unsafe { add_wad_file(filename.as_ptr(), &mut fh) };
            if !wad.is_null() {
                // SAFETY: `wad` was just allocated by add_wad_file
                unsafe { (*wad).next = wads };
                wads = wad;
            } else {
                // SAFETY: add_wad_file rejected the wad without taking the
                // handle, so it is still open
                unsafe { quake_c_sys::FS_fclose(&mut fh) };
            }
        }

        match next {
            Some(n) => rest = n,
            None => break,
        }
    }

    wads
}

/// C: `void W_FreeWadList (wad_t *wads);`
///
/// # Safety
/// `wads` must be a list from W_LoadWadList (or NULL).
#[no_mangle]
pub unsafe extern "C" fn W_FreeWadList(mut wads: *mut Wad) {
    while !wads.is_null() {
        // SAFETY: list nodes and lump arrays were Mem_Alloc'd by add_wad_file
        // and the handle is still open; `next` is read before the node is
        // freed
        let next = unsafe {
            quake_c_sys::FS_fclose(core::ptr::addr_of_mut!((*wads).fh));
            quake_c_sys::Mem_Free((*wads).lumps as *const c_void);
            let next = (*wads).next;
            quake_c_sys::Mem_Free(wads as *const c_void);
            next
        };
        wads = next;
    }
}

/// C: `lumpinfo_t *W_GetLumpinfoList (wad_t *wads, const char *name, wad_t **out_wad);`
///
/// # Safety
/// `wads` a valid list or NULL; `name` NUL-terminated; `out_wad` writable
/// (only written when found, like C).
#[no_mangle]
pub unsafe extern "C" fn W_GetLumpinfoList(
    mut wads: *mut Wad,
    name: *const c_char,
    out_wad: *mut *mut Wad,
) -> *mut LumpInfo {
    while !wads.is_null() {
        // SAFETY: `wads` is a list node from W_LoadWadList, so its lump array
        // is valid; `name` per the caller contract above
        let info = unsafe { get_lumpinfo((*wads).lumps, (*wads).numlumps, name) };
        if !info.is_null() {
            // SAFETY: `out_wad` is writable per the contract above
            unsafe { *out_wad = wads };
            return info;
        }
        // SAFETY: `wads` is a list node
        wads = unsafe { (*wads).next };
    }
    core::ptr::null_mut()
}
