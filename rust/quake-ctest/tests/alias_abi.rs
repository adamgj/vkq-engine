//! ABI cross-check for the MDL/SPR half of the model seam: the
//! `quake_types::{modelgen, spritegn, model_mem}` mirrors vs what the
//! engine's own headers (`modelgen.h` / `spritegn.h` / `gl_model.h`) say on
//! this platform. Same contract as `bsp_abi.rs` -- under
//! `-Duse_rust_formats` the C renderer walks `aliashdr_t` / `msprite_t`
//! trees the Rust loaders filled in place, so mirror drift is silent memory
//! corruption; the const asserts in quake-types pin the 64-bit layout at
//! Rust compile time and this test is the per-platform gate CI runs on
//! every OS in the matrix (RA1 in the phase plan).

use core::mem::{offset_of, size_of};

use quake_ctest as _;
use quake_types::model_mem::{
    AliasHdr, MAliasFrameDesc, MSprite, MSpriteFrame, MSpriteFrameDesc, MSpriteGroup, MTriangle,
    Md3XyzNormal, Md5Vert, Md5Vert8,
};
use quake_types::modelgen::{
    DAliasFrame, DAliasFrameType, DAliasGroup, DAliasInterval, DAliasSkinGroup, DAliasSkinInterval,
    DAliasSkinType, DTriangle, MdlT, StVert, TriVertX,
};
use quake_types::spritegn::{
    DSprite, DSpriteFrame, DSpriteFrameType, DSpriteGroup, DSpriteInterval,
};

extern "C" {
    fn ctest_abi_alias_lookup(key: *const core::ffi::c_char) -> usize;
}

fn c_abi(key: &str) -> usize {
    let cstr = std::ffi::CString::new(key).unwrap();
    // SAFETY: the probe only strcmp's the key against a compile-time table.
    let v = unsafe { ctest_abi_alias_lookup(cstr.as_ptr()) };
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
    check_size!(MdlT, "mdl_t");
    check_size!(StVert, "stvert_t");
    check_size!(DTriangle, "dtriangle_t");
    check_size!(TriVertX, "trivertx_t");
    check_size!(DAliasFrame, "daliasframe_t");
    check_size!(DAliasGroup, "daliasgroup_t");
    check_size!(DAliasSkinGroup, "daliasskingroup_t");
    check_size!(DAliasInterval, "daliasinterval_t");
    check_size!(DAliasSkinInterval, "daliasskininterval_t");
    check_size!(DAliasFrameType, "daliasframetype_t");
    check_size!(DAliasSkinType, "daliasskintype_t");

    check_size!(DSprite, "dsprite_t");
    check_size!(DSpriteFrame, "dspriteframe_t");
    check_size!(DSpriteGroup, "dspritegroup_t");
    check_size!(DSpriteInterval, "dspriteinterval_t");
    check_size!(DSpriteFrameType, "dspriteframetype_t");
}

#[test]
fn on_disk_field_offsets_match_the_c_headers() {
    check_offsets!(
        MdlT,
        "mdl_t",
        [
            ident,
            version,
            scale,
            scale_origin,
            boundingradius,
            eyeposition,
            numskins,
            skinwidth,
            skinheight,
            numverts,
            numtris,
            numframes,
            synctype,
            flags,
            size,
        ]
    );
    check_offsets!(StVert, "stvert_t", [onseam, s, t]);
    check_offsets!(DTriangle, "dtriangle_t", [facesfront, vertindex]);
    check_offsets!(TriVertX, "trivertx_t", [v, lightnormalindex]);
    check_offsets!(DAliasFrame, "daliasframe_t", [bboxmin, bboxmax, name]);
    check_offsets!(DAliasGroup, "daliasgroup_t", [numframes, bboxmin, bboxmax]);

    check_offsets!(
        DSprite,
        "dsprite_t",
        [
            ident,
            version,
            boundingradius,
            width,
            height,
            numframes,
            beamlength,
            synctype,
        ]
    );
    // `type` is a keyword; the mirror names it type_
    assert_eq!(
        offset_of!(DSprite, type_),
        c_abi("dsprite_t.type"),
        "dsprite_t.type"
    );
    check_offsets!(DSpriteFrame, "dspriteframe_t", [origin, width, height]);
    check_offsets!(DSpriteGroup, "dspritegroup_t", [numframes]);
    check_offsets!(DSpriteInterval, "dspriteinterval_t", [interval]);
}

#[test]
fn in_memory_struct_sizes_match_the_c_headers() {
    check_size!(AliasHdr, "aliashdr_t");
    check_size!(MAliasFrameDesc, "maliasframedesc_t");
    check_size!(MTriangle, "mtriangle_t");
    check_size!(Md5Vert, "md5vert_t");
    check_size!(Md5Vert8, "md5vert8_t");
    check_size!(Md3XyzNormal, "md3XyzNormal_t");

    check_size!(MSprite, "msprite_t");
    check_size!(MSpriteFrame, "mspriteframe_t");
    check_size!(MSpriteGroup, "mspritegroup_t");
    check_size!(MSpriteFrameDesc, "mspriteframedesc_t");
}

#[test]
fn in_memory_field_offsets_match_the_c_headers() {
    check_offsets!(
        AliasHdr,
        "aliashdr_t",
        [
            ident,
            version,
            scale,
            scale_origin,
            boundingradius,
            eyeposition,
            numskins,
            skinwidth,
            skinheight,
            numverts,
            numtris,
            numframes,
            synctype,
            flags,
            size,
            numindexes,
            numverts_vbo,
            numposes,
            nextsurface,
            numjoints,
            poseverttype,
            gltextures,
            fbtextures,
            texels,
            vertex_buffer,
            vbostofs,
            joints_set,
            frames,
        ]
    );
    check_offsets!(
        MAliasFrameDesc,
        "maliasframedesc_t",
        [firstpose, numposes, interval, bboxmin, bboxmax, frame, name]
    );
    check_offsets!(MTriangle, "mtriangle_t", [facesfront, vertindex]);
    check_offsets!(Md5Vert, "md5vert_t", [xyz]);
    check_offsets!(Md5Vert8, "md5vert8_t", [xyz]);
    check_offsets!(Md3XyzNormal, "md3XyzNormal_t", [xyz, latlong]);

    check_offsets!(
        MSprite,
        "msprite_t",
        [maxwidth, maxheight, numframes, frames]
    );
    check_offsets!(
        MSpriteFrame,
        "mspriteframe_t",
        [width, height, up, down, left, right, smax, tmax, gltexture]
    );
    check_offsets!(
        MSpriteGroup,
        "mspritegroup_t",
        [numframes, intervals, frames]
    );
    check_offsets!(MSpriteFrameDesc, "mspriteframedesc_t", [frameptr]);
    // `type` is a keyword; the mirrors name it type_
    assert_eq!(
        offset_of!(MSprite, type_),
        c_abi("msprite_t.type"),
        "msprite_t.type"
    );
    assert_eq!(
        offset_of!(MSpriteFrameDesc, type_),
        c_abi("mspriteframedesc_t.type"),
        "mspriteframedesc_t.type"
    );
}
