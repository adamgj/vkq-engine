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
