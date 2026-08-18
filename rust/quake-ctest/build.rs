//! Compiles the original C implementations (renamed to c_ref_*) into the
//! differential test binaries. The C sources are compiled straight out of
//! Quake/ while they exist; the Phase 1 deletion step repoints this at frozen
//! copies under csrc/.

use std::path::PathBuf;

const C_SOURCES: &[&str] = &[
    "Quake/crc.c",
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
        .include(repo_root.join("Quake"));

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
    println!("cargo:rerun-if-changed={}", prelude.display());

    build.compile("quake_c_ref");
}
