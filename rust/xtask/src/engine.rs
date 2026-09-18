//! `cargo xtask build` / `cargo xtask run`: the Meson configure+compile step
//! and launching the resulting `vkqr-engine` binary, with one command line on
//! every platform. Meson stays the build system (AGENTS.md); these tasks only
//! drive it, with the same compiler and option choices CI makes (up to
//! project defaults).

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

/// Cargo builds `libquake_rs` for the host triple, and Meson links it only
/// as the MSVC-ABI staticlib on Windows: the MinGW/clangarm64 builds stay
/// C-only (PLAN.md section 3), so a `*-windows-gnu` xtask defaults to
/// `-Duse_rust=disabled`.
pub const RUST_LINKS_BY_DEFAULT: bool = !cfg!(all(windows, target_env = "gnu"));

/// Windows builds go through clang-cl (PLAN.md section 3; `meson.build` has
/// no `cl.exe` handling, e.g. the ADR-010 `-ffp-contract=off` pin), so an
/// MSVC-toolchain xtask supplies `CC=clang-cl` when nothing set `CC`.
pub const DEFAULT_CC: Option<&str> = if cfg!(all(windows, target_env = "msvc")) {
    Some("clang-cl")
} else {
    None
};

/// What `cargo xtask build` configures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildOptions {
    /// Meson build directory, relative to the repository root unless absolute.
    pub build_dir: PathBuf,
    /// Meson `--buildtype`.
    pub buildtype: String,
    /// `-Duse_rust=enabled` (the default where the staticlib links) or
    /// `disabled` for the C-only oracle.
    pub use_rust: bool,
    /// Re-run `meson setup --reconfigure` even if the directory is configured.
    pub reconfigure: bool,
    /// Extra arguments appended verbatim to `meson setup` (e.g. `-Dtrace=true`).
    pub setup_args: Vec<String>,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            build_dir: PathBuf::from("build"),
            buildtype: "release".into(),
            use_rust: RUST_LINKS_BY_DEFAULT,
            reconfigure: false,
            setup_args: Vec::new(),
        }
    }
}

/// Resolve `dir` against the repository root unless it is already absolute.
pub fn resolve_dir(root: &Path, dir: &Path) -> PathBuf {
    if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        root.join(dir)
    }
}

/// The engine executable Meson writes into `build_dir`.
pub fn engine_path(build_dir: &Path) -> PathBuf {
    build_dir.join(format!("vkqr-engine{}", env::consts::EXE_SUFFIX))
}

/// A Meson build directory is configured once `meson setup` has written its
/// private state; a partially written or stale directory is left to Meson's
/// own `--reconfigure` handling.
pub fn is_configured(build_dir: &Path) -> bool {
    build_dir
        .join("meson-private")
        .join("coredata.dat")
        .is_file()
}

fn is_unset(value: Option<OsString>) -> bool {
    value.is_none_or(|v| v.is_empty())
}

/// Whether `meson setup` should activate the Visual Studio environment
/// itself: only when this xtask (and so the staticlib cargo builds for the
/// host triple) targets MSVC and no Developer shell already did. `CC` says
/// which compiler to use, not that the environment is set up, so it does not
/// count; a MinGW (`*-windows-gnu`) toolchain never asks for it.
pub fn wants_vsenv(env_lookup: impl Fn(&str) -> Option<OsString>) -> bool {
    cfg!(all(windows, target_env = "msvc"))
        && ["VSINSTALLDIR", "VCINSTALLDIR"]
            .iter()
            .all(|var| is_unset(env_lookup(var)))
}

/// The `CC` value `meson setup` should run with when the caller set none:
/// [`DEFAULT_CC`] on an MSVC toolchain, nothing elsewhere.
pub fn default_cc(env_lookup: impl Fn(&str) -> Option<OsString>) -> Option<&'static str> {
    DEFAULT_CC.filter(|_| is_unset(env_lookup("CC")))
}

/// The `meson setup` argument list for `options`, without the program name.
/// `reconfigure` is passed only for a directory Meson already configured;
/// on a fresh one it is a plain setup.
pub fn setup_args(
    options: &BuildOptions,
    build_dir: &Path,
    reconfigure: bool,
    vsenv: bool,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["setup".into(), build_dir.as_os_str().to_owned()];
    if reconfigure {
        args.push("--reconfigure".into());
    }
    if vsenv {
        args.push("--vsenv".into());
    }
    args.push(format!("--buildtype={}", options.buildtype).into());
    let use_rust = if options.use_rust {
        "enabled"
    } else {
        "disabled"
    };
    args.push(format!("-Duse_rust={use_rust}").into());
    args.extend(options.setup_args.iter().map(OsString::from));
    args
}

fn status_error(program: &str, status: ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("{program} exited with status {code}"),
        None => format!("{program} was terminated by a signal"),
    }
}

fn run_checked(mut command: Command, program: &str) -> Result<(), String> {
    eprintln!("xtask: {command:?}");
    let status = command.status().map_err(|e| {
        format!("could not start `{program}`: {e} (is it installed and on PATH? see readme.md)")
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(status_error(program, status))
    }
}

/// Configure (if needed) and compile the engine. Returns the engine
/// executable's path.
pub fn build(root: &Path, options: &BuildOptions) -> Result<PathBuf, String> {
    let build_dir = resolve_dir(root, &options.build_dir);
    let meson = env::var_os("MESON").unwrap_or_else(|| "meson".into());
    let configured = is_configured(&build_dir);
    if options.reconfigure || !configured {
        let vsenv = wants_vsenv(|name| env::var_os(name));
        let mut setup = Command::new(&meson);
        setup
            .args(setup_args(
                options,
                &build_dir,
                options.reconfigure && configured,
                vsenv,
            ))
            .current_dir(root);
        if let Some(cc) = default_cc(|name| env::var_os(name)) {
            setup.env("CC", cc);
        }
        run_checked(setup, "meson setup")?;
    }
    let mut compile = Command::new(&meson);
    compile
        .arg("compile")
        .arg("-C")
        .arg(&build_dir)
        .current_dir(root);
    run_checked(compile, "meson compile")?;
    let exe = engine_path(&build_dir);
    if !exe.is_file() {
        return Err(format!(
            "meson compile succeeded but {} does not exist",
            exe.display()
        ));
    }
    Ok(exe)
}

/// The `-basedir` `cargo xtask run` gives the engine: an explicit
/// `--basedir`, else `QUAKE_GAME_DATA` (the harness convention), else none.
/// With none the engine keeps its own lookup (the working directory, then
/// the Steam/GOG/Epic store detection that an explicit `-basedir` would
/// switch off).
pub fn resolve_basedir(
    explicit: Option<&Path>,
    env_lookup: impl Fn(&str) -> Option<OsString>,
) -> Option<PathBuf> {
    explicit.map_or_else(
        || {
            env_lookup("QUAKE_GAME_DATA")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        },
        |dir| Some(dir.to_path_buf()),
    )
}

/// Whether `dir` holds game data the engine accepts as a basedir: classic
/// `id1/` or the 2021 re-release `QuakeEX.kpf` (`COM_IsValidFlavorDir`).
pub fn looks_like_basedir(dir: &Path) -> bool {
    dir.join("id1").is_dir() || dir.join("QuakeEX.kpf").is_file()
}

/// Launch `exe` with `-basedir basedir` (if any, and unless `engine_args`
/// already carry one: `COM_CheckParm` takes the first match, so ours would
/// shadow theirs) followed by `engine_args`, and return its exit status.
pub fn run(
    exe: &Path,
    basedir: Option<&Path>,
    engine_args: &[String],
) -> Result<ExitStatus, String> {
    let mut command = Command::new(exe);
    let basedir = match basedir {
        Some(_) if engine_args.iter().any(|a| a == "-basedir") => {
            eprintln!("xtask: engine arguments carry -basedir; leaving it to them");
            None
        }
        Some(dir) => {
            // absolute, not canonicalize: the engine gets a plain path, never
            // a Windows `\\?\` one
            let dir =
                std::path::absolute(dir).map_err(|e| format!("basedir {}: {e}", dir.display()))?;
            if !dir.is_dir() {
                return Err(format!("basedir {} is not a directory", dir.display()));
            }
            if !looks_like_basedir(&dir) {
                eprintln!(
                    "xtask: warning: {} has neither id1/ nor QuakeEX.kpf; the engine will not find game data",
                    dir.display()
                );
            }
            Some(dir)
        }
        None => None,
    };
    if let Some(dir) = &basedir {
        command.arg("-basedir").arg(dir).current_dir(dir);
    }
    command.args(engine_args);
    eprintln!("xtask: {command:?}");
    command
        .status()
        .map_err(|e| format!("could not start {}: {e}", exe.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<OsString> {
        None
    }

    fn strs(args: &[OsString]) -> Vec<&str> {
        args.iter().map(|a| a.to_str().unwrap()).collect()
    }

    #[test]
    fn default_setup_args_match_ci_up_to_project_defaults() {
        let args = setup_args(&BuildOptions::default(), Path::new("build"), false, false);
        let use_rust = if RUST_LINKS_BY_DEFAULT {
            "-Duse_rust=enabled"
        } else {
            "-Duse_rust=disabled"
        };
        assert_eq!(
            strs(&args),
            ["setup", "build", "--buildtype=release", use_rust]
        );
    }

    #[test]
    fn setup_args_carry_every_option() {
        let options = BuildOptions {
            build_dir: PathBuf::from("build-c-trace"),
            buildtype: "debug".into(),
            use_rust: false,
            reconfigure: true,
            setup_args: vec!["-Dtrace=true".into(), "-Duse_sdl3=disabled".into()],
        };
        let args = setup_args(&options, Path::new("build-c-trace"), true, true);
        assert_eq!(
            strs(&args),
            [
                "setup",
                "build-c-trace",
                "--reconfigure",
                "--vsenv",
                "--buildtype=debug",
                "-Duse_rust=disabled",
                "-Dtrace=true",
                "-Duse_sdl3=disabled",
            ]
        );
        let fresh = setup_args(&options, Path::new("build-c-trace"), false, true);
        assert!(!strs(&fresh).contains(&"--reconfigure"));
    }

    #[test]
    fn vsenv_only_outside_a_developer_shell() {
        let msvc_host = cfg!(all(windows, target_env = "msvc"));
        assert_eq!(wants_vsenv(no_env), msvc_host);
        for var in ["VSINSTALLDIR", "VCINSTALLDIR"] {
            assert!(!wants_vsenv(
                |name| (name == var).then(|| OsString::from("x"))
            ));
        }
        assert_eq!(
            wants_vsenv(|name| (name == "VSINSTALLDIR").then(OsString::new)),
            msvc_host
        );
        // CC picks the compiler, it does not set up the environment
        assert_eq!(
            wants_vsenv(|name| (name == "CC").then(|| OsString::from("clang-cl"))),
            msvc_host
        );
    }

    #[test]
    fn cc_defaults_to_clang_cl_only_when_unset() {
        assert_eq!(default_cc(no_env), DEFAULT_CC);
        assert_eq!(
            default_cc(|name| (name == "CC").then(OsString::new)),
            DEFAULT_CC
        );
        assert_eq!(
            default_cc(|name| (name == "CC").then(|| OsString::from("gcc"))),
            None
        );
    }

    #[test]
    fn rust_is_off_by_default_on_windows_gnu() {
        assert_eq!(
            BuildOptions::default().use_rust,
            !cfg!(all(windows, target_env = "gnu"))
        );
    }

    #[test]
    fn relative_build_dir_resolves_under_root() {
        let root = Path::new("repo");
        assert_eq!(
            resolve_dir(root, Path::new("build")),
            Path::new("repo").join("build")
        );
        let absolute = env::temp_dir().join("elsewhere");
        assert_eq!(resolve_dir(root, &absolute), absolute);
    }

    #[test]
    fn engine_path_uses_the_host_exe_suffix() {
        let exe = engine_path(Path::new("out"));
        let name = exe.file_name().unwrap().to_str().unwrap();
        assert_eq!(name, format!("vkqr-engine{}", env::consts::EXE_SUFFIX));
    }

    #[test]
    fn basedir_precedence() {
        let game_data = |name: &str| (name == "QUAKE_GAME_DATA").then(|| OsString::from("data"));
        assert_eq!(
            resolve_basedir(Some(Path::new("explicit")), game_data).as_deref(),
            Some(Path::new("explicit"))
        );
        assert_eq!(
            resolve_basedir(None, game_data).as_deref(),
            Some(Path::new("data"))
        );
        assert_eq!(resolve_basedir(None, no_env), None);
        let empty = |name: &str| (name == "QUAKE_GAME_DATA").then(OsString::new);
        assert_eq!(resolve_basedir(None, empty), None);
    }

    #[test]
    fn basedir_accepts_classic_and_rerelease_layouts() {
        let dir = env::temp_dir().join(format!("xtask-basedir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!looks_like_basedir(&dir));
        std::fs::write(dir.join("QuakeEX.kpf"), b"").unwrap();
        assert!(looks_like_basedir(&dir));
        std::fs::remove_file(dir.join("QuakeEX.kpf")).unwrap();
        std::fs::create_dir(dir.join("id1")).unwrap();
        assert!(looks_like_basedir(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unconfigured_dir_is_detected() {
        let dir = env::temp_dir().join(format!("xtask-engine-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!is_configured(&dir));
        std::fs::create_dir_all(dir.join("meson-private")).unwrap();
        assert!(!is_configured(&dir));
        std::fs::write(dir.join("meson-private").join("coredata.dat"), b"").unwrap();
        assert!(is_configured(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
