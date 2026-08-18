//! Compiles the original C implementations (renamed to c_ref_*) into the
//! differential test binaries. The C sources are compiled straight out of
//! Quake/ while they exist; the Phase 1 deletion step repoints this at frozen
//! copies under csrc/.

use std::path::PathBuf;

const C_SOURCES: &[&str] = &[
    "Quake/crc.c",
    "Quake/hash_map.c",
    "Quake/mathlib.c",
    "Quake/mdfour.c",
    "Quake/strlcpy.c",
    "Quake/strlcat.c",
];

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let repo_root = manifest.ancestors().nth(2).unwrap().to_path_buf();
    let prelude = manifest.join("include").join("c_ref_prelude.h");

    let mut build = cc::Build::new();
    build
        .include(manifest.join("include"))
        .include(repo_root.join("Quake"))
        // ADR-010: the engine pins -ffp-contract=off (meson.build) so C and
        // Rust agree on FMA behavior; the reference build must match. The
        // no-builtin flags stop clang fusing adjacent sinf/cosf into Apple's
        // __sincosf_stret, whose results differ from separate calls by 1 ulp
        // (renderer-only divergence, accepted in ADR-010's Phase 1 amendment).
        .flag_if_supported("-ffp-contract=off")
        .flag_if_supported("-fno-builtin-sinf")
        .flag_if_supported("-fno-builtin-cosf");

    if build.get_compiler().is_like_msvc() {
        build.flag(format!("/FI{}", prelude.display()));
    } else {
        build.flag("-include").flag(prelude.to_str().unwrap());
    }

    for src in C_SOURCES {
        let path = repo_root.join(src);
        println!("cargo:rerun-if-changed={}", path.display());
        build.file(path);
    }
    build.file(manifest.join("stubs").join("snprintf_oracle.c"));
    build.file(manifest.join("stubs").join("stubs.c"));
    build.file(manifest.join("stubs").join("anorms_ref.c"));
    build.file(manifest.join("stubs").join("hashers_ref.c"));
    println!("cargo:rerun-if-changed={}", prelude.display());
    println!(
        "cargo:rerun-if-changed={}",
        manifest.join("stubs").join("snprintf_oracle.c").display()
    );

    build.compile("quake_c_ref");
}
