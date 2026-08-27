# Rust migration Phase 6 — Progs VM (`quake-progs`) — M1–M5 this session

## Context

Phases 0–5 are complete on `master` (`e11d134c`); this branch is `feature/rust-conversion-phase-6-c6b79f`. Phase 6 (`docs/rust-migration/ROADMAP.md:142`) ports the QuakeC VM — loader, edict arena, interpreter, `ED_Write`/`ED_Parse*`, builtins — into `rust/quake-progs` (today a 3-line Phase-0 stub). It is deliberately sequenced **before** Phase 7 so client/server porting sits on a trace-verified Rust VM, and the roadmap flags it as the highest-compatibility-risk phase: 12,461 lines of C (`pr_edict.c` 2304, `pr_exec.c` 602, `pr_cmds.c` 2080, `pr_ext.c` 6823, `progs.h` 438, `pr_comp.h` 214).

Phase 0 built the trace oracle's **producer** (`Quake/pr_trace.{c,h}`, `-Dtrace=true` → `-DPR_TRACE`, hooks already live in `pr_exec.c`, `scripts/harness/run_trace.py`). It has no consumer and no CI leg — `run_trace.py:5` says outright "The Phase 6 differ consumes these". Building that differ is this phase's first deliverable, because every later milestone is verified by it.

**User decisions (recorded):**
- **Execution scope this session: M1–M5 only** (scaffolding + trace differ → arena/strings → interpreter flip → loader → savegame writer/parser). Stop for review before the builtin milestones (M6–M10), whose full design is recorded below.
- **`pr_ext.c` scope: decide per batch during implementation.** Port builtins in dependency-ordered batches; stop a batch when its dependencies are dominated by Phase 7/8 code and record the carve-out in the ROADMAP the way Phase 5 did for `net_wins.c`.
- **Deletions deferred again** (Phases 1–5 precedent). The C progs files stay compiled under `-Duse_rust_progs=disabled` — and here that is not just convention: **the trace oracle needs the C build to diff against.**
- Local gates run everything runnable on this Mac including registered-tier data via the known `QUAKE_GAME_DATA`; unreachable legs documented as deferred (Phase 3/4/5 precedent).
- Suspected bugs preserved bug-for-bug with `// COMPAT:` comments (see landmines 5, 10, 12) and logged as post-parity fix candidates.

Governing: ADR-004 (unsafe), ADR-005 (printf — see landmine 13), ADR-006 (edict arena), ADR-007 (needs a progs row), ADR-008 (ambient qcvm), ADR-009 (no longjmp through Rust), ADR-010 (libm call-through), ADR-011 (hand-written mirrors), ADR-019 (gate 3 = the trace oracle). Template: `docs/ai/plans/rust-conversion-phase-5.md` — one milestone per commit, tree green after each, amendment log. This plan is committed as `docs/ai/plans/rust-conversion-phase-6.md` in M1.

## Verified ground truth (landmines)

1. **`qcvm_t` is C-owned storage**, embedded in `server_t sv` (`server.h:62`) and `client_state_t cl` (`client.h:282`). It is a 400+ line struct (`progs.h:361-426`) containing `builtins[1024]`, `stack[1024]`, `localstack[16384]`, `freelist_t` (a `uint16_t[MAX_EDICTS]` circular FIFO), `areanodes[1024]` (world.c's tree — Phase 7 semantics, Phase 6 only needs its offset), and the string table. Rust cannot own it this phase: it mirrors it per ADR-011 and threads the C instance through, exactly as Phase 5's dgrm threaded `packetBuffer`.
2. **`ADR-006`'s arena is an owner in the end state but a *view* during Phase 6** — `qcvm->edicts` is allocated by `PR_LoadProgs`/`PR_ClearProgs` in C (`pr_edict.c:1704`) until the loader flips at M4. Record as an ADR-006 amendment; the fuzz/ctest paths construct the arena over a Rust-owned buffer so `quake-progs` stays `quake-c-sys`-free and fuzzable.
3. **Re-entrancy is the phase's #1 aliasing hazard.** `PR_ExecuteProgram` dispatches a builtin (`pr_exec.c:564`), the builtin is C, and C builtins call `PR_ExecuteProgram` again. A Rust `&mut QcVm`/`&mut EdictArena` held across that dispatch is instant UB. ADR-006's "no reference escapes a VM step" must be tightened to **"no Rust reference lives across a builtin dispatch"**: the interpreter re-derives raw base pointers per step. Miri models the re-entrancy shape ([[unsafe-claims-must-be-verified]]).
4. `pr_exec.c` is nearly self-contained — it calls only `Con_Printf`, `Con_Warning`, `Host_Error` and reads `sv.state` (`pr_exec.c:501`). This is why it flips first.
5. **Negative string offsets** (`pr_edict.c:2182-2201`): `PR_GetString` indexes `knownstrings[-1 - num]` for negatives; the invalid-offset `Host_Error` at :2196 is **dead code after a `return`**, so bad handles silently yield the empty string. `PR_SetEngineString` (:2233) has an off-by-two range test (`s <= strings + stringssize - 2`). `PR_ClearEdictStrings` (:2299) skips free-slot reuse under `_DEBUG`. `ED_RezoneString` (:1236) keys a `knownzone` bitmap on `-1 - ref`, grown 32 slots at a time.
6. **Hash maps built in reverse** (`pr_edict.c:1963-2005`), all three, with the explicit comment that this preserves linear-search first-match on duplicate symbols. `fielddefs_map` is over-reserved by `countof(extrafields) * 3` for the later merge. `quake_util::hash_map::QHashMap` already exists; first-wins is the *caller's* responsibility.
7. **`PR_MergeEngineFieldDefs`** (`pr_edict.c:1732-1795`) appends 8 engine fields (+ synthesized `colormod_x/_y/_z`), mutating `entityfields` → `edict_size` → savegame output, and **reallocates `fielddefs` away from the progs image**. `PR_ClearProgs` (:1706) distinguishes the two ownership states by pointer comparison against the image.
8. **Byteswap is in-place after load** (`pr_edict.c:1878-2012`); `progscrc`/`progshash` are computed **before** it (:1872-1876). Zero-copy LE reads must not disturb that ordering.
9. **CRC diagnostics** (`pr_edict.c:1893-1927`): `PROGHEADER_CRC` 5927 plus a hardcoded foreign-CRC switch (22390, 52195, 54730, 26940, 32401, 38488, 26905, 14046) whose console text must match byte-for-byte.
10. **Interpreter operand fork** (`pr_exec.c:302-304` vs :528/533/577): `dstatement_t`'s `short a,b,c` are read **unsigned** as global offsets and **signed** as jump deltas. `OP_DIV_F` has no zero guard; `OP_BITAND`/`OP_BITOR` use C float→int truncation (arch-dependent UB — reuse `quake-net`'s `c_cast_i32` per-arch emulation). Runaway threshold is `0x1000000` (:337, deliberately unlike vanilla/QSS — error text parity).
11. **`PR_SwitchQCVM` `Sys_Error`s if a VM is already active** (`pr_edict.c:1659`), so every non-NULL switch is bracketed by a NULL switch; 50 call sites across 9 files, including the render thread under `draw_qcvm_mutex` (`gl_rmain.c:715`, `gl_screen.c:1142`, `gl_rmisc.c:266`). ADR-008: resolve the ambient global **once per boundary entry**.
12. **Re-release builtin patching** (`pr_edict.c:1835-1861`): `exbuiltins[]` rewrites `first_statement` −90→−73, −91→−23, −92→−24 **by function name, only when the value matches exactly**, and runs *after* `PR_EnableExtensions` (:2036) — so it can undo an extension binding. `PR_FindSupportedEffects` (:1810) probes `EF_QUADLIGHT`/`EF_PENTLIGHT`/`EF_PENTALIGHT` float values to set `sv.effectsmask` (Arcane Dimensions depends on it).
13. **ADR-005 `%g`/`%e` are unimplemented and `panic!`** — `ROADMAP.md:42` names this as a Phase 6 obligation. Every writer moved onto `quake_util::printf` must have its specifier set audited first: `PR_FloatFormat`/`PR_DoubleFormat` (`pr_edict.c:432`), `PR_ValueString`/`PR_UglyValueString`, `ED_Write`, `ED_WriteGlobals`, and the `csprogsvers/%x.dat` key (`%x` is implemented).
14. **Debug/release layout fork**: `edict_t` gains 3 header fields under `DEBUG`/`_DEBUG` (`progs.h:50-58`); `EDICT_NUM`/`NUM_FOR_EDICT`/`NEXT_EDICT`/`EDICT_TO_PROG`/`PROG_TO_EDICT` are functions in debug and macros in release (`progs.h:141-155`). Mirror asserts must be per-profile — the `engine-debug` cargo feature already exists for this.
15. **`pr_comp.h:27-42`** declares the 64-bit QC types `Q_ALIGN(4)` — deliberately under-aligned. Rust must use unaligned reads, not `&u64`.
16. **Process-global, not per-VM**: `pr_string_temp[1024][1024]`/`pr_string_tempindex` (`pr_cmds.c:26`), `checkpvs` (:742), `qctoken` (`pr_ext.c:1498`), `qcfiles` (:3042), `strbuflist[64]` (:3280), `nearsurface_cache_valid` (:2143). Both VMs share them; `PR_ShutdownExtensions` (:6078) wipes all of them at map end regardless of which VM tore down. Moving them into `qcvm_t` would be a behavior change.
17. **`PF_Fixme` self-patches** (`pr_ext.c:6033-6075`): it decodes the callee from `qcvm->statements[qcvm->xstatement]`, looks the number up in `extensionbuiltins[]`, writes `qcvm->builtins[binum]` and calls it — so `xstatement` must be accurate at builtin entry. `PR_InitExtensions` (:6135) mutates the *static* table, assigning undocumented builtins numbers counting **down from 1024**, so numbering depends on table order and on running exactly once.
18. **Trace format** (`pr_trace.h:26-46`, emitters `pr_trace.c:49-96`): `PRTRACE 1` header; `E/L/S/W/P/B/R` records; offsets and counts signed decimal, values unsigned hex without `0x` or padding. Hooks at `pr_exec.c:235,276,348,472,478,488,495,563,565`. **Only headless traces are oracles** (CSQC drawing runs on a task worker). Traces are pairwise-compared, never committed as goldens — so bumping the format version is cheap.
19. **PLAN §7.3 promises string-temp allocation records that the C emitter does not emit.** Resolve by adding a `T` record to both sides (bump to `PRTRACE 2`) at the first temp-string builtin in M7 — not now.
20. Harness gaps to fill: no `trace_diff.py`, no `-Dtrace` job in any workflow, and `Misc/harness/README.md:21` notes **CSQC (`cl.qcvm`) globals are not hashed** — the trace oracle has to cover CSQC instead.
21. `Con_Printf` is not a leaf (it can reach `SCR_UpdateScreen`) — Phase 5's lesson. Accumulate diagnostics Rust-side and drain them from the C glue frame after all borrows end.

## Flip-mechanism map

| Scope | Mechanism |
|---|---|
| `pr_exec.c` (M3) | **Pattern A** whole-file swap in meson + new `Quake/pr_exec_glue.c`. Glue keeps the *diagnostic* printers (`PR_PrintStatement`, `PR_StackTrace`, `pr_opnames[]`, `PR_Profile_f`) in C, owns nothing else, and re-raises the Rust `PR_ExecuteProgram` status as `Host_Error` from a C frame (ADR-009). `PR_RunError`'s noreturn contract becomes: Rust returns an error descriptor (message + `xstatement`/`depth` snapshot) → glue prints → `Host_Error`. |
| `pr_edict.c` (M4/M5) | **Pattern A** whole-file swap + `Quake/pr_edict_glue.c`, which keeps the C-owned storage every other file reads: `qcvm`, `pr_global_struct`, the 11 cvars (`nomonsters`, `gamecfg`, `scratch1-4`, `savedgamecfg`, `saved1-4`), `type_size[]`, plus Host_Error trampolines. M4 flips the loader half, M5 the `ED_Write`/`ED_Parse*` half; the file is `#ifndef`-sectioned in between. |
| `pr_cmds.c` / `pr_ext.c` (M6+) | **Pattern C** per-builtin vtable-slot flip: under `USE_RUST_PROGS`, individual entries in `pr_ssqcbuiltins[]` / `pr_csqcbuiltins[]` / `extensionbuiltins[]` point at `rust_pf_*` using designated initializers, with a per-slot comment where a builtin stays C. Both files stay compiled in both configs. This is what makes the user's "decide per batch" scope workable. |
| Ambient `qcvm` | Boundary shims call `quake_rs_progs_current_vm()` once per entry (ADR-008). `PR_SwitchQCVM` itself stays C until M4. |

One meson option `use_rust_progs` (`'auto'`, follows `use_rust`, enabled-without-`use_rust` errors — verbatim `use_rust_net` pattern, `meson.build:386-390`) → cargo feature `progs`. Per-milestone granularity via `#ifdef` contents, not extra options.

## Milestones — THIS SESSION (M1–M5)

One commit per milestone; tree green (all configs build, gates below) after each.

### M1 — Scaffolding: ABI mirrors, the trace differ, meson/CI wiring
- `rust/quake-types/src/progs.rs`: hand-written `#[repr(C)]` mirrors (ADR-011) for `dprograms_t`, `dstatement_t`, `ddef_t` (+`DEF_SAVEGLOBAL`), `dfunction_t`, `edict_t`'s header (incl. the debug-only 3-field prefix), `prstack_t`, `freelist_t`, `areanode_t`, `qcvm_t`, and `entvars_t`/`globalvars_t` from `progdefs.q1`; `etype_t`, `opcode_t`, `OFS_*`, `PROG_VERSION 6`. Const size/offset asserts **per build profile** via `engine-debug`.
- `rust/quake-ctest/stubs/abi_probe.c`: progs lookup table; `rust/quake-ctest/tests/progs_abi.rs` using the existing `check_size!`/`check_offset!` macros.
- `rust/quake-progs/src/lib.rs`: `#![deny(unsafe_code)]` module index with one `#[allow(unsafe_code)] pub mod arena` island (ADR-004 Phase-5 precedent); deps `quake-types` + `quake-util` only — **no `quake-c-sys`**, so the crate stays fuzzable.
- quake-c-sys allowlist + `scripts/gen_c_bindings.sh` regen: `qcvm`, `pr_global_struct`, `Host_Error`, `Con_Printf`/`Con_DPrintf`/`Con_Warning`, `PR_PrintStatement`, `PR_StackTrace`, and a `PRExec_Glue_SvState()` accessor funnel for `sv.state` (server.h is not bindgen-clean).
- meson: `use_rust_progs` option in `meson_options.txt` + the four wiring sites in `meson.build`; `progs` feature in `quake-capi/Cargo.toml` and in quake-ctest's feature list; feature-gated `capi::progs` stub module.
- **`scripts/harness/trace_diff.py`** — the ADR-019 gate-3 consumer: run `run_trace.py` on two builds over a corpus entry, gunzip-stream both, compare record-by-record, and on mismatch report the first divergent record with N records of context plus the decoded opcode name. Enforce a **minimum record count** so an inert trace cannot pass (Phase 5's delivered-record-floor lesson).
- CI: `build-rs-cprogs` (`-Duse_rust_progs=disabled`) added to the three lists in `.github/workflows/build-linux.yml` (build step, `check_capi_signatures.sh` line, `--compare`/`save_diff` legs); a new `-Dtrace=true` C build (`build-c-trace`) plus a `trace_diff.py` self-compare step proving the differ and the producer are stable before anything is ported.
- Docs: commit this plan as `docs/ai/plans/rust-conversion-phase-6.md`; ADR-007 progs row (`qcvm` C-owned, P6–P7 dual view); ADR-006 amendment (arena-as-view during P6; the "no reference across a builtin dispatch" tightening); ADR-004 amendment (the `arena` unsafe island); ADR-011 note (progs headers are not bindgen-clean roots).
- Gates: all configs build; `run_corpus.py --check` unchanged; bindgen regen-diff clean; `check_headers.sh`, `check_capi_signatures.sh`, `check_ctest_symbols.sh` clean; `trace_diff.py` C-vs-C green on `id1-demo1` and `map-e1m2`.

### M2 — Edict arena + string table (ctest-only, no engine flip)
- `quake-progs::arena`: `EdictArena` over a base pointer + `edict_size` stride, `EdictId(u32)`, `FieldOfs` handles validated once, `EntVars` view; `EDICT_TO_PROG`/`PROG_TO_EDICT` byte-offset semantics; both the debug and release header layouts. Raw base pointers re-derived per access (landmine 3).
- `quake-progs::alloc`: `ED_Alloc` FIFO free-list with `MAX_EDICT_FREETIME_ALWAYS_REUSE`, `ED_Free`, `ED_AddToFreeList`/`ED_RemoveFromFreeList`, `ED_freetime_compare_func`, `ED_CheckFreeList`, `ED_RebuildFreeList` — entity numbering is observable, so this is bit-exact or nothing.
- `quake-progs::strings`: `PR_GetString`/`PR_SetEngineString`/`PR_AllocString`/`PR_ClearEngineString`/`PR_ClearEdictStrings`/`ED_NewString`/`ED_RezoneString` + the `knownzone` bitmap, with `// COMPAT:` on every landmine-5 quirk incl. the unreachable `Host_Error` and the off-by-two.
- ctest: `c_ref_` the relevant `pr_edict.c` sections via `include/c_ref_prelude.h`; `progs_arena_differential.rs`, `progs_strings_differential.rs` — randomized alloc/free/realloc sequences comparing free-list state and every returned number, plus the negative/out-of-range string domain.
- **Miri**: a re-entrancy model test — Rust arena access → simulated builtin → nested arena access — run under `cargo +nightly miri test` locally, recorded in the amendment log.
- Gates: ctest green; engine untouched; corpus + trace_diff unchanged.

### M3 — Interpreter port + first engine flip + trace parity goes live
- `quake-progs::exec`: `PR_ExecuteProgram`, `PR_EnterFunction`/`PR_LeaveFunction` (raw localstack int copies), the full opcode switch transliterated per landmine 10, runaway counter, profile accounting, and the trace emitters at exactly the nine C hook sites with identical operand derivations. Errors are `Result`, never raised.
- `quake-capi::progs_exec` + `Quake/pr_exec_glue.c` per the flip map; meson swaps `pr_exec.c` out under `use_rust_progs`.
- ctest: `progs_exec_differential.rs` over synthetic progs images exercising every opcode incl. `OP_DIV_F` by zero, `OP_BITAND`/`OP_BITOR` float→int UB per-arch, negative jump deltas, `OP_ADDRESS` world-entity guard, `OP_STATE`, stack overflow/underflow, and the builtin-dispatch path via a mock builtin table.
- **The gate that defines this phase**: `trace_diff.py build-c-trace vs build-rs-trace` byte-identical over the corpus. This is the first milestone where a real progs.dat executes on Rust.
- Gates: trace parity on `id1-demo1/2/3`, `save-e1m1`, `map-e1m2` + local registered-tier entries; `run_corpus.py --check` and `--compare`; `save_diff.py`; capi signature parity on `build-rs-cprogs`.

### M4 — Loader
- `quake-progs::load`: `PR_LoadProgs` — version/CRC checks and the foreign-CRC diagnostic switch, CRC16 (`quake_util::crc::crc_block`) and folded MD4 (`quake_util::mdfour::block_checksum`) computed **before** the in-place byteswap, lump byteswap in C order, the three reverse-built hash maps with the `extrafields*3` over-reserve, `PR_MergeEngineFieldDefs` with exact append order and synthesized `colormod_x/_y/_z`, builtin-table copy, `PR_PatchRereleaseBuiltins` after `PR_EnableExtensions`, `PR_FindSupportedEffects`, `PR_ClearProgs` with its two-ownership-state `fielddefs` test.
- `PR_SwitchQCVM` + the active-VM `Sys_Error` assertion move to Rust; `qcvm`/`pr_global_struct` storage stays C in `pr_edict_glue.c`.
- ctest: `progs_load_differential.rs` — load every `progs.dat`/`csprogs.dat` in the local depot on both sides and compare the whole resulting `qcvm_t` field-by-field, all three hash maps by exhaustive lookup, and the merged fielddef table byte-for-byte.
- `fuzz_progs_load` target (pure Rust, malformed progs images) + committed seeds + the CI loop entry.
- Gates: M3's gates plus loader coverage; `trace_diff` still byte-identical (the loader now feeds the Rust interpreter).

### M5 — `ED_Write` / `ED_Parse*` + the savegame gate
- **ADR-005 specifier audit first** (landmine 13): enumerate the format strings in `PR_FloatFormat`, `PR_DoubleFormat`, `PR_ValueString`, `PR_UglyValueString`, `ED_Write`, `ED_WriteGlobals`, `ED_Print`, `ED_FieldValueString`; implement any missing `quake_util::printf` conversion **before** moving a writer onto it, or keep that writer C and record why.
- `quake-progs::save`: `ED_Write`/`ED_WriteGlobals` with every skip rule, `ED_ParseEdict`/`ED_ParseGlobals`/`ED_ParseEpair` incl. the bare-token vector coercion and `"entity "` prefix handling, `ED_LoadFromFile` spawn dispatch, `ED_IsRelevantField`/`ED_AppendFlagString`/`ED_FieldValueString`.
- `pr_edict.c`'s save/parse section flips; `host_cmd.c` call sites unchanged.
- ctest: `progs_edwrite_differential.rs` (write both, byte-diff) and a parse corpus over real `.sav` files and `.ent`/entity lumps.
- `fuzz_ed_parse` target + seeds + CI entry.
- Gates: `save_diff.py` byte-clean C-vs-mixed **and** cross-load compat both directions (C-written save loaded by Rust and vice versa); trace parity; full local corpus. **Then STOP for user review.**

## Future milestones (M6–M10, next session; designed, not executed now)

- **M6** — `pr_cmds.c` core builtin table, batch 1: the self-contained builtins (math, vector, string/`PR_GetTempString`, `PF_random`/`PF_aim`/`PF_changeyaw` via `quake_c_sys::libm` per ADR-010, find/precache, cvar/console). Per-builtin dispatch flip; `PRTRACE 2` gains the `T` string-temp record (landmine 19).
- **M7** — `pr_cmds.c` batch 2 (the `sv_*`/`cl`-coupled builtins) — port where the dependency is a stable C funnel, carve out where Phase 7 dominates; `WriteDest` + the eight `PF_sv_Write*`.
- **M8** — `pr_ext.c` batches by dependency class, stopping per the user's per-batch rule: strings/`PF_sprintf_internal`/tokenizer/type-conversions/FRIK_FILE/strbufs/`PF_crc16` first (self-contained, ~2500 lines); the temp-entity and particle blocks next; **explicitly carved out**: the CSQC 2D drawing block (`pr_ext.c:4736-5086`, hard Vulkan coupling) and `PF_getsurface*`/`PF_cl_getrenderentity` → Phase 8, shimmed through `quake-capi` to C rendering per `ROADMAP.md:159`.
- **M9** — Extension machinery + CSQC: `PR_InitExtensions`/`PR_EnableExtensions`/`PR_ShutdownExtensions`, `PF_Fixme` lazy self-patching resolution, `PF_checkextension`/`builtinsupported`/`checkbuiltin`, `qcextensions[]`, `PR_DumpPlatform_f`, `PR_AutoCvarChanged`; the CSQC load chain (`csprogsvers/%x.dat` → `csprogs.dat` → `progs.dat`, `CSQC_DrawHud`); **builtin-table dump diff gate** (ordinal→name after resolution and re-release patching, for every progs.dat in the corpus).
- **M10** — Phase exit: full ADR-019 gate-3 checklist, fuzz soak, ROADMAP carve-out note + deferred-deletes list, `/integration-review` from fresh context, ROADMAP checkbox.

## Risks (top)

| Risk | Mitigation |
|---|---|
| Builtin re-entry into `PR_ExecuteProgram` while Rust holds `&mut` → UB | Re-derive raw pointers per step; no borrow crosses a dispatch; Miri model in M2; ADR-006 amendment states the rule |
| `qcvm_t` mirror drifts from the C struct (400 lines, two build profiles) | `abi_probe.c` const asserts on size + every offset, per profile; `progs_abi.rs` in CI on all three OSes |
| Trace differ passes vacuously (empty/short traces) | Minimum-record-count floor + C-vs-C self-compare proving the gate in M1, before any port |
| `%g`/`%e` `panic!` reached from a savegame writer at runtime | Specifier audit is the first task of M5, not a discovery during it |
| `Host_Error` longjmp through Rust interpreter frames | ADR-009 status returns from M3 onward; diagnostics accumulated Rust-side and drained from the C glue frame (landmine 21) |
| Phase 6 balloons into Phase 7 via `pr_ext.c`'s sv/cl coupling | Per-batch stop rule + explicit carve-out recorded in ROADMAP (Phase 5 `net_wins.c` precedent) |
| CSQC state is invisible to the state-hash harness | Trace parity covers `cl.qcvm` instead; noted as the coverage argument in M3's gates |

## Non-goals

`sv_*`/`cl_*`/`world.c` simulation (Phase 7); the CSQC 2D drawing builtins and `PF_getsurface*` (Phase 8); `tasks.c` (ADR-016); any C deletion; hashing CSQC globals in `harness.c`; fixing the preserved bugs; the `PRTRACE 2` format bump (M6); M6–M10 in this session.

## Verification summary (this session)

Per milestone: `cargo test -p quake-ctest` (targeted differentials), `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, `./format.sh` then `git checkout -- Shaders/`, and the affected meson configs (`build-c`, `build-rs`, `build-rs-cprogs`, `build-c-trace`, `build-rs-trace`).

Milestone boundaries: `run_corpus.py --check` (darwin-arm64 goldens) + `--compare` C-vs-mixed, `save_diff.py`, **`trace_diff.py`** from M3 on, `check_headers.sh`, `check_capi_signatures.sh`, `check_ctest_symbols.sh`, bindgen regen-diff, `cargo deny check`.

M5 exit (the "review and validate" step for this session): full corpus including local registered-tier entries, savegame byte-diff + cross-load compat both directions, trace parity across every corpus entry, fuzz smoke on the two new targets, then a fresh-context `compatibility-reviewer` pass over M1–M5 against this plan and the ADRs (note: that agent's final report gets truncated in the task result — recover it with a follow-up "deliver the report" message). Findings addressed and logged in the amendment log before stopping for user review.

## Amendment log

- **2026-08-27 M1**: landed. Deviations from the M1 bullet list, all recorded here rather than silently absorbed:
  1. **The `quake-c-sys` allowlist additions were deferred to M3.** M1's plan listed `qcvm`, `pr_global_struct`, `Host_Error`, the console printers and a `PRExec_Glue_SvState()` funnel. None has a consumer yet — `quake-progs` is deliberately `quake-c-sys`-free and the first capi shim lands with the interpreter — so adding them now would commit dead bindings and a dead accessor to the regen-diff. They land with `pr_exec_glue.c`.
  2. **No `capi::progs` stub module.** Same reason: the `progs` cargo feature and the `-Duse_rust_progs` switch are wired end-to-end (meson → feature list → `-DUSE_RUST_PROGS`), but an empty module would be noise. `quake-ctest` picks the feature up at M3 too.
  3. **`MIN_EDICTS` / `MIN_EDICT_AGE_FOR_REUSE` added alongside `MAX_EDICT_FREETIME_ALWAYS_REUSE`.** `freelist_t` sizes its circular buffer on `MAX_EDICTS` and `ED_Alloc`'s reuse policy is written against *both* age thresholds, so the prelude and the Rust consts carry all four.
- **2026-08-27 M1 — verified finding: demo playback traces nothing.** `trace_diff.py` over `demo1` produced **0 records**. Demo playback never starts a server, so no SSQC runs, and id1 ships no CSQC — the trace file contains its `PRTRACE 1` header and nothing else. Two consequences, both now baked in: the trace oracle's scenarios must be **map** entries (`e1m1` gives 117,672 records over 300 frames), and the differ carries a `--min-records` floor (default 10,000) so an inert trace, an early `Host_Error`, or a build accidentally made without `-DPR_TRACE` fails loudly instead of comparing equal. This is the direct analogue of Phase 5's delivered-record floor.
- **2026-08-27 M1 — the differ's failure path was tested, not assumed.** A crafted pair of traces differing in one `B` record reports `trace divergence at record 5` with three records of decoded context and both sides' decoded records; a truncated trace reports `<end of trace>`. The C-vs-C run over `e1m1` is byte-identical.
- **2026-08-27 M1 — ADR amendments**: ADR-004 (the `quake-progs` `arena` unsafe island), ADR-006 (**two** clarifications: the arena is a *view* over C-owned memory until the loader flips, and "no reference escapes a VM step" is tightened to "no reference lives across a builtin dispatch", which is the real aliasing hazard), ADR-007 (two new rows: the ambient `qcvm`/`pr_global_struct`/edict-array dual-view window, and the Rust-owned VM internals), ADR-011 (`progs.h` is not a bindgen-clean root; the `engine-debug` layout fork and the probe's `const.ENGINE_DEBUG` guard).

- **2026-08-27 M2**: landed — the edict arena, the FIFO free list and the progs string table, ctest-only. 18 differential tests green plus a Miri model; the engine is untouched.
  1. **Deviation from the flip-mechanism map: `pr_edict.c` was split, not `#ifdef`-sectioned.** `Quake/pr_edict_arena.c` now holds the free-list block and the string table, moved **verbatim** (behaviour-neutral, the Phase 2/3/5 idiom). Two reasons: the differential oracle gets a stub surface of five symbols instead of pr_edict.c's whole dependency graph, and M4/M5 can flip this file with the established Pattern-A meson swap rather than threading `#ifdef`s through 2300 lines. Neutrality verified: `run_corpus --check` still matches the darwin-arm64 goldens and the e1m1 trace is unchanged at 117,672 records.
  2. **`ED_NewString`/`ED_RezoneString` and the `knownzone` bitmap moved to M5.** Both are `static` in `pr_edict.c` and have exactly one caller, `ED_ParseEpair`, which is M5 work; splitting them out now would have broken their internal linkage for no gain. M5's scope line therefore reads "the parse half **and** its two string helpers".
  3. **`ED_RebuildFreeList`'s sort is a parameter, not an implementation.** `ED_freetime_compare_func` returns `(int)copysign (1.0, a - b)`, which is **never 0**, so equal freetimes compare as "greater" and the comparator is *inconsistent*. `qsort` with an inconsistent comparator has implementation-defined ordering — and ties are the common case, because every edict freed in one frame gets the same `freetime`. Reproducing that means calling the same platform `qsort`, so `rebuild_free_list` takes `sort: &mut dyn FnMut(&mut [c_int])`; the engine and the tests both hand it libc's. `quake_progs::alloc::freetime_compare` carries the comparator itself so only the sort call is injected.
  4. **`EDICT_NUM`'s bounds check is currently an `assert!`.** C's `EDICT_NUM` `Host_Error`s on `n < 0 || n >= max_edicts`; the Rust arena panics instead. Harmless while the arena is only reachable from ctest, but **M4/M6 must route it as a status before the engine flip** (ADR-009: a panic must never cross into a C frame). Tracked as a flip precondition, not a bug in the port.
  5. **`ED_Alloc` clears `entityfields * 4` bytes, not `edict_size - offsetof(edict_t, v)`.** The two differ by the pointer-alignment padding `edict_size` is rounded up with, and C leaves those padding bytes alone. `clear_fields` preserves that; the byte-for-byte edict comparison in `progs_arena_differential` is what would catch a regression.
  6. **`ED_ALLOC_HOOK` stays C-side.** `ED_AllocSetHook`'s hook is read by `sv_phys.c`; the Rust `ed_alloc` returns the id and the caller invokes the hook, so ownership does not move this phase.
  7. **Miri, strict provenance, green** on the re-entrancy model: an `EdictArena` over an externally-owned allocation, with `ed_free`'s unlink callback writing the same edict through a second pointer of the same provenance while the Rust call is still on the stack. This is the ADR-006 amendment's rule under test, not just in prose.
  8. Test-fixture note: `ctest_progs_reset_vm` computes `edict_size` with `PR_LoadProgs`' exact expression, so the oracle's stride and the Rust arena's agree by construction rather than by a transcribed constant.
