//! The GLSL -> SPIR-V pipeline of `meson.build`: `glslangValidator -V
//! --quiet [-g]` per job, then `spirv-opt -Os --canonicalize-ids
//! --strip-debug` unless the build is a debug one or the build machine is
//! macOS (MoltenVK's spirv-cross has a bug that breaks optimized shaders).
//! The job table below is `meson.build`'s `shaders` + `shader_variants`, in
//! its order.

use std::path::{Path, PathBuf};
use std::process::Command;

/// One shader compile: `source` (repository-relative) built as `name`
/// (`<stem>.<stage>`, the Meson output name) with extra glslang `args`.
#[derive(Clone, Copy, Debug)]
pub struct Job {
    pub source: &'static str,
    pub name: &'static str,
    pub args: &'static [&'static str],
}

impl Job {
    /// The `bintoc` symbol (`name + '_spv'` with '.' -> '_'), and the
    /// `Shader` name on the Rust side.
    pub fn symbol(&self) -> String {
        crate::bintoc::symbol(&format!("{}_spv", self.name))
    }
}

/// `meson.build`'s `shaders`: sources built once under their own name.
pub const SOURCES: &[&str] = &[
    "Shaders/alias.vert",
    "Shaders/md5.vert",
    "Shaders/basic.vert",
    "Shaders/basic_alphatest.frag",
    "Shaders/basic_notex.frag",
    "Shaders/cs_tex_warp.comp",
    "Shaders/indirect.comp",
    "Shaders/indirect_clear.comp",
    "Shaders/postprocess.frag",
    "Shaders/postprocess.vert",
    "Shaders/showtris.frag",
    "Shaders/showtris.vert",
    "Shaders/sky_box.frag",
    "Shaders/sky_cube.frag",
    "Shaders/sky_cube.vert",
    "Shaders/sky_layer.frag",
    "Shaders/sky_layer.vert",
    "Shaders/world.vert",
    "Shaders/ray_debug.comp",
    "Shaders/mesh_interpolate.comp",
    "Shaders/skinning.comp",
];

const SOPS: &[&str] = &["--target-env", "vulkan1.1"];

/// `meson.build`'s `shader_variants`: sources compiled several times under
/// different preprocessor defines.
#[rustfmt::skip]
pub const VARIANTS: &[Job] = &[
    Job { source: "Shaders/basic.frag", name: "basic.frag", args: &[] },
    Job { source: "Shaders/basic.frag", name: "basic_oit.frag", args: &["-DWBOIT=1"] },
    Job { source: "Shaders/basic.frag", name: "basic_mboit_moment.frag", args: &["-DMBOIT=1"] },
    Job { source: "Shaders/basic.frag", name: "basic_mboit_composite.frag", args: &["-DMBOIT=1", "-DMBOIT_COMPOSITE=1"] },
    Job { source: "Shaders/basic.frag", name: "basic_mboit_composite_msaa.frag", args: &["-DMBOIT=1", "-DMBOIT_COMPOSITE=1", "-DMSAA=1"] },
    Job { source: "Shaders/world.frag", name: "world.frag", args: &[] },
    Job { source: "Shaders/world.frag", name: "world_oit.frag", args: &["-DWBOIT=1"] },
    Job { source: "Shaders/world.frag", name: "world_mboit_moment.frag", args: &["-DMBOIT=1"] },
    Job { source: "Shaders/world.frag", name: "world_mboit_composite.frag", args: &["-DMBOIT=1", "-DMBOIT_COMPOSITE=1"] },
    Job { source: "Shaders/world.frag", name: "world_mboit_composite_msaa.frag", args: &["-DMBOIT=1", "-DMBOIT_COMPOSITE=1", "-DMSAA=1"] },
    Job { source: "Shaders/alias.frag", name: "alias.frag", args: &[] },
    Job { source: "Shaders/alias.frag", name: "alias_alphatest.frag", args: &["-DALIAS_ALPHA_TEST=1"] },
    Job { source: "Shaders/alias.frag", name: "alias_oit.frag", args: &["-DWBOIT=1"] },
    Job { source: "Shaders/alias.frag", name: "alias_alphatest_oit.frag", args: &["-DALIAS_ALPHA_TEST=1", "-DWBOIT=1"] },
    Job { source: "Shaders/alias.frag", name: "alias_mboit_moment.frag", args: &["-DMBOIT=1"] },
    Job { source: "Shaders/alias.frag", name: "alias_alphatest_mboit_moment.frag", args: &["-DALIAS_ALPHA_TEST=1", "-DMBOIT=1"] },
    Job { source: "Shaders/alias.frag", name: "alias_mboit_composite.frag", args: &["-DMBOIT=1", "-DMBOIT_COMPOSITE=1"] },
    Job { source: "Shaders/alias.frag", name: "alias_alphatest_mboit_composite.frag", args: &["-DALIAS_ALPHA_TEST=1", "-DMBOIT=1", "-DMBOIT_COMPOSITE=1"] },
    Job { source: "Shaders/alias.frag", name: "alias_mboit_composite_msaa.frag", args: &["-DMBOIT=1", "-DMBOIT_COMPOSITE=1", "-DMSAA=1"] },
    Job { source: "Shaders/alias.frag", name: "alias_alphatest_mboit_composite_msaa.frag", args: &["-DALIAS_ALPHA_TEST=1", "-DMBOIT=1", "-DMBOIT_COMPOSITE=1", "-DMSAA=1"] },
    Job { source: "Shaders/alias.frag", name: "md5_mboit_composite.frag", args: &["-DMBOIT=1", "-DMBOIT_COMPOSITE=1", "-DMBOIT_INPUT_SET=4"] },
    Job { source: "Shaders/alias.frag", name: "md5_alphatest_mboit_composite.frag", args: &["-DALIAS_ALPHA_TEST=1", "-DMBOIT=1", "-DMBOIT_COMPOSITE=1", "-DMBOIT_INPUT_SET=4"] },
    Job { source: "Shaders/alias.frag", name: "md5_mboit_composite_msaa.frag", args: &["-DMBOIT=1", "-DMBOIT_COMPOSITE=1", "-DMSAA=1", "-DMBOIT_INPUT_SET=4"] },
    Job { source: "Shaders/alias.frag", name: "md5_alphatest_mboit_composite_msaa.frag", args: &["-DALIAS_ALPHA_TEST=1", "-DMBOIT=1", "-DMBOIT_COMPOSITE=1", "-DMSAA=1", "-DMBOIT_INPUT_SET=4"] },
    Job { source: "Shaders/md5.vert", name: "md5_8.vert", args: &["-DEIGHT_WEIGHT_SKINNING"] },
    Job { source: "Shaders/wboit_resolve.frag", name: "wboit_resolve.frag", args: &[] },
    Job { source: "Shaders/wboit_resolve.frag", name: "wboit_resolve_msaa.frag", args: &["-DMSAA=1"] },
    Job { source: "Shaders/mboit_resolve.frag", name: "mboit_resolve.frag", args: &[] },
    Job { source: "Shaders/mboit_resolve.frag", name: "mboit_resolve_msaa.frag", args: &["-DMSAA=1"] },
    Job { source: "Shaders/screen_effects.comp", name: "screen_effects_8bit.comp", args: &[] },
    Job { source: "Shaders/screen_effects.comp", name: "screen_effects_8bit_scale.comp", args: &["-DSCALING=1"] },
    Job { source: "Shaders/screen_effects.comp", name: "screen_effects_8bit_scale_sops.comp", args: &["--target-env", "vulkan1.1", "-DSCALING=1", "-DUSE_SUBGROUP_OPS=1"] },
    Job { source: "Shaders/screen_effects.comp", name: "screen_effects_10bit.comp", args: &["-DUSE_10BIT=1"] },
    Job { source: "Shaders/screen_effects.comp", name: "screen_effects_10bit_scale.comp", args: &["-DUSE_10BIT=1", "-DSCALING=1"] },
    Job { source: "Shaders/screen_effects.comp", name: "screen_effects_10bit_scale_sops.comp", args: &["--target-env", "vulkan1.1", "-DUSE_10BIT=1", "-DSCALING=1", "-DUSE_SUBGROUP_OPS=1"] },
    Job { source: "Shaders/update_lightmap.comp", name: "update_lightmap_8bit.comp", args: &[] },
    Job { source: "Shaders/update_lightmap.comp", name: "update_lightmap_8bit_rt.comp", args: &["-DRAY_QUERIES=1"] },
    Job { source: "Shaders/update_lightmap.comp", name: "update_lightmap_10bit.comp", args: &["-DUSE_10BIT=1"] },
    Job { source: "Shaders/update_lightmap.comp", name: "update_lightmap_10bit_rt.comp", args: &["-DUSE_10BIT=1", "-DRAY_QUERIES=1"] },
    Job { source: "Shaders/skinning.comp", name: "skinning_8.comp", args: &["-DEIGHT_WEIGHT_SKINNING"] },
];

/// `meson.build`'s `shader_jobs`: the plain sources (`--target-env
/// vulkan1.1` for the ones named `sops`), then the variants.
pub fn jobs() -> Vec<Job> {
    SOURCES
        .iter()
        .map(|source| Job {
            source,
            name: source.rsplit('/').next().unwrap_or(source),
            args: if source.contains("sops") { SOPS } else { &[] },
        })
        .chain(VARIANTS.iter().copied())
        .collect()
}

/// Which of Meson's two pipelines to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// `buildtype.startswith('debug')`: `-g` for glslang.
    pub debug: bool,
    /// Run `spirv-opt` (off for debug buildtypes and on a macOS build
    /// machine, exactly like `meson.build`).
    pub spirv_opt: bool,
}

impl Options {
    pub fn for_build(debug: bool, build_machine_is_darwin: bool) -> Self {
        Self {
            debug,
            spirv_opt: !(debug || build_machine_is_darwin),
        }
    }
}

/// The tool binaries. `QUAKE_GLSLANG`/`QUAKE_SPIRV_OPT` (Meson passes its
/// `find_program` results through them so both builds compile with the same
/// binaries) win over `PATH`, which wins over `$VULKAN_SDK/bin`.
#[derive(Clone, Debug)]
pub struct Tools {
    pub glslang: PathBuf,
    pub spirv_opt: Option<PathBuf>,
}

fn find_program(env_var: &str, name: &str) -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os(env_var).filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    let exe_names: Vec<String> = if cfg!(windows) {
        vec![format!("{name}.exe"), name.to_string()]
    } else {
        vec![name.to_string()]
    };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect())
        .unwrap_or_default();
    if let Some(sdk) = std::env::var_os("VULKAN_SDK") {
        dirs.push(Path::new(&sdk).join("bin"));
        dirs.push(Path::new(&sdk).join("Bin"));
    }
    for dir in dirs {
        for exe in &exe_names {
            let candidate = dir.join(exe);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(format!(
        "{name} not found (set {env_var}, add it to PATH, or set VULKAN_SDK)"
    ))
}

impl Tools {
    pub fn find(options: Options) -> Result<Self, String> {
        Ok(Self {
            glslang: find_program("QUAKE_GLSLANG", "glslangValidator")?,
            spirv_opt: if options.spirv_opt {
                Some(find_program("QUAKE_SPIRV_OPT", "spirv-opt")?)
            } else {
                None
            },
        })
    }

    /// `meson.build`'s `has_canonicalize_ids_spirv_opt` probe.
    pub fn spirv_opt_has_canonicalize_ids(&self) -> Result<bool, String> {
        let Some(spirv_opt) = &self.spirv_opt else {
            return Ok(true);
        };
        let output = Command::new(spirv_opt)
            .arg("-h")
            .output()
            .map_err(|e| format!("{}: {e}", spirv_opt.display()))?;
        Ok(String::from_utf8_lossy(&output.stdout).contains("--canonicalize-ids"))
    }
}

/// Meson's argument lists.
pub const GLSLANG_ARGS: &[&str] = &["-V", "--quiet"];
pub const SPIRV_OPT_ARGS: &[&str] = &["-Os", "--canonicalize-ids", "--strip-debug"];

fn run(command: &mut Command) -> Result<(), String> {
    let output = command
        .output()
        .map_err(|e| format!("{:?}: {e}", command.get_program()))?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "{command:?} failed ({}):\n{}{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

/// Compiles one job into `out_dir/<name>.spv` (via `<name>.raw.spv` when
/// spirv-opt runs), glslang invoked from `repo_root` with the
/// repository-relative source path.
pub fn compile(
    job: &Job,
    repo_root: &Path,
    out_dir: &Path,
    options: Options,
    tools: &Tools,
) -> Result<PathBuf, String> {
    let spv = out_dir.join(format!("{}.spv", job.name));
    let glslang_out = if tools.spirv_opt.is_some() {
        out_dir.join(format!("{}.raw.spv", job.name))
    } else {
        spv.clone()
    };
    let mut glslang = Command::new(&tools.glslang);
    glslang.current_dir(repo_root).args(GLSLANG_ARGS);
    if options.debug {
        glslang.arg("-g");
    }
    glslang
        .args(job.args)
        .arg("-o")
        .arg(&glslang_out)
        .arg(job.source);
    run(&mut glslang)?;
    if let Some(spirv_opt) = &tools.spirv_opt {
        run(Command::new(spirv_opt)
            .args(SPIRV_OPT_ARGS)
            .arg(&glslang_out)
            .arg("-o")
            .arg(&spv))?;
    }
    Ok(spv)
}

/// Compiles every job into `out_dir`, in parallel; returns the `.spv` paths
/// in [`jobs`] order. Under a build script Cargo's `NUM_JOBS` bounds the
/// tool processes, since rustc jobs are already using the cores.
pub fn compile_all(
    repo_root: &Path,
    out_dir: &Path,
    options: Options,
    tools: &Tools,
) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;
    let jobs = jobs();
    let threads = std::env::var("NUM_JOBS")
        .ok()
        .and_then(|n| n.parse().ok())
        .or_else(|| std::thread::available_parallelism().ok().map(|n| n.get()))
        .unwrap_or(1)
        .clamp(1, jobs.len().max(1));
    let chunk = jobs.len().div_ceil(threads);
    let results: Vec<Result<PathBuf, String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .chunks(chunk)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|job| compile(job, repo_root, out_dir, options, tools))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("shader compile thread panicked"))
            .collect()
    });
    results.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_table_matches_meson() {
        let jobs = jobs();
        assert_eq!(jobs.len(), SOURCES.len() + VARIANTS.len());
        assert_eq!(jobs[0].name, "alias.vert");
        assert_eq!(jobs[0].symbol(), "alias_vert_spv");
        assert!(jobs
            .iter()
            .all(|j| j.args.contains(&"vulkan1.1") == j.name.contains("sops")));
        let mut names: Vec<_> = jobs.iter().map(|j| j.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), jobs.len());
    }

    #[test]
    fn job_table_covers_every_shader() {
        use quake_render::rmisc::Shader;
        let jobs = jobs();
        assert_eq!(jobs.len(), Shader::ALL.len());
        for shader in Shader::ALL {
            let name = shader.name().to_str().unwrap();
            assert_eq!(
                jobs.iter()
                    .filter(|j| crate::bintoc::symbol(j.name) == name)
                    .count(),
                1,
                "{name}"
            );
        }
    }
}
