//! `cargo xtask`: the Meson shader/pak pipeline steps as commands, for the
//! byte-identity differential against the C tools' outputs
//! (scripts/harness/xtask_diff.py) and for inspecting what the quake-capi
//! build script embeds.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use xtask::shaders::{Options, Tools};

const USAGE: &str = "usage:
  cargo xtask shaders --out DIR [--debug] [--no-spirv-opt] [--c]
      compile every meson.build shader job into DIR/<name>.spv; --c also
      writes the bintoc DIR/<name>.c files
  cargo xtask pak --out DIR [--depfile FILE]
      write DIR/vkquake.pak (mkpak) and DIR/embedded_pak.c (bintoc -c)
  cargo xtask mkpak OUTPUT ROOT TOC [DEPFILE]
  cargo xtask bintoc [-c] INPUT SYMBOL OUTPUT";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("shaders") => shaders(&args[1..]),
        Some("pak") => pak(&args[1..]),
        Some("mkpak") => mkpak(&args[1..]),
        Some("bintoc") => bintoc(&args[1..]),
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xtask: {message}");
            ExitCode::FAILURE
        }
    }
}

fn option_value(args: &[String], name: &str) -> Result<Option<PathBuf>, String> {
    match args.iter().position(|a| a == name) {
        Some(i) => args
            .get(i + 1)
            .map(|v| Some(PathBuf::from(v)))
            .ok_or_else(|| format!("{name} needs a value\n{USAGE}")),
        None => Ok(None),
    }
}

fn shaders(args: &[String]) -> Result<(), String> {
    let out = option_value(args, "--out")?.ok_or_else(|| USAGE.to_string())?;
    let debug = args.iter().any(|a| a == "--debug");
    let options = Options {
        debug,
        spirv_opt: !(debug
            || cfg!(target_os = "macos")
            || args.iter().any(|a| a == "--no-spirv-opt")),
    };
    let tools = Tools::find(options)?;
    if !tools.spirv_opt_has_canonicalize_ids()? {
        return Err(
            "spirv-opt does not support --canonicalize-ids, a newer Vulkan SDK is required".into(),
        );
    }
    let root = xtask::repo_root();
    let spvs = xtask::shaders::compile_all(&root, &out, options, &tools)?;
    if args.iter().any(|a| a == "--c") {
        for (job, spv) in xtask::shaders::jobs().iter().zip(&spvs) {
            let c = out.join(format!("{}.c", job.name));
            xtask::bintoc::convert(spv, &format!("{}_spv", job.name), &c, false)?;
        }
    }
    println!("xtask: {} shaders in {}", spvs.len(), out.display());
    Ok(())
}

fn pak(args: &[String]) -> Result<(), String> {
    let out = option_value(args, "--out")?.ok_or_else(|| USAGE.to_string())?;
    let depfile = option_value(args, "--depfile")?;
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let root = xtask::repo_root().join("Misc").join("vq_pak");
    let pak = out.join("vkquake.pak");
    xtask::pak::build(
        &pak,
        &root,
        &root.join("vq_pak_contents.txt"),
        depfile.as_deref(),
    )?;
    xtask::bintoc::convert(&pak, "vkquake.pak", &out.join("embedded_pak.c"), true)?;
    println!("xtask: embedded pak in {}", out.display());
    Ok(())
}

fn mkpak(args: &[String]) -> Result<(), String> {
    let [output, root, toc, rest @ ..] = args else {
        return Err(USAGE.to_string());
    };
    let depfile = match rest {
        [] => None,
        [depfile] => Some(Path::new(depfile)),
        _ => return Err(USAGE.to_string()),
    };
    xtask::pak::build(Path::new(output), Path::new(root), Path::new(toc), depfile)
}

fn bintoc(args: &[String]) -> Result<(), String> {
    let (compress, rest) = match args {
        [flag, rest @ ..] if flag == "-c" => (true, rest),
        _ => (false, args),
    };
    let [input, symbol, output] = rest else {
        return Err(USAGE.to_string());
    };
    xtask::bintoc::convert(Path::new(input), symbol, Path::new(output), compress)
}
