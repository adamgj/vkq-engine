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
