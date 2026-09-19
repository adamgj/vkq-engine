project(
    'vkqr-engine',
    'c',
    meson_version : '>= 1.3',
    default_options : ['c_std=gnu11,c11', 'buildtype=release', 'strip=false', 'debug=true', 'b_ndebug=if-release', 'warning_level=1', 'werror=true'])

cc = meson.get_compiler('c')

# Always build keeping frame pointers to get better backtraces
add_project_arguments(cc.get_supported_arguments('-fno-omit-frame-pointer', '/Oy-'), language: 'c')

# Ask linker not to locate uninitialized variables into COMMON (will use .bss then) to avoid macOS link issue
# (clang-cl doesn't accept it, and warning-as-error would turn that into a build failure)
add_project_arguments(cc.get_supported_arguments('-fno-common'), language: 'c')

# ADR-010 (Rust migration): pin FP contraction off so C and Rust builds agree
# on FMA usage. Compilers default to contracting a*b+c into fused fmadd on
# targets that have it (arm64), which plain Rust float ops don't reproduce;
# only arm64 targets change behavior (x86-64 baseline has no FMA). The
# darwin-arm64 harness goldens were regenerated for this (Phase 1).
# note: this is a *project-wide* flag -- it covers the vendored miniz /
# stb_image / mimalloc sources and the whole renderer, not just the ported
# leaves. That is deliberate: every later phase ports code onto a C build that
# already matches Rust's FP semantics. See ADR-010's Phase 1 amendment.
# first_supported_argument, not get_supported_arguments: clang-cl accepts both
# spellings, and adding both would pass the flag twice.
add_project_arguments(cc.first_supported_argument('-ffp-contract=off', '/clang:-ffp-contract=off'), language: 'c')

if host_machine.system() == 'windows'
    foreach native : [true, false]
        add_project_arguments('-D_CRT_SECURE_NO_WARNINGS', '-D_CRT_NONSTDC_NO_DEPRECATE', '-D_WINSOCK_DEPRECATED_NO_WARNINGS', language : 'c', native : native)
    endforeach
endif

if host_machine.system() != 'windows'
    if cc.has_header_symbol('sched.h', 'CPU_ZERO', prefix: '#define _GNU_SOURCE')
        message('Info: Task affinity support enabled.')
        add_project_arguments('-D_GNU_SOURCE', language: 'c')
    else
        message('Warning: sched.h not found, disabling Task affinity support.')
        add_project_arguments('-DTASK_AFFINITY_NOT_AVAILABLE', language: 'c')
    endif
endif

# Prefer SDL3 over SDL2; -Duse_sdl3=enabled requires SDL3, disabled forces SDL2
use_sdl3_opt = get_option('use_sdl3')
use_sdl3 = false
if host_machine.system() == 'windows'
    if use_sdl3_opt.disabled()
        error('SDL2 is not supported on Windows')
    endif
    # use the bundled SDL3 like the other Windows builds do
    sdl_dep = declare_dependency(
        include_directories : include_directories('Windows/SDL3/include'),
        dependencies : cc.find_library('SDL3', dirs : meson.project_source_root() / 'Windows/SDL3/lib64'))
    use_sdl3 = true
else
    if not use_sdl3_opt.disabled()
        sdl_dep = dependency('sdl3', required : use_sdl3_opt.enabled())
        use_sdl3 = sdl_dep.found()
    endif
endif
if use_sdl3
    add_project_arguments('-DUSE_SDL3', language: 'c')
    message('Info: Using SDL3.')
else
    sdl_dep = dependency('sdl2')
    message('Info: Using SDL2.')
endif

glslang = find_program('glslangValidator')
spirv_opt = find_program('spirv-opt')

# Rust migration, Phase 9 close (the deletion PR): the engine is the Rust
# staticlib libquake_rs (rust/, crate quake-capi) plus the C remnant TUs
# listed below. There is no C-only configuration any more -- the last
# C-capable tree is tag c-reference/final (ADR-019) -- so the Rust toolchain
# and cbindgen are hard requirements. cargo builds for the host default
# triple, so it must match the C compiler's ABI (clang-cl on Windows).
cargo = find_program('cargo')
cbindgen = find_program('cbindgen')
python3 = find_program('python3')
# the quake-capi build script compiles the shaders and builds the embedded
# pak with these tools (Phase 8 M11, rust/xtask/src/shaders.rs); Meson only
# locates them and passes them through, and mirrors the build script's
# spirv-opt requirement so the failure is reported at configure time
run_test_canonicalize_ids_spirv_opt = run_command(spirv_opt, '-h', check : false)
has_canonicalize_ids_spirv_opt = (run_test_canonicalize_ids_spirv_opt.stdout().strip().contains('--canonicalize-ids'))
if not ((build_machine.system() == 'darwin') or get_option('buildtype').startswith('debug'))
    if not has_canonicalize_ids_spirv_opt
        error('spirv-opt does not support --canonicalize-ids, a newer Vulkan SDK is required')
    endif
endif

rust_profile = get_option('buildtype') == 'debug' ? 'debug' : 'release'
# engine-alloc binds Rust's global allocator to the mimalloc that mem.c
# amalgamates (ADR-013). The per-module features are all on: the switches
# that used to gate them (-Duse_rust_*) went with the C oracle; the features
# themselves collapse at Phase 10 M9.
rust_feature_list = [
    'engine-alloc',
    'fs', 'image', 'formats', 'snd', 'net', 'progs', 'cvar', 'host', 'tasks', 'render', 'platform',
    'progs-host',
]
# the ported code's debug-only diagnostics must track the C engine's
# DEBUG/_DEBUG, not the Rust profile: -D_DEBUG is set for every debug*
# buildtype while the Rust profile is release for debugoptimized
if get_option('buildtype').startswith('debug')
    rust_feature_list += 'engine-debug'
endif
# cargo features tracking C defines the fs shims must agree with
if use_sdl3
    rust_feature_list += 'sdl3'
else
    rust_feature_list += 'sdl2'
endif
if get_option('do_userdirs').enabled()
    # the Rust filesystem does not implement the DO_USERDIRS lookup branches
    # (quake-capi fails the combination with a compile_error!); say so here
    # rather than several minutes into a cargo build
    error('-Ddo_userdirs=enabled is not supported by the Rust filesystem')
endif
# the codec-availability features are appended after the codec probes below

# The C remnant. Shared engine TUs port at Phase 10 M8; the *_glue.c TUs
# (C-visible data, variadic wrappers, ADR-009 Host_Guard/Host_Reraise frames)
# go stratum by stratum at Phase 10 M6; the codec bridges (ADR-014), the
# lodepng/stb hosts (ADR-012) and the harness TUs (ADR-019) stay.
srcs = [
    'Quake/cd_null.c',
    'Quake/common.c',
    'Quake/gl_model.c',
    'Quake/harness.c',
    'Quake/harness_render.c',
    'Quake/q_thread_sdl.c',
    'Quake/pr_trace.c',
    'Quake/image.c',
    'Quake/image_stb.c',
    'Quake/mem.c',
    'Quake/net_loop.c',
    'Quake/net_main.c',
    'Quake/palette.c',
    'Quake/pr_cmds.c',
    'Quake/pr_edict.c',
    'Quake/pr_ext.c',
    'Quake/steam_api.c',
    'Quake/model_parse.c',
    'Quake/model_parse_glue.c',
    'Quake/net_msg_glue.c',
    'Quake/net_dgrm_glue.c',
    'Quake/net_dgrm_orch_glue.c',
    'Quake/in_sdl_glue.c',
    'Quake/sys_glue.c',
    'Quake/pr_exec_glue.c',
    'Quake/pr_edict_save_glue.c',
    'Quake/pr_edict_parse_glue.c',
    'Quake/pr_edict_load_glue.c',
    'Quake/pr_edict_arena_glue.c',
    'Quake/pr_cmds_glue.c',
    'Quake/cvar_cmd_glue.c',
    'Quake/world_glue.c',
    'Quake/sv_move_glue.c',
    'Quake/sv_phys_glue.c',
    'Quake/pr_cmds_sv_glue.c',
    'Quake/pr_cmds_sv_msg_glue.c',
    'Quake/pr_cmds_sv_fx_glue.c',
    'Quake/pr_cmds_cl_glue.c',
    'Quake/pr_edict_dispatch_glue.c',
    'Quake/sv_main_glue.c',
    'Quake/sv_send_glue.c',
    'Quake/sv_user_glue.c',
    'Quake/chase_glue.c',
    'Quake/cl_input_glue.c',
    'Quake/cl_tent_glue.c',
    'Quake/view_glue.c',
    'Quake/cl_parse_glue.c',
    'Quake/cl_main_glue.c',
    'Quake/cl_demo_glue.c',
    'Quake/host_glue.c',
    'Quake/host_cmd_glue.c',
    'Quake/keys_glue.c',
    'Quake/console_glue.c',
    'Quake/sbar_glue.c',
    'Quake/menu_glue.c',
    'Quake/r_part_glue.c',
    'Quake/r_part_fte_glue.c',
    'Quake/tasks_glue.c',
    'Quake/gl_heap_glue.c',
    'Quake/gl_texmgr_glue.c',
    'Quake/gl_rmisc_glue.c',
    'Quake/gl_vidsdl_glue.c',
    'Quake/gl_draw_glue.c',
    'Quake/gl_fog_glue.c',
    'Quake/gl_refrag_glue.c',
    'Quake/gl_rlight_glue.c',
    'Quake/gl_screen_glue.c',
    'Quake/gl_sky_glue.c',
    'Quake/gl_warp_glue.c',
    'Quake/r_sprite_glue.c',
    'Quake/r_alias_glue.c',
    'Quake/gl_rmain_glue.c',
    'Quake/snd_glue.c',
]

if host_machine.system() == 'windows'
    srcs += [
        'Quake/net_win.c',
    ]
    srcs += import('windows').compile_resources(
        'Windows/vkqr-engine.rc',
        # the \xA9 copyright escape is Windows-1252; llvm-rc refuses 8 bit
        # codepoints without an explicit codepage. gcc builds use windres,
        # which takes no /C option
        args : cc.get_id() == 'gcc' ? [] : ['/C', '1252'],
        include_directories : include_directories('Windows', 'Quake'),
        depend_files : ['Windows/vkQuake.ico'])
else
    srcs += [
        'Quake/net_bsd.c',
    ]
endif

# ADR-017: the SDL2 audio backend stays C until Phase 10 M9 ports it (the
# file self-gates on USE_SDL3)
if not use_sdl3
    srcs += 'Quake/snd_sdl.c'
endif

cflags = cc.get_supported_arguments('-Wno-trigraphs', '-Wno-microsoft-enum-forward-reference') + ['-D_FILE_OFFSET_BITS=64']
if cc.get_id() == 'clang' or cc.get_id() == 'clang-cl'
    # the stb single header libraries define their whole API static
    # (clang-cl reports its id as 'clang-cl', so it needs the same suppression)
    cflags += cc.get_supported_arguments('-Wno-unused-function')
endif

deps = [
    cc.find_library('m', required : false),
    cc.find_library('dl', required : false),
    dependency('threads'),
    sdl_dep,
]

# (the quake_rs staticlib dependency is appended after the codec probes below)

if host_machine.system() == 'windows'
    # the Rust standard library on *-pc-windows-msvc needs the second group
    win_syslibs = ['ws2_32', 'winmm', 'dbghelp', 'ole32', 'uuid', 'advapi32', 'userenv', 'bcrypt', 'ntdll', 'synchronization']
    foreach syslib : win_syslibs
        deps += cc.find_library(syslib)
    endforeach
    import('fs').copyfile('Windows/SDL3/lib64/SDL3.dll')
endif

if build_machine.system() == 'darwin'
    molten_dirs = []
    if import('fs').exists('/opt/homebrew/lib/libMoltenVK.dylib')
        molten_dirs += '/opt/homebrew/lib'
    endif
    deps += cc.find_library('MoltenVK', required : true, dirs : molten_dirs)
else
    deps += dependency('vulkan', version: '>=1.4.341')
endif

if get_option('trace')
    cflags += '-DPR_TRACE'
endif

if get_option('use_codec_wave').enabled()
    cflags += '-DUSE_CODEC_WAVE'
endif

if host_machine.system() == 'windows'
    # the bundled codec libraries, like the other Windows builds use
    codec_inc = include_directories('Windows/codecs/include')
    codec_lib_dir = meson.project_source_root() / 'Windows/codecs/x64'
    codec_deps = {}
    foreach name, option : {
        'libmad' : 'use_codec_mp3',
        'libmpg123' : 'use_codec_mp3',
        'libFLAC' : 'use_codec_flac',
        'libogg' : 'use_codec_vorbis',
        'libvorbisfile' : 'use_codec_vorbis',
        'libvorbis' : 'use_codec_vorbis',
        'libopus' : 'use_codec_opus',
        'libopusfile' : 'use_codec_opus',
    }
        lib = cc.find_library(name, dirs : codec_lib_dir, required : get_option(option))
        codec_deps += {name : lib.found() ? declare_dependency(include_directories : codec_inc, dependencies : lib) : lib}
    endforeach
    foreach dll : ['libFLAC-8.dll', 'libmad-0.dll', 'libmpg123-0.dll', 'libogg-0.dll', 'libopus-0.dll', 'libopusfile-0.dll', 'libvorbis-0.dll', 'libvorbisfile-3.dll']
        import('fs').copyfile('Windows/codecs/x64' / dll)
    endforeach
    mad_dep = codec_deps['libmad']
    mpg123_dep = codec_deps['libmpg123']
    flac_dep = codec_deps['libFLAC']
    ogg_dep = codec_deps['libogg']
    vorbisfile_dep = codec_deps['libvorbisfile']
    vorbis_dep = codec_deps['libvorbis']
    tremor_dep = dependency('', required : false)
    opus_dep = codec_deps['libopus']
    opusfile_dep = codec_deps['libopusfile']
else
    if get_option('mp3_lib') == 'mad'
        mad_dep = dependency('mad', required : get_option('use_codec_mp3'))
    else
        mpg123_dep = dependency('libmpg123', required : get_option('use_codec_mp3'))
    endif
    flac_dep = dependency('flac', required : get_option('use_codec_flac'))
    ogg_dep = dependency('ogg', required : get_option('use_codec_vorbis'))
    if get_option('vorbis_lib') == 'vorbis'
        vorbisfile_dep = dependency('vorbisfile', required : get_option('use_codec_vorbis'))
        vorbis_dep = dependency('vorbis', required : get_option('use_codec_vorbis'))
    else
        tremor_dep = dependency('vorbisidec', required : get_option('use_codec_vorbis'))
    endif
    opus_dep = dependency('opus', required : get_option('use_codec_opus'))
    opusfile_dep = dependency('opusfile', required : get_option('use_codec_opus'))
endif

# the mp3 tag skipper is the Rust port (quake-capi exports mp3_skiptags);
# the decoder wrappers themselves stay C (ADR-014)
mp3tag_srcs = []
if get_option('mp3_lib') == 'mad'
    if mad_dep.found()
        srcs += [ 'Quake/snd_mp3.c' ] + mp3tag_srcs
        deps += mad_dep
        cflags += '-DUSE_CODEC_MP3'
    endif
else
    if mpg123_dep.found()
        srcs += [ 'Quake/snd_mpg123.c' ] + mp3tag_srcs
        deps += mpg123_dep
        cflags += '-DUSE_CODEC_MP3'
    endif
endif

if flac_dep.found()
    srcs += 'Quake/snd_flac.c'
    deps += flac_dep
    cflags += '-DUSE_CODEC_FLAC'
endif

if get_option('vorbis_lib') == 'vorbis'
    if ogg_dep.found() and vorbisfile_dep.found() and vorbis_dep.found()
        srcs += 'Quake/snd_vorbis.c'
        deps += [ ogg_dep, vorbisfile_dep, vorbis_dep ]
        cflags += '-DUSE_CODEC_VORBIS'
    endif
else
    if ogg_dep.found() and tremor_dep.found()
        srcs += 'Quake/snd_vorbis.c'
        deps += [ ogg_dep, tremor_dep ]
        cflags += [ '-DUSE_CODEC_VORBIS', '-DVORBIS_USE_TREMOR' ]
    endif
endif

if opus_dep.found() and opusfile_dep.found()
    srcs += 'Quake/snd_opus.c'
    deps += [ opus_dep, opusfile_dep ]
    cflags += '-DUSE_CODEC_OPUS'
endif

# codec-availability cargo features mirror the USE_CODEC_* decisions just
# made, so the Rust codec registry (Phase 4, ADR-014) registers exactly
# the wrappers this build links
if '-DUSE_CODEC_WAVE' in cflags
    rust_feature_list += 'codec-wave'
endif
if '-DUSE_CODEC_MP3' in cflags
    rust_feature_list += 'codec-mp3'
endif
if '-DUSE_CODEC_FLAC' in cflags
    rust_feature_list += 'codec-flac'
endif
if '-DUSE_CODEC_VORBIS' in cflags
    rust_feature_list += 'codec-vorbis'
endif
if '-DUSE_CODEC_OPUS' in cflags
    rust_feature_list += 'codec-opus'
endif
rust_features = ','.join(rust_feature_list)
rust_lib_name = host_machine.system() == 'windows' ? 'quake_rs.lib' : 'libquake_rs.a'
quake_rs = custom_target(
    'quake_rs',
    output : [rust_lib_name, 'quake_rs.h'],
    command : [
        python3, meson.project_source_root() / 'scripts/cargo_build.py',
        '--cargo', cargo,
        '--manifest-path', meson.project_source_root() / 'rust/Cargo.toml',
        '--target-dir', meson.project_build_root() / 'cargo-target',
        '--profile', rust_profile,
        '--output', '@OUTPUT0@',
        '--depfile', '@DEPFILE@',
        '--features', rust_features,
        '--cbindgen', cbindgen,
        '--header-output', '@OUTPUT1@',
    ],
    depfile : 'quake_rs.d',
    # the quake-capi build script compiles the shaders with the same
    # tools Meson found (Phase 8 M11); env is inherited through cargo
    env : {'QUAKE_GLSLANG' : glslang.full_path(), 'QUAKE_SPIRV_OPT' : spirv_opt.full_path()},
    console : true,
    # cargo is its own (fast) change detector; always let it decide
    build_always_stale : true,
)
# link_with the staticlib; sources orders C compilation after quake_rs.h
# generation (the header lands in the build root, which is on the include
# path implicitly)
deps += declare_dependency(link_with : quake_rs[0], sources : quake_rs[1])

if get_option('buildtype').startswith('debug')
    cflags += '-D_DEBUG'
endif

if get_option('do_userdirs').enabled()
    cflags += '-DDO_USERDIRS=1'
endif

incdirs = [
    'Quake/mimalloc',
]

executable(
    'vkqr-engine',
    srcs,
    dependencies : deps,
    include_directories : incdirs,
    c_args : cflags,
    c_pch : 'Quake/quakedef.h',
    win_subsystem : 'windows')
