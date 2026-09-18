//! `cargo xtask`: the Meson shader/pak pipeline steps as commands, for the
//! byte-identity differential against the C tools' outputs
//! (scripts/harness/xtask_diff.py) and for inspecting what the quake-capi
//! build script embeds; plus `build`/`run`, which drive the Meson engine
//! build and launch the result with one command line on every platform.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use xtask::engine::BuildOptions;
use xtask::shaders::{Options, Tools};

const USAGE: &str = "usage:
  cargo xtask build [BUILD-OPTIONS] [-- MESON-SETUP-ARGS...]
      configure (first time, or with --reconfigure) and compile the engine
      with Meson; the executable lands in the build directory
  cargo xtask run [BUILD-OPTIONS] [--no-build] [--basedir DIR] [-- ENGINE-ARGS...]
      build, then launch vkqr-engine with ENGINE-ARGS; with --basedir DIR
      (else $QUAKE_GAME_DATA) it runs from DIR with `-basedir DIR`, otherwise
      the engine looks for game data itself (working directory, then the
      Steam/GOG/Epic store detection an explicit -basedir switches off)
    BUILD-OPTIONS:
      --build-dir DIR   Meson build directory (default: build, under the repo root)
      --debug           --buildtype=debug (default: release)
      --buildtype TYPE  any Meson buildtype
      --c-only          -Duse_rust=disabled (the C oracle; default: enabled,
                        except on a *-windows-gnu toolchain, which is C-only)
      --reconfigure     re-run meson setup --reconfigure on an existing directory
    the MESON environment variable overrides the `meson` program name; on an
    MSVC toolchain meson setup runs with CC=clang-cl unless CC is set, and
    with --vsenv unless a Visual Studio Developer shell is already active
    (PATH directories holding a stray cl.exe, which would make Meson skip
    the activation, are left out of the Meson commands' PATH)
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
        Some("build") => build(&args[1..]),
        Some("run") => run(&args[1..]),
        Some("shaders") => shaders(&args[1..]).map(|()| ExitCode::SUCCESS),
        Some("pak") => pak(&args[1..]).map(|()| ExitCode::SUCCESS),
        Some("mkpak") => mkpak(&args[1..]).map(|()| ExitCode::SUCCESS),
        Some("bintoc") => bintoc(&args[1..]).map(|()| ExitCode::SUCCESS),
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(code) => code,
        Err(message) => {
            eprintln!("xtask: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Split `args` at the first `--`: the part before is ours, the rest is
/// passed through verbatim.
fn split_passthrough(args: &[String]) -> (&[String], &[String]) {
    match args.iter().position(|a| a == "--") {
        Some(i) => (&args[..i], &args[i + 1..]),
        None => (args, &[]),
    }
}

/// Parse the BUILD-OPTIONS shared by `build` and `run`; `extra` receives
/// the flags they do not know, for the caller to accept or reject.
fn build_options(args: &[String], extra: &mut Vec<String>) -> Result<BuildOptions, String> {
    let mut options = BuildOptions::default();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let mut value = |name: &str| {
            iter.next()
                .cloned()
                .ok_or_else(|| format!("{name} needs a value\n{USAGE}"))
        };
        match arg.as_str() {
            "--build-dir" => options.build_dir = PathBuf::from(value("--build-dir")?),
            "--buildtype" => options.buildtype = value("--buildtype")?,
            "--debug" => options.buildtype = "debug".into(),
            "--c-only" => options.use_rust = false,
            "--reconfigure" => options.reconfigure = true,
            _ => extra.push(arg.clone()),
        }
    }
    Ok(options)
}

fn build(args: &[String]) -> Result<ExitCode, String> {
    let (ours, setup_args) = split_passthrough(args);
    let mut unknown = Vec::new();
    let mut options = build_options(ours, &mut unknown)?;
    if let Some(flag) = unknown.first() {
        return Err(format!("unknown build option {flag}\n{USAGE}"));
    }
    options.setup_args = setup_args.to_vec();
    let exe = xtask::engine::build(&xtask::repo_root(), &options)?;
    println!("xtask: built {}", exe.display());
    Ok(ExitCode::SUCCESS)
}

fn run(args: &[String]) -> Result<ExitCode, String> {
    let (ours, engine_args) = split_passthrough(args);
    let mut rest = Vec::new();
    let options = build_options(ours, &mut rest)?;
    let mut no_build = false;
    let mut basedir = None;
    let mut iter = rest.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--no-build" => no_build = true,
            "--basedir" => {
                let dir = iter
                    .next()
                    .ok_or_else(|| format!("--basedir needs a value\n{USAGE}"))?;
                basedir = Some(PathBuf::from(dir));
            }
            flag => return Err(format!("unknown run option {flag}\n{USAGE}")),
        }
    }
    let root = xtask::repo_root();
    let exe = if no_build {
        let defaults = BuildOptions::default();
        if options.buildtype != defaults.buildtype
            || options.use_rust != defaults.use_rust
            || options.reconfigure
        {
            return Err(format!(
                "--no-build skips meson setup, so --debug/--buildtype/--c-only/--reconfigure have no effect; drop them or --no-build\n{USAGE}"
            ));
        }
        let build_dir = xtask::engine::resolve_dir(&root, &options.build_dir);
        let exe = xtask::engine::engine_path(&build_dir);
        if !exe.is_file() {
            return Err(format!(
                "{} does not exist; run without --no-build first",
                exe.display()
            ));
        }
        exe
    } else {
        xtask::engine::build(&root, &options)?
    };
    let basedir = xtask::engine::resolve_basedir(basedir.as_deref(), |name| std::env::var_os(name));
    let status = xtask::engine::run(&exe, basedir.as_deref(), engine_args)?;
    Ok(match status.code() {
        Some(0) => ExitCode::SUCCESS,
        Some(code) => {
            eprintln!("xtask: vkqr-engine exited with status {code}");
            // keep a nonzero status nonzero after the u8 truncation
            ExitCode::from(u8::try_from(code.rem_euclid(256)).map_or(1, |c| c.max(1)))
        }
        None => {
            eprintln!("xtask: vkqr-engine was terminated by a signal");
            ExitCode::FAILURE
        }
    })
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
