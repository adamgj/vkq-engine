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

shaders = [
    'Shaders/alias.vert',
    'Shaders/md5.vert',
    'Shaders/basic.vert',
    'Shaders/basic_alphatest.frag',
    'Shaders/basic_notex.frag',
    'Shaders/cs_tex_warp.comp',
    'Shaders/indirect.comp',
    'Shaders/indirect_clear.comp',
    'Shaders/postprocess.frag',
    'Shaders/postprocess.vert',
    'Shaders/showtris.frag',
    'Shaders/showtris.vert',
    'Shaders/sky_box.frag',
    'Shaders/sky_cube.frag',
    'Shaders/sky_cube.vert',
    'Shaders/sky_layer.frag',
    'Shaders/sky_layer.vert',
    'Shaders/world.vert',
    'Shaders/ray_debug.comp',
    'Shaders/mesh_interpolate.comp',
    'Shaders/skinning.comp',
]

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
run_test_canonicalize_ids_spirv_opt = run_command(spirv_opt, '-h', check : false)
has_canonicalize_ids_spirv_opt = (run_test_canonicalize_ids_spirv_opt.stdout().strip().contains('--canonicalize-ids'))
bintoc = executable('bintoc', ['Shaders/bintoc.c'], native: true)
mkpak = executable('mkpak', ['Misc/vq_pak/mkpak.c'], native: true)

if (build_machine.system() == 'darwin') or get_option('buildtype').startswith('debug')
    spirv_opt_args = [] # MoltenVK spirv-cross has a bug that breaks optimized shaders
else
    if not has_canonicalize_ids_spirv_opt
        error('spirv-opt does not support --canonicalize-ids, a newer Vulkan SDK is required')
    endif
    spirv_opt_args = ['-Os', '--canonicalize-ids', '--strip-debug']
endif

glslang_args = ['-V', '--quiet']
if get_option('buildtype').startswith('debug')
    glslang_args += '-g'
endif

vq_pak_path = join_paths(meson.project_source_root(), 'Misc', 'vq_pak')
mkpak_toc_file = join_paths(vq_pak_path, 'vq_pak_contents.txt')

mkpak_command = [mkpak, '@OUTPUT@', vq_pak_path, mkpak_toc_file, '@DEPFILE@']

run_mkpak = custom_target('run-mkpak',
  output : 'vkquake.pak',
  depfile : 'vkquake.pak.d',
  command: mkpak_command)

# Write the embedded pak into the build dir so Meson can track it.
bintoc_compress_command = [bintoc, '-c', '@INPUT@', 'vkquake.pak', '@OUTPUT@']

run_generate_embedded_pak = custom_target('run-embed_vkquake_pak',
							output : 'embedded_pak.c',
							input : run_mkpak,
							command : bintoc_compress_command)

# shader sources compiled multiple times with different preprocessor defines
shader_variants = [
    ['Shaders/basic.frag', 'basic.frag', []],
    ['Shaders/basic.frag', 'basic_oit.frag', ['-DWBOIT=1']],
    ['Shaders/basic.frag', 'basic_mboit_moment.frag', ['-DMBOIT=1']],
    ['Shaders/basic.frag', 'basic_mboit_composite.frag', ['-DMBOIT=1', '-DMBOIT_COMPOSITE=1']],
    ['Shaders/basic.frag', 'basic_mboit_composite_msaa.frag', ['-DMBOIT=1', '-DMBOIT_COMPOSITE=1', '-DMSAA=1']],
    ['Shaders/world.frag', 'world.frag', []],
    ['Shaders/world.frag', 'world_oit.frag', ['-DWBOIT=1']],
    ['Shaders/world.frag', 'world_mboit_moment.frag', ['-DMBOIT=1']],
    ['Shaders/world.frag', 'world_mboit_composite.frag', ['-DMBOIT=1', '-DMBOIT_COMPOSITE=1']],
    ['Shaders/world.frag', 'world_mboit_composite_msaa.frag', ['-DMBOIT=1', '-DMBOIT_COMPOSITE=1', '-DMSAA=1']],
    ['Shaders/alias.frag', 'alias.frag', []],
    ['Shaders/alias.frag', 'alias_alphatest.frag', ['-DALIAS_ALPHA_TEST=1']],
    ['Shaders/alias.frag', 'alias_oit.frag', ['-DWBOIT=1']],
    ['Shaders/alias.frag', 'alias_alphatest_oit.frag', ['-DALIAS_ALPHA_TEST=1', '-DWBOIT=1']],
    ['Shaders/alias.frag', 'alias_mboit_moment.frag', ['-DMBOIT=1']],
    ['Shaders/alias.frag', 'alias_alphatest_mboit_moment.frag', ['-DALIAS_ALPHA_TEST=1', '-DMBOIT=1']],
    ['Shaders/alias.frag', 'alias_mboit_composite.frag', ['-DMBOIT=1', '-DMBOIT_COMPOSITE=1']],
    ['Shaders/alias.frag', 'alias_alphatest_mboit_composite.frag', ['-DALIAS_ALPHA_TEST=1', '-DMBOIT=1', '-DMBOIT_COMPOSITE=1']],
    ['Shaders/alias.frag', 'alias_mboit_composite_msaa.frag', ['-DMBOIT=1', '-DMBOIT_COMPOSITE=1', '-DMSAA=1']],
    ['Shaders/alias.frag', 'alias_alphatest_mboit_composite_msaa.frag', ['-DALIAS_ALPHA_TEST=1', '-DMBOIT=1', '-DMBOIT_COMPOSITE=1', '-DMSAA=1']],
    ['Shaders/alias.frag', 'md5_mboit_composite.frag', ['-DMBOIT=1', '-DMBOIT_COMPOSITE=1', '-DMBOIT_INPUT_SET=4']],
    ['Shaders/alias.frag', 'md5_alphatest_mboit_composite.frag', ['-DALIAS_ALPHA_TEST=1', '-DMBOIT=1', '-DMBOIT_COMPOSITE=1', '-DMBOIT_INPUT_SET=4']],
    ['Shaders/alias.frag', 'md5_mboit_composite_msaa.frag', ['-DMBOIT=1', '-DMBOIT_COMPOSITE=1', '-DMSAA=1', '-DMBOIT_INPUT_SET=4']],
    ['Shaders/alias.frag', 'md5_alphatest_mboit_composite_msaa.frag', ['-DALIAS_ALPHA_TEST=1', '-DMBOIT=1', '-DMBOIT_COMPOSITE=1', '-DMSAA=1', '-DMBOIT_INPUT_SET=4']],
    ['Shaders/md5.vert', 'md5_8.vert', ['-DEIGHT_WEIGHT_SKINNING']],
    ['Shaders/wboit_resolve.frag', 'wboit_resolve.frag', []],
    ['Shaders/wboit_resolve.frag', 'wboit_resolve_msaa.frag', ['-DMSAA=1']],
    ['Shaders/mboit_resolve.frag', 'mboit_resolve.frag', []],
    ['Shaders/mboit_resolve.frag', 'mboit_resolve_msaa.frag', ['-DMSAA=1']],
    ['Shaders/screen_effects.comp', 'screen_effects_8bit.comp', []],
    ['Shaders/screen_effects.comp', 'screen_effects_8bit_scale.comp', ['-DSCALING=1']],
    ['Shaders/screen_effects.comp', 'screen_effects_8bit_scale_sops.comp', ['--target-env', 'vulkan1.1', '-DSCALING=1', '-DUSE_SUBGROUP_OPS=1']],
    ['Shaders/screen_effects.comp', 'screen_effects_10bit.comp', ['-DUSE_10BIT=1']],
    ['Shaders/screen_effects.comp', 'screen_effects_10bit_scale.comp', ['-DUSE_10BIT=1', '-DSCALING=1']],
    ['Shaders/screen_effects.comp', 'screen_effects_10bit_scale_sops.comp', ['--target-env', 'vulkan1.1', '-DUSE_10BIT=1', '-DSCALING=1', '-DUSE_SUBGROUP_OPS=1']],
    ['Shaders/update_lightmap.comp', 'update_lightmap_8bit.comp', []],
    ['Shaders/update_lightmap.comp', 'update_lightmap_8bit_rt.comp', ['-DRAY_QUERIES=1']],
    ['Shaders/update_lightmap.comp', 'update_lightmap_10bit.comp', ['-DUSE_10BIT=1']],
    ['Shaders/update_lightmap.comp', 'update_lightmap_10bit_rt.comp', ['-DUSE_10BIT=1', '-DRAY_QUERIES=1']],
    ['Shaders/skinning.comp', 'skinning_8.comp', ['-DEIGHT_WEIGHT_SKINNING']],
]

fs = import('fs')

shader_jobs = []
foreach shader : shaders
    target_env = shader.contains('sops') ? ['--target-env', 'vulkan1.1'] : []
    shader_jobs += [[shader, fs.name(shader), target_env]]
endforeach
foreach variant : shader_variants
    shader_jobs += [[variant[0], variant[1], variant[2]]]
endforeach

shaders_c = []
foreach job : shader_jobs
    source = job[0]
    name = job[1]
    args = glslang_args + job[2]
    if spirv_opt_args.length() > 0
        spv = custom_target(name.underscorify() + '_raw_spv',
            input : source,
            output : name + '.raw.spv',
            depfile : name + '.deps',
            command : [glslang] + args + ['-o', '@OUTPUT@', '@INPUT@', '--depfile', '@DEPFILE@'])
        spv = custom_target(name.underscorify() + '_spv',
            input : spv,
            output : name + '.spv',
            command : [spirv_opt] + spirv_opt_args + ['@INPUT@', '-o', '@OUTPUT@'])
    else
        spv = custom_target(name.underscorify() + '_spv',
            input : source,
            output : name + '.spv',
            depfile : name + '.deps',
            command : [glslang] + args + ['-o', '@OUTPUT@', '@INPUT@', '--depfile', '@DEPFILE@'])
    endif
    shaders_c += custom_target(name.underscorify() + '_c',
        input : spv,
        output : name + '.c',
        command : [bintoc, '@INPUT@', name + '_spv', '@OUTPUT@'])
endforeach

srcs = [
    'Quake/cd_null.c',
    'Quake/common.c',
    'Quake/gl_draw.c',
    'Quake/gl_fog.c',
    'Quake/gl_heap.c',
    'Quake/gl_mesh.c',
    'Quake/gl_model.c',
    'Quake/gl_refrag.c',
    'Quake/gl_rlight.c',
    'Quake/gl_rmain.c',
    'Quake/gl_rmisc.c',
    'Quake/gl_screen.c',
    'Quake/gl_sky.c',
    'Quake/gl_texmgr.c',
    'Quake/gl_vidsdl.c',
    'Quake/gl_warp.c',
    'Quake/harness.c',
    'Quake/q_thread_sdl.c',
    'Quake/pr_trace.c',
    'Quake/image.c',
    'Quake/image_stb.c',
    'Quake/in_sdl.c',
    'Quake/in_sdl2.c',
    'Quake/in_sdl3.c',
    'Quake/main_sdl.c',
    'Quake/mem.c',
    'Quake/net_loop.c',
    'Quake/net_main.c',
    'Quake/palette.c',
    'Quake/pr_cmds.c',
    'Quake/pr_edict.c',
    'Quake/pr_ext.c',
    'Quake/r_alias.c',
    'Quake/r_brush.c',
    'Quake/r_part.c',
    'Quake/r_part_fte.c',
    'Quake/r_sprite.c',
    'Quake/r_world.c',
    'Quake/steam_api.c',
	'Quake/sys_sdl.c',
    'Quake/tasks.c',
	 run_generate_embedded_pak,
]

# Phase 3 (Rust migration): C format/asset parsing split out of gl_model.c and
# image.c. model_parse.c is always compiled -- -Duse_rust_formats only
# preprocesses out the ported loaders (Mod_SetExtraFlags and the alias scratch
# arrays gl_mesh.c reads stay C) and adds the Host_Error trampolines;
# image_decode.c (PCX/LMP) is gated by -Duse_rust_image below and stays the
# comparison oracle.
# image_stb.c is always compiled: under -Duse_rust_image (USE_RUST_IMAGE) it
# keeps only Image_DecodeSTBMem, the in-memory stb fallback/oracle the Rust
# Image_DecodeSTB routes crate-undecoded formats through (Phase 3 M8).
phase3_formats_c_srcs = [
    'Quake/model_parse.c',
]
phase3_formats_glue_c_srcs = [
    'Quake/model_parse_glue.c',
]
phase3_image_c_srcs = [
    'Quake/image_decode.c',
]

# Phase 2 (Rust migration): the C filesystem, compiled unless -Duse_rust_fs
# provides these symbols from quake-fs/quake-capi instead. Deleted at phase
# exit (ROADMAP.md Phase 2).
phase2_c_srcs = [
    'Quake/common_fs.c',
    'Quake/steam.c',
]

# Phase 1 (Rust migration): C originals of modules ported to rust/. Compiled
# only when use_rust is disabled; the mixed build resolves their symbols from
# libquake_rs.a instead. Deleted at phase exit (ROADMAP.md Phase 1).
phase1_c_srcs = [
    'Quake/cfgfile.c',
    'Quake/crc.c',
    'Quake/hash_map.c',
    'Quake/json.c',
    'Quake/mathlib.c',
    'Quake/mdfour.c',
    'Quake/strlcat.c',
    'Quake/strlcpy.c',
    'Quake/wad.c',
]

if host_machine.system() == 'windows'
    srcs += [
        'Quake/net_win.c',
        'Quake/net_wins.c',
        'Quake/pl_win.c',
        'Quake/sys_sdl_win.c',
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
        'Quake/net_udp.c',
        'Quake/pl_linux.c',
        'Quake/sys_sdl_unix.c',
    ]
endif

cflags = cc.get_supported_arguments('-Wno-trigraphs', '-Wno-microsoft-enum-forward-reference') + ['-D_FILE_OFFSET_BITS=64']
if cc.get_id() == 'clang' or cc.get_id() == 'clang-cl'
    # the stb single header libraries define their whole API static
    # (clang-cl reports its id as 'clang-cl', so it needs the same suppression)
    cflags += cc.get_supported_arguments('-Wno-unused-function')
endif

cargo = find_program('cargo', required : get_option('use_rust'))
use_rust = cargo.found() and not get_option('use_rust').disabled()
# Phase 2: first per-module switch; the C filesystem stays the comparison
# oracle under -Duse_rust_fs=disabled. 'auto' follows use_rust; asking for it
# explicitly without the Rust staticlib is a contradiction, not a downgrade
if get_option('use_rust_fs').enabled() and not use_rust
    error('-Duse_rust_fs=enabled requires -Duse_rust=enabled (the filesystem ships in libquake_rs.a)')
endif
use_rust_fs = use_rust and not get_option('use_rust_fs').disabled()
# Phase 3 M2: PCX/LMP decoders; image_decode.c stays the comparison oracle
# under -Duse_rust_image=disabled. Same auto-follows-use_rust rule as above
if get_option('use_rust_image').enabled() and not use_rust
    error('-Duse_rust_image=enabled requires -Duse_rust=enabled (the image decoders ship in libquake_rs.a)')
endif
use_rust_image = use_rust and not get_option('use_rust_image').disabled()
# Phase 3 M3: brush/BSP loaders; the brush half of model_parse.c stays the
# comparison oracle under -Duse_rust_formats=disabled. Same
# auto-follows-use_rust rule as above
if get_option('use_rust_formats').enabled() and not use_rust
    error('-Duse_rust_formats=enabled requires -Duse_rust=enabled (the brush/BSP loaders ship in libquake_rs.a)')
endif
use_rust_formats = use_rust and not get_option('use_rust_formats').disabled()
# Phase 4: sound engine; the C snd_* files stay the comparison oracle under
# -Duse_rust_snd=disabled. Same auto-follows-use_rust rule as above
if get_option('use_rust_snd').enabled() and not use_rust
    error('-Duse_rust_snd=enabled requires -Duse_rust=enabled (the sound engine ships in libquake_rs.a)')
endif
use_rust_snd = use_rust and not get_option('use_rust_snd').disabled()
# Phase 5: networking wire layer; the C originals stay the comparison oracle
# under -Duse_rust_net=disabled. Same auto-follows-use_rust rule as above
if get_option('use_rust_net').enabled() and not use_rust
    error('-Duse_rust_net=enabled requires -Duse_rust=enabled (the networking wire layer ships in libquake_rs.a)')
endif
use_rust_net = use_rust and not get_option('use_rust_net').disabled()
# Phase 6: the progs VM; the C pr_*.c originals stay the comparison oracle
# under -Duse_rust_progs=disabled -- and here that is load-bearing beyond
# convention: the ADR-019 gate-3 trace differ needs a C build to diff against.
# Same auto-follows-use_rust rule as above
if get_option('use_rust_progs').enabled() and not use_rust
    error('-Duse_rust_progs=enabled requires -Duse_rust=enabled (the progs VM ships in libquake_rs.a)')
endif
use_rust_progs = use_rust and not get_option('use_rust_progs').disabled()
# Phase 7: the cvar/command registry and the client/server simulation; the C
# originals stay the comparison oracle under -Duse_rust_cvar=disabled /
# -Duse_rust_host=disabled. Same auto-follows-use_rust rule as above
if get_option('use_rust_cvar').enabled() and not use_rust
    error('-Duse_rust_cvar=enabled requires -Duse_rust=enabled (the cvar/command registry ships in libquake_rs.a)')
endif
use_rust_cvar = use_rust and not get_option('use_rust_cvar').disabled()
if get_option('use_rust_host').enabled() and not use_rust
    error('-Duse_rust_host=enabled requires -Duse_rust=enabled (the client/server simulation ships in libquake_rs.a)')
endif
use_rust_host = use_rust and not get_option('use_rust_host').disabled()
# the Rust filesystem does not implement the DO_USERDIRS lookup branches yet
# (quake-capi fails this combination with a compile_error!); say so here
# rather than several minutes into a cargo build
if use_rust_fs and get_option('do_userdirs').enabled()
    error('-Ddo_userdirs=enabled is not supported by the Rust filesystem yet; build with -Duse_rust_fs=disabled')
endif
if use_rust
    cbindgen = find_program('cbindgen', required : true)
    cflags += '-DUSE_RUST'
    python3 = find_program('python3')
    rust_profile = get_option('buildtype') == 'debug' ? 'debug' : 'release'
    # engine-alloc binds Rust's global allocator to the mimalloc that mem.c
    # amalgamates (ADR-013); only the mixed build links those mi_* symbols
    rust_feature_list = ['engine-alloc']
    # the ported code's debug-only diagnostics must track the C engine's
    # DEBUG/_DEBUG, not the Rust profile: -D_DEBUG is set for every debug*
    # buildtype while the Rust profile is release for debugoptimized
    if get_option('buildtype').startswith('debug')
        rust_feature_list += 'engine-debug'
    endif
    if use_rust_fs
        rust_feature_list += 'fs'
    endif

    if use_rust_image
        rust_feature_list += 'image'
    endif

    if use_rust_formats
        rust_feature_list += 'formats'
    endif
    if use_rust_snd
        rust_feature_list += 'snd'
    endif
    if use_rust_net
        rust_feature_list += 'net'
    endif
    if use_rust_progs
        rust_feature_list += 'progs'
    endif
    if use_rust_cvar
        rust_feature_list += 'cvar'
    endif
    if use_rust_host
        rust_feature_list += 'host'
    endif
    # Phase 7 M5: the server- and client-coupled pr_cmds.c/pr_ext.c builtins
    # and the ED_Parse* dispatchers straddle both strata -- the builtin tables
    # are progs, the sv/cl glue their cores call is host -- so the feature that
    # compiles them tracks the conjunction, mirroring the C-side PF_RSH gate.
    if use_rust_progs and use_rust_host
        rust_feature_list += 'progs-host'
    endif
    # cargo features tracking C defines the fs shims must agree with
    if use_sdl3
        rust_feature_list += 'sdl3'
    endif
    if get_option('do_userdirs').enabled()
        rust_feature_list += 'userdirs'
    endif
    # the custom_target itself is created after the codec-library probes
    # below, so the codec-availability cargo features can track the same
    # dependency decisions the C build makes (Phase 4, ADR-014)
else
    # C-only build: keep compiling the originals of the Rust-ported modules
    srcs += phase1_c_srcs
endif

if not use_rust_fs
    # the C filesystem stays in as long as the Phase 2 switch is off
    srcs += phase2_c_srcs
endif

# Phase 3: model_parse.c is always compiled; under -Duse_rust_formats its
# brush/BSP half is preprocessed out and the Host_Error trampolines that call
# the Rust loaders (PLAN.md 4.3) come in alongside it
srcs += phase3_formats_c_srcs

if use_rust_formats
    cflags += '-DUSE_RUST_FORMATS'
    srcs += phase3_formats_glue_c_srcs
endif

if use_rust_image
    cflags += '-DUSE_RUST_IMAGE'
else
    # the C PCX/LMP decoders stay in as long as the Phase 3 image switch is off
    srcs += phase3_image_c_srcs
endif

# Phase 4: the sound engine flips file-by-file; the list grows per milestone
# and the C originals stay the comparison oracle under -Duse_rust_snd=disabled
phase4_c_srcs = [
    'Quake/bgmusic.c',
    'Quake/snd_codec.c',
    'Quake/snd_dma.c',
    'Quake/snd_mem.c',
    'Quake/snd_mix.c',
    'Quake/snd_umx.c',
    'Quake/snd_wave.c',
]
phase4_glue_c_srcs = [
    'Quake/snd_glue.c',
]
# Phase 5: the networking wire layer. The MSG/SZ section lives in net_msg.c
# (split from common.c at M2) and swaps whole-file for the Rust exports plus
# the net_msg_glue.c Host_Error trampolines; the drivers flip per
# driver-vtable slot (net_bsd.c/net_win.c ifdefs) and cl_demo.c in-file. The
# C originals stay the -Duse_rust_net=disabled comparison oracle
if use_rust_net
    cflags += '-DUSE_RUST_NET'
    srcs += ['Quake/net_msg_glue.c', 'Quake/net_dgrm_glue.c', 'Quake/net_dgrm_orch_glue.c']
else
    srcs += ['Quake/net_msg.c', 'Quake/net_dgrm_rel.c', 'Quake/net_dgrm.c']
endif

# Phase 6: the progs VM. The flip is per-file and lands milestone by milestone
# (pr_exec.c at M3, then the pr_edict.c splits -- pr_edict_save.c at M4,
# pr_edict_parse.c at M5 and pr_edict_load.c at M6 -- then the builtin tables
# slot by slot). pr_edict.c
# itself never flips: the parts that move out are split into their own files
# first, so what remains is unconditionally C.
# The C originals stay the -Duse_rust_progs=disabled oracle -- and here that
# is load-bearing, because the ADR-019 gate-3 trace differ compares against
# them.
if use_rust_progs
    cflags += '-DUSE_RUST_PROGS'
    srcs += [
        'Quake/pr_exec_glue.c',
        'Quake/pr_edict_save_glue.c',
        'Quake/pr_edict_parse_glue.c',
        'Quake/pr_edict_load_glue.c',
        'Quake/pr_edict_arena_glue.c',
        'Quake/pr_cmds_glue.c',
    ]
else
    srcs += [
        'Quake/pr_exec.c',
        'Quake/pr_edict_save.c',
        'Quake/pr_edict_parse.c',
        'Quake/pr_edict_load.c',
        'Quake/pr_edict_arena.c',
    ]
endif

# Phase 7 M2: the cvar/command registry. Whole-file Pattern A swap: the Rust
# exports replace cvar.c/cmd.c, with cvar_cmd_glue.c supplying the C-visible
# data symbols, the Host_Guard trampolines for command handlers and cvar
# callbacks, and the Host_Reraise wrappers (ADR-009). The C originals stay the
# -Duse_rust_cvar=disabled comparison oracle.
if use_rust_cvar
    cflags += '-DUSE_RUST_CVAR'
    srcs += ['Quake/cvar_cmd_glue.c']
else
    srcs += ['Quake/cvar.c', 'Quake/cmd.c']
endif
# Phase 7 M3: world.c. Same whole-file Pattern A swap: the Rust exports
# replace world.c, with world_glue.c supplying the two C-visible cvars and
# the ADR-009 raise topology. Six of world.c's entry points are raise-capable
# -- SV_LinkEdict reaches PR_ExecuteProgram, and SV_HullForEntity's two
# Con_Warning sites reach Host_Error through PR_GetString
# (pr_edict_arena.c:315), which makes SV_ClipMoveToEntity, SV_Move,
# SV_TestEntityPosition and SV_PointContentsAllBsps raise-capable in turn --
# so each is a reraising C wrapper here over a quake_rs_* status core, and
# the Host_Guard trampolines around every C callee that can longjmp live in
# the glue. No longjmp crosses a Rust frame. The glue also carries the
# cl.entities accessors World_ClipToNetwork needs until the M7 client port
# gives `cl` an ADR-011 mirror. The C original stays the
# -Duse_rust_host=disabled comparison oracle.
if use_rust_host
    cflags += '-DUSE_RUST_HOST'
    srcs += ['Quake/world_glue.c']
else
    srcs += ['Quake/world.c']
endif

# Phase 7 M4: sv_move.c + sv_phys.c, riding the same use_rust_host switch.
# Nine of their twelve entry points are raise-capable, so each is a reraising
# C wrapper in the glue over a quake_rs_* status core, exactly as M3 did for
# world.c; SV_FixCheckBottom, SV_CloseEnough and SV_PushGridEntityLinked are
# plain Rust exports. sv_phys_glue.c also owns the twelve physics cvars
# (sv_main.c registers them by address), sv_analyticphysics_frame (read by
# sv_user.c, C until M6) and the sv_speeds_* counters (read and zeroed by
# host.c). The C originals stay the -Duse_rust_host=disabled oracle.
if use_rust_host
    srcs += ['Quake/sv_move_glue.c', 'Quake/sv_phys_glue.c']
else
    srcs += ['Quake/sv_move.c', 'Quake/sv_phys.c']
endif

# Phase 7 M5: the glue behind the server- and client-coupled pr_cmds.c /
# pr_ext.c builtins and the ED_Parse* key dispatchers. Pattern C, so unlike M3
# and M4 there is no whole-file swap here: pr_cmds.c, pr_ext.c and pr_edict.c
# stay compiled in every configuration and the flip happens slot by slot
# through PF_RSH, which expands to the Rust export only when USE_RUST_PROGS
# and USE_RUST_HOST are both defined. These files only *define* the
# PRBI_*Glue_* / PREdictDispatch_Glue_* trampolines the Rust cores call back
# into -- they reference no quake_rs_* symbol themselves -- so riding
# use_rust_host alone is link-safe in all four progs/host combinations.
if use_rust_host
    srcs += [
        'Quake/pr_cmds_sv_glue.c',
        'Quake/pr_cmds_sv_msg_glue.c',
        'Quake/pr_cmds_sv_fx_glue.c',
        'Quake/pr_cmds_cl_glue.c',
        'Quake/pr_edict_dispatch_glue.c',
    ]
endif

# Phase 7 M6: sv_main.c, sv_send.c and sv_user.c -- the server simulation's
# top half. Pattern A whole-file swap on use_rust_host, as M3 and M4 did.
# T6.1 first split sv_main.c into a connection half (sv_main.c) and an entity
# send half (sv_send.c); both halves plus sv_user.c port here.
#
# Glue ownership: sv_main_glue.c owns sv_protocol/sv_protocol_pext1/pext2 and
# the SV_ConnectClient reraise; sv_user_glue.c owns the five sv_user.c cvars
# (sv_edgefriction, sv_idealpitchscale, sv_altnoclip, sv_maxspeed,
# sv_accelerate), which sv_main.c registers by address; sv_send_glue.c owns no
# cvars -- sv_send.c declared none -- and exists for the ADR-009 guards alone.
# server_t sv and server_static_t svs are now Rust-owned (quake-capi
# sv_main.rs), closing the last ADR-007 sv/svs dual view; server.h's extern
# declarations are unchanged and 35 C translation units keep reading them.
#
# SV_ConnectClient is statusized on Host_Guard's own 0/1/2 set, propagated
# verbatim -- that is the ADR-009 precondition M9's NET funnel flips depend on.
# The C originals stay the -Duse_rust_host=disabled oracle.
if use_rust_host
    srcs += ['Quake/sv_main_glue.c', 'Quake/sv_send_glue.c', 'Quake/sv_user_glue.c']
else
    srcs += ['Quake/sv_main.c', 'Quake/sv_send.c', 'Quake/sv_user.c']
endif

# Phase 7 M7 T7.2: the client stratum's leaves -- view.c, chase.c, cl_input.c
# and cl_tent.c -- riding the same use_rust_host switch as M3/M4/M6.
#
# Glue ownership follows the sv_user_glue.c pattern: the cvar_t storage stays
# C-visible because other still-C translation units reach it by plain name.
# view_glue.c owns view.c's 30 cvars plus v_punchangles/v_blend/cshift_*
# (gl_screen.c, gl_rmain.c, menu.c and sbar.c read them); chase_glue.c owns the
# four chase cvars; cl_input_glue.c owns cl_input.c's nine cvars and the 17
# kbutton_t in_* objects (in_sdl.c and menu.c drive them, and cl_main_glue.c
# is what registers the cvars); cl_tent_glue.c owns cl_beams[],
# cl_temp_entities[] and num_temp_entities, which cl_demo.c, cl_main.c and
# host_cmd.c read. cl_tent.c's seven cl_sfx_* handles were file-static and move
# to Rust.
if use_rust_host
    srcs += ['Quake/chase_glue.c', 'Quake/cl_input_glue.c', 'Quake/cl_tent_glue.c', 'Quake/view_glue.c']
else
    srcs += ['Quake/chase.c', 'Quake/cl_input.c', 'Quake/cl_tent.c', 'Quake/view.c']
endif

# Phase 7 M7 T7.3: cl_parse.c -- the server-message parser, and the phase's
# largest ADR-009 surface. Pattern A whole-file swap on the same use_rust_host
# switch as M3/M4/M6/T7.2.
#
# cl_parse.c has 38 Host_Error/Host_EndGame sites, of which 34 are live: the
# three at :886/:890/:894 sit inside the #if 0 CL_KeepaliveMessage (:852-:915,
# no callers) and the UF_UNUSED2 Host_EndGame at :439 is unreachable because
# protocol.h:33 defines LERP_BANDAID unconditionally. cl_parse_glue.c mirrors
# the 34 live sites as CLPARSE_* status codes and re-issues each raise verbatim
# from ClParse_Raise, the PRBI_Raise pattern from pr_cmds_glue.c:309; the Rust
# core returns status and never jumps.
#
# Glue ownership: cl_parse_glue.c owns svc_strings[128] (cl_parse.c:30-88),
# external linkage in the original but declared in no header, and supplies the
# 21 Host_Guard trampolines. The five entry points keep their exact C
# signatures; CL_EntityNum and CL_ParseLocalSound are likewise header-less
# externals, and r_part_fte.c:156 re-declares CL_EntityNum locally and calls it
# at :4458, :5961 and :7251, so the plain name stays a real C function.
#
# The cl/cls ADR-007 row closed in T7.4: their storage now lives in
# rust/quake-capi/src/cl_main.rs. The C originals stay the
# -Duse_rust_host=disabled oracle.
if use_rust_host
    srcs += ['Quake/cl_parse_glue.c']
else
    srcs += ['Quake/cl_parse.c']
endif

# Phase 7 M7 T7.4: cl_main.c + cl_demo.c -- the client entry points, lerp and
# relink, and demo record/playback. Pattern A whole-file swap on the same
# use_rust_host switch as M3/M4/M6/T7.2/T7.3. This closes the ADR-007 cl/cls
# row: their storage moves to rust/quake-capi/src/cl_main.rs.
#
# cl_main.c has three live raise sites (:232, :983, :1108); cl_demo.c has none
# and exits only through Sys_Error, which aborts rather than longjmps.
#
# cl_main.c owns cl/cls and ~20 cvars; cl_maxpitch, cl_minpitch, cl_confirmquit,
# needs_relink and CL_GenerateRandomParticlePrecache are header-less externals
# that other TUs (cl_input.c, in_sdl.c, menu.c, view.c, cl_parse_glue.c) resolve
# by local re-declaration, so those stay real C objects in the glue.
if use_rust_host
    srcs += ['Quake/cl_main_glue.c']
else
    srcs += ['Quake/cl_main.c']
endif

if use_rust_host
    srcs += ['Quake/cl_demo_glue.c']
else
    srcs += ['Quake/cl_demo.c']
endif

# Phase 7 M8 T8.2: host.c -- the frame loop, timing accumulator, bring-up and
# shutdown orchestration. Pattern A whole-file swap on the same use_rust_host
# switch as M3/M4/M6/M7.
#
# The ADR-009 setjmp shell stays C: Host_Guard (host.c:302) and Host_Reraise
# (host.c:339) live in Quake/host_glue.c along with the _Host_Frame setjmp,
# which Host_Glue_FrameInner owns so no longjmp crosses a Rust frame.
#
# T8.2 closes no ADR-007 row: every host.c data object (host_parms, host_client,
# the 20 cvars, oldrealtime, host_colormap, the sv_speeds counters) keeps its C
# storage in the glue, because other TUs resolve them by local re-declaration.
if use_rust_host
    srcs += ['Quake/host_glue.c']
else
    srcs += ['Quake/host.c']
endif

# Phase 7 M8 T8.3: host_cmd.c -> quake-capi (Pattern A whole-file swap).
#
# Like T8.2, no ADR-007 row opens or closes: current_skill, the four filelist
# heads plus extralevels_sorted, and noclip_anglehack all keep C storage in the
# glue, and the ExtraMaps_* parsing thread stays C behind named seams
# (ADR-016).
if use_rust_host
    srcs += ['Quake/host_cmd_glue.c']
else
    srcs += ['Quake/host_cmd.c']
endif

# Phase 7 M10b: keys.c -> quake-capi (Pattern A whole-file swap), on the same
# use_rust_host switch as M3/M4/M6/M7/M8.
#
# Quake/keys_glue.c keeps every C-visible object keys.c defined -- the
# key_lines edit line, key_linepos/key_insert/key_blinktime, edit_line,
# history_line, key_dest, keybindings[], consolekeys[], menubound[],
# keydown[], key_tabhint[], chat_team and the keynames[] table -- because
# console.c, menu.c, sbar.c, in_sdl.c and the net/host code all resolve them
# by plain name. Only keys.c's six file-statics (the chat buffer and its
# length, key_inputgrab, and the three function-statics) move to Rust.
#
# The glue also carries the eleven ADR-009 Host_Guard trampolines the port
# needs (SCR_UpdateScreen, the six Con_* console entry points, M_Keydown,
# M_Charinput, M_ToggleMenu_f and VID_Toggle) and the Host_Reraise frames for
# the eight raise-capable entry points.
if use_rust_host
    srcs += ['Quake/keys_glue.c']
else
    srcs += ['Quake/keys.c']
endif

# Phase 7 M10c: console.c -> quake-capi (Pattern A whole-file swap), on the
# same use_rust_host switch as M3/M4/M6/M7/M8/M10b.
#
# Quake/console_glue.c keeps every C-visible object console.c defined -- the
# scrollback buffer and its geometry, the six con_* cvars, the notify ring,
# con_lastcenterstring, the rcon redirect buffer, con_mutex and
# key_tabpartial -- because keys.c, gl_screen.c, host.c and the net code
# resolve them by plain name.
#
# It also keeps the eight C-variadic entry points (Con_Printf and friends) so
# libc still formats console output byte for byte, Con_Printf's
# SCR_UpdateScreen tail (which reaches Mod_LoadModel -> Host_Error),
# Con_NotifyBox, the three ADR-009 Host_Guard trampolines (M_Menu_Main_f and
# the two tab-completion callbacks) and the Host_Reraise frames for
# Con_ToggleConsole_f and Con_TabComplete.
if use_rust_host
    srcs += ['Quake/console_glue.c']
else
    srcs += ['Quake/console.c']
endif

# Phase 7 M10d: sbar.c -> quake-capi (Pattern A whole-file swap), on the same
# use_rust_host switch as M3/M4/M6/M7/M8/M10b/M10c.
#
# Quake/sbar_glue.c keeps the four C-visible objects sbar.c defined --
# sb_showscores, sb_lines, fragsort[] and scoreboardlines -- because
# gl_screen.c writes and reads sb_lines, reads sb_showscores, and pr_ext.c
# reads fragsort/scoreboardlines by plain name. Everything else sbar.c
# defined at file scope is static (the qpic_t * handles, hipweapons[] and
# hudtype) and moves to Rust.
#
# It also keeps every plain Sbar_* name as a wrapper over a quake_rs_sbar_*
# core, because most of those prototypes mention cb_context_t * or qpic_t *,
# neither of which cbindgen can spell. Five of the wrappers re-raise:
# PR_ExecuteProgram is the file's only longjmp-capable callee (sbar.c:82,
# :864, :870, :1590) and the Rust side reaches it through the existing
# Host_Glue_PR_ExecuteProgram trampoline, so Host_Reraise is called only from
# the glue and no jump crosses a Rust frame.
if use_rust_host
    srcs += ['Quake/sbar_glue.c']
else
    srcs += ['Quake/sbar.c']
endif

# Phase 7 M10e: menu.c -> quake-capi (Pattern A whole-file swap), on the same
# use_rust_host switch as M3/M4/M6/M7/M8/M10b/M10c/M10d.
#
# Quake/menu_glue.c keeps the eight C-visible objects menu.c defined --
# m_state, m_return_state, m_entersound, m_is_quitting, vid_menucmdfn,
# vid_menukeyfn, m_return_onerror and m_return_reason[] -- because keys.c,
# gl_screen.c, host.c and the already-Rust net_dgrm orchestration read or
# write them by plain name. Everything else menu.c defined at file scope is
# static and moves to Rust.
#
# It also keeps every plain M_* name as a wrapper over a quake_rs_menu_*
# core, for the same cbindgen reason as M10d (cb_context_t * / qpic_t *).
# Four of the wrappers re-raise: M_ToggleMenu_f, M_UpdateMouse, M_Draw and
# M_Keydown. Ten Host_Guard trampolines live in the glue, so Host_Reraise is
# called only from the glue TU and no longjmp crosses a Rust frame. That
# includes Con_ToggleConsole_f, which has itself been a re-raising wrapper
# since M10c (console_glue.c:234) and therefore cannot be called directly
# from a Rust frame.
if use_rust_host
    srcs += ['Quake/menu_glue.c']
else
    srcs += ['Quake/menu.c']
endif

if use_rust_snd
    cflags += '-DUSE_RUST_SND'
    srcs += phase4_glue_c_srcs
    if not use_sdl3
        # ADR-017: the SDL2 audio backend stays C until a use_rust+SDL2 CI
        # leg exists to verify its Rust port (the file self-gates on USE_SDL3)
        srcs += 'Quake/snd_sdl.c'
    endif
else
    srcs += phase4_c_srcs + [ 'Quake/snd_sdl.c', 'Quake/snd_sdl3.c' ]
endif

deps = [
    cc.find_library('m', required : false),
    cc.find_library('dl', required : false),
    dependency('threads'),
    sdl_dep,
]

# (the quake_rs staticlib dependency is appended after the codec probes below)

if host_machine.system() == 'windows'
    win_syslibs = ['ws2_32', 'winmm', 'dbghelp', 'ole32', 'uuid']
    if use_rust
        # the Rust standard library on *-pc-windows-msvc needs these
        win_syslibs += ['advapi32', 'userenv', 'bcrypt', 'ntdll', 'synchronization']
    endif
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

# under -Duse_rust_snd the mp3 tag skipper is the Rust port (quake-capi
# exports mp3_skiptags); the decoder wrappers themselves stay C (ADR-014)
mp3tag_srcs = use_rust_snd ? [] : [ 'Quake/snd_mp3tag.c' ]
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

if use_rust
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
        console : true,
        # cargo is its own (fast) change detector; always let it decide
        build_always_stale : true,
    )
    # link_with the staticlib; sources orders C compilation after quake_rs.h
    # generation (the header lands in the build root, which is on the include
    # path implicitly)
    deps += declare_dependency(link_with : quake_rs[0], sources : quake_rs[1])
endif

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
    [srcs, shaders_c],
    dependencies : deps,
    include_directories : incdirs,
    c_args : cflags,
    c_pch : 'Quake/quakedef.h',
    win_subsystem : 'windows')
