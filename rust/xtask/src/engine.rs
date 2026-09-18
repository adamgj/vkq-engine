//! `cargo xtask build` / `cargo xtask run`: the Meson configure+compile step
//! and launching the resulting `vkqr-engine` binary, with one command line on
//! every platform. Meson stays the build system (AGENTS.md); these tasks only
//! drive it, so the flags mirror what CI passes.

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

/// What `cargo xtask build` configures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildOptions {
    /// Meson build directory, relative to the repository root unless absolute.
    pub build_dir: PathBuf,
    /// Meson `--buildtype`.
    pub buildtype: String,
    /// `-Duse_rust=enabled` (the default) or `disabled` for the C-only oracle.
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
            use_rust: true,
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

/// Whether `meson setup` should activate the Visual Studio environment
/// itself: only when this xtask (and so the staticlib cargo builds for the
/// host triple) targets MSVC, and nothing else has picked a compiler through
/// a `CC` override or an already active vcvars shell. A MinGW
/// (`*-windows-gnu`) toolchain never asks for it (PLAN.md section 3).
pub fn wants_vsenv(env_lookup: impl Fn(&str) -> Option<OsString>) -> bool {
    cfg!(all(windows, target_env = "msvc"))
        && ["CC", "VSINSTALLDIR", "VCINSTALLDIR"]
            .iter()
            .all(|var| env_lookup(var).is_none_or(|v| v.is_empty()))
}

/// The `meson setup` argument list for `options`, without the program name.
pub fn setup_args(options: &BuildOptions, build_dir: &Path, vsenv: bool) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["setup".into(), build_dir.as_os_str().to_owned()];
    if options.reconfigure {
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
    if options.reconfigure || !is_configured(&build_dir) {
        let vsenv = wants_vsenv(|name| env::var_os(name));
        let mut setup = Command::new(&meson);
        setup
            .args(setup_args(options, &build_dir, vsenv))
            .current_dir(root);
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

/// Where `cargo xtask run` launches the engine from: an explicit `--basedir`,
/// else `QUAKE_GAME_DATA` (the harness convention), else the current
/// directory. The engine resolves `id1/` under its `-basedir`.
pub fn resolve_basedir(
    explicit: Option<&Path>,
    env_lookup: impl Fn(&str) -> Option<OsString>,
    cwd: &Path,
) -> PathBuf {
    explicit.map_or_else(
        || {
            env_lookup("QUAKE_GAME_DATA")
                .filter(|v| !v.is_empty())
                .map_or_else(|| cwd.to_path_buf(), PathBuf::from)
        },
        Path::to_path_buf,
    )
}

/// Launch `exe` with `-basedir basedir` followed by `engine_args`, and
/// return its exit status.
pub fn run(exe: &Path, basedir: &Path, engine_args: &[String]) -> Result<ExitStatus, String> {
    // absolute, not canonicalize: the engine gets a plain path, never a
    // Windows `\\?\` one
    let basedir =
        std::path::absolute(basedir).map_err(|e| format!("basedir {}: {e}", basedir.display()))?;
    if !basedir.is_dir() {
        return Err(format!("basedir {} is not a directory", basedir.display()));
    }
    if !basedir.join("id1").is_dir() {
        eprintln!(
            "xtask: warning: {} has no id1/ directory; the engine will not find game data",
            basedir.display()
        );
    }
    let mut command = Command::new(exe);
    command
        .arg("-basedir")
        .arg(&basedir)
        .args(engine_args)
        .current_dir(&basedir);
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
    fn default_setup_args_mirror_ci() {
        let args = setup_args(&BuildOptions::default(), Path::new("build"), false);
        assert_eq!(
            strs(&args),
            [
                "setup",
                "build",
                "--buildtype=release",
                "-Duse_rust=enabled"
            ]
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
        let args = setup_args(&options, Path::new("build-c-trace"), true);
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
    }

    #[test]
    fn vsenv_only_when_nothing_else_chose_a_compiler() {
        let msvc_host = cfg!(all(windows, target_env = "msvc"));
        assert_eq!(wants_vsenv(no_env), msvc_host);
        for var in ["CC", "VSINSTALLDIR", "VCINSTALLDIR"] {
            assert!(!wants_vsenv(
                |name| (name == var).then(|| OsString::from("x"))
            ));
        }
        assert_eq!(
            wants_vsenv(|name| (name == "CC").then(OsString::new)),
            msvc_host
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
        let cwd = Path::new("cwd");
        let game_data = |name: &str| (name == "QUAKE_GAME_DATA").then(|| OsString::from("data"));
        assert_eq!(
            resolve_basedir(Some(Path::new("explicit")), game_data, cwd),
            Path::new("explicit")
        );
        assert_eq!(resolve_basedir(None, game_data, cwd), Path::new("data"));
        assert_eq!(resolve_basedir(None, no_env, cwd), cwd);
        let empty = |name: &str| (name == "QUAKE_GAME_DATA").then(OsString::new);
        assert_eq!(resolve_basedir(None, empty, cwd), cwd);
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
