//! `gl_draw.c` -- 2D drawing (Rust migration Phase 8 M7).
//!
//! Owns the pic cache (`Draw_PicFromWad2`/`Draw_TryCachePic`/`Draw_CachePic`),
//! the two 256x256 scrap atlases, the console font texture, the built-in
//! cursor/missing pics, the 2D quad emitters and the canvas/viewport setup.
//! Everything is exported under the C names so the remaining C
//! (`pr_ext.c`, `gl_screen.c`, the console/menu/sbar C oracles) and the
//! Phase 7 Rust modules (`console.rs`/`menu.rs`/`sbar.rs`, which reach the
//! draw layer through the `quake-c-sys` extern declarations) link against
//! this file when `-Duse_rust_render` is on.
//!
//! The only C left with the file is the `scr_conalpha` cvar definition in
//! `Quake/gl_draw_glue.c` (registered from `Draw_Init` here).
//!
//! Observable-vs-internal: the C name → `cachepic_t *` hash map
//! (`hash_map_t` with `HashStr`/`HashStrCmp`, both `static inline` in
//! `hash_map.h`) is replaced by a `std::collections::HashMap` keyed on the
//! `q_strlcpy`-truncated name bytes; lookups by full path behave exactly as
//! the C `strcmp` against the truncated `pic->name`. The chained
//! `q_cachepics` list, the `Mem_Alloc` allocation of every `cachepic_t` and
//! the `glpic_t` payload written over `qpic_t::data` are kept byte for byte
//! because `pr_ext.c` and the C oracles read those records.

use core::ffi::{c_char, c_float, c_int, c_uint, c_void};
use core::ptr;
use std::collections::HashMap;
use std::sync::Mutex;

use ash::vk;
use quake_c_sys as c;
use quake_math::mathlib::{vector_ma, Vec3};
use quake_render::cb::{self, CmdProcs};
use quake_types::host::ClientState;
use quake_types::render::{BasicVertex, CbContext, GlTexture, VRect, VulkanPipeline};
use quake_types::wad::{LumpInfo, QPic};

use crate::gl_rmisc::{device, vg, with_ctx, DYN};
use crate::gl_texmgr::{TexMgr_LoadImage, TexMgr_ReloadImage};
use crate::wad::{wad_base, W_GetLumpName, W_LoadWadFile};

/// `q_types.h:240`.
const MAX_QPATH: usize = 64;
/// `draw.h:26`.
const CHARACTER_SIZE: f32 = 8.0;
/// `draw.h:36` -- `PICFLAG_AUTO`.
const PICFLAG_AUTO: c_int = 0;
/// `wad.h:42`.
const TYP_QPIC: c_char = 66;
/// `wad.h:50`.
const WADFILENAME: &core::ffi::CStr = c"gfx.wad";
/// `quakedef.h:219`.
const WARPIMAGESIZE: f32 = 512.0;
/// `protocol.h:240`.
const GAME_DEATHMATCH: c_int = 1;

/// `gl_texmgr.h:54` -- `enum srcformat`.
const SRC_INDEXED: c_int = 0;
/// `gl_texmgr.h:35-47` -- `TEXPREF_*`.
const TEXPREF_NEAREST: c_uint = 0x0004;
const TEXPREF_ALPHA: c_uint = 0x0008;
const TEXPREF_PAD: c_uint = 0x0010;
const TEXPREF_PERSIST: c_uint = 0x0020;
const TEXPREF_OVERWRITE: c_uint = 0x0040;
const TEXPREF_NOPICMIP: c_uint = 0x0080;
const TEXPREF_CONCHARS: c_uint = 0x0400;

/// `quakedef.h:237-252` -- `canvastype`.
pub const CANVAS_NONE: c_int = 0;
pub const CANVAS_DEFAULT: c_int = 1;
pub const CANVAS_CONSOLE: c_int = 2;
pub const CANVAS_MENU: c_int = 3;
pub const CANVAS_SBAR: c_int = 4;
pub const CANVAS_WARPIMAGE: c_int = 5;
pub const CANVAS_CROSSHAIR: c_int = 6;
pub const CANVAS_BOTTOMLEFT: c_int = 7;
pub const CANVAS_TOPLEFT: c_int = 8;
pub const CANVAS_BOTTOMRIGHT: c_int = 9;
pub const CANVAS_TOPRIGHT: c_int = 10;
pub const CANVAS_CSQC: c_int = 11;

extern "C" {
    /// `client.h` -- `client_state_t cl` (Rust-owned under `host`).
    static mut cl: ClientState;
    /// `Quake/gl_draw_glue.c` -- `cvar_t scr_conalpha`.
    static mut scr_conalpha: c::cvar_t;
    /// `gl_screen.c:139` -- `qmutex_t *draw_qcvm_mutex`.
    static mut draw_qcvm_mutex: *mut c::qmutex_t;
    /// `gl_screen.c:131` -- `vrect_t scr_vrect`.
    static mut scr_vrect: VRect;
    /// `screen.h` -- `cvar_t scr_sbarscale`, `scr_crosshairscale`, `scr_style`.
    static mut scr_sbarscale: c::cvar_t;
    static mut scr_crosshairscale: c::cvar_t;
    static mut scr_style: c::cvar_t;
    /// `menu.h:70` -- `float M_GetScale ()` (`menu.rs` under `host`, else `menu.c`).
    fn M_GetScale() -> c_float;
    /// `screen.h:29` -- `void SCR_LoadPics (void)` (`gl_screen.c`).
    fn SCR_LoadPics();
    /// `sbar.h:32` -- `void Sbar_LoadPics (void)` (`sbar.rs` under `host`, else `sbar.c`).
    fn Sbar_LoadPics();
}

/// `gl_draw.c:33-38` -- the C-visible pics and the console font.
#[no_mangle]
pub static mut draw_disc: *mut QPic = ptr::null_mut();
#[no_mangle]
pub static mut draw_backtile: *mut QPic = ptr::null_mut();
#[no_mangle]
pub static mut char_texture: *mut GlTexture = ptr::null_mut();
#[no_mangle]
pub static mut pic_ovr: *mut QPic = ptr::null_mut();
#[no_mangle]
pub static mut pic_ins: *mut QPic = ptr::null_mut();
#[no_mangle]
pub static mut pic_nul: *mut QPic = ptr::null_mut();

// johnfitz -- new pics
static PIC_OVR_DATA: [[u8; 8]; 8] = [
    [255, 255, 255, 255, 255, 255, 255, 255],
    [255, 15, 15, 15, 15, 15, 15, 255],
    [255, 15, 15, 15, 15, 15, 15, 2],
    [255, 15, 15, 15, 15, 15, 15, 2],
    [255, 15, 15, 15, 15, 15, 15, 2],
    [255, 15, 15, 15, 15, 15, 15, 2],
    [255, 15, 15, 15, 15, 15, 15, 2],
    [255, 255, 2, 2, 2, 2, 2, 2],
];

static PIC_INS_DATA: [[u8; 8]; 9] = [
    [15, 15, 255, 255, 255, 255, 255, 255],
    [15, 15, 2, 255, 255, 255, 255, 255],
    [15, 15, 2, 255, 255, 255, 255, 255],
    [15, 15, 2, 255, 255, 255, 255, 255],
    [15, 15, 2, 255, 255, 255, 255, 255],
    [15, 15, 2, 255, 255, 255, 255, 255],
    [15, 15, 2, 255, 255, 255, 255, 255],
    [15, 15, 2, 255, 255, 255, 255, 255],
    [255, 2, 2, 255, 255, 255, 255, 255],
];

static PIC_NUL_DATA: [[u8; 8]; 8] = [
    [252, 252, 252, 252, 0, 0, 0, 0],
    [252, 252, 252, 252, 0, 0, 0, 0],
    [252, 252, 252, 252, 0, 0, 0, 0],
    [252, 252, 252, 252, 0, 0, 0, 0],
    [0, 0, 0, 0, 252, 252, 252, 252],
    [0, 0, 0, 0, 252, 252, 252, 252],
    [0, 0, 0, 0, 252, 252, 252, 252],
    [0, 0, 0, 0, 252, 252, 252, 252],
];

/// `gl_draw.c:76-80` -- `glpic_t`, the payload written over `qpic_t::data`
/// (unaligned: `qpic_t::data` sits at offset 8 of a 4-aligned struct).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GlPic {
    pub gltexture: *mut GlTexture,
    pub sl: f32,
    pub tl: f32,
    pub sh: f32,
    pub th: f32,
}

//==============================================================================
//
//  PIC CACHING
//
//==============================================================================

/// `gl_draw.c:87-95` -- `cachepic_t`, one `Mem_Alloc` record per cached pic.
#[repr(C)]
pub struct CachePic {
    pub next: *mut CachePic,
    pub name: [c_char; MAX_QPATH],
    pub picflags: c_int,
    pub pic: QPic,
    pub padding: [u8; 32],
}

// The `glpic_t` payload starts at `pic.data` and runs into `padding`
// (`gl_draw.c:99-100`).
const _: () = assert!(
    core::mem::offset_of!(CachePic, padding)
        == core::mem::offset_of!(CachePic, pic) + core::mem::size_of::<QPic>()
);
const _: () = assert!(32 >= core::mem::size_of::<GlPic>());
const _: () = assert!(core::mem::size_of::<CachePic>() == 8 + MAX_QPATH + 4 + 12 + 32);

/// `gl_draw.c:102` -- the chained cache, head and tail.
static mut Q_CACHEPICS: *mut CachePic = ptr::null_mut();
static mut Q_CACHEPICS_LAST_ENTRY: *mut CachePic = ptr::null_mut();

/// `gl_draw.c:108` -- name → record lookup (see the module note); the value
/// is the `CachePic` pointer as an address.
static Q_CACHEPICS_MAP: Mutex<Option<HashMap<Box<[u8]>, usize>>> = Mutex::new(None);

//  scrap allocation
//  Allocate all the little status bar obejcts into a single texture
//  to crutch up stupid hardware / drivers

const MAX_SCRAPS: usize = 2;
const BLOCK_WIDTH: usize = 256;
const BLOCK_HEIGHT: usize = 256;

static mut SCRAP_ALLOCATED: [[c_int; BLOCK_WIDTH]; MAX_SCRAPS] = [[0; BLOCK_WIDTH]; MAX_SCRAPS];
static mut SCRAP_TEXELS: [[u8; BLOCK_WIDTH * BLOCK_HEIGHT]; MAX_SCRAPS] =
    [[0; BLOCK_WIDTH * BLOCK_HEIGHT]; MAX_SCRAPS];
static mut SCRAP_DIRTY: bool = false;
static mut SCRAP_TEXTURES: [*mut GlTexture; MAX_SCRAPS] = [ptr::null_mut(); MAX_SCRAPS];

/// `Scrap_AllocBlock` -- returns an index into scrap_texnums[] and the
/// position inside it.
///
/// # Safety
/// Main thread, under the draw lock like every cache mutation.
unsafe fn scrap_alloc_block(w: c_int, h: c_int, x: &mut c_int, y: &mut c_int) -> usize {
    // SAFETY: per the contract.
    let allocated = unsafe { &mut *ptr::addr_of_mut!(SCRAP_ALLOCATED) };
    for (texnum, row) in allocated.iter_mut().enumerate() {
        let mut best = BLOCK_HEIGHT as c_int;

        let mut i = 0;
        while i < BLOCK_WIDTH as c_int - w {
            let mut best2 = 0;

            let mut j = 0;
            while j < w {
                let v = row[(i + j) as usize];
                if v >= best {
                    break;
                }
                if v > best2 {
                    best2 = v;
                }
                j += 1;
            }
            if j == w {
                // this is a valid spot
                *x = i;
                best = best2;
                *y = best;
            }
            i += 1;
        }

        if best + h > BLOCK_HEIGHT as c_int {
            continue;
        }

        for i in 0..w {
            row[(*x + i) as usize] = best + h;
        }

        return texnum;
    }

    // SAFETY: `Sys_Error` exits (ADR-009 allows it from Rust).
    unsafe { c::Sys_Error(c"Scrap_AllocBlock: full".as_ptr()) }
}

/// `Scrap_Upload` -- johnfitz -- now uses TexMgr.
///
/// # Safety
/// Main thread; the texture manager is initialized.
unsafe fn scrap_upload() {
    let mut name = [0 as c_char; 8];
    for i in 0..MAX_SCRAPS {
        // SAFETY: per the contract; `name` is an 8-byte buffer like C's.
        unsafe {
            c::cl_main::q_snprintf(
                name.as_mut_ptr(),
                name.len(),
                c"scrap%i".as_ptr(),
                i as c_int,
            );
            let texels = ptr::addr_of_mut!(SCRAP_TEXELS[i]).cast::<u8>();
            SCRAP_TEXTURES[i] = TexMgr_LoadImage(
                ptr::null_mut(),
                name.as_ptr(),
                BLOCK_WIDTH as c_int,
                BLOCK_HEIGHT as c_int,
                SRC_INDEXED,
                texels,
                c"".as_ptr(),
                texels as usize,
                TEXPREF_ALPHA | TEXPREF_OVERWRITE | TEXPREF_NOPICMIP,
            );
        }
    }

    // SAFETY: main thread.
    unsafe { SCRAP_DIRTY = false };
}

/// The bytes of `name` after `q_strlcpy (pic->name, name, MAX_QPATH)`: the
/// hash key C stores is a pointer to that truncated copy.
///
/// # Safety
/// `name` is NUL-terminated.
unsafe fn truncated_name(name: *const c_char) -> Box<[u8]> {
    // SAFETY: per the contract.
    let bytes = unsafe { core::ffi::CStr::from_ptr(name) }.to_bytes();
    let n = bytes.len().min(MAX_QPATH - 1);
    Box::from(&bytes[..n])
}

/// The tail of `Draw_PicFromWad2`/`Draw_TryCachePic`: allocate the record,
/// chain it and index it.
///
/// # Safety
/// `name` NUL-terminated; `pic` a valid `qpic_t` header (width/height).
unsafe fn cache_new_pic(
    name: *const c_char,
    picflags: c_int,
    header: QPic,
    gl: GlPic,
) -> *mut QPic {
    // SAFETY: `Mem_Alloc` zero-fills (mimalloc calloc path), like C relies on
    // for `pic->next == NULL`.
    let pic = unsafe { c::Mem_Alloc(core::mem::size_of::<CachePic>()) }.cast::<CachePic>();
    // SAFETY: fresh record; `name` per the contract.
    unsafe {
        (*pic).picflags = picflags;
        c::cl_main::q_strlcpy((*pic).name.as_mut_ptr(), name, MAX_QPATH);
        (*pic).pic = header;
        ptr::write_unaligned(ptr::addr_of_mut!((*pic).pic.data).cast::<GlPic>(), gl);

        // Add to cache:
        debug_assert!((*pic).next.is_null());

        if Q_CACHEPICS.is_null() {
            Q_CACHEPICS = pic;
            Q_CACHEPICS_LAST_ENTRY = pic;
        } else {
            (*Q_CACHEPICS_LAST_ENTRY).next = pic;
            Q_CACHEPICS_LAST_ENTRY = pic;
        }
    }

    // SAFETY: `name` per the contract.
    let key = unsafe { truncated_name(name) };
    let mut map = Q_CACHEPICS_MAP.lock().unwrap_or_else(|e| e.into_inner());
    map.get_or_insert_with(HashMap::new)
        .insert(key, pic as usize);

    // SAFETY: `pic` is live.
    unsafe { ptr::addr_of_mut!((*pic).pic) }
}

/// `Draw_PicFromWad2`
///
/// # Safety
/// `name` NUL-terminated; main thread with gfx.wad loaded.
#[no_mangle]
pub unsafe extern "C" fn Draw_PicFromWad2(
    name: *const c_char,
    texflags: c_uint,
    picflags: c_int,
) -> *mut QPic {
    // Fast lookup:
    // SAFETY: per the contract.
    let p = unsafe { Draw_GetCachedPic(name) };

    if !p.is_null() {
        return p;
    }

    // not cached, searched for it:
    let mut info: *mut LumpInfo = ptr::null_mut();
    // SAFETY: per the contract.
    let p = unsafe { W_GetLumpName(name, &mut info) }.cast::<QPic>();

    // SAFETY: `pic_nul` is set by `Draw_Init`; the lump and its info are
    // valid for the loaded wad.
    unsafe {
        if p.is_null() {
            c::Con_Warning(c"W_GetLumpName: %s not found\n".as_ptr(), name);
            return pic_nul; // johnfitz
        }
        if (*info).type_ != TYP_QPIC {
            // can be another format that QPIC (.lmp), ex. png, tga, jpg, pcx , this is not an error at that point
            c::Con_DPrintf(
                c"Draw_PicFromWad: lump \"%s\" is not a qpic\n".as_ptr(),
                name,
            );
            return pic_nul; // johnfitz
        }

        // We have TYP_QPIC, check its basic characteristics:
        let width = (*p).width;
        let height = (*p).height;
        let size = (*info).size;
        if size < 8
            || (8usize).wrapping_add((width as usize).wrapping_mul(height as usize)) > size as usize
        {
            c::Con_Warning(c"Draw_PicFromWad: pic \"%s\" truncated\n".as_ptr(), name);
            return pic_nul; // johnfitz
        }

        if width < 0 || height < 0 {
            c::Con_Warning(
                c"Draw_PicFromWad: bad size (%dx%d) for pic \"%s\"\n".as_ptr(),
                width,
                height,
                name,
            );
            return pic_nul; // johnfitz
        }

        let data = ptr::addr_of!((*p).data).cast::<u8>();
        let gl;
        // load little ones into the scrap
        if width < 64 && height < 64 {
            let mut x = 0;
            let mut y = 0;
            let texnum = scrap_alloc_block(width, height, &mut x, &mut y);
            SCRAP_DIRTY = true;
            let mut k = 0usize;
            for i in 0..height as usize {
                for j in 0..width as usize {
                    SCRAP_TEXELS[texnum][(y as usize + i) * BLOCK_WIDTH + x as usize + j] =
                        *data.add(k);
                    k += 1;
                }
            }
            // johnfitz -- no longer go from 0.01 to 0.99
            gl = GlPic {
                gltexture: SCRAP_TEXTURES[texnum], // johnfitz -- changed to an array
                sl: x as f32 / BLOCK_WIDTH as f32,
                sh: (x + width) as f32 / BLOCK_WIDTH as f32,
                tl: y as f32 / BLOCK_WIDTH as f32,
                th: (y + height) as f32 / BLOCK_WIDTH as f32,
            };
        } else {
            let mut texturename = [0 as c_char; 64]; // johnfitz
            c::cl_main::q_snprintf(
                texturename.as_mut_ptr(),
                texturename.len(),
                c"%s:%s".as_ptr(),
                WADFILENAME.as_ptr(),
                name,
            ); // johnfitz

            let offset = (p as usize).wrapping_sub(wad_base as usize).wrapping_add(8); // johnfitz

            gl = GlPic {
                gltexture: TexMgr_LoadImage(
                    ptr::null_mut(),
                    texturename.as_ptr(),
                    width,
                    height,
                    SRC_INDEXED,
                    data.cast_mut(),
                    WADFILENAME.as_ptr(),
                    offset,
                    texflags,
                ), // johnfitz -- TexMgr
                sl: 0.0,
                sh: 1.0,
                tl: 0.0,
                th: 1.0,
            };
        }

        // Create a new pic:
        cache_new_pic(name, picflags, ptr::read_unaligned(p), gl)
    }
}

/// # Safety
/// As for `Draw_PicFromWad2`.
#[no_mangle]
pub unsafe extern "C" fn Draw_PicFromWad(name: *const c_char) -> *mut QPic {
    // SAFETY: per the contract.
    unsafe {
        Draw_PicFromWad2(
            name,
            TEXPREF_ALPHA | TEXPREF_PAD | TEXPREF_NOPICMIP,
            PICFLAG_AUTO,
        )
    }
}

/// `Draw_GetCachedPic` : get a pic from cache if already present, or return
/// NULL if not.
///
/// # Safety
/// `path` NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn Draw_GetCachedPic(path: *const c_char) -> *mut QPic {
    // SAFETY: per the contract.
    let key = unsafe { core::ffi::CStr::from_ptr(path) }.to_bytes();
    let map = Q_CACHEPICS_MAP.lock().unwrap_or_else(|e| e.into_inner());
    match map.as_ref().and_then(|m| m.get(key)) {
        // SAFETY: the map only holds live `Mem_Alloc` records (cleared with
        // the chain in `Draw_NewGame`).
        Some(&addr) => unsafe { ptr::addr_of_mut!((*(addr as *mut CachePic)).pic) },
        None => ptr::null_mut(),
    }
}

/// `Draw_TryCachePic`
///
/// # Safety
/// `path` NUL-terminated; main thread.
#[no_mangle]
pub unsafe extern "C" fn Draw_TryCachePic(
    path: *const c_char,
    texflags: c_uint,
    picflags: c_int,
) -> *mut QPic {
    // Fast lookup:
    // SAFETY: per the contract.
    let p = unsafe { Draw_GetCachedPic(path) };

    if !p.is_null() {
        return p;
    }

    //
    // load the pic from disk
    //
    let mut pic_width: c_int = 0;
    let mut pic_height: c_int = 0;
    let mut pic_fmt: c_int = SRC_INDEXED;

    // Image_LoadImage works without file extensions.
    let mut npath = [0 as c_char; MAX_QPATH];
    // SAFETY: per the contract; the out-pointers are live locals.
    let pic_data = unsafe {
        c::COM_StripExtension(path, npath.as_mut_ptr(), npath.len());
        c::render::Image_LoadImage(
            npath.as_ptr(),
            &mut pic_width,
            &mut pic_height,
            &mut pic_fmt,
            0,
        )
    };

    if pic_data.is_null() {
        return ptr::null_mut();
    }

    // pass the extensionless name as the source so TexMgr_ReloadImage can find the image
    // again through Image_LoadImage (needed to recolor gfx/menuplyr.lmp in the setup menu)
    let gl = GlPic {
        // SAFETY: `pic_data` is the loaded image; freed below like C.
        gltexture: unsafe {
            TexMgr_LoadImage(
                ptr::null_mut(),
                path,
                pic_width,
                pic_height,
                pic_fmt,
                pic_data,
                npath.as_ptr(),
                0,
                texflags | TEXPREF_NOPICMIP,
            )
        }, // johnfitz -- TexMgr
        // those are always normalized coordinates
        sl: 0.0,
        sh: 1.0,
        tl: 0.0,
        th: 1.0,
    };

    let header = QPic {
        width: pic_width,
        height: pic_height,
        data: [0; 4],
    };
    // SAFETY: per the contract.
    let r = unsafe { cache_new_pic(path, picflags, header, gl) };

    // SAFETY: `pic_data` came from `Image_LoadImage` (Mem_Alloc).
    unsafe { c::Mem_Free(pic_data.cast::<c_void>()) };

    r
}

/// # Safety
/// As for `Draw_TryCachePic`; `Sys_Error`s when the pic cannot be loaded.
#[no_mangle]
pub unsafe extern "C" fn Draw_CachePic(path: *const c_char) -> *mut QPic {
    // SAFETY: per the contract.
    let pic = unsafe {
        Draw_TryCachePic(
            path,
            TEXPREF_ALPHA | TEXPREF_PAD | TEXPREF_NOPICMIP,
            PICFLAG_AUTO,
        )
    };
    if pic.is_null() {
        // SAFETY: `Sys_Error` exits.
        unsafe { c::Sys_Error(c"Draw_CachePic: failed to load %s".as_ptr(), path) }
    }
    pic
}

/// `Draw_MakePic` -- johnfitz -- generate pics from internal data.
///
/// # Safety
/// `data` holds `width * height` indexed texels that outlive the texture.
unsafe fn draw_make_pic(
    name: &core::ffi::CStr,
    width: c_int,
    height: c_int,
    data: *const u8,
) -> *mut QPic {
    let flags = TEXPREF_NEAREST | TEXPREF_ALPHA | TEXPREF_PERSIST | TEXPREF_NOPICMIP | TEXPREF_PAD;

    // SAFETY: the record is `sizeof (qpic_t) - 4 + sizeof (glpic_t)` bytes,
    // enough for the header plus the payload written over `data`.
    unsafe {
        let pic = c::Mem_Alloc(core::mem::size_of::<QPic>() - 4 + core::mem::size_of::<GlPic>())
            .cast::<QPic>();
        (*pic).width = width;
        (*pic).height = height;

        let gl = GlPic {
            gltexture: TexMgr_LoadImage(
                ptr::null_mut(),
                name.as_ptr(),
                width,
                height,
                SRC_INDEXED,
                data.cast_mut(),
                c"".as_ptr(),
                data as usize,
                flags,
            ),
            sl: 0.0,
            sh: 1.0,
            tl: 0.0,
            th: 1.0,
        };

        ptr::write_unaligned(ptr::addr_of_mut!((*pic).data).cast::<GlPic>(), gl);

        pic
    }
}

//==============================================================================
//
//  INIT
//
//==============================================================================

/// `Draw_LoadPics` -- johnfitz
///
/// # Safety
/// gfx.wad loaded; main thread.
unsafe fn draw_load_pics() {
    let mut info: *mut LumpInfo = ptr::null_mut();
    // SAFETY: per the contract.
    unsafe {
        let data = W_GetLumpName(c"conchars".as_ptr(), &mut info).cast::<u8>();
        if data.is_null() {
            c::Sys_Error(c"Draw_LoadPics: couldn't load conchars".as_ptr());
        }
        let offset = (data as usize).wrapping_sub(wad_base as usize);
        char_texture = TexMgr_LoadImage(
            ptr::null_mut(),
            c"gfx.wad:conchars".as_ptr(),
            128,
            128,
            SRC_INDEXED,
            data,
            WADFILENAME.as_ptr(),
            offset,
            TEXPREF_ALPHA | TEXPREF_NEAREST | TEXPREF_NOPICMIP | TEXPREF_CONCHARS,
        );

        draw_disc = Draw_PicFromWad(c"disc".as_ptr());
        draw_backtile = Draw_PicFromWad(c"backtile".as_ptr());
    }
}

/// Clears the scrap bookkeeping and texels (`memset` 0 / 255).
///
/// # Safety
/// Main thread.
unsafe fn scrap_clear() {
    // SAFETY: per the contract.
    unsafe {
        ptr::write_bytes(ptr::addr_of_mut!(SCRAP_ALLOCATED), 0, 1);
        ptr::write_bytes(ptr::addr_of_mut!(SCRAP_TEXELS), 255, 1);
    }
}

/// `Draw_NewGame` -- johnfitz
///
/// # Safety
/// C ABI entry point; main thread with the renderer up.
#[no_mangle]
pub unsafe extern "C" fn Draw_NewGame() {
    // SAFETY: per the contract; `draw_qcvm_mutex` is created by `SCR_Init`.
    unsafe {
        c::QMutex_Lock(draw_qcvm_mutex);

        // empty scrap and reallocate gltextures
        scrap_clear();

        scrap_upload(); // creates 2 empty gltextures

        // empty pic cache :
        let mut cached_pic = Q_CACHEPICS;
        while !cached_pic.is_null() {
            let next_cached_pic = (*cached_pic).next;
            c::Mem_Free(cached_pic.cast::<c_void>());
            cached_pic = next_cached_pic;
        }
        Q_CACHEPICS = ptr::null_mut();
        Q_CACHEPICS_LAST_ENTRY = ptr::null_mut();
    }

    if let Some(map) = Q_CACHEPICS_MAP
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_mut()
    {
        map.clear();
    }

    // SAFETY: per the contract.
    unsafe {
        // reload wad pics
        W_LoadWadFile(); // johnfitz -- filename is now hard-coded for honesty
        draw_load_pics();
        SCR_LoadPics();
        Sbar_LoadPics();

        c::QMutex_Unlock(draw_qcvm_mutex);
    }
}

/// `Draw_Init` -- johnfitz -- rewritten
///
/// # Safety
/// C ABI entry point; called once from `Host_Init` after the texture manager
/// and gfx.wad are up.
#[no_mangle]
pub unsafe extern "C" fn Draw_Init() {
    *Q_CACHEPICS_MAP.lock().unwrap_or_else(|e| e.into_inner()) = Some(HashMap::new());

    // SAFETY: per the contract.
    unsafe {
        c::Cvar_RegisterVariable(ptr::addr_of_mut!(scr_conalpha));

        // clear scrap and allocate gltextures
        scrap_clear();

        scrap_upload(); // creates 2 empty textures

        // create internal pics
        pic_ins = draw_make_pic(c"ins", 8, 9, PIC_INS_DATA.as_ptr().cast::<u8>());
        pic_ovr = draw_make_pic(c"ovr", 8, 8, PIC_OVR_DATA.as_ptr().cast::<u8>());
        pic_nul = draw_make_pic(c"nul", 8, 8, PIC_NUL_DATA.as_ptr().cast::<u8>());

        // load game pics
        draw_load_pics();
    }
}

//==============================================================================
//
//  2D DRAWING
//
//==============================================================================

static mut CANVAS_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// `GL_SetCanvasColor`
///
/// # Safety
/// Main thread (the 2D pass is single-threaded).
#[no_mangle]
pub unsafe extern "C" fn GL_SetCanvasColor(r: c_float, g: c_float, b: c_float, a: c_float) {
    // SAFETY: per the contract.
    unsafe { CANVAS_COLOR = [r, g, b, a] };
}

/// A `basicvertex_t` `memset` to 255 (`gl_draw.c:589`).
const fn vertex_ff() -> BasicVertex {
    BasicVertex {
        position: [f32::from_bits(0xFFFF_FFFF); 3],
        texcoord: [f32::from_bits(0xFFFF_FFFF); 2],
        color: [255; 4],
    }
}

/// A `basicvertex_t` `memset` to 0 (`gl_draw.c:977`).
const fn vertex_zero() -> BasicVertex {
    BasicVertex {
        position: [0.0; 3],
        texcoord: [0.0; 2],
        color: [0; 4],
    }
}

/// The six-vertex quad every emitter writes (`0 1 2 2 3 0`).
///
/// # Safety
/// `output` points at six writable vertices.
unsafe fn write_quad(output: *mut BasicVertex, corner_verts: &[BasicVertex; 4]) {
    // SAFETY: per the contract.
    unsafe {
        *output = corner_verts[0];
        *output.add(1) = corner_verts[1];
        *output.add(2) = corner_verts[2];
        *output.add(3) = corner_verts[2];
        *output.add(4) = corner_verts[3];
        *output.add(5) = corner_verts[0];
    }
}

/// `Draw_FillCharacterQuad` -- `num` is C `char`, so the row of a high
/// glyph is negative where `char` is signed (x86) and positive where it is
/// unsigned (AArch64 Linux); `c_char` follows the same platform rule.
///
/// # Safety
/// `output` points at six writable vertices.
unsafe fn draw_fill_character_quad(
    x: f32,
    y: f32,
    num: c_char,
    output: *mut BasicVertex,
    rotation: c_int,
) {
    let row = (num as c_int) >> 4;
    let col = (num as c_int) & 15;
    let st_size = 1.0f32 / 16.0f32;
    // Fixes sampling into previous/next character because of float rounding
    let texel_offset = 0.001f32;
    let frow = row as f32 * st_size;
    let fcol = col as f32 * st_size;

    let mut corner_verts = [vertex_ff(); 4];

    let texcoords: [[f32; 2]; 4] = [
        [x, y],
        [x + CHARACTER_SIZE, y],
        [x + CHARACTER_SIZE, y + CHARACTER_SIZE],
        [x, y + CHARACTER_SIZE],
    ];

    // SAFETY: main thread.
    let canvas_color = unsafe { *ptr::addr_of!(CANVAS_COLOR) };
    for v in corner_verts.iter_mut() {
        for (dst, src) in v.color.iter_mut().zip(canvas_color) {
            *dst = (src * 255.0) as u8;
        }
    }

    let uv = [
        [fcol + texel_offset, frow + texel_offset],
        [fcol + st_size - texel_offset, frow + texel_offset],
        [fcol + st_size - texel_offset, frow + st_size - texel_offset],
        [fcol + texel_offset, frow + st_size - texel_offset],
    ];
    for (i, v) in corner_verts.iter_mut().enumerate() {
        let t = texcoords[((rotation + i as c_int) % 4) as usize];
        v.position = [t[0], t[1], 0.0];
        v.texcoord = uv[i];
    }

    // SAFETY: per the contract.
    unsafe { write_quad(output, &corner_verts) };
}

/// Binds the basic vertex stream and the blend/alphatest character pipeline
/// with `char_texture`, then draws `num_verts`.
///
/// # Safety
/// `cbx` valid; `char_texture` loaded.
unsafe fn draw_characters(
    cbx: &mut CbContext,
    blend: bool,
    buffer: vk::Buffer,
    buffer_offset: vk::DeviceSize,
    num_verts: u32,
) {
    let idx = cbx.render_pass_index as usize;
    with_ctx(|ctx| {
        let procs = CmdProcs::new(ctx.vg);
        let pipeline: VulkanPipeline = if blend {
            vg!(ctx, basic_blend_pipeline[idx])
        } else {
            vg!(ctx, basic_alphatest_pipeline[idx])
        };
        let layout: vk::PipelineLayout = vg!(ctx, basic_pipeline_layout.handle);
        // SAFETY: per the contract.
        let set = unsafe { (*char_texture).descriptor_set };
        // SAFETY: `cbx.cb` is recording; the handles are live.
        unsafe {
            ctx.device
                .cmd_bind_vertex_buffers(cbx.cb, 0, &[buffer], &[buffer_offset])
        };
        cb::bind_pipeline(&procs, cbx, vk::PipelineBindPoint::GRAPHICS, pipeline);
        // SAFETY: `cbx.cb` is recording; the handles are live.
        unsafe {
            ctx.device.cmd_bind_descriptor_sets(
                cbx.cb,
                vk::PipelineBindPoint::GRAPHICS,
                layout,
                0,
                &[set],
                &[],
            )
        };
        // SAFETY: `cbx.cb` is recording; the handles are live.
        unsafe { cb::draw(&CmdProcs::new(ctx.vg), cbx.cb, num_verts, 1, 0, 0) };
    });
}

/// `Draw_Character`
///
/// # Safety
/// `cbx` is a recording `cb_context_t`; main thread.
#[no_mangle]
pub unsafe extern "C" fn Draw_Character(cbx: *mut CbContext, x: c_float, y: c_float, num: c_int) {
    if y <= -CHARACTER_SIZE {
        return; // totally off screen
    }

    let rotation = (num / 256) % 4;
    let num = num & 255;

    if num == 32 {
        return; // don't waste verts on spaces
    }

    let a =
        with_ctx(|ctx| DYN.vertex_allocate(ctx, 6 * core::mem::size_of::<BasicVertex>() as u32));
    let vertices = a.data.cast::<BasicVertex>();

    // SAFETY: per the contract; the allocation holds six vertices.
    unsafe {
        draw_fill_character_quad(x, y, num as c_char, vertices, rotation);
        let blend = (*ptr::addr_of!(CANVAS_COLOR))[3] < 1.0;
        draw_characters(&mut *cbx, blend, a.buffer, a.buffer_offset, 6);
    }
}

/// `Draw_String`
///
/// # Safety
/// `cbx` is a recording `cb_context_t`; `str_` NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn Draw_String(
    cbx: *mut CbContext,
    x: c_float,
    y: c_float,
    str_: *const c_char,
) {
    if y <= -CHARACTER_SIZE {
        return; // totally off screen
    }

    // SAFETY: per the contract.
    let bytes = unsafe { core::ffi::CStr::from_ptr(str_) }.to_bytes();
    let num_verts = bytes.iter().filter(|&&ch| ch != 32).count() as u32 * 6;

    let a = with_ctx(|ctx| {
        DYN.vertex_allocate(ctx, num_verts * core::mem::size_of::<BasicVertex>() as u32)
    });
    let vertices = a.data.cast::<BasicVertex>();

    let mut x = x;
    let mut i = 0usize;
    for &ch in bytes {
        if ch != 32 {
            // SAFETY: `i * 6 + 6 <= num_verts`.
            unsafe { draw_fill_character_quad(x, y, ch as c_char, vertices.add(i * 6), 0) };
            i += 1;
        }
        x += CHARACTER_SIZE;
    }

    // SAFETY: per the contract.
    unsafe {
        let blend = (*ptr::addr_of!(CANVAS_COLOR))[3] < 1.0;
        draw_characters(&mut *cbx, blend, a.buffer, a.buffer_offset, num_verts);
    }
}

/// Reads the `glpic_t` payload behind a `qpic_t`.
///
/// # Safety
/// `pic` was produced by this file.
unsafe fn glpic_of(pic: *const QPic) -> GlPic {
    // SAFETY: per the contract (unaligned like the C memcpy).
    unsafe { ptr::read_unaligned(ptr::addr_of!((*pic).data).cast::<GlPic>()) }
}

/// Binds the vertex stream, the blend/alphatest pipeline and `set`, then
/// draws the six-vertex quad.
fn draw_textured_quad(
    cbx: &mut CbContext,
    alpha_blend: bool,
    set: vk::DescriptorSet,
    buffer: vk::Buffer,
    buffer_offset: vk::DeviceSize,
) {
    let idx = cbx.render_pass_index as usize;
    with_ctx(|ctx| {
        let procs = CmdProcs::new(ctx.vg);
        let pipeline: VulkanPipeline = if alpha_blend {
            vg!(ctx, basic_blend_pipeline[idx])
        } else {
            vg!(ctx, basic_alphatest_pipeline[idx])
        };
        let layout: vk::PipelineLayout = vg!(ctx, basic_pipeline_layout.handle);
        // SAFETY: `cbx.cb` is recording; the handles are live.
        unsafe {
            ctx.device
                .cmd_bind_vertex_buffers(cbx.cb, 0, &[buffer], &[buffer_offset])
        };
        cb::bind_pipeline(&procs, cbx, vk::PipelineBindPoint::GRAPHICS, pipeline);
        // SAFETY: `cbx.cb` is recording; the handles are live.
        unsafe {
            ctx.device.cmd_bind_descriptor_sets(
                cbx.cb,
                vk::PipelineBindPoint::GRAPHICS,
                layout,
                0,
                &[set],
                &[],
            )
        };
        // SAFETY: `cbx.cb` is recording; the handles are live.
        unsafe { cb::draw(&CmdProcs::new(ctx.vg), cbx.cb, 6, 1, 0, 0) };
    });
}

/// `Draw_Pic` -- johnfitz -- modified
///
/// # Safety
/// `cbx` recording; `pic` from this file's cache.
#[no_mangle]
pub unsafe extern "C" fn Draw_Pic(
    cbx: *mut CbContext,
    x: c_float,
    y: c_float,
    pic: *mut QPic,
    alpha: c_float,
    alpha_blend: bool,
) {
    // SAFETY: per the contract.
    unsafe {
        if SCRAP_DIRTY {
            scrap_upload();
        }
    }
    // SAFETY: per the contract.
    let (gl, width, height) = unsafe { (glpic_of(pic), (*pic).width as f32, (*pic).height as f32) };

    let a =
        with_ctx(|ctx| DYN.vertex_allocate(ctx, 6 * core::mem::size_of::<BasicVertex>() as u32));
    let vertices = a.data.cast::<BasicVertex>();

    let mut corner_verts = [vertex_ff(); 4];

    corner_verts[0].position = [x, y, 0.0];
    corner_verts[0].texcoord = [gl.sl, gl.tl];

    corner_verts[1].position = [x + width, y, 0.0];
    corner_verts[1].texcoord = [gl.sh, gl.tl];

    corner_verts[2].position = [x + width, y + height, 0.0];
    corner_verts[2].texcoord = [gl.sh, gl.th];

    corner_verts[3].position = [x, y + height, 0.0];
    corner_verts[3].texcoord = [gl.sl, gl.th];

    for v in corner_verts.iter_mut() {
        v.color[3] = (alpha * 255.0) as u8;
    }

    // SAFETY: the allocation holds six vertices; `gl.gltexture` is live.
    unsafe {
        write_quad(vertices, &corner_verts);
        draw_textured_quad(
            &mut *cbx,
            alpha_blend,
            (*gl.gltexture).descriptor_set,
            a.buffer,
            a.buffer_offset,
        );
    }
}

/// `Draw_SubPic`
///
/// # Safety
/// `cbx` recording; `pic` from this file's cache; `rgb` NULL or three floats.
#[no_mangle]
pub unsafe extern "C" fn Draw_SubPic(
    cbx: *mut CbContext,
    x: c_float,
    y: c_float,
    w: c_float,
    h: c_float,
    pic: *mut QPic,
    s1: c_float,
    t1: c_float,
    s2: c_float,
    t2: c_float,
    rgb: *mut c_float,
    alpha: c_float,
) {
    let alpha_blend = alpha < 1.0;
    if alpha <= 0.0 {
        return;
    }

    let s2 = s2 + s1;
    let t2 = t2 + t1;

    // SAFETY: per the contract.
    unsafe {
        if SCRAP_DIRTY {
            scrap_upload();
        }
    }
    // SAFETY: per the contract.
    let gl = unsafe { glpic_of(pic) };
    if gl.gltexture.is_null() {
        return;
    }

    let mut rgba = [255.0f32; 4];
    if !rgb.is_null() {
        for (i, v) in rgba.iter_mut().enumerate().take(3) {
            // SAFETY: per the contract.
            *v *= unsafe { *rgb.add(i) };
        }
    }
    rgba[3] *= alpha;

    let a =
        with_ctx(|ctx| DYN.vertex_allocate(ctx, 6 * core::mem::size_of::<BasicVertex>() as u32));
    let vertices = a.data.cast::<BasicVertex>();

    let mut corner_verts = [vertex_ff(); 4];

    corner_verts[0].position = [x, y, 0.0];
    corner_verts[0].texcoord = [
        gl.sl * (1.0 - s1) + s1 * gl.sh,
        gl.tl * (1.0 - t1) + t1 * gl.th,
    ];

    corner_verts[1].position = [x + w, y, 0.0];
    corner_verts[1].texcoord = [
        gl.sl * (1.0 - s2) + s2 * gl.sh,
        gl.tl * (1.0 - t1) + t1 * gl.th,
    ];

    corner_verts[2].position = [x + w, y + h, 0.0];
    corner_verts[2].texcoord = [
        gl.sl * (1.0 - s2) + s2 * gl.sh,
        gl.tl * (1.0 - t2) + t2 * gl.th,
    ];

    corner_verts[3].position = [x, y + h, 0.0];
    corner_verts[3].texcoord = [
        gl.sl * (1.0 - s1) + s1 * gl.sh,
        gl.tl * (1.0 - t2) + t2 * gl.th,
    ];

    for v in corner_verts.iter_mut() {
        for (dst, src) in v.color.iter_mut().zip(rgba) {
            *dst = src as u8;
        }
    }

    // SAFETY: the allocation holds six vertices; `gl.gltexture` is live.
    unsafe {
        write_quad(vertices, &corner_verts);
        draw_textured_quad(
            &mut *cbx,
            alpha_blend,
            (*gl.gltexture).descriptor_set,
            a.buffer,
            a.buffer_offset,
        );
    }
}

/// `Draw_TransPicTranslate` -- johnfitz -- rewritten to use texmgr to do
/// translation. Only used for the player color selection menu.
///
/// # Safety
/// As for `Draw_Pic`.
#[no_mangle]
pub unsafe extern "C" fn Draw_TransPicTranslate(
    cbx: *mut CbContext,
    x: c_float,
    y: c_float,
    pic: *mut QPic,
    top: c_int,
    bottom: c_int,
) {
    static mut OLDTOP: c_int = -2;
    static mut OLDBOTTOM: c_int = -2;

    // SAFETY: main thread; per the contract.
    unsafe {
        if top != OLDTOP || bottom != OLDBOTTOM {
            let glt = glpic_of(pic).gltexture;
            OLDTOP = top;
            OLDBOTTOM = bottom;
            TexMgr_ReloadImage(glt, top, bottom);
        }
        Draw_Pic(cbx, x, y, pic, 1.0, false);
    }
}

/// `Draw_ConsoleBackground` -- johnfitz -- rewritten
///
/// # Safety
/// `cbx` recording; main thread.
#[no_mangle]
pub unsafe extern "C" fn Draw_ConsoleBackground(cbx: *mut CbContext) {
    // SAFETY: per the contract.
    unsafe {
        let pic = Draw_CachePic(c"gfx/conback.lmp".as_ptr());
        (*pic).width = c::cl_parse::vid.conwidth;
        (*pic).height = c::cl_parse::vid.conheight;

        let alpha: f32 = if c::console::con_forcedup {
            1.0
        } else {
            scr_conalpha.value
        };

        GL_SetCanvas(cbx, CANVAS_CONSOLE); // in case this is called from weird places

        if alpha > 0.0 {
            Draw_Pic(cbx, 0.0, 0.0, pic, alpha, alpha < 1.0);
        }
    }
}

/// `Draw_TileClear` -- this repeats a 64*64 tile graphic to fill the screen
/// around a sized down refresh window.
///
/// # Safety
/// `cbx` recording; `draw_backtile` loaded.
#[no_mangle]
pub unsafe extern "C" fn Draw_TileClear(
    cbx: *mut CbContext,
    x: c_float,
    y: c_float,
    w: c_float,
    h: c_float,
) {
    // SAFETY: per the contract.
    let gl = unsafe { glpic_of(draw_backtile) };

    let a =
        with_ctx(|ctx| DYN.vertex_allocate(ctx, 6 * core::mem::size_of::<BasicVertex>() as u32));
    let vertices = a.data.cast::<BasicVertex>();

    let mut corner_verts = [vertex_ff(); 4];

    let d64 = |v: f32| (v as f64 / 64.0) as f32;

    corner_verts[0].position = [x, y, 0.0];
    corner_verts[0].texcoord = [d64(x), d64(y)];

    corner_verts[1].position = [x + w, y, 0.0];
    corner_verts[1].texcoord = [d64(x + w), d64(y)];

    corner_verts[2].position = [x + w, y + h, 0.0];
    corner_verts[2].texcoord = [d64(x + w), d64(y + h)];

    corner_verts[3].position = [x, y + h, 0.0];
    corner_verts[3].texcoord = [d64(x), d64(y + h)];

    // SAFETY: the allocation holds six vertices.
    unsafe { write_quad(vertices, &corner_verts) };

    // SAFETY: per the contract.
    let cbx = unsafe { &mut *cbx };
    let idx = cbx.render_pass_index as usize;
    with_ctx(|ctx| {
        let procs = CmdProcs::new(ctx.vg);
        let pipeline: VulkanPipeline = vg!(ctx, basic_blend_pipeline[idx]);
        let layout: vk::PipelineLayout = vg!(ctx, basic_pipeline_layout.handle);
        // SAFETY: `gl.gltexture` is live.
        let set = unsafe { (*gl.gltexture).descriptor_set };
        cb::bind_pipeline(&procs, cbx, vk::PipelineBindPoint::GRAPHICS, pipeline);
        // SAFETY: `cbx.cb` is recording; the handles are live.
        unsafe {
            ctx.device.cmd_bind_descriptor_sets(
                cbx.cb,
                vk::PipelineBindPoint::GRAPHICS,
                layout,
                0,
                &[set],
                &[],
            )
        };
        // SAFETY: `cbx.cb` is recording; the handles are live.
        unsafe {
            ctx.device
                .cmd_bind_vertex_buffers(cbx.cb, 0, &[a.buffer], &[a.buffer_offset])
        };
        // SAFETY: `cbx.cb` is recording; the handles are live.
        unsafe { cb::draw(&CmdProcs::new(ctx.vg), cbx.cb, 6, 1, 0, 0) };
    });
}

/// Binds the vertex stream and the untextured blend pipeline, then draws
/// the six-vertex quad (`Draw_Fill`, `Draw_FadeScreen`).
fn draw_notex_quad(cbx: &mut CbContext, buffer: vk::Buffer, buffer_offset: vk::DeviceSize) {
    let idx = cbx.render_pass_index as usize;
    with_ctx(|ctx| {
        let procs = CmdProcs::new(ctx.vg);
        let pipeline: VulkanPipeline = vg!(ctx, basic_notex_blend_pipeline[idx]);
        // SAFETY: `cbx.cb` is recording; the handles are live.
        unsafe {
            ctx.device
                .cmd_bind_vertex_buffers(cbx.cb, 0, &[buffer], &[buffer_offset])
        };
        cb::bind_pipeline(&procs, cbx, vk::PipelineBindPoint::GRAPHICS, pipeline);
        // SAFETY: `cbx.cb` is recording; the handles are live.
        unsafe { cb::draw(&CmdProcs::new(ctx.vg), cbx.cb, 6, 1, 0, 0) };
    });
}

/// `Draw_Fill` -- fills a box of pixels with a single color.
///
/// # Safety
/// `cbx` recording; `c` a palette index.
#[no_mangle]
pub unsafe extern "C" fn Draw_Fill(
    cbx: *mut CbContext,
    x: c_float,
    y: c_float,
    w: c_float,
    h: c_float,
    c_: c_int,
    alpha: c_float,
) {
    // johnfitz -- use d_8to24table instead of host_basepal
    // SAFETY: `c` indexes the 256-entry palette like C.
    let pal = unsafe { (*ptr::addr_of!(c::render::d_8to24table))[c_ as usize] }.to_ne_bytes();

    let a =
        with_ctx(|ctx| DYN.vertex_allocate(ctx, 6 * core::mem::size_of::<BasicVertex>() as u32));
    let vertices = a.data.cast::<BasicVertex>();

    let mut corner_verts = [vertex_zero(); 4];

    corner_verts[0].position[0] = x;
    corner_verts[0].position[1] = y;

    corner_verts[1].position[0] = x + w;
    corner_verts[1].position[1] = y;

    corner_verts[2].position[0] = x + w;
    corner_verts[2].position[1] = y + h;

    corner_verts[3].position[0] = x;
    corner_verts[3].position[1] = y + h;

    for v in corner_verts.iter_mut() {
        v.color = [pal[0], pal[1], pal[2], (alpha * 255.0) as u8];
    }

    // SAFETY: the allocation holds six vertices; `cbx` per the contract.
    unsafe {
        write_quad(vertices, &corner_verts);
        draw_notex_quad(&mut *cbx, a.buffer, a.buffer_offset);
    }
}

/// `Draw_FadeScreen`
///
/// # Safety
/// `cbx` recording.
#[no_mangle]
pub unsafe extern "C" fn Draw_FadeScreen(cbx: *mut CbContext) {
    // SAFETY: per the contract.
    unsafe { GL_SetCanvas(cbx, CANVAS_DEFAULT) };

    let a =
        with_ctx(|ctx| DYN.vertex_allocate(ctx, 6 * core::mem::size_of::<BasicVertex>() as u32));
    let vertices = a.data.cast::<BasicVertex>();

    // SAFETY: plain reads of the C ints.
    let (glwidth, glheight) = unsafe { (c::console::glwidth as f32, c::console::glheight as f32) };

    let mut corner_verts = [vertex_zero(); 4];

    corner_verts[1].position[0] = glwidth;

    corner_verts[2].position[0] = glwidth;
    corner_verts[2].position[1] = glheight;

    corner_verts[3].position[1] = glheight;

    for v in corner_verts.iter_mut() {
        v.color[3] = 128;
    }

    // SAFETY: the allocation holds six vertices; `cbx` per the contract.
    unsafe {
        write_quad(vertices, &corner_verts);
        draw_notex_quad(&mut *cbx, a.buffer, a.buffer_offset);
    }
}

/// `GL_OrthoMatrix`
fn gl_ortho_matrix(
    cbx: &mut CbContext,
    left: f32,
    right: f32,
    bottom: f32,
    top: f32,
    n: f32,
    f: f32,
) {
    let tx = -(right + left) / (right - left);
    let ty = (top + bottom) / (top - bottom);
    let tz = -(f + n) / (f - n);

    let mut matrix = [0.0f32; 16];

    // First column
    matrix[0] = 2.0 / (right - left);

    // Second column
    matrix[5] = -2.0 / (top - bottom);

    // Third column
    matrix[10] = -2.0 / (f - n);

    // Fourth column
    matrix[12] = tx;
    matrix[13] = ty;
    matrix[14] = tz;
    matrix[15] = 1.0;

    let bytes: Vec<u8> = matrix.iter().flat_map(|v| v.to_ne_bytes()).collect();
    with_ctx(|ctx| {
        let procs = CmdProcs::new(ctx.vg);
        cb::push_constants(&procs, cbx, vk::ShaderStageFlags::ALL_GRAPHICS, 0, &bytes);
    });
}

/// `GL_Viewport`
///
/// # Safety
/// `cbx` recording.
#[no_mangle]
pub unsafe extern "C" fn GL_Viewport(
    cbx: *mut CbContext,
    x: c_float,
    y: c_float,
    width: c_float,
    height: c_float,
    min_depth: c_float,
    max_depth: c_float,
) {
    // SAFETY: per the contract; `vid` is the C video state.
    let (cb, vid_height) = unsafe { ((*cbx).cb, c::cl_parse::vid.height as f32) };
    let viewport = vk::Viewport {
        x,
        y: vid_height - (y + height),
        width,
        height,
        min_depth,
        max_depth,
    };
    // SAFETY: `cb` is recording.
    unsafe { device().cmd_set_viewport(cb, 0, &[viewport]) };
}

/// C `CLAMP (1.0, x, max)` with a `double` lower bound and the given upper
/// bound (`x < 1.0 ? 1.0 : x > max ? max : x`).
fn clamp1(x: f32, max: f32) -> f32 {
    if (x as f64) < 1.0 {
        1.0
    } else if x > max {
        max
    } else {
        x
    }
}

/// `GL_SetCanvas` -- johnfitz -- support various canvas types
///
/// # Safety
/// `cbx` recording; main thread (reads the screen/menu cvars and `cl`).
#[no_mangle]
pub unsafe extern "C" fn GL_SetCanvas(cbx: *mut CbContext, newcanvas: c_int) {
    // SAFETY: per the contract.
    let cbx = unsafe { &mut *cbx };
    if newcanvas == cbx.current_canvas {
        return;
    }

    cbx.current_canvas = newcanvas;

    // SAFETY: plain reads of C globals on the main thread.
    let (glwidth, glheight, vid, vrect) = unsafe {
        (
            c::console::glwidth,
            c::console::glheight,
            c::cl_parse::vid,
            *ptr::addr_of!(scr_vrect),
        )
    };
    let glw = glwidth as f32;
    let glh = glheight as f32;
    const BIG: f32 = 99999.0;

    match newcanvas {
        CANVAS_NONE => {}
        CANVAS_DEFAULT => {
            gl_ortho_matrix(cbx, 0.0, glw, glh, 0.0, -BIG, BIG);
            // SAFETY: `cbx` is live.
            unsafe { GL_Viewport(cbx, 0.0, 0.0, glw, glh, 0.0, 1.0) };
        }
        CANVAS_CONSOLE => {
            // SAFETY: plain read.
            let con_current = unsafe { c::console::scr_con_current };
            let lines =
                (vid.conheight as f32 - (con_current * vid.conheight as f32 / glh)) as c_int;
            gl_ortho_matrix(
                cbx,
                0.0,
                vid.conwidth as f32,
                (vid.conheight + lines) as f32,
                lines as f32,
                -BIG,
                BIG,
            );
            // SAFETY: `cbx` is live.
            unsafe { GL_Viewport(cbx, 0.0, 0.0, glw, glh, 0.0, 1.0) };
        }
        CANVAS_MENU => {
            // `q_min` over doubles, then `CLAMP (1.0, M_GetScale (), s)`.
            let s = f64::min(glwidth as f64 / 320.0, glheight as f64 / 200.0) as f32;
            // SAFETY: `M_GetScale` reads the menu scale cvar.
            let s = clamp1(unsafe { M_GetScale() }, s);
            let u = (glw - (320.0 * s)) / (2.0 * s);
            let v = (glh - (200.0 * s)) / (2.0 * s);
            gl_ortho_matrix(cbx, -u, 320.0 + u, 200.0 + v, -v, -BIG, BIG);
            // SAFETY: `cbx` is live.
            unsafe { GL_Viewport(cbx, 0.0, 0.0, glw, glh, 0.0, 1.0) };
        }
        CANVAS_CSQC => {
            // SAFETY: plain cvar read.
            let s = clamp1(
                unsafe { scr_sbarscale.value },
                (glwidth as f64 / 320.0) as f32,
            );
            gl_ortho_matrix(cbx, 0.0, glw / s, glh / s, 0.0, -BIG, BIG);
            // SAFETY: `cbx` is live.
            unsafe { GL_Viewport(cbx, 0.0, 0.0, glw, glh, 0.0, 1.0) };
        }
        CANVAS_SBAR => {
            // SAFETY: plain cvar/`cl` reads.
            let (s, deathmatch, style) = unsafe {
                (
                    clamp1(scr_sbarscale.value, (glwidth as f64 / 320.0) as f32),
                    (*ptr::addr_of!(cl)).gametype == GAME_DEATHMATCH,
                    scr_style.value,
                )
            };
            if deathmatch && style < 2.0 {
                gl_ortho_matrix(cbx, 0.0, glw / s, 48.0, 0.0, -BIG, BIG);
                // SAFETY: `cbx` is live.
                unsafe { GL_Viewport(cbx, 0.0, 0.0, glw, 48.0 * s, 0.0, 1.0) };
            } else {
                gl_ortho_matrix(cbx, 0.0, 320.0, 48.0, 0.0, -BIG, BIG);
                // SAFETY: `cbx` is live.
                unsafe {
                    GL_Viewport(
                        cbx,
                        (glw - 320.0 * s) / 2.0,
                        0.0,
                        320.0 * s,
                        48.0 * s,
                        0.0,
                        1.0,
                    )
                };
            }
        }
        CANVAS_WARPIMAGE => {
            gl_ortho_matrix(cbx, 0.0, 128.0, 0.0, 128.0, -BIG, BIG);
            // SAFETY: `cbx` is live.
            unsafe {
                GL_Viewport(
                    cbx,
                    0.0,
                    glh - WARPIMAGESIZE,
                    WARPIMAGESIZE,
                    WARPIMAGESIZE,
                    0.0,
                    1.0,
                )
            };
        }
        CANVAS_CROSSHAIR => {
            // 0,0 is center of viewport
            // SAFETY: plain cvar read.
            let s = clamp1(unsafe { scr_crosshairscale.value }, 10.0);
            gl_ortho_matrix(
                cbx,
                (vrect.width / -2) as f32 / s,
                (vrect.width / 2) as f32 / s,
                (vrect.height / 2) as f32 / s,
                (vrect.height / -2) as f32 / s,
                -BIG,
                BIG,
            );
            // SAFETY: `cbx` is live.
            unsafe {
                GL_Viewport(
                    cbx,
                    vrect.x as f32,
                    (glheight - vrect.y - vrect.height) as f32,
                    (vrect.width & !1) as f32,
                    (vrect.height & !1) as f32,
                    0.0,
                    1.0,
                )
            };
        }
        CANVAS_BOTTOMLEFT => {
            // used by devstats
            let s = glw / vid.conwidth as f32; // use console scale
            gl_ortho_matrix(cbx, 0.0, 320.0, 200.0, 0.0, -BIG, BIG);
            // SAFETY: `cbx` is live.
            unsafe { GL_Viewport(cbx, 0.0, 0.0, 320.0 * s, 200.0 * s, 0.0, 1.0) };
        }
        CANVAS_TOPLEFT => {
            // for modern HUD frag counter
            let s = glw / vid.conwidth as f32; // use console scale
            gl_ortho_matrix(cbx, 0.0, 320.0, 200.0, 0.0, -BIG, BIG);
            // SAFETY: `cbx` is live.
            unsafe { GL_Viewport(cbx, 0.0, glh - 200.0 * s, 320.0 * s, 200.0 * s, 0.0, 1.0) };
        }
        CANVAS_BOTTOMRIGHT => {
            // used by fps/clock
            let s = glw / vid.conwidth as f32; // use console scale
            gl_ortho_matrix(cbx, 0.0, 320.0, 200.0, 0.0, -BIG, BIG);
            // SAFETY: `cbx` is live.
            unsafe { GL_Viewport(cbx, glw - 320.0 * s, 0.0, 320.0 * s, 200.0 * s, 0.0, 1.0) };
        }
        CANVAS_TOPRIGHT => {
            // for modern HUD weapon icons
            let s = glw / vid.conwidth as f32; // use console scale
            gl_ortho_matrix(cbx, 0.0, 320.0, 200.0, 0.0, -BIG, BIG);
            // SAFETY: `cbx` is live.
            unsafe {
                GL_Viewport(
                    cbx,
                    glw - 320.0 * s,
                    glh - 200.0 * s,
                    320.0 * s,
                    200.0 * s,
                    0.0,
                    1.0,
                )
            };
        }
        // SAFETY: `Sys_Error` exits.
        _ => unsafe { c::Sys_Error(c"GL_SetCanvas: bad canvas type".as_ptr()) },
    }
}

//==============================================================================
//
//  3D BILLBOARD DRAWING
//
//==============================================================================

/// `Draw_FillCharacterQuad_3D`
///
/// # Safety
/// `output` points at six writable vertices; main thread (`vup`/`vright`).
unsafe fn draw_fill_character_quad_3d(
    coords: &Vec3,
    xoff: f32,
    yoff: f32,
    size: f32,
    num: c_char,
    output: *mut BasicVertex,
) {
    let xoff = xoff * size;
    let yoff = yoff * size;

    let row = (num as c_int) >> 4;
    let col = (num as c_int) & 15;

    let frow = (row as f64 * 0.0625) as f32;
    let fcol = (col as f64 * 0.0625) as f32;
    let tile_size = 0.0625f32;

    let mut corner_verts = [vertex_ff(); 4];

    // SAFETY: per the contract.
    let (vup, vright) = unsafe {
        (
            *ptr::addr_of!(c::host::vup),
            *ptr::addr_of!(c::host::vright),
        )
    };

    let mut p0 = Vec3::default();
    vector_ma(coords, size / 2.0 - yoff, &vup, &mut p0);
    let mut p0b = Vec3::default();
    vector_ma(&p0, -size / 2.0 + xoff, &vright, &mut p0b);
    corner_verts[0].position = p0b;
    corner_verts[0].texcoord = [fcol, frow];

    let mut p1 = Vec3::default();
    vector_ma(&p0b, size, &vright, &mut p1);
    corner_verts[1].position = p1;
    corner_verts[1].texcoord = [fcol + tile_size, frow];

    let mut p2 = Vec3::default();
    vector_ma(&p1, -size, &vup, &mut p2);
    corner_verts[2].position = p2;
    corner_verts[2].texcoord = [fcol + tile_size, frow + tile_size];

    let mut p3 = Vec3::default();
    vector_ma(&p2, -size, &vright, &mut p3);
    corner_verts[3].position = p3;
    corner_verts[3].texcoord = [fcol, frow + tile_size];

    // SAFETY: per the contract.
    unsafe { write_quad(output, &corner_verts) };
}

/// `Draw_String_3D`
///
/// # Safety
/// `cbx` recording; `coords` three floats; `str_` NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn Draw_String_3D(
    cbx: *mut CbContext,
    coords: *const c_float,
    size: c_float,
    str_: *const c_char,
) {
    // SAFETY: per the contract.
    let (bytes, coords) = unsafe {
        (
            core::ffi::CStr::from_ptr(str_).to_bytes(),
            *coords.cast::<Vec3>(),
        )
    };
    let num_verts = bytes.iter().filter(|&&ch| ch != 32).count() as u32 * 6;

    let a = with_ctx(|ctx| {
        DYN.vertex_allocate(ctx, num_verts * core::mem::size_of::<BasicVertex>() as u32)
    });
    let vertices = a.data.cast::<BasicVertex>();

    let mut xoff = -0.5f32 * bytes.len() as f32 + 0.5;

    let mut i = 0usize;
    for &ch in bytes {
        if ch != 32 {
            // SAFETY: `i * 6 + 6 <= num_verts`.
            unsafe {
                draw_fill_character_quad_3d(
                    &coords,
                    xoff,
                    0.0,
                    size,
                    ch as c_char,
                    vertices.add(i * 6),
                )
            };
            i += 1;
        }
        xoff += 1.0;
    }

    // SAFETY: per the contract.
    let cbx = unsafe { &mut *cbx };
    let idx = cbx.render_pass_index as usize;
    with_ctx(|ctx| {
        let procs = CmdProcs::new(ctx.vg);
        let pipeline: VulkanPipeline = vg!(ctx, basic_alphatest_pipeline[idx]);
        let layout: vk::PipelineLayout = vg!(ctx, basic_pipeline_layout.handle);
        // SAFETY: `char_texture` is loaded.
        let set = unsafe { (*char_texture).descriptor_set };
        cb::bind_pipeline(&procs, cbx, vk::PipelineBindPoint::GRAPHICS, pipeline);
        // SAFETY: `cbx.cb` is recording; the handles are live.
        unsafe {
            ctx.device
                .cmd_bind_vertex_buffers(cbx.cb, 0, &[a.buffer], &[a.buffer_offset])
        };
        // SAFETY: `cbx.cb` is recording; the handles are live.
        unsafe {
            ctx.device.cmd_bind_descriptor_sets(
                cbx.cb,
                vk::PipelineBindPoint::GRAPHICS,
                layout,
                0,
                &[set],
                &[],
            )
        };
        // SAFETY: `cbx.cb` is recording; the handles are live.
        unsafe { cb::draw(&CmdProcs::new(ctx.vg), cbx.cb, num_verts, 1, 0, 0) };
    });
}
