//! ABI cross-check for the MD3/MD5 half of the model seam: the
//! `quake_types::{md3, model_mem}` mirrors vs what the engine's own
//! `gl_model.h` says on this platform. Same contract as `bsp_abi.rs` /
//! `alias_abi.rs` -- under `-Duse_rust_formats` the C renderer walks the
//! `aliashdr_t` chains and the vertex/joint buffers the Rust MD3/MD5 loaders
//! filled, so mirror drift is silent memory corruption. The const asserts in
//! quake-types pin the 64-bit layout at Rust compile time; this test is the
//! per-platform gate CI runs on every OS in the matrix (RA1 in the phase
//! plan).

use core::mem::{offset_of, size_of};

use quake_ctest as _;
use quake_types::md3::{Md3Frame, Md3Header, Md3Shader, Md3St, Md3Surface, Md3Triangle};
use quake_types::model_mem::{
    AliasMesh, AllSurfacesDef, JointPose, Md5Vert, Md5Vert8, QPathStr, SkinDef, SurfaceDef,
};

extern "C" {
    fn ctest_abi_mdx_lookup(key: *const core::ffi::c_char) -> usize;
}

fn c_abi(key: &str) -> usize {
    let cstr = std::ffi::CString::new(key).unwrap();
    // SAFETY: the probe only strcmp's the key against a compile-time table.
    let v = unsafe { ctest_abi_mdx_lookup(cstr.as_ptr()) };
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
    ($rust:ty, $ctag:literal, [$(($field:ident, $cfield:literal)),+ $(,)?]) => {
        $(
            assert_eq!(
                offset_of!($rust, $field),
                c_abi(concat!($ctag, ".", $cfield)),
                concat!($ctag, ".", $cfield)
            );
        )+
    };
}

#[test]
fn md3_on_disk_layout_matches_the_c_headers() {
    check_size!(Md3Header, "md3Header_t");
    check_size!(Md3Frame, "md3Frame_t");
    check_size!(Md3Surface, "md3Surface_t");
    check_size!(Md3Triangle, "md3Triangle_t");
    check_size!(Md3St, "md3St_t");
    check_size!(Md3Shader, "md3Shader_t");

    check_offsets!(
        Md3Header,
        "md3Header_t",
        [
            (ident, "ident"),
            (version, "version"),
            (name, "name"),
            (flags, "flags"),
            (num_frames, "numFrames"),
            (num_tags, "numTags"),
            (num_surfaces, "numSurfaces"),
            (num_skins, "numSkins"),
            (ofs_frames, "ofsFrames"),
            (ofs_tags, "ofsTags"),
            (ofs_surfaces, "ofsSurfaces"),
            (ofs_end, "ofsEnd"),
        ]
    );
    check_offsets!(
        Md3Frame,
        "md3Frame_t",
        [
            (bounds, "bounds"),
            (local_origin, "localOrigin"),
            (radius, "radius"),
            (name, "name"),
        ]
    );
    check_offsets!(
        Md3Surface,
        "md3Surface_t",
        [
            (ident, "ident"),
            (name, "name"),
            (flags, "flags"),
            (num_frames, "numFrames"),
            (num_shaders, "numShaders"),
            (num_verts, "numVerts"),
            (num_triangles, "numTriangles"),
            (ofs_triangles, "ofsTriangles"),
            (ofs_shaders, "ofsShaders"),
            (ofs_st, "ofsSt"),
            (ofs_xyz_normals, "ofsXyzNormals"),
            (ofs_end, "ofsEnd"),
        ]
    );
    check_offsets!(Md3Triangle, "md3Triangle_t", [(indexes, "indexes")]);
    check_offsets!(Md3St, "md3St_t", [(s, "s"), (t, "t")]);
    check_offsets!(
        Md3Shader,
        "md3Shader_t",
        [(name, "name"), (shader_index, "shaderIndex")]
    );
}

#[test]
fn md5_vertex_layouts_match_the_c_headers() {
    check_offsets!(
        Md5Vert,
        "md5vert_t",
        [
            (xyz, "xyz"),
            (norm, "norm"),
            (st, "st"),
            (joint_weights, "joint_weights"),
            (joint_indices, "joint_indices"),
            (joint_position_x, "joint_position_x"),
            (joint_position_y, "joint_position_y"),
            (joint_position_z, "joint_position_z"),
        ]
    );
    check_offsets!(
        Md5Vert8,
        "md5vert8_t",
        [
            (xyz, "xyz"),
            (norm, "norm"),
            (st, "st"),
            (joint_weights, "joint_weights"),
            (joint_indices, "joint_indices"),
            (joint_position_x, "joint_position_x"),
            (joint_position_y, "joint_position_y"),
            (joint_position_z, "joint_position_z"),
        ]
    );
}

#[test]
fn upload_buffer_record_layouts_match_the_c_headers() {
    check_size!(JointPose, "jointpose_t");
    check_size!(AliasMesh, "aliasmesh_t");
    check_offsets!(JointPose, "jointpose_t", [(mat, "mat")]);
    check_offsets!(
        AliasMesh,
        "aliasmesh_t",
        [(st, "st"), (vertindex, "vertindex")]
    );
}

#[test]
fn skin_definition_block_matches_the_c_headers() {
    check_size!(QPathStr, "qpath_str_t");
    check_size!(SkinDef, "skin_def_t");
    check_size!(SurfaceDef, "surface_def_t");
    check_size!(AllSurfacesDef, "all_surfaces_def_t");

    check_offsets!(QPathStr, "qpath_str_t", [(c_str, "c_str")]);
    check_offsets!(
        SkinDef,
        "skin_def_t",
        [
            (framegroups, "framegroups"),
            (numframegroups, "numframegroups"),
        ]
    );
    check_offsets!(
        SurfaceDef,
        "surface_def_t",
        [
            (surfname, "surfname"),
            (skins, "skins"),
            (numskins, "numskins"),
        ]
    );
    check_offsets!(
        AllSurfacesDef,
        "all_surfaces_def_t",
        [(surfaces, "surfaces"), (numsurfaces, "numsurfaces")]
    );
}
