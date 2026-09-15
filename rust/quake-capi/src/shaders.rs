//! Phase 8 M11 (task plan D8, ADR-015): the SPIR-V blobs Meson's `bintoc`
//! custom targets used to compile into `<name>_spv` C arrays. `build.rs` runs
//! the xtask pipeline (glslangValidator + spirv-opt with `meson.build`'s
//! flags) and generates the `include_bytes!` table keyed by the bintoc symbol
//! stem, which is also [`quake_render::rmisc::shaders::Shader::name`].

include!(concat!(env!("OUT_DIR"), "/shaders_gen.rs"));
