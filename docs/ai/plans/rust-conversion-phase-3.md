# Feature task plan: Rust migration Phase 3 — Formats & assets (quake-formats, quake-image)

Status: approved
Owner: rust-migration
Baseline: branch `feature/rust-conversion-phase-3-e2bc8e` at plan authoring
Last materially updated: 2026-08-20

For Rust-migration work, this task plan is subordinate to `docs/rust-migration/PLAN.md`, `ROADMAP.md`, and applicable ADRs. It cannot change phase ordering or compatibility policy without an explicit approved documentation/ADR change.

## Objective

Port all asset-format parsing out of `Quake/gl_model.c` and `Quake/image.c` into the pure crates `rust/quake-formats` and `rust/quake-image`, exposed through `rust/quake-capi` shims that populate the exact C-layout structures the renderer consumes, switchable via `-Duse_rust_formats` / `-Duse_rust_image` with the C originals retained as the differential oracle. Covered formats: BSP 29 / 30-Valve / BSP2 / 2PSB / Quake64 (including `.lit`/`.vis`/`.ent` externals, PVS decompression, hull setup, submodels), MDL v6, MD3, MD5, SPR, LMP/QPIC, PCX, TGA, and — behind the ADR-012 pixel-exact gate — PNG/JPG/TGA decode via crates. The renderer half of `gl_model.c` (mesh building, texture upload, task dispatch) stays C and is out of scope.

## Requirements and non-goals

- R1: `quake-formats` parses all brush-model lumps (all five BSP dialects), decompresses PVS, builds hull0 and clip hulls, and sets up submodels, producing structures bit-identical to the C loaders over the full asset corpus.
- R2: `quake-formats` parses MDL v6 (frames, groups, flood-fill skin prep, bounds, extra-flags), SPR, MD3, and MD5 (mesh + anim, influence baking, normal computation) with field-masked-hash parity against C (see D6).
- R3: External asset handling reaches parity: `.lit` (QLIT magic, strict size check, Q64 16-bit unpack, Valve RGB), external `.vis` (`vispatch_t` records, BSP29 + cvar + worldmodel gating), external `.ent` (`"%s@%04x.ent"` via the already-ported Rust CRC).
- R4: `quake-image` provides PCX and LMP/QPIC decoders (hand-ported) and, as a separately gated late milestone, PNG/JPG/TGA decode via crates; `Image_LoadImage`'s two-pass `path_id` search loop stays C (D7). PNG *encode* stays lodepng (ADR-012).
- R5: Rust loaders run correctly on C task workers where C ones do today (brush texture parsing, surface extents): all worker-callable Rust code is Send-pure over byte slices (ADR-016).
- R6: Accept/reject parity on malformed input: differential fuzzing (C-via-FFI vs Rust) for BSP, MDL, MD3, MD5, SPR, PCX, LMP; Sys_Error/reject decisions match.
- R7: Every milestone leaves all Meson configs (`build-c`, `build-rs`, `build-rs-cfs`, plus new formats/image configs) building and the demo-hash harness byte-identical.
- NG1: No deletion of C originals (same MinGW blocker as Phases 1–2, PLAN §3); C stays compiled behind `-Duse_rust_formats=disabled` / `-Duse_rust_image=disabled` as oracle.
- NG2: No renderer work: `gl_mesh.c`, `gl_texmgr.c` texture upload, `GL_MakeAliasModelDisplayLists`, `GLMesh_UploadBuffers`, `Mod_LoadTextureTask`/`Mod_LoadSkinTask` GPU glue, `Mod_PolyForUnlitSurface` stay C.
- NG3: No port of the `.skin`-file → TexMgr callback plumbing (`SKIN_PATTERN` handling, gl_model.c L5721–6006 glue); it stays C wholesale this phase.
- NG4: No port of octree palette tables (`palette.c` — data, stays) and no port of `TexMgr_LoadPalette`/`TexMgr_LoadMiptexPalette` GPU-side derived-table upload.
- NG5: MD5 "JSON metadata via Phase-1 parser" (ROADMAP Phase 3 bullet) does not apply to this tree — `json.h` is only included by `host_cmd.c`/`steam.c`; MD5 loading is a bespoke text tokenizer. Documented as not-applicable; ROADMAP gets a one-line correction in M1 (RA8).

## Invariants and compatibility surfaces

- I1: `#![forbid(unsafe_code)]` in `quake-formats` and `quake-image`; all FFI/unsafe in `quake-capi` (ADR-004). No transmute-over-lumps: read-and-construct parsing only.
- I2: Allocation ownership per ADR-013: capi shims populate buffers obtained from the same C allocator calls the originals make; C frees them identically in both builds.
- I3: Float behavior per ADR-010: MDL scale/origin math, MD5 quaternion-w reconstruction, `CalcSurfaceExtents` double math carry `// COMPAT` comments and use libm-direct calls; parity is bit-for-bit per platform.
- I4: Struct layout per ADR-011: hand-written `#[repr(C)]` mirrors in `quake-types` for `bspfile.h`, `modelgen.h`, `spritegn.h` and in-memory targets the shims write, with const size/offset asserts plus runtime `abi_probe` checks.
- I5: Preserved behavior: demo-hash harness output unchanged in every config; savegame/protocol untouched.
- I6: Thread contract: nothing Rust that runs on task workers touches globals or thread-locals (`com_filesize`/`com_token`/`file_from_pak` are C `THREAD_LOCAL` and must not be shadowed by Rust state) (ADR-016).
- I7: Dependency policy: new crates (`png`, `zune-jpeg`) need ADR-003 review notes + deny.toml pass; Apache-2.0-only excluded (MIT-dual OK).
- I8: Goldens/corpora per ADR-019: goldens generated from the C reference build only; non-redistributable game data referenced by hash, never committed.

## Migration authority

- Roadmap phase: Phase 3 "Formats & assets" (`docs/rust-migration/ROADMAP.md:79-94`); Phases 1–2 complete (delete steps deferred).
- Applicable ADRs: 003 (deps), 004 (unsafe), 010 (determinism), 011 (FFI tooling/mirrors), 012 (vendored libs: image swap gate, lodepng encode stays), 013 (allocator), 016 (task system), 019 (verification).
- C reference/oracle: `Quake/gl_model.c` (post-M1 split: `Quake/model_parse.c`) and `Quake/image.c` (post-M1 split: `Quake/image_decode.c`), compiled into `quake-ctest` via `c_ref_prelude.h` renaming, and into the engine when the switches are disabled.
- Deferred: all C deletions; MinGW blocker per PLAN §3.

## Architecture decisions

- D1 Seam granularity: per-parse-function symbol replacement (the `quake-capi/src/wad.rs` pattern). Orchestrators (`Mod_LoadModel`, `Mod_LoadBrushModel`, task dispatch, all GPU calls) stay C and link against Rust symbols when the switch is on. Alternative (whole-orchestrator seam) rejected: larger unsafe surface, all-or-nothing parity debugging.
- D2 C prep — behavior-neutral file split (Phase-2 `common_fs.c` precedent), not `#ifdef`: `Quake/model_parse.c`/`.h` holds every function Rust will replace; `Quake/image_decode.c` holds the decoders. Micro-refactors folded in (all behavior-neutral, harness-verified): hoist `Mod_PolyForUnlitSurface` out of `Mod_LoadFaces`; split `Mod_LoadTextures` into `Mod_ParseTextures` (moves) + GPU/task loop (stays); split `Mod_LoadAliasModel` at the `GL_MakeAliasModelDisplayLists` boundary. Deferred out of M1 by the amendments below: de-staticing the alias parse path into a context struct (M4 — the `stverts`/`triangles`/`poseverts` externs are consumed by frozen `gl_mesh.c`), the `Mod_LoadSpriteFrame` vs `TexMgr_LoadImage` split (M4), and the MD5/MD3 splits at the `GLMesh_UploadBuffers` boundary (M5). Scope boundary: `model_parse.c` holds format parsing only — the PVS *runtime queries* `Mod_LeafPVS`/`Mod_NoVisPVS` and their `mod_novis` scratch cache stay in `gl_model.c`; only `Mod_DecompressVis` crosses into the seam.
- D3 Meson: `-Duse_rust_formats` / `-Duse_rust_image` feature options (`auto` follows `use_rust`, mirroring `use_rust_fs`), excluding `phase3_formats_c_srcs` / `phase3_image_c_srcs`; cargo features `formats`/`image`; `cargo_build.py` unchanged.
- D4 Data return: pure crates parse `&[u8]` into owned Rust types; `quake-capi/src/{model_parse.rs,image_decode.rs}` allocate via the same C allocator calls and populate `quake-types` `#[repr(C)]` mirrors in place (pointer wiring, anim chains, MD3 `hdrsize*numsurfs` single-block `nextsurface` chaining reproduced exactly).
- D5 Signature gate: `aliashdr_t`/`qmodel_t`-typed declarations in `quake_rs.h` `after_includes` guarded on gl_model.h's include guard; `check_capi_signatures.sh` gains a second gate TU with the Vulkan include path. Fallback: split `quake_rs_model.h` checked in the engine build TU.
- D6 Parity definition: committed field-mask + canonical hash walker (ctest `tests/support/model_hash.rs` + mirrored C walker). Hash all parse-derived scalars and pointed-to arrays; exclude Vk handles, texture pointers, raw ofs values. Mask changes require plan amendment. Brush models hashed likewise.
- D7 `Image_LoadImage` search loop stays C (path_id policy = fs logic); Rust replaces only the post-split `Image_Decode*` entry points.
- D8 Image crates: `png` (PNG) + `zune-jpeg` (JPEG); TGA hand-ported in `quake-image`. `image` facade rejected (tree size; Phase-2 `zip` precedent). Each swap its own revertible commit behind the pixel-exact gate; stb stays compiled until phase-exit deletion.
- D9 Corpus struct-hash gate lives in quake-ctest (synthetic committed fixtures + `QUAKE_GAME_DATA` real corpus via `fetch_shareware.py`), plus thin `scripts/harness/run_formats_corpus.py` emitting per-asset hash manifests (ADR-019: hashes only). No engine dump mode.
- D10 Threading proof: ctest concurrent test over the Rust texture/extents entry points + asan CI on the combined config + demo harness exercising all 4 dispatch sites (gl_model.c task submits).
- D11 Differential fuzzing: `rust/fuzz` gains a cc-based build.rs compiling the c_ref archive; targets `bsp_diff`, `mdl_diff`, `md3_diff`, `md5_diff`, `spr_diff`, `pcx_diff`, `lmp_diff` asserting (accept?, hash) equality with `Sys_Error` trapping. `rust.yml` fuzz-smoke hardcoded list extended.
- D12 Milestone ordering: image first (smallest slice proves every new mechanism), brush before alias, MD5 last among formats, crate swap dead last and deferrable.

## Change boundary

Expected to change: `Quake/gl_model.c`, new `Quake/model_parse.c`/`.h`, `Quake/image.c`, new `Quake/image_decode.c`; `rust/quake-formats/`, `rust/quake-image/`; `rust/quake-capi/src/{model_parse.rs,image_decode.rs,lib.rs}` + `cbindgen.toml`; `rust/quake-types/src/{bspfile.rs,modelgen.rs,spritegn.rs,model_mem.rs}`; `rust/quake-c-sys` allowlists + regen; `rust/quake-ctest/*`; `rust/fuzz/*`; `rust/Cargo.{toml,lock}`, `rust/deny.toml` (M8); `meson.build`, `meson_options.txt`; `scripts/harness/{check_capi_signatures.sh,run_formats_corpus.py}`; `.github/workflows/{rust.yml,build-linux.yml}`; `docs/rust-migration/ROADMAP.md`, ADR-003/ADR-012 amendments (M8).

Must not change without plan amendment: `Quake/gl_mesh.c`, `Quake/gl_texmgr.c`, `Quake/tasks.c`, renderer consumers of `aliashdr_t`; `Quake/gl_model.h` struct layouts; `bspfile.h`/`modelgen.h`/`spritegn.h` on-disk formats; `image.h` 4-function seam signatures; lodepng encode path; the D6 field mask once landed; Phase-1/2 crates and switches; harness demo goldens.

## Acceptance matrix

| ID | Acceptance criterion | Verification/gate | Status |
|---|---|---|---|
| AC1 | C prep splits are behavior-neutral | `run_corpus.py --compare` demo hashes identical pre/post on `build-c`; all 3 existing configs build | M1: local Windows clang-cl compare green (5/5 identical, shareware tier); Linux CI configs pending PR |
| AC2 | quake-types mirrors match C layouts | const asserts + `abi_probe` runtime checks | M3: green locally (`bsp_abi` 5/5, Windows MSVC); per-platform confirmation via the rust.yml test matrix on PR |
| AC3 | PCX/LMP Rust decode byte-identical incl. reject behavior | ctest `image_differential`; `pcx_diff`/`lmp_diff` fuzz | M2: ctest portion green locally (10/10, Windows); fuzz portion lands M7 |
| AC4 | Brush parsing (5 dialects, PVS, hulls, submodels, .lit/.vis/.ent) hash-identical | ctest `bsp_differential` + `run_formats_corpus.py`; `bsp_diff` fuzz | M3: ctest portion green locally (9/9, Windows); corpus + fuzz portions land M7 |
| AC5 | MDL/SPR parity (field-masked hash) | ctest `alias_differential`, `sprite_differential`; fuzz | pending |
| AC6 | MD3/MD5 parity incl. baked influences/normals bit-for-bit | ctest `md3_differential`, `md5_differential`; fuzz | pending |
| AC7 | Engine behavior unchanged with switches on | `run_corpus.py --compare` vs `build-c`; `save_diff.py` | pending |
| AC8 | Threaded loading works with Rust loaders | ctest concurrent test + demo harness + asan | pending |
| AC9 | Signature/ABI/bindgen gates extended and green | `check_capi_signatures.sh` (both TUs), `check_headers.sh`, bindgen regen-diff | M2: image portion green locally; M3: second gate TU added (`gl_model.h`/`bspfile.h`/`model_parse.h` behind Vk + atomics stand-ins) and green under clang-cl; c-sys regen diff = `COM_SkipPath`/`COM_StripExtension`/`external_ents` only; Linux CI canonical run pending PR |
| AC10 | Differential fuzz infrastructure runs in CI | `rust.yml` fuzz-smoke with new targets; local soak clean | pending |
| AC11 | PNG/JPG/TGA crate decode pixel-exact vs stb over corpus | ctest `image_crate_differential` per format; `cargo deny`; ADR-003 amendment | pending |
| AC12 | Phase-exit full-suite rerun (ADR-019) | all workflows + full corpus + fuzz smoke on final milestone | pending |

## Milestones

One milestone per `/feature-implement` session; stop at each boundary and record evidence.

- **M1 — C prep: behavior-neutral split.** Create `Quake/model_parse.c`/`.h` (move: `Mod_DecompressVis`, brush lump loaders incl. BSP2/2PSB variants, `CalcSurfaceExtents`, `Mod_MakeHull0`, `Mod_LoadLighting`/`Visibility`/`Entities`, external-vis loaders, `Mod_SetupSubmodels`, `Mod_ParseTextures` split, alias and sprite parse moves with the `Mod_ParseAliasModel` GPU-call hoist; MD3/MD5 deferred to M5, alias de-static and the sprite TexMgr sub-split to M4) and `Quake/image_decode.c` (PCX, LMP, stb wrapper). Register `phase3_formats_c_srcs`/`phase3_image_c_srcs` in meson (unconditionally compiled for now). ROADMAP MD5-JSON correction. Commit this plan. `./format.sh`. Acceptance: AC1.
- **M2 — quake-image PCX/LMP port + `-Duse_rust_image` + differential substrate.** `quake-image::{pcx,lmp}`; capi `image_decode.rs`; Meson option; ctest c_ref of `image_decode.c` + `image_differential.rs`; signature gate adds `image.h`. Acceptance: AC3 (tests), AC9 (image portion).
- **M3 — quake-types mirrors + brush/BSP port + `-Duse_rust_formats`.** `bspfile.rs`/`model_mem.rs` mirrors + abi_probe; `quake-formats::bsp`; capi `model_parse.rs`; D5 second gate TU; `bsp_differential.rs` with synthetic dialect fixtures; c-sys regen. Acceptance: AC2, AC4 (tests), AC9.
- **M4 — MDL + SPR.** `quake-formats::{mdl,spr}`; `modelgen.rs`/`spritegn.rs` mirrors (document `synctype_t` dual-definition hazard); D6 masked-hash differentials. Acceptance: AC5 (tests).
- **M5 — MD3 + MD5.** Parse-only ports incl. `MD5_BakeInfluences`/`MD5_ComputeNormals` bit-exact vectors; MD3 single-block allocation reproduced. Acceptance: AC6 (tests).
- **M6 — Threaded-loading proof.** ctest `threaded_parse.rs`; asan config; global-state audit. Acceptance: AC8.
- **M7 — Differential fuzzing + corpus gate + CI.** fuzz build.rs + 7 diff targets; `run_formats_corpus.py`; CI configs `build-rs-formats`/`build-rs-all`. Acceptance: AC3–AC6 fuzz portions, AC7, AC10.
- **M8 — PNG/JPG/TGA crate decode (ADR-012 gate).** TGA hand-port, `png`, `zune-jpeg`, each revertible and corpus-gated; deny/ADR-003 amendment; phase-exit full rerun + `/integration-review`. Acceptance: AC11, AC12.

## Final verification

- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (release + debug), `cargo deny check` (main + fuzz manifests).
- Meson builds: `build-c`, `build-rs`, `build-rs-cfs`, `build-rs-formats`, `build-rs-all`.
- `check_capi_signatures.sh` (both TUs), `check_headers.sh`, `gen_c_bindings.sh` regen-diff.
- `run_corpus.py --compare`, `save_diff.py`, `run_formats_corpus.py` over shareware; fuzz soak per D11 target.
- darwin-arm64 golden rerun is manual/not-run-here (goldens exist only there); Windows/MinGW C-only legs unaffected.

## Risks, assumptions, open questions

| ID | Type | Item | Mitigation/decision | Status |
|---|---|---|---|---|
| RA1 | risk | In-memory mirrors drift vs Vulkan-bearing `gl_model.h` | abi_probe runtime offset checks for every shim-written field; D5 gate TU | M3: both mitigations in place and green on Windows (`bsp_abi` 5/5, second gate TU); stays open until the CI matrix confirms Linux/macOS |
| RA2 | risk | Float math differs bitwise on some platform | ADR-010 discipline; targeted bit-exact vectors in M5 | open |
| RA3 | risk | c_ref build of `model_parse.c` needs many stubs | keep moved functions' external calls minimal via M1 hoists; malloc-backed arena stub | closed M3 — the brush half needed 8 new stubs plus a dummy-texture helper (amendment log); oracle builds and runs |
| RA4 | risk | Synthetic BSP2/2PSB/Q64 fixtures hard to author | commit generator code, not binaries; hash-manifest real assets per ADR-019 | closed M3 — `bsp_differential.rs` builds all 5 dialects in-process from committed Rust lump builders; real-asset corpus still lands M7 |
| RA5 | risk | Vulkan headers in signature-gate TU brittle on CI | fallback `quake_rs_model.h` checked in engine build TU | closed M3 — the second TU uses `__Q_RENDER_TYPES_H`/`__ATOMICS_H` stand-ins and needs no Vulkan SDK; fallback unused |
| RA6 | risk | Task-worker bugs only reproduce under load | M6 asan + concurrent ctest; harness exercises all 4 dispatch sites per map load | open |
| RA7 | risk | `png` transitive deps or license drift | ADR-003 review notes + deny CI gate; M8 deferrable | open |
| RA8 | decision | ROADMAP "MD5 + JSON metadata" has no C counterpart here | documented not-applicable; one-line ROADMAP correction in M1, flagged in PR | open |
| RA9 | assumption | Dev host Windows; parity gates run in Linux CI; shareware via `fetch_shareware.py` | local dev uses synthetic fixtures; corpus/harness gates rely on CI; darwin goldens manual | open |
| RA10 | question | stb link-exclusion timing | leave compiled (oracle) until phase-exit deletion (NG1) | open |
| RA11 | risk | `synctype_t` dual definition (modelgen vs spritegn, unequal) | quake-types defines one canonical enum; comment documents the C hazard | open |

## Plan amendment log

| Date | Repository contradiction/evidence | Smallest amendment | Acceptance impact | Approval |
|---|---|---|---|---|
| 2026-08-20 | `stverts`/`triangles`/`poseverts` externs (gl_model.h:578-580) are consumed by frozen `gl_mesh.c`; introducing `aliasparsectx_t` would change that header/consumer | M1 moves the alias globals to `model_parse.c` verbatim (identical linkage) instead of de-staticizing into a context struct; de-static decision deferred to M4 (Rust side owns its own state; capi writes the same globals) | none — AC5 unchanged | in-session, behavior-neutral |
| 2026-08-20 | `Mod_FloodFillSkin` callers (`Mod_LoadAllSkins`, MD5 skin path) all stay in `gl_model.c` | `Mod_FloodFillSkin` stays in `gl_model.c` for M1 | none | in-session |
| 2026-08-20 | MD5/MD3 loaders are tightly interleaved with `GLMesh_UploadBuffers`/TexMgr calls; extracting them is the M5 port's natural first step, not a neutral move | MD5/MD3 parse-split deferred from M1 to M5; sprite `TexMgr_LoadImage` sub-split deferred to M4 | none — split lands with the port it serves | in-session |
| 2026-08-20 | `RadiusFromBounds` sole caller is `Mod_SetupSubmodels` (moved) | `RadiusFromBounds` moved to `model_parse.c` (not in original move list, internal linkage only — no header entry) | none | in-session |
| 2026-08-20 | PR review: `Mod_LeafPVS`/`Mod_NoVisPVS` are per-frame runtime queries (`pr_cmds.c`, `pr_ext.c`, `r_world.c`, `sv_main.c`), not format parsing; moving them put mutable `mod_novis` process-global scratch into the module ROADMAP calls "pure functions over byte slices" | Both functions and the `mod_novis` cache returned to `gl_model.c`; only `Mod_DecompressVis` (with its own separate `mod_decompressed` cache) stays in `model_parse.c`. D2 records the scope boundary | none — removes a dual-state hazard for the M3 ctest oracle | review round |
| 2026-08-20 | PR review: `model_parse.h` declared three `gl_model.c`-owned symbols, making the seam header circular and putting non-`quake-formats` symbols on the future FFI boundary | `Mod_FindName`/`Mod_LoadWadTexture`/`Mod_LoadAllSkins` prototypes moved to `gl_model.h`, matching the existing `Mod_SetExtraFlags` convention; `model_parse.h` now declares only what `model_parse.c` provides | prevents 3 spurious symbols reaching the M2+ signature/bindgen gates (AC9) | review round |

| 2026-08-21 | M2: the milestone's ctest c_ref build assumed stb-in-TU (`common_fs.c` precedent), but the STB decoder needs no differential oracle (stays C until M8) and drags the 7k-line stb_image implementation plus its Mem_/Sys_ surface into every c_ref TU | `image_decode.c` split again: PCX/LMP (the gated oracle, excluded under `-Duse_rust_image`) stay in `image_decode.c`; `Image_DecodeSTB` + stb implementation moved byte-identically to new `Quake/image_stb.c`, compiled unconditionally in all configs until M8 | none — AC3 oracle smaller and stub-free; stb deletion timing (NG1/RA10) unchanged | in-session, behavior-neutral |
| 2026-08-21 | M2: the C PCX decoder streams via `Sys_fgetc`/`Sys_FilePos` and trusts input — RLE overrun writes past the `(w*h+1)*4` heap block, EOF mid-run indexes `palette[-3]`, and a sub-768-byte pak resource seeks/reads outside the resource bounds: all UB or out-of-resource reads with no defined behavior to replicate | quake-image bounds every access to the resource slice and the capi shim `Sys_Error`s `'%s' is not a valid PCX file` on exactly those inputs; every defined C path (including the error order, message bytes, alloc-before-palette sizing, run spill-over between rows, last-line padding write) is replicated. Divergence is confined to UB inputs and marked `// COMPAT` in `pcx.rs` | AC3 parity is asserted over defined behavior; `image_differential` reject cases cover the C-defined `Sys_Error` paths only | in-session |
| 2026-08-21 | M2: a streaming Rust port would need `Sys_FilePos`/`Sys_fgetc`/`Sys_feof` added to the FFI surface solely to reproduce the C read pattern | capi shims bulk-read the resource once (`COM_ThreadFileSize` truncated to int, one `Sys_FileRead`) and hand slices to pure parsers; only `COM_CloseFile` was added to the c-sys allowlist | none — same bytes observed; shrinks the AC9 surface | in-session |
| 2026-08-21 | M3: `Mod_LoadLeafsExternal` also raises `Host_Error` (external-vis leaf-count mismatch), so the plan's "4 Host_Error-capable entry points" undercounted the seam by one | five status-returning exports (`quake_rs_mod_load_{leafs,clipnodes,marksurfaces,leafs_external}`, `quake_rs_mod_setup_submodels`) with a `char err[256]` out-param, re-raised from `Quake/model_parse_glue.c` | none — AC4 unchanged; PLAN.md 4.3 satisfied for every longjmp site | in-session |
| 2026-08-21 | M3: excluding `model_parse.c` from the build under the switch (the M2 `image_decode.c` pattern) would also drop the alias/sprite half, which stays C until M4/M5 | `model_parse.c` is compiled unconditionally; `-Duse_rust_formats` defines `USE_RUST_FORMATS`, which `#ifndef`-guards out only the brush/BSP region (4 guard lines, no body edits), and adds `model_parse_glue.c` to the sources | none — the C brush half stays a byte-identical oracle under `-Duse_rust_formats=disabled` | in-session |
| 2026-08-21 | M3: `qboolean` is `typedef bool` (1 byte), not `int`, so the `qboolean bsp2` parameters of `Mod_ParseFaces`/`Mod_LoadClipnodes` are not ABI-compatible with `c_int` | those two shims take Rust `bool`; the `int bsp2` loaders (`Mod_LoadEdges`/`Nodes`/`Leafs`/`Marksurfaces`) keep `c_int`, matching `model_parse.h` exactly (diffed by the new gate TU) | none — prevents a silent ABI mismatch | in-session |
| 2026-08-21 | M3: `Mod_ParseFaces` ORs into `msurface_t::styles_bitmap` without initializing it, and the surfaces come from `Mem_AllocNonZero` — with a poisoned ctest allocator the C oracle is not reproducible | the ctest `Mem_AllocNonZero` stub zero-fills (documented in `stubs.c`); the engine bug is left untouched and the Rust port reproduces the OR-into-zero result | AC4 parity is asserted against a zero-initialized arena; the underlying engine bug is recorded, not fixed | in-session |
| 2026-08-21 | M3: `com_filesize` is per-thread in this engine (`COM_ThreadFileSize()`), so the plan's "allowlist `com_filesize`" would have bound the shims to a global that does not exist | the `.lit`/`.ent` shims call `COM_ThreadFileSize()`; the c-sys allowlist additions are only `COM_SkipPath`, `COM_StripExtension` and the `external_ents` cvar | none — smaller AC9 surface | in-session |
| 2026-08-21 | M3: the ctest c_ref build of `model_parse.c` needed more of the engine than RA3 anticipated — `BSP29_VALVE` (unconditional in `quakedef.h`, gated on by both `bspfile.h` and `model_parse.c`), the `r_notexture_mip`/`r_notexture_mip2` dummy slots `Mod_LoadTextures` fills in `gl_model.c` after `Mod_ParseTextures` (without them `Mod_ParseFaces` NULL-derefs on `TEX_MISSING` texinfos), and five symbols pulled in by the still-C alias/sprite half | prelude defines `BSP29_VALVE`; `stubs.c` gains `ctest_fill_dummy_textures` (called on both sides by the differential driver) plus `q_strncasecmp`, `PScript_UpdateModelEffects`, `thread_stack_alloc_size`, `max_thread_stack_alloc_size`, `r_nolerp_list` | none — oracle fidelity only; RA3 closed | in-session |
| 2026-08-21 | M3: the approved plan's ground facts referenced a `gl_load24bit` cvar read in the brush range; no such symbol exists in the repo | reference dropped; the only cvar the brush range reads is `external_ents` | none | in-session |
## M1 verification evidence (2026-08-20)

- Split: 44 functions moved `gl_model.c` → `model_parse.c` script-driven; every moved body verified byte-identical to `HEAD:Quake/gl_model.c` modulo `static ` removal ("non-verbatim: NONE"). Seam wrappers created: `Mod_LoadTextures`→`Mod_ParseTextures`+GPU/task tail, `Mod_LoadFaces`→`Mod_ParseFaces`+`SURF_DRAWTILED` poly post-pass+extents tail, `Mod_LoadAliasModel`→`Mod_ParseAliasModel`+display-list tail. `image.c`→`image_decode.c` (`Image_DecodeSTB`/`PCX`/`LMP`), search loop stays.
- Build (targeted, run): Windows Meson + clang-cl 22.1.8, full build + link green pre- and post-format (`356/356`).
- Behavior-neutrality (targeted, run): `run_corpus.py --compare` post-split build vs pre-split baseline built from HEAD e1e3fadd, shareware corpus (`fetch_shareware.py` checksums verified: quake106.zip + pak0.pak match pins) — **5 ran (id1-demo1/2/3, save-e1m1, map-e1m2), all identical, 0 failed**; 9 skipped (registered/mission-pack/mod tiers, local-only data absent).
- Formatting: `format.sh` docker unavailable; local clang-format 22.1.8 verified byte-identical to repo baseline formatting, then applied to the 6 touched files.
- Not run locally (CI on PR): Linux/macOS/MinGW builds, `build-rs`/`build-rs-cfs` configs, `check_headers.sh`, save_diff.py.
- **Limits of the state-hash gate (review follow-up):** `run_corpus.py --compare` hashes simulation state, so it is blind to console-output ordering and to failure paths not exercised by a clean corpus. Two such cases were found by code review, not by the harness, and are recorded here rather than claimed as verified-neutral:
  1. `Mod_LoadWadFiles` is hoisted above `Mod_ParseTextures` (it must be, now that `wads` is a parameter). It reads neither `mod->textures` nor `mod->numtextures`, so state is equivalent, but `W_LoadWadList` warnings for missing/bad wads now print *before* `"Mod_LoadTextures: no textures in bsp file"` instead of after — reachable only on a map with an empty texture lump and a `wad` key.
  2. The `if (data)` guard around `*fmt = SRC_RGBA` was initially dropped when the STB decode body moved into `Image_DecodeSTB`, making a failed decode overwrite `glt->source_format` via `TexMgr_ReloadImage`. Fixed in the review round; needs a corrupt/truncated image in the search path to observe, so no corpus run would have caught it.

## M2 verification evidence (2026-08-21)

- Port: `quake-image::{pcx,lmp}` pure parsers (`#![forbid(unsafe_code)]`, 8 unit tests green) + capi `image_decode.rs` shims (`Image_DecodePCX`/`Image_DecodeLMP`, cargo feature `image`); `-Duse_rust_image` feature option (`auto` follows `use_rust`, error if enabled without it); ctest c_ref build of `image_decode.c` with `Sys_FilePos`/`Sys_fgetc`/`LittleShort` stubs.
- Differential (targeted, run): `image_differential.rs` **10/10 green** (Windows MSVC): PCX literal+run decode, run spill across rows, `bytes_per_line` padding write into the +1 slot, LMP roundtrip, LMP size-mismatch NULL parity, and 5 `Sys_Error` message-parity cases (PCX bad signature/version/encoding/short header, LMP short header; C via longjmp trap, Rust via child-process probe). Full-allocation byte compare (including padding slot), out-dims, con log, and open-handle count all asserted equal.
- Rust battery (broad, run): `cargo test` workspace 48/48 suites green; `cargo clippy --workspace --all-targets --locked -- -D warnings` clean; `cargo fmt --check` clean.
- Gates (targeted, run): `check_capi_signatures.sh` green with `image.h` in the TU, both against local cbindgen output and `build-rs/quake_rs.h` (CC=clang-cl; TU defines a stand-in `enum srcformat` tag because the real one lives in Vulkan-bearing `gl_texmgr.h`). c-sys bindgen regen diff is exactly `+COM_CloseFile` (Windows-host regen; 4 host-dependent enum typedefs hand-reverted to Linux-canonical — Linux CI bindgen-smoke is authoritative).
- Meson (targeted, run, Windows clang-cl): `build-c` green (`image_decode.c` + `image_stb.c` compiled); `build-rs` green with `image` in the cargo feature list and `Quake_image_decode.c.obj` absent from the target (symbols provided by `quake_rs.lib`), `image_stb.c` still compiled.
- Formatting: `format.sh` docker image (clang-format 18) dry-run `-Werror` clean on `image_decode.c`/`image_stb.c`.
- Not run locally (CI on PR): Linux/macOS/MinGW builds, `build-rs-cfs` config, Linux-canonical bindgen regen-diff, `check_headers.sh`, corpus/save_diff harness legs.

## M3 verification evidence (2026-08-21)

- Mirrors: `quake-types::bspfile` (291 lines: on-disk `dheader_t`/`lump_t`/`dmodel_t`/`dmiptexlump_t`/`miptex_t`/`miptex64_t`/vertex/plane/texinfo plus the S/L1/L2 node/leaf/clipnode/edge/face triples, magics, lump indices, limits) and `quake-types::model_mem` (316 lines: `MVertex`/`MEdge`/`MTexInfo`/`MSurface`/`MNode`/`MLeaf`/`MClipnode`/`Hull`/`Texture`/`QModel`, Vk tail as pointer-sized stand-ins), both with const size/offset asserts.
- Port: `quake-formats::bsp` (`lumps`, `textures`, `lighting`, `vis`, `extents`; 1665 lines; `#![forbid(unsafe_code)]`; **33 unit tests green**) plus capi `model_parse.rs` (1671 lines, cargo feature `formats`): 16 seam functions exported under their C names and 5 status-returning `quake_rs_mod_*` trampolines re-raised by `Quake/model_parse_glue.c` (77 lines, compiled only under the switch). `CalcSurfaceExtents` uses f64 intermediates per ADR-010 (`// COMPAT: ADR-010`); every other deliberate bug-for-bug divergence carries a `// COMPAT:` comment at its site (Q64 halving order, the L2 `p > 0` child branch, the C out-of-bounds reads on malformed lumps, the `ents` leak on the CRC-mismatch path, the `submodels[0]` read with zero submodels, ASCII-only `q_strcasecmp` in the vispatch scan).
- ABI (targeted, run): `bsp_abi.rs` **5/5 green** — on-disk and in-memory struct sizes plus `qmodel_t`, surface/node/leaf and `texture_t` field offsets read back from the real engine headers through `abi_probe.c`. AC2 was locked before any logic was written (stage 1 of the milestone).
- Differential (targeted, run): `bsp_differential.rs` **9/9 green** (Windows MSVC, `--test-threads=1`): `bsp_parity_all_dialects` (BSP29, BSP30-Valve, 2PSB, BSP2, Quake64 — all loaders in `Mod_LoadBrushModel` order, world plus submodels, deep-walked and compared field by field via `tests/support/model_hash.rs`), `worldmodel_clipbox_branch_parity`, `lit_parity`, `ent_parity` (including the CRC-mismatch reject path), `decompress_vis_parity`, `external_vis_parity` (real vispatch container through `FILE*`), `host_error_parity` (C via longjmp trap vs Rust via status return, message bytes compared), `sys_error_parity`, `rust_fatal_child`. The snapshot walker compares full `visdata`/`lightdata` bytes and the entity string, and a line-count floor guards against a silently truncated snapshot.
- Rust battery (broad, run): `cargo test --workspace --locked` **50 suites / 202 tests, 0 failed** in debug and again under `--release`; `cargo clippy --workspace --all-targets --locked -- -D warnings` clean; `cargo fmt --check` clean.
- Gates (targeted, run): `check_capi_signatures.sh` green — the new second TU pre-defines `__Q_RENDER_TYPES_H` and `__ATOMICS_H` with 64-bit Vk handle and `atomic_uint32_t` stand-ins, then includes `bspfile.h` + `gl_model.h` + `model_parse.h` + `quake_rs.h` (CC=clang-cl, cbindgen-generated header); **RA5 closed without the Vulkan SDK — the fallback `quake_rs_model.h` was not needed**. The c-sys regen diff is exactly `COM_SkipPath`, `COM_StripExtension` and `external_ents`; the Windows-host regen reproduces those additions verbatim, but its MSYS `//`-to-`/` argv mangling and host-dependent enum reprs mean the committed file stays Linux-canonical (rust.yml bindgen-smoke is authoritative).
- Meson (targeted, run, Windows clang-cl): `build` (`-Duse_rust=disabled`) green with no `USE_RUST_FORMATS` and no glue object; `build-rs` (`-Duse_rust=enabled`, formats auto to on) green with `Quake_model_parse.c.obj` *and* `Quake_model_parse_glue.c.obj` present and `formats` in the cargo feature list; `-Duse_rust=enabled -Duse_rust_formats=disabled` also configures, builds and links (the C brush oracle path — no CI leg covers this combination).
- Formatting: `format.sh` docker image (clang-format 18) dry-run `-Werror` clean on `model_parse.c`, `model_parse_glue.c`, `model_parse.h`.
- Not run locally (CI on PR): Linux/macOS/MinGW builds, `build-rs-cfs`, the Linux-canonical bindgen regen-diff, `check_headers.sh` (`gl_model.h` deliberately stays out of its list), and the corpus/`save_diff` legs (AC7 lands M7).

## Verification evidence and handoff

| Milestone | Changed files/behavior | Check and result | Acceptance IDs | Remaining risk/next action |
|---|---|---|---|---|
| M1 | `Quake/model_parse.c`/`.h` (new), `Quake/gl_model.c` (6203→3828 lines), `Quake/image_decode.c` (new), `Quake/image.c`, `Quake/image.h`, `meson.build`, `Quake/common.make`, ROADMAP MD5-JSON correction | Windows clang-cl build green; byte-identical move verification; `run_corpus.py --compare` 5/5 identical (see M1 evidence above) | AC1 (local portion) | CI matrix + harness gates run on PR; next: M2 (quake-image PCX/LMP + `-Duse_rust_image`) |
| M2 | `rust/quake-image/src/{lib,pcx,lmp}.rs`, `rust/quake-capi/src/image_decode.rs` (+`lib.rs`, `Cargo.toml`), `Quake/image_stb.c` (new, STB split out of `image_decode.c`), `meson.build`/`meson_options.txt` (`use_rust_image`), `Quake/common.make`, `scripts/gen_c_bindings.sh` + `rust/quake-c-sys/src/generated.rs` (`COM_CloseFile`), `rust/quake-ctest/{tests/image_differential.rs,stubs/stubs.c,include/c_ref_prelude.h,build.rs,Cargo.toml}`, `scripts/harness/check_capi_signatures.sh` | image_differential 10/10; workspace tests/clippy/fmt clean; signature gate green both header sources; build-c + build-rs green, `image_decode.c` excluded under the switch (see M2 evidence above) | AC3 (ctest portion), AC9 (image portion) | UB-input divergence documented (amendment log); fuzz + corpus legs land M7; Linux CI canonical for bindgen/headers; next: M3 (quake-types mirrors + brush/BSP + `-Duse_rust_formats`) |
| M3 | new: `rust/quake-types/src/{bspfile.rs,model_mem.rs}`, `rust/quake-formats/src/bsp/{mod,lumps,textures,lighting,vis,extents}.rs`, `rust/quake-capi/src/model_parse.rs`, `Quake/model_parse_glue.c`, `rust/quake-ctest/tests/{bsp_abi.rs,bsp_differential.rs,support/model_hash.rs}`; modified: `Quake/model_parse.c` (4 guard lines), `meson.build`/`meson_options.txt` (`use_rust_formats`), the `quake-types`/`quake-formats`/`quake-capi` lib files, manifests and `cbindgen.toml`, `rust/quake-c-sys/{bindings_wrapper.h,src/generated.rs}`, `rust/quake-ctest/{build.rs,Cargo.toml,include/c_ref_prelude.h,stubs/{stubs.c,abi_probe.c}}`, `scripts/gen_c_bindings.sh`, `scripts/harness/check_capi_signatures.sh` | bsp_abi 5/5; bsp_differential 9/9 (5 dialects, PVS, hulls, submodels, .lit/.vis/.ent, Host_Error and Sys_Error parity); workspace 202 tests in debug *and* release; clippy/fmt clean; signature gate green with the new second TU; three Meson configurations build and link; clang-format clean (see M3 evidence above) | AC2, AC4 (ctest portion), AC9 (formats portion) | Parity is asserted against synthetic fixtures only — real-map corpus (AC7) and `bsp_diff` fuzz (AC10) land M7; `Mod_SetupSubmodels`'s `Mod_FindName` recursion is stubbed in ctest and unexercised locally; mirror layout (RA1) confirmed only on Windows until the CI matrix runs; next: M4 (MDL + SPR) |

## Completion gate

- [ ] Requirements map to acceptance criteria (R1→AC4, R2→AC5/AC6, R3→AC4, R4→AC3/AC11, R5→AC8, R6→AC10, R7→AC1/AC7/AC12).
- [ ] Required compatibility and ADR constraints satisfied (ADR-003/004/010/011/012/013/016/019).
- [ ] Current evidence exists for every completed acceptance criterion.
- [ ] Required CI/harness/manual verification complete or explicitly not run (darwin goldens explicitly listed).
- [ ] Independent integration review (`/integration-review`) complete.
- [ ] Remaining risk explicit and accepted.
- [ ] Diff contains no unrelated work.
