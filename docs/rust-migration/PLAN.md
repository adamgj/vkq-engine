# vkqr-engine C → Rust Migration Plan

**Status:** Approved plan, pre-implementation
**Scope:** Convert as much of vkqr-engine's engine and game code to Rust as possible while maintaining **100% backwards compatibility** with game logic (QuakeC), art assets, savegames, demos, networking (client and server), mods, mission packs, and 2021 re-release content.
**Companion documents:** [ROADMAP.md](ROADMAP.md) (phases and exit criteria), [adr/](adr/README.md) (Architecture Decision Records).

---

## 1. Goals and non-goals

### Goals

1. **Compatibility first.** Every observable behavior that game content, mods, saves, demos, or remote peers can depend on is preserved exactly. The definition of "observable" is deliberately broad — see §6 (Compatibility surfaces).
2. **Maximize Rust.** The end state is a Rust engine (`main()` in Rust, Cargo-driven) with a small enumerated set of native C remnants linked in as isolated modules.
3. **Rust best practices and type safety** wherever they do not conflict with goal 1. Where they do conflict, compatibility wins and the exception is documented in an ADR (see [ADR-006](adr/ADR-006-edict-arena.md) for the flagship example).
4. **Cross-platform:** Windows, Linux, macOS — the same three platforms the C engine supports today, with the same Vulkan (MoltenVK on macOS) and SDL foundations.
5. **Continuously shippable.** At the end of every phase the engine builds and passes the full differential-verification suite on all three platforms. There is no big-bang cutover.

### Non-goals (during the migration)

- No renderer modernization (no dynamic rendering, no bindless, no VMA) until the port is complete — see [ADR-015](adr/ADR-015-renderer-port-then-modernize.md).
- No gameplay/physics "fixes." Bug-for-bug parity is a requirement, not an accident (e.g. `OP_DIV_F` performs raw float division with no zero guard — that stays).
- No cross-platform *floating-point* determinism beyond what the C engine has today — see [ADR-010](adr/ADR-010-determinism-policy.md).
- No new features while a subsystem is mid-port.

### Approved scope reductions

Confirmed with the project owner ([ADR-018](adr/ADR-018-dropped-features.md)):

- **IPX networking** (`net_wipx.c`, Windows-only) is removed.
- **Makefile-only music codecs** (mikmod, xmp, modplug — already absent from the Meson build) are removed.
- **The MSVC solution** (`Windows/VisualStudio/`) is retired; Meson (+ clang-cl) becomes the sole Windows build.

Everything else — including SDL2 support ([ADR-017](adr/ADR-017-sdl-policy.md)) — is preserved.

---

## 2. Strategy: hybrid incremental oxidation

Full rationale in [ADR-001](adr/ADR-001-migration-strategy.md). Summary:

- **Incremental in-place oxidation** is the delivery vehicle. A Cargo workspace (`rust/`) produces a single `staticlib` (`libquake_rs.a` / `quake_rs.lib`) which the existing Meson build links into the `vkquake` executable. Modules are ported bottom-up along the dependency graph. Each ported module lands behind a transitional Meson option (`-Duse_rust_<module>=true|false`) so C and Rust implementations can be A/B-diffed at runtime; once the verification gates pass and a soak period ends, the C file is **deleted** and the option removed. Options are time-boxed — dual implementations are a testing tool, not a support burden. *(Clarified in Phase 1: the Phase 1 leaves use the global `use_rust` switch as their transitional switch — the roadmap names no soak window for them and they are non-interacting, so the build-C-vs-build-Rust CI diff provides the A/B; per-module options begin at Phase 2 with `-Duse_rust_fs`.)*
- **Ownership inversion** happens late (Phase 9): `main()` moves to the Rust `quake-host` binary crate and the remaining C compiles as a static library that Rust links. Meson remains the build orchestrator until the C remnant is small enough for the `cc` crate, and cargo remains the Rust compiler driver throughout.
- **c2rust is an oracle, never a code base.** The Immunant ioquake3 translation proved c2rust can faithfully translate a Quake-family engine — and also that its output (raw pointers, `unsafe` everywhere, mechanical C idioms) cannot satisfy this project's type-safety requirement. We run c2rust per-subsystem into `tools/c2rust-oracle/` (a quarantined workspace member, never linked into any shipping binary) and use it for (a) resolving ambiguous C semantics (integer promotion, sequence points, float truncation) and (b) three-way differential fuzzing of the QuakeC interpreter and physics hull checks.
- **The pure-C build is the reference oracle.** It stays green in CI until Phase 9 completes; every differential test compares Rust output against C output from the same commit.

### Why not the alternatives

- *c2rust transpile-then-refactor:* refactoring transpiled code toward idiomatic Rust fights both the original design and the transpiler's artifacts; the intermediate states are unshippable; the result would violate the type-safety requirement for years of calendar time.
- *Clean-room rewrite (Richter/Seismon-style):* existing Rust Quake engines are protocol-15-only and incomplete; a from-scratch rewrite cannot demonstrate 100% compatibility incrementally and loses vkqr-engine's decade of accumulated compatibility fixes (QSS extensions, re-release support, Ironwail imports, this fork's pusher physics).

---

## 3. Build integration

- **Meson stays primary** through Phase 8. Meson ≥ 1.3 is already required. The Rust workspace builds via a `custom_target` invoking `cargo build --locked` with `--target-dir` inside the Meson build directory (falling back to Meson's rust module where it helps). The staticlib links into `vkquake` exactly as the generated shader/pak C files do today — the build already has precedent for generated inputs.
- **Windows:** Meson + clang-cl links the MSVC-ABI Rust staticlib (`x86_64-pc-windows-msvc`). *(Correction, Phase 0: no Windows Meson CI existed before this migration — `build-windows.yml` was pure MSBuild and the MinGW jobs use the GNU Makefiles; the Meson+clang-cl Windows job was created in Phase 0.)* The GNU Makefiles (`Quake/Makefile*`) gain a cargo step or are retired in favor of Meson at the maintainers' option once Meson covers their use cases; the MinGW/clangarm64 CI legs stay C-only (GNU-ABI Rust linkage is out of scope).
- **Shader pipeline** moves to the `xtask` crate in Phase 8: GLSL → `glslangValidator` → `spirv-opt` (same flags, same macOS skip) → `include_bytes!`, retiring `bintoc` and the 61 generated C translation units. The embedded pak (`mkpak` + `bintoc -c`) moves to `xtask` at the same time.
- **CI matrix** from Phase 0: {Windows, Linux, macOS} × {debug, release} × {C-only, mixed, (later) Rust-primary}, plus `cargo audit`, `cargo deny`, clippy, rustfmt, and the differential harness (§7). Existing workflows (`.github/workflows/build-*.yml`) are extended rather than replaced.

---

## 4. FFI boundary architecture

### 4.1 Phase-0 header surgery (pure C, before any Rust)

`Quake/quakedef.h` is a god-header (and PCH): via `q_stdinc.h` it pulls `SDL.h`, and directly pulls `vulkan_core.h`, `gl_model.h`, `gl_texmgr.h`, `glquake.h` into **every** translation unit. There is no engine/renderer seam to cut along today. Before any Rust lands:

1. Split out **`q_types.h`**: fixed-width scalars, `qboolean`, `vec3_t`, and other primitives — zero SDL/Vulkan includes.
2. Make the **wire/disk-format headers self-contained** (`protocol.h`, `modelgen.h`, `spritegn.h`, `bspfile.h` (already clean), pak/wad structs out of `common.h`/`wad.h`). These become the bindgen roots.
3. **Contain the SDL leak:** engine code (renderer, sound, `pr_ext.c`) uses `SDL_mutex`/`SDL_Thread` directly and `common.c` uses SDL file IO. Introduce `q_thread.h` (opaque `qmutex_t`/`qthread_t`/`qcond_t`/`qsem_t` wrappers) and funnel file IO through the existing `Sys_File*` seam so that only `quake-platform`-destined files include SDL headers.
4. Extract **`q_render_types.h`** for the structs that mix engine data with Vulkan handles (`gltexture_t`, `struct lightmap_s`, the Vulkan members of `qmodel_t`/`aliashdr_t`/`entity_t`), so non-render code can be compiled — and bound — without `vulkan_core.h`.

**Exit criterion:** every C TU compiles against either "core" headers (no SDL/Vulkan) or "platform/render" headers, and bindgen can process the core headers standalone. This refactor is behavior-neutral and validated by the untouched C build passing the full harness.

### 4.2 Binding tooling ([ADR-011](adr/ADR-011-ffi-tooling.md))

- **C → Rust:** one `quake-c-sys` crate runs **bindgen** over the split headers (per-module allowlists, layout tests enabled — they are free static asserts). All `extern "C"` declarations for engine C live here; no other crate declares externs to engine C.
- **Rust → C:** one `quake-capi` crate holds hand-written `#[no_mangle] extern "C"` shims whose header (`quake_rs.h`) is generated by **cbindgen** into the build dir. Shims match the existing C signatures exactly, so a C call site changes only by including the new header. Shims are deleted as their C callers are ported.
- **Compat-critical ABI structs** (`entvars_t`, `dprograms_t`/`dstatement_t`/`ddef_t`/`dfunction_t`, net message headers, BSP/MDL/SPR lumps, savegame-relevant layouts): hand-written `#[repr(C)]` mirrors in `quake-types`, each with `const` size/offset assertions, **plus** a CI job that diffs them against bindgen output while the C headers still exist. Hand-written because these mirrors must outlive the C headers and carry the documentation of every invariant (e.g. `edict_size = progs->entityfields*4 + sizeof(edict_t) - sizeof(entvars_t)`, rounded up to pointer alignment; debug builds add three fields to `edict_t` — layout differs by build profile and the asserts are per-profile).

### 4.3 The hard patterns

**Anchor global singletons** (`cl`, `cls`, `sv`, `svs`, `vulkan_globals`, `vid`, `r_refdef`, `mod_known`, `com_searchpaths`). Transition: they stay **C-owned**; Rust accesses them only through one audited module (`quake_c_sys::globals`) whose accessors carry `// SAFETY:` comments naming the synchronization argument (single-threaded host frame, or the specific task-graph phase that owns the data). When a subsystem's phase lands, ownership flips: Rust owns the (idiomatized where possible) struct and exports a C-layout view **only while** remaining C still touches it. The roadmap ordering minimizes these dual-view windows; [ADR-007](adr/ADR-007-singleton-ownership.md) enumerates each one and its retirement phase. End state: singletons are fields of a `Host` struct passed by `&mut`.

**Ambient `qcvm` pointer.** `qcvm_t *qcvm` is a global implicit receiver swapped by `PR_SwitchQCVM()` (two live VMs: `sv.qcvm` and `cl.qcvm`/CSQC), with all field access via macros (`G_FLOAT`, `EDICT_NUM`, …). Transition: the C global remains the boundary contract; Rust internals thread `&mut QcVm` explicitly, and the boundary shim resolves the ambient global exactly once per entry. End state: an RAII `QcVmGuard` reproduces `PR_SwitchQCVM` semantics — including the "switching while a VM is active is an error" assertion — for host-loop call paths, while pure-Rust paths pass `&mut QcVm`. [ADR-008](adr/ADR-008-ambient-qcvm.md).

**`setjmp`/`longjmp`.** `Host_Error`/`Host_EndGame` longjmp to `host_abortserver` (set in `_Host_Frame`) and `screen_error` (set in `SCR_UpdateScreen`). Hard rules, enforced by construction:

- A `longjmp` must never unwind a Rust frame (UB). A Rust panic must never cross into C (UB with `panic=abort` boundary rules; aborts at best).
- Every Rust function exported to C returns a status; a small C macro at the call site re-raises via `Host_Error` when needed, so the longjmp originates and lands entirely in C frames.
- Every C function that Rust calls which can `Host_Error` is wrapped in a C trampoline (`int Host_Guard(void (*fn)(void *), void *arg)`) that `setjmp`s locally and returns an error code. The trampoline list stays small because error-raising leaves are ported early.
- End state (Phase 9): errors are `Result<T, HostError>` (`HostError::{Error, EndGame, Abort}`) propagated to the Rust host loop, which performs today's longjmp-target behavior (abort server, drop to console). `setjmp` is deleted with its last C caller. [ADR-009](adr/ADR-009-error-handling.md).

**Thread-local globals** (`com_token`, `va()` ring buffers, `thread_stack_alloc_size`, task-worker index). Shims use `thread_local!`; idiomatic internals return owned values (`COM_Parse` returns the token; `va()` internals become `format!`). The thread-local shim survives only as long as C callers do.

**Function-pointer vtables** — the *best* seams in the codebase. `net_drivers[]`/`net_landrivers[]`, the sound codec registry (`snd_codec_t`), QC builtin tables, cvar/cmd callbacks: Rust implements a driver/codec/builtin as `extern "C"` functions registered into the existing C table, allowing one driver or one codec or one batch of builtins to be ported — and A/B-tested — at a time. The **builtin ordinal tables are ABI**: ordinals, the extension name-vs-number resolution order, the "don't clobber non-`PF_Fixme` slots" rule, and `PR_PatchRereleaseBuiltins` renumbering (−90→−73, −91→−23, −92→−24) are all preserved and covered by the trace oracle.

**Vulkan handles inside shared structs** (`entity_t.blas_data`, `qmodel_t.{blas,buffer,address}`, `aliashdr_t` buffers, `gltexture_t`, `struct lightmap_s`) and **`cb_context_t`** (a `VkCommandBuffer` wrapper threaded through the entire `Draw_*`/`R_*` API): during the split-renderer window these are `#[repr(C)]` mirrors using `ash::vk` types — ash handles are `#[repr(transparent)]` over `u64`/pointers, so layout compatibility holds and is locked by const asserts. Once the renderer is fully Rust (end of Phase 8), `CbContext` and friends become idiomatic types with frame-scoped lifetimes.

---

## 5. Workspace design

```
rust/                      # Cargo workspace
├── quake-types/           # #[repr(C)] wire/disk/ABI mirrors + const layout tests
├── quake-math/            # mathlib: vec3 ops, AngleVectors, BoxOnPlaneSide (bit-exact)
├── quake-util/            # crc16, folded MD4 (mdfour), strl*, q_ctype,
│                          #   C-printf-compatible float formatter (ADR-005)
├── quake-cvar/            # cvar/cmd/alias registry + callbacks (C-callable during transition)
├── quake-fs/              # PAK, WAD2, search paths, gamedir/flavor logic, .kpf zip,
│                          #   localization, Steam/GOG/EGS discovery
├── quake-formats/         # BSP29/30/BSP2/2PSB/Q64, .lit/.vis/.ent, MDL v6, MD3,
│                          #   MD5(+JSON metadata), SPR, LMP/QPIC, PCX, TGA
├── quake-image/           # decode/encode orchestration (png/image crates per ADR-012)
├── quake-progs/           # QCVM: loader, edict arena, interpreter, ED_Write/Parse, builtins
├── quake-net/             # MSG_* readers/writers, protocol 15/666/999 + PRFL + PEXT,
│                          #   loopback/dgrm/UDP drivers, CCREQ/CCREP + rcon, demo IO
├── quake-snd/             # mixer, snd_mem, dma, bgmusic, codec trait (C codecs behind it)
├── quake-render/          # ash: gl_heap suballocator, texmgr, pipelines, frame graph, RT
├── quake-tasks/           # work-stealing job system (crossbeam-deque), API-compatible
├── quake-platform/        # SDL2+SDL3 (feature-gated), input, video glue, sys/pl layers
├── quake-host/            # host loop; becomes the bin crate with main() in Phase 9
├── quake-c-sys/           # bindgen externs to remaining C (shrinks to empty)
├── quake-capi/            # cbindgen-exported shims to C (shrinks to empty)
└── xtask/                 # shader compilation, pak embedding, codegen, CI helpers
tools/
└── c2rust-oracle/         # quarantined transpiler output; differential testing only
```

### Dependency policy ([ADR-003](adr/ADR-003-dependency-policy.md))

Third-party crates must be widely used and actively maintained, with **no open high/critical advisories** — enforced in CI by `cargo audit` (RustSec) and `cargo deny` (advisories + license allowlist). `Cargo.lock` is committed; MSRV is pinned; each new direct dependency requires a short review note in its introducing PR (maintenance status, transitive footprint, alternatives considered).

**Licensing: permissive only, MIT preferred.** The allowlist is `MIT`, `MIT-0`, `Apache-2.0` (incl. `WITH LLVM-exception`), `BSD-2-Clause`, `BSD-3-Clause`, `Zlib`, `ISC`, `0BSD`, `Unicode-DFS-2016`, `Unicode-3.0`. Copyleft crates — `LGPL-*`, `GPL-*`, `AGPL-*`, `MPL-2.0`, `CDDL-*`, `EPL-*`, `EUPL-*`, `CC-BY-SA-*` — and crates needing a paid or commercial license (`BUSL-1.1`, `SSPL-*`, `Elastic-2.0`, "free for non-commercial use") are rejected, transitively as well as directly. GPLv2-compatibility alone is not enough: LGPL is compatible but pushes relinking and source-disclosure obligations onto downstream packagers, which this project won't take on. Exceptions require an ADR-003 amendment, not a per-PR waiver.

Expected direct dependencies: `ash` (Vulkan), `sdl2` + `sdl3` (feature-gated), `crossbeam-deque`/`crossbeam-utils`, `zip` or `flate2`+`miniz_oxide` (kpf/zip), `png` + `image` (or `zune-jpeg`), `libmimalloc-sys` (allocator, ADR-013), `bitflags`, `libc`; build/dev: `bindgen`, `cbindgen`, `cargo-fuzz`/`libfuzzer-sys`, `criterion`, `proptest`, `loom`.

### Unsafe policy ([ADR-004](adr/ADR-004-unsafe-policy.md))

- `#![forbid(unsafe_code)]` in pure crates: `quake-math`, `quake-util`, `quake-formats` (parsing), `quake-cvar`, `quake-fs` (logic), `quake-image` orchestration.
- Everywhere else: `#![deny(unsafe_op_in_unsafe_fn)]` + `clippy::undocumented_unsafe_blocks` — every `unsafe` block carries a `// SAFETY:` comment.
- Unsafe is concentrated in: `quake-c-sys`, `quake-capi`, `quake-render` (Vulkan is inherently unsafe at the ash level), `quake-tasks` (bounded, loom-tested), and the edict arena module of `quake-progs`.

### Error handling ([ADR-009](adr/ADR-009-error-handling.md))

Layered error enums (`ParseError`, `NetError`, `ProgsError`, …) all convertible into `HostError`; `Result` propagation replaces longjmp; panics indicate engine bugs (release builds use `panic = "abort"`; transition-period FFI shims are no-unwind by construction).

### The edict arena ([ADR-006](adr/ADR-006-edict-arena.md)) — flagship type-safety exception

Edicts cannot be Rust structs: their layout is a **runtime ABI** decided at progs load (`entityfields*4` + fixed engine header; `PR_MergeEngineFieldDefs` appends engine fields — `alpha`, `scale`, `emiteffectnum`, `traileffectnum`, `tag_entity`, `tag_index`, `modelflags`, `colormod` — mutating `entityfields` and therefore `edict_size` and savegame output; mod-defined fields extend past `entvars_t`; `EDICT_TO_PROG` byte offsets are serialized into savegames and the wire protocol; debug builds have a different header layout). Design:

- `EdictArena` owns one aligned byte buffer; `EdictId(u32)` newtype indices; no references into the arena escape a VM step.
- Typed accessors are generated from the merged fielddef table (`arena.f32(ed, fld)`, an `EntVars` view of getters/setters for engine-known fields).
- `string_t`/`func_t`/entity values remain raw `i32` exactly as the VM ABI requires — including negative engine-string indices and the raw-int `localstack` copies.
- Interpreter semantics preserved exactly: `OP_DIV_F` raw division, C float→int **truncation** in `OP_BITAND`/`OP_BITOR`, C float comparison semantics, `STRINGTEMP_BUFFERS 1024 × STRINGTEMP_LENGTH 1024` ring wraparound, runaway-loop counter.

### Allocator ([ADR-013](adr/ADR-013-allocator.md))

One allocator on both sides of the boundary: the vendored mimalloc build, with Rust's `#[global_allocator]` backed by `libmimalloc-sys` pointing at the same library. Any buffer whose ownership crosses the language boundary is allocated/freed via the `Mem_*` API. Revisit (pure-Rust allocator or system allocator) at Phase 10 when the boundary is gone.

---

## 6. Compatibility surfaces (the contract)

This is the checklist every phase's verification gates trace back to. Items marked **[byte]** must be byte-identical; **[behavior]** must be observationally identical.

1. **Progs VM** — `PROG_VERSION 6`, `PROGHEADER_CRC 5927`; CRC16 over progs bytes; folded-MD4 hash driving the `csprogsvers/%x.dat` CSQC lookup **[behavior]**; lump byteswap rules; load-time hash maps built in reverse to preserve linear first-match semantics for duplicate symbols **[behavior]**; builtin ordinal ABI (`pr_ssqcbuiltins`/`pr_csqcbuiltins`) and extension resolution (name-vs-number, don't-clobber) **[behavior]**; re-release builtin renumbering and `EF_QEX_*` effect-mask probing (Arcane Dimensions depends on it) **[behavior]**; interpreter arithmetic quirks (raw `OP_DIV_F`, float→int truncation, localstack raw copies, string-temp ring) **[behavior]**; negative engine `string_t` convention **[behavior]**; edict layout and `EDICT_TO_PROG` byte offsets **[byte, in saves/wire]**.
2. **Savegames & config** — `SAVEGAME_VERSION 5` text format: C `"%f"` float formatting via `PR_UglyValueString`, `ED_Write` field-skip rules (fielddef 0 skipped, `DEF_SAVEGLOBAL` skipped, `_x/_y/_z` skipped, zero values skipped, free edicts as `{\n}\n`), 39-char comment with spaces→`_`, QuakeSpasm extended `/* */` block, spawn-parm `%f` lines **[byte]**. `config.cfg` write order (optional `unbindall`, bindings, archived cvars incl. `seta`, trailing `+mlook`) and `cfgfile.c`'s exact early-parse line format **[byte]**. Load-compat both directions with C-era saves **[behavior]**.
3. **Networking** — protocols 15/666/999, `PRFL_*` coord/angle variants, FTE `PEXT1_CSQC`, `PEXT2_REPLACEMENTDELTAS|PREDINFO` (+ accepted-only bits); `MSG_*` encodings incl. varint u64, coord16/24/32f, `ENTALPHA`/`ENTSCALE` 4.4 fixed-point rounding **[byte]**; svc opcode set incl. re-release 38/45–56 vs DP/FTE collisions and their `cl_parse.c` disambiguation **[behavior]**; dgrm reliable layer (`NETFLAG_*`, header sizes) **[byte]**; CCREQ/CCREP connectionless protocol + rcon **[byte]**; loopback driver semantics; IPv6 (v6only, `ff03::1` multicast discovery) **[behavior]**; demo format (forcetrack line; length + 3 viewangle floats + payload records) **[byte]**; ProQuake angle-hack detection **[behavior]**.
4. **Physics & simulation** — `sv_phys.c` including this fork's pusher-support-frame subsystem (modes 0–3) and `sv_smoothplatformlerps`; `sv_user.c` movement (libm `sin`/`cos` call sites); `world.c` hull checks (`DIST_EPSILON` biasing; **both** implementations behind `sv_fte_recursivehullckeck`); `SV_TouchLinks`/area-node ordering and `ED_Alloc` free-list ordering (entity numbering is observable) **[behavior, per-platform bit-parity — ADR-010]**; `PF_random` RNG sequence; `BoxOnPlaneSide` sign-bit fast path; host fixed-step accumulator (`MAX_PHYSICS_FREQ 72`, `host_netinterval`) **[behavior]**.
5. **Asset formats** — PAK (+CRC checks), WAD2, BSP29/BSP30-Valve/BSP2/2PSB/Quake64, `.lit`/`.vis`/`.ent` externals, MDL v6, MD3, MD5 (+JSON metadata, tolerant jsmn parsing), SPR, LMP/QPIC, PCX, TGA/PNG/JPG decode, `.kpf` zip localization with its escape parser, UMX **[behavior: identical accept/reject and identical parsed results]**.
6. **CSQC** — dual live QCVMs, `PR_SwitchQCVM` semantics, `CSQC_DrawHud` requirement, `csprogsvers` lookup chain **[behavior]**.
7. **Game-dir & flavor logic** — searchpath ordering and `path_id` semantics, mission-pack detection (`-rogue`/`-hipnotic`/`-quoth` + gamedir-name auto-detect), shareware `registered` gating, original-vs-remastered flavor dirs, Nightdive addon dir, Steam/GOG/EGS discovery **[behavior]**.

---

## 7. Verification strategy ([ADR-019](adr/ADR-019-verification-architecture.md))

Built in **Phase 0, before any port lands**. Every phase's exit criteria reference these gates; they run in CI on every PR touching the affected subsystem.

1. **Demo-determinism harness.** A headless mode plays a fixed demo corpus (id1, Hipnotic, Rogue, 2021 re-release, and mods including Arcane Dimensions, Copper, Alkaline, Quoth, and CSQC-using mods) and emits a per-frame hash chain over canonical game state (edict arena bytes, RNG state, client sim variables). The C build generates per-platform goldens; the Rust build must match **on the same platform** (the C engine is not cross-platform FP-deterministic; matching per-platform C behavior is the bar — [ADR-010](adr/ADR-010-determinism-policy.md)). Where `f32::sin`/`cos` could differ from platform libm, physics call sites call libm directly.
2. **Savegame/config byte-diff.** Scripted saves at fixed simulation points, `cmp`'d byte-for-byte between C and Rust builds (this gate is what forces ADR-005's printf-compatible formatter). Cross-load matrix: C-save→Rust-load→resave→diff, and the reverse. Same for `config.cfg`.
3. **Progs trace oracle.** `-Dtrace=true` builds (both languages) emit per-instruction records — pc, opcode, operands, global/field writes, builtin calls with arguments and returns, string-temp allocations — diffed over the corpus. Plus a builtin-table dump diff (ordinal→name after extension resolution and re-release patching) for every progs.dat in the corpus. The c2rust oracle is the tiebreaker when C behavior is ambiguous.
4. **Protocol goldens & interop matrix.** Byte-diffed captures of canonical sessions; a 4-way localhost matrix (C/Rust client × C/Rust server) across protocol 15/666/999 × PRFL combinations × PEXT on/off; demos recorded by both engines diffed byte-for-byte.
5. **Fuzzing.** `cargo-fuzz` targets for every parser: pak/wad/kpf, all BSP variants, MDL/MD3/MD5+JSON, SPR, LMP/PCX/TGA, per-protocol net message reader, `ED_ParseEdict`/entity lexer, cfgfile, UMX. Differential fuzzing (same input into C-via-FFI and Rust; compare parse result or rejection) wherever feasible.
6. **Float-formatter conformance.** Stratified f32 bit-pattern sampling on every CI run plus a scheduled exhaustive (all 2³² patterns) job, comparing the Rust formatter against C `snprintf("%f")` on each CI OS.
7. **Sound parity.** Fixed-input mixer runs compared by PCM hash; codec decode comparisons per format.
8. **Renderer verification** (Phase 8): screenshot corpus compared by SSIM within a defined tolerance (pixel-exactness is not achievable across FP/driver variance — policy in [ADR-015](adr/ADR-015-renderer-port-then-modernize.md)); Vulkan validation layers clean; lavapipe/SwiftShader smoke tests in CI; `timedemo` performance benchmarks with regression thresholds.
9. **Sanitizers.** ASan/UBSan on the C side of mixed builds; Miri on pure crates' unit tests; TSan and `loom` for `quake-tasks`.
10. **Mod bench.** Scripted load + demo smoke across the curated mod list per release.

---

## 8. Risks and mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Silent VM/physics drift breaking mods/demos/saves | Med | **Critical** | Trace oracle + demo-hash + save byte-diff gate every PR; c2rust tiebreaker; port `pr_exec`/`sv_phys` as near-transliteration first, idiomatize only after parity is proven |
| `longjmp` unwinding Rust frames (UB) | Med | High | Trampoline pattern + status-code shims (§4.3); error-raising leaves ported early; `panic=abort` in release |
| Cross-language allocator mismatch (alloc in one, free in other) | Med | High | Single shared mimalloc (ADR-013); `Mem_*` API mandatory at the boundary; ASan in CI |
| Renderer port destabilizes performance or the frame graph | High | High | Sub-slice switches (Phase 8); gl_heap property tests vs C on random alloc traces; identical task-graph semantics; timedemo benchmarks with thresholds |
| `repr(C)` mirror layout drift (esp. debug-vs-release edict header) | Med | High | Per-profile const size/offset asserts + CI bindgen-diff while C headers exist |
| Dual-build maintenance drag (C oracle must stay green) | High | Med | Strict phase exits delete C promptly; `-Duse_rust_*` switches are time-boxed |
| Crate substitutions change parse acceptance (png/zip/json) | Med | Med | Golden corpora + differential fuzzing gate before any swap; keep the C path until the gate passes; jsmn hand-ported rather than replaced (ADR-012) |
| SDL2+SDL3 dual support doubles platform-layer work | High | Med | Accepted cost (ADR-017); mitigated by the same thin-shim structure the C code uses (`in_sdl2/3`-style feature gates in `quake-platform`) |
| Meson↔Cargo link issues on Windows (clang-cl, CRT selection) | Med | Med | Phase 0 proves the link on all three OSes before any port; MSVC-ABI target; CI from day one |
| Long tail of `pr_ext.c` builtins (6.8k LOC) stalls Phase 6 | High | Med | Per-builtin dispatch lets C and Rust builtins coexist; corpus-driven prioritization (port what the mod bench exercises first) |
| MoltenVK direct-link + ash quirks on macOS | Low | Med | Mirror the current loading path (SDL `vkGetInstanceProcAddr`); macOS CI runs the harness |

---

## 9. Documentation and process rules

- Every deviation from Rust best practice made for compatibility gets an ADR (see [adr/README.md](adr/README.md)) and a `// COMPAT:` comment at the code site linking to it.
- Every phase completion updates [ROADMAP.md](ROADMAP.md) status and records which C files were deleted.
- The last C-capable commit before each deletion wave is tagged (`c-reference/<phase>`), so the oracle can always be rebuilt.
- New Rust code follows rustfmt defaults + workspace clippy configuration; `werror`-equivalent (`-D warnings`) in CI, matching the C build's `werror=true` discipline.
