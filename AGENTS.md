# AGENTS.md

Instructions for AI coding agents (Codex, GitHub Copilot, Claude Code, etc.) working in this repo. Read this before making changes. Keep answers and diffs scoped — this file is intentionally short; follow the links for detail instead of asking to have it restated.

## Project

vkQuake is a C99 port of id Software's Quake using Vulkan instead of OpenGL, based on QuakeSpasm/QuakeSpasm-Spiked. Cross-platform: Windows, Linux, macOS. Engine source lives in `Quake/` (including inlined vendored libs like `miniz`, `stb_image`, `mimalloc`). Shaders are in `Shaders/`. Windows-only resources (VS solution, vendored SDL3) are in `Windows/`.

## Build & verify

- **Primary:** Meson — `meson setup build && ninja -C build`.
- **Fallback (Linux/macOS):** `cd Quake && make -j`.
- **Windows alternative:** `Windows/VisualStudio/vkquake.sln` (MSVC), or MinGW/MSYS2.
- There is **no automated test suite** in this repo. Verification is: the build succeeds for the platform(s) you touched, and behavior is checked manually (or via the Rust-migration verification harness, see below, where applicable). CI (`.github/workflows/`) runs build matrices for Windows (MSVC + MinGW + arm64), Linux, macOS, plus `clang-format-check.yml` — treat those workflows as the ground truth for what a change must pass.

## Code style

Formatting is enforced by `.clang-format` (LLVM base, tabs, 4-width, Allman braces, 160-column limit, pointers right-aligned). Run `./format.sh` before committing rather than hand-matching the rules. Existing code uses minimal comments — match that; don't add comments explaining *what* code does, only non-obvious *why*.

## Branching & workflow (simplified gitflow)

`master` is the single trunk and also serves as the integration branch — there is no separate `develop`.

- `feature/<name>` — branched off `master`, merged back via PR.
- `release/<version>` — branched off `master` for release stabilization/tagging.
- `hotfix/<name>` — branched off `master` for urgent fixes, merged back via PR.

Commit messages: concise, imperative mood, matching existing `git log` history. Don't invent a different branch taxonomy.

## Rust migration

A migration plan exists to incrementally port the C engine to Rust. **Read `docs/rust-migration/PLAN.md` and `docs/rust-migration/ROADMAP.md` before touching migration-related code**; ADRs are indexed in `docs/rust-migration/adr/README.md`.

Key facts:
- Strategy is hybrid incremental oxidation (ADR-001): a Cargo workspace under `rust/` (not yet created — Phase 0 hasn't started) builds a staticlib linked into the existing Meson build, module by module, behind `-Duse_rust_<module>` flags. C is deleted only after each phase's exit criteria pass.
- The C build remains the reference oracle until Phase 9 (host inversion).
- The roadmap is 11 phases (0–10) with explicit scope, exit criteria, and deletion lists per phase — **do not port code or delete C files out of roadmap order**, and don't touch code explicitly deferred to a later phase (e.g. `tasks.c` stays C until Phase 8, per ADR-016).
- Follow ADR decisions exactly, especially the `(compat exception)` ones (e.g. ADR-005 float formatter, ADR-006 edict arena, ADR-008 ambient qcvm, ADR-010 determinism) — these are deliberate deviations from idiomatic Rust made to preserve bug-for-bug compatibility. Mark code implementing one with a `// COMPAT:` comment linking to the ADR, per the ADR template.
- Check ADR-003 before adding any third-party crate, and ADR-004 before writing `unsafe`.
- **Crate licensing is permissive-only, and MIT is preferred** (ADR-003). Never introduce a crate — directly or transitively — under a copyleft license (`LGPL`, `GPL`, `AGPL`, `MPL-2.0`, `CDDL`, `EPL`, `EUPL`, `CC-BY-SA`) or one requiring a paid/commercial license (`BUSL-1.1`, `SSPL`, `Elastic-2.0`, "free for non-commercial use"). Allowed: `MIT`, `MIT-0`, `Apache-2.0`, `BSD-2-Clause`, `BSD-3-Clause`, `Zlib`, `ISC`, `0BSD`, `Unicode-DFS-2016`, `Unicode-3.0`. Check the license of every crate you propose *and* its dependency tree (`cargo deny check licenses`, or `cargo tree` plus the crates.io listing) before writing it into a `Cargo.toml`; when two crates would both work, take the MIT one. If the only crate that fits is copyleft or paid, stop and ask rather than adding it — the fallback is a permissive alternative or in-tree code.

## Testing & verification strategy

- Ordinary C engine changes: build on the affected platform(s); there's no test suite to run, so be conservative and verify behavior manually where practical.
- Rust migration work (Phase 0+): the differential-verification harness described in `PLAN.md` §7 and ADR-019 (demo-determinism state-hash chains, savegame/config byte-diffing, progs VM trace oracle, protocol goldens, differential fuzzing, float-formatter conformance, sound PCM-hash parity, sanitizers) is the safety net for compatibility — it must exist and stay green alongside any port, not be treated as optional scaffolding.

## Agent conduct

- Keep diffs minimal and scoped to what was asked; don't refactor unrelated code or add abstractions the task doesn't need.
- Don't introduce new build systems, dependencies, or vendored libraries without checking the relevant ADR first — for Rust crates that means the permissive-only, MIT-preferred license rule in ADR-003 above.
- Prefer targeted reads (specific files/ranges) over dumping whole files or directories into context.
- When a change touches the Rust migration, name the roadmap phase and/or ADR it relates to in the commit/PR description.
