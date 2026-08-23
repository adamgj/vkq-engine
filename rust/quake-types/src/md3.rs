//! On-disk MD3 mirrors (`Quake/gl_model.h`, "MD3 MODELS" section).
//!
//! The engine reads these straight out of the loaded file image, so the
//! layouts are the file format. Every multi-byte scalar is little-endian on
//! disk and goes through `LittleLong`/`LittleShort` in the C loader; the
//! mirrors keep the native types and the decoders in `quake_formats::md3` do
//! the byte-swapping, so nothing here may be read by a plain struct copy on a
//! big-endian target.
//!
//! `md3XyzNormal_t` lives in [`crate::model_mem`] instead: the loader copies
//! it into engine memory unchanged and `Mod_CalcAliasBounds` reads it back
//! from there, so it is an in-memory type as much as an on-disk one.

use core::ffi::c_char;

pub const MD3_VERSION: i32 = 15;
/// `('I' << 0) | ('D' << 8) | ('P' << 16) | ('3' << 24)`
pub const IDMD3HEADER: i32 = 0x3350_4449;
/// `MD3_XYZ_SCALE`
pub const MD3_XYZ_SCALE: f32 = 1.0 / 64.0;

/// `md3Header_t`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Md3Header {
    pub ident: i32,
    pub version: i32,
    pub name: [c_char; 64],
    /// assumed to match quake1 models, for lack of somewhere better
    pub flags: i32,
    pub num_frames: i32,
    pub num_tags: i32,
    pub num_surfaces: i32,
    pub num_skins: i32,
    pub ofs_frames: i32,
    pub ofs_tags: i32,
    pub ofs_surfaces: i32,
    pub ofs_end: i32,
}

/// `md3Frame_t` — `header->numFrames` of these at `header->ofsFrames`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Md3Frame {
    pub bounds: [[f32; 3]; 2],
    pub local_origin: [f32; 3],
    pub radius: f32,
    pub name: [c_char; 16],
}

/// `md3Surface_t` — `header->numSurfaces` of these, chained by `ofsEnd`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Md3Surface {
    pub ident: i32,
    /// polyset name
    pub name: [c_char; 64],
    pub flags: i32,
    /// all surfaces in a model should have the same
    pub num_frames: i32,
    /// all surfaces in a model should have the same
    pub num_shaders: i32,
    pub num_verts: i32,
    pub num_triangles: i32,
    pub ofs_triangles: i32,
    /// offset from start of `md3Surface_t`
    pub ofs_shaders: i32,
    /// texture coords are common for all frames
    pub ofs_st: i32,
    /// `numVerts * numFrames`
    pub ofs_xyz_normals: i32,
    /// next surface follows
    pub ofs_end: i32,
}

/// `md3Triangle_t` — `surf->numTriangles` at `surf + surf->ofsTriangles`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Md3Triangle {
    pub indexes: [i32; 3],
}

/// `md3St_t` — `surf->numVerts` at `surf + surf->ofsSt`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Md3St {
    pub s: f32,
    pub t: f32,
}

/// `md3Shader_t`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Md3Shader {
    pub name: [c_char; 64],
    pub shader_index: i32,
}

mod layout_asserts {
    use super::*;
    use core::mem::{offset_of, size_of};

    const _: () = assert!(size_of::<Md3Header>() == 108);
    const _: () = assert!(offset_of!(Md3Header, name) == 8);
    const _: () = assert!(offset_of!(Md3Header, flags) == 72);
    const _: () = assert!(offset_of!(Md3Header, num_frames) == 76);
    const _: () = assert!(offset_of!(Md3Header, num_surfaces) == 84);
    const _: () = assert!(offset_of!(Md3Header, ofs_frames) == 92);
    const _: () = assert!(offset_of!(Md3Header, ofs_surfaces) == 100);
    const _: () = assert!(offset_of!(Md3Header, ofs_end) == 104);

    const _: () = assert!(size_of::<Md3Frame>() == 56);
    const _: () = assert!(offset_of!(Md3Frame, local_origin) == 24);
    const _: () = assert!(offset_of!(Md3Frame, radius) == 36);
    const _: () = assert!(offset_of!(Md3Frame, name) == 40);

    const _: () = assert!(size_of::<Md3Surface>() == 108);
    const _: () = assert!(offset_of!(Md3Surface, name) == 4);
    const _: () = assert!(offset_of!(Md3Surface, flags) == 68);
    const _: () = assert!(offset_of!(Md3Surface, num_frames) == 72);
    const _: () = assert!(offset_of!(Md3Surface, num_verts) == 80);
    const _: () = assert!(offset_of!(Md3Surface, num_triangles) == 84);
    const _: () = assert!(offset_of!(Md3Surface, ofs_triangles) == 88);
    const _: () = assert!(offset_of!(Md3Surface, ofs_st) == 96);
    const _: () = assert!(offset_of!(Md3Surface, ofs_xyz_normals) == 100);
    const _: () = assert!(offset_of!(Md3Surface, ofs_end) == 104);

    const _: () = assert!(size_of::<Md3Triangle>() == 12);
    const _: () = assert!(size_of::<Md3St>() == 8);
    const _: () = assert!(size_of::<Md3Shader>() == 68);
}
