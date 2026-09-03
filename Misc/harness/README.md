# Differential-verification harness

The safety net for the C→Rust migration ([PLAN.md §7](../../docs/rust-migration/PLAN.md), [ADR-019](../../docs/rust-migration/adr/ADR-019-verification-architecture.md)): headless demo playback with a per-frame chained state hash, scripted savegame byte-diffing, a progs VM instruction trace, and raw protocol capture. Built in Phase 0, before any code is ported; every later phase's exit criteria reference these gates.

## Engine flags

All compiled into every build (runtime-gated); see [harness.h](../../Quake/harness.h):

| Flag | Effect |
|---|---|
| `-headless` | client without video/audio/input (a third mode besides windowed and `-dedicated`) |
| `-demohash <file>` | write the per-frame state-hash chain; forces a fixed 1/72s timestep and a fixed RNG seed |
| `-exitafter <n>` | hard frame cap, exit code 2 (runaway guard) |
| `-harnesscmds <file>` | inject console commands at fixed frames (`<frame> <command>` per line) — also how demos/maps are started, since the `cmdline` cvar is deliberately empty in shareware installs |
| `-netcapture <file>` | framed capture of all traffic at the `NET_*` funnels |
| `-netreplay <file>` | deterministic client-side replay of a `-netcapture` recv stream (one record per frame; sends absorbed); forces the fixed timestep, so with `-demohash` the replayed session byte-compares across builds |
| `-tracefile <file>` | per-instruction progs VM trace (needs a `-Dtrace=true` build) |

The state hash covers: per-edict `free`/`freetime`/`alpha`/`baseline`/lerp fields/`num_leafs`+populated `leafnums` + the full progs-visible field block, progs globals, VM time, client sim variables, client entity states, and the RNG state. It deliberately excludes pointers, area links, the debug-only edict header, and `leafnums` entries past `num_leafs` (stale leftovers no observer can see), so debug and release builds hash the same *state* (though FP differences mean goldens are release-only).

**The server half only applies when a server is running.** Demo playback never starts one, so for the demo entries `Harness_HashServer` returns immediately and the chain covers client sim state, client entity states and the RNG only. Server/edict state is exercised by the map entries (`save-e1m1`, `map-e1m2`, `save-e2m1`, and the Phase 7 pusher entries `e1m1-long`, `e1m5-trains`, `e1m1-plat-crush`), which is why CI runs the corpus rather than demos alone. Since Phase 7 M1 the client half also hashes `cl.qcvm` when a csprogs is live, so CSQC globals are covered — on id1 data no csprogs loads, so that branch is dormant and was proven live by fault injection rather than by any corpus entry.

Harness runs are hermetic: per-user files (`vkQuake.cfg`, `basedirs.txt`, console history, and the `-condebug` log) are redirected into the disposable staging dir rather than the real pref directory.

## Scripts (`scripts/harness/`)

- `run_demo.py` — one headless run; stages a writable basedir from `$QUAKE_GAME_DATA`
- `run_corpus.py` — drive [corpus.json](corpus.json): `--generate` / `--check` goldens, `--stability` (run-twice), or `--compare <other-build>` (the mixed-vs-C-only gate, which needs no goldens and so works on platforms that have none yet)
- `save_diff.py` — scripted map+save scenario byte-compared between two builds
- `capture_session.py` — dedicated server + headless client localhost protocol capture
- `capture_diff.py` — structural capture differ (reliable-stream prefix under a calibrated window + per-kind counts)
- `record_diff.py` — deterministic loopback `record` session byte-compared between two builds
- `netreplay_diff.py` — replay one capture on two builds; state-hash chains + a demo recorded mid-replay must be byte-identical (the timing-noise-free net gate)
- `interop_matrix.py` — 4-way C/Rust client x server localhost matrix across the negotiable protocol cells (`Base-/FTE+` 15/666/999; optional `--ipv6` leg). `--soak` runs long sessions instead (see below)
- `physics_matrix.py` — server-physics cvar sweep: `sv_fte_recursivehullckeck` (0/1) × `sv_gameplayfix_elevators` (0–3) × `sv_smoothplatformlerps` (0/1), each cell a state-hash compare of two builds over the pusher/elevator-heavy corpus entries. `--cells all` is the 16-cell factorial (local); the default `DEFAULT_CELLS` is the 12-cell CI trim. Cvars are delivered as prepended `0 <cvar> <value>` lines in the `-harnesscmds` script, **not** `+cvar` on the command line (Windows command-line length cap). The file header records which axes are proven to fire on the current corpus and which are not — the lerps axis is still vacuous, so a green run must not be read as coverage of it
- `run_trace.py` — progs trace collection on a `-Dtrace=true` build (`--game <dir>` to trace a mission pack's or mod's own progs.dat, which needs registered data)
- `trace_diff.py` — the ADR-019 gate-3 consumer: same headless scenario on two `-Dtrace=true` builds, every VM record compared in order, with a minimum-record floor (demo playback starts no server and so emits **zero** progs records — the oracle scenarios are maps)
- `builtin_diff.py` — the resolved QuakeC builtin table (`pr_dumpbuiltins`) compared across two builds: `name declared-number bound-ordinal` per `extensionbuiltins[]` entry plus the re-release `first_statement` patches. Builtin *numbering* is set by `PR_InitExtensions`/`PR_EnableExtensions`/`PR_PatchRereleaseBuiltins` and is invisible to a trace unless a mod calls the affected builtin. Carries a minimum-entry floor
- `fetch_shareware.py` — pull the redistributable 1.06 shareware data for CI
- `check_headers.sh` — core headers compile standalone + bindgen smoke

### `interop_matrix.py --soak`

Long two-process localhost sessions (dedicated server + headless client), for desync classes that only appear over tens of thousands of server frames. The pass criterion is deliberately **not** hash identity: which server frame a live UDP datagram lands on is scheduler- and socket-buffer-dependent, and a one-frame shift in applying a `clc_move` forks the two simulations permanently. A cell passes when all four hold:

1. no `Host_Error`/`Sys_Error`, crash, timeout or unexpected exit;
2. the negotiated protocol is the expected one;
3. **liveness** — every reference checkpoint frame was reached;
4. the traffic profile is within the same tolerance the non-soak matrix gate uses.

The frame at which the hash chains first diverge is reported as a diagnostic only. C/C is re-run like any other combo rather than short-circuited as "equal to its own reference" — that short-circuit is what hid two real engine bugs (client pacing in `Quake/main_sdl.c`, `WSAECONNRESET` treated as fatal in `Quake/net_wins.c`) for the whole of M1, because the comparison path had never once executed against two independently launched processes. Red-tested by injecting `net_messagetimeout 0`, which drops the client and fails condition 3/4.

CI runs a 20k-frame C/C smoke; the full 100k-frame × 8-cell soaks are local-only (M9 and M11), with results recorded in the phase plan's amendment log.

### Phase 7 pusher/elevator entries

`e1m1-long`, `e1m5-trains` and `e1m1-plat-crush` (T1.1) exist to give the physics matrix something that actually moves pushers. `e1m1-plat-crush` is the one with non-obvious construction, so the derivation is recorded here rather than in a corpus note:

`sv_gameplayfix_elevators` only changes behaviour on the BLOCKED branch of `SV_PushMove` ([sv_phys.c:704-705](../../Quake/sv_phys.c), gated at sv_phys.c:1540-1546): it fires when an entity is *riding* a pusher (`FL_ONGROUND` with `groundentity == pusher`) and the pusher's move leaves it still embedded — an elevator crushing its rider, not a horizontal pusher shoving something aside, which the cvar does not affect. e1m1's edict 22 `func_plat` (`blocked = plat_crush()`) has a static, always-active `trigger_field` (edict 23, `touch = plat_center_touch()`, no player-trigger or button needed) spanning its travel column. `setpos -544 2656 38` at frame 10 drops the player into that field just above the plat's resting top surface without embedding in solid BSP — most of the plat's nominal footprint *is* solid, and this spot was found by binary-search probing candidates with `-harnesscmds` `edicts` dumps — and `noclip` at frame 25 restores `MOVETYPE_WALK` so the player can be pushed and blocked. The plat then cycles and crushes its rider repeatedly (health 100→79→65→50→23 over ~500 frames), taking the blocked path every cycle.

Differential evidence that the axis is live (same exe, `run_corpus.run_entry` with `0 sv_gameplayfix_elevators 0` / `3` prepended per `physics_matrix.py`'s `cvar_cmds_for` pattern): the two hash chains first diverge at `F 56`, right as the player is caught by the plat, and differ overall; a same-cvar rerun (0 twice) is byte-identical, so the divergence is the cvar and not nondeterminism.

`sv_smoothplatformlerps` — the `MOVETYPE_STEP` walking-monster-on-a-lift axis — is **not** covered. e1m1's `monster_dog` (edict 118) wanders near the plat but its patrol AI never steps into the trigger footprint inside the frame budgets tried, and there is no monster equivalent of `setpos` to script it. That axis still passes vacuously; a green matrix run is not evidence about it.

Point `QUAKE_GAME_DATA` at a directory containing `id1/` (mission packs, `rerelease/`, and mod dirs beside it are picked up by their corpus tiers). The path never appears in the repo.

## Corpus tiers and goldens

- **shareware** — runs in CI from `fetch_shareware.py` data.
- **registered / rerelease** — need locally provided data; entries are pinned to data versions by checksum and skip (with a warning) on mismatch.
- **mod** (Arcane Dimensions, Copper, Alkaline, Quoth) — entries exist but skip until the mod data is dropped into the game-data dir; adding a mod is a data drop plus golden regeneration, not a code change.

Goldens live in `goldens/<os>-<arch>/` with a `MANIFEST.json` recording provenance. Rules:

- goldens are generated from **release** builds only, and compared only against the **same platform** (the C engine is not cross-platform FP-deterministic — [ADR-010](../../docs/rust-migration/adr/ADR-010-determinism-policy.md));
- regenerate only from a `c-reference/*` tag, never hand-edit;
- the headless RNG stream intentionally differs from a windowed run (menu/renderer RNG consumers are absent), so goldens only ever compare headless runs against headless runs.

Current coverage: `darwin-arm64` (full local tier, generated before Phase 7) and `windows-x86_64` (generated at Phase 7 M11). The three pusher entries, `e1m1-long`, `save-e2m1` and `music-wav` have goldens on `windows-x86_64` only; `darwin-arm64` predates them and `run_corpus.py --check` skips a missing golden without failing, so a green `--check` on macOS is not evidence about those entries. Windows CI runs `--check` (plain and `--sndhash`) against both the C-only and mixed builds; the `--stability` and mixed-vs-C-only `--compare` steps stay because they also cover the registered-tier entries CI has no data for. Linux goldens remain a known gap (recorded ROADMAP amendment, Phase 7) until a machine with game data generates them; Linux CI meanwhile enforces run-twice stability and mixed-vs-C-only identity, which need no goldens.
