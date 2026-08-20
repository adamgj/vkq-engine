# Fable 5 workflow for large vkqr-engine changes

This workflow controls repeated exploration, continuous re-planning, unchanged retries, raw-output pollution, speculative scope, and unbounded agents. It does not reduce required repository context, compatibility analysis, platform coverage, or verification.

## Persistent versus on-demand context

`AGENTS.md` contains only durable project constraints and execution discipline. `CLAUDE.md` imports it. Detailed procedures live here and in `.claude/skills/`, so they consume context only when relevant.

For ordinary C/Vulkan/platform work, read the task, relevant source and callers, build configuration, and affected CI workflow. For any Rust-migration work, also read `docs/rust-migration/PLAN.md`, `docs/rust-migration/ROADMAP.md`, the ADR index at `docs/rust-migration/adr/README.md`, every applicable ADR, and `Misc/harness/README.md`. A task plan may refine a roadmap milestone but cannot supersede it silently.

## Model and effort routing

| Work | Selected by | Model | Effort | Boundary |
|---|---|---|---:|---|
| Architecture/task plan | `/feature-plan` | Fable 5 | xhigh | One read-only pass |
| Difficult implementation | `/feature-implement` | Fable 5 | high | One approved milestone per turn |
| Loop diagnosis | `/loop-breaker` | Fable 5 | high | Read-only, one diagnostic |
| Repository discovery | `repo-researcher` | Haiku | low | Read-only, 14 turns |
| Build/test/harness diagnosis | `verification-diagnostician` | Sonnet | medium | No edits, 10 turns |
| Independent integration review | `compatibility-reviewer` (`/integration-review`) | Opus | high | Fresh read-only context, 20 turns |

Model and effort are set in each skill's and agent's frontmatter; the table above is a description of those files, not a second source of truth. `/loop-breaker` deliberately keeps Fable at `high` despite being a short read-only turn: it fires after a budget overrun, and a wrong next action there costs another loop.

The independent reviewer uses a different capable model to reduce shared implementation assumptions. Change it to `model: fable` only for an exceptionally high-risk review whose expected benefit justifies the additional cost.

Do not make Fable the project-wide model in `.claude/settings.json`; the manual skills select it for the turns where its capability is useful. Keep normal effort at `high`. Do not enable permanent `xhigh`, `max`, or `ultracode`.

## 1. Prepare a decision-ready brief

Copy `docs/ai/FEATURE_PLAN_TEMPLATE.md` to a task-specific path such as `docs/ai/plans/<feature>.md` when a durable plan is warranted. **Commit the plan.** It is the durable context that survives compaction and the artifact `/feature-implement` and `/integration-review` read back, so it has to exist in a fresh clone or worktree; `docs/**` is excluded from CI, so committing it costs nothing. Fill requirements, non-goals, invariants, compatibility surfaces, affected platforms, and testable acceptance criteria.

For migration work, identify the exact roadmap phase and ADRs before planning. If the requested feature conflicts with phase order, a compat exception, an approved deletion list, or the dependency/unsafe policies, stop and surface the conflict rather than designing around it.

## 2. Run one architecture pass

Enter Plan Mode and invoke:

```text
/feature-plan docs/ai/plans/<feature>.md
```

Review and save the approved plan. Do not request repeated full plans. Amend it only when a concrete repository contradiction makes the approved path impossible, unsafe, or materially wrong.

The architecture pass is complete when call paths and contracts are sufficiently traced, existing patterns are selected, milestones are coherent, platform/migration impacts are explicit, and every milestone has acceptance criteria plus verification.

## 3. Implement one milestone

Leave Plan Mode and invoke:

```text
/feature-implement docs/ai/plans/<feature>.md M1
```

At the milestone boundary, review the diff, run its targeted checks, update the plan evidence/handoff table, and stop. Start the next milestone with a new invocation.

For a contained non-interactive milestone, add a hard turn boundary:

```text
claude -p --model fable --effort high --max-turns 12 "/feature-implement docs/ai/plans/<feature>.md M1"
```

Tune the cap from observed work. A cap that truncates coherent milestones harms quality; a cap that permits known loops is ineffective. Reaching the cap is a checkpoint, not permission to relaunch unchanged.

## Verification selection

Use the cheapest relevant check capable of disproving the current change first. The affected `.github/workflows/*.yml` file is the final authority for CI commands and platform coverage.

### C/engine changes

- Format applicable C/shader changes with `./format.sh` before completion.
- Primary local build: `meson setup build && ninja -C build` (or the documented Windows clang-cl/MSVC environment command).
- Manually exercise changed behavior where practical because there is no general engine test suite.

### Rust changes

From `rust/`, use the relevant subset first, then the CI-equivalent final gates:

```text
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked --release
cargo test --workspace --locked                 # debug profile: debug_assert! canaries (ADR-013)
cargo deny check
cargo deny check licenses --manifest-path fuzz/Cargo.toml --config deny.toml
```

The debug-profile `cargo test` is a real PR gate, not a duplicate of the release run: `rust.yml` runs it so `debug_assert!` canaries fire (the `hash_map` storage-alignment canary guarding the ADR-013 global allocator). `rust/fuzz/` is a separate workspace with its own lockfile, so the main `cargo deny` never sees `libfuzzer-sys` and friends — the second deny run is what enforces ADR-003 licensing on fuzz dependencies. `cargo audit` is **not** a per-change gate; `rust.yml` runs it on the weekly cron only, because cargo-deny's advisories check already covers RustSec on every PR.

Run `./scripts/harness/check_headers.sh` for core-header/bindgen boundary changes and the binding-regeneration diff/checks required by `.github/workflows/rust.yml` for `quake-c-sys` changes. For changes to the `quake-fs` parsers (pak/wad/kpf/loc/vdf), also run the fuzz targets the `fuzz-smoke` job covers.

### Mixed C/Rust and compatibility-sensitive changes

Build both reference and mixed configurations using the platform's CI commands. CI gates **three** configurations, not two — during Phase 2 the Rust filesystem is behind its own transitional switch (`-Duse_rust_fs`, `meson_options.txt`), and `build-linux.yml` requires the harness green with it both on and off. Match CI's option set exactly, including `-Duse_sdl3=disabled`: a differently configured build invalidates state-hash and savegame byte comparisons in both directions. On Linux the core form is:

```text
meson setup build-c -Duse_sdl3=disabled -Duse_rust=disabled && ninja -C build-c
meson setup build-rs -Duse_sdl3=disabled -Duse_rust=enabled && ninja -C build-rs
meson setup build-rs-cfs -Duse_sdl3=disabled -Duse_rust=enabled -Duse_rust_fs=disabled && ninja -C build-rs-cfs
./scripts/harness/check_capi_signatures.sh build-rs/quake_rs.h
./scripts/harness/check_capi_signatures.sh build-rs-cfs/quake_rs.h
```

(macOS omits `-Duse_sdl3=disabled` and Windows uses `-Duse_sdl3=enabled`; the affected workflow file is authoritative.)

With `QUAKE_GAME_DATA` pointing to a directory containing `id1/`, select gates required by the affected subsystem and roadmap phase:

```text
python3 scripts/harness/run_corpus.py --vkquake build-c/vkqr-engine --check --tier shareware
python3 scripts/harness/run_corpus.py --vkquake build-c/vkqr-engine --stability --tier shareware
python3 scripts/harness/run_corpus.py --vkquake build-c/vkqr-engine --compare build-rs/vkqr-engine --tier shareware
python3 scripts/harness/run_corpus.py --vkquake build-c/vkqr-engine --compare build-rs-cfs/vkqr-engine --tier shareware
python3 scripts/harness/save_diff.py --vkquake build-c/vkqr-engine --vkquake-b build-rs/vkqr-engine
python3 scripts/harness/save_diff.py --vkquake build-c/vkqr-engine --vkquake-b build-rs-cfs/vkqr-engine
```

`--check` compares against the committed per-frame goldens and is the gate that catches a real regression, so run it wherever `Misc/harness/goldens/` has an entry for the platform — today only `darwin-arm64`, which is why `build-mac.yml` runs `--check` and `build-linux.yml` does not. `--stability` and `--compare` are the fallback where goldens do not exist yet; they cannot substitute for `--check` where they do.

Skipping the `build-rs-cfs` leg reports the harness clean while the C-filesystem-fallback configuration was never differentially compared — a false green on exactly the Phase 2 exit criterion.

Use Windows executable paths and PowerShell environment syntax on Windows, matching `build-windows.yml`. Do not hand-edit or casually regenerate goldens. Goldens follow `Misc/harness/README.md`, ADR-010, c-reference tag, release-build, and same-platform rules.

Do not skip a relevant gate to save tokens. Do not run every gate after every edit. The approved plan specifies which gates run in the inner loop, at milestones, and at final completion.

## Loop detection

Interrupt when an unchanged command/search is about to repeat without transient evidence, two edits target the same failure without a more precise hypothesis, two distinct approaches fail, an approved architecture is repeatedly reopened without contradiction, or exploration expands without a dependency path.

On the next turn run:

```text
/loop-breaker <symptom or subsystem>
```

It is deliberately read-only. Apply an evidence-backed fix later through `/feature-implement`.

## Context management

The durable context is the approved task plan plus repository documentation and evidence—not the transcript. Keep raw logs in subagent context or files and return only conclusions with references.

Compact after architecture approval, a coherent milestone, or resolution of a major failure:

```text
/compact Preserve the approved objective, requirements, non-goals, roadmap phase
and ADR constraints, architecture decisions and evidence, compatibility and FFI
invariants, current milestone and acceptance criteria, changed files and why,
verification results, failed approaches that constrain future attempts, remaining
risks, and the single next action. Discard raw command output, full file contents
already on disk, repeated searches, superseded hypotheses, rejected alternatives
that no longer affect decisions, and conversational narration.
```

After compaction, reread the task plan and applicable roadmap/ADRs before editing. Use `/rewind` when a bad approach has anchored the conversation rather than spending many turns correcting it in place.

## Final review and stop condition

For high-risk, cross-platform, compatibility-sensitive, or migration work, invoke:

```text
/integration-review docs/ai/plans/<feature>.md
```

Fix only evidence-backed findings. Completion requires all acceptance criteria, required formatting/build/CI/harness/manual evidence, satisfied compatibility and ADR constraints, no unexplained diff, and explicit treatment of remaining risk.

When those conditions are met, stop. Do not ask Fable to continue until an undefined notion of perfection or to search for unrelated improvements.
