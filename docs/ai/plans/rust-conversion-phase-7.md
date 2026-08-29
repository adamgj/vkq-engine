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
