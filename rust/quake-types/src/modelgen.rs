//! MDL on-disk mirrors (`Quake/modelgen.h`). Compat-critical: the alias
//! loader in `quake-formats::mdl` reads these layouts field-by-field from
//! the file image at the same `offsetof`-derived byte offsets the C
//! `ReadLongUnaligned`/`ReadFloatUnaligned` calls in `model_parse.c` use.
//!
//! RA11 hazard -- `synctype_t` is defined **twice** in the engine, in
//! `modelgen.h` and in `spritegn.h`, both under the same `#ifndef
//! SYNCTYPE_T` guard, and the two definitions are *not* equal:
//! modelgen.h's has `ST_SYNC, ST_RAND, ST_FRAMETIME`, spritegn.h's only
//! `ST_SYNC, ST_RAND`. Whichever header a translation unit includes first
//! wins, so C code can see either enum. It never changes the ABI (both are
//! int-sized, and the shared values agree), and neither loader validates
//! the field -- it is copied straight into `qmodel_t.synctype` as an int.
//! So this crate defines the union of the two, once, here, and
//! `crate::spritegn` re-exports it rather than declaring a second copy.

use core::ffi::c_char;

pub const ALIAS_VERSION: i32 = 6;
pub const ALIAS_ONSEAM: i32 = 0x0020;
pub const DT_FACES_FRONT: i32 = 0x0010;
/// little-endian "IDPO"
pub const IDPOLYHEADER: i32 =
    (b'O' as i32) << 24 | (b'P' as i32) << 16 | (b'D' as i32) << 8 | b'I' as i32;

/// `synctype_t` -- the union of the two C definitions; see the module note.
pub const ST_SYNC: i32 = 0;
pub const ST_RAND: i32 = 1;
/// modelgen.h only: sync to when `.frame` changes
pub const ST_FRAMETIME: i32 = 2;

/// `aliasframetype_t`
pub const ALIAS_SINGLE: i32 = 0;
pub const ALIAS_GROUP: i32 = 1;

/// `aliasskintype_t`
pub const ALIAS_SKIN_SINGLE: i32 = 0;
pub const ALIAS_SKIN_GROUP: i32 = 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MdlT {
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
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct StVert {
    pub onseam: i32,
    pub s: i32,
    pub t: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DTriangle {
    pub facesfront: i32,
    pub vertindex: [i32; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TriVertX {
    pub v: [u8; 3],
    pub lightnormalindex: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DAliasFrame {
    /// lightnormal isn't used
    pub bboxmin: TriVertX,
    /// lightnormal isn't used
    pub bboxmax: TriVertX,
    /// frame name from grabbing
    pub name: [c_char; 16],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DAliasGroup {
    pub numframes: i32,
    pub bboxmin: TriVertX,
    pub bboxmax: TriVertX,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DAliasSkinGroup {
    pub numskins: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DAliasInterval {
    pub interval: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DAliasSkinInterval {
    pub interval: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DAliasFrameType {
    /// `aliasframetype_t`
    pub type_: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DAliasSkinType {
    /// `aliasskintype_t`
    pub type_: i32,
}

use std::mem::{offset_of, size_of};

const _: () = assert!(size_of::<MdlT>() == 84);
const _: () = assert!(offset_of!(MdlT, scale) == 8);
const _: () = assert!(offset_of!(MdlT, scale_origin) == 20);
const _: () = assert!(offset_of!(MdlT, boundingradius) == 32);
const _: () = assert!(offset_of!(MdlT, eyeposition) == 36);
const _: () = assert!(offset_of!(MdlT, numskins) == 48);
const _: () = assert!(offset_of!(MdlT, numverts) == 60);
const _: () = assert!(offset_of!(MdlT, numtris) == 64);
const _: () = assert!(offset_of!(MdlT, numframes) == 68);
const _: () = assert!(offset_of!(MdlT, synctype) == 72);
const _: () = assert!(offset_of!(MdlT, flags) == 76);
const _: () = assert!(offset_of!(MdlT, size) == 80);
const _: () = assert!(size_of::<StVert>() == 12);
const _: () = assert!(size_of::<DTriangle>() == 16);
const _: () = assert!(offset_of!(DTriangle, vertindex) == 4);
const _: () = assert!(size_of::<TriVertX>() == 4);
const _: () = assert!(size_of::<DAliasFrame>() == 24);
const _: () = assert!(offset_of!(DAliasFrame, name) == 8);
const _: () = assert!(size_of::<DAliasGroup>() == 12);
const _: () = assert!(offset_of!(DAliasGroup, bboxmin) == 4);
const _: () = assert!(size_of::<DAliasSkinGroup>() == 4);
const _: () = assert!(size_of::<DAliasInterval>() == 4);
const _: () = assert!(size_of::<DAliasSkinInterval>() == 4);
const _: () = assert!(size_of::<DAliasFrameType>() == 4);
const _: () = assert!(size_of::<DAliasSkinType>() == 4);
