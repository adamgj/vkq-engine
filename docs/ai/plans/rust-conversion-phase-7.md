# Rust migration Phase 7 — Client & server simulation (`quake-host`, `quake-cvar`, `quake-net` remainder) — agent-executed

## Context

Phases 0–6 are complete on `master` (`27a5fe89`); this branch is `feature/rust-conversion-phase-7-a3ee42`. Phase 7 (`docs/rust-migration/ROADMAP.md:181`) ports client & server simulation to Rust: ~52k LOC across 26 C files, plus the Phase 5 carry-ins (net_main.c dispatch funnels, net_dgrm.c orchestration half) and Phase 6 carve-outs (43 server-coupled pr_cmds builtins, pr_ext strzone/knownzone, the pr_edict_arena flip, ED_ParseEdict/ED_ParseGlobals dispatchers, CSQC load chain). It is the phase that closes four ADR-007 dual-view windows: `sv`/`svs`, `cl`/`cls`, `net_message`/`msg_readcount`/`msg_badread`, and `qcvm`/`pr_global_struct`/`qcvm->edicts`.

**What makes this plan different from Phases 5/6:** the user has authorized agent execution with parallelism. Discrete tasks are assigned to subagents with explicit model (Opus/Sonnet/Haiku) and effort levels; the main session (Fable, high effort) orchestrates, integrates, and is the sole writer of shared build files. Priority is accuracy/correctness/quality; cheaper models are used only for genuinely mechanical or read-only work, and only where a bit-exact differential gate exists to catch divergence.

**User decisions (recorded):**
- Agent-parallel execution authorized (the planning request). Parallelism is in *development* (isolated worktrees, ctest-only work); **landing stays strictly serialized** — one commit per milestone, tree green (all meson configs + gates) after each.
- Compat policy unchanged from Phases 5/6: bug-for-bug preservation, deletions recorded but deferred (C stays the oracle), `(compat exception)` ADRs followed exactly with `// COMPAT:` markers.
- **Golden platform coverage:** Phase 7 exits with darwin-arm64 (existing) + windows-x64 goldens (generated locally in M11 from the c-reference build, CI `--check` leg added). **Linux goldens are deferred to a later phase** — recorded as an amendment against the ROADMAP's "per platform" exit wording; Linux CI keeps `--compare`/`--stability` coverage meanwhile.
- **Soak budget: 100k server frames per cell × 8 cells** for the full soaks at M9 and M11. Smoke cells at every milestone from M4. If a desync ever reproduces only beyond 100k frames, extend that cell ad hoc.

Governing: ADR-003 (deps), ADR-004 (unsafe), ADR-005 (formatter; %g/%e panic — specifier audits required), ADR-006 (edict arena view→owner flip), ADR-007 (dual-view table updates are review-blocking), ADR-008 (ambient qcvm; `_Host_Frame` VM-switch ordering; no QcVmGuard until Phase 9), ADR-009 (Host_Guard/Host_Reraise; no longjmp through Rust frames; post-guard invariant; hot-path guard cost must be re-measured), ADR-010 (per-platform determinism; libm call-through; -ffp-contract=off project-wide; NaN-sign exception takedown when r_part_fte sim callers port), ADR-011 (FFI mirrors), ADR-016 (tasks.c stays C — hard non-goal), ADR-019 (gate-first verification; full-suite re-run at phase exit). Template: `docs/ai/plans/rust-conversion-phase-6.md` — one milestone per commit, tree green after each, amendment log.

## Milestone graph

```
M1 gates-first ─┬─▶ M2 cvar+cmd ─┬─▶ M3 world ─▶ M4 sv_move+sv_phys ─▶ M5 pr builtins+ED_Parse*  [server track]
                │                │        ══ STOP A: compat review M1–M5 ══
                │                └─▶ M7 client stratum (ctest/worktree dev ∥ M3–M6; LANDS after M6) [client track]
                │                    M6 sv_user + sv_main (after M5)
                └───────────────────▶ M8 host.c + host_cmd.c (after M6 and M7)
                                          ══ STOP B: compat review M6–M8 ══
                     M9 NET funnels + pr_ext/strzone + arena flip (after M8) ─┐
                     M10 UI stratum + r_part sim (dev ∥ M9, lands after) ─────┴─▶ M11 phase exit
```

Rationale for ordering (deviations from naive file order):
- **M2 before M3**: no hard dependency either way, but the physics cvar-matrix oracle should not have a registry flip landing mid-physics-port, and M2 proves the new Pattern A + meson-switch machinery on the cheapest subject (cvar.c has one global, `cvar_vars` at cvar.c:26).
- **Client track is independent of the server track** (needs only M1 gates + M2 registry; sits on the already-Rust MSG layer) — the biggest parallelism win. It develops concurrently with M3–M6 but lands after M6.
- **M9 (NET funnels) before M10 (UI)**: funnel flips become legal once M6/M7/M8 statusize the strata beneath; landing them right after M8 gives the phase-exit soak maximum bake time in the final NET configuration. UI doesn't affect the soak.

## Switch granularity (decision)

Two new meson switches: **`use_rust_cvar`** and **`use_rust_host`** (+ CI leg `build-rs-chost`). NET carry-ins ride the existing `use_rust_net`; pr_cmds/pr_ext/arena work rides `use_rust_progs` Pattern C. Rationale: matches ROADMAP crate boundaries (quake-cvar, quake-host, quake-net remainder); within-phase oracle granularity comes free (unported files remain C in both legs); a sv/cl switch split would double the meson/CI matrix to enable only mixed-language single-process configs the interop matrix already covers at process level. No dedicated `build-rs-ccvar` leg (small enough to bisect locally).

## Agent execution rules

- Main session (Fable, high): orchestration, all landing commits, sole writer of `meson.build`, `meson_options.txt`, `Quake/common.make`, `rust/quake-ctest/build.rs`, CI workflows. Parallel agents deliver source files + tests only (worktree isolation).
- Compat-critical porting → **Opus high**. Mechanical transliteration behind strong differential gates → **Sonnet medium/high**. Read-only discovery/audit/inventory → **Haiku low** (Sonnet low where judgment is needed). Existing project agents keep their roles: repo-researcher (Haiku low), verification-diagnostician (Sonnet medium), compatibility-reviewer (Opus high, fresh context) at STOP A/B and M11.
- Every port task's prompt includes: the exact C source range, the governing ADRs, the flip pattern to use, the differential-test contract, and "return status codes, never panic across FFI; no longjmp-reachable C callee without a trampoline."
- After two failed attempts on the same problem: stop, run a `/loop-breaker`-style read-only diagnosis (Sonnet/Opus), don't iterate blind.

## Milestones and tasks

### M1 — Gates first (verification infrastructure; no porting)

All tasks parallel except T1.8 (integration, last).

| # | Task | Model/effort |
|---|---|---|
| T1.1 | New longer server-exercising corpus entries (e1m1-long, pusher/elevator-heavy segments; registered-tier → local-only where needed) | Sonnet medium |
| T1.2 | `scripts/harness/physics_matrix.py`: cvar-matrix sweep over `--compare-extra-args` — `sv_fte_recursivehullckeck 0/1` × `sv_gameplayfix_elevators 0–3` × `sv_smoothplatformlerps 0/1` (12 cells; document CI vs local split) | Sonnet high |
| T1.3 | Soak driver: `interop_matrix.py --soak` — frame-count target, periodic server state-hash exchange, desync = hash mismatch vs same-seed C/C reference OR Host_Error/protocol error/drop; on failure auto-dump hash window + repro demo (see Soak definition below) | Sonnet high (design reviewed by main session) |
| T1.4 | **Extend the existing `Harness_HashClient` (`Quake/harness.c:236`)** to cover `cl.qcvm` globals when active — closes the CSQC hash gap; client sim vars are already hashed | Opus medium |
| T1.5 | ADR-011 ABI mirrors for `sv`/`svs`/`cl`/`cls`/`client_t`/`client_state_t` + inventory of big statics (`sv_pusher_support[MAX_EDICTS]` etc.) | Sonnet medium |
| T1.6 | Meson switches `use_rust_cvar`/`use_rust_host` (boilerplate per `meson.build:386-390`) + cargo features + `build-rs-chost` CI leg | Sonnet low |
| T1.7 | Three read-only audits, parallel: (a) ADR-005 specifier audit of host_cmd.c savegame writer + `Host_WriteConfiguration` + `Key_WriteBindings`; (b) cl_parse.c inventory of all 39 `Host_*` sites with per-svc context; (c) cvar/cmd external-linkage surface | Haiku low ×3 |
| T1.8 | Integration: C-vs-C self-compare of T1.2/T1.3/T1.4 **plus fault-injection red-test of each** (a detector that has never fired is not a gate); wire CI; open ADR-007 rows annotated with closing milestones | Main session |

- Gates: `physics_matrix.py` C-vs-C all cells green, injected fault red; `interop_matrix.py --soak --frames 20000` C-vs-C clean, injected fault detected at the right frame; `run_corpus.py --stability` over new entries; `check_headers.sh`; both new meson legs compile.

### M2 — cvar.c + cmd.c → quake-cvar (first flip)

- T2.1 (Sonnet medium, first): add cvar.c/cmd.c to quake-ctest `C_SOURCES` (+`common.make` same commit); differentials: registration order, `cvar_vars` walk order, completion, alias expansion, tokenizer edge cases, config-dump ordering.
- T2.2 (Opus medium) port cvar.c ∥ T2.3 (Sonnet high) port cmd.c — Pattern A + glue.
- T2.4 (main session): flip under `use_rust_cvar`; verify no double-registration (Rust code already registers through the C registry — direction inverts here).
- Gates: new ctest differentials; `run_corpus.py --compare build-c build-rs`; config.cfg byte-diff C-vs-Rust (writer still C, registry Rust); `check_ctest_symbols.sh`.

### M3 — world.c

- T3.1 (Opus medium, first): world.c into ctest; differentials for **both** hullchecks (`sv_fte_recursivehullckeck`, `world.c:33`), areanode insertion order, `SV_LinkEdict`/`SV_TouchLinks` ordering, `SV_Move` pipeline vs captured fixtures; hullcheck fuzz target.
- T3.2 (**Opus high**): port world.c — both hullcheck impls, areanodes, `SV_TouchLinks` re-entrancy into `PR_ExecuteProgram` with ADR-009 guard placement **and cost measurement** (guard economics were validated at ~13 calls/frame; this is per-link per-move).
- T3.3 (main session): flip under `use_rust_host`; physics matrix goes C-vs-Rust.
- Gates: ctest world differentials both hullcheck settings; `physics_matrix.py --compare` all 12 cells; corpus compare incl. server-hash entries; guard-overhead measurement recorded.

### M4 — sv_move.c + sv_phys.c

- T4.1 (Opus medium, first): ctest differentials — pusher modes 0–3, `sv_pusher_support` frame states, gravity half-step, `sv_analyticphysics`, checkbottom/movestep fixtures.
- T4.2 (Sonnet high) port sv_move.c ∥ T4.3 (**Opus high**) port sv_phys.c (pusher-support-frame subsystem + integrator; float-operation-order exact per ADR-010; 12 cvars; `sv_pusher_support[MAX_EDICTS]` ownership per T1.5).
- T4.4 (main session): flip; full physics matrix.
- Gates: physics matrix all cells C-vs-Rust; long-entry server-hash parity; `--stability`; soak smoke cell.

### M5 — server-coupled pr_cmds builtins + ED_Parse* dispatchers

- T5.1: the 43 builtins via existing Pattern C (`PF_RS`/`RUST_PF`), in parallel groups: link group setorigin/setsize/setmodel (**Opus high** — SV_LinkEdict→SV_TouchLinks→PR_ExecuteProgram re-entrancy) ∥ trace/movement group traceline/tracebox/checkpos/walkmove/droptofloor/checkbottom/pointcontents/aim/findradius (**Opus high**) ∥ PVS group checkclient + `checkpvs` global (**Opus high**) ∥ message group 8× PF_Write*, stuffcmd/bprint/sprint/centerprint (Sonnet high) ∥ world-effects group sound/particle/lightstyle/makestatic/ambientsound/changelevel/setspawnparms/precaches (Sonnet high) ∥ `PF_cl_*` set (Sonnet high). `PF_break` stays C permanently.
- T5.2 (Sonnet high, ∥): ED_ParseEdict/ED_ParseGlobals key dispatchers (ctest infra exists from Phase 6); pr_edict.c residue onto the deletion list. Needed before M6 (`SV_SpawnServer`) and M8 (`Host_Loadgame_f`).
- T5.3 (main session): integrate; re-baseline `builtin_diff.py` / `trace_diff.py` floors.
- Gates: `builtin_diff.py`; `trace_diff.py` (min-records floor); `save_diff.py`; full corpus compare; interop smoke.

**══ STOP A — fresh-context compatibility review (compatibility-reviewer, Opus high) of M1–M5 ══**

### M6 — sv_user.c + sv_main.c

- T6.1 (Sonnet medium, first): verbatim behaviour-neutral C split of sv_main.c into send half (FTE delta writer, `SVFTE_*`, baselines, `MSG_WriteStaticOrBaseLine`, PVS culling, `SV_WriteEntitiesToClient`) and connection half (`SV_ConnectClient`, `SV_SpawnServer`, protocol negotiation, sv/svs storage); corpus-verified neutral; `common.make` same commit.
- T6.2 (Opus medium): ctest differentials for the delta writer vs record fixtures.
- T6.3 (**Opus high**) port send half ∥ T6.4 (Sonnet high) port sv_user.c (libm trig call-through, ADR-010) ∥ T6.5 (**Opus high**) port connection half with **statusized** `SV_ConnectClient` (the ADR-009 unlock for M9).
- T6.6 (main session): flip; sv/svs storage → Rust; **close ADR-007 sv/svs row**.
- Gates: `record_diff.py` + `netreplay_diff.py` C-vs-Rust; `interop_matrix.py` full 6×4; physics matrix re-run; soak smoke.

### M7 — client stratum (dev ∥ M3–M6; lands after M6)

- T7.1 (Opus medium): ctest differentials — cl_parse svc dispatch fixtures from recorded server streams, lerp/relink, tent tables.
- T7.2 (parallel leaf ports against the still-C cl/cls dual-view, flipped individually): view.c+chase.c (Sonnet high) ∥ cl_input.c (Sonnet medium) ∥ cl_tent.c (Sonnet medium).
- T7.3 (**Opus high**): cl_parse.c — svc dispatch, statusization of all 39 `Host_*` sites (input: T1.7b inventory), CSQC load chain. The phase's largest ADR-009 surface.
- T7.4 (Opus high): cl_main.c (lerp/relink, cl/cls storage) + cl_demo.c playback logic; **close ADR-007 cl/cls row**.
- Gates: client-hash parity (incl. the new cl.qcvm coverage) on client corpus entries; `capture_diff.py`/`record_diff.py`; demo-playback corpus compare; interop full matrix.

### M8 — host.c + host_cmd.c

- T8.1 (Opus medium): ctest differentials — `Host_FilterTime` accumulator, savegame writer/reader byte fixtures.
- T8.2 (**main session, Fable high**): host.c orchestration behind the C setjmp shell (`Host_Guard` host.c:302 / `Host_Reraise` host.c:339 stay C until Phase 9; `_Host_Frame` VM-switch ordering per ADR-008). The ADR-008/009 center of the phase.
- T8.3: host_cmd.c — `Host_Savegame_f`:1554/`Host_Loadgame_f`:1797 (**Opus high**; byte-diff subject; T1.7a audit as input) ∥ game commands (Sonnet high) ∥ `ExtraMaps_*` background-thread boundary (Opus medium — thread stays C or the Rust side is Send-audited).
- T8.4 (Sonnet high): `Host_WriteConfiguration`:486 config.cfg writer via the ADR-005 formatter (Key_WriteBindings still C through capi until M10; end-to-end config byte-diff completes there).
- Gates: `save_diff.py` byte-clean in all three directions (C-write/Rust-read, Rust-write/C-read, Rust/Rust); config.cfg byte-diff; full corpus; soak smoke.

**══ STOP B — fresh-context compatibility review (Opus high) of M6–M8 ══**

### M9 — NET funnel flips + pr_ext/arena (after M8; dev ∥ M10)

- T9.1 (Sonnet high): flip the net_main.c Pattern B funnels (NET_Connect:548, NET_GetMessage:703, NET_GetServerMessage:761, NET_Send*:814/840, NET_CanSendMessage:874, NET_SendToAll:890, NET_Init:974/Shutdown:1054, NET_Poll:1078/SchedulePollProcedure:1093, slist trio) — legal now that all strata beneath are statusized.
- T9.2 (**Opus high**): net_dgrm.c orchestration half (`_Datagram_ServerControlPacket`:689 → Rust SV_ConnectClient, hostcache/slist, heartbeats, rcon).
- T9.3 (Opus medium): pr_ext strzone/strunzone + knownzone with the **pr_edict_arena flip** (`qcvm->edicts` Rust-owned; ADR-006 view→owner); remaining pr_ext Phase-7 builtins; **close ADR-007 qcvm/pr_global_struct/edicts row and the net_message/msg_readcount/msg_badread row** (storage leaves net_msg_glue.c:38).
- Gates: `netreplay_diff.py`; interop full matrix; **first full-length soak in final NET config**; **save byte-diff suite re-run post-arena-flip** (M8's verification predates the flip).

### M10 — UI stratum + particle sim (dev ∥ M9; lands after)

- T10.1 (**Opus high**) console.c — `con_mutex`, non-leaf `Con_Printf` (reaches SCR_UpdateScreen; capi shim + re-entrancy story) ∥ T10.2 (Sonnet high) keys.c incl. `Key_WriteBindings` (completes config byte-diff end-to-end) ∥ T10.3 (Sonnet medium) sbar.c (98 Draw_* capi shims) ∥ T10.4 (Sonnet high, may split by menu groups) menu.c (4950 LOC, 108 Draw_* shims; drawing stays shimmed until Phase 8).
- T10.5 (Opus medium): r_part.c + r_part_fte.c **simulation halves**; **ADR-010 NaN-sign exception takedown** (PerpendicularVector/RotatePointAroundVector sim callers port; ADR amendment removes the fence).
- Gates: `capture_diff.py` on console/menu/sbar-exercising entries; config.cfg byte-diff fully Rust; particle-sim state in capture/record diffs; ADR-010 amendment landed.

### M11 — Phase exit

- Full netplay soak (100k × 8 cells); **full ADR-019 suite re-run** (corpus, physics matrix, interop, all byte-diffs, fuzz soak, Miri on new unsafe islands); **windows-x64 golden generation** from the c-reference build + CI `--check` leg (Linux deferred per recorded decision, ROADMAP amendment noted); fresh-context `/integration-review` (Opus high); ROADMAP status + deletion lists recorded (deletion itself stays deferred — C remains the oracle); ADR-007 table shows zero open Phase-7 rows; amendment log finalized.

## Soak definition ("hours-long netplay soak")

Local-only gate (registered-tier precedent), via `interop_matrix.py --soak`:
- 4 build combos (C/C, C/R, R/C, R/R) × 2 protocols (666, FTE+999) = 8 cells; dedicated server + scripted deterministic client input; faster-than-realtime where the fixed timestep allows.
- Duration is frame-count based: **100k server frames/cell** (user decision; extend a cell ad hoc if a failure needs longer to reproduce). CI keeps existing short cells + one 20k-frame nightly smoke.
- Pass = zero desyncs; desync = periodic (every 64 frames) server state-hash mismatch vs the same-seed C/C reference stream, or any Host_Error/protocol error/drop. **Never byte identity** (live timing is nondeterministic — Phase 5 finding).
- On failure: auto-dump last-N hash window + server demo + frame index, sufficient for offline replay.
- Proven by fault injection in M1; smoke cell in every Gates line from M4; full runs at M9 and M11, results recorded in the amendment log (CI cannot attest them).

## Verified ground truth (landmines)

1. `world.c:33` — two hullcheck impls behind `sv_fte_recursivehullckeck` (engine's misspelling); nothing in current tooling toggles it; both port, both in the M1 matrix.
2. `harness.c:195` — `Harness_HashServer` returns early unless `sv.active && vm->progs`; only `save-e1m1` + `map-e1m2` exercise it in CI today; demos start no server (zero progs trace records — Phase 6 finding).
3. `Harness_HashClient` exists (`harness.c:236`) and covers client sim vars — but **cl.qcvm/CSQC globals are not hashed**; T1.4 closes exactly that gap, not more.
4. host_cmd.c `ExtraMaps_*` runs a background parsing thread — port around the thread boundary, not through it (ADR-016: no Rust thread-local assumptions on C workers).
5. cl_parse.c has 39 `Host_*` sites — the phase's largest ADR-009 statusization surface; per-svc status returns; post-guard invariant per message.
6. ADR-009 guard cost validated only at ~13 calls/frame; `SV_TouchLinks`→`PR_ExecuteProgram` is per-link per-move — measure before the M3 flip.
7. setorigin/setsize/setmodel → `SV_LinkEdict` → `SV_TouchLinks` → `PR_ExecuteProgram` → possibly the same builtins again: Rust world must tolerate recursive VM entry; ADR-006's tightened rule ("no reference lives across a builtin dispatch") exists for exactly this path; Miri model it.
8. sv_phys.c: `sv_pusher_support[MAX_EDICTS]` static, 12 cvars, pusher modes 0–3, gravity half-step, `sv_analyticphysics` — float-operation-order exact (ADR-010), no reassociation.
9. sv_user.c trig = libm call-through per ADR-010 (`quake_c_sys::libm`, `// COMPAT: ADR-010` per site), never `f32::` methods.
10. `Con_Printf` is not a leaf — reaches `SCR_UpdateScreen`, holds `con_mutex`; console flip needs a capi shim and a re-entrancy story.
11. menu.c (108) + sbar.c (98) `Draw_*` calls — renderer stays C until Phase 8; the shim surface is a scoped deliverable, deliberately throwaway.
12. ADR-010's NaN-sign exception (PerpendicularVector/RotatePointAroundVector) is fenced only because all callers are in r_part_fte.c — M10 must take it down with an ADR amendment.
13. ADR-007 rows close at specific milestones: sv/svs (M6), cl/cls (M7), net_message trio (M9), qcvm/pr_global_struct/edicts (M9). "An unlisted dual view is a review error."
14. host.c setjmp shell (`Host_Guard`:302/`Host_Reraise`:339) stays C until Phase 9; `_Host_Frame` VM-switch ordering (ADR-008) and `Host_FilterTime` are behavior-bearing.
15. `Host_WriteConfiguration`:486 + `Host_Savegame_f`:1554/`Host_Loadgame_f`:1797 are byte-diff subjects; the ADR-005 specifier audit (T1.7a) is a prerequisite (%g/%e panic if reached).
16. quake-ctest `C_SOURCES` has zero Phase 7 files; every differential requires `c_ref_*` additions, and `Quake/common.make` must change in the same commit as any C split (real MinGW link failure precedent).
17. The cvar flip inverts registration direction (Rust currently registers into the C registry); double-registration and callback-direction bugs are the M2 failure mode. `cmd_text` already sits on Rust SZ buffers.
18. NET funnel flips are illegal until M6 (SV_ConnectClient), M7 (cl_parse), and M8 (host) statusize the strata beneath — M9 must not be pulled earlier.
19. Live captures are timing-nondeterministic — soak pass criterion is state-hash desync detection, never byte identity.
20. Goldens exist for darwin-arm64 only; the dev machine is Windows, so local gates are `--compare`/`--stability` until the M11 windows-x64 goldens land; Linux goldens deferred (recorded user decision).
21. `PF_changeyaw` precedent (`pr_cmds.c:1573`): a builtin with a direct non-table caller flips in place via `#ifdef`, not slot-only — applies to any M5 builtin `sv_move.c`/others call directly.

## Risks

| Risk | Mitigation |
|---|---|
| Physics float divergence invisible to short corpus entries | M1 long entries + 12-cell matrix + fault-injection-proven detectors before any port |
| Missed Host_Error path longjmps through a Rust frame (UB) | T1.7b inventory; per-svc statusization; STOP A/B reviews check reachability |
| Delta-writer mismatch only visible in live netplay | record/netreplay diffs at M6; soak smoke every milestone from M4; full soak M9/M11 |
| Guard overhead in SV_TouchLinks/SV_Move hot paths | Measure at M3 pre-flip; batch guards at funnel level if needed |
| Parallel worktree conflicts on build files | Main session sole writer of meson/common.make/build.rs/CI; agents deliver source+tests |
| cvar registry flip silently breaks Rust-registered cvars | M2 registration-order differential + double-registration assert |
| Savegame byte-diff blocked by specifier drift | T1.7a ADR-005 audit first; fixtures before port |
| Arena flip invalidates M8 savegame verification | Save suite re-run in M9 Gates line |
| Soak flakiness erodes trust | Deterministic scripted input; hash-based criterion; auto-captured repro artifacts |
| Sonnet-tier transliteration introduces subtle divergence | Only used where a bit-exact differential gate exists; Opus/main-session review at landing; STOP reviews |

## Non-goals

Renderer halves of r_part*/menu/sbar/console drawing internals (Phase 8); `tasks.c` (ADR-016, Phase 8); setjmp shell removal and `QcVmGuard` (Phase 9); C deletions (recorded, deferred — C stays the oracle); CSQC feature work beyond load-chain parity; performance work beyond guard-overhead neutrality; net_dgrm_rel.c/net_loop.c/wire layer/demo format (already Rust); `net_wins.c`/`net_bsd.c`/`net_win.c` (Phase 9); gameplay behavior changes; `PF_break` (C forever); `PF_Fixme`/extension machinery (follows the builtins, stays C this phase unless the last slot flips).

## Verification summary (three tiers)

- **Inner loop (per task):** `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, the task's ctest differentials (release + debug), `check_ctest_symbols.sh` when C_SOURCES changed.
- **Milestone boundary:** full ctest suite; `run_corpus.py --compare build-c build-rs` (+ `build-rs-chost` oracle); `save_diff.py`; `physics_matrix.py` (from M3); `builtin_diff.py`/`trace_diff.py` (progs-touching milestones); interop/soak smoke (from M4); `check_headers.sh`/`check_capi_signatures.sh`; all meson configs build.
- **Phase exit (M11):** full ADR-019 suite; full soak (100k × 8 cells); fuzz soak; Miri; fresh-context `/integration-review`; windows-x64 goldens + CI leg, Linux deferred.

## Amendment log

- **2026-08-28 plan committed** (pre-M1). Planning was agent-executed: three parallel exploration agents (code inventory, conventions/ADRs, harness/CI), one Plan agent for milestone design, cross-checked by the main session. One Plan-agent error caught in review and corrected before approval: T1.4 originally proposed *creating* a client state-hash; `Harness_HashClient` already exists (`harness.c:236`), so T1.4 targets only the cl.qcvm/CSQC gap. User decisions recorded above: Linux goldens deferred; soak budget 100k frames/cell.
- **2026-08-29 M1 gate evidence (all measured, Windows x64, this checkout).**
  - `check_headers.sh`: 12/12 headers compile standalone, 12/12 bindgen ok, exit 0.
  - Both new meson legs configure and build clean: `-Duse_rust_cvar=enabled` (146/146) and `-Duse_rust_host=enabled` (345/345).
  - `run_corpus.py --compare` (C-only vs mixed, shareware tier): **8 ran, 0 skipped, 0 failed**, every entry `identical` — including the three new pusher entries `e1m1-long`, `e1m5-trains`, `e1m1-plat-crush`.
  - `run_corpus.py --stability` (shareware tier): **8 ran, 0 failed**, all `stable`.
  - `physics_matrix.py` 12-cell CI trim × 7 entries (`save-e1m1`, `map-e1m2`, `e1m1-long`, `e1m5-trains`, `e1m1-plat-crush`, `e3m6-trains`, `save-e2m1`), C-vs-C: **84 ok, 0 skipped, 0 failed** (re-measured 2026-08-29 after the review pass; the first M1 measurement read *72 ok, 12 skipped* because it predated `e1m1-plat-crush` landing and still selected `gamedir-switch`, which now opts out via `"physics": false`); red-tested on the elevators axis via `e1m1-plat-crush` (hash chains fork at `F 56` between `sv_gameplayfix_elevators` 0 and 3, same-cvar rerun byte-identical). Lerps axis still vacuous per the KNOWN GAP above.
  - `interop_matrix.py --soak --frames 20000 --combos C/C`, with the C/C short-circuit removed so this is a genuine two-process comparison: **PASS both protocols** — `Base-666` 313 checkpoints reached, traffic in band, hash-identical; `FTE+999` 313 checkpoints reached, traffic in band, `hash forks @128` reported as the expected diagnostic. Reference streams: rel 3/4 unrel 19910/19986 (Base-666) and rel 4/5 unrel 19904/19983 (FTE+999). Red-tested separately via `--inject-desync-at` (`net_messagetimeout 0`), which fails both protocols on the reliable-traffic condition.
  - T1.4's `cl.qcvm` hash branch red-tested by two-stage fault injection: an injected build that loads id1 `progs.dat` into `cl.qcvm`, compared against a control build carrying the *same* injection with only the T1.4 branch disabled — **all 8 entries differ**, so the new hash line is the sole cause. Dormant on id1 data (no csprogs), hence injection rather than a corpus entry.
  - `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings`: clean. `clang-format` clean on all four touched C files.
  - Not run in M1 (out of scope, no Phase 7 code ported yet): `save_diff.py`, `builtin_diff.py`, `trace_diff.py`, `record_diff.py`, `netreplay_diff.py`, Miri, the full 100k×8 soak (M9/M11), and any Linux/macOS leg — CI covers the last on push.
- **2026-08-29 PR #26 review pass.** Findings triaged individually rather than accepted wholesale; accepted ones fixed in a follow-up commit on the same branch.
  - *Accepted, code:* `Quake/sys_sdl_win.c` now keys the console-input suppression on the actual failing call (`GetNumberOfConsoleInputEvents` returning 0 degrades to "no console input, ever", printed once) instead of on `harness_active` — the old predicate was a proxy for the cause and also silenced console input for a legitimate `-dedicated -harnesscmds` operator. `Quake/net_wins.c`'s `WSAECONNRESET` diagnostic no longer prints an address: `recvfrom()` returned `SOCKET_ERROR`, so `addr` may still hold the caller's previous sender. `Quake/main_sdl.c`'s two near-identical pacing branches deduped. `Quake/harness.c` gained `_Static_assert`s tying the seven constants `rust/quake-ctest/stubs/abi_probe.c` hardcodes to the real headers, and `Quake/render.h` gained the file's existing "must be changed in … too" markers on the three shadowed structs.
  - *Accepted, harness:* `physics_matrix.py` keeps both hash chains on failure (`--results-dir`) and reports the first diverging record line instead of only "DIFFERS"; corpus entries can opt out of the physics selector with `"physics": false` (used by `gamedir-switch`); `interop_matrix.py` gained a minimum-checkpoint floor on the reference stream, frame-derived client/server timeouts, and three corrected docstring claims (a dedicated server *is* fixed-dt under `-demohash`; a missing checkpoint means the server died, not that the client was dropped; both processes are now paced).
  - *Accepted, docs:* the CI soak smoke drops from 20000 to 5000 frames (~30 min serial at 20k because both processes pace to real time; both bugs it exists to catch surfaced by ~frame 128 — 20000 stays the local budget); `meson.build` and `build-linux.yml` now say plainly that `USE_RUST_CVAR`/`USE_RUST_HOST` and the `build-rs-chost` leg have no consumer until M2 and so cannot fail yet; `Misc/harness/README.md` records that the three new pusher entries have **no goldens on any platform**, so `--check` skips them, and carries the `e1m1-plat-crush` derivation the corpus note used to hold; `abi_probe.c`'s "not circular" claim replaced with an explicit KNOWN GAP (the `entity_t`/`entlerp_t`/`lightcache_t` shadow is transcribed, not header-derived, and only the constants are gated).
  - *Rejected:* "`run_corpus.py --check` silently skips missing goldens" — it already prints `skipped <name>: no golden for <plat>` per entry plus a summary count (`run_corpus.py:219-221`); the real gap was that this was undocumented, which the README change fixes. The remaining review asks about darwin-arm64 rerelease/mod-tier goldens cannot be answered from this machine (Windows, no macOS host) and stay open.
  - *Re-verified after the fixes:* full C build via meson/ninja (so the new `_Static_assert`s and all four C edits compile); `physics_matrix.py` C-vs-C **84 ok, 0 skipped, 0 failed**; `clang-format` clean on every touched C file; `python -m py_compile` clean on both touched scripts; `corpus.json` parses. Not re-run (unchanged inputs): the soak, corpus `--compare`/`--stability`, `check_headers.sh`.
- **2026-08-29 M2 landed** (`use_rust_cvar` first flip: cvar.c + cmd.c → `quake-cvar` helpers + `quake-capi` cores + `Quake/cvar_cmd_glue.c`).
  - *Task-shape amendment:* T2.2 (cvar.c) and T2.3 (cmd.c) were executed as **one Opus-high port agent**, not two parallel agents — the two files share the raise topology, the glue file, the cbindgen exclusions, and the `cvar_vars`/`cmd_functions` registry interlock (`Cvar_Create` checks `Cmd_Exists`; `Cmd_AddCommand2` checks `Cvar_FindVar`); splitting them would have serialized on a shared contract anyway. The ctest differential suite (T2.1) ran as a second parallel agent against a frozen contract (`m2-contract.md`).
  - *Architecture as landed:* ADR-009 raise topology — glue C owns the reraising ABI wrappers (`Cbuf_Execute`, `Cmd_ExecuteString`, `Cvar_Set*`, …) over `quake_rs_*` status cores; Rust-internal calls use cores only; Rust `xcommand_t` handlers propagate raises via a `PENDING_RAISE` side-channel drained after guarded dispatch. Glue C also owns `cmd_text`/`cmd_source`/`cmd_functions`/`cmd_alias`/`cl_nopext`/`cmd_warncmd` storage and the serverinfo/userinfo replication blocks (incl. the `&cl_name`/`&cl_topcolor`/`&cl_bottomcolor` pointer-identity hacks) so no ADR-011 svs/cls mirrors were needed in M2; those port with M6/M7. `cvar_vars` is Rust-private.
  - *Contract deviations (recorded, accepted):* (1) `Cbuf_InsertText` and `Cmd_ForwardToServer` are glue wrappers, not direct Rust exports — `SZ_GetSpace` (net_msg.c:481) can Host_Error on overflow, so both are raise-capable. (2) `Cvar_Set_f` returns on NULL from `Cvar_Create` instead of C's NULL-deref; `CvarCmd_Glue_HostClientName` returns `""` for NULL `host_client` — both replace C crash paths, unreachable in normal operation. (3) The port agent edited `rust/quake-capi/cbindgen.toml` (hand decls + ~40 excludes) — needed for `check_capi_signatures.sh`, no other owner existed.
  - *Pre-existing exposure recorded (not fixed in M2):* Rust modules from earlier phases call the now-reraising wrappers via `quake-c-sys` (`bgmusic.rs:137,523,525`; `cfgfile.rs:154,217`; `fs.rs:1323-1324`; `net_main.rs:317,325` — `c::Cvar_Set`/`c::Cvar_SetQuick`/`c::Cvar_RegisterVariable`/`c::Cbuf_AddText`). A caught raise would longjmp through those Rust frames — but this exposure predates M2 (the same calls previously hit the C functions with identical longjmp behavior). Later milestones statusize those strata; no new exposure added.
  - *Drive-by fix (required to build):* `Quake/pr_cmds.c:41` and `Quake/pr_ext.c:261` carried unguarded `#pragma GCC diagnostic` from Phase 6 commit 0911af5a → C4068 under MSVC `/W2 /WX`. Invisible until now because only clang-cl/MinGW CI ever compiled the `use_rust_progs` leg; the M2 local MSVC `build-rs` config exposed it. Wrapped in `#if defined(__GNUC__)` per the `gl_texmgr.c` pattern.
  - *Gate evidence (all measured, Windows x64, this checkout):*
    - All three meson configs build clean: `build-rs` (flip live), `build-rs-chost` (C oracle leg), `build-c`.
    - `check_capi_signatures.sh`: OK.
    - `run_corpus.py --compare build-c build-rs` (registered tier): **11 ran, 8 skipped (missing mod/rerelease data — normal for this machine), 0 failed**, every entry `identical`.
    - Config byte-diff (writer still C, registry Rust): `+seta _m2test 3.14 +set gamma 0.6 +quit` headless run of both builds → `id1/vkQuake.cfg` **byte-identical** (104 lines; `seta` archived, plain `set` correctly not). Side finding: with shareware data `COM_CheckRegistered` never sets the `cmdline` cvar so `stuffcmds`/`+`-commands are inert — stock quirk, needed registered PAKs for this gate.
    - ctest differential suite: new `cvar_cmd_differential.rs` (20 tests: registration order/dup/collision, FindVarAfter flag filter, completion, alias define/redefine/unalias/expansion, 4 tokenizer edge-case sets, Cbuf ordering, WriteVariables byte-compare, SetQuick ROM/LOCKED/CHANGED/default_string semantics, SetValue formatting, Cvar_Create matrix, ExecuteString srctype filter, CheckParm) — **20/20 pass debug AND release**, deterministic across 3 runs; full quake-ctest suite green including `cfgfile_differential`, which the flip broke (its old plain-named Cvar_Set capture stubs had to go) and which was re-seamed via a callback-based capture (`ctest_register_logged_cvar_pair` registers the tested cvars into BOTH registries with a change-logging callback — Cvar_SetQuick's no-change early return makes only real changes log, identically on both sides). Documented gap: the ADR-009 raise/longjmp paths are not differentially tested (see module doc).
    - `check_ctest_symbols.sh`: OK — 30 oracle sources (now incl. cvar.c, cmd.c) export only `c_ref_*` and allowlisted symbols.
    - `cargo fmt --check` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean (post-agent fixes: 23 `// SAFETY:` comments per repo convention + 3 misc lints in the new test file; no behavior change, tests re-run green after).
    - `clang-format` clean on `cvar_cmd_glue.c`, `pr_cmds.c`, `pr_ext.c`.
    - Not run (out of scope for M2): `save_diff.py` (savegame writer untouched), `physics_matrix.py` C-vs-Rust (starts M3), `builtin_diff.py`/`trace_diff.py` (no progs change), interop/soak (starts M4), Linux/macOS legs (CI on push).
- **2026-08-29 PR #28 review pass (M2).** Six findings, triaged individually; five accepted, one accepted with its scope narrowed.
  - *Accepted, blocking, code:* `cvar_inc_f` (`rust/quake-capi/src/cvar.rs`) narrowed to `c_float` **before** the `+ 1`, so `inc` rounded twice where C rounds once. C is `Cvar_SetValue (name, Cvar_VariableValue (name) + 1)` with `double Cvar_VariableValue` (`Quake/cvar.c:83-98`, `Quake/cvar.h:128`), i.e. a `f64` add and one narrowing at the call. Divergence is real above f32 integer precision: `"16777217"` + 1 gives `16777218` in C and `16777216` (unchanged) two-step. Fixed and marked `// COMPAT: ADR-010`.
  - *Accepted, docs:* the `Cbuf_InsertText`/`Cmd_ForwardToServer` comment in `Quake/cvar_cmd_glue.c` claimed the glue wrapper is the `"cmd"` `xcommand_t`; it is not — `Cmd_Init` registers the private Rust `cmd_forward_to_server_handler`, which parks its raise in `PENDING_RAISE`. Comment corrected.
  - *Accepted, docs:* `PENDING_RAISE` (`rust/quake-capi/src/cmd.rs`) gained an explicit INVARIANT paragraph naming `cmd_execute_string_core` as the sole drain site, the public `Cmd_FindCommand` as the pointer-leak vector, and misattribution as the failure mode. The dispatcher now also *clears* any stale value before each guarded handler call (`debug_assert_eq!` in debug), so a leak cannot be re-issued as a `Host_Error` blamed on the next command in release builds.
  - *Accepted, code + docs:* `cmd_stuffcmds_f`'s `j` is unbounded exactly as in C; the no-overflow argument rests entirely on `Quake/common.c:1396` clamping `com_cmdline` to `CMDLINE_LENGTH - 1`. Recorded as a `// COMPAT:` note, and `Quake/harness.c` now `_Static_assert`s `CMDLINE_LENGTH == 256` (copied into `cmd.rs` *and* `c_ref_prelude.h`) and `sizeof (CONFIG_NAME) == sizeof ("vkQuake.cfg")`. Only the length of `CONFIG_NAME` is checked: a compile-time string compare needs `__builtin_strcmp`, which MSVC `cl` lacks; the spelling is covered by the config byte-diff gate, which reads the file back by name.
  - *Accepted, test, scope narrowed:* the review said `cfgfile_differential`'s capture cannot see sets to unregistered names **and** that its second `CFG_ReadCvars` pass is invisible. The first half is largely theoretical — `CFG_ReadCvarOverrides` iterates the caller-supplied `vars` array and cannot set a name outside it. The second half is real and was fixed: pass 2 re-set identical values, so `Cvar_SetQuick`'s no-change early return logged nothing. `stubs.c` gained `ctest_reset_logged_cvars(side)` (side-selective, so the shared log stays symmetric), called between the passes, plus an assertion that a `vid_width` set actually appears after the sentinel writes. **Red-tested:** stubbing out the Rust `FS_rewind` now fails the differential; before the fix it passed.
  - *Accepted, test, blocking:* `cvar_cmd_differential.rs` covered zero console-command handlers. Added `console_command_handlers_match` — 48 command lines fed to both sides through `Cbuf_AddText`/`Cbuf_Execute`, comparing the captured `ctest_con_log` stream **and** the resulting cvar state after every line: `inc` (usage/unit/amount/missing), `toggle` (usage/missing/numeric/explicit pair), `cycle` (usage/no-match/mid-list/last), `set`/`seta` (usage/extra-args/create/archive), `cvarlist`/`cmdlist`/`apropos` (prefix-scoped), `echo`, `alias`/`unalias`/`unaliasall` through the buffer, `exec` (missing file/usage), `stuffcmds` (driven off a real `cmdline` string), `wait`, `reset`/`resetcfg`/`resetall`. **Red-tested:** reintroducing the `inc` double-narrowing makes it fail with `left: 16777218 / right: 16777216`. Incidental finding while writing it: `Cbuf_Execute` never clears `cmd_wait` (host.c does, via `Cbuf_Waited`), so a test that issues `wait` must emulate a frame boundary or every later `Cbuf_Execute` in the process is a no-op.
  - *Re-verified after the fixes:* `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings` clean; full `cargo test --workspace` green **debug and release** (21/21 in `cvar_cmd_differential`, 1/1 in `cfgfile_differential`); all three meson configs (`build-c`, `build-rs`, `build-rs-chost`) build under MSVC/clang-cl, so the new `_Static_assert`s compile; `run_corpus.py --compare build-c build-rs` **11 ran, 8 skipped, 0 failed**, every entry `identical`; `check_ctest_symbols.sh` OK (30 oracle sources); `check_capi_signatures.sh` OK; `check_headers.sh` 12/12 compile + 12/12 bindgen. Not re-run (inputs unchanged): the config byte-diff (no writer or `Cvar_WriteVariables` change; the ctest byte-compare covers the writer), physics/interop/soak (start at M3/M4), Linux/macOS legs (CI on push).
- **2026-08-30 M3 landed** (`use_rust_host` first flip: world.c -> `rust/quake-capi/src/world.rs` + `Quake/world_glue.c`).
  - *Contract amendment (the milestone's one substantive finding).* The frozen M3 contract asserted **exactly one** raise-capable entry point, `SV_LinkEdict`. That is wrong. `PR_GetString` reaches `Host_Error` (`Quake/pr_edict_arena.c:315`), so the two `Con_Warning` sites in `SV_HullForEntity` (`world.c:145,152`) are raise-capable, as is `assert_always` at `SV_Move`'s tail (`world.c:1306`); by call chain that makes `SV_ClipMoveToEntity` (`world.c:928` -> `942`), `SV_Move` (`world.c:1260`), `SV_TestEntityPosition` (`world.c:605`) and `SV_PointContentsAllBsps` (`world.c:588`) raise-capable too. **Six** entry points are therefore ADR-009 status cores (`quake_rs_sv_link_edict`, `..._hull_for_entity`, `..._clip_move_to_entity`, `..._move`, `..._test_entity_position`, `..._point_contents_all_bsps`) wrapped by plain-named C in `world_glue.c`. The port agent initially proposed calling `Host_Reraise` from the Rust frame of `SV_PointContentsAllBsps` (arguing the frame holds only plain data with no `Drop`, under `panic = "abort"`); **rejected** -- the longjmp would still unwind a Rust frame, which ADR-009 prohibits structurally, with no exception for frames that happen to be trivially destructible. The `Host_Reraise` declaration was removed from `quake-c-sys` so the rule is enforced by the absence of the symbol: `grep` finds no `Host_Reraise` declaration or call site anywhere under `rust/`.
  - *Architecture as landed:* 18 `#[no_mangle]` exports from `world.rs` (2139 lines, 29 `// COMPAT:` markers) -- 12 plain non-raising names plus the six cores. Both hull-check implementations ported bit-exactly (FTE `Q1BSP_RecursiveHullTrace` and QuakeSpasm `SV_SlowRecursiveHullCheck`), including `DoublePrecisionDotProduct` narrowing, the `DIST_EPSILON` double-literal promotions, and the `frac -= 0.1` double promotion in the solid back-off loop (each marked ADR-010). `box_hull`/`box_clipnodes`/`box_planes` became Rust-private statics; the two `sv_fte_*` cvar objects stay C-owned in `world_glue.c` because `sv_main.c` registers them. `World_ClipToNetwork` reads `cl.entities` through `World_Glue_ClNumEntities`/`World_Glue_ClEntity`/`World_Glue_QcvmIsClient` accessors rather than an ADR-011 `cl` mirror -- mirrors for `cl`/`cls` are M7 work and adding one here would open an ADR-007 row this milestone cannot close. **No ADR-007 or ADR-011 mirror changed in M3.**
  - *ADR-006 re-entrancy:* `sv_touch_links` holds no Rust reference across `World_Glue_CallTouch`; the touched set is a `Vec<u16>` of edict *numbers* (the ADR-006 substitution for C's `TEMP_ALLOC` list), every pointer is re-derived through a guarded `World_Glue_EdictNum` after each dispatch, and the full C re-validation triple (`free`/identity, `touch`/`solid`, 6-way bbox) is preserved. The deliberate **non**-restore of `pr_global_struct->self`/`other` on the raising path is kept: `world.c`'s longjmp skips that restore, so returning the status early is the faithful behaviour, and it is commented as such.
  - *Signature gate extended:* `check_capi_signatures.sh`'s model TU now includes `protocol.h`, `progs.h` and `world.h` (plus the `@PER_LEVEL_LIMITS@` substitution) so the wrapper signatures are diffed against the engine headers. That required prototypes `world.h` never had -- `SV_CreateAreaNode`, `SV_InitBoxHull`, `SV_HullForBox`, `SV_HullForEntity` (+6 lines). Coverage is now **17 of 18** exports; `SV_FindTouchedLeafs` and `SV_MoveBounds` remain header-less and are covered only by the differential tests. **Red-tested four ways:** mutating `SV_CreateAreaNode`'s signature fails the gate; mutating `SV_InitBoxHull`/`SV_HullForBox` fails with two errors; mutating the still-header-less `SV_MoveBounds` **passes**, proving the gate is declaration-driven and the 17/18 figure is honest; mutating a wrapper parameter in a throwaway `world_glue.c` copy fails `clang-cl -fsyntax-only`.
  - *Guard-cost gate (ROADMAP landmine #6, now closed).* `Host_Guard` measured at **16.805 ns/call** vs **1.744 ns** for the same call direct (clang-cl `/O2`, 20M iterations; `sizeof (jmp_buf)` is 256, so each guard is ~1 KB of `memcpy` plus two `setjmp`s). Guards-per-server-frame measured on three server-exercising corpus entries with temporary counters in `world_glue.c` (applied, measured, reverted -- not committed): `e1m1-long` 2730 guards / 6001 frames = **0.45/frame**; `e1m1-plat-crush` 7605 / 4001 = **1.90/frame**; `e3m6-trains` 16338 / 6001 = **2.72/frame**. Worst case is ~46 ns/frame, ~0.0003% of a 14 ms frame. The fear that motivated the landmine -- a guard per link per move -- does not materialise: the only guard sites are `World_Glue_CallTouch`, `World_Glue_EdictNum` and `World_Glue_NumForEdict`, all inside `SV_TouchLinks`, so guard count tracks trigger *overlaps*, not moves. The `SV_Move`/hull-check hot paths take zero guards, and the two `World_Glue_WarnSolidBsp*` guards never fired on any entry. No guard batching needed.
  - *Gate evidence (all measured, Windows x64, this checkout):*
    - All three meson configs build clean, each with the right world TU: `build-rs` links `Quake_world_glue.c.obj` and **no** `Quake_world.c.obj`; `build-c` and `build-rs-chost` link `Quake_world.c.obj`.
    - `run_corpus.py --compare build-c build-rs` (registered tier): **11 ran, 8 skipped (missing mod/rerelease data -- normal for this machine), 0 failed**, every entry `identical`.
    - `physics_matrix.py --vkquake-a build-c --vkquake-b build-rs`, 12 cells x 7 entries, C-vs-Rust: **84 ok, 0 skipped, 0 failed** -- both `sv_fte_recursivehullckeck` settings, so both hull-check implementations are exercised against the C oracle.
    - New `rust/quake-ctest/tests/world_differential.rs`: **31/31 pass**. Both hull checks, areanode insertion order under both `sv_fte_createareanode` settings, `SV_LinkEdict`/`SV_TouchLinks` ordering incl. re-entrant relink, the `SV_Move` pipeline, `SV_PointContentsAllBsps`, and two raise tests driving the `SV_HullForEntity` `Con_Warning` paths through `ctest_try_host` (asserting the raise actually fired -- `c.0 == 1` -- before comparing the `PR_GetString` message, otherwise the test would prove nothing about propagation). **Red-tested by mutation across three rounds** (13/29 failures on the first sweep); all mutations reverted, zero `MUTATION` markers remain.
    - Harness topology mirrors the engine: `stubs.c` owns plain-named wrappers over all six cores, and the fixture's re-entrant link hook is installed from C (`ctest_world_set_rust_link_fns`) rather than from a Rust function pointer, so no longjmp unwinds a Rust frame in the tests either.
    - `check_ctest_symbols.sh`: OK -- 31 oracle sources (now incl. `world.c`).
    - `check_capi_signatures.sh`: OK against a freshly regenerated cbindgen header. `check_headers.sh`: exit 0, 12/12 compile + 12/12 bindgen. (Note: `check_headers.sh:17` hardcodes `cc`, which Git Bash does not provide; a `cc -> clang` shim was needed to run it locally. Not an M3 regression -- the checked list is 12 fixed core headers, none of them `world.h` -- and left unfixed as out of scope.)
    - `cargo fmt --check` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace` **83 test binaries, 619 tests passed, 0 failed**.
    - `clang-format` clean on `Quake/world_glue.c` and `Quake/world.h`. (`format.sh` runs clang-format 18 in docker and could **not** run -- docker daemon down; local clang-format 22.1.8 was used and agrees with the checked-in formatting of both files.)
    - `Quake/common.make` deliberately unchanged: it has `world.o` but zero `USE_RUST`/`RUST` references (it never gained M2's `cvar_cmd_glue.o` either), so it is a pure-C path, and M3 splits no C file.
    - Not run (out of scope for M3): `save_diff.py`, `builtin_diff.py`/`trace_diff.py` (no progs or savegame change), `record_diff.py`/`netreplay_diff.py`, interop/soak (start at M4), Miri, Linux/macOS legs (CI on push).
