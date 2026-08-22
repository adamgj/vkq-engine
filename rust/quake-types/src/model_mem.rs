//! In-memory model mirrors (`Quake/gl_model.h`). Compat-critical: the
//! Rust loaders in `quake-capi` fill these structs inside `Mem_Alloc`-backed
//! engine memory that the C renderer and server then walk directly, so every
//! field offset must match the C build's layout exactly. The layout is
//! verified per-platform by the ctest ABI probes (`tests/bsp_abi.rs` for the
//! brush half, `tests/alias_abi.rs` for the alias/sprite half); the const
//! asserts below pin the 64-bit layout at compile time.
//!
//! `PSET_SCRIPT` fields are included unconditionally — the engine defines
//! `PSET_SCRIPT` on every platform this mirror targets.
//!
//! The Vulkan fields of `QModel` (`blas`/`buffer`/`address`) and of
//! `AliasHdr` (the vertex/index/joints buffer block) are mirrored as
//! pointer-sized stand-ins; valid on 64-bit targets only, where every Vk
//! handle type is 8 bytes. Parsing never touches those fields.

use core::ffi::{c_char, c_void};

use crate::bspfile::{DModel, MAXLIGHTMAPS, MAX_MAP_HULLS, MIPLEVELS, NUM_AMBIENTS};
use crate::modelgen::TriVertX;
use crate::plane::MPlane;

pub const MAX_QPATH: usize = 64;
/// `MAX_DLIGHTS` (64) packed into 32-bit words
pub const DLIGHT_WORDS: usize = 2;
/// `chain_num` from the `texchain_t` enum
pub const CHAIN_NUM: usize = 9;
/// `TEXTYPE_COUNT` from the `textype_t` enum
pub const TEXTYPE_COUNT: usize = 7;
/// `PV_SIZE` extradata slots
pub const PV_SIZE: usize = 4;
pub const MAX_SKINS: usize = 32;
pub const MAX_FRAMEGROUPS: usize = 4;

/// `poseverttype_t`
pub const PV_QUAKE1: i32 = 0;
pub const PV_MD5: i32 = 1;
pub const PV_MD5_8: i32 = 2;
pub const PV_QUAKE3: i32 = 3;

pub const MAXALIASVERTS_QS: i32 = 2000;
pub const MAXALIASTRIS_QS: i32 = 4096;
pub const MAXALIASVERTS: i32 = 0x7fff;
pub const MAXALIASFRAMES: i32 = 2048;

/// MD3 scale is a constant
pub const MD3_XYZ_SCALE: f32 = 1.0 / 64.0;

pub const TEXTYPE_DEFAULT: i32 = 0;
pub const TEXTYPE_CUTOUT: i32 = 1;
pub const TEXTYPE_SKY: i32 = 2;
pub const TEXTYPE_LAVA: i32 = 3;
pub const TEXTYPE_SLIME: i32 = 4;
pub const TEXTYPE_TELE: i32 = 5;
pub const TEXTYPE_WATER: i32 = 6;

pub const MOD_BRUSH: i32 = 0;
pub const MOD_SPRITE: i32 = 1;
pub const MOD_ALIAS: i32 = 2;

pub const SURF_PLANEBACK: i32 = 2;
pub const SURF_DRAWSKY: i32 = 4;
pub const SURF_DRAWSPRITE: i32 = 8;
pub const SURF_DRAWTURB: i32 = 0x10;
pub const SURF_DRAWTILED: i32 = 0x20;
pub const SURF_DRAWBACKGROUND: i32 = 0x40;
pub const SURF_UNDERWATER: i32 = 0x80;
pub const SURF_NOTEXTURE: i32 = 0x100;
pub const SURF_DRAWFENCE: i32 = 0x200;
pub const SURF_DRAWLAVA: i32 = 0x400;
pub const SURF_DRAWSLIME: i32 = 0x800;
pub const SURF_DRAWTELE: i32 = 0x1000;
pub const SURF_DRAWWATER: i32 = 0x2000;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MVertex {
    pub position: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MEdge {
    pub v: [u32; 2],
    pub cachededgeoffset: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Texture {
    pub name: [c_char; 16],
    pub width: u32,
    pub height: u32,
    /// Q64
    pub shift: u32,
    /// `textype_t`
    pub type_: i32,
    pub source_file: [c_char; MAX_QPATH],
    /// `src_offset_t` (uintptr_t)
    pub source_offset: usize,
    pub gltexture: *mut c_void,
    pub fullbright: *mut c_void,
    pub warpimage: *mut c_void,
    /// `atomic_uint32_t` — a 4-byte atomic/volatile u32 on every target
    pub update_warp: u32,
    pub texturechains: [*mut MSurface; CHAIN_NUM],
    pub chain_size: [u32; CHAIN_NUM],
    pub anim_total: i32,
    pub anim_min: i32,
    pub anim_max: i32,
    pub anim_next: *mut Texture,
    pub alternate_anims: *mut Texture,
    pub offsets: [u32; MIPLEVELS],
    pub palette: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MTexInfo {
    pub vecs: [[f32; 4]; 2],
    pub texture: *mut Texture,
    pub flags: i32,
    pub tex_idx: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MSurface {
    pub visframe: i32,
    pub plane: *mut MPlane,
    pub flags: i32,
    /// negative numbers are backwards edges
    pub firstedge: i32,
    pub numedges: i32,
    pub texturemins: [i16; 2],
    pub extents: [i16; 2],
    pub light_s: i32,
    pub light_t: i32,
    /// `glpoly_t *`
    pub polys: *mut c_void,
    pub texturechains: [*mut MSurface; CHAIN_NUM],
    pub texinfo: *mut MTexInfo,
    pub indirect_idx: i32,
    pub vbo_firstvert: i32,
    pub dlightframe: i32,
    pub dlightbits: [u32; DLIGHT_WORDS],
    pub lightmaptexturenum: i32,
    pub styles: [u8; MAXLIGHTMAPS],
    pub styles_bitmap: u32,
    pub cached_light: [i32; MAXLIGHTMAPS],
    pub cached_dlight: bool,
    /// [numstyles*surfsize]
    pub samples: *mut u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MNode {
    /// 0, to differentiate from leafs
    pub contents: i32,
    pub minmaxs: [f32; 6],
    pub firstsurface: u32,
    pub numsurfaces: u32,
    pub plane: *mut MPlane,
    pub children: [*mut MNode; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MLeaf {
    /// will be a negative contents number
    pub contents: i32,
    pub minmaxs: [f32; 6],
    pub nummarksurfaces: i32,
    pub combined_deps: i32,
    pub ambient_sound_level: [u8; NUM_AMBIENTS],
    pub compressed_vis: *mut u8,
    pub firstmarksurface: *mut i32,
    /// `efrag_t *`
    pub efrags: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MClipnode {
    pub planenum: i32,
    /// negative numbers are contents
    pub children: [i32; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Hull {
    pub clipnodes: *mut MClipnode,
    pub planes: *mut MPlane,
    pub firstclipnode: i32,
    pub lastclipnode: i32,
    pub clip_mins: [f32; 3],
    pub clip_maxs: [f32; 3],
}

/// `soa_aabb_t`: 8 AABBs in SoA form
pub type SoaAabb = [f32; 2 * 3 * 8];
/// `soa_plane_t`: 8 planes in SoA form
pub type SoaPlane = [f32; 4 * 8];

#[repr(C)]
#[derive(Clone, Copy)]
pub struct QModel {
    pub name: [c_char; MAX_QPATH],
    pub is_worldmodel: bool,
    pub path_id: u32,
    pub needload: bool,
    /// `modtype_t`
    pub type_: i32,
    pub numframes: i32,
    /// `synctype_t`
    pub synctype: i32,
    pub flags: i32,
    // PSET_SCRIPT block (always compiled in)
    pub emiteffect: i32,
    pub traileffect: i32,
    /// `struct skytris_s *`
    pub skytris: *mut c_void,
    /// `struct skytriblock_s *`
    pub skytrimem: *mut c_void,
    pub skytime: f64,
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub ymins: [f32; 3],
    pub ymaxs: [f32; 3],
    pub rmins: [f32; 3],
    pub rmaxs: [f32; 3],
    pub clipbox: bool,
    pub clipmins: [f32; 3],
    pub clipmaxs: [f32; 3],
    pub firstmodelsurface: i32,
    pub nummodelsurfaces: i32,
    pub numsubmodels: i32,
    pub submodels: *mut DModel,
    pub numplanes: i32,
    pub planes: *mut MPlane,
    /// number of visible leafs, not counting 0
    pub numleafs: i32,
    pub leafs: *mut MLeaf,
    pub numvertexes: i32,
    pub vertexes: *mut MVertex,
    pub numedges: i32,
    pub edges: *mut MEdge,
    pub numnodes: i32,
    pub nodes: *mut MNode,
    pub numtexinfo: i32,
    pub texinfo: *mut MTexInfo,
    pub numsurfaces: i32,
    pub surfaces: *mut MSurface,
    pub numsurfedges: i32,
    pub surfedges: *mut i32,
    pub numclipnodes: i32,
    pub clipnodes: *mut MClipnode,
    pub nummarksurfaces: i32,
    pub marksurfaces: *mut i32,
    pub soa_leafbounds: *mut SoaAabb,
    pub surfvis: *mut u8,
    pub soa_surfplanes: *mut SoaPlane,
    pub hulls: [Hull; MAX_MAP_HULLS],
    pub numtextures: i32,
    pub textures: *mut *mut Texture,
    pub texofs: [i32; TEXTYPE_COUNT + 1],
    pub usedtextures: *mut i32,
    pub visdata: *mut u8,
    pub lightdata: *mut u8,
    pub entities: *mut c_char,
    /// for Mod_DecompressVis()
    pub viswarn: bool,
    pub bogus_tree: bool,
    pub bspversion: i32,
    pub contentstransparent: i32,
    pub combined_deps: i32,
    pub used_specials: i32,
    pub water_surfs: *mut i32,
    pub used_water_surfs: i32,
    pub water_surfs_specials: i32,
    /// only access through Mod_Extradata
    pub extradata: [*mut u8; PV_SIZE],
    // Ray tracing tail — Vk handle stand-ins, 64-bit targets only
    /// `VkAccelerationStructureKHR`
    pub blas: *mut c_void,
    /// `VkBuffer`
    pub buffer: *mut c_void,
    /// `VkDeviceAddress`
    pub address: u64,
}

// ---------------------------------------------------------------------------
// Sprite models

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MSpriteFrame {
    pub width: i32,
    pub height: i32,
    pub up: f32,
    pub down: f32,
    pub left: f32,
    pub right: f32,
    pub smax: f32,
    pub tmax: f32,
    /// `struct gltexture_s *`
    pub gltexture: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MSpriteGroup {
    pub numframes: i32,
    pub intervals: *mut f32,
    /// variable sized
    pub frames: [*mut MSpriteFrame; 1],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MSpriteFrameDesc {
    /// `spriteframetype_t`
    pub type_: i32,
    pub frameptr: *mut MSpriteFrame,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MSprite {
    pub type_: i32,
    pub maxwidth: i32,
    pub maxheight: i32,
    pub numframes: i32,
    /// variable sized
    pub frames: [MSpriteFrameDesc; 1],
}

// ---------------------------------------------------------------------------
// Alias models

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MAliasFrameDesc {
    pub firstpose: i32,
    pub numposes: i32,
    pub interval: f32,
    pub bboxmin: TriVertX,
    pub bboxmax: TriVertX,
    pub frame: i32,
    pub name: [c_char; 16],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MTriangle {
    pub facesfront: i32,
    pub vertindex: [i32; 3],
}

/// `md5vert_t` (MD5, up to 4 influences). Only `xyz` is read by the parsing
/// seam (`Mod_CalcAliasBounds`); the rest is here to pin the stride.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Md5Vert {
    pub xyz: [f32; 3],
    pub norm: [f32; 3],
    pub st: [f32; 2],
    pub joint_weights: [u8; 4],
    pub joint_indices: [u8; 4],
    pub joint_position_x: [f32; 4],
    pub joint_position_y: [f32; 4],
    pub joint_position_z: [f32; 4],
}

/// `md5vert8_t` (MD5, up to 8 influences)
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Md5Vert8 {
    pub xyz: [f32; 3],
    pub norm: [f32; 3],
    pub st: [f32; 2],
    pub joint_weights: [u8; 8],
    pub joint_indices: [u8; 8],
    pub joint_position_x: [f32; 8],
    pub joint_position_y: [f32; 8],
    pub joint_position_z: [f32; 8],
}

/// `md3XyzNormal_t`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Md3XyzNormal {
    pub xyz: [i16; 3],
    pub latlong: [u8; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AliasHdr {
    pub ident: i32,
    pub version: i32,
    pub scale: [f32; 3],
    pub scale_origin: [f32; 3],
    pub boundingradius: f32,
    pub eyeposition: [f32; 3],
    pub numskins: i32,
    pub skinwidth: i32,
    pub skinheight: i32,
    pub numverts: i32,
    pub numtris: i32,
    pub numframes: i32,
    /// `synctype_t`
    pub synctype: i32,
    pub flags: i32,
    pub size: f32,
    pub numindexes: i32,
    pub numverts_vbo: i32,
    pub numposes: i32,
    pub nextsurface: *mut AliasHdr,
    /// for md5
    pub numjoints: i32,
    /// `poseverttype_t`
    pub poseverttype: i32,
    /// `struct gltexture_s *`
    pub gltextures: [[*mut c_void; MAX_FRAMEGROUPS]; MAX_SKINS],
    /// `struct gltexture_s *`
    pub fbtextures: [[*mut c_void; MAX_FRAMEGROUPS]; MAX_SKINS],
    /// only for player skins
    pub texels: [*mut u8; MAX_SKINS],
    // Vk handle stand-ins, 64-bit targets only; parsing never touches these
    /// `VkBuffer`
    pub vertex_buffer: *mut c_void,
    /// `glheapallocation_t *`
    pub vertex_allocation: *mut c_void,
    /// `VkDeviceAddress`
    pub vertex_buffer_address: u64,
    /// `VkBuffer`
    pub index_buffer: *mut c_void,
    /// `glheapallocation_t *`
    pub index_allocation: *mut c_void,
    /// `VkDeviceAddress`
    pub index_buffer_address: u64,
    /// offset in vbo of hdr->numverts_vbo meshst_t
    pub vbostofs: i32,
    /// `VkBuffer`
    pub joints_buffer: *mut c_void,
    /// `glheapallocation_t *`
    pub joints_allocation: *mut c_void,
    /// `VkDeviceAddress`
    pub joints_buffer_address: u64,
    /// `VkDescriptorSet`
    pub joints_set: *mut c_void,
    /// variable sized
    pub frames: [MAliasFrameDesc; 1],
}

#[cfg(target_pointer_width = "64")]
mod layout_asserts {
    use super::*;
    use std::mem::{offset_of, size_of};

    const _: () = assert!(size_of::<MVertex>() == 12);
    const _: () = assert!(size_of::<MEdge>() == 12);
    const _: () = assert!(size_of::<MClipnode>() == 12);
    const _: () = assert!(size_of::<Hull>() == 48);
    const _: () = assert!(size_of::<MTexInfo>() == 48);
    const _: () = assert!(offset_of!(MTexInfo, texture) == 32);
    const _: () = assert!(size_of::<MNode>() == 64);
    const _: () = assert!(offset_of!(MNode, plane) == 40);
    const _: () = assert!(offset_of!(MNode, children) == 48);
    const _: () = assert!(size_of::<MLeaf>() == 64);
    const _: () = assert!(offset_of!(MLeaf, compressed_vis) == 40);
    const _: () = assert!(offset_of!(MLeaf, efrags) == 56);
    const _: () = assert!(size_of::<MSurface>() == 200);
    const _: () = assert!(offset_of!(MSurface, plane) == 8);
    const _: () = assert!(offset_of!(MSurface, polys) == 48);
    const _: () = assert!(offset_of!(MSurface, texinfo) == 128);
    const _: () = assert!(offset_of!(MSurface, samples) == 192);
    const _: () = assert!(size_of::<Texture>() == 296);
    const _: () = assert!(offset_of!(Texture, source_offset) == 96);
    const _: () = assert!(offset_of!(Texture, update_warp) == 128);
    const _: () = assert!(offset_of!(Texture, texturechains) == 136);
    const _: () = assert!(offset_of!(Texture, offsets) == 272);
    const _: () = assert!(offset_of!(Texture, palette) == 288);
    const _: () = assert!(size_of::<QModel>() == 800);
    const _: () = assert!(offset_of!(QModel, emiteffect) == 92);
    const _: () = assert!(offset_of!(QModel, skytime) == 120);
    const _: () = assert!(offset_of!(QModel, mins) == 128);
    const _: () = assert!(offset_of!(QModel, firstmodelsurface) == 228);
    const _: () = assert!(offset_of!(QModel, submodels) == 240);
    const _: () = assert!(offset_of!(QModel, hulls) == 432);
    const _: () = assert!(offset_of!(QModel, numtextures) == 624);
    const _: () = assert!(offset_of!(QModel, texofs) == 640);
    const _: () = assert!(offset_of!(QModel, visdata) == 680);
    const _: () = assert!(offset_of!(QModel, bspversion) == 708);
    const _: () = assert!(offset_of!(QModel, extradata) == 744);
    const _: () = assert!(offset_of!(QModel, blas) == 776);

    const _: () = assert!(size_of::<MSpriteFrame>() == 40);
    const _: () = assert!(offset_of!(MSpriteFrame, up) == 8);
    const _: () = assert!(offset_of!(MSpriteFrame, smax) == 24);
    const _: () = assert!(offset_of!(MSpriteFrame, gltexture) == 32);
    const _: () = assert!(size_of::<MSpriteGroup>() == 24);
    const _: () = assert!(offset_of!(MSpriteGroup, intervals) == 8);
    const _: () = assert!(offset_of!(MSpriteGroup, frames) == 16);
    const _: () = assert!(size_of::<MSpriteFrameDesc>() == 16);
    const _: () = assert!(offset_of!(MSpriteFrameDesc, frameptr) == 8);
    const _: () = assert!(size_of::<MSprite>() == 32);
    const _: () = assert!(offset_of!(MSprite, numframes) == 12);
    const _: () = assert!(offset_of!(MSprite, frames) == 16);

    const _: () = assert!(size_of::<MAliasFrameDesc>() == 40);
    const _: () = assert!(offset_of!(MAliasFrameDesc, interval) == 8);
    const _: () = assert!(offset_of!(MAliasFrameDesc, bboxmin) == 12);
    const _: () = assert!(offset_of!(MAliasFrameDesc, bboxmax) == 16);
    const _: () = assert!(offset_of!(MAliasFrameDesc, frame) == 20);
    const _: () = assert!(offset_of!(MAliasFrameDesc, name) == 24);
    const _: () = assert!(size_of::<MTriangle>() == 16);
    const _: () = assert!(size_of::<Md5Vert>() == 88);
    const _: () = assert!(size_of::<Md5Vert8>() == 144);
    const _: () = assert!(size_of::<Md3XyzNormal>() == 8);
    const _: () = assert!(size_of::<AliasHdr>() == 2544);
    const _: () = assert!(offset_of!(AliasHdr, scale) == 8);
    const _: () = assert!(offset_of!(AliasHdr, scale_origin) == 20);
    const _: () = assert!(offset_of!(AliasHdr, boundingradius) == 32);
    const _: () = assert!(offset_of!(AliasHdr, eyeposition) == 36);
    const _: () = assert!(offset_of!(AliasHdr, numskins) == 48);
    const _: () = assert!(offset_of!(AliasHdr, numverts) == 60);
    const _: () = assert!(offset_of!(AliasHdr, numtris) == 64);
    const _: () = assert!(offset_of!(AliasHdr, numframes) == 68);
    const _: () = assert!(offset_of!(AliasHdr, synctype) == 72);
    const _: () = assert!(offset_of!(AliasHdr, flags) == 76);
    const _: () = assert!(offset_of!(AliasHdr, size) == 80);
    const _: () = assert!(offset_of!(AliasHdr, numposes) == 92);
    const _: () = assert!(offset_of!(AliasHdr, nextsurface) == 96);
    const _: () = assert!(offset_of!(AliasHdr, poseverttype) == 108);
    const _: () = assert!(offset_of!(AliasHdr, gltextures) == 112);
    const _: () = assert!(offset_of!(AliasHdr, fbtextures) == 1136);
    const _: () = assert!(offset_of!(AliasHdr, texels) == 2160);
    const _: () = assert!(offset_of!(AliasHdr, vertex_buffer) == 2416);
    const _: () = assert!(offset_of!(AliasHdr, vbostofs) == 2464);
    const _: () = assert!(offset_of!(AliasHdr, joints_set) == 2496);
    const _: () = assert!(offset_of!(AliasHdr, frames) == 2504);
}
