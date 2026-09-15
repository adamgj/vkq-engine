//! Phase 9 M4: `pl_linux.c` embeds the window icon as `Quake/qs_bmp.h`, a
//! comma-separated hex byte list spliced into a C array initializer. Rust
//! cannot `include!` a bare list, so under the `platform` feature the same
//! header is parsed here into a binary blob that `src/pl/sdl.rs`
//! `include_bytes!`s -- one source of truth for both builds.

use std::path::{Path, PathBuf};

fn main() {
    if std::env::var_os("CARGO_FEATURE_PLATFORM").is_none() {
        return;
    }
    let bmp_h = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("quake-platform lives inside the repository")
        .join("Quake")
        .join("qs_bmp.h");
    println!("cargo:rerun-if-changed={}", bmp_h.display());
    let text = std::fs::read_to_string(&bmp_h).expect("Quake/qs_bmp.h");
    let bytes: Vec<u8> = text
        .split(',')
        .map(str::trim)
        .filter(|tok| !tok.is_empty())
        .map(|tok| {
            let hex = tok.strip_prefix("0x").expect("qs_bmp.h: hex byte literal");
            u8::from_str_radix(hex, 16).expect("qs_bmp.h: byte value")
        })
        .collect();
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR")).join("qs_bmp.bin");
    std::fs::write(&out, bytes).expect("write qs_bmp.bin");
}
