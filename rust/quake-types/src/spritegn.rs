//! SPR on-disk mirrors (`Quake/spritegn.h`). Compat-critical: the sprite
//! loader in `quake-formats::spr` reads these layouts field-by-field from
//! the file image, at the byte offsets the C `LittleLong`/`LittleFloat`
//! struct reads in `model_parse.c` use.
//!
//! `synctype_t` is re-exported from [`crate::modelgen`] rather than
//! redefined -- spritegn.h's copy is the shorter of the two C definitions
//! that share the `SYNCTYPE_T` guard (RA11); see the note there.

pub use crate::modelgen::{ST_RAND, ST_SYNC};

pub const SPRITE_VERSION: i32 = 1;
/// little-endian "IDSP"
pub const IDSPRITEHEADER: i32 =
    (b'P' as i32) << 24 | (b'S' as i32) << 16 | (b'D' as i32) << 8 | b'I' as i32;

pub const SPR_VP_PARALLEL_UPRIGHT: i32 = 0;
pub const SPR_FACING_UPRIGHT: i32 = 1;
pub const SPR_VP_PARALLEL: i32 = 2;
pub const SPR_ORIENTED: i32 = 3;
pub const SPR_VP_PARALLEL_ORIENTED: i32 = 4;

/// `spriteframetype_t`
pub const SPR_SINGLE: i32 = 0;
pub const SPR_GROUP: i32 = 1;
pub const SPR_ANGLED: i32 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DSprite {
    pub ident: i32,
    pub version: i32,
    pub type_: i32,
    pub boundingradius: f32,
    pub width: i32,
    pub height: i32,
    pub numframes: i32,
    pub beamlength: f32,
    /// `synctype_t`
    pub synctype: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DSpriteFrame {
    pub origin: [i32; 2],
    pub width: i32,
    pub height: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DSpriteGroup {
    pub numframes: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DSpriteInterval {
    pub interval: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct DSpriteFrameType {
    /// `spriteframetype_t`
    pub type_: i32,
}

use std::mem::{offset_of, size_of};

const _: () = assert!(size_of::<DSprite>() == 36);
const _: () = assert!(offset_of!(DSprite, type_) == 8);
const _: () = assert!(offset_of!(DSprite, width) == 16);
const _: () = assert!(offset_of!(DSprite, height) == 20);
const _: () = assert!(offset_of!(DSprite, numframes) == 24);
const _: () = assert!(offset_of!(DSprite, synctype) == 32);
const _: () = assert!(size_of::<DSpriteFrame>() == 16);
const _: () = assert!(offset_of!(DSpriteFrame, width) == 8);
const _: () = assert!(offset_of!(DSpriteFrame, height) == 12);
const _: () = assert!(size_of::<DSpriteGroup>() == 4);
const _: () = assert!(size_of::<DSpriteInterval>() == 4);
const _: () = assert!(size_of::<DSpriteFrameType>() == 4);
