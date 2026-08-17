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
| `-tracefile <file>` | per-instruction progs VM trace (needs a `-Dtrace=true` build) |

The state hash covers: per-edict `free`/`freetime`/`alpha` + the full progs-visible field block, progs globals, VM time, client sim variables, client entity states, and the RNG state. It deliberately excludes pointers, area links and the debug-only edict header, so debug and release builds hash the same *state* (though FP differences mean goldens are release-only).

Harness runs are hermetic: per-user files (`vkQuake.cfg`, history, remembered basedirs) are redirected into the disposable staging gamedir.

## Scripts (`scripts/harness/`)

- `run_demo.py` — one headless run; stages a writable basedir from `$QUAKE_GAME_DATA`
- `run_corpus.py` — drive [corpus.json](corpus.json): `--generate` / `--check` goldens, or `--stability` (run-twice)
- `save_diff.py` — scripted map+save scenario byte-compared between two builds
- `capture_session.py` — dedicated server + headless client localhost protocol capture
- `run_trace.py` — progs trace collection on a `-Dtrace=true` build
- `fetch_shareware.py` — pull the redistributable 1.06 shareware data for CI
- `check_headers.sh` — core headers compile standalone + bindgen smoke

Point `QUAKE_GAME_DATA` at a directory containing `id1/` (mission packs, `rerelease/`, and mod dirs beside it are picked up by their corpus tiers). The path never appears in the repo.

## Corpus tiers and goldens

- **shareware** — runs in CI from `fetch_shareware.py` data.
- **registered / rerelease** — need locally provided data; entries are pinned to data versions by checksum and skip (with a warning) on mismatch.
- **mod** (Arcane Dimensions, Copper, Alkaline, Quoth) — entries exist but skip until the mod data is dropped into the game-data dir; adding a mod is a data drop plus golden regeneration, not a code change.

Goldens live in `goldens/<os>-<arch>/` with a `MANIFEST.json` recording provenance. Rules:

- goldens are generated from **release** builds only, and compared only against the **same platform** (the C engine is not cross-platform FP-deterministic — [ADR-010](../../docs/rust-migration/adr/ADR-010-determinism-policy.md));
- regenerate only from a `c-reference/*` tag, never hand-edit;
- the headless RNG stream intentionally differs from a windowed run (menu/renderer RNG consumers are absent), so goldens only ever compare headless runs against headless runs.

Current coverage: `darwin-arm64` (full local tier). Linux/Windows goldens are a known gap until a machine with game data generates them (CI meanwhile enforces run-twice stability and mixed-vs-C-only identity, which need no goldens).
