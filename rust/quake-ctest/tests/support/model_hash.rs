//! Canonical deep-walk snapshot of a loaded `qmodel_t` (Phase 3 M3, D6).
//!
//! The C and Rust loaders fill two different `Mem_Alloc` heaps, so the raw
//! pointer values necessarily differ; everything else must be identical.
//! This walks the parse-produced graph and emits one `key = value` line per
//! observable datum, resolving every pointer to an index into the array it
//! points at (`#3`), an offset into `visdata`/`lightdata` (`visdata+12`), or
//! `null`. Comparing two snapshots line-by-line gives a readable diff instead
//! of a hash mismatch.
//!
//! Renderer-owned fields (gltexture/warpimage/fullbright, polys, texturechains,
//! lightmaptexturenum, vbo_firstvert, indirect_idx, the Vk tail, ...) are
//! excluded: parsing never writes them, and they are not part of the
//! compatibility surface this milestone ports.

#![allow(dead_code)]

use core::ffi::c_char;

use quake_types::model_mem::{
    AliasHdr, MAliasFrameDesc, MClipnode, MEdge, MLeaf, MNode, MSprite, MSpriteFrame,
    MSpriteFrameDesc, MSpriteGroup, MSurface, MTexInfo, MTriangle, QModel, Texture, PV_QUAKE1,
};
use quake_types::modelgen::{StVert, TriVertX};
use quake_types::MPlane;

/// Byte lengths of the two blobs `qmodel_t` owns but does not record a size
/// for; the fixture knows them (visibility lump length, expanded lighting).
#[derive(Clone, Copy, Default)]
pub struct BlobLens {
    pub visdata: usize,
    pub lightdata: usize,
}

pub struct Snapshot {
    pub lines: Vec<String>,
}

impl Snapshot {
    /// Line-by-line comparison; panics with the first differing pair (plus a
    /// little context) rather than dumping two multi-thousand-line vectors.
    pub fn assert_eq(&self, other: &Snapshot, what: &str) {
        for (i, (a, b)) in self.lines.iter().zip(other.lines.iter()).enumerate() {
            assert_eq!(
                a,
                b,
                "{what}: snapshot line {i} differs\n  C    : {a}\n  Rust : {b}\n  context: {:?}",
                &self.lines[i.saturating_sub(3)..i]
            );
        }
        assert_eq!(
            self.lines.len(),
            other.lines.len(),
            "{what}: snapshot length differs (C {} vs Rust {}); first extra: {:?}",
            self.lines.len(),
            other.lines.len(),
            self.lines
                .get(other.lines.len())
                .or_else(|| other.lines.get(self.lines.len()))
        );
    }
}

struct Walker {
    lines: Vec<String>,
}

/// Resolves `p` to `#i` when it lands inside the `n`-element array at `base`.
fn slot<T>(p: *mut T, base: *mut T, n: i32) -> String {
    if p.is_null() {
        return "null".to_string();
    }
    if base.is_null() || n <= 0 {
        return "extern".to_string();
    }
    // SAFETY: pointer arithmetic on the same allocation; out-of-range values
    // fall through to the "extern" label without being dereferenced.
    let off = (p as isize - base as isize) / core::mem::size_of::<T>() as isize;
    let rem = (p as isize - base as isize) % core::mem::size_of::<T>() as isize;
    if rem == 0 && off >= 0 && off < n as isize {
        format!("#{off}")
    } else {
        "extern".to_string()
    }
}

/// Resolves `p` to `<name>+<byte offset>` inside a flat blob.
fn blob(p: *mut u8, base: *mut u8, len: usize, name: &str) -> String {
    if p.is_null() {
        return "null".to_string();
    }
    if base.is_null() {
        return "extern".to_string();
    }
    let off = p as isize - base as isize;
    if off >= 0 && (off as usize) <= len {
        format!("{name}+{off}")
    } else {
        "extern".to_string()
    }
}

fn cstr16(s: &[c_char]) -> String {
    let bytes: Vec<u8> = s.iter().map(|&c| c as u8).collect();
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    format!("{:?}", String::from_utf8_lossy(&bytes[..end]))
}

impl Walker {
    fn put(&mut self, key: &str, value: impl std::fmt::Display) {
        self.lines.push(format!("{key} = {value}"));
    }
}

/// # Safety
/// `m` must point at a `qmodel_t` filled by one of the brush loaders, whose
/// arrays are all still allocated. `lens` must describe the real byte lengths
/// of `visdata`/`lightdata`.
pub unsafe fn snapshot(m: *const QModel, lens: BlobLens) -> Snapshot {
    let mut w = Walker { lines: Vec::new() };
    // SAFETY: caller's contract
    unsafe {
        let md = &*m;

        w.put("model.type", md.type_);
        w.put("model.flags", md.flags);
        w.put("model.bspversion", md.bspversion);
        w.put("model.numframes", md.numframes);
        w.put("model.mins", format!("{:?}", md.mins));
        w.put("model.maxs", format!("{:?}", md.maxs));
        w.put("model.ymins", format!("{:?}", md.ymins));
        w.put("model.ymaxs", format!("{:?}", md.ymaxs));
        w.put("model.rmins", format!("{:?}", md.rmins));
        w.put("model.rmaxs", format!("{:?}", md.rmaxs));
        w.put("model.clipbox", md.clipbox);
        w.put("model.clipmins", format!("{:?}", md.clipmins));
        w.put("model.clipmaxs", format!("{:?}", md.clipmaxs));
        w.put("model.firstmodelsurface", md.firstmodelsurface);
        w.put("model.nummodelsurfaces", md.nummodelsurfaces);
        w.put("model.viswarn", md.viswarn);
        w.put("model.bogus_tree", md.bogus_tree);
        w.put("model.contentstransparent", md.contentstransparent);
        w.put("model.used_specials", md.used_specials);
        w.put("model.texofs", format!("{:?}", md.texofs));

        vertexes(&mut w, md);
        edges(&mut w, md);
        surfedges(&mut w, md);
        marksurfaces(&mut w, md);
        planes(&mut w, md);
        textures(&mut w, md);
        texinfo(&mut w, md);
        surfaces(&mut w, md, lens);
        leafs(&mut w, md, lens);
        nodes(&mut w, md);
        clipnodes(&mut w, md);
        hulls(&mut w, md);
        submodels(&mut w, md);
        used_textures(&mut w, md);
        blobs(&mut w, md, lens);
    }
    Snapshot { lines: w.lines }
}

unsafe fn vertexes(w: &mut Walker, md: &QModel) {
    w.put("numvertexes", md.numvertexes);
    for i in 0..md.numvertexes.max(0) {
        // SAFETY: i < numvertexes, array allocated by the loader
        let v = unsafe { &*md.vertexes.offset(i as isize) };
        w.put(
            &format!("vertex[{i}].position"),
            format!("{:?}", v.position),
        );
    }
}

unsafe fn edges(w: &mut Walker, md: &QModel) {
    w.put("numedges", md.numedges);
    for i in 0..md.numedges.max(0) {
        // SAFETY: i < numedges
        let e: &MEdge = unsafe { &*md.edges.offset(i as isize) };
        w.put(&format!("edge[{i}].v"), format!("{:?}", e.v));
        w.put(&format!("edge[{i}].cachededgeoffset"), e.cachededgeoffset);
    }
}

unsafe fn surfedges(w: &mut Walker, md: &QModel) {
    w.put("numsurfedges", md.numsurfedges);
    for i in 0..md.numsurfedges.max(0) {
        // SAFETY: i < numsurfedges
        w.put(&format!("surfedge[{i}]"), unsafe {
            *md.surfedges.offset(i as isize)
        });
    }
}

unsafe fn marksurfaces(w: &mut Walker, md: &QModel) {
    w.put("nummarksurfaces", md.nummarksurfaces);
    for i in 0..md.nummarksurfaces.max(0) {
        // SAFETY: i < nummarksurfaces
        w.put(&format!("marksurface[{i}]"), unsafe {
            *md.marksurfaces.offset(i as isize)
        });
    }
}

unsafe fn planes(w: &mut Walker, md: &QModel) {
    w.put("numplanes", md.numplanes);
    for i in 0..md.numplanes.max(0) {
        // SAFETY: i < numplanes
        let p: &MPlane = unsafe { &*md.planes.offset(i as isize) };
        w.put(&format!("plane[{i}].normal"), format!("{:?}", p.normal));
        w.put(&format!("plane[{i}].dist"), format!("{:?}", p.dist));
        w.put(&format!("plane[{i}].type"), p.type_);
        w.put(&format!("plane[{i}].signbits"), p.signbits);
    }
}

/// C: `pixels = width * height / 64 * 85`, plus the Valve palette that
/// follows it (a `unsigned short` count then `count * 3` bytes).
unsafe fn texture_payload_len(tx: &Texture) -> usize {
    let mip = (tx.width as usize) * (tx.height as usize) / 64 * 85;
    if !tx.palette {
        return mip;
    }
    // SAFETY: the mip chain was copied into the tail of the same allocation
    let after = unsafe { (tx as *const Texture).add(1).cast::<u8>().add(mip) };
    // SAFETY: two bytes of the same allocation (zero when the loader did not
    // copy them, which is exactly the divergence this catches)
    let colors = u16::from_le_bytes(unsafe { [*after, *after.add(1)] }) as usize;
    mip + 2 + colors * 3
}

unsafe fn textures(w: &mut Walker, md: &QModel) {
    w.put("numtextures", md.numtextures);
    for i in 0..md.numtextures.max(0) {
        // SAFETY: i < numtextures
        let txp = unsafe { *md.textures.offset(i as isize) };
        if txp.is_null() {
            w.put(&format!("texture[{i}]"), "null");
            continue;
        }
        // SAFETY: non-null slot filled by the loader
        let tx: &Texture = unsafe { &*txp };
        w.put(&format!("texture[{i}].name"), cstr16(&tx.name));
        w.put(&format!("texture[{i}].width"), tx.width);
        w.put(&format!("texture[{i}].height"), tx.height);
        w.put(&format!("texture[{i}].shift"), tx.shift);
        w.put(&format!("texture[{i}].type"), tx.type_);
        w.put(
            &format!("texture[{i}].source_file"),
            cstr16(&tx.source_file),
        );
        w.put(&format!("texture[{i}].source_offset"), tx.source_offset);
        w.put(&format!("texture[{i}].update_warp"), tx.update_warp);
        w.put(
            &format!("texture[{i}].offsets"),
            format!("{:?}", tx.offsets),
        );
        w.put(&format!("texture[{i}].palette"), tx.palette);
        w.put(&format!("texture[{i}].anim_total"), tx.anim_total);
        // SAFETY: payload length derived from the loader's own formula
        let len = unsafe { texture_payload_len(tx) };
        // SAFETY: the pixels immediately follow the struct, as C lays them out
        let pixels = unsafe { core::slice::from_raw_parts(txp.add(1).cast::<u8>(), len) };
        w.put(
            &format!("texture[{i}].pixels"),
            format!("{len} bytes {:02x?}", &pixels[..len.min(64)]),
        );
        w.put(
            &format!("texture[{i}].pixels_tail"),
            format!("{:02x?}", &pixels[len.saturating_sub(16)..]),
        );
    }
}

unsafe fn texinfo(w: &mut Walker, md: &QModel) {
    w.put("numtexinfo", md.numtexinfo);
    for i in 0..md.numtexinfo.max(0) {
        // SAFETY: i < numtexinfo
        let t: &MTexInfo = unsafe { &*md.texinfo.offset(i as isize) };
        w.put(&format!("texinfo[{i}].vecs"), format!("{:?}", t.vecs));
        w.put(&format!("texinfo[{i}].flags"), t.flags);
        w.put(&format!("texinfo[{i}].tex_idx"), t.tex_idx);
        // the texture pointer is one of mod->textures[]'s slots
        let mut which = "extern".to_string();
        if t.texture.is_null() {
            which = "null".to_string();
        } else {
            for j in 0..md.numtextures.max(0) {
                // SAFETY: j < numtextures
                if unsafe { *md.textures.offset(j as isize) } == t.texture {
                    which = format!("#{j}");
                    break;
                }
            }
        }
        w.put(&format!("texinfo[{i}].texture"), which);
    }
}

unsafe fn surfaces(w: &mut Walker, md: &QModel, lens: BlobLens) {
    w.put("numsurfaces", md.numsurfaces);
    for i in 0..md.numsurfaces.max(0) {
        // SAFETY: i < numsurfaces
        let s: &MSurface = unsafe { &*md.surfaces.offset(i as isize) };
        let k = |f: &str| format!("surface[{i}].{f}");
        w.put(&k("firstedge"), s.firstedge);
        w.put(&k("numedges"), s.numedges);
        w.put(&k("flags"), s.flags);
        w.put(&k("texturemins"), format!("{:?}", s.texturemins));
        w.put(&k("extents"), format!("{:?}", s.extents));
        w.put(&k("styles"), format!("{:?}", s.styles));
        w.put(&k("styles_bitmap"), s.styles_bitmap);
        w.put(&k("plane"), slot(s.plane, md.planes, md.numplanes));
        w.put(&k("texinfo"), slot(s.texinfo, md.texinfo, md.numtexinfo));
        w.put(
            &k("samples"),
            blob(s.samples, md.lightdata, lens.lightdata, "lightdata"),
        );
    }
}

unsafe fn leafs(w: &mut Walker, md: &QModel, lens: BlobLens) {
    w.put("numleafs", md.numleafs);
    for i in 0..md.numleafs.max(0) {
        // SAFETY: i < numleafs
        let l: &MLeaf = unsafe { &*md.leafs.offset(i as isize) };
        let k = |f: &str| format!("leaf[{i}].{f}");
        w.put(&k("contents"), l.contents);
        w.put(&k("minmaxs"), format!("{:?}", l.minmaxs));
        w.put(&k("nummarksurfaces"), l.nummarksurfaces);
        w.put(
            &k("ambient_sound_level"),
            format!("{:?}", l.ambient_sound_level),
        );
        w.put(
            &k("compressed_vis"),
            blob(l.compressed_vis, md.visdata, lens.visdata, "visdata"),
        );
        w.put(
            &k("firstmarksurface"),
            slot(l.firstmarksurface, md.marksurfaces, md.nummarksurfaces),
        );
    }
}

/// Node children point at either `mod->nodes` or `mod->leafs`; the label keeps
/// the two spaces apart so a mis-punned child cannot compare equal.
unsafe fn node_child(md: &QModel, p: *mut MNode) -> String {
    if p.is_null() {
        return "null".to_string();
    }
    let as_node = slot(p, md.nodes, md.numnodes);
    if as_node != "extern" {
        return format!("node{as_node}");
    }
    let as_leaf = slot(p.cast::<MLeaf>(), md.leafs, md.numleafs);
    if as_leaf != "extern" {
        return format!("leaf{as_leaf}");
    }
    "extern".to_string()
}

unsafe fn nodes(w: &mut Walker, md: &QModel) {
    w.put("numnodes", md.numnodes);
    for i in 0..md.numnodes.max(0) {
        // SAFETY: i < numnodes
        let n: &MNode = unsafe { &*md.nodes.offset(i as isize) };
        let k = |f: &str| format!("node[{i}].{f}");
        w.put(&k("contents"), n.contents);
        w.put(&k("minmaxs"), format!("{:?}", n.minmaxs));
        w.put(&k("firstsurface"), n.firstsurface);
        w.put(&k("numsurfaces"), n.numsurfaces);
        w.put(&k("plane"), slot(n.plane, md.planes, md.numplanes));
        // SAFETY: children were written by the loader
        w.put(&k("children[0]"), unsafe { node_child(md, n.children[0]) });
        // SAFETY: as above
        w.put(&k("children[1]"), unsafe { node_child(md, n.children[1]) });
    }
}

unsafe fn clipnodes(w: &mut Walker, md: &QModel) {
    w.put("numclipnodes", md.numclipnodes);
    for i in 0..md.numclipnodes.max(0) {
        // SAFETY: i < numclipnodes
        let c: &MClipnode = unsafe { &*md.clipnodes.offset(i as isize) };
        w.put(&format!("clipnode[{i}].planenum"), c.planenum);
        w.put(
            &format!("clipnode[{i}].children"),
            format!("{:?}", c.children),
        );
    }
}

unsafe fn hulls(w: &mut Walker, md: &QModel) {
    for (h, hull) in md.hulls.iter().enumerate() {
        let k = |f: &str| format!("hull[{h}].{f}");
        w.put(&k("firstclipnode"), hull.firstclipnode);
        w.put(&k("lastclipnode"), hull.lastclipnode);
        w.put(&k("clip_mins"), format!("{:?}", hull.clip_mins));
        w.put(&k("clip_maxs"), format!("{:?}", hull.clip_maxs));
        w.put(&k("planes"), slot(hull.planes, md.planes, md.numplanes));
        let shared = slot(hull.clipnodes, md.clipnodes, md.numclipnodes);
        if shared != "extern" || hull.clipnodes.is_null() {
            w.put(&k("clipnodes"), format!("shared {shared}"));
            continue;
        }
        // hull 0 owns a private clipnode array built from the node tree
        w.put(&k("clipnodes"), "private");
        let n = (hull.lastclipnode - hull.firstclipnode + 1).max(0);
        for i in 0..n {
            // SAFETY: the loader allocated lastclipnode+1 entries
            let c: &MClipnode = unsafe { &*hull.clipnodes.offset(i as isize) };
            w.put(
                &format!("hull[{h}].clipnode[{i}]"),
                format!("{} {:?}", c.planenum, c.children),
            );
        }
    }
}

unsafe fn submodels(w: &mut Walker, md: &QModel) {
    w.put("numsubmodels", md.numsubmodels);
    for i in 0..md.numsubmodels.max(0) {
        // SAFETY: i < numsubmodels
        let s = unsafe { &*md.submodels.offset(i as isize) };
        let k = |f: &str| format!("submodel[{i}].{f}");
        w.put(&k("mins"), format!("{:?}", s.mins));
        w.put(&k("maxs"), format!("{:?}", s.maxs));
        w.put(&k("origin"), format!("{:?}", s.origin));
        w.put(&k("headnode"), format!("{:?}", s.headnode));
        w.put(&k("visleafs"), s.visleafs);
        w.put(&k("firstface"), s.firstface);
        w.put(&k("numfaces"), s.numfaces);
    }
}

/// `usedtextures` is the per-texture-type index list Mod_CalcSpecialsAndTextures
/// builds; its length is the last `texofs` entry.
unsafe fn used_textures(w: &mut Walker, md: &QModel) {
    let total = md.texofs[md.texofs.len() - 1];
    if md.usedtextures.is_null() {
        w.put("usedtextures", "null");
        return;
    }
    w.put("usedtextures.len", total);
    for i in 0..total.max(0) {
        // SAFETY: i < texofs[TEXTYPE_COUNT], the array's allocated length
        w.put(&format!("usedtextures[{i}]"), unsafe {
            *md.usedtextures.offset(i as isize)
        });
    }
}

unsafe fn blobs(w: &mut Walker, md: &QModel, lens: BlobLens) {
    if md.visdata.is_null() {
        w.put("visdata", "null");
    } else {
        // SAFETY: the fixture supplied the real length
        let d = unsafe { core::slice::from_raw_parts(md.visdata, lens.visdata) };
        w.put("visdata", format!("{} {:02x?}", d.len(), d));
    }
    if md.lightdata.is_null() {
        w.put("lightdata", "null");
    } else {
        // SAFETY: as above
        let d = unsafe { core::slice::from_raw_parts(md.lightdata, lens.lightdata) };
        w.put("lightdata", format!("{} {:02x?}", d.len(), d));
    }
    if md.entities.is_null() {
        w.put("entities", "null");
    } else {
        // SAFETY: the loaders always NUL-terminate the entity string
        let s = unsafe { std::ffi::CStr::from_ptr(md.entities) };
        w.put("entities", format!("{:?}", s.to_string_lossy()));
    }
}

// ---------------------------------------------------------------------------
// Alias / sprite models (Phase 3 M4)

/// `spriteframetype_t::SPR_SINGLE` (spritegn.h); the group variants all reuse
/// the same `frameptr` slot for an `mspritegroup_t`.
const SPR_SINGLE: i32 = 0;

/// The alias loader's scratch state. `stverts`/`triangles`/`poseverts` keep C
/// linkage and are shared by both sides, so they are snapshotted right after
/// each side's parse; `base` is the file image both sides were handed, which
/// turns the `poseverts` pointers into comparable offsets.
#[derive(Clone, Copy)]
pub struct AliasScratch {
    pub stverts: *const StVert,
    pub triangles: *const MTriangle,
    pub poseverts: *const *const TriVertX,
    pub base: *const u8,
    pub base_len: usize,
}

/// The `qmodel_t` fields the alias and sprite loaders write. `extradata` is
/// reported as "set"/"null" only -- the pointed-to header is walked by the
/// caller-facing snapshot, and the pointer itself is heap-dependent.
///
/// # Safety
/// `m` must point at a live `qmodel_t`.
unsafe fn model_fields(w: &mut Walker, m: *const QModel) {
    // SAFETY: caller's contract
    let md = unsafe { &*m };
    w.put("model.type", md.type_);
    w.put("model.flags", md.flags);
    w.put("model.numframes", md.numframes);
    w.put("model.synctype", md.synctype);
    w.put("model.mins", format!("{:?}", md.mins));
    w.put("model.maxs", format!("{:?}", md.maxs));
    w.put("model.ymins", format!("{:?}", md.ymins));
    w.put("model.ymaxs", format!("{:?}", md.ymaxs));
    w.put("model.rmins", format!("{:?}", md.rmins));
    w.put("model.rmaxs", format!("{:?}", md.rmaxs));
    for (i, e) in md.extradata.iter().enumerate() {
        w.put(
            &format!("model.extradata[{i}]"),
            if e.is_null() { "null" } else { "set" },
        );
    }
}

/// Snapshot of an `aliashdr_t` filled by `Mod_ParseAliasModel`, plus the
/// scratch arrays it wrote through.
///
/// Excluded (renderer-owned, never written by the parse): `gltextures`,
/// `fbtextures`, `texels`, `nextsurface`, the vertex/index/joints buffer
/// handles and `vbostofs`.
///
/// # Safety
/// `h` must point at a header `Mod_ParseAliasModel` returned for `m`, and
/// `scratch` must describe the globals as they stood right after that call.
pub unsafe fn alias_snapshot(
    m: *const QModel,
    h: *const AliasHdr,
    scratch: AliasScratch,
) -> Snapshot {
    let mut w = Walker { lines: Vec::new() };
    // SAFETY: caller's contract
    unsafe {
        model_fields(&mut w, m);

        let a = &*h;
        w.put("alias.ident", a.ident);
        w.put("alias.version", a.version);
        w.put("alias.scale", format!("{:?}", a.scale));
        w.put("alias.scale_origin", format!("{:?}", a.scale_origin));
        w.put("alias.boundingradius", a.boundingradius);
        w.put("alias.eyeposition", format!("{:?}", a.eyeposition));
        w.put("alias.numskins", a.numskins);
        w.put("alias.skinwidth", a.skinwidth);
        w.put("alias.skinheight", a.skinheight);
        w.put("alias.numverts", a.numverts);
        w.put("alias.numtris", a.numtris);
        w.put("alias.numframes", a.numframes);
        w.put("alias.synctype", a.synctype);
        w.put("alias.flags", a.flags);
        w.put("alias.size", a.size);
        w.put("alias.numindexes", a.numindexes);
        w.put("alias.numverts_vbo", a.numverts_vbo);
        w.put("alias.numposes", a.numposes);
        w.put("alias.numjoints", a.numjoints);
        w.put("alias.poseverttype", a.poseverttype);

        let frames = core::ptr::addr_of!((*h).frames).cast::<MAliasFrameDesc>();
        for i in 0..a.numframes.max(0) {
            let f = &*frames.offset(i as isize);
            w.put(&format!("alias.frame[{i}].firstpose"), f.firstpose);
            w.put(&format!("alias.frame[{i}].numposes"), f.numposes);
            w.put(&format!("alias.frame[{i}].interval"), f.interval);
            w.put(
                &format!("alias.frame[{i}].bboxmin"),
                format!("{:?}", f.bboxmin.v),
            );
            w.put(
                &format!("alias.frame[{i}].bboxmin.lni"),
                f.bboxmin.lightnormalindex,
            );
            w.put(
                &format!("alias.frame[{i}].bboxmax"),
                format!("{:?}", f.bboxmax.v),
            );
            w.put(
                &format!("alias.frame[{i}].bboxmax.lni"),
                f.bboxmax.lightnormalindex,
            );
            w.put(&format!("alias.frame[{i}].frame"), f.frame);
            w.put(&format!("alias.frame[{i}].name"), cstr16(&f.name));
        }

        for i in 0..a.numverts.max(0) {
            let v = &*scratch.stverts.offset(i as isize);
            w.put(
                &format!("stvert[{i}]"),
                format!("onseam={} s={} t={}", v.onseam, v.s, v.t),
            );
        }
        for i in 0..a.numtris.max(0) {
            let t = &*scratch.triangles.offset(i as isize);
            w.put(
                &format!("triangle[{i}]"),
                format!("front={} idx={:?}", t.facesfront, t.vertindex),
            );
        }
        for i in 0..a.numposes.max(0) {
            let p = *scratch.poseverts.offset(i as isize);
            w.put(
                &format!("posevert[{i}]"),
                blob(
                    p as *mut u8,
                    scratch.base as *mut u8,
                    scratch.base_len,
                    "file",
                ),
            );
        }
    }
    Snapshot { lines: w.lines }
}

/// Snapshot of the `aliashdr_t` chain an MD3 or MD5 load produced
/// (Phase 3 M5). The headers are one `Mem_Alloc` block of `hdrsize`-strided
/// entries linked through `nextsurface`, so the walk follows the chain and
/// records each link as `#i` (its index in that block) or `null` -- the raw
/// pointer is heap-dependent.
///
/// Excluded, as in [`alias_snapshot`]: `gltextures`, `fbtextures`, `texels`,
/// the vertex/index/joints buffer handles and `vbostofs`. The *contents* the
/// loader parsed are not in the header at all -- they go straight to
/// `GLMesh_UploadBuffers` -- so the differential compares the recorded upload
/// buffers alongside this snapshot.
///
/// # Safety
/// `m` must point at a `qmodel_t` a successful MD3/MD5 load filled, and
/// `slot` must be the `extradata` index that load wrote.
pub unsafe fn mdx_snapshot(m: *const QModel, slot: usize) -> Snapshot {
    let mut w = Walker { lines: Vec::new() };
    // SAFETY: caller's contract
    unsafe {
        model_fields(&mut w, m);

        let head = (*m).extradata[slot].cast::<AliasHdr>();
        if head.is_null() {
            w.put("mdx.head", "null");
            return Snapshot { lines: w.lines };
        }

        // The block stride: every header in the chain is `hdrsize` apart, so
        // the first link gives it. A single-surface model has no link, and
        // then no index resolution is needed either.
        let stride = {
            let next = (*head).nextsurface;
            if next.is_null() {
                0usize
            } else {
                (next.cast::<u8>()).offset_from(head.cast::<u8>()) as usize
            }
        };
        w.put("mdx.hdrsize", stride);

        let mut surf = head;
        let mut index = 0usize;
        while !surf.is_null() {
            let a = &*surf;
            let k = |f: &str| format!("mdx.surf[{index}].{f}");
            let link = if a.nextsurface.is_null() {
                "null".to_string()
            } else {
                let delta = (a.nextsurface.cast::<u8>()).offset_from(head.cast::<u8>()) as usize;
                // stride is 0 only when the head has no link at all, so a
                // non-null link with an unknown stride cannot be resolved
                match delta.checked_div(stride) {
                    Some(i) => format!("#{i}"),
                    None => "extern".to_string(),
                }
            };
            w.put(&k("nextsurface"), link);
            w.put(&k("ident"), a.ident);
            w.put(&k("version"), a.version);
            w.put(&k("scale"), format!("{:?}", a.scale));
            w.put(&k("scale_origin"), format!("{:?}", a.scale_origin));
            w.put(&k("boundingradius"), a.boundingradius);
            w.put(&k("eyeposition"), format!("{:?}", a.eyeposition));
            w.put(&k("numskins"), a.numskins);
            w.put(&k("skinwidth"), a.skinwidth);
            w.put(&k("skinheight"), a.skinheight);
            w.put(&k("numverts"), a.numverts);
            w.put(&k("numtris"), a.numtris);
            w.put(&k("numframes"), a.numframes);
            w.put(&k("synctype"), a.synctype);
            w.put(&k("flags"), a.flags);
            w.put(&k("size"), a.size);
            w.put(&k("numindexes"), a.numindexes);
            w.put(&k("numverts_vbo"), a.numverts_vbo);
            w.put(&k("numposes"), a.numposes);
            w.put(&k("numjoints"), a.numjoints);
            w.put(&k("poseverttype"), a.poseverttype);

            let frames = core::ptr::addr_of!((*surf).frames).cast::<MAliasFrameDesc>();
            for i in 0..a.numframes.max(0) {
                let f = &*frames.offset(i as isize);
                w.put(&k(&format!("frame[{i}].firstpose")), f.firstpose);
                w.put(&k(&format!("frame[{i}].numposes")), f.numposes);
                w.put(&k(&format!("frame[{i}].interval")), f.interval);
                w.put(
                    &k(&format!("frame[{i}].bboxmin")),
                    format!("{:?}", f.bboxmin.v),
                );
                w.put(
                    &k(&format!("frame[{i}].bboxmin.lni")),
                    f.bboxmin.lightnormalindex,
                );
                w.put(
                    &k(&format!("frame[{i}].bboxmax")),
                    format!("{:?}", f.bboxmax.v),
                );
                w.put(
                    &k(&format!("frame[{i}].bboxmax.lni")),
                    f.bboxmax.lightnormalindex,
                );
                w.put(&k(&format!("frame[{i}].frame")), f.frame);
                w.put(&k(&format!("frame[{i}].name")), cstr16(&f.name));
            }

            surf = a.nextsurface;
            index += 1;
        }
        w.put("mdx.numsurfaces", index);
    }
    Snapshot { lines: w.lines }
}

/// Snapshot of an `msprite_t` filled by `Mod_LoadSpriteModel`. The
/// `gltexture` pointers are excluded (the differential compares the
/// `TexMgr_LoadImage` argument stream instead).
///
/// # Safety
/// `m` must point at a `qmodel_t` a successful `Mod_LoadSpriteModel` filled.
pub unsafe fn sprite_snapshot(m: *const QModel) -> Snapshot {
    let mut w = Walker { lines: Vec::new() };
    // SAFETY: caller's contract
    unsafe {
        model_fields(&mut w, m);

        let s = (*m).extradata[PV_QUAKE1 as usize].cast::<MSprite>();
        if s.is_null() {
            w.put("sprite", "null");
            return Snapshot { lines: w.lines };
        }
        let sp = &*s;
        w.put("sprite.type", sp.type_);
        w.put("sprite.maxwidth", sp.maxwidth);
        w.put("sprite.maxheight", sp.maxheight);
        w.put("sprite.numframes", sp.numframes);

        let descs = core::ptr::addr_of!((*s).frames).cast::<MSpriteFrameDesc>();
        for i in 0..sp.numframes.max(0) {
            let d = &*descs.offset(i as isize);
            w.put(&format!("sprite.frame[{i}].type"), d.type_);
            if d.type_ == SPR_SINGLE {
                sprite_frame(&mut w, &format!("sprite.frame[{i}]"), d.frameptr);
            } else {
                let g = d.frameptr.cast::<MSpriteGroup>();
                if g.is_null() {
                    w.put(&format!("sprite.frame[{i}].group"), "null");
                    continue;
                }
                let gr = &*g;
                w.put(&format!("sprite.frame[{i}].group.numframes"), gr.numframes);
                for j in 0..gr.numframes.max(0) {
                    w.put(
                        &format!("sprite.frame[{i}].group.interval[{j}]"),
                        *gr.intervals.offset(j as isize),
                    );
                }
                let sub = core::ptr::addr_of!((*g).frames).cast::<*mut MSpriteFrame>();
                for j in 0..gr.numframes.max(0) {
                    sprite_frame(
                        &mut w,
                        &format!("sprite.frame[{i}].group[{j}]"),
                        *sub.offset(j as isize),
                    );
                }
            }
        }
    }
    Snapshot { lines: w.lines }
}

/// # Safety
/// `f` must be null or point at a live `mspriteframe_t`.
unsafe fn sprite_frame(w: &mut Walker, key: &str, f: *mut MSpriteFrame) {
    if f.is_null() {
        w.put(key, "null");
        return;
    }
    // SAFETY: caller's contract
    let fr = unsafe { &*f };
    w.put(&format!("{key}.width"), fr.width);
    w.put(&format!("{key}.height"), fr.height);
    w.put(&format!("{key}.up"), fr.up);
    w.put(&format!("{key}.down"), fr.down);
    w.put(&format!("{key}.left"), fr.left);
    w.put(&format!("{key}.right"), fr.right);
    w.put(&format!("{key}.smax"), fr.smax);
    w.put(&format!("{key}.tmax"), fr.tmax);
}
