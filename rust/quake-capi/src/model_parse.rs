//! Brush/BSP loader shims (Quake/model_parse.c, Phase 3 M3)
//!
//! Each export replicates its C original's signature from
//! `Quake/model_parse.h`, its Mem_* allocation sizes and ownership
//! (ADR-013), its console output byte for byte, and the exact set of
//! `qmodel_t` fields it writes (fields the C leaves untouched — notably the
//! `Mem_AllocNonZero` tail of `msurface_t` — stay untouched here too).
//! The record decoding itself lives in the pure `quake_formats::bsp`
//! modules; this file only moves bytes into engine memory.
//!
//! PLAN.md section 4.3: `Host_Error` longjmps must never unwind a Rust
//! frame, so the four Host_Error-capable entry points are exported as
//! `quake_rs_mod_*` status functions (1 = ok, 0 = the C would have raised
//! Host_Error) with a 256-byte `err` out-parameter holding the message;
//! `Quake/model_parse_glue.c` re-raises it from a pure C frame. `Sys_Error`
//! aborts, so it is called directly (M2 precedent).
//!
//! COMPAT: the C originals read out of bounds on malformed input in several
//! places (texture headers past the lump end, negative offsets, a `.lit`
//! file shorter than its 8-byte header, `mod->submodels[0]` with zero
//! submodels). Where the pure parsers bound those reads, these shims take
//! the bounded path and behave as if the out-of-range bytes were absent;
//! every such site is marked below. `CalcSurfaceExtents` diverges once
//! more: on `Sys_Error ("Bad surface extents")` the C has already written
//! the axis-0 `texturemins`/`extents`, this shim writes neither (the
//! process aborts either way; only a Sys_Error-trapping harness can see
//! the difference).

use core::ffi::{c_char, c_int, c_long, c_uint, c_void};
use core::ptr::{addr_of, addr_of_mut, null_mut};
use std::ffi::{CStr, CString};

use quake_c_sys as sys;
use quake_formats::bsp::{
    extents, lighting, lumps,
    lumps::{LeafError, MarkError, NodeChild},
    textures,
    textures::TexWork,
    Bsp2,
};
use quake_types::bspfile::{
    DModel, LumpT, BSPVERSION_QUAKE64, BSPVERSION_VALVE, MAX_MAP_HULLS, MIPLEVELS, TEX_SPECIAL,
};
use quake_types::model_mem::{
    MClipnode, MEdge, MLeaf, MNode, MSurface, MTexInfo, MVertex, QModel, Texture, MAX_QPATH,
    SURF_DRAWLAVA, SURF_DRAWSKY, SURF_DRAWSLIME, SURF_DRAWTELE, SURF_DRAWTURB, SURF_DRAWWATER,
    SURF_PLANEBACK, TEXTYPE_COUNT,
};
use quake_types::MPlane;
use quake_util::crc::crc_block;
use quake_util::strl::{strlcat, strlcpy};

const SIZEOF_TEXTURE: usize = core::mem::size_of::<Texture>();
const SIZEOF_MIPTEX: usize = 40;
const VISPATCH_HEADER_LEN: usize = 36;
const SEEK_SET: c_int = 0;

unsafe extern "C" {
    // console.h: not a bindgen-clean root, so the one console function the
    // brush loaders use that the c-sys wrapper does not declare is declared
    // here; the signature must match console.h exactly.
    fn Con_DWarning(fmt: *const c_char, ...);
    // gl_model.c: stay C (Phase 3 keeps the model cache and the wad
    // texture loader on the C side)
    fn Mod_FindName(name: *const c_char) -> *mut QModel;
    fn Mod_LoadWadTexture(m: *mut QModel, wads: *mut c_void, name: *const c_char) -> *mut Texture;
    // stdio, as used by the external-vis trio
    fn fread(ptr: *mut c_void, size: usize, nmemb: usize, stream: *mut sys::FILE) -> usize;
    fn fclose(stream: *mut sys::FILE) -> c_int;
}

/// The C `bsp2` int (0 = BSP29/BSP30, 1 = 2PSB, 2 = BSP2)
fn dialect_of(bsp2: c_int) -> Bsp2 {
    match bsp2 {
        0 => Bsp2::No,
        2 => Bsp2::L2,
        _ => Bsp2::L1,
    }
}

/// C `count * sizeof (x)`: the int is converted to size_t, sign extension
/// included.
fn alloc_size(count: c_int, elem: usize) -> usize {
    (count as isize as usize).wrapping_mul(elem)
}

/// # Safety
/// `p` must point at a NUL-terminated string that outlives `'a`.
unsafe fn cstr_bytes<'a>(p: *const c_char) -> &'a [u8] {
    let mut n = 0usize;
    // SAFETY: caller guarantees a NUL-terminated string
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(p.cast::<u8>(), n)
    }
}

/// # Safety
/// `m` must point at a live qmodel_t.
unsafe fn model_name(m: *const QModel) -> *const c_char {
    // SAFETY: caller guarantees a live qmodel_t
    unsafe { addr_of!((*m).name).cast::<c_char>() }
}

/// # Safety
/// `mod_base + l->fileofs` must be readable for `l->filelen` bytes.
unsafe fn lump_slice<'a>(mod_base: *mut u8, l: *const LumpT) -> &'a [u8] {
    // SAFETY: caller guarantees the lump is in the loaded file image
    unsafe {
        let len = (*l).filelen.max(0) as usize;
        if len == 0 {
            return &[];
        }
        core::slice::from_raw_parts(mod_base.offset((*l).fileofs as isize), len)
    }
}

/// A C string for a `%s` argument, cut at the first NUL like printf.
fn cstring(bytes: &[u8]) -> CString {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    CString::new(&bytes[..end]).unwrap_or_default()
}

/// `q_snprintf (dst, sizeof (dst), ...)` of an already-formatted string.
fn snprintf_into(dst: &mut [u8], s: &str) {
    debug_assert!(!dst.is_empty(), "q_snprintf needs room for the NUL");
    let n = s.len().min(dst.len() - 1);
    dst[..n].copy_from_slice(&s.as_bytes()[..n]);
    dst[n] = 0;
}

/// `q_snprintf (dst, sizeof (dst), ...)` of a byte-wise concatenation, so
/// non-UTF-8 path bytes survive unchanged.
fn snprintf_parts(dst: &mut [u8], parts: &[&[u8]]) {
    debug_assert!(!dst.is_empty(), "q_snprintf needs room for the NUL");
    let cap = dst.len() - 1;
    let mut n = 0usize;
    for p in parts {
        let take = p.len().min(cap - n);
        dst[n..n + take].copy_from_slice(&p[..take]);
        n += take;
        if n == cap {
            break;
        }
    }
    dst[n] = 0;
}

/// Store a Host_Error message in the glue file's 256-byte buffer.
///
/// # Safety
/// `err` must point at 256 writable bytes.
unsafe fn fill_err(err: *mut c_char, msg: &str) -> c_int {
    let b = msg.as_bytes();
    let n = b.len().min(255);
    // SAFETY: caller guarantees a 256-byte buffer
    unsafe {
        core::ptr::copy_nonoverlapping(b.as_ptr(), err.cast::<u8>(), n);
        *err.add(n) = 0;
    }
    0
}

/// `Sys_Error ("<what>: funny lump size in %s", mod->name)`
///
/// # Safety
/// `m` must point at a live qmodel_t.
unsafe fn funny_lump_size(m: *const QModel, what: &CStr) -> ! {
    // SAFETY: Sys_Error is diverging; the format string and varargs match
    // the C call sites
    unsafe {
        sys::Sys_Error(
            c"%s: funny lump size in %s".as_ptr(),
            what.as_ptr(),
            model_name(m),
        )
    }
}

/*
===================
Mod_DecompressVis
===================
*/

static mut MOD_DECOMPRESSED: *mut u8 = null_mut();
static mut MOD_DECOMPRESSED_CAPACITY: c_int = 0;

/// # Safety
/// C ABI contract of Mod_DecompressVis: `in_` is NULL or a compressed PVS
/// row inside `model->visdata`, `model` a live qmodel_t.
#[no_mangle]
pub unsafe extern "C" fn Mod_DecompressVis(in_: *mut u8, model: *mut QModel) -> *mut u8 {
    // SAFETY: C ABI contract above; the cache statics are only touched from
    // the main thread, exactly like the C file-scope pair they replace.
    unsafe {
        let row = ((*model).numleafs + 31) / 8;
        if MOD_DECOMPRESSED.is_null() || row > MOD_DECOMPRESSED_CAPACITY {
            MOD_DECOMPRESSED_CAPACITY = row;
            MOD_DECOMPRESSED = sys::Mem_Realloc(
                MOD_DECOMPRESSED.cast::<c_void>(),
                MOD_DECOMPRESSED_CAPACITY as isize as usize,
            )
            .cast::<u8>();
            if MOD_DECOMPRESSED.is_null() {
                sys::Sys_Error(
                    c"Mod_DecompressVis: realloc() failed on %d bytes".as_ptr(),
                    MOD_DECOMPRESSED_CAPACITY,
                );
            }
        }
        let base = MOD_DECOMPRESSED;
        let mut out = base;
        let outend = base.wrapping_offset(row as isize);

        if in_.is_null() {
            // no vis info, so make all visible
            let mut left = row;
            while left != 0 {
                *out = 0xff;
                out = out.add(1);
                left -= 1;
            }
            return base;
        }

        let mut inp = in_;
        loop {
            if *inp != 0 {
                *out = *inp;
                out = out.add(1);
                inp = inp.add(1);
            } else {
                // COMPAT: the run length is clamped to the remaining row, but
                // the input itself is read unbounded, like the C
                let mut c = c_int::from(*inp.add(1));
                inp = inp.add(2);
                let left = row as isize - out.offset_from(base);
                if c_long::from(c) as isize > left {
                    c = left as c_int;
                }
                while c != 0 {
                    if out == outend {
                        if !(*model).viswarn {
                            (*model).viswarn = true;
                            sys::Con_Warning(
                                c"Mod_DecompressVis: output overrun on model \"%s\"\n".as_ptr(),
                                model_name(model),
                            );
                        }
                        return base;
                    }
                    *out = 0;
                    out = out.add(1);
                    c -= 1;
                }
            }
            if out.offset_from(base) >= row as isize {
                break;
            }
        }

        base
    }
}

/*
=================
Mod_ParseTextures
=================
*/

/// # Safety
/// C ABI contract of Mod_ParseTextures.
#[no_mangle]
pub unsafe extern "C" fn Mod_ParseTextures(
    m: *mut QModel,
    mod_base: *mut u8,
    l: *const LumpT,
    wads: *mut c_void,
) {
    // SAFETY: C ABI contract above
    unsafe {
        let lump = lump_slice(mod_base, l);
        let valve = (*m).bspversion == BSPVERSION_VALVE;
        let q64 = (*m).bspversion == BSPVERSION_QUAKE64;

        // johnfitz -- don't return early if no textures; still need to
        // create dummy texture
        if (*l).filelen == 0 {
            sys::Con_Printf(c"Mod_LoadTextures: no textures in bsp file\n".as_ptr());
        }
        let (nummiptex, work) = textures::parse_textures(lump, valve, q64);

        // johnfitz -- need 2 dummy texture chains for missing textures
        (*m).numtextures = nummiptex + 2;
        (*m).textures = sys::Mem_Alloc(alloc_size(
            (*m).numtextures,
            core::mem::size_of::<*mut Texture>(),
        ))
        .cast::<*mut Texture>();

        for (i, w) in work.iter().enumerate() {
            let rec = match w {
                TexWork::Skip => continue,
                TexWork::ZeroSized { name } => {
                    sys::Con_Warning(
                        c"Zero sized texture %s in %s!\n".as_ptr(),
                        cstring(name).as_ptr(),
                        model_name(m),
                    );
                    continue;
                }
                TexWork::Tex(rec) => rec,
            };
            let slot = (*m).textures.add(i);

            // an offset of zero indicates an external texture
            if rec.external_candidate {
                let tex = Mod_LoadWadTexture(m, wads, cstring(&rec.name).as_ptr());
                *slot = tex;
                if !tex.is_null() {
                    continue;
                }
            }

            let tx =
                sys::Mem_Alloc(SIZEOF_TEXTURE.wrapping_add(rec.alloc_pixels as isize as usize))
                    .cast::<Texture>();
            *slot = tx;

            core::ptr::copy_nonoverlapping(
                rec.name.as_ptr(),
                addr_of_mut!((*tx).name).cast::<u8>(),
                16,
            );
            (*tx).width = rec.width;
            (*tx).height = rec.height;
            (*tx).type_ = rec.textype;
            for j in 0..MIPLEVELS {
                // the pixels immediately follow the structures
                (*tx).offsets[j] = rec.offsets[j]
                    .wrapping_add(SIZEOF_TEXTURE as u32)
                    .wrapping_sub(SIZEOF_MIPTEX as u32);
            }

            // ericw -- pixels extending past the end of the lump appear in
            // the wild; e.g. jam2_tronyn.bsp, kellbase1.bsp
            if rec.truncated {
                sys::Con_DPrintf(
                    c"Texture %s extends past end of lump\n".as_ptr(),
                    cstring(&rec.name).as_ptr(),
                );
            }
            let mut source_file = [0u8; MAX_QPATH];
            strlcpy(&mut source_file, cstr_bytes(model_name(m)));
            core::ptr::copy_nonoverlapping(
                source_file.as_ptr(),
                addr_of_mut!((*tx).source_file).cast::<u8>(),
                MAX_QPATH,
            );
            (*tx).source_offset = ((*l).fileofs as isize as usize).wrapping_add(rec.pixels_ofs);

            (*tx).update_warp = 0; // johnfitz
            (*tx).warpimage = null_mut(); // johnfitz
            (*tx).fullbright = null_mut(); // johnfitz
            (*tx).shift = 0; // Q64 only
            (*tx).palette = rec.palette;

            if q64 {
                (*tx).shift = rec.shift as u32;
            }
            // COMPAT: the C Q64 copy starts 4 bytes past the clamp base and
            // can therefore read past the lump end; bounded here
            let copy_len =
                (rec.copy_len.max(0) as usize).min(lump.len().saturating_sub(rec.copy_ofs));
            if copy_len != 0 {
                core::ptr::copy_nonoverlapping(
                    lump.as_ptr().add(rec.copy_ofs),
                    tx.cast::<u8>().add(SIZEOF_TEXTURE),
                    copy_len,
                );
            }
        }
    }
}

/*
=================
Mod_LoadLighting -- johnfitz -- replaced with lit support code via lordhavoc
=================
*/

/// # Safety
/// C ABI contract of Mod_LoadLighting.
#[no_mangle]
pub unsafe extern "C" fn Mod_LoadLighting(m: *mut QModel, mod_base: *mut u8, l: *const LumpT) {
    // SAFETY: C ABI contract above
    unsafe {
        let lump = lump_slice(mod_base, l);
        let filelen = (*l).filelen;
        (*m).lightdata = null_mut();

        // LordHavoc: check for a .lit file
        let mut litfilename = [0u8; sys::MAX_OSPATH];
        strlcpy(&mut litfilename, cstr_bytes(model_name(m)));
        sys::COM_StripExtension(
            litfilename.as_ptr().cast::<c_char>(),
            litfilename.as_mut_ptr().cast::<c_char>(),
            sys::MAX_OSPATH,
        );
        strlcat(&mut litfilename, b".lit");

        let mut path_id: c_uint = 0;
        let data = sys::COM_LoadFile(litfilename.as_ptr().cast::<c_char>(), &mut path_id);
        if !data.is_null() {
            let com_filesize = sys::COM_ThreadFileSize();
            // use lit file only from the same gamedir as the map itself or
            // from a searchpath with higher priority.
            if path_id < (*m).path_id {
                sys::Con_DPrintf(
                    c"ignored %s from a gamedir with lower priority\n".as_ptr(),
                    litfilename.as_ptr(),
                );
            } else {
                // COMPAT: the C reads the 8-byte header unconditionally; a
                // shorter file is treated as corrupt here
                let body = core::slice::from_raw_parts(data, com_filesize.max(0) as usize);
                match lighting::check_lit(body, filelen, com_filesize) {
                    lighting::LitCheck::Ok => {
                        sys::Con_DPrintf2(c"%s loaded\n".as_ptr(), litfilename.as_ptr());
                        let n = filelen.wrapping_mul(3);
                        (*m).lightdata = sys::Mem_AllocNonZero(n as isize as usize).cast::<u8>();
                        core::ptr::copy_nonoverlapping(
                            data.add(8),
                            (*m).lightdata,
                            n.max(0) as usize,
                        );
                        sys::Mem_Free(data.cast::<c_void>());
                        return;
                    }
                    lighting::LitCheck::WrongSize { expected } => {
                        sys::Con_Printf(
                            c"Outdated .lit file (%s should be %u bytes, not %lld)\n".as_ptr(),
                            litfilename.as_ptr(),
                            expected,
                            com_filesize,
                        );
                    }
                    lighting::LitCheck::BadVersion(v) => {
                        sys::Con_Printf(c"Unknown .lit file version (%d)\n".as_ptr(), v);
                    }
                    lighting::LitCheck::NotQlit => {
                        sys::Con_Printf(c"Corrupt .lit file (old version?), ignoring\n".as_ptr());
                    }
                }
            }
            sys::Mem_Free(data.cast::<c_void>());
        }

        // LordHavoc: no .lit found, expand the white lighting data to color
        if filelen == 0 {
            return;
        }

        // Quake64 bsp lightmap data: RGB samples packed in 16 bits
        if (*m).bspversion == BSPVERSION_QUAKE64 {
            let out = sys::Mem_Alloc((filelen / 2).wrapping_mul(3) as isize as usize).cast::<u8>();
            (*m).lightdata = out;
            let expanded = lighting::expand_q64(lump);
            core::ptr::copy_nonoverlapping(expanded.as_ptr(), out, expanded.len());
            return;
        }

        if (*m).bspversion == BSPVERSION_VALVE {
            // lightmap samples are already stored as rgb
            let out = sys::Mem_Alloc(filelen as isize as usize).cast::<u8>();
            (*m).lightdata = out;
            core::ptr::copy_nonoverlapping(lump.as_ptr(), out, lump.len());
            return;
        }

        let out = sys::Mem_Alloc(filelen.wrapping_mul(3) as isize as usize).cast::<u8>();
        (*m).lightdata = out;
        let expanded = lighting::expand_white(lump);
        core::ptr::copy_nonoverlapping(expanded.as_ptr(), out, expanded.len());
    }
}

/*
=================
Mod_LoadVisibility
=================
*/

/// # Safety
/// C ABI contract of Mod_LoadVisibility.
#[no_mangle]
pub unsafe extern "C" fn Mod_LoadVisibility(m: *mut QModel, mod_base: *mut u8, l: *const LumpT) {
    // SAFETY: C ABI contract above
    unsafe {
        (*m).viswarn = false;
        if (*l).filelen == 0 {
            (*m).visdata = null_mut();
            return;
        }
        let lump = lump_slice(mod_base, l);
        let out = sys::Mem_Alloc((*l).filelen as isize as usize).cast::<u8>();
        (*m).visdata = out;
        core::ptr::copy_nonoverlapping(lump.as_ptr(), out, lump.len());
    }
}

/*
=================
Mod_LoadEntities
=================
*/

/// # Safety
/// C ABI contract of Mod_LoadEntities.
#[no_mangle]
pub unsafe extern "C" fn Mod_LoadEntities(m: *mut QModel, mod_base: *mut u8, l: *const LumpT) {
    // SAFETY: C ABI contract above
    unsafe {
        let lump = lump_slice(mod_base, l);
        let filelen = (*l).filelen;
        let mut ents: *mut c_char = null_mut();

        if sys::external_ents.value != 0.0 {
            let mut crc: c_uint = 0;
            if filelen > 0 {
                crc = c_uint::from(crc_block(&lump[..lump.len() - 1]));
            }

            let mut basemapname = [0u8; MAX_QPATH];
            strlcpy(&mut basemapname, cstr_bytes(model_name(m)));
            sys::COM_StripExtension(
                basemapname.as_ptr().cast::<c_char>(),
                basemapname.as_mut_ptr().cast::<c_char>(),
                MAX_QPATH,
            );
            let base = cstr_bytes(basemapname.as_ptr().cast::<c_char>());

            let mut entfilename = [0u8; MAX_QPATH];
            let mut versioned = true;
            let mut path_id: c_uint = 0;
            snprintf_parts(
                &mut entfilename,
                &[base, format!("@{crc:04x}.ent").as_bytes()],
            );
            sys::Con_DPrintf2(c"trying to load %s\n".as_ptr(), entfilename.as_ptr());
            ents = sys::COM_LoadFile(entfilename.as_ptr().cast::<c_char>(), &mut path_id)
                .cast::<c_char>();

            if ents.is_null() {
                snprintf_parts(&mut entfilename, &[base, b".ent"]);
                sys::Con_DPrintf2(c"trying to load %s\n".as_ptr(), entfilename.as_ptr());
                ents = sys::COM_LoadFile(entfilename.as_ptr().cast::<c_char>(), &mut path_id)
                    .cast::<c_char>();
                versioned = false;
            }

            if !ents.is_null() {
                // use ent file only from the same gamedir as the map itself
                // or from a searchpath with higher priority unless we got a
                // CRC match
                if !versioned && path_id < (*m).path_id {
                    sys::Con_DPrintf(
                        c"ignored %s from a gamedir with lower priority\n".as_ptr(),
                        entfilename.as_ptr(),
                    );
                } else {
                    (*m).entities = ents;
                    sys::Con_DPrintf(
                        c"Loaded external entity file %s\n".as_ptr(),
                        entfilename.as_ptr(),
                    );
                    return;
                }
            }
        }

        // _load_embedded
        if filelen == 0 {
            // COMPAT: the C leaks `ents` on this path
            sys::Mem_Free((*m).entities.cast::<c_void>());
            (*m).entities = null_mut();
            return;
        }
        // over-allocate + 1 byte with Mem_Alloc (which 0-initializes) so
        // COM_Parse always sees a terminated string
        let out = sys::Mem_Alloc(filelen.wrapping_add(1) as isize as usize).cast::<c_char>();
        (*m).entities = out;
        core::ptr::copy_nonoverlapping(lump.as_ptr(), out.cast::<u8>(), lump.len());
        sys::Mem_Free(ents.cast::<c_void>());
    }
}

/*
=================
Mod_LoadVertexes
=================
*/

/// # Safety
/// C ABI contract of Mod_LoadVertexes.
#[no_mangle]
pub unsafe extern "C" fn Mod_LoadVertexes(m: *mut QModel, mod_base: *mut u8, l: *const LumpT) {
    // SAFETY: C ABI contract above
    unsafe {
        let lump = lump_slice(mod_base, l);
        let Ok(recs) = lumps::parse_vertexes(lump) else {
            funny_lump_size(m, c"MOD_LoadBmodel");
        };
        let count = recs.len() as c_int;
        let out =
            sys::Mem_Alloc(alloc_size(count, core::mem::size_of::<MVertex>())).cast::<MVertex>();

        (*m).vertexes = out;
        (*m).numvertexes = count;

        for (i, r) in recs.iter().enumerate() {
            (*out.add(i)).position = *r;
        }
    }
}

/*
=================
Mod_LoadEdges
=================
*/

/// # Safety
/// C ABI contract of Mod_LoadEdges.
#[no_mangle]
pub unsafe extern "C" fn Mod_LoadEdges(
    m: *mut QModel,
    mod_base: *mut u8,
    l: *const LumpT,
    bsp2: c_int,
) {
    // SAFETY: C ABI contract above
    unsafe {
        let lump = lump_slice(mod_base, l);
        let Ok(recs) = lumps::parse_edges(lump, bsp2 != 0) else {
            funny_lump_size(m, c"MOD_LoadBmodel");
        };
        let count = recs.len() as c_int;
        let out = sys::Mem_Alloc(alloc_size(
            count.wrapping_add(1),
            core::mem::size_of::<MEdge>(),
        ))
        .cast::<MEdge>();

        (*m).edges = out;
        (*m).numedges = count;

        for (i, r) in recs.iter().enumerate() {
            (*out.add(i)).v = *r;
        }
    }
}

/*
=================
Mod_LoadTexinfo
=================
*/

/// # Safety
/// C ABI contract of Mod_LoadTexinfo.
#[no_mangle]
pub unsafe extern "C" fn Mod_LoadTexinfo(m: *mut QModel, mod_base: *mut u8, l: *const LumpT) {
    // SAFETY: C ABI contract above
    unsafe {
        let lump = lump_slice(mod_base, l);
        let Ok(recs) = lumps::parse_texinfo(lump) else {
            funny_lump_size(m, c"MOD_LoadBmodel");
        };
        let count = recs.len() as c_int;
        let out =
            sys::Mem_Alloc(alloc_size(count, core::mem::size_of::<MTexInfo>())).cast::<MTexInfo>();

        (*m).texinfo = out;
        (*m).numtexinfo = count;

        let numtextures = (*m).numtextures;
        let textures = (*m).textures;
        let mut missing = 0; // johnfitz

        for (i, r) in recs.iter().enumerate() {
            let o = out.add(i);
            (*o).vecs = r.vecs;

            // johnfitz -- rewrote this section
            let res = lumps::resolve_texinfo(r.miptex, r.flags, numtextures, |slot| {
                !(*textures.offset(slot as isize)).is_null()
            });
            (*o).texture = *textures.offset(res.texture_slot as isize);
            (*o).flags = res.flags;
            (*o).tex_idx = res.tex_idx;
            if res.missing {
                missing += 1;
            }
            // johnfitz
        }

        // johnfitz: report missing textures
        if missing != 0 && numtextures > 1 {
            sys::Con_Printf(
                c"Mod_LoadTexinfo: %d texture(s) missing from BSP file\n".as_ptr(),
                missing,
            );
        }
    }
}

/*
================
CalcSurfaceExtents

Fills in s->texturemins[] and s->extents[]
================
*/

/// # Safety
/// C ABI contract of CalcSurfaceExtents: `s` is one of `mod`'s surfaces,
/// with `texinfo`, `firstedge` and `numedges` already filled in.
#[no_mangle]
pub unsafe extern "C" fn CalcSurfaceExtents(m: *mut QModel, s: *mut MSurface) {
    // SAFETY: C ABI contract above; the surfedge/edge/vertex indices are
    // followed exactly like the C (unvalidated, as the BSP is trusted here)
    unsafe {
        let tex = (*s).texinfo;
        let vecs = (*tex).vecs;

        let mut points: Vec<[f32; 3]> = Vec::new();
        for i in 0..(*s).numedges {
            let e = *(*m)
                .surfedges
                .offset((*s).firstedge.wrapping_add(i) as isize);
            let v = if e >= 0 {
                (*(*m).edges.offset(e as isize)).v[0]
            } else {
                (*(*m).edges.offset(e.wrapping_neg() as isize)).v[1]
            };
            points.push((*(*m).vertexes.offset(v as isize)).position);
        }

        let special = (*tex).flags & TEX_SPECIAL != 0;
        match extents::calc_surface_extents(points.into_iter(), &vecs, special) {
            Ok(e) => {
                (*s).texturemins = e.texturemins;
                (*s).extents = e.extents;
            }
            // johnfitz -- was 512 in glquake, 256 in winquake
            Err(_) => sys::Sys_Error(c"Bad surface extents".as_ptr()),
        }
    }
}

/*
=================
Mod_ParseFaces
=================
*/

/// # Safety
/// C ABI contract of Mod_ParseFaces.
#[no_mangle]
pub unsafe extern "C" fn Mod_ParseFaces(
    m: *mut QModel,
    mod_base: *mut u8,
    l: *const LumpT,
    bsp2: bool,
) {
    // SAFETY: C ABI contract above
    unsafe {
        let lump = lump_slice(mod_base, l);
        let Ok(recs) = lumps::parse_faces(lump, bsp2) else {
            funny_lump_size(m, c"MOD_LoadBmodel");
        };
        let count = recs.len() as c_int;
        let out = sys::Mem_AllocNonZero(alloc_size(count, core::mem::size_of::<MSurface>()))
            .cast::<MSurface>();

        // johnfitz -- warn mappers about exceeding old limits
        if count > 32767 && !bsp2 {
            Con_DWarning(
                c"%i faces exceeds standard limit of 32767.\n".as_ptr(),
                count,
            );
        }

        (*m).surfaces = out;
        (*m).numsurfaces = count;

        let q64 = (*m).bspversion == BSPVERSION_QUAKE64;
        let valve = (*m).bspversion == BSPVERSION_VALVE;

        for (surfnum, r) in recs.iter().enumerate() {
            let o = out.add(surfnum);
            (*o).firstedge = r.firstedge;
            (*o).numedges = r.numedges;
            for style in &r.warned_styles {
                sys::Con_Warning(c"Invalid lightstyle %d\n".as_ptr(), c_int::from(*style));
            }
            (*o).styles = r.styles;
            (*o).styles_bitmap = r.styles_bitmap;

            (*o).flags = 0;
            (*o).polys = null_mut();

            if r.side != 0 {
                (*o).flags |= SURF_PLANEBACK;
            }

            (*o).plane = (*m).planes.wrapping_offset(r.planenum as isize);
            (*o).texinfo = (*m).texinfo.wrapping_offset(r.texinfo as isize);

            // lighting info
            (*o).samples = match lumps::face_samples_offset(r.lightofs, q64, valve) {
                None => null_mut(),
                // johnfitz -- lit support via lordhavoc (was "+ i")
                Some(ofs) => (*m).lightdata.wrapping_offset(ofs as isize),
            };

            // johnfitz -- this section rewritten
            (*o).lightmaptexturenum = -1;
            let cls = lumps::classify_face(
                (*(*(*o).texinfo).texture).type_,
                (*(*o).texinfo).flags,
                !(*o).samples.is_null(),
                (*o).styles[0],
            );
            (*o).flags |= cls.flags;
            if cls.warn_missing_samples {
                // unlit surf in a lit texture
                sys::Con_Warning(
                    c"Mod_LoadFaces: TEX_MISSING without TEX_SPECIAL missing lightmap samples"
                        .as_ptr(),
                );
            }
            (*o).lightmaptexturenum = cls.lightmaptexturenum;
            // johnfitz
        }
    }
}

/*
=================
Mod_LoadNodes
=================
*/

/// # Safety
/// C ABI contract of Mod_LoadNodes.
#[no_mangle]
pub unsafe extern "C" fn Mod_LoadNodes(
    m: *mut QModel,
    mod_base: *mut u8,
    l: *const LumpT,
    bsp2: c_int,
) {
    // SAFETY: C ABI contract above
    unsafe {
        let lump = lump_slice(mod_base, l);
        let dialect = dialect_of(bsp2);
        let numleafs = (*m).numleafs;
        let recs = match lumps::parse_nodes(lump, dialect, numleafs) {
            Ok(recs) => recs,
            Err(_) if dialect == Bsp2::No => funny_lump_size(m, c"MOD_LoadBmodel"),
            Err(_) => funny_lump_size(m, c"Mod_LoadNodes"),
        };
        let count = recs.len() as c_int;
        let out = sys::Mem_Alloc(alloc_size(count, core::mem::size_of::<MNode>())).cast::<MNode>();

        // johnfitz -- warn mappers about exceeding old limits
        if dialect == Bsp2::No && count > 32767 {
            Con_DWarning(
                c"%i nodes exceeds standard limit of 32767.\n".as_ptr(),
                count,
            );
        }

        (*m).nodes = out;
        (*m).numnodes = count;

        for (i, r) in recs.iter().enumerate() {
            let o = out.add(i);
            (*o).minmaxs = r.minmaxs;
            (*o).plane = (*m).planes.wrapping_offset(r.planenum as isize);
            // johnfitz -- explicit cast as unsigned short
            (*o).firstsurface = r.firstsurface;
            (*o).numsurfaces = r.numsurfaces;

            for j in 0..2 {
                // johnfitz -- hack to handle nodes > 32k, adapted from
                // darkplaces
                (*o).children[j] = match r.children[j] {
                    NodeChild::Node(p) => out.wrapping_offset(p as isize),
                    NodeChild::Leaf(p) => (*m).leafs.wrapping_offset(p as isize).cast::<MNode>(),
                    NodeChild::InvalidLeaf(p) => {
                        sys::Con_Printf(
                            c"Mod_LoadNodes: invalid leaf index %i (file has only %i leafs)\n"
                                .as_ptr(),
                            p,
                            numleafs,
                        );
                        // map it to the solid leaf
                        (*m).leafs.cast::<MNode>()
                    }
                };
                // johnfitz
            }
        }
    }
}

/*
=================
Mod_ProcessLeafs
=================
*/

/// Shared body of Mod_ProcessLeafs_{S,L1,L2}; returns 0 with `err` filled
/// where the C would have raised Host_Error.
///
/// # Safety
/// `m` must be a live qmodel_t and `err` a 256-byte buffer.
unsafe fn process_leafs(m: *mut QModel, buf: &[u8], dialect: Bsp2, err: *mut c_char) -> c_int {
    // SAFETY: caller contract above
    unsafe {
        let recs = match lumps::parse_leafs(buf, dialect) {
            Ok(recs) => recs,
            Err(LeafError::FunnySize) => funny_lump_size(m, c"Mod_ProcessLeafs"),
            Err(LeafError::TooMany(count)) => {
                // johnfitz -- the C allocates before the limit check and
                // leaks the block; mod->leafs/numleafs stay unwritten
                sys::Mem_Alloc(alloc_size(count, core::mem::size_of::<MLeaf>()));
                return fill_err(
                    err,
                    &format!("Mod_LoadLeafs: {count} leafs exceeds limit of 32767."),
                );
            }
        };
        let count = recs.len() as c_int;
        let out = sys::Mem_Alloc(alloc_size(count, core::mem::size_of::<MLeaf>())).cast::<MLeaf>();

        (*m).leafs = out;
        (*m).numleafs = count;

        for (i, r) in recs.iter().enumerate() {
            let o = out.add(i);
            (*o).minmaxs = r.minmaxs;
            (*o).contents = r.contents;
            // johnfitz -- unsigned short
            (*o).firstmarksurface = (*m)
                .marksurfaces
                .wrapping_offset(r.firstmarksurface as isize);
            (*o).nummarksurfaces = r.nummarksurfaces;

            (*o).compressed_vis = if r.visofs == -1 {
                null_mut()
            } else if dialect == Bsp2::No {
                // COMPAT: only the BSP29 path NULL-checks mod->visdata
                if (*m).visdata.is_null() {
                    null_mut()
                } else {
                    (*m).visdata.wrapping_offset(r.visofs as isize)
                }
            } else {
                (*m).visdata.wrapping_offset(r.visofs as isize)
            };
            (*o).efrags = null_mut();
            (*o).ambient_sound_level = r.ambient;

            // johnfitz -- removed code to mark surfaces as SURF_UNDERWATER
        }
        1
    }
}

/// Mod_LoadLeafs; 1 = ok, 0 = the C would have raised Host_Error (`err`).
///
/// # Safety
/// C ABI contract of Mod_LoadLeafs plus a 256-byte `err` buffer.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_mod_load_leafs(
    m: *mut QModel,
    mod_base: *mut u8,
    l: *const LumpT,
    bsp2: c_int,
    err: *mut c_char,
) -> c_int {
    // SAFETY: C ABI contract above
    unsafe {
        let lump = lump_slice(mod_base, l);
        process_leafs(m, lump, dialect_of(bsp2), err)
    }
}

/*
=================
Mod_LoadClipnodes
=================
*/

/// Mod_LoadClipnodes; 1 = ok, 0 = the C would have raised Host_Error.
///
/// # Safety
/// C ABI contract of Mod_LoadClipnodes plus a 256-byte `err` buffer.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_mod_load_clipnodes(
    m: *mut QModel,
    mod_base: *mut u8,
    l: *const LumpT,
    bsp2: bool,
    err: *mut c_char,
) -> c_int {
    // SAFETY: C ABI contract above
    unsafe {
        let lump = lump_slice(mod_base, l);
        let Ok(recs) = lumps::parse_clipnodes(lump, bsp2) else {
            funny_lump_size(m, c"Mod_LoadClipnodes");
        };
        let count = recs.len() as c_int;
        let out = sys::Mem_Alloc(alloc_size(count, core::mem::size_of::<MClipnode>()))
            .cast::<MClipnode>();

        // johnfitz -- warn about exceeding old limits
        if count > 32767 && !bsp2 {
            Con_DWarning(
                c"%i clipnodes exceeds standard limit of 32767.\n".as_ptr(),
                count,
            );
        }

        (*m).clipnodes = out;
        (*m).numclipnodes = count;

        let planes = (*m).planes;
        for (h, mins, maxs) in [
            (1usize, [-16.0f32, -16.0, -24.0], [16.0f32, 16.0, 32.0]),
            (2usize, [-32.0f32, -32.0, -24.0], [32.0f32, 32.0, 64.0]),
        ] {
            let hull = addr_of_mut!((*m).hulls[h]);
            (*hull).clipnodes = out;
            (*hull).firstclipnode = 0;
            (*hull).lastclipnode = count - 1;
            (*hull).planes = planes;
            (*hull).clip_mins = mins;
            (*hull).clip_maxs = maxs;
        }

        let numplanes = (*m).numplanes;
        for (i, r) in recs.iter().enumerate() {
            let o = out.add(i);
            (*o).planenum = r.planenum;

            // johnfitz -- bounds check
            if r.planenum < 0 || r.planenum >= numplanes {
                return fill_err(err, "Mod_LoadClipnodes: planenum out of bounds");
            }
            // johnfitz

            // johnfitz -- support clipnodes > 32k
            (*o).children = r.children;
        }
        1
    }
}

/*
=================
Mod_MakeHull0

Duplicate the drawing hull structure as a clipping hull
=================
*/

/// # Safety
/// C ABI contract of Mod_MakeHull0: `mod`'s nodes and planes are loaded.
#[no_mangle]
pub unsafe extern "C" fn Mod_MakeHull0(m: *mut QModel) {
    // SAFETY: C ABI contract above
    unsafe {
        let nodes = (*m).nodes;
        let count = (*m).numnodes;
        let out = sys::Mem_Alloc(alloc_size(count, core::mem::size_of::<MClipnode>()))
            .cast::<MClipnode>();

        let hull = addr_of_mut!((*m).hulls[0]);
        (*hull).clipnodes = out;
        (*hull).firstclipnode = 0;
        (*hull).lastclipnode = count - 1;
        (*hull).planes = (*m).planes;

        for i in 0..count as isize {
            let in_ = nodes.offset(i);
            let o = out.offset(i);
            (*o).planenum = (*in_).plane.offset_from((*m).planes) as c_int;
            for j in 0..2 {
                let child = (*in_).children[j];
                (*o).children[j] = if (*child).contents < 0 {
                    (*child).contents
                } else {
                    child.offset_from(nodes) as c_int
                };
            }
        }
    }
}

/*
=================
Mod_LoadMarksurfaces
=================
*/

/// Mod_LoadMarksurfaces; 1 = ok, 0 = the C would have raised Host_Error.
///
/// # Safety
/// C ABI contract of Mod_LoadMarksurfaces plus a 256-byte `err` buffer.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_mod_load_marksurfaces(
    m: *mut QModel,
    mod_base: *mut u8,
    l: *const LumpT,
    bsp2: c_int,
    err: *mut c_char,
) -> c_int {
    // SAFETY: C ABI contract above
    unsafe {
        let lump = lump_slice(mod_base, l);
        let bsp2 = bsp2 != 0;
        let res = lumps::parse_marksurfaces(lump, bsp2, (*m).numsurfaces);
        if res.error == Some(MarkError::FunnySize) {
            let name = cstring(cstr_bytes(model_name(m)));
            return fill_err(
                err,
                &format!(
                    "Mod_LoadMarksurfaces: funny lump size in {}",
                    name.to_string_lossy()
                ),
            );
        }

        let rec_size = if bsp2 { 4 } else { 2 };
        let count = (lump.len() / rec_size) as c_int;
        let out = sys::Mem_Alloc(alloc_size(count, core::mem::size_of::<c_int>())).cast::<c_int>();

        (*m).marksurfaces = out;
        (*m).nummarksurfaces = count;

        // johnfitz -- warn mappers about exceeding old limits
        if !bsp2 && count > 32767 {
            Con_DWarning(
                c"%i marksurfaces exceeds standard limit of 32767.\n".as_ptr(),
                count,
            );
        }

        for (i, j) in res.entries.iter().enumerate() {
            *out.add(i) = *j;
        }

        if res.error == Some(MarkError::BadSurface) {
            if bsp2 {
                return fill_err(err, "Mod_LoadMarksurfaces: bad surface number");
            }
            sys::Sys_Error(c"Mod_LoadMarksurfaces: bad surface number".as_ptr());
        }
        1
    }
}

/*
=================
Mod_LoadSurfedges
=================
*/

/// # Safety
/// C ABI contract of Mod_LoadSurfedges.
#[no_mangle]
pub unsafe extern "C" fn Mod_LoadSurfedges(m: *mut QModel, mod_base: *mut u8, l: *const LumpT) {
    // SAFETY: C ABI contract above
    unsafe {
        let lump = lump_slice(mod_base, l);
        let Ok(recs) = lumps::parse_surfedges(lump) else {
            funny_lump_size(m, c"MOD_LoadBmodel");
        };
        let count = recs.len() as c_int;
        let out = sys::Mem_Alloc(alloc_size(count, core::mem::size_of::<c_int>())).cast::<c_int>();

        (*m).surfedges = out;
        (*m).numsurfedges = count;

        for (i, r) in recs.iter().enumerate() {
            *out.add(i) = *r;
        }
    }
}

/*
=================
Mod_LoadPlanes
=================
*/

/// # Safety
/// C ABI contract of Mod_LoadPlanes.
#[no_mangle]
pub unsafe extern "C" fn Mod_LoadPlanes(m: *mut QModel, mod_base: *mut u8, l: *const LumpT) {
    // SAFETY: C ABI contract above
    unsafe {
        let lump = lump_slice(mod_base, l);
        let Ok(recs) = lumps::parse_planes(lump) else {
            funny_lump_size(m, c"MOD_LoadBmodel");
        };
        let count = recs.len() as c_int;
        let out = sys::Mem_Alloc(alloc_size(
            count.wrapping_mul(2),
            core::mem::size_of::<MPlane>(),
        ))
        .cast::<MPlane>();

        (*m).planes = out;
        (*m).numplanes = count;

        for (i, r) in recs.iter().enumerate() {
            let o = out.add(i);
            (*o).normal = r.normal;
            (*o).dist = r.dist;
            (*o).type_ = r.type_;
            (*o).signbits = r.signbits;
        }
    }
}

/*
=================
Mod_LoadSubmodels
=================
*/

/// # Safety
/// C ABI contract of Mod_LoadSubmodels.
#[no_mangle]
pub unsafe extern "C" fn Mod_LoadSubmodels(m: *mut QModel, mod_base: *mut u8, l: *const LumpT) {
    // SAFETY: C ABI contract above
    unsafe {
        let lump = lump_slice(mod_base, l);
        let Ok(recs) = lumps::parse_submodels(lump) else {
            funny_lump_size(m, c"MOD_LoadBmodel");
        };
        let count = recs.len() as c_int;
        let out =
            sys::Mem_Alloc(alloc_size(count, core::mem::size_of::<DModel>())).cast::<DModel>();

        (*m).submodels = out;
        (*m).numsubmodels = count;

        for (i, r) in recs.iter().enumerate() {
            let o = out.add(i);
            // spread the mins / maxs by a pixel (applied by the parser)
            (*o).mins = r.mins;
            (*o).maxs = r.maxs;
            (*o).origin = r.origin;
            (*o).headnode = r.headnode;
            (*o).visleafs = r.visleafs;
            (*o).firstface = r.firstface;
            (*o).numfaces = r.numfaces;
        }

        // johnfitz -- check world visleafs -- adapted from bjp
        // COMPAT: the C reads submodels[0] even with no submodels
        if let Some(first) = recs.first() {
            if first.visleafs > 8192 {
                Con_DWarning(
                    c"%i visleafs exceeds standard limit of 8192.\n".as_ptr(),
                    first.visleafs,
                );
            }
        }
    }
}

/*
================
Mod_CalcSpecialsAndTextures
================
*/

/// # Safety
/// `model`'s surfaces and texinfo must be loaded.
unsafe fn texture_index_for_surface(model: *mut QModel, surf: *mut MSurface) -> c_int {
    // SAFETY: caller contract above
    unsafe {
        let texinfo = (*surf).texinfo;
        let texture = (*texinfo).texture;
        let tex_idx = (*texinfo).tex_idx;

        if tex_idx >= 0
            && tex_idx < (*model).numtextures
            && *(*model).textures.offset(tex_idx as isize) == texture
        {
            return tex_idx;
        }

        for i in 0..(*model).numtextures {
            if *(*model).textures.offset(i as isize) == texture {
                return i;
            }
        }

        -1
    }
}

/// # Safety
/// `model` must be a live qmodel_t and `err` a 256-byte buffer.
unsafe fn calc_specials_and_textures(model: *mut QModel, err: *mut c_char) -> c_int {
    // SAFETY: caller contract above
    unsafe {
        let is_submodel = (*model).name[0] == b'*' as c_char;

        (*model).used_specials = 0;

        let mut used_tex = vec![0u8; (*model).numtextures.max(0) as usize];

        for i in 0..(*model).nummodelsurfaces {
            let psurf = (*model)
                .surfaces
                .wrapping_offset((*model).firstmodelsurface as isize)
                .wrapping_offset(i as isize);
            (*model).used_specials |= (SURF_DRAWSKY
                | SURF_DRAWTURB
                | SURF_DRAWWATER
                | SURF_DRAWLAVA
                | SURF_DRAWSLIME
                | SURF_DRAWTELE)
                & (*psurf).flags;

            let tex_idx = (*(*psurf).texinfo).tex_idx;
            if is_submodel && tex_idx >= 0 {
                if tex_idx < (*model).numtextures {
                    used_tex[tex_idx as usize] = 1;
                } else {
                    let name = cstring(cstr_bytes(model_name(model)));
                    return fill_err(
                        err,
                        &format!(
                            "Mod_CalcSpecialsAndTextures: {} invalid tex_idx {tex_idx}",
                            name.to_string_lossy()
                        ),
                    );
                }
            }
        }

        if is_submodel {
            let mut total = 0;
            for u in &used_tex {
                if *u != 0 {
                    total += 1;
                }
            }

            let orig_textures = (*model).textures;
            (*model).textures =
                sys::Mem_AllocNonZero(alloc_size(total, core::mem::size_of::<*mut Texture>()))
                    .cast::<*mut Texture>();
            (*model).numtextures = total;

            let mut placed = 0;
            let mut i = 0;
            while placed < total {
                if used_tex[i as usize] != 0 {
                    *(*model).textures.offset(placed as isize) = *orig_textures.offset(i as isize);
                    placed += 1;
                }
                i += 1;
            }
        }

        used_tex.fill(0);
        let mut tex_counts = [0i32; TEXTYPE_COUNT];
        let mut tex_offsets = [0i32; TEXTYPE_COUNT];

        for i in 0..(*model).nummodelsurfaces {
            let psurf = (*model)
                .surfaces
                .wrapping_offset((*model).firstmodelsurface as isize)
                .wrapping_offset(i as isize);
            let tex_index = texture_index_for_surface(model, psurf);
            if tex_index >= 0 {
                used_tex[tex_index as usize] = 1;
            }
        }

        for i in 0..(*model).numtextures {
            let texture = *(*model).textures.offset(i as isize);
            if !texture.is_null() && used_tex[i as usize] != 0 {
                tex_counts[(*texture).type_ as usize] += 1;
            }
        }

        let mut total = 0;
        for i in 0..TEXTYPE_COUNT {
            tex_offsets[i] = total;
            (*model).texofs[i] = total;
            total += tex_counts[i];
        }
        (*model).texofs[TEXTYPE_COUNT] = total;

        (*model).usedtextures = if total != 0 {
            sys::Mem_Alloc(alloc_size(total, core::mem::size_of::<c_int>())).cast::<c_int>()
        } else {
            null_mut()
        };
        for i in 0..(*model).numtextures {
            let texture = *(*model).textures.offset(i as isize);
            if !texture.is_null() && used_tex[i as usize] != 0 {
                let t = (*texture).type_ as usize;
                *(*model).usedtextures.offset(tex_offsets[t] as isize) = i;
                tex_offsets[t] += 1;
            }
        }

        1
    }
}

/*
=================
Mod_SetupSubmodels
set up the submodels (FIXME: this is confusing)
=================
*/

/// Mod_SetupSubmodels; 1 = ok, 0 = the C would have raised Host_Error.
/// `sv_modelname` is `sv.modelname`, passed in by the glue so the shim
/// never touches the server globals.
///
/// # Safety
/// C ABI contract of Mod_SetupSubmodels plus a NUL-terminated
/// `sv_modelname` and a 256-byte `err` buffer.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_mod_setup_submodels(
    m: *mut QModel,
    sv_modelname: *const c_char,
    err: *mut c_char,
) -> c_int {
    // SAFETY: C ABI contract above
    unsafe {
        let mut m = m;
        let orig_textures = (*m).textures;
        let orig_numtextures = (*m).numtextures;

        // johnfitz -- we're looping through the submodels starting at 0.
        // Submodel 0 is the main model, so we don't have to worry about
        // clobbering data the first time through, since it's the same data.
        // At the end of the loop, we create a new copy of the data to use
        // the next time through.
        for i in 0..(*m).numsubmodels {
            let bm = (*m).submodels.offset(i as isize);

            (*m).hulls[0].firstclipnode = (*bm).headnode[0];
            for j in 1..MAX_MAP_HULLS {
                (*m).hulls[j].firstclipnode = (*bm).headnode[j];
                (*m).hulls[j].lastclipnode = (*m).numclipnodes - 1;
            }

            (*m).firstmodelsurface = (*bm).firstface;
            (*m).nummodelsurfaces = (*bm).numfaces;

            (*m).maxs = (*bm).maxs;
            (*m).mins = (*bm).mins;

            // johnfitz -- calculate rotate bounds and yaw bounds
            let radius = lumps::radius_from_bounds(&(*m).mins, &(*m).maxs);
            (*m).rmaxs = [radius; 3];
            (*m).ymaxs = [radius; 3];
            (*m).rmins = [-radius; 3];
            (*m).ymins = [-radius; 3];
            // johnfitz

            // johnfitz -- correct physics cullboxes so that outlying clip
            // brushes on doors and stuff are handled right; skip submodel 0
            // of sv.worldmodel, which is the actual world
            if i > 0 || cstr_bytes(model_name(m)) != cstr_bytes(sv_modelname) {
                // start with the hull0 bounds
                (*m).clipmaxs = (*m).maxs;
                (*m).clipmins = (*m).mins;
            }
            // johnfitz

            (*m).numleafs = (*bm).visleafs;

            (*m).textures = orig_textures;
            (*m).numtextures = orig_numtextures;
            if calc_specials_and_textures(m, err) == 0 {
                return 0;
            }

            if i < (*m).numsubmodels - 1 {
                // duplicate the basic information
                let mut name = [0u8; 12];
                snprintf_into(&mut name, &format!("*{}", i + 1));
                let submodel = Mod_FindName(name.as_ptr().cast::<c_char>());
                core::ptr::copy(m.cast_const(), submodel, 1);
                let n = cstr_bytes(name.as_ptr().cast::<c_char>()).len();
                core::ptr::copy_nonoverlapping(
                    name.as_ptr(),
                    addr_of_mut!((*submodel).name).cast::<u8>(),
                    n + 1,
                );
                // Need to NULL this otherwise we double delete in
                // PScript_ClearSurfaceParticles
                (*submodel).skytrimem = null_mut();
                m = submodel;
            }
        }
        1
    }
}

/* EXTERNAL VIS FILE SUPPORT:
 */

/// # Safety
/// C ABI contract of Mod_FindVisibilityExternal: NUL-terminated
/// `loadname`, live `mod`.
#[no_mangle]
pub unsafe extern "C" fn Mod_FindVisibilityExternal(
    m: *mut QModel,
    loadname: *const c_char,
) -> *mut sys::FILE {
    // SAFETY: C ABI contract above
    unsafe {
        let mut visfilename = [0u8; MAX_QPATH];
        snprintf_parts(&mut visfilename, &[b"maps/", cstr_bytes(loadname), b".vis"]);

        let mut f: *mut sys::FILE = null_mut();
        let mut path_id: c_uint = 0;
        if sys::COM_FOpenFile(visfilename.as_ptr().cast::<c_char>(), &mut f, &mut path_id) < 0 {
            sys::Con_DPrintf(c"%s not found, trying ".as_ptr(), visfilename.as_ptr());
            let gamedir = sys::COM_SkipPath(addr_of!(sys::com_gamedir).cast::<c_char>());
            snprintf_parts(&mut visfilename, &[cstr_bytes(gamedir), b".vis"]);
            sys::Con_DPrintf(c"%s\n".as_ptr(), visfilename.as_ptr());
            if sys::COM_FOpenFile(visfilename.as_ptr().cast::<c_char>(), &mut f, &mut path_id) < 0 {
                sys::Con_DPrintf(c"external vis not found\n".as_ptr());
                return null_mut();
            }
        }
        if path_id < (*m).path_id {
            fclose(f);
            sys::Con_DPrintf(
                c"ignored %s from a gamedir with lower priority\n".as_ptr(),
                visfilename.as_ptr(),
            );
            return null_mut();
        }

        sys::Con_DPrintf(c"Found external VIS %s\n".as_ptr(), visfilename.as_ptr());

        let shortname = sys::COM_SkipPath(model_name(m));
        let mut pos: c_long = 0;
        let mut header = [0u8; VISPATCH_HEADER_LEN];
        let mut found = false;
        loop {
            let r = fread(
                header.as_mut_ptr().cast::<c_void>(),
                1,
                VISPATCH_HEADER_LEN,
                f,
            );
            if r != VISPATCH_HEADER_LEN {
                break;
            }
            let filelen = i32::from_le_bytes([header[32], header[33], header[34], header[35]]);
            if filelen <= 0 {
                // bad entry -- don't trust the rest.
                fclose(f);
                return null_mut();
            }
            // COMPAT: q_strcasecmp is ASCII-only here, and the mapname
            // scan is bounded to the 32-byte header field
            if cstring(&header[..32])
                .as_bytes()
                .eq_ignore_ascii_case(cstr_bytes(shortname))
            {
                found = true;
                break;
            }
            pos = pos
                .wrapping_add(filelen as c_long)
                .wrapping_add(VISPATCH_HEADER_LEN as c_long);
            sys::Sys_fseek(f, sys::qfileofs_t::from(pos), SEEK_SET);
        }
        if !found {
            fclose(f);
            sys::Con_DPrintf(
                c"%s not found in %s\n".as_ptr(),
                shortname,
                visfilename.as_ptr(),
            );
            return null_mut();
        }

        f
    }
}

/// # Safety
/// C ABI contract of Mod_LoadVisibilityExternal: `f` positioned at a
/// vispatch payload.
#[no_mangle]
pub unsafe extern "C" fn Mod_LoadVisibilityExternal(f: *mut sys::FILE) -> *mut u8 {
    // SAFETY: C ABI contract above
    unsafe {
        let mut raw = [0u8; 4];
        if fread(raw.as_mut_ptr().cast::<c_void>(), 1, 4, f) != 4 {
            return null_mut();
        }
        let filelen = i32::from_le_bytes(raw);
        if filelen <= 0 {
            return null_mut();
        }
        sys::Con_DPrintf(c"...%d bytes visibility data\n".as_ptr(), filelen);
        let visdata = sys::Mem_Alloc(filelen as isize as usize).cast::<u8>();
        if fread(visdata.cast::<c_void>(), filelen as usize, 1, f) != 1 {
            return null_mut();
        }
        visdata
    }
}

/// Mod_LoadLeafsExternal; 1 = ok, 0 = the C would have raised Host_Error.
///
/// # Safety
/// C ABI contract of Mod_LoadLeafsExternal plus a 256-byte `err` buffer.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_mod_load_leafs_external(
    m: *mut QModel,
    f: *mut sys::FILE,
    err: *mut c_char,
) -> c_int {
    // SAFETY: C ABI contract above
    unsafe {
        let mut raw = [0u8; 4];
        if fread(raw.as_mut_ptr().cast::<c_void>(), 1, 4, f) != 4 {
            sys::Sys_Error(c"Invalid leaf".as_ptr());
        }
        let filelen = i32::from_le_bytes(raw);
        if filelen <= 0 {
            return 1;
        }
        sys::Con_DPrintf(c"...%d bytes leaf data\n".as_ptr(), filelen);
        let in_ = sys::Mem_Alloc(filelen as isize as usize).cast::<u8>();
        if fread(in_.cast::<c_void>(), filelen as usize, 1, f) != 1 {
            return 1;
        }
        let buf = core::slice::from_raw_parts(in_, filelen as usize);
        process_leafs(m, buf, Bsp2::No, err)
    }
}
