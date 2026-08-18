//! WAD2 on-disk/ABI mirrors (`Quake/wad.h`). Compat-critical: the C renderer
//! does pointer arithmetic against `wad_base` and dereferences `lumpinfo_t`
//! entries that live *inside* the loaded file image (in-place byteswapped).

use core::ffi::c_char;

pub const CMP_NONE: c_char = 0;
pub const TYP_QPIC: c_char = 66;

/// 'W' | 'A'<<8 | 'D'<<16 | '2'<<24
pub const WADID: i32 = 0x32444157;
/// WAD3 (Valve)
pub const WADID_VALVE: i32 = 0x33444157;

pub const WADFILENAME: &str = "gfx.wad";

#[repr(C)]
#[derive(Clone, Copy)]
pub struct QPic {
    pub width: i32,
    pub height: i32,
    /// variably sized
    pub data: [u8; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WadInfo {
    /// should be WAD2 or 2DAW
    pub identification: [c_char; 4],
    pub numlumps: i32,
    pub infotableofs: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LumpInfo {
    pub filepos: i32,
    pub disksize: i32,
    /// uncompressed
    pub size: i32,
    pub type_: c_char,
    pub compression: c_char,
    pub pad1: c_char,
    pub pad2: c_char,
    /// must be null terminated
    pub name: [c_char; 16],
}

const _: () = assert!(std::mem::size_of::<QPic>() == 12);
const _: () = assert!(std::mem::size_of::<WadInfo>() == 12);
const _: () = assert!(std::mem::size_of::<LumpInfo>() == 32);
const _: () = assert!(std::mem::offset_of!(LumpInfo, filepos) == 0);
const _: () = assert!(std::mem::offset_of!(LumpInfo, disksize) == 4);
const _: () = assert!(std::mem::offset_of!(LumpInfo, size) == 8);
const _: () = assert!(std::mem::offset_of!(LumpInfo, type_) == 12);
const _: () = assert!(std::mem::offset_of!(LumpInfo, name) == 16);
