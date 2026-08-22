//! BSP on-disk mirrors (`Quake/bspfile.h`). Compat-critical: the loaders in
//! `quake-formats::bsp` parse these layouts field-by-field from the file
//! image, and the byte offsets must match the C `offsetof`-based readers in
//! `model_parse.c` exactly (5 dialects: BSP29, BSP30/Valve, 2PSB, BSP2, Q64).

use core::ffi::c_char;

pub const BSPVERSION: i32 = 29;
/// Half-Life map support (`BSP29_VALVE` is unconditionally defined)
pub const BSPVERSION_VALVE: i32 = 30;
/// RMQ support (2PSB). 32 bits instead of shorts for all but bbox sizes
pub const BSP2VERSION_2PSB: i32 =
    (b'B' as i32) << 24 | (b'S' as i32) << 16 | (b'P' as i32) << 8 | b'2' as i32;
/// BSP2 support. 32 bits instead of shorts for everything (bboxes use floats)
pub const BSP2VERSION_BSP2: i32 =
    (b'B' as i32) | (b'S' as i32) << 8 | (b'P' as i32) << 16 | (b'2' as i32) << 24;
pub const BSPVERSION_QUAKE64: i32 =
    (b'Q' as i32) << 24 | (b'6' as i32) << 16 | (b'4' as i32) << 8 | b' ' as i32;

pub const MAX_MAP_HULLS: usize = 4;

pub const LUMP_ENTITIES: usize = 0;
pub const LUMP_PLANES: usize = 1;
pub const LUMP_TEXTURES: usize = 2;
pub const LUMP_VERTEXES: usize = 3;
pub const LUMP_VISIBILITY: usize = 4;
pub const LUMP_NODES: usize = 5;
pub const LUMP_TEXINFO: usize = 6;
pub const LUMP_FACES: usize = 7;
pub const LUMP_LIGHTING: usize = 8;
pub const LUMP_CLIPNODES: usize = 9;
pub const LUMP_LEAFS: usize = 10;
pub const LUMP_MARKSURFACES: usize = 11;
pub const LUMP_EDGES: usize = 12;
pub const LUMP_SURFEDGES: usize = 13;
pub const LUMP_MODELS: usize = 14;
pub const HEADER_LUMPS: usize = 15;

pub const PLANE_X: i32 = 0;
pub const PLANE_Y: i32 = 1;
pub const PLANE_Z: i32 = 2;
pub const PLANE_ANYX: i32 = 3;
pub const PLANE_ANYY: i32 = 4;
pub const PLANE_ANYZ: i32 = 5;

pub const CONTENTS_EMPTY: i32 = -1;
pub const CONTENTS_SOLID: i32 = -2;

pub const TEX_SPECIAL: i32 = 1;
pub const TEX_MISSING: i32 = 2;

pub const MIPLEVELS: usize = 4;
pub const MAXLIGHTMAPS: usize = 4;
pub const NUM_AMBIENTS: usize = 4;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LumpT {
    pub fileofs: i32,
    pub filelen: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DHeader {
    pub version: i32,
    pub lumps: [LumpT; HEADER_LUMPS],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DModel {
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub origin: [f32; 3],
    pub headnode: [i32; MAX_MAP_HULLS],
    /// not including the solid leaf 0
    pub visleafs: i32,
    pub firstface: i32,
    pub numfaces: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DMipTexLump {
    pub nummiptex: i32,
    /// really [nummiptex]
    pub dataofs: [i32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MipTex {
    pub name: [c_char; 16],
    pub width: u32,
    pub height: u32,
    pub offsets: [u32; MIPLEVELS],
}

/// Quake64 variant: extra `shift` between height and offsets
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MipTex64 {
    pub name: [c_char; 16],
    pub width: u32,
    pub height: u32,
    pub shift: u32,
    pub offsets: [u32; MIPLEVELS],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DVertex {
    pub point: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DPlane {
    pub normal: [f32; 3],
    pub dist: f32,
    pub type_: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DSNode {
    pub planenum: i32,
    /// negative numbers are -(leafs+1), not nodes
    pub children: [i16; 2],
    pub mins: [i16; 3],
    pub maxs: [i16; 3],
    pub firstface: u16,
    pub numfaces: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DL1Node {
    pub planenum: i32,
    pub children: [i32; 2],
    pub mins: [i16; 3],
    pub maxs: [i16; 3],
    pub firstface: u32,
    pub numfaces: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DL2Node {
    pub planenum: i32,
    pub children: [i32; 2],
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub firstface: u32,
    pub numfaces: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DSClipnode {
    pub planenum: i32,
    /// negative numbers are contents
    pub children: [i16; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DLClipnode {
    pub planenum: i32,
    pub children: [i32; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TexInfo {
    /// [s/t][xyz offset]
    pub vecs: [[f32; 4]; 2],
    pub miptex: i32,
    pub flags: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DSEdge {
    pub v: [u16; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DLEdge {
    pub v: [u32; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DSFace {
    pub planenum: i16,
    pub side: i16,
    pub firstedge: i32,
    pub numedges: i16,
    pub texinfo: i16,
    pub styles: [u8; MAXLIGHTMAPS],
    /// start of [numstyles*surfsize] samples
    pub lightofs: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DLFace {
    pub planenum: i32,
    pub side: i32,
    pub firstedge: i32,
    pub numedges: i32,
    pub texinfo: i32,
    pub styles: [u8; MAXLIGHTMAPS],
    pub lightofs: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DSLeaf {
    pub contents: i32,
    /// -1 = no visibility info
    pub visofs: i32,
    pub mins: [i16; 3],
    pub maxs: [i16; 3],
    pub firstmarksurface: u16,
    pub nummarksurfaces: u16,
    pub ambient_level: [u8; NUM_AMBIENTS],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DL1Leaf {
    pub contents: i32,
    pub visofs: i32,
    pub mins: [i16; 3],
    pub maxs: [i16; 3],
    pub firstmarksurface: u32,
    pub nummarksurfaces: u32,
    pub ambient_level: [u8; NUM_AMBIENTS],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DL2Leaf {
    pub contents: i32,
    pub visofs: i32,
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub firstmarksurface: u32,
    pub nummarksurfaces: u32,
    pub ambient_level: [u8; NUM_AMBIENTS],
}

use std::mem::{offset_of, size_of};

const _: () = assert!(size_of::<LumpT>() == 8);
const _: () = assert!(size_of::<DHeader>() == 124);
const _: () = assert!(size_of::<DModel>() == 64);
const _: () = assert!(offset_of!(DModel, headnode) == 36);
const _: () = assert!(size_of::<DMipTexLump>() == 20);
const _: () = assert!(size_of::<MipTex>() == 40);
const _: () = assert!(size_of::<MipTex64>() == 44);
const _: () = assert!(offset_of!(MipTex64, shift) == 24);
const _: () = assert!(size_of::<DVertex>() == 12);
const _: () = assert!(size_of::<DPlane>() == 20);
const _: () = assert!(size_of::<DSNode>() == 24);
const _: () = assert!(offset_of!(DSNode, firstface) == 20);
const _: () = assert!(size_of::<DL1Node>() == 32);
const _: () = assert!(offset_of!(DL1Node, firstface) == 24);
const _: () = assert!(size_of::<DL2Node>() == 44);
const _: () = assert!(offset_of!(DL2Node, firstface) == 36);
const _: () = assert!(size_of::<DSClipnode>() == 8);
const _: () = assert!(size_of::<DLClipnode>() == 12);
const _: () = assert!(size_of::<TexInfo>() == 40);
const _: () = assert!(size_of::<DSEdge>() == 4);
const _: () = assert!(size_of::<DLEdge>() == 8);
const _: () = assert!(size_of::<DSFace>() == 20);
const _: () = assert!(offset_of!(DSFace, texinfo) == 10);
const _: () = assert!(offset_of!(DSFace, lightofs) == 16);
const _: () = assert!(size_of::<DLFace>() == 28);
const _: () = assert!(offset_of!(DLFace, lightofs) == 24);
const _: () = assert!(size_of::<DSLeaf>() == 28);
const _: () = assert!(offset_of!(DSLeaf, firstmarksurface) == 20);
const _: () = assert!(offset_of!(DSLeaf, ambient_level) == 24);
const _: () = assert!(size_of::<DL1Leaf>() == 32);
const _: () = assert!(offset_of!(DL1Leaf, ambient_level) == 28);
const _: () = assert!(size_of::<DL2Leaf>() == 44);
const _: () = assert!(offset_of!(DL2Leaf, ambient_level) == 40);
