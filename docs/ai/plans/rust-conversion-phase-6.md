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
- **2026-08-27 M3**: landed — the interpreter is ported and **flipped**; `pr_exec.c` is no longer compiled under `-Duse_rust_progs`. ADR-019 gate 3 is live and green.
  1. **`Host_Guard`/`Host_Reraise` were built here** (`Quake/host.c`, `quakedef.h`). ADR-009 rule 3 named the trampoline but nothing had needed it yet; the interpreter does, because `OP_CALL*` dispatches C builtins that `Host_Error`. The subtlety the ADR did not anticipate: `Host_Error` shuts down the server, disconnects and prints *before* it jumps, so catching the jump and calling `Host_Error` again to re-raise would do all of that twice. The guard therefore catches, and a separate `Host_Reraise` re-issues the *same* jump from a pure C frame once Rust has unwound. Both `host_abortserver` and `screen_error` are hijacked (Host_Error takes the latter under CSQC drawing). Nesting unwinds one guard at a time. Recorded as an ADR-009 amendment.
  2. **The per-builtin guard cost was measured, not assumed.** The e1m1 trace has 4,028 builtin calls over 300 frames — ~13 per frame — so two `jmp_buf` copies plus a `setjmp` per dispatch is not measurable against a 13.9 ms frame. Written into the ADR-009 amendment so a future milestone that puts a guard somewhere hot has to re-measure.
  3. **Accepted divergence: the interpreter range-checks edict field offsets and function indices.** C's `OP_STOREP_*`, `OP_LOAD_*` and `OP_CALL*` index with no bounds check at all, so a malformed progs is an arbitrary read/write primitive reachable from mod data. The port raises instead. No progs in the corpus reaches it — trace parity over five maps is byte-identical — so the divergence is unobservable for valid input. Recorded in `arena.rs` and in the `pr_exec_glue.c` switch.
  4. **`sv.state` and `strcmp` are `ExecSys` callbacks, not reimplementations.** `OP_NE_S` stores `strcmp`'s *raw return value* into a float slot where QuakeC can read it, and libc's exact non-zero result is platform-specific — so the port calls the platform `strcmp` rather than a Rust byte comparison (ADR-010).
  5. **`PR_GetString` is now shared, not duplicated.** M2's `StringTable::get` and M3's borrow-free `VmRaw::get_string` both delegate to one `resolve_string` helper, so the owning view and the interpreter path cannot drift.
  6. **The prelude's rename strategy changed.** M2 renamed the ambient `qcvm` to `c_ref_qcvm`; an object-like macro also rewrites the identically-named *struct field* `sv.qcvm`, which `PR_Profile_f` dereferences. Only symbols the oracle files themselves define are renamed now; stub-owned ambients keep their real names — which is also what the quake-capi progs shims will import.
  7. **`FuncRef` newtype.** Clippy's `not_unsafe_ptr_arg_deref` flagged `VmRaw`'s `*mut DFunction` parameters. Wrapping them is not lint-dodging: it means `exec` (a `deny(unsafe_code)` module) cannot fabricate a function pointer — every `FuncRef` comes from the VM's own lump.
  8. **Coverage**: real gameplay reaches 53 of 66 opcodes. `progs_exec_differential.rs` covers the other 13 plus the cases no shipping progs takes — `OP_DIV_F` by zero (compared as raw bits, so a NaN payload difference would show), the `OP_BITAND`/`OP_BITOR` float→int truncation across the INT_MIN/INT_MAX/NaN/infinity domain, negative jump deltas, the `OP_ADDRESS` world guard in both server states, stack overflow and underflow, and the out-of-range builtin ordinal falling back to slot 0.
  9. **The `progs` capi feature is still not enabled in quake-ctest** (M1 deferred it to M3; deferring again). Enabling it makes `quake-capi::progs_exec` import `qcvm` and the twelve `PRExec_Glue_*` entry points, none of which exist in the ctest link — they live in `pr_exec_glue.c`, which is engine-only. Adding stand-ins would mean reimplementing the guard and the trace sink in `stubs.c` to test a shim that is a thin translation layer. The shim is covered instead by `check_capi_signatures.sh` (which now includes `progs.h`, so `quake_rs_pr_execute_program`'s declaration is checked against the engine headers) and by the engine-level trace gate, which exercises it on every instruction.
  10. **Gates**: `trace_diff.py` byte-identical C-vs-Rust over e1m1/e1m2/e2m1/e3m1/e4m1 (117k–308k records each); full corpus `--check` and `--compare`; `save_diff` clean. CI gained `build-rs-trace` and a three-map parity leg.
- **2026-08-27 — plan amendment: M4 and M5 swapped, and the loader moves out of this session's block.** M4 as designed (the loader) calls `ED_NewString`, which the M2 amendment had already deferred to M5 — so the planned order had M4 depending on M5. Rather than thread a callback around a dependency the plan created, the savegame work comes first. This session's block is now: M1 scaffolding, M2 arena+strings, M3 interpreter, **M4 savegame writer**, **M5 savegame reader**. `PR_LoadProgs`/`PR_MergeEngineFieldDefs`/`PR_ClearProgs` become **M6**, first item of the next session, joining the builtin milestones (which shift to M7–M11). Nothing is dropped; the ordering is now dependency-correct.
- **2026-08-27 M4**: landed — the savegame **writer** is ported and flipped. `save_diff.py` reports byte-identical 91 KB savegames C-vs-Rust.
  1. **The ADR-005 specifier audit (ROADMAP.md:42) is discharged, and the answer is "nothing to do".** Every format string in `pr_edict.c` was enumerated mechanically: `%c %d %f %i %p %s %u` plus `PRIi64`/`PRIu64`, with `%f` appearing as `%f`, `%5.1f`, `%7.1f`, `%+.2f`, `% 5.0f`, `% 7.1f`, `% 13.0lf`, `% 15.1lf`. **There is no `%g` or `%e` anywhere** — in `pr_edict.c`, in `host_cmd.c`'s savegame lines, or in the `csprogsvers/%x.dat` key. The two conversions `quake_util::printf` leaves unimplemented are unreachable from this phase, so no formatter extension was needed. `%p` appears only inside `#if defined(DEBUG)` `Host_Error` diagnostics in `EDICT_NUM`/`NUM_FOR_EDICT`, which stay C.
  2. **Second behaviour-neutral C split**: `Quake/pr_edict_save.c` holds `PR_UglyValueString`, `ED_Write` and `ED_WriteGlobals`, moved verbatim; `pr_edict_save_glue.c` replaces it under the switch. `ED_FieldAtOfs` became non-static (declared in `progs.h`) because both the split TU and the Rust writer's `SaveSys` callback need it. Neutrality verified before the flip: goldens and `save_diff` unchanged with the C split alone.
  3. **Scope line — the console printers stay C.** `ED_Print`, `ED_FieldValueString`, `ED_IsRelevantField`, `ED_AppendFlagString`, `ED_PrintNum`/`ED_PrintEdicts`/`ED_Count` and `PR_ValueString` are the `edict` console command's path. They carry the large MOVETYPE/SOLID/FLAGS decode tables, never reach a savegame, and `PR_ValueString` is the only user of the `% 7.1f`-style formats. Left in `pr_edict.c` deliberately, not overlooked.
  4. **`PR_UglyValueString` returns bytes, and the glue owns the static buffer.** C returns a pointer to a 1024-byte `static char line[]`; the shim renders into the caller's buffer with the same truncation. `ED_Write`/`ED_WriteGlobals` build a reused Rust buffer that the glue `fwrite`s — same bytes, one write instead of many `fprintf`s, which cannot change file content.
  5. **Accepted divergence: `ev_field` with an unknown offset.** C dereferences `ED_FieldAtOfs`'s result with no null check; the port writes an empty name instead of crashing. Unreachable from a well-formed progs.
  6. **Gates**: `save_diff.py` byte-identical (91,562 and 91,547 bytes) both C-vs-Rust and C-progs-vs-Rust; 8 differential tests covering every `PR_UglyValueString` arm over subnormals, ±0, ±inf, NaN, `f32::MAX`, `i64::MIN`/`MAX` and the `Q_ALIGN(4)` two-word reads; every `ED_Write` skip rule; the free-edict short circuit; the manual-alpha fallback across both `extfields.alpha` states; and `ED_WriteGlobals`' type filter. Trace parity and the full corpus unchanged.
- **2026-08-27 M5**: landed — the savegame **reader**'s value parser is ported and flipped. Corpus, `save_diff` and trace parity all green with the Rust parser on both the write and read paths.
  1. **Scope line: the value parser moves, the key dispatchers stay.** `ED_NewString`, `ED_RezoneString` and `ED_ParseEpair` are Rust; `ED_ParseEdict` and `ED_ParseGlobals` stay in `pr_edict.c`. Their `COM_Parse` loops over the ambient `com_token`, the `_precache_model`/`_precache_sound` hacks, the PSET_SCRIPT `traileffect`/`emiteffect` branches and the `sv.state` tests are server code Phase 7 owns — the same carve-out shape Phase 5 used for `net_main.c`'s dispatch funnels. What moved is where the compatibility risk actually is: the numeric conversions and the entity-allocation side effects.
  2. **`ED_NewString` had to be exported.** `ED_ParseEdict` and `PR_MergeEngineFieldDefs` both still call it, so it is no longer `static` and is declared in `progs.h`. Third behaviour-neutral split: `Quake/pr_edict_parse.c`, verified against the goldens and `save_diff` before the flip.
  3. **COMPAT (ADR-010): every numeric conversion is a callback, not a Rust parser.** `atof` is `strtod`, and its rounding of a decimal savegame literal back to the binary value *is* the round-trip the byte-diff gate checks. Same for `atoi` and for `strtoll`/`strtoull` — which C calls with **base 0**, so `0x`/`0` prefixes are honoured there but not in the `atoi` arms. Both sides of the differential call the same libc.
  4. **Bug caught by the gates, not by review.** The first flip built and passed the ctest suite but failed 10 of 11 corpus entries with a Rust panic. Cause: the capi shim built an `EdictArena` unconditionally, and `ED_NewString` also runs *during* `PR_LoadProgs` — `PR_MergeEngineFieldDefs` calls it before `edict_size` is computed and `edicts` allocated — so the arena's stride assertion fired on a not-yet-loaded VM. Fixed by splitting `ambient()` into `ambient_vm()` and `ambient_arena()`, and only the `ev_entity` arm needs the latter. This is exactly the class of thing the corpus gate exists for: the unit tests all constructed fully-loaded fixtures.
  5. **Fixture finding: `ED_Free` requires `entityfields >= 105`.** It resets fields as far out as `nextthink`, so a fixture with a smaller field block has C writing into the *next* edict while the Rust arena's bounds check catches it. A real progs always defines at least the engine block, so C never notices. The parse fixture now uses 128; recorded because a future milestone writing fixtures could trip over it again.
  6. **COMPAT quirks preserved with notes**: `ED_NewString`'s escape loop runs inclusive of the NUL and does not treat a *trailing* backslash as an escape; a `\` followed by anything but `n` becomes a lone `\`, shortening the string and leaving the tail of the allocation as the zeroes `Mem_Alloc` gave. `ED_ParseEpair`'s `ev_ext_uint32` arm uses `atoi`, not an unsigned parse. The vector arm copies into a 128-byte buffer first, so a longer literal is truncated before parsing, and its `w <= end` bound lets the last component start exactly at the terminator.
  7. **Gates**: 8 differential tests (every scalar arm over 20 float and 13 integer literals including `inf`/`nan`/overflow/garbage; the vector arm's splitting, truncation and zero fill; field/function lookup including the silent `sky`/`fog` suppression; `ED_NewString`'s escape table; the `ev_entity` arm's `num_edicts` extension and gap-freeing; the `etos` prefix strip; the past-`max_edicts` report; the `default: break` arm). Engine-level: full corpus `--check` on all three configs, `--compare` both ways, `save_diff` byte-identical, trace parity over five maps.
- **2026-08-27 M5 follow-up — the trace oracle only covered id1 progs.** `run_trace.py` had no `--game` flag, so every trace-parity run was against `id1/progs.dat` no matter which scenario was named. Mission-pack and re-release game code — genuinely different QuakeC — was only covered by demo state-hash comparison, which does not run the server VM at all. Added `--game` to `run_trace.py` and `trace_diff.py`. Now verified byte-identical: **hipnotic/start 200,683 records, rogue/start 96,160, rerelease/e1m1 148,206**, on top of the five id1 maps. These are registered-tier data, so they stay local-only like the rest of that tier (CI has shareware only) — the same deferral Phases 0–5 carry.

### 2026-08-27 — M1–M5 compatibility review (fresh context), findings and disposition

A fresh-context `compatibility-reviewer` pass over the five commits. It covered the interpreter, the arena/free-list/string-table, the parser, the three C splits and `Host_Guard`, and **ran out of budget before reaching** the savegame writer, `progs_exec.rs`, the ABI mirrors, the ctest suites and the gate-honesty area — a second pass was commissioned for exactly those. Its verdict on what it did read was **not ready**, on the strength of findings 1–4 below.

**Confirmed correct** (verified line-by-line against the C, recorded here so it is not re-litigated): both readings of `dstatement_t`'s shorts; `OP_STATE`'s double arithmetic; `ED_Alloc`'s `entityfields*4` clear; `ED_RemoveFromFreeList`'s non-compacting removal; the injected qsort; `PR_SetEngineString`'s off-by-two; `PR_ClearEdictStrings`' `_DEBUG` fork; `PR_GetString`'s dead `Host_Error`; `ED_RezoneString`'s `knownzone` growth; `ED_NewString`'s escape loop; the `ev_ext_uint32`-uses-`atoi` arm; the vector arm's truncation and `w <= end` bound; all 66 opcode arms; `c_cast_i32`; the runaway threshold; the hoisted stack-depth check. Also: the three C splits are genuine verbatim moves (only `ED_FieldAtOfs` and `ED_NewString` changed linkage, both declared in `progs.h`), the single-unsafe-island structure holds, and `Host_Guard`'s nesting, `volatile` usage and `inerror` interaction are correct.

**Fixed in response:**

1. **Negative entity numbers were clamped instead of raising** (compatibility defect, `parse.rs`). C's guard in `ED_ParseEpair` is upper-bound only, but `EDICT_NUM` then rejects `n < 0` **unconditionally** — it is not a debug-only check. The port clamped to `EdictId(0)`, which would have marked the *world entity* allocated and mutated the free list on a savegame containing `"entity -5"`. Now returns `ParseError::BadEdictNum`, raised by the glue as C's `EDICT_NUM: bad edict_num %i`. Regression test added. This was reachable from mod/save data — the most serious finding.
2. **`EDICT_NUM` reachable unguarded from a Rust frame** (soundness defect, `pr_edict_parse_glue.c`). `PRParse_Glue_UnlinkEdict` called `EDICT_NUM`, whose bounds check raises unconditionally, with no `Host_Guard`. Switched to `EDICT_NUM_NO_CHECK` — the Rust caller has already bounds-checked and now raises for negatives. The audit the reviewer asked for is recorded in the glue: `SV_UnlinkEdict` (world.c) tests `area.prev`, calls `RemoveLink` (common.c) and nulls two pointers; neither can raise, so the callback needs no guard.
3. **The destination slice was built wider than the field** (soundness defect, `progs_parse.rs`). `type_size(ty).max(2)` gave every 1-word type a 2-word slice; `slice::from_raw_parts_mut` requires the whole range in one allocation, so a def at the tail of the globals block formed a slice one word past the progs image. No out-of-range write occurred, but it was latent UB and Miri-visible. Added `save::value_words`, which returns the exact width (3 for vectors, 2 for the `Q_ALIGN(4)` 64-bit types, else 1), with a test pinning every type.
4. **The `_DEBUG` free-list overflow raise was dropped** (compatibility defect, debug builds only). `alloc::free_list_would_overflow` existed with no caller, so a debug mixed build lost a diagnostic the C oracle has. Now called from the `ev_entity` arm and raised by the glue as C's `ED_AddToFreeList : has more than max_edicts >= %i`.
5. **Console diagnostics were lossy for non-ASCII progs strings** (nit). `to_string_lossy` replaced high-bit bytes — which Quake strings routinely carry for the coloured-text charset — with U+FFFD, so `Can't find field <name>` could print different bytes than C's `%s`. The `ParseSys` console methods now take raw byte slices.
6. **The little-endian word ordering is now asserted, not assumed** (nit). The 64-bit QC types are written low word first; C stores through `*(qcdouble_t *)d`, which would flip on a big-endian target. Added a `const _: () = assert!(cfg!(target_endian = "little"), ...)` so a big-endian port cannot land silently.

**Documented rather than changed:**

7. **The post-guard invariant** (was: unstated). `Host_Error` runs `PR_SwitchQCVM (NULL)`, `Host_ShutdownServer` and `CL_Disconnect` *before* it jumps, so after a caught guard the ambient VM is deselected and the server is down. The guard makes the *jump* safe; it does not make the *world* unchanged. Verified that `Host_ShutdownServer` does **not** call `PR_ClearProgs` (only `Host_ClearMemory` does), so the lumps survive and this is a discipline rather than a live use-after-free — but the interpreter's early return on `guard != 0` was load-bearing and undocumented. Now stated in an ADR-009 sub-section and in `ExecSys::call_builtin`'s doc comment, so a future trace-emit or diagnostic-drain on that path cannot be added innocently.

**Accepted as-is:**

8. `PR_AllocString` clamping a negative `size` to 0 where C passes it to `Mem_Alloc` — unreachable from the only caller (`ED_NewString` always passes `strlen + 1`).
9. `VmRaw::statement`'s unchecked read, where the port range-checks field offsets and function indices. The asymmetry is real but C is identical here, and bounding it would need the statements lump's length threaded through — deferred to M6, which loads it.

**Verification gaps the reviewer identified, and what was done:**

- **Miri does not cover `progs_parse.rs`'s `&mut free_list` beside the raw `VmRaw`.** Correct, and worth closing — logged for M6 rather than bolted on here, since the loader milestone touches the same shim.
- **No gate exercises a `_DEBUG` mixed build**, so finding 4 is invisible to CI. True of every phase so far; recorded rather than fixed, as adding a debug leg is a CI-cost decision for the owner.
- **No fuzz target feeds malformed savegame text through `ED_ParseEpair`.** True — `fuzz_ed_parse` was listed under the *old* M5 scope and has not landed. Findings 1 and 5 sat in exactly that gap, which is the argument for landing it; carried to M6.
- **Big-endian is structurally undetectable by the gates.** Now caught at compile time instead (finding 6).
- **Local-only ASan run added**: `map e1m2` + savegame write + savegame load under `-Db_sanitize=address` with the Rust VM — clean. Note macOS needs `-Dc_args=-Wno-deprecated-declarations` for an ASan build at all, because ASan disables `_FORTIFY_SOURCE` and exposes deprecated `sprintf` declarations in *pre-existing* C (`host_cmd.c:1538`, `pr_edict.c`'s `PR_ValueString`); CI runs ASan on Linux, where this does not arise.

**Structural note worth carrying forward.** `Quake/pr_edict_arena.c` is compiled **unconditionally**, so the edict arena, free list and string table are still C in the shipped engine — M2 was explicitly ctest-only. The Rust `alloc` module is reached from the engine only through `ED_ParseEpair`'s `ev_entity` arm, which is precisely where findings 1 and 4 sat. The corpus and trace gates therefore exercise `quake_progs::alloc` far more narrowly than "bit-exact or nothing" suggests; the C flip lands with the loader at M6.

### 2026-08-27 — M1–M5 review, second pass (the areas the first ran out of budget for)

The first pass never reached the savegame writer, `progs_exec.rs`, the ABI mirrors, the ctest suites or the gate-honesty area, and said so. A second fresh-context pass was commissioned for exactly those. Verdict: **not ready**, on one live defect (F1) plus its own budget exhaustion on area 5.

**Confirmed correct** (so it is not re-litigated): all twelve `PR_UglyValueString` arms plus the default; every `ED_Write` skip rule, including that `NUM_TYPE_SIZES = 8` correctly excludes every `ev_ext_*` type; `ED_WriteGlobals`' eight-type filter and its masked-vs-raw type distinction; `ENTALPHA_TOSAVE` in both `extfields.alpha` states; `read_i64`'s `Q_ALIGN(4)` two-word read; the ADR-005 audit conclusion. The **post-guard path is clean** — `execute_program` returns immediately on a non-zero guard, skips the builtin-return trace record exactly as C's longjmp does, touches no VM state, and has no `Drop` epilogue, so it does *not* rely on `Host_ShutdownServer` leaving the lumps alive. Every field of every mirror was hand-checked against the C headers and **all match**; `qcvm_t`'s 46 fields are exhaustively probed. `progs_save_differential.rs` was read in full and found substantive, not vacuous.

**Fixed in response:**

1. **`ED_Write`/`ED_WriteGlobals` bypassed `PR_UglyValueString`'s 1024-byte cap** (compatibility defect — a real savegame byte divergence, and the most serious finding of either pass). C formats into a `static char line[1024]` with `q_snprintf`, so a value over 1023 bytes is truncated and `ED_Write` writes the truncated form. The port returned an unbounded `Vec`, so a mod storing a long `strzone`d string in an entity field would save differently under Rust than under C — and a C-written and a Rust-written save of the same state would differ. The cap now lives in `ugly_value_string` itself, which is the function whose contract *is* that buffer, rather than only at the exported shim. Also **recorded-but-wrong**: amendment M4.4 claimed "the same truncation", which was true only of the exported symbol. Regression test added (`long_string_values_are_truncated_at_cs_buffer`) — and the reason no existing test caught it is that no fixture produced a value over 1023 bytes, which the reviewer identified as a structural blind spot in both the ctest microscope and `save_diff.py`.
2. **The exported `PR_UglyValueString` shim read four words unconditionally** (soundness defect). C reads 1–3 depending on the arm, and its callers pass pointers into the globals block or an edict's field block, so a fixed 4-word `from_raw_parts` runs past the allocation for a def at either tail — UB before the load. Now sized by `value_words(ty)`. Same fix applied to `ed_write_globals`' `g_words(def.ofs, 2)`. This is the same class as the parser slice the first pass caught; it should have been swept for then.
3. **`ev_function` with an out-of-range index was an undocumented divergence** (compatibility defect, unreachable from a well-formed progs). C computes `functions + val->function` unchecked and dereferences; the port writes an empty name. Now carries the same `// COMPAT:` treatment `ev_field` already had.
4. **`progs_abi.rs`'s `engine-debug` branch was vacuous** (vacuous test). It asserted `c_abi("edict_t.edict_ptr") == 0` — C's own `offsetof` of its first member, which cannot fail and compares nothing against the mirror — and duplicated the `area` check. The three debug-prefix fields are all 8-byte members, so a mis-ordering would leave `sizeof` and every later offset intact. Now checks each individually, under a real `#[cfg(feature = "engine-debug")]` (the fields only exist on the mirror under that feature, so a runtime `if` could not compile them); quake-ctest gained the forwarding feature.
5. **`GuardCaught(_)` mapped every unknown guard kind to `SCREEN_ERROR`** (nit). Correct today, since `Host_Guard` returns only three values, but a fourth would have been silently re-raised as the wrong jump. Now explicit, with anything else landing in the glue's existing "unknown status" arm.
6. Removed a dead loop in `manual_alpha_fallback_matches` the reviewer spotted.

**Also strengthened, prompted by finding F5** (the reviewer verified all six spot-checked mirrors by hand and found them correct, but noted the *probe* could not have caught a transposition of same-typed neighbours): `globalvars_t` (55 fields), `entvars_t` (77) and `dprograms_t` (15) are now probed **exhaustively**, field by field, rather than by size plus a handful of spot offsets — 147 offset assertions where there were 12. The four `MIN_EDICTS`/`MAX_EDICTS`/age-threshold constants are probed too. Verified non-vacuous by mutation: transposing `ammo_shells`/`ammo_nails` in the mirror — a change that leaves `sizeof` identical — now fails `progdefs_mirrors_match_engine_headers`, and did not before.

**Open, carried to M6:**

- **Area 5 (gate honesty) was not examined by either reviewer.** I verified the load-bearing properties myself rather than leaving it unattested: `trace_diff.py`'s `--min-records` floor is enforced on the comparison path; `build-linux.yml:194` is a genuine `build-c-trace` vs `build-rs-trace` comparison, not C-vs-C; and `build-rs` compiles `pr_edict_save_glue.c` rather than `pr_edict_save.c`, so the CI `save_diff` leg does gate the Rust writer. That is my own check, not an independent review, and the four other differential suites remain unexamined by a reviewer.
- The `progs` capi feature is still not enabled in quake-ctest, so the three savegame shims are covered only by `check_capi_signatures.sh` and the engine gates — no sanitizer sees them. F2 was exactly that class. Landing `fuzz_ed_parse` and enabling the feature are the two cheapest closures; both carried to M6.


### 2026-08-27 — PR #23 review response (owner review) + CI red

The PR's own review found two red CI checks and seven substantive items. Assessment and disposition below; every item was checked against the C originals before acting.

**CI (must fix — both were real, and neither was caught locally):**

- **MinGW/clangarm64 link failure.** `Quake/common.make`'s object list was never updated for the three behaviour-neutral C splits, so `pr_edict_arena.o`, `pr_edict_save.o` and `pr_edict_parse.o` were simply not built by the Makefile fallback and every symbol they define came up undefined (`PR_GetString`, `ED_NewString`, `ED_ParseEpair`, `PR_UglyValueString`, `ED_AllocSetHook`, …). All three Makefiles (`Makefile`, `Makefile.w64`, `Makefile.w64a`) share `common.make`, so one addition fixes both jobs. **Root cause worth carrying: the split milestones were verified with Meson only.** Meson names sources explicitly per-config and so tracked the new files; the Makefile carries an independent hand-maintained list that nothing cross-checks. Verified by a real `make -j8` link locally, not by inspection.
- **`cargo clippy --workspace --all-targets --locked -- -D warnings` failed.** Four errors, not the two the review named: `double_must_use` on `StringTable::get` and `needless_lifetimes` on `save::resolve` (both in `quake-progs`, which is where compilation stopped), plus `excessive_precision` and `manual_repeat_n` in `progs_save_differential.rs` that CI never reached. The local runs during implementation used the same flags but a different toolchain resolution; the fix is the code, and all four are gone.

**Findings accepted and fixed:**

1. **`PR_UglyValueString`'s `ev_entity` arm dropped a raise** (compatibility defect, and the most substantive item in the review). C goes through `NUM_FOR_EDICT`, whose `b < 0 || b >= qcvm->num_edicts` test raises `Host_Error ("NUM_FOR_EDICT: bad pointer")` in **release** builds — it is not one of the `#if DEBUG` consistency checks. The port divided silently and wrote a garbage entity number into the savegame. This is the exact mirror of the negative-entity fix the compatibility review made on the read side, which is what makes leaving it bare indefensible. Now raises through a new `SaveError::BadEdictPointer` → `PRSAVE_ERR_BAD_EDICT` → the C glue's `Host_Error`, with the identical message. Regression test `entity_values_out_of_range_raise_on_both_sides` drives both sides under the ctest `Host_Error` trap and asserts C raises where Rust raises; **verified non-vacuous by mutation** (removing the range test fails it).
2. **`ed_write` silently omitting an out-of-range field.** Accepted as a finding, resolved by documentation rather than by a raise. C reads `(char *)&ed->v + d->ofs * 4` with no bounds check at all, so the alternatives are an out-of-bounds read (which the port cannot reproduce) or inventing a raise C does not have (which would abort saves C completes). The omission stands; it is now the documented third behaviour instead of an undocumented one, and the note records that it is unreachable from a well-formed progs, where `entityfields` covers every fielddef offset by construction.
3. **The free-list overflow check collapsed two distinct C messages.** `ED_AddToFreeList` raises `"… is full"` for `size >= MAX_EDICTS` and `"… has more than max_edicts >= %i"` for `size >= max_edicts`, and the glue always emitted the second. Now a `FreeListOverflow` enum with the two arms, plumbed to two glue cases, testing `MAX_EDICTS` first the way C does so the same arm wins when both hold. Debug-only, but the message text is exactly the kind of thing this phase is otherwise precise about.
4. **The aliasing contract in `quake_rs_ed_parse_epair`.** The real problem was the contract, not the code: `EdictArena::borrowed` demanded the array be "unaliased", which `ED_ParseEdict` structurally cannot satisfy — its `base` is `&ed->v`, inside that very array. Two changes. `borrowed`'s safety doc now states the requirement the arena actually has (it keeps a raw base pointer and re-derives every access, so a non-overlapping live reference into the array is fine), and the shim records the invariant that makes non-overlap hold: both callers raise `qcvm->num_edicts` above the edict they are about to parse *before* calling `ED_ParseEdict`, and the `ev_entity` arm's only arena writes are at indices `>= num_edicts`, hence strictly above the parsed edict. The reviewer is right that this was recorded nowhere.
5. **Two further undocumented divergences.** `parse.rs`'s `ev_entity` init loop drops C's `EDICT_NUM (j)` — whose release-build range test cannot fire here, but whose DEBUG header-consistency raise is not reproduced — now carries a `// COMPAT:` note. `OP_STATE`'s writes are now named in `VmRaw`'s accepted-divergence note alongside `OP_STOREP_*`/`OP_LOAD_*`, which is where the reviewer expected to find them.
6. **`ED_Write`'s glue introduced a raise on a path that had none.** `NUM_FOR_EDICT (ed)` → `NUM_FOR_EDICT_NO_CHECK (ed)`. C's `ED_Write` cannot `Host_Error`; the caller already holds a valid edict pointer, and the parse glue argues for the unchecked form on identical grounds.

**Nits, all fixed:** `builtin_is_null` deleted (dead code whose doc claimed it was used; the guarded-dispatch milestone can reintroduce it with a caller); `edict_size_for_test` renamed `edict_stride`, since it is on the production savegame path and the old name asserted otherwise; `check_capi_signatures.sh` now extracts `MIN_EDICTS`/`MAX_EDICTS` from `Quake/quakedef.h` instead of duplicating their values, so `freelist_t` is always sized with the engine's own constant; the `meson.build` comment no longer implies `pr_edict.c` flips — it never does, which is the point of splitting first.

**PR description corrected.** The claim that M2 is "ctest-only by design" stopped being true at M5, and the review is right that this matters: `ed_parse_epair`'s `ev_entity` arm drives `alloc::{ed_free, add_to_free_list, remove_from_free_list}` in the live engine against the same `qcvm->free_list` that C's still-compiled `ED_Alloc`/`ED_Free` manage. The mixed ownership is intended and the ABI probe covers `freelist_t`, but free-list ordering decides entity numbering, which is observable in savegames and on the wire. The structural note two sections above said the same thing and the PR body contradicted it; the body now states it explicitly.

**Not changed, with reasons:** nothing in the review was rejected outright. Item 2 was resolved by documentation rather than by the raise the wording leaned toward, because a raise would be behaviour C does not have — recorded above so the choice is reviewable rather than silent.

**Second CI iteration — an arch-gated clippy error local runs structurally cannot see.** The push fixing the above left `fmt + clippy + deny` red on a *different* error: `manual_range_contains` at `exec.rs:536`, inside `c_cast_i32`'s `#[cfg(target_arch = "x86_64")]` arm. That arm does not compile on this Apple Silicon host, so no amount of local `cargo clippy` would ever have linted it — the aarch64 arm compiles instead. **The check that closes this gap is `cargo clippy -p quake-progs --all-targets --locked --target x86_64-apple-darwin -- -D warnings`**: `quake-progs` has no `quake-c-sys` dependency (deliberately, so it stays fuzzable), so it cross-checks with nothing to link. Verified non-vacuous by mutation — restoring the old expression reproduces CI's exact error. The rewrite to `(-2147483648.0..2147483648.0).contains(&v)` is a De Morgan transformation of the original and was checked to be bit-identical over NaN, both infinities, both boundaries and 2000 pseudo-random bit patterns before being applied, because this function is the per-arch float→int UB emulation and its behaviour is the contract. `exec.rs` is the only `target_arch` gate in the Phase 6 crates; the other platform gates in the workspace are Phase 2/5 code that Linux CI already lints.

### 2026-08-27 M6 — the progs loader

Landed: `PR_LoadProgs`, `PR_MergeEngineFieldDefs`, `PR_ClearProgs`,
`PR_HasGlobal`/`PR_FindSupportedEffects` and `PR_PatchRereleaseBuiltins` are
Rust and **flipped**. `PR_SwitchQCVM` and the `qcvm`/`pr_global_struct`
storage stay C in the glue, per the flip-mechanism map.

1. **Fourth behaviour-neutral C split**: `Quake/pr_edict_load.c`, moved
   verbatim (lines 1019–1399 of `pr_edict.c`), swapped for
   `pr_edict_load_glue.c` under the switch. Neutrality verified before the
   flip: corpus `--check` 11/11 against the darwin-arm64 goldens with the
   split alone. `Quake/common.make` was updated in the same commit — the M5
   PR-review root cause was that the Makefile fallback's hand-maintained
   object list is independent of Meson and nothing cross-checks it.
2. **Second unsafe island (ADR-004 amendment): `quake_progs::image`.** The
   progs image is untyped C memory whose interior layout comes from the file
   header at runtime, the same shape of problem as the edict arena, so
   `ProgsImage`, `DefTable` and `VmLoad` hold the raw access and
   `quake_progs::load` stays `deny(unsafe_code)`. Every lump read is
   `read_unaligned`: C dereferences `(dstatement_t *)((byte *)progs + ofs)`
   directly, and the port must not add alignment UB on top of the bounds
   checks it adds.
3. **`VmLoad` is separate from `VmRaw` on purpose.** `VmRaw::new` asserts the
   lumps are loaded, which is precisely what is not yet true mid-load, and
   keeping the loader's setters off the execution path means nothing in
   `exec` can reach a setter that only makes sense during `PR_LoadProgs`.
4. **COMPAT, bug preserved deliberately: the `colormod_x/_y/_z` map keys are
   `va` pointers.** `PR_MergeEngineFieldDefs` inserts the three vector
   components into `fielddefs_map` keyed on `va ("%s_%c", ...)`, i.e. into one
   of `va`'s eight rotating THREAD_LOCAL buffers. `hash_map_t` stores the key
   *pointer* and dereferences it on lookup, so once eight more `va` calls have
   gone by, `ED_FindField ("colormod_x")` compares against whatever now
   occupies that buffer and misses. The port therefore calls the engine's own
   `va` through `LoadSys::va_component_name` rather than using a stable Rust
   string: the divergence is not in *what* is inserted but in *which storage*
   the key points at, and a stable key would make three fields findable that
   are not findable in C. Logged as a post-parity fix candidate.
5. **Accepted divergence: an out-of-range lump is refused.** C computes every
   lump base as `(byte *)progs + ofs` and walks `count` entries with no
   validation at all, so a malformed `progs.dat` — mod data — is an
   out-of-bounds read *and* an in-place byteswap write past the buffer. The
   port returns `LoadError::LumpOutOfRange`, which the glue raises as
   `"%s has a lump that runs past the end of the file"`. C's own
   strings-past-end check is evaluated **before** the added bounds pass so a
   truncated `progs.dat` still reports the message C reports.
6. **COMPAT: the non-fatal arms leak the file buffer, and that is preserved.**
   C sets `qcvm->progs = NULL` and returns without freeing what `COM_LoadFile`
   allocated. Freeing it would be the only behaviour a sanitizer run over the
   mixed build could see differ from the C oracle, so the leak stands.
7. **The hash maps are created through `LoadSys`, not `quake_util::hash_map`.**
   `ED_FindField`/`ED_FindGlobal`/`ED_FindFunction` stay in `pr_edict.c` and
   look these maps up from C, so the object C dereferences has to be the one
   C's `HashMap_Lookup` was written against. Same call-through argument as the
   injected `qsort` in M2 and the libc conversions in M5.
8. **Deviation from the milestone plan: there is no `progs_load_differential`
   C oracle.** `PR_LoadProgs` reaches `COM_LoadFile` (renamed to
   `c_ref_COM_LoadFile` by the differential prelude, so an oracle would need a
   staged gamedir per case) and `PR_EnableExtensions`/`PR_ShutdownExtensions`
   (in `pr_ext.c`, not an oracle file, so both sides would have to call a
   stub). What replaces it: `progs_load_synthetic.rs`, 16 Rust-side tests over
   synthetic images with a recording `LoadSys` mock — reverse map build order
   and first-match resolution, the reserve margin, the merge offsets and
   `edict_size`, the already-defined-field case, the `va`-keyed components,
   both version arms, all nine foreign-CRC arms, `DEF_SAVEGLOBAL`,
   strings-past-end, five out-of-range lumps, the re-release patch's
   exact-match rule, the effects mask in all four states, and both
   `PR_ClearProgs` ownership states — plus `fuzz_progs_load`. The reason this
   is proportionate rather than a gap: **trace parity is what compares the
   loader against C**, and it is byte-identical over id1 e1m1/e2m1/e3m1 and
   hipnotic/rogue/rerelease `start` (223k–489k records each), which cannot
   hold unless the globals block, both lumps and all three maps came out
   identical; `save_diff` covers `edict_size` and the merge. What the
   synthetic suite adds is the arms real data never reaches.
9. **`fuzz_ed_parse` landed too** — the gap the M1–M5 review named, where
   findings 1 and 3 of that review sat. Both new targets are in the CI fuzz
   loop with committed seeds (7 and 12).
10. **New CI gate: `cargo clippy -p quake-capi --features progs`.** The
    `progs` feature is off by default, so the workspace clippy pass had never
    type-checked `quake-capi::progs_*` at all. Verified non-vacuous the hard
    way: it immediately found four pre-existing lints in the M3–M5 shims (two
    no-op `drop()`s of types that do not implement `Drop`, an undocumented
    `unsafe` block, and an `unnecessary_lazy_evaluations`), all fixed. The two
    `drop()`s were standing in for "the raw views end before `Con_Printf`
    runs"; that is now a real scope block rather than a no-op call.
11. **Gates**: all five meson configs build; corpus `--check` 11/11 on
    `build-c`, `build-rs` and `build-rs-cprogs` plus `--compare` C-vs-mixed;
    `save_diff` byte-identical (91,562 / 91,547); trace parity byte-identical
    over six map/game combinations; 48 ctest suites green; `check_headers`,
    `check_capi_signatures`, `check_ctest_symbols` clean; bindgen regen-diff
    clean; `cargo fmt --check` and both clippy passes clean.
