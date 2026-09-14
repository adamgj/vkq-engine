//! Phase 8 M11 (task plan D8): `embedded_pak.c`, the raw-deflated
//! `vkquake.pak` Meson generated with `mkpak` + `bintoc -c`. `build.rs` builds
//! the pak from `Misc/vq_pak/vq_pak_contents.txt` and deflates it with the
//! same miniz flags; the three symbols keep the C names because
//! `common_fs.c` (and `fs.rs` through the c-sys externs) read them under
//! `-Duse_rust_render`, exactly as the C `bintoc` output did.

use core::ffi::c_int;

mod generated {
    include!(concat!(env!("OUT_DIR"), "/embedded_pak_gen.rs"));
}

/// `const unsigned char vkquake_pak[]`
#[no_mangle]
pub static vkquake_pak: [u8; generated::COMPRESSED.len()] = *generated::COMPRESSED;
/// `const int vkquake_pak_size`
#[no_mangle]
pub static vkquake_pak_size: c_int = generated::COMPRESSED.len() as c_int;
/// `const int vkquake_pak_decompressed_size`
#[no_mangle]
pub static vkquake_pak_decompressed_size: c_int = generated::DECOMPRESSED_SIZE as c_int;
