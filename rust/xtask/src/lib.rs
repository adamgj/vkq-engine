//! Build helper tasks (Phase 8 M11, task plan D8 -- ADR-015): the shader
//! pipeline and the embedded pak that `meson.build` otherwise produces with
//! `glslangValidator`/`spirv-opt` plus the `bintoc` and `mkpak` C tools.
//!
//! `quake-capi`'s build script drives [`shaders`], [`pak`] and [`bintoc`]
//! in-process under its `render` feature; the `cargo xtask` binary exposes
//! the same steps so `scripts/harness/xtask_diff.py` can compare their
//! outputs byte for byte against the Meson C-build's. [`engine`] adds the
//! cross-platform `cargo xtask build` / `cargo xtask run` wrappers over Meson.

pub mod bintoc;
pub mod engine;
pub mod pak;
pub mod shaders;

use std::path::{Path, PathBuf};

/// The repository root (`rust/xtask` is two levels below it).
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("xtask lives inside the repository")
        .to_path_buf()
}
