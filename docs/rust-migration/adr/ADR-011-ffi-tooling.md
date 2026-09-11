# ADR-011: bindgen + cbindgen + hand-mirrored `repr(C)` ABI structs

**Status:** Accepted
**Date:** 2026-08-16
**Tags:** —

## Context

The transition period needs C→Rust bindings (Rust calling remaining C) and Rust→C bindings (C calling ported Rust). Three sources of truth are possible for shared types: bindgen-generated, cbindgen-generated, or hand-written mirrors. Compat-critical structs (`entvars_t`, `dprograms_t`, `dstatement_t`, `ddef_t`, `dfunction_t`, net message layouts, BSP/MDL/SPR lumps) must remain correct **after** the C headers are deleted, and carry invariants a generator cannot express.

## Decision

- **C→Rust:** a single `quake-c-sys` crate runs **bindgen** at build time over the Phase-0 split headers, with per-module allowlists and **layout tests enabled** (free static assertions). No other crate declares `extern "C"` imports of engine C symbols.
- **Rust→C:** a single `quake-capi` crate contains hand-written `#[no_mangle] extern "C"` shims; **cbindgen** generates `quake_rs.h` into the build directory. Shims replicate existing C signatures exactly so call sites change only their `#include`. Shims are deleted with their last C caller.
- **Compat-critical ABI structs:** hand-written `#[repr(C)]` mirrors in `quake-types`, each carrying:
  - `const` assertions for size and field offsets (`core::mem::offset_of!`), **per build profile** where layout differs (debug `edict_t` header);
  - doc comments stating the invariant and its consumer (savegame, wire, progs ABI);
  - a CI job that diffs the mirrors against bindgen output for as long as the C headers exist.
- Vulkan handles in mirrors use `ash::vk` types (they are `#[repr(transparent)]` over `u64`/pointers); a const assertion locks each such field's size/alignment.

## Consequences

- Exactly two FFI crates to audit; everything else is FFI-free by construction.
- Hand-mirrored structs survive header deletion with their invariants documented; the bindgen-diff job catches drift while both exist.
- Cost: mirrors are manual work for a bounded list of types; the list is exactly the compat-critical set, which deserves manual attention anyway.

## Amended (Phase 1, 2026-08-17)

- **Generation-time bindgen, committed output.** `quake-c-sys` bindings are
  produced by `scripts/gen_c_bindings.sh` (bindgen CLI over
  `bindings_wrapper.h` with an explicit allowlist) and committed; the
  `bindgen-smoke` CI job regenerates and diffs them (version-pinned bindgen).
  This preserves the single-source-of-truth property without making libclang
  a build prerequisite on every environment. Layout tests are disabled in the
  committed output (they would bake generation-host type sizes); layout
  coverage comes from quake-types' const asserts and the differential suite.
- **Non-engine C declarations.** C-standard-library symbols (libm, `strtod`,
  `snprintf` in tests) are hand-declared — `quake_c_sys::libm` with safe
  wrappers for the `forbid(unsafe_code)` crates (ADR-010) — since they are
  not engine headers. Engine globals whose C types are not portably
  representable (platform-dependent array lengths, e.g. `com_basedir`) live
  as hand-written externs in `quake_c_sys::manual`; thread-local engine
  globals are unreachable through bindgen and are read through
  behavior-neutral C accessor seams instead (`COM_ThreadFileSize`,
  `COM_ThreadFileFromPak`).
- **Signature parity gate.** `scripts/harness/check_capi_signatures.sh`
  compiles one TU including the cbindgen-generated `quake_rs.h` together with
  the original engine headers: any shim signature drift is a
  conflicting-declaration compile error. Shims whose exact C types cbindgen
  cannot express (struct tags, pointer-to-array parameters, opaque handles)
  are excluded from generation and hand-declared in cbindgen's
  `after_includes`, covered by the same gate.

## Amended (Phase 6 M1, 2026-08-27)

`progs.h` joins `net_defs.h` on the not-a-bindgen-clean-root list: it pulls
`pr_comp.h`, `progdefs.h` (→ `progdefs.q1`), `common.h` for `link_t` and
`protocol.h` for `entity_state_t`, and its `MAX_EDICTS`-sized free list comes
from `quakedef.h`. The progs ABI is therefore hand-written in
`quake-types::progs` — `dprograms_t`, `dstatement_t`, `ddef_t`, `dfunction_t`,
`globalvars_t`, `entvars_t`, `entity_state_t`, `link_t`, `prstack_t`,
`freelist_t`, `areanode_t`, `edict_t`'s fixed header, the three `pr_ext*`
structs, and the whole of `qcvm_t` — and verified per-platform by
`quake-ctest/tests/progs_abi.rs` against a probe compiled from the engine's
own headers.

`edict_t` carries a **per-build-profile** fork (`DEBUG`/`_DEBUG` prepends
three bookkeeping fields), so the mirror is gated on the `engine-debug` cargo
feature and the probe publishes `const.ENGINE_DEBUG`. The suite asserts the
two agree *before* checking any offset — a mismatch there would otherwise make
every subsequent assertion compare against the wrong C layout.

## Amended (Phase 8 M3, 2026-09-06)

`glquake.h` and `gl_heap.h` join the not-a-bindgen-root list: both include
`<vulkan/vulkan_core.h>` (and `glquake.h` pulls SDL through `q_stdinc.h`),
so the renderer ABI is hand-mirrored in `quake-types::render` as the phase
ports it. M3 adds `vulkan_memory_type_t`, `vulkan_memory_t` and
`glheapstats_t`; the `VkDeviceMemory` field is `ash::vk::DeviceMemory`
(`#[repr(transparent)]` over `u64`, the ADR's Vulkan-handle rule), which
matches the C typedef where `vulkan_core.h` sets
`VK_USE_64_BIT_PTR_DEFINES` (a pointer, 8 bytes on every 64-bit target) and
elsewhere (a `uint64_t`); only 64-bit targets have been checked, and a
32-bit leg that disagreed would take the task plan's D2 fallback (a `u64`
newtype). `quake-ctest/tests/render_abi.rs` checks every size and offset,
plus `sizeof (VkDeviceMemory)` and the enum values, against a probe
(`stubs/abi_probe.c`, `ctest_abi_render_lookup`) whose `glheapstats_t` is
the engine's own `gl_heap.h` but whose `vulkan_memory_t`,
`vulkan_memory_type_t` and `VkDeviceMemory` are the prelude's hand copies
of `glquake.h:178-190` and `vulkan_core.h` (the real headers pull in the
Vulkan SDK). Those three rows are therefore mirror-vs-copy; the check of
the mirror against the real `glquake.h` is a block of `COMPILE_TIME_ASSERT`s
in `Quake/gl_heap_glue.c` (sizes, offsets, enum values), which every
`-Duse_rust_render` build compiles with the SDK header in scope, so a drift
in `glquake.h` fails the mixed build rather than a test. The three C callees the Rust heap needs
(`R_AllocateVulkanMemory`, `R_FreeVulkanMemory`, `GL_SetObjectName`) are
hand externs in `quake-c-sys/src/render.rs` with `void *` parameters; the
typed side lives in `quake-capi/src/gl_heap.rs`, where the mirrors and the
`ash::vk` structs are in scope.

`glheap_t` and `glheapallocation_t` are opaque to C and are boxed Rust
`Heap`/`Allocation` values; cbindgen cannot spell the Vulkan and
`glquake.h` types their seven entry points take, so those declarations are
hand-written in `cbindgen.toml`'s preamble under `gl_heap.h`'s include
guard and cross-checked by `check_capi_signatures.sh` (the `tasks.h`
precedent from M2).

## Amended (Phase 8 M4, 2026-09-07)

`gltexture_t` (`gl_texmgr.h`) embeds `VkImage`/`VkImageView`/`VkFramebuffer`/`VkDescriptorSet` handles and `enum srcformat`, so it is a hand-mirrored `#[repr(C)] GlTexture` in `quake-types::render` (with `SrcFormat` and the `TEXPREF_*` bits). Two checks keep the mirror honest: the `quake-ctest` ABI probe (`render_abi.rs` + `abi_probe.c`) measures the 22 field offsets, the size, `sizeof (enum srcformat)`, the six `SRC_*` and the fourteen `TEXPREF_*` values against the prelude's copy of the header, and `Quake/gl_texmgr_glue.c` carries a `COMPILE_TIME_ASSERT` block over the real `gl_texmgr.h`/`vulkan_core.h` for the same layout, so a header change breaks the mixed build before it can reach a Rust write. The eleven `vulkan_globals` members `gl_texmgr.c` reads cross through `texmgr_glue_env_t` (glue) / `GlueEnv` (`quake-capi/src/gl_texmgr.rs`), size-asserted on both sides, until the M5 ownership flip made `vulkan_globals` itself a mirror (see the M5 amendment). The Vulkan loader entry points the texture manager calls are declared by hand in `quake-c-sys::render` as `extern "system"` (matching `VKAPI_CALL`) with handles as `u64`/`*mut c_void`; the `ash::vk` structs are built on the `quake-capi` side and passed as `*const c_void`.

## Amended (Phase 8 M5, 2026-09-10)

`vulkanglobals_t` (`glquake.h`) is the `#[repr(C)] VulkanGlobals` mirror in `quake-types::render`, with `dynbuffer_t`, `vulkan_pipeline_layout_t`, `vulkan_pipeline_t`, `vulkan_desc_set_layout_t` and `buffer_create_info_t` mirrored alongside it; `quake-capi/src/gl_rmisc.rs` exports the storage (ADR-007 row), so every C reader and writer sees the Rust-owned instance through the unchanged header. The mirror is checked two ways, as at M4: the `render_abi.rs`/`abi_probe.c` probe measures the five small structs against the prelude's now-faithful copies of their definitions, and `Quake/gl_rmisc_glue.c` carries a 152-line `COMPILE_TIME_ASSERT` block over the real `glquake.h`/`vulkan_core.h` for `vulkanglobals_t`'s size and every member offset (the size depends on `_DEBUG`, which maps to the cargo `engine-debug` feature). `vulkanglobals_t` and `cb_context_t` themselves stay compile-only approximations in `c_ref_prelude.h`: the prelude cannot spell the real Vulkan handle and function-pointer members without the SDK headers, so the real-header check for those two is the glue block, not the probe (D5 amendment in the plan). The `VkPipeline`/`VkPipelineLayout`/`VkDescriptorSetLayout` members are `ash::vk` handle types (`repr(transparent)` `u64`), the same rule as `VkDeviceMemory` at M3.
