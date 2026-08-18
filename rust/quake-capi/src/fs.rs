//! COM_* filesystem shims (Quake/common_fs.c): searchpaths, pak mounting,
//! gamedir/flavor logic, store-install discovery, pref files.
//!
//! The Rust side owns the searchpath globals under -Duse_rust_fs; C code
//! (host_cmd.c, pr_ext.c, ...) walks them directly, so every node is
//! allocated with the C layout via `Mem_*` (ADR-013 boundary rule) and every
//! byte written matches what common_fs.c would have written. Decision logic
//! lives in `quake_fs`; this file is plumbing and state.

use core::ffi::{c_char, c_int, c_uint, c_void, CStr};
use core::ptr;

use quake_c_sys as sys;
use quake_fs::{flavor, pak, searchpath as sp};
use quake_types::fs::{Pack, PackFile, SearchPath, MAX_OSPATH};

pub const MAX_BASEDIRS: usize = 4;
const FS_ENT_FILE: c_int = 1 << 0;
const FS_ENT_DIRECTORY: c_int = 1 << 1;
const SEEK_SET: c_int = 0;
const GAMENAME: &CStr = c"id1";
const CONFIG_NAME: &CStr = c"vkQuake.cfg";

// Layout parity with quake-c-sys' hand-declared platform constant
const _: () = assert!(MAX_OSPATH == sys::MAX_OSPATH);

/// C: `qboolean com_modified` (set true if using non-id files; also written
/// by the C COM_Game_f)
#[no_mangle]
pub static mut com_modified: bool = false;

/// C: `qboolean standard_quake = true, rogue, hipnotic;`
#[no_mangle]
pub static mut standard_quake: bool = true;
#[no_mangle]
pub static mut rogue: bool = false;
#[no_mangle]
pub static mut hipnotic: bool = false;

/// C: `char com_gamenames[1024];` — eg "hipnotic;quoth;warp", no id1
#[no_mangle]
pub static mut com_gamenames: [c_char; 1024] = [0; 1024];
/// C: `char com_gamedir[MAX_OSPATH];`
#[no_mangle]
pub static mut com_gamedir: [c_char; MAX_OSPATH] = [0; MAX_OSPATH];
/// C: `char com_basedir[MAX_OSPATH];`
#[no_mangle]
pub static mut com_basedir: [c_char; MAX_OSPATH] = [0; MAX_OSPATH];
/// C: `char com_basedirs[MAX_BASEDIRS][MAX_OSPATH];`
#[no_mangle]
pub static mut com_basedirs: [[c_char; MAX_OSPATH]; MAX_BASEDIRS] = [[0; MAX_OSPATH]; MAX_BASEDIRS];
/// C: `int com_numbasedirs;`
#[no_mangle]
pub static mut com_numbasedirs: c_int = 0;

/// C: `searchpath_t *com_searchpaths;` / `*com_base_searchpaths;`
#[no_mangle]
pub static mut com_searchpaths: *mut SearchPath = ptr::null_mut();
#[no_mangle]
pub static mut com_base_searchpaths: *mut SearchPath = ptr::null_mut();

// ---------------------------------------------------------------------------
// small C-string helpers (single-threaded engine: all callers are the main
// thread, like the C they replace)

/// Borrow a NUL-terminated C string as bytes (no allocation).
///
/// # Safety
/// `p` must be a valid NUL-terminated string.
unsafe fn cstr_bytes<'a>(p: *const c_char) -> &'a [u8] {
    // SAFETY: caller guarantees p is a valid NUL-terminated C string.
    unsafe { CStr::from_ptr(p) }.to_bytes()
}

fn bytes_of(buf: &[c_char]) -> &[u8] {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    // SAFETY-free cast: c_char and u8 have identical layout
    let (head, _) = buf.split_at(len);
    // c_char is i8 on most targets; reinterpret element-wise without unsafe
    // by copying is wasteful; use a checked transmute-free view:
    unsafe {
        // SAFETY: i8 and u8 have the same size/alignment; the slice covers
        // initialized memory of `len` elements.
        core::slice::from_raw_parts(head.as_ptr().cast::<u8>(), head.len())
    }
}

/// q_strlcpy: bounded copy + NUL, truncating like the C.
fn strlcpy_into(dst: &mut [c_char], src: &[u8]) {
    let n = src.len().min(dst.len() - 1);
    for (d, &s) in dst.iter_mut().zip(&src[..n]) {
        *d = s as c_char;
    }
    dst[n] = 0;
}

/// q_snprintf "%s/%s" with truncation; returns the would-be length like C
/// snprintf so callers can replicate overflow checks.
fn path_join_into(dst: &mut [c_char], a: &[u8], b: &[u8]) -> usize {
    let mut tmp = Vec::with_capacity(a.len() + 1 + b.len());
    tmp.extend_from_slice(a);
    tmp.push(b'/');
    tmp.extend_from_slice(b);
    let would_be = tmp.len();
    strlcpy_into(dst, &tmp);
    would_be
}

fn eq_ignore_case(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.eq_ignore_ascii_case(y))
}

/// Allocate a zeroed C-layout object on the engine heap (Mem_Alloc zeroes).
unsafe fn mem_alloc_zeroed<T>() -> *mut T {
    // SAFETY: Mem_Alloc returns memory valid for size_of::<T>() bytes,
    // zero-initialized; T is a repr(C) plain-old-data type here.
    unsafe { sys::Mem_Alloc(core::mem::size_of::<T>()) }.cast()
}

// ---------------------------------------------------------------------------

/// C: `void COM_AddBaseDir (const char *dir)`
///
/// # Safety
/// `dir` must be a valid NUL-terminated string. Main thread only (mutates
/// the searchpath globals), like the C.
#[no_mangle]
pub unsafe extern "C" fn COM_AddBaseDir(dir: *const c_char) {
    // SAFETY: dir is a valid C string per the contract above.
    let dirb = unsafe { cstr_bytes(dir) };
    // SAFETY: main-thread-only globals, matching the C's un-synchronized use.
    unsafe {
        for i in 0..com_numbasedirs as usize {
            if eq_ignore_case(bytes_of(&(*ptr::addr_of!(com_basedirs))[i]), dirb) {
                return;
            }
        }
        if com_numbasedirs as usize == MAX_BASEDIRS {
            sys::Sys_Error(c"COM_AddBaseDir: too many base directories".as_ptr());
        }
        let slot = com_numbasedirs as usize;
        com_numbasedirs += 1;
        strlcpy_into(&mut (*ptr::addr_of_mut!(com_basedirs))[slot], dirb);
    }
}

/// C: `static void COM_Path_f (void)` — the "path" console command.
extern "C" fn com_path_f() {
    // SAFETY: main thread; walks the Rust-owned searchpath list whose nodes
    // are valid until the list is rebuilt.
    unsafe {
        sys::Con_Printf(c"Current search path:\n".as_ptr());
        let mut s = com_searchpaths;
        while !s.is_null() {
            let pack = (*s).pack;
            if !pack.is_null() {
                sys::Con_Printf(
                    c"%s (%i files)\n".as_ptr(),
                    (*pack).filename.as_ptr(),
                    (*pack).numfiles,
                );
            } else {
                sys::Con_Printf(c"%s\n".as_ptr(), (*s).filename.as_ptr());
            }
            s = (*s).next;
        }
    }
}

/// C: `void COM_WriteFile (const char *filename, const void *data, int len)`
///
/// # Safety
/// `filename` must be a valid C string; `data` must point to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn COM_WriteFile(filename: *const c_char, data: *const c_void, len: c_int) {
    let mut name = [0 as c_char; MAX_OSPATH];
    // SAFETY: filename valid per contract; com_gamedir is the engine global.
    unsafe {
        path_join_into(
            &mut name,
            bytes_of(&*ptr::addr_of!(com_gamedir)),
            cstr_bytes(filename),
        );
        let handle = sys::Sys_FileOpenWrite(name.as_ptr());
        if handle == -1 {
            sys::Sys_Printf(c"COM_WriteFile: failed on %s\n".as_ptr(), name.as_ptr());
            return;
        }
        sys::Sys_Printf(c"COM_WriteFile: %s\n".as_ptr(), name.as_ptr());
        sys::Sys_FileWrite(handle, data, len);
        sys::Sys_FileClose(handle);
    }
}

/// C: `static qfilesize_t COM_FindFile (...)` — the searchpath walk.
///
/// # Safety
/// Pointer args as in C: at most one of handle/file non-null; main thread.
unsafe fn com_find_file(
    filename: *const c_char,
    handle: *mut c_int,
    file: *mut *mut sys::FILE,
    path_id: *mut c_uint,
) -> sys::qfilesize_t {
    // SAFETY: engine contract, same as the C's Sys_Error on misuse.
    unsafe {
        if !file.is_null() && !handle.is_null() {
            sys::Sys_Error(c"COM_FindFile: both handle and file set".as_ptr());
        }

        let fnameb = cstr_bytes(filename);
        // C: is_config = !q_strcasecmp (filename, "config.cfg")
        let is_config = eq_ignore_case(fnameb, b"config.cfg");

        sys::COM_SetThreadFileFromPak(0);

        let mut netpath = [0 as c_char; MAX_OSPATH];
        let mut search = com_searchpaths;
        while !search.is_null() {
            let pack = (*search).pack;
            if !pack.is_null() {
                // look through all the pak file elements
                let files =
                    core::slice::from_raw_parts((*pack).files, (*pack).numfiles.max(0) as usize);
                for pf in files {
                    // case-SENSITIVE strcmp, like the C
                    if bytes_of(&pf.name) != fnameb {
                        continue;
                    }
                    sys::COM_SetThreadFileSize(pf.filelen as sys::qfilesize_t);
                    sys::COM_SetThreadFileFromPak(1);
                    if !path_id.is_null() {
                        *path_id = (*search).path_id;
                    }
                    if !handle.is_null() {
                        // concurrent pak reads need an independent handle
                        let new_handle = sys::Sys_DuplicateHandle((*pack).handle);
                        if new_handle < 0 {
                            sys::Sys_Error(
                                c"COM_FindFile: couldn't reopen %s".as_ptr(),
                                (*pack).filename.as_ptr(),
                            );
                        }
                        sys::Sys_FileSeek(new_handle, pf.filepos as sys::qfileofs_t);
                        *handle = new_handle;
                        return pf.filelen as sys::qfilesize_t;
                    } else if !file.is_null() {
                        // open a new FILE* on the pakfile
                        *file = sys::Sys_fopen((*pack).filename.as_ptr(), c"rb".as_ptr());
                        if !(*file).is_null() {
                            sys::Sys_fseek(*file, pf.filepos as sys::qfileofs_t, SEEK_SET);
                        }
                        return pf.filelen as sys::qfilesize_t;
                    } else {
                        // for COM_FileExists()
                        return pf.filelen as sys::qfilesize_t;
                    }
                }
            } else {
                // check a file in the directory tree
                if registered_value() == 0.0 {
                    // shareware: don't ever go beyond base
                    if fnameb.contains(&b'/') || fnameb.contains(&b'\\') {
                        search = (*search).next;
                        continue;
                    }
                }

                let mut found = false;
                if is_config {
                    path_join_into(
                        &mut netpath,
                        bytes_of(&(*search).filename),
                        CONFIG_NAME.to_bytes(),
                    );
                    if sys::Sys_FileType(netpath.as_ptr()) & FS_ENT_FILE != 0 {
                        found = true;
                    }
                }
                if !found {
                    path_join_into(&mut netpath, bytes_of(&(*search).filename), fnameb);
                    if sys::Sys_FileType(netpath.as_ptr()) & FS_ENT_FILE == 0 {
                        search = (*search).next;
                        continue;
                    }
                }

                if !path_id.is_null() {
                    *path_id = (*search).path_id;
                }
                if !handle.is_null() {
                    let mut h: c_int = 0;
                    let size = sys::Sys_FileOpenRead(netpath.as_ptr(), &mut h);
                    sys::COM_SetThreadFileSize(size);
                    *handle = h;
                    return size;
                } else if !file.is_null() {
                    *file = sys::Sys_fopen(netpath.as_ptr(), c"rb".as_ptr());
                    let size = if (*file).is_null() {
                        -1
                    } else {
                        sys::Sys_filelength(*file)
                    };
                    sys::COM_SetThreadFileSize(size);
                    return size;
                } else {
                    // dummy valid value for COM_FileExists()
                    return 0;
                }
            }
            search = (*search).next;
        }

        if sys::developer.value > 1.0 {
            sys::Con_DPrintf(c"FindFile: can't find %s\n".as_ptr(), filename);
        }

        if !handle.is_null() {
            *handle = -1;
        }
        if !file.is_null() {
            *file = ptr::null_mut();
        }
        sys::COM_SetThreadFileSize(-1);
        -1
    }
}

fn registered_value() -> f32 {
    // SAFETY: registered is the engine cvar global, main-thread access.
    unsafe { sys::registered.value }
}

/// C: `qboolean COM_FileExists (const char *filename, unsigned int *path_id)`
///
/// # Safety
/// `filename` valid C string; `path_id` null or valid.
#[no_mangle]
pub unsafe extern "C" fn COM_FileExists(filename: *const c_char, path_id: *mut c_uint) -> bool {
    // SAFETY: forwarded contract.
    let ret = unsafe { com_find_file(filename, ptr::null_mut(), ptr::null_mut(), path_id) };
    ret != -1
}

/// C: `qfilesize_t COM_OpenFile (const char *filename, int *handle, unsigned int *path_id)`
///
/// # Safety
/// As in C: `handle` must be valid; `path_id` null or valid.
#[no_mangle]
pub unsafe extern "C" fn COM_OpenFile(
    filename: *const c_char,
    handle: *mut c_int,
    path_id: *mut c_uint,
) -> sys::qfilesize_t {
    // SAFETY: forwarded contract.
    unsafe { com_find_file(filename, handle, ptr::null_mut(), path_id) }
}

/// C: `qfilesize_t COM_FOpenFile (const char *filename, FILE **file, unsigned int *path_id)`
///
/// # Safety
/// As in C: `file` must be valid; `path_id` null or valid.
#[no_mangle]
pub unsafe extern "C" fn COM_FOpenFile(
    filename: *const c_char,
    file: *mut *mut sys::FILE,
    path_id: *mut c_uint,
) -> sys::qfilesize_t {
    // SAFETY: forwarded contract.
    unsafe { com_find_file(filename, ptr::null_mut(), file, path_id) }
}

/// C: `void COM_CloseFile (int h)` — pak handles are never really closed.
///
/// # Safety
/// Main thread (walks the searchpath list).
#[no_mangle]
pub unsafe extern "C" fn COM_CloseFile(h: c_int) {
    if h < 0 {
        return;
    }
    // SAFETY: main-thread list walk over valid nodes.
    unsafe {
        let mut s = com_searchpaths;
        while !s.is_null() {
            let pack = (*s).pack;
            if !pack.is_null() && (*pack).handle == h {
                return;
            }
            s = (*s).next;
        }
        sys::Sys_FileClose(h);
    }
}

/// C: `byte *COM_LoadFile (const char *path, unsigned int *path_id)`
/// Always appends a 0 byte; buffer is Mem-owned (caller Mem_Free's).
///
/// # Safety
/// `path` valid C string; `path_id` null or valid.
#[no_mangle]
pub unsafe extern "C" fn COM_LoadFile(path: *const c_char, path_id: *mut c_uint) -> *mut u8 {
    // SAFETY: forwarded contract; Mem buffer ownership crosses to the caller
    // per ADR-013 (allocated with Mem_AllocNonZero exactly like the C).
    unsafe {
        let mut h: c_int = 0;
        let len = com_find_file(path, &mut h, ptr::null_mut(), path_id);
        if h == -1 {
            return ptr::null_mut();
        }
        let buf = sys::Mem_AllocNonZero((len + 1) as usize).cast::<u8>();
        if buf.is_null() {
            sys::Sys_Error(c"COM_LoadFile: not enough space for %s".as_ptr(), path);
        }
        *buf.add(len as usize) = 0;
        sys::Sys_FileRead(h, buf.cast(), len as c_int);
        COM_CloseFile(h);
        buf
    }
}

/// C: `byte *COM_LoadMallocFile_TextMode_OSPath (const char *path, long *len_out)`
/// Text-mode load (CRLF→LF on Windows), ignores searchpaths.
///
/// # Safety
/// `path` valid C string; `len_out` null or valid.
#[no_mangle]
pub unsafe extern "C" fn COM_LoadMallocFile_TextMode_OSPath(
    path: *const c_char,
    len_out: *mut core::ffi::c_long,
) -> *mut u8 {
    // SAFETY: forwarded contract; stdio calls mirror the C exactly.
    unsafe {
        let f = sys::Sys_fopen(path, c"rt".as_ptr());
        if f.is_null() {
            return ptr::null_mut();
        }
        let len = sys::Sys_filelength(f);
        if len < 0 {
            sys::stdio::fclose(f);
            return ptr::null_mut();
        }
        let data = sys::Mem_AllocNonZero((len + 1) as usize).cast::<u8>();
        if data.is_null() {
            sys::stdio::fclose(f);
            return ptr::null_mut();
        }
        // (actuallen < len) if CRLF to LF translation was performed
        let actuallen = sys::stdio::fread(data.cast(), 1, len as usize, f);
        if sys::stdio::ferror(f) != 0 {
            sys::stdio::fclose(f);
            sys::Mem_Free(data.cast());
            return ptr::null_mut();
        }
        *data.add(actuallen) = 0;
        if !len_out.is_null() {
            *len_out = actuallen as core::ffi::c_long;
        }
        sys::stdio::fclose(f);
        data
    }
}

/// C: `static pack_t *COM_LoadPackFile (const char *packfile, int packhandle)`
unsafe fn load_pack_file(packfile: *const c_char, packhandle: c_int) -> *mut Pack {
    // SAFETY: packfile is valid; packhandle is an open engine file handle.
    unsafe {
        let mut header = quake_types::fs::DPackHeader {
            id: [0; 4],
            dirofs: 0,
            dirlen: 0,
        };
        sys::Sys_FileRead(
            packhandle,
            (&mut header as *mut quake_types::fs::DPackHeader).cast(),
            core::mem::size_of::<quake_types::fs::DPackHeader>() as c_int,
        );
        // engine targets are little-endian (COM_Init asserts); LittleLong is
        // the identity here, like the C at runtime
        let id = [
            header.id[0] as u8,
            header.id[1] as u8,
            header.id[2] as u8,
            header.id[3] as u8,
        ];
        let numpackfiles = match pak::check_header(id, header.dirofs, header.dirlen) {
            Err(pak::PakError::NotAPackfile) => {
                sys::Sys_Error(c"%s is not a packfile".as_ptr(), packfile);
            }
            Err(pak::PakError::InvalidDirectory { dirlen, dirofs }) => {
                sys::Sys_Error(
                    c"Invalid packfile %s (dirlen: %i, dirofs: %i)".as_ptr(),
                    packfile,
                    dirlen,
                    dirofs,
                );
            }
            Err(pak::PakError::TooManyFiles(n)) => {
                sys::Sys_Error(c"%s has %i files".as_ptr(), packfile, n);
            }
            Ok(Err(pak::PakEmpty)) => {
                sys::Sys_Printf(c"WARNING: %s has no files, ignored\n".as_ptr(), packfile);
                sys::Sys_FileClose(packhandle);
                return ptr::null_mut();
            }
            Ok(Ok(n)) => n,
        };

        if numpackfiles != pak::PAK0_COUNT {
            com_modified = true; // not the original file
        }

        let newfiles = sys::Mem_Alloc(numpackfiles as usize * core::mem::size_of::<PackFile>())
            .cast::<PackFile>();

        let mut dir_bytes = vec![0u8; header.dirlen as usize];
        sys::Sys_FileSeek(packhandle, header.dirofs as sys::qfileofs_t);
        sys::Sys_FileRead(packhandle, dir_bytes.as_mut_ptr().cast(), header.dirlen);

        // crc the directory to check for modifications
        if !matches!(
            pak::directory_crc(&dir_bytes),
            pak::PAK0_CRC_V106 | pak::PAK0_CRC_V101 | pak::PAK0_CRC_V100
        ) {
            com_modified = true;
        }

        // parse the directory
        for (i, entry) in pak::parse_entries(&dir_bytes, numpackfiles)
            .iter()
            .enumerate()
        {
            let dst = &mut *newfiles.add(i);
            for (d, &s) in dst.name.iter_mut().zip(entry.name.iter()) {
                *d = s as c_char;
            }
            dst.filepos = entry.filepos;
            dst.filelen = entry.filelen;
        }

        let pack = mem_alloc_zeroed::<Pack>();
        strlcpy_into(&mut (*pack).filename, cstr_bytes(packfile));
        (*pack).handle = packhandle;
        (*pack).numfiles = numpackfiles;
        (*pack).files = newfiles;
        pack
    }
}

// C: va()'s ring of 8 static buffers; COM_GetGameNames is the only fs-side
// user. VA_BUFFERLEN mirrors common.c (max(MAX_OSPATH, 1024)).
const VA_NUM_BUFFS: usize = 8;
const VA_BUFFERLEN: usize = if MAX_OSPATH >= 1024 { MAX_OSPATH } else { 1024 };
static mut GAMENAMES_VA: [[c_char; VA_BUFFERLEN]; VA_NUM_BUFFS] = [[0; VA_BUFFERLEN]; VA_NUM_BUFFS];
static mut GAMENAMES_VA_IDX: usize = 0;

/// C: `const char *COM_GetGameNames (qboolean full)`
///
/// # Safety
/// Main thread; the returned pointer is valid like a C va() buffer (a ring
/// of 8, recycled by later calls).
#[no_mangle]
pub unsafe extern "C" fn COM_GetGameNames(full: bool) -> *const c_char {
    // SAFETY: main-thread globals; the ring replicates va()'s lifetime.
    unsafe {
        if full {
            let names = sp::full_game_names(bytes_of(&*ptr::addr_of!(com_gamenames)));
            GAMENAMES_VA_IDX = (GAMENAMES_VA_IDX + 1) & (VA_NUM_BUFFS - 1);
            let buf = &mut (*ptr::addr_of_mut!(GAMENAMES_VA))[GAMENAMES_VA_IDX];
            strlcpy_into(buf, &names);
            return buf.as_ptr();
        }
        (*ptr::addr_of!(com_gamenames)).as_ptr()
    }
}

/// C: `qboolean COM_GameDirMatches (const char *tdirs)`
///
/// # Safety
/// `tdirs` valid C string; main thread.
#[no_mangle]
pub unsafe extern "C" fn COM_GameDirMatches(tdirs: *const c_char) -> bool {
    // SAFETY: forwarded contract.
    unsafe { sp::game_dir_matches(bytes_of(&*ptr::addr_of!(com_gamenames)), cstr_bytes(tdirs)) }
}

/// C: `qboolean COM_ModForbiddenChars (const char *p)`
///
/// # Safety
/// `p` valid C string.
#[no_mangle]
pub unsafe extern "C" fn COM_ModForbiddenChars(p: *const c_char) -> bool {
    // SAFETY: forwarded contract.
    sp::mod_forbidden_chars(unsafe { cstr_bytes(p) })
}

// The embedded vkquake.pak, extracted once (Mem-owned, lives for the process
// like the C function-static).
static mut VKQUAKE_PAK_EXTRACTED: *mut u8 = ptr::null_mut();

/// C: `static void COM_AddGameDirectoryRoot (const char *base, const char *dir,
///      unsigned int path_id, qboolean add_embedded)`
unsafe fn add_game_directory_root(base: &[u8], dir: &[u8], path_id: c_uint, add_embedded: bool) {
    // SAFETY: main-thread global/list mutation, engine seams as in C.
    unsafe {
        path_join_into(&mut *ptr::addr_of_mut!(com_gamedir), base, dir);

        // add the directory to the search path
        let search = mem_alloc_zeroed::<SearchPath>();
        (*search).path_id = path_id;
        strlcpy_into(
            &mut (*search).filename,
            bytes_of(&*ptr::addr_of!(com_gamedir)),
        );
        strlcpy_into(&mut (*search).dir, dir);
        (*search).next = com_searchpaths;
        com_searchpaths = search;

        // add any pak files in the format pak0.pak pak1.pak, ...
        let mut i = 0u32;
        loop {
            let mut pakfile = [0 as c_char; MAX_OSPATH];
            let name = format!("pak{}.pak", i);
            path_join_into(
                &mut pakfile,
                bytes_of(&*ptr::addr_of!(com_gamedir)),
                name.as_bytes(),
            );
            let mut packhandle: c_int = 0;
            if sys::Sys_FileOpenRead(pakfile.as_ptr(), &mut packhandle) == -1 {
                break;
            }
            let pak_ptr = load_pack_file(pakfile.as_ptr(), packhandle);
            if !pak_ptr.is_null() {
                let search = mem_alloc_zeroed::<SearchPath>();
                (*search).path_id = path_id;
                (*search).pack = pak_ptr;
                strlcpy_into(&mut (*search).dir, dir);
                (*search).next = com_searchpaths;
                com_searchpaths = search;
            }

            if i == 0 && path_id == 1 && add_embedded {
                let compressed = core::slice::from_raw_parts(
                    sys::vkquake_pak.as_ptr(),
                    sys::vkquake_pak_size as usize,
                );
                let extracted_size = sys::vkquake_pak_decompressed_size as usize;
                if VKQUAKE_PAK_EXTRACTED.is_null() {
                    match quake_fs::zipdir::inflate_embedded(compressed, extracted_size) {
                        Ok(data) => {
                            let buf = sys::Mem_Alloc(extracted_size).cast::<u8>();
                            ptr::copy_nonoverlapping(data.as_ptr(), buf, data.len());
                            VKQUAKE_PAK_EXTRACTED = buf;
                        }
                        Err(_) => {
                            sys::Sys_Error(c"Error extracting embedded pack".as_ptr());
                        }
                    }
                }
                let pak0_modified = com_modified;
                let mut packhandle: c_int = 0;
                sys::Sys_MemFileOpenRead(
                    VKQUAKE_PAK_EXTRACTED.cast(),
                    extracted_size as sys::qfilesize_t,
                    &mut packhandle,
                );
                let pak_ptr = load_pack_file(c"vkquake.pak".as_ptr(), packhandle);
                let search = mem_alloc_zeroed::<SearchPath>();
                (*search).path_id = path_id;
                (*search).pack = pak_ptr;
                strlcpy_into(&mut (*search).dir, dir);
                (*search).next = com_searchpaths;
                com_searchpaths = search;
                com_modified = pak0_modified;
            }

            if pak_ptr.is_null() {
                break;
            }
            i += 1;
        }
    }
}

/// C: `static void COM_AddGameDirectory (const char *dir)`
unsafe fn add_game_directory(dir: &[u8]) {
    // SAFETY: main-thread globals; seams as in C.
    unsafe {
        let mut names = bytes_of(&*ptr::addr_of!(com_gamenames)).to_vec();
        sp::append_game_name(&mut names, dir);
        strlcpy_into(&mut *ptr::addr_of_mut!(com_gamenames), &names);

        // quakespasm enables mission pack flags automatically
        if eq_ignore_case(dir, b"rogue") {
            rogue = true;
            standard_quake = false;
        }
        if eq_ignore_case(dir, b"hipnotic") || eq_ignore_case(dir, b"quoth") {
            hipnotic = true;
            standard_quake = false;
        }

        // assign a path_id to this game directory; all roots share it
        let path_id = sp::next_path_id(if com_searchpaths.is_null() {
            None
        } else {
            Some((*com_searchpaths).path_id)
        });

        // mount all roots in order; the userdir last as the write target
        let userdir = sys::COM_HostUserdir();
        let basedir_parm = sys::COM_HostBasedir();
        let have_userdir = userdir != basedir_parm;
        for i in 0..com_numbasedirs as usize {
            let root = bytes_of(&(*ptr::addr_of!(com_basedirs))[i]).to_vec();
            let is_main = eq_ignore_case(&root, bytes_of(&*ptr::addr_of!(com_basedir)));
            let is_user = have_userdir && eq_ignore_case(&root, cstr_bytes(userdir));

            let mut path = [0 as c_char; MAX_OSPATH];
            path_join_into(&mut path, &root, dir);
            if is_user {
                sys::Sys_mkdir(path.as_ptr());
            } else if !is_main && sys::Sys_FileType(path.as_ptr()) != FS_ENT_DIRECTORY {
                continue;
            }
            add_game_directory_root(&root, dir, path_id, is_main);
        }
    }
}

/// C: `void COM_ResetGameDirectories (const char *newdirs)`
///
/// # Safety
/// `newdirs` valid C string; main thread.
#[no_mangle]
pub unsafe extern "C" fn COM_ResetGameDirectories(newdirs: *const c_char) {
    // SAFETY: main-thread teardown of Rust-owned, Mem-allocated nodes.
    unsafe {
        let newdirs_owned = cstr_bytes(newdirs).to_vec();
        // kill the extra game if it is loaded
        while com_searchpaths != com_base_searchpaths {
            let s = com_searchpaths;
            let pack = (*s).pack;
            if !pack.is_null() {
                sys::Sys_FileClose((*pack).handle);
                sys::Mem_Free((*pack).files.cast());
                sys::Mem_Free(pack.cast());
            }
            com_searchpaths = (*s).next;
            sys::Mem_Free(s.cast());
        }
        hipnotic = false;
        rogue = false;
        standard_quake = true;
        // wipe the list of mod gamedirs
        com_gamenames[0] = 0;
        // reset this too
        let last_root =
            bytes_of(&(*ptr::addr_of!(com_basedirs))[(com_numbasedirs - 1) as usize]).to_vec();
        path_join_into(
            &mut *ptr::addr_of_mut!(com_gamedir),
            &last_root,
            GAMENAME.to_bytes(),
        );

        for dir in sp::parse_new_gamedirs(&newdirs_owned) {
            add_game_directory(dir);
        }
    }
}

/// C: `FILE *COM_FOpenPrefFile (const char *filename, const char *mode)`
///
/// # Safety
/// Both args valid C strings.
#[no_mangle]
pub unsafe extern "C" fn COM_FOpenPrefFile(
    filename: *const c_char,
    mode: *const c_char,
) -> *mut sys::FILE {
    // SAFETY: forwarded contract; seams as in C.
    unsafe {
        // harness runs must be hermetic: per-user state is redirected into
        // the disposable gamedir
        if sys::harness_active {
            let mut path = [0 as c_char; MAX_OSPATH];
            path_join_into(
                &mut path,
                bytes_of(&*ptr::addr_of!(com_gamedir)),
                cstr_bytes(filename),
            );
            return sys::Sys_fopen(path.as_ptr(), mode);
        }
        let pref_path = sys::Sys_GetPrefPath(c"".as_ptr(), c"vkQuake".as_ptr());
        let mut path = [0 as c_char; MAX_OSPATH];
        path_join_into(&mut path, cstr_bytes(pref_path), cstr_bytes(filename));
        let f = sys::Sys_fopen(path.as_ptr(), mode);
        sys::Mem_Free(pref_path.cast());
        f
    }
}

// C: static buffer whose address becomes host_parms->userdir
static mut USERPREFDIR: [c_char; MAX_OSPATH] = [0; MAX_OSPATH];

/// C: `static void COM_SetUserPrefDir (void)`
unsafe fn set_user_pref_dir() {
    // SAFETY: main-thread; the static buffer outlives the process like C's.
    unsafe {
        if sys::COM_HostUserdir() != sys::COM_HostBasedir() {
            return;
        }
        let pref_path = sys::Sys_GetPrefPath(c"".as_ptr(), c"vkQuake".as_ptr());
        if pref_path.is_null() {
            return;
        }
        strlcpy_into(&mut *ptr::addr_of_mut!(USERPREFDIR), cstr_bytes(pref_path));
        sys::Mem_Free(pref_path.cast());
        // strip trailing dir separators
        let mut len = bytes_of(&*ptr::addr_of!(USERPREFDIR)).len();
        while len > 0 {
            let c = USERPREFDIR[len - 1] as u8;
            if c == b'/' || c == b'\\' {
                len -= 1;
                USERPREFDIR[len] = 0;
            } else {
                break;
            }
        }
        sys::COM_SetHostUserdir((*ptr::addr_of!(USERPREFDIR)).as_ptr());
        sys::Sys_Printf(
            c"Writing user files to %s\n".as_ptr(),
            (*ptr::addr_of!(USERPREFDIR)).as_ptr(),
        );
    }
}

// ---------------------------------------------------------------------------
// basedirs.txt (folder picked in the SDL3 dialog)

#[cfg(feature = "sdl3")]
mod basedirs {
    use super::*;

    // indexed by quakeflavor_t
    pub static mut COM_STOREDBASEDIRS: [[c_char; MAX_OSPATH]; 2] = [[0; MAX_OSPATH]; 2];
    pub static mut COM_PENDINGBASEDIRWRITE: bool = false;

    /// C: `static void COM_LoadSelectedBaseDirs (void)`
    pub unsafe fn load_selected_base_dirs() {
        // SAFETY: main thread; stdio over the pref file exactly like the C.
        unsafe {
            let f = COM_FOpenPrefFile(c"basedirs.txt".as_ptr(), c"r".as_ptr());
            if f.is_null() {
                return;
            }
            let mut line = [0 as c_char; MAX_OSPATH + 16];
            while !sys::stdio::fgets(line.as_mut_ptr(), line.len() as c_int, f).is_null() {
                let bytes = bytes_of(&line).to_vec();
                let Some(space) = bytes.iter().position(|&b| b == b' ') else {
                    continue;
                };
                let (tag, rest) = bytes.split_at(space);
                let mut path = &rest[1..];
                if let Some(end) = path.iter().position(|&b| b == b'\r' || b == b'\n') {
                    path = &path[..end];
                }
                if tag == b"classic" {
                    strlcpy_into(&mut (*ptr::addr_of_mut!(COM_STOREDBASEDIRS))[0], path);
                } else if tag == b"remastered" {
                    strlcpy_into(&mut (*ptr::addr_of_mut!(COM_STOREDBASEDIRS))[1], path);
                }
            }
            sys::stdio::fclose(f);
        }
    }

    /// C: `static qboolean COM_SelectBaseDir (int flavor, char *dst, size_t dstsize)`
    pub unsafe fn select_base_dir(flavor: c_int, dst: &mut [c_char]) -> bool {
        // SAFETY: main thread; SDL dialog seams.
        unsafe {
            let (title, complaint): (&CStr, &CStr) = match flavor {
                0 => (
                    c"Select your classic Quake folder",
                    c"The selected folder does not contain id1/pak0.pak.",
                ),
                1 => (
                    c"Select your remastered Quake folder",
                    c"The selected folder does not contain QuakeEX.kpf.",
                ),
                _ => (
                    c"Select your Quake folder",
                    c"The selected folder does not contain Quake game data (id1/pak0.pak or QuakeEX.kpf).",
                ),
            };
            let default_location: *const c_char = match flavor {
                0 => COM_STOREDBASEDIRS[0].as_ptr(),
                1 => COM_STOREDBASEDIRS[1].as_ptr(),
                _ => {
                    if COM_STOREDBASEDIRS[1][0] != 0 {
                        COM_STOREDBASEDIRS[1].as_ptr()
                    } else {
                        COM_STOREDBASEDIRS[0].as_ptr()
                    }
                }
            };

            loop {
                let result = sys::Sys_SelectFolder(
                    title.as_ptr(),
                    default_location,
                    dst.as_mut_ptr(),
                    dst.len(),
                );
                if result <= 0 {
                    if result == 0 {
                        // cancelled
                        sys::Sys_QuitNoShutdown();
                    }
                    return false; // no dialog could be shown
                }
                if is_valid_flavor_dir(bytes_of(dst), flavor) {
                    return true;
                }
                sys::Sys_MessageBoxWarning(c"vkqr-engine".as_ptr(), complaint.as_ptr());
            }
        }
    }

    /// C: `static void COM_SetPendingBaseDir (int flavor, const char *dir)`
    pub unsafe fn set_pending_base_dir(flavor: c_int, dir: &[u8]) {
        // SAFETY: main-thread statics.
        unsafe {
            strlcpy_into(
                &mut (*ptr::addr_of_mut!(COM_STOREDBASEDIRS))[flavor as usize],
                dir,
            );
            COM_PENDINGBASEDIRWRITE = true;
        }
    }
}

/// C: `void COM_WriteSelectedBaseDir (void)` (body is #ifdef USE_SDL3)
///
/// # Safety
/// Main thread.
#[no_mangle]
pub unsafe extern "C" fn COM_WriteSelectedBaseDir() {
    #[cfg(feature = "sdl3")]
    // SAFETY: main-thread statics + stdio, like the C.
    unsafe {
        use basedirs::*;
        if !COM_PENDINGBASEDIRWRITE {
            return;
        }
        let f = COM_FOpenPrefFile(c"basedirs.txt".as_ptr(), c"w".as_ptr());
        if f.is_null() {
            return;
        }
        for (tag, dir) in [
            (&b"classic "[..], &COM_STOREDBASEDIRS[0]),
            (&b"remastered "[..], &COM_STOREDBASEDIRS[1]),
        ] {
            if dir[0] != 0 {
                let mut lineb = tag.to_vec();
                lineb.extend_from_slice(bytes_of(dir));
                lineb.push(b'\n');
                sys::stdio::fwrite(lineb.as_ptr().cast(), 1, lineb.len(), f);
            }
        }
        sys::stdio::fclose(f);
        COM_PENDINGBASEDIRWRITE = false;
    }
}

// C: static char com_nightdivedir[MAX_OSPATH];
static mut COM_NIGHTDIVEDIR: [c_char; MAX_OSPATH] = [0; MAX_OSPATH];

/// C: `static void COM_MountNightdiveUserDir (void)`
unsafe fn mount_nightdive_user_dir() {
    // SAFETY: main thread; seams as in C.
    unsafe {
        if COM_NIGHTDIVEDIR[0] == 0 || sys::COM_CheckParm(c"-nonightdive".as_ptr()) != 0 {
            return;
        }
        if sys::Sys_FileType((*ptr::addr_of!(COM_NIGHTDIVEDIR)).as_ptr()) != FS_ENT_DIRECTORY {
            return;
        }
        COM_AddBaseDir((*ptr::addr_of!(COM_NIGHTDIVEDIR)).as_ptr());
        sys::Sys_Printf(
            c"Mounted Nightdive add-on dir %s\n".as_ptr(),
            (*ptr::addr_of!(COM_NIGHTDIVEDIR)).as_ptr(),
        );
    }
}

fn flavor_of(i: c_int) -> flavor::FlavorRequest {
    match i {
        0 => flavor::FlavorRequest::Original,
        1 => flavor::FlavorRequest::Remastered,
        _ => flavor::FlavorRequest::NoPreference,
    }
}

/// C: `static qboolean COM_IsValidFlavorDir (const char *dir, int flavor)`
unsafe fn is_valid_flavor_dir(dir: &[u8], flavor_i: c_int) -> bool {
    // SAFETY: file probes through the Sys seam, like the C.
    unsafe {
        let mut path = [0 as c_char; MAX_OSPATH];
        let mut sub = GAMENAME.to_bytes().to_vec();
        sub.extend_from_slice(b"/pak0.pak");
        let classic_exists = path_join_into(&mut path, dir, &sub) < MAX_OSPATH
            && sys::Sys_FileType(path.as_ptr()) == FS_ENT_FILE;
        let kpf_exists = path_join_into(&mut path, dir, b"QuakeEX.kpf") < MAX_OSPATH
            && sys::Sys_FileType(path.as_ptr()) == FS_ENT_FILE;
        flavor::is_valid_flavor_dir(flavor_of(flavor_i), classic_exists, kpf_exists)
    }
}

/// C: `static int COM_RequestedQuakeFlavor (void)`
unsafe fn requested_quake_flavor() -> c_int {
    // SAFETY: parm lookups through the seam.
    unsafe {
        let has_remaster = sys::COM_CheckParm(c"-prefremaster".as_ptr()) != 0
            || sys::COM_CheckParm(c"-remaster".as_ptr()) != 0
            || sys::COM_CheckParm(c"-remastered".as_ptr()) != 0;
        let has_original = sys::COM_CheckParm(c"-preforiginal".as_ptr()) != 0
            || sys::COM_CheckParm(c"-original".as_ptr()) != 0;
        match flavor::requested_flavor(has_remaster, has_original) {
            flavor::FlavorRequest::Remastered => 1,
            flavor::FlavorRequest::Original => 0,
            flavor::FlavorRequest::NoPreference => -1,
        }
    }
}

/// C: `static qboolean COM_FindStoreBaseDir (void)`
unsafe fn find_store_base_dir() -> bool {
    // SAFETY: main thread; store seams + Rust steam shims, matching the C
    // control flow statement for statement.
    unsafe {
        let mut steamquake = sys::steamgame_t {
            appid: 0,
            subdir: ptr::null_mut(),
            library: [0; sys::MAX_OSPATH],
        };
        let mut original = [0 as c_char; MAX_OSPATH];
        let mut remastered = [0 as c_char; MAX_OSPATH];
        let force_steam = sys::COM_CheckParm(c"-steam".as_ptr()) != 0;
        let force_gog = sys::COM_CheckParm(c"-gog".as_ptr()) != 0;
        let force_egs =
            sys::COM_CheckParm(c"-egs".as_ptr()) != 0 || sys::COM_CheckParm(c"-epic".as_ptr()) != 0;
        let forced = force_steam || force_gog || force_egs;

        if (!forced || force_steam)
            && sys::COM_CheckParm(c"-nosteam".as_ptr()) == 0
            && crate::steam::Steam_FindGame(&mut steamquake, 2310)
            && crate::steam::Steam_ResolvePath(original.as_mut_ptr(), original.len(), &steamquake)
        {
            let mut rem = bytes_of(&original).to_vec();
            rem.extend_from_slice(b"/rerelease");
            if rem.len() >= MAX_OSPATH {
                remastered[0] = 0;
            } else {
                strlcpy_into(&mut remastered, &rem);
                if !sys::Sys_GetNightdiveUserDir(
                    (*ptr::addr_of_mut!(COM_NIGHTDIVEDIR)).as_mut_ptr(),
                    MAX_OSPATH,
                    steamquake.library.as_ptr(),
                ) {
                    COM_NIGHTDIVEDIR[0] = 0;
                }
            }
        }

        if (!forced || force_gog) && sys::COM_CheckParm(c"-nogog".as_ptr()) == 0 {
            if original[0] == 0 && !sys::Sys_GetGOGQuakeDir(original.as_mut_ptr(), MAX_OSPATH) {
                original[0] = 0;
            }
            if remastered[0] == 0 {
                if sys::Sys_GetGOGQuakeEnhancedDir(remastered.as_mut_ptr(), MAX_OSPATH) {
                    if COM_NIGHTDIVEDIR[0] == 0
                        && !sys::Sys_GetNightdiveUserDir(
                            (*ptr::addr_of_mut!(COM_NIGHTDIVEDIR)).as_mut_ptr(),
                            MAX_OSPATH,
                            ptr::null(),
                        )
                    {
                        COM_NIGHTDIVEDIR[0] = 0;
                    }
                } else {
                    remastered[0] = 0;
                }
            }
        }

        if (!forced || force_egs)
            && sys::COM_CheckParm(c"-noegs".as_ptr()) == 0
            && sys::COM_CheckParm(c"-noepic".as_ptr()) == 0
            && remastered[0] == 0
        {
            if crate::steam::EGS_FindGame(
                remastered.as_mut_ptr(),
                MAX_OSPATH,
                c"f57987ad149c43b3a7a66a7f10828f92".as_ptr(),
                c"19e3c0be6d6c4d4b84b1bc2248f94b43".as_ptr(),
                c"18161d3ef68e4166968036626d173f25".as_ptr(),
            ) {
                if COM_NIGHTDIVEDIR[0] == 0
                    && !sys::Sys_GetNightdiveUserDir(
                        (*ptr::addr_of_mut!(COM_NIGHTDIVEDIR)).as_mut_ptr(),
                        MAX_OSPATH,
                        ptr::null(),
                    )
                {
                    COM_NIGHTDIVEDIR[0] = 0;
                }
            } else {
                remastered[0] = 0;
            }
        }

        if original[0] != 0 && !is_valid_flavor_dir(bytes_of(&original), 0) {
            original[0] = 0;
        }
        if remastered[0] != 0 && !is_valid_flavor_dir(bytes_of(&remastered), 1) {
            remastered[0] = 0;
            COM_NIGHTDIVEDIR[0] = 0;
        }

        let requested = requested_quake_flavor();

        if !forced && !sys::isDedicated {
            #[cfg(feature = "sdl3")]
            {
                use basedirs::*;
                load_selected_base_dirs();

                // use the folder picked in a previous run unless the user
                // wants a new one
                if sys::COM_CheckParm(c"-select-basedir".as_ptr()) == 0 {
                    if original[0] == 0
                        && COM_STOREDBASEDIRS[0][0] != 0
                        && is_valid_flavor_dir(
                            bytes_of(&(*ptr::addr_of!(COM_STOREDBASEDIRS))[0]),
                            0,
                        )
                    {
                        let stored = bytes_of(&(*ptr::addr_of!(COM_STOREDBASEDIRS))[0]).to_vec();
                        strlcpy_into(&mut original, &stored);
                    }
                    if remastered[0] == 0
                        && COM_STOREDBASEDIRS[1][0] != 0
                        && is_valid_flavor_dir(
                            bytes_of(&(*ptr::addr_of!(COM_STOREDBASEDIRS))[1]),
                            1,
                        )
                    {
                        let stored = bytes_of(&(*ptr::addr_of!(COM_STOREDBASEDIRS))[1]).to_vec();
                        strlcpy_into(&mut remastered, &stored);
                    }
                }

                // still missing: ask for the folder, remember once usable
                if requested == 0 && original[0] == 0 {
                    if select_base_dir(0, &mut original) {
                        let sel = bytes_of(&original).to_vec();
                        set_pending_base_dir(0, &sel);
                    }
                } else if requested == 1 && remastered[0] == 0 {
                    if select_base_dir(1, &mut remastered) {
                        let sel = bytes_of(&remastered).to_vec();
                        set_pending_base_dir(1, &sel);
                    }
                } else if requested < 0 && original[0] == 0 && remastered[0] == 0 {
                    let mut selected = [0 as c_char; MAX_OSPATH];
                    if select_base_dir(-1, &mut selected) {
                        let sel = bytes_of(&selected).to_vec();
                        if is_valid_flavor_dir(&sel, 1) {
                            strlcpy_into(&mut remastered, &sel);
                            set_pending_base_dir(1, &sel);
                        } else {
                            strlcpy_into(&mut original, &sel);
                            set_pending_base_dir(0, &sel);
                        }
                    }
                }
            }
            #[cfg(not(feature = "sdl3"))]
            {
                // no folder picker without the SDL3 dialog API
                if requested == 0 && original[0] == 0 {
                    sys::Sys_Error(
                        c"Couldn't find the classic Quake folder. Use -basedir to specify it."
                            .as_ptr(),
                    );
                } else if requested == 1 && remastered[0] == 0 {
                    sys::Sys_Error(
                        c"Couldn't find the remastered Quake folder. Use -basedir to specify it."
                            .as_ptr(),
                    );
                }
            }
        }

        if original[0] == 0 && remastered[0] == 0 {
            if force_steam {
                sys::Sys_Error(c"Couldn't find Steam Quake".as_ptr());
            }
            if force_gog {
                sys::Sys_Error(c"Couldn't find GOG Quake".as_ptr());
            }
            if force_egs {
                sys::Sys_Error(c"Couldn't find Epic Games Store Quake".as_ptr());
            }
            return false; // fall through to the regular missing-data error
        }

        let flavor_choice: c_int = if requested == 1 && remastered[0] != 0 {
            1
        } else if requested == 0 && original[0] != 0 {
            0
        } else if original[0] != 0 && remastered[0] != 0 {
            sys::ChooseQuakeFlavor() as c_int
        } else if remastered[0] != 0 {
            1
        } else {
            0
        };

        let chosen = if flavor_choice == 1 {
            bytes_of(&remastered).to_vec()
        } else {
            bytes_of(&original).to_vec()
        };
        strlcpy_into(&mut *ptr::addr_of_mut!(com_basedir), &chosen);
        sys::Sys_Printf(
            c"Using Quake data from %s\n".as_ptr(),
            (*ptr::addr_of!(com_basedir)).as_ptr(),
        );

        if flavor_choice == 1 {
            mount_nightdive_user_dir();
        } else {
            COM_NIGHTDIVEDIR[0] = 0;
        }

        true
    }
}

/// C: `static void COM_InitSteamAPI (void)`
unsafe fn init_steam_api() {
    // SAFETY: main thread; Rust steam shims + C steam_api.c runtime.
    unsafe {
        if sys::COM_CheckParm(c"-nosteam".as_ptr()) != 0 {
            return;
        }
        let mut steamquake = sys::steamgame_t {
            appid: 0,
            subdir: ptr::null_mut(),
            library: [0; sys::MAX_OSPATH],
        };
        let mut steampath = [0 as c_char; MAX_OSPATH];
        if !crate::steam::Steam_FindGame(&mut steamquake, 2310)
            || !crate::steam::Steam_ResolvePath(steampath.as_mut_ptr(), MAX_OSPATH, &steamquake)
        {
            return;
        }
        if !sp::is_path_prefix(bytes_of(&steampath), bytes_of(&*ptr::addr_of!(com_basedir))) {
            return;
        }
        sys::Steam_Init(&steamquake);
    }
}

/// C: `void COM_InitFilesystem (void)`
///
/// # Safety
/// Called once at startup on the main thread, after COM_InitArgv.
#[no_mangle]
pub unsafe extern "C" fn COM_InitFilesystem() {
    // SAFETY: startup sequence identical to the C, using the same seams.
    unsafe {
        sys::Cvar_RegisterVariable(&raw mut sys::registered);
        sys::Cvar_RegisterVariable(&raw mut sys::cmdline);
        // C: Cmd_AddCommand is a cmd.h macro over Cmd_AddCommand2
        sys::Cmd_AddCommand2(
            c"path".as_ptr(),
            Some(com_path_f),
            sys::cmd_source_t_src_command,
            false,
        );
        sys::Cmd_AddCommand2(
            c"game".as_ptr(),
            Some(sys::COM_Game_f),
            sys::cmd_source_t_src_command,
            false,
        );

        let i = sys::COM_CheckParm(c"-basedir".as_ptr());
        if i != 0 && i < sys::com_argc - 1 {
            let arg = *sys::com_argv.add((i + 1) as usize);
            let argb = cstr_bytes(arg).to_vec();
            strlcpy_into(&mut *ptr::addr_of_mut!(com_basedir), &argb);
        } else {
            let basedir = cstr_bytes(sys::COM_HostBasedir()).to_vec();
            strlcpy_into(&mut *ptr::addr_of_mut!(com_basedir), &basedir);
        }

        let j = bytes_of(&*ptr::addr_of!(com_basedir)).len();
        if j < 1 {
            sys::Sys_Error(c"Bad argument to -basedir".as_ptr());
        }
        let last = com_basedir[j - 1] as u8;
        if last == b'\\' || last == b'/' {
            com_basedir[j - 1] = 0;
        }

        // no explicit -basedir: run store detection if the working directory
        // has no game data for the requested version, or a store was named
        let mut store_install = false;
        if i == 0
            && (!is_valid_flavor_dir(
                bytes_of(&*ptr::addr_of!(com_basedir)),
                requested_quake_flavor(),
            ) || sys::COM_CheckParm(c"-steam".as_ptr()) != 0
                || sys::COM_CheckParm(c"-gog".as_ptr()) != 0
                || sys::COM_CheckParm(c"-egs".as_ptr()) != 0
                || sys::COM_CheckParm(c"-epic".as_ptr()) != 0)
        {
            store_install = find_store_base_dir();
        }

        // keep all writes out of the game dirs for store installs / -multiuser
        if store_install || sys::multiuser {
            set_user_pref_dir();
        }

        // achievements/rich presence if the data comes from the Steam install
        init_steam_api();

        // register the remaining content roots
        COM_AddBaseDir((*ptr::addr_of!(com_basedir)).as_ptr());
        if sys::COM_HostUserdir() != sys::COM_HostBasedir() {
            COM_AddBaseDir(sys::COM_HostUserdir());
        }

        let mut i = sys::COM_CheckParmNext(i, c"-basegame".as_ptr());
        if i != 0 {
            // -basegame replaces all hardcoded dirs (alternative to id1)
            com_modified = true;
            loop {
                if i == 0 || i >= sys::com_argc - 1 {
                    break;
                }
                let p = *sys::com_argv.add((i + 1) as usize);
                if COM_ModForbiddenChars(p) {
                    sys::Sys_Error(
                        c"gamedir should be a single directory name, not a path\n".as_ptr(),
                    );
                }
                if !p.is_null() {
                    let dir = cstr_bytes(p).to_vec();
                    add_game_directory(&dir);
                }
                i = sys::COM_CheckParmNext(i, c"-basegame".as_ptr());
            }
        } else {
            // start up with GAMENAME by default (id1)
            add_game_directory(GAMENAME.to_bytes());
        }

        // end of the base searchpath: gamedirs from -game or the "game"
        // command are freed up to here on a new game command
        com_base_searchpaths = com_searchpaths;
        COM_ResetGameDirectories(c"".as_ptr());

        // add mission pack requests (only one should be specified)
        if sys::COM_CheckParm(c"-rogue".as_ptr()) != 0 {
            add_game_directory(b"rogue");
        }
        if sys::COM_CheckParm(c"-hipnotic".as_ptr()) != 0 {
            add_game_directory(b"hipnotic");
        }
        if sys::COM_CheckParm(c"-quoth".as_ptr()) != 0 {
            add_game_directory(b"quoth");
        }

        let mut i = 0;
        loop {
            i = sys::COM_CheckParmNext(i, c"-game".as_ptr());
            if i == 0 || i >= sys::com_argc - 1 {
                break;
            }
            let p = *sys::com_argv.add((i + 1) as usize);
            if COM_ModForbiddenChars(p) {
                sys::Sys_Error(c"gamedir should be a single directory name, not a path\n".as_ptr());
            }
            com_modified = true;
            if !p.is_null() {
                let dir = cstr_bytes(p).to_vec();
                add_game_directory(&dir);
            }
        }

        sys::COM_CheckRegistered();
    }
}

/// C: `void COM_Effectinfo_Enumerate (int (*cb) (const char *pname))`
/// (PSET_SCRIPT: dp effect names for compat with dpp7 protocols)
///
/// # Safety
/// `cb` must be a valid callback; main thread (uses the COM_Parse token).
#[no_mangle]
pub unsafe extern "C" fn COM_Effectinfo_Enumerate(
    cb: Option<unsafe extern "C" fn(*const c_char) -> c_int>,
) {
    const DPNAMES: [&CStr; 35] = [
        c"TE_GUNSHOT",
        c"TE_GUNSHOTQUAD",
        c"TE_SPIKE",
        c"TE_SPIKEQUAD",
        c"TE_SUPERSPIKE",
        c"TE_SUPERSPIKEQUAD",
        c"TE_WIZSPIKE",
        c"TE_KNIGHTSPIKE",
        c"TE_EXPLOSION",
        c"TE_EXPLOSIONQUAD",
        c"TE_TAREXPLOSION",
        c"TE_TELEPORT",
        c"TE_LAVASPLASH",
        c"TE_SMALLFLASH",
        c"TE_FLAMEJET",
        c"EF_FLAME",
        c"TE_BLOOD",
        c"TE_SPARK",
        c"TE_PLASMABURN",
        c"TE_TEI_G3",
        c"TE_TEI_SMOKE",
        c"TE_TEI_BIGEXPLOSION",
        c"TE_TEI_PLASMAHIT",
        c"EF_STARDUST",
        c"TR_ROCKET",
        c"TR_GRENADE",
        c"TR_BLOOD",
        c"TR_WIZSPIKE",
        c"TR_SLIGHTBLOOD",
        c"TR_KNIGHTSPIKE",
        c"TR_VORESPIKE",
        c"TR_NEHAHRASMOKE",
        c"TR_NEXUIZPLASMA",
        c"TR_GLOWTRAIL",
        c"SVC_PARTICLE",
    ];
    // SAFETY: same call pattern as the C; buf is a Mem-owned NUL-terminated
    // image from COM_LoadFile, walked via the C COM_Parse/com_token seam.
    unsafe {
        let Some(cb) = cb else { return };
        let buf = COM_LoadFile(c"effectinfo.txt".as_ptr(), ptr::null_mut());
        if buf.is_null() {
            return;
        }
        for name in DPNAMES {
            cb(name.as_ptr());
        }
        let mut f = buf.cast::<c_char>().cast_const();
        while !f.is_null() {
            let mut e = sys::COM_Parse(f);
            if cstr_bytes(sys::COM_ThreadToken()) == b"effect" {
                e = sys::COM_Parse(e);
                cb(sys::COM_ThreadToken());
            }
            while !e.is_null() && *e != 0 && *e != b'\n' as c_char {
                e = e.add(1);
            }
            f = e;
        }
        sys::Mem_Free(buf.cast());
    }
}
