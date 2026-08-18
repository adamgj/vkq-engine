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
    unsafe {
        let cleaned = cleanup_name(&read_name(p));
        core::ptr::copy_nonoverlapping(cleaned.as_ptr() as *const c_char, p, 16);
    }
}

/// C: `void W_CleanupName (const char *in, char *out);`
///
/// # Safety
/// `in_` must be a NUL-terminated string or at least 16 readable bytes;
/// `out` must be writable for 16 bytes. In-place calls are fine, like C.
#[no_mangle]
pub unsafe extern "C" fn W_CleanupName(in_: *const c_char, out: *mut c_char) {
    // SAFETY: caller contract above; read completes before the write
    unsafe {
        let cleaned = cleanup_name(&read_name(in_));
        core::ptr::copy_nonoverlapping(cleaned.as_ptr() as *const c_char, out, 16);
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
    // SAFETY: lump points at a lumpinfo_t inside a readable/writable buffer;
    // reads/writes are unaligned-safe
    unsafe {
        if cleanup_first {
            cleanup_name_in_place(core::ptr::addr_of_mut!((*lump).name) as *mut c_char);
        }
        let mut filepos = i32::from_le(core::ptr::addr_of!((*lump).filepos).read_unaligned());
        let mut size = i32::from_le(core::ptr::addr_of!((*lump).size).read_unaligned());
        let disksize = i32::from_le(core::ptr::addr_of!((*lump).disksize).read_unaligned());
        let problem = repair_lump(&mut filepos, &mut size, disksize, file_len);
        if let Some(problem) = problem {
            warn(core::ptr::addr_of!((*lump).name) as *const c_char, problem);
        }
        core::ptr::addr_of_mut!((*lump).filepos).write_unaligned(filepos);
        core::ptr::addr_of_mut!((*lump).size).write_unaligned(size);
        if !cleanup_first {
            cleanup_name_in_place(core::ptr::addr_of_mut!((*lump).name) as *mut c_char);
        }
    }
}

/// C: `void W_LoadWadFile (void);` — loads the global gfx.wad.
///
/// # Safety
/// Engine single-threaded init path, like the C original.
#[no_mangle]
pub unsafe extern "C" fn W_LoadWadFile() {
    let filename = c"gfx.wad";
    // SAFETY: engine C API calls with valid arguments; wad_base ownership is
    // this function's, exactly as in the C original
    unsafe {
        if !wad_base.is_null() {
            quake_c_sys::Mem_Free(wad_base as *const c_void);
        }
        wad_base = quake_c_sys::COM_LoadFile(filename.as_ptr(), core::ptr::null_mut());
        if wad_base.is_null() {
            quake_c_sys::Sys_Error(
                c"W_LoadWadFile: couldn't load %s\n\nBasedir is: %s\n\nCheck that this has an id1 subdirectory containing pak0.pak and pak1.pak, or use the -basedir command-line option to specify another directory."
                    .as_ptr(),
                filename.as_ptr(),
                core::ptr::addr_of!(quake_c_sys::manual::com_basedir) as *const c_char,
            );
        }
        let file_len = quake_c_sys::COM_ThreadFileSize();

        let header = wad_base as *const WadInfo;
        let ident = core::ptr::addr_of!((*header).identification).read_unaligned();
        let ident_bytes = [
            ident[0] as u8,
            ident[1] as u8,
            ident[2] as u8,
            ident[3] as u8,
        ];
        let mut infotableofs: i32 = 0;
        if !wad2_id_ok(&ident_bytes) {
            quake_c_sys::Con_Printf(
                c"Wad file %s doesn't have WAD2 id\n".as_ptr(),
                filename.as_ptr(),
            );
            wad_numlumps = 0;
        } else {
            wad_numlumps = i32::from_le(core::ptr::addr_of!((*header).numlumps).read_unaligned());
            infotableofs =
                i32::from_le(core::ptr::addr_of!((*header).infotableofs).read_unaligned());
        }
        wad_lumps = wad_base.offset(infotableofs as isize) as *mut LumpInfo;
        if header_extends_beyond(infotableofs, wad_numlumps, file_len) {
            quake_c_sys::Con_Printf(
                c"Wad file %s header extends beyond end of file\n".as_ptr(),
                filename.as_ptr(),
            );
            wad_numlumps = 0;
        }

        for i in 0..wad_numlumps.max(0) {
            let lump = wad_lumps.add(i as usize);
            fixup_lump(lump, file_len, false, |name, problem| {
                match problem {
                LumpProblem::BeginsBeyond { over } => quake_c_sys::Con_Printf(
                    c"Wad file %s lump \"%.16s\" begins %lld bytes beyond end of wad\n".as_ptr(),
                    filename.as_ptr(),
                    name,
                    over,
                ),
                LumpProblem::ExtendsBeyond { over, size } => quake_c_sys::Con_Printf(
                    c"Wad file %s lump \"%.16s\" extends %lld bytes beyond end of wad (lump size: %u)\n".as_ptr(),
                    filename.as_ptr(),
                    name,
                    over,
                    size,
                ),
            }
            });
            if core::ptr::addr_of!((*lump).type_).read_unaligned() == TYP_QPIC {
                let filepos = core::ptr::addr_of!((*lump).filepos).read_unaligned();
                SwapPic(wad_base.offset(filepos as isize) as *mut QPic);
            }
        }
    }
}

unsafe fn get_lumpinfo(
    lumps: *mut LumpInfo,
    numlumps: c_int,
    name: *const c_char,
) -> *mut LumpInfo {
    // SAFETY: lumps/numlumps describe a valid lump array; names are 16-byte
    // fields compared with C strcmp semantics on cleaned names
    unsafe {
        let clean = cleanup_name(&read_name(name));
        for i in 0..numlumps.max(0) {
            let lump = lumps.add(i as usize);
            let lump_name: [u8; 16] = *(core::ptr::addr_of!((*lump).name) as *const [u8; 16]);
            // both sides are cleaned (zero-filled) 16-byte names, so C's
            // strcmp equality is exactly 16-byte equality
            if clean == lump_name {
                return lump;
            }
        }
        core::ptr::null_mut()
    }
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
    // SAFETY: globals valid after W_LoadWadFile; caller contract above
    unsafe {
        let lump = get_lumpinfo(wad_lumps, wad_numlumps, name);
        if lump.is_null() {
            return core::ptr::null_mut();
        }
        *out_info = lump;
        let filepos = core::ptr::addr_of!((*lump).filepos).read_unaligned();
        wad_base.offset(filepos as isize) as *mut c_void
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
    // SAFETY: engine C API; fh points to a writable fshandle_t
    unsafe {
        let mut f: *mut quake_c_sys::FILE = core::ptr::null_mut();
        let length = quake_c_sys::COM_FOpenFile(filename, &mut f, core::ptr::null_mut());
        if length == -1 {
            return false;
        }
        (*fh).file = f;
        (*fh).start = quake_c_sys::Sys_ftell(f);
        (*fh).pos = 0;
        (*fh).length = length;
        (*fh).pak = quake_c_sys::COM_ThreadFileFromPak() != 0;
        true
    }
}

unsafe fn add_wad_file(name: *const c_char, fh: *mut quake_c_sys::fshandle_t) -> *mut Wad {
    // SAFETY: fh is an open handle; engine C API calls with valid arguments;
    // allocations use Mem_Alloc since the wad_t/lump list crosses the FFI
    // boundary (ADR-013)
    unsafe {
        let mut header = WadInfo {
            identification: [0; 4],
            numlumps: 0,
            infotableofs: 0,
        };
        quake_c_sys::FS_fread(
            core::ptr::addr_of_mut!(header) as *mut c_void,
            1,
            core::mem::size_of::<WadInfo>(),
            fh,
        );

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
                quake_c_sys::Con_Warning(c"%s is not a valid WAD\n".as_ptr(), name);
                return core::ptr::null_mut();
            }
            AddWadVerdict::BadCounts => {
                quake_c_sys::Con_Warning(
                    c"%s is not a valid WAD (%i lumps, %i info table offset)\n".as_ptr(),
                    name,
                    numlumps,
                    infotableofs,
                );
                return core::ptr::null_mut();
            }
            AddWadVerdict::Empty => {
                quake_c_sys::Con_DPrintf2(c"WAD file %s has no lumps, ignored\n".as_ptr(), name);
                return core::ptr::null_mut();
            }
            AddWadVerdict::Ok => {}
        }

        let lumps = quake_c_sys::Mem_Alloc(numlumps as usize * core::mem::size_of::<LumpInfo>())
            as *mut LumpInfo;
        quake_c_sys::FS_fseek(fh, infotableofs as i64, 0 /* SEEK_SET */);
        quake_c_sys::FS_fread(
            lumps as *mut c_void,
            1,
            numlumps as usize * core::mem::size_of::<LumpInfo>(),
            fh,
        );

        for i in 0..numlumps {
            let lump = lumps.add(i as usize);
            fixup_lump(lump, (*fh).length, true, |lump_name, problem| {
                match problem {
                LumpProblem::BeginsBeyond { over } => quake_c_sys::Con_Warning(
                    c"WAD file %s lump \"%.16s\" begins %lld bytes beyond end of WAD\n".as_ptr(),
                    name,
                    lump_name,
                    over,
                ),
                LumpProblem::ExtendsBeyond { over, size } => quake_c_sys::Con_Warning(
                    c"WAD file %s lump \"%.16s\" extends %lld bytes beyond end of WAD (lump size is %i)\n".as_ptr(),
                    name,
                    lump_name,
                    over,
                    size,
                ),
            }
            });
        }

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

        quake_c_sys::Con_DPrintf(c"%s\n".as_ptr(), name);
        wad
    }
}

/// C: `wad_t *W_LoadWadList (const char *names);` — `;`-separated list from
/// the worldspawn "wad" key; wads are prepended, so the returned list is in
/// reverse order.
///
/// # Safety
/// `names` must be a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn W_LoadWadList(names: *const c_char) -> *mut Wad {
    // SAFETY: names NUL-terminated per the wad.h contract; engine C API calls
    unsafe {
        let all = CStr::from_ptr(names).to_bytes();
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
            quake_c_sys::COM_FileBase(seg_z.as_ptr() as *const c_char, filename.as_mut_ptr(), 64);
            quake_c_sys::COM_AddExtension(filename.as_mut_ptr(), c".wad".as_ptr(), 64);

            let mut fh: quake_c_sys::fshandle_t = core::mem::zeroed();
            let mut opened = open_wad_file(filename.as_ptr(), &mut fh);
            if !opened {
                // try the "gfx" directory: C memmoves the name up 4 bytes and
                // stamps "gfx/", re-terminating the last byte
                core::ptr::copy(filename.as_ptr(), filename.as_mut_ptr().add(4), 60);
                filename[0] = b'g' as c_char;
                filename[1] = b'f' as c_char;
                filename[2] = b'x' as c_char;
                filename[3] = b'/' as c_char;
                filename[63] = 0;
                opened = open_wad_file(filename.as_ptr(), &mut fh);
            }

            if opened {
                let wad = add_wad_file(filename.as_ptr(), &mut fh);
                if !wad.is_null() {
                    (*wad).next = wads;
                    wads = wad;
                } else {
                    quake_c_sys::FS_fclose(&mut fh);
                }
            }

            match next {
                Some(n) => rest = n,
                None => break,
            }
        }

        wads
    }
}

/// C: `void W_FreeWadList (wad_t *wads);`
///
/// # Safety
/// `wads` must be a list from W_LoadWadList (or NULL).
#[no_mangle]
pub unsafe extern "C" fn W_FreeWadList(mut wads: *mut Wad) {
    // SAFETY: list nodes and lump arrays were Mem_Alloc'd by add_wad_file
    unsafe {
        while !wads.is_null() {
            quake_c_sys::FS_fclose(core::ptr::addr_of_mut!((*wads).fh));
            quake_c_sys::Mem_Free((*wads).lumps as *const c_void);
            let next = (*wads).next;
            quake_c_sys::Mem_Free(wads as *const c_void);
            wads = next;
        }
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
    // SAFETY: caller contract above
    unsafe {
        while !wads.is_null() {
            let info = get_lumpinfo((*wads).lumps, (*wads).numlumps, name);
            if !info.is_null() {
                *out_wad = wads;
                return info;
            }
            wads = (*wads).next;
        }
        core::ptr::null_mut()
    }
}
