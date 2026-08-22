//! ABI cross-check: the `quake_types::{bspfile, model_mem}` mirrors vs what
//! the engine's own headers (`bspfile.h` / `gl_model.h`) say on this platform.
//!
//! Under `-Duse_rust_formats` the C renderer and server walk `qmodel_t`
//! trees that the Rust loaders filled in place, so any mirror drift is
//! silent memory corruption rather than a link error. The const asserts in
//! quake-types pin the 64-bit layout at Rust compile time; this test is the
//! per-platform gate, compiled from the engine's own headers by CI on every
//! OS in the test matrix (RA1 in the phase plan).
//!
//! The probe is name-keyed ("sizeof.qmodel_t", "qmodel_t.hulls", ...) so the
//! C table and this consumer can't drift by index; an unknown key returns
//! usize::MAX and fails the assert.

use core::mem::{offset_of, size_of};

use quake_ctest as _;
use quake_types::bspfile::{
    DHeader, DL1Leaf, DL1Node, DL2Leaf, DL2Node, DLClipnode, DLEdge, DLFace, DMipTexLump, DModel,
    DPlane, DSClipnode, DSEdge, DSFace, DSLeaf, DSNode, DVertex, LumpT, MipTex, MipTex64, TexInfo,
};
use quake_types::model_mem::{
    Hull, MClipnode, MEdge, MLeaf, MNode, MSurface, MTexInfo, MVertex, QModel, Texture,
};
use quake_types::MPlane;

extern "C" {
    fn ctest_abi_bsp_lookup(key: *const core::ffi::c_char) -> usize;
}

fn c_abi(key: &str) -> usize {
    let cstr = std::ffi::CString::new(key).unwrap();
    // SAFETY: the probe only strcmp's the key against a compile-time table.
    let v = unsafe { ctest_abi_bsp_lookup(cstr.as_ptr()) };
    assert_ne!(v, usize::MAX, "key {key:?} missing from the C probe table");
    v
}

macro_rules! check_size {
    ($rust:ty, $ctag:literal) => {
        assert_eq!(
            size_of::<$rust>(),
            c_abi(concat!("sizeof.", $ctag)),
            concat!("sizeof ", $ctag)
        );
    };
}

macro_rules! check_offsets {
    ($rust:ty, $ctag:literal, [$($field:ident),+ $(,)?]) => {
        $(
            assert_eq!(
                offset_of!($rust, $field),
                c_abi(concat!($ctag, ".", stringify!($field))),
                concat!($ctag, ".", stringify!($field))
            );
        )+
    };
}

#[test]
fn on_disk_struct_sizes_match_the_c_headers() {
    check_size!(LumpT, "lump_t");
    check_size!(DHeader, "dheader_t");
    check_size!(DModel, "dmodel_t");
    check_size!(DMipTexLump, "dmiptexlump_t");
    check_size!(MipTex, "miptex_t");
    check_size!(MipTex64, "miptex64_t");
    check_size!(DVertex, "dvertex_t");
    check_size!(DPlane, "dplane_t");
    check_size!(DSNode, "dsnode_t");
    check_size!(DL1Node, "dl1node_t");
    check_size!(DL2Node, "dl2node_t");
    check_size!(DSClipnode, "dsclipnode_t");
    check_size!(DLClipnode, "dlclipnode_t");
    check_size!(TexInfo, "texinfo_t");
    check_size!(DSEdge, "dsedge_t");
    check_size!(DLEdge, "dledge_t");
    check_size!(DSFace, "dsface_t");
    check_size!(DLFace, "dlface_t");
    check_size!(DSLeaf, "dsleaf_t");
    check_size!(DL1Leaf, "dl1leaf_t");
    check_size!(DL2Leaf, "dl2leaf_t");
}

#[test]
fn in_memory_struct_sizes_match_the_c_headers() {
    check_size!(QModel, "qmodel_t");
    check_size!(Texture, "texture_t");
    check_size!(MSurface, "msurface_t");
    check_size!(MNode, "mnode_t");
    check_size!(MLeaf, "mleaf_t");
    check_size!(MClipnode, "mclipnode_t");
    check_size!(Hull, "hull_t");
    check_size!(MTexInfo, "mtexinfo_t");
    check_size!(MEdge, "medge_t");
    check_size!(MVertex, "mvertex_t");
    check_size!(MPlane, "mplane_t");
}

#[test]
fn qmodel_field_offsets_match_the_c_headers() {
    check_offsets!(
        QModel,
        "qmodel_t",
        [
            name,
            needload,
            flags,
            mins,
            maxs,
            ymins,
            rmins,
            clipbox,
            clipmins,
            firstmodelsurface,
            nummodelsurfaces,
            numsubmodels,
            submodels,
            numplanes,
            planes,
            numleafs,
            leafs,
            numvertexes,
            vertexes,
            numedges,
            edges,
            numnodes,
            nodes,
            numtexinfo,
            texinfo,
            numsurfaces,
            surfaces,
            numsurfedges,
            surfedges,
            numclipnodes,
            clipnodes,
            nummarksurfaces,
            marksurfaces,
            soa_leafbounds,
            hulls,
            numtextures,
            textures,
            texofs,
            usedtextures,
            visdata,
            lightdata,
            entities,
            viswarn,
            bogus_tree,
            bspversion,
            contentstransparent,
            used_specials,
            extradata,
            blas,
        ]
    );
    // `type` is a keyword; the mirror names it type_
    assert_eq!(
        offset_of!(QModel, type_),
        c_abi("qmodel_t.type"),
        "qmodel_t.type"
    );
}

#[test]
fn texture_field_offsets_match_the_c_headers() {
    check_offsets!(
        Texture,
        "texture_t",
        [
            name,
            width,
            height,
            shift,
            source_file,
            source_offset,
            update_warp,
            texturechains,
            chain_size,
            anim_total,
            anim_min,
            anim_max,
            anim_next,
            alternate_anims,
            offsets,
            palette,
        ]
    );
    assert_eq!(
        offset_of!(Texture, type_),
        c_abi("texture_t.type"),
        "texture_t.type"
    );
}

#[test]
fn surface_and_tree_field_offsets_match_the_c_headers() {
    check_offsets!(
        MSurface,
        "msurface_t",
        [
            visframe,
            plane,
            flags,
            firstedge,
            numedges,
            texturemins,
            extents,
            polys,
            texinfo,
            dlightbits,
            styles,
            styles_bitmap,
            cached_light,
            cached_dlight,
            samples,
        ]
    );
    check_offsets!(
        MNode,
        "mnode_t",
        [
            contents,
            minmaxs,
            firstsurface,
            numsurfaces,
            plane,
            children,
        ]
    );
    check_offsets!(
        MLeaf,
        "mleaf_t",
        [
            contents,
            minmaxs,
            nummarksurfaces,
            combined_deps,
            ambient_sound_level,
            compressed_vis,
            firstmarksurface,
            efrags,
        ]
    );
    check_offsets!(MClipnode, "mclipnode_t", [planenum, children]);
    check_offsets!(
        Hull,
        "hull_t",
        [
            clipnodes,
            planes,
            firstclipnode,
            lastclipnode,
            clip_mins,
            clip_maxs,
        ]
    );
    check_offsets!(MTexInfo, "mtexinfo_t", [vecs, texture, flags, tex_idx]);
    check_offsets!(MEdge, "medge_t", [v, cachededgeoffset]);
}
