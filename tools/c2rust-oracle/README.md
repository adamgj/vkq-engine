# c2rust oracle

Quarantined [c2rust](https://github.com/immunant/c2rust) transpiler output used as a **semantic tiebreaker and differential-fuzz oracle** for the Rust migration — never as a code base and never linked into any shipping binary ([PLAN.md §2](../../docs/rust-migration/PLAN.md)). When the C's behavior is ambiguous (integer promotion, sequence points, float truncation), the mechanical translation here is the reference for what the C actually does.

This package is deliberately **excluded from the `rust/` workspace**: transpiled output is raw-pointer, `unsafe`-everywhere code that must never be able to break `cargo build/clippy --workspace` or pollute the workspace's lint/audit gates. Build it only from this directory, only when consulting the oracle.

## Usage

1. `./gen_compile_commands.sh` — configure a C-only build dir and copy its `compile_commands.json` here (Meson generates it automatically).
2. `./translate.sh` — transpile the Phase 0 oracle targets (`pr_exec.c`, `mathlib.c`, `world.c`) into `translated/`. Adjust the `--filter` for other subsystems as later phases need them (Phase 1: `mathlib`; Phase 6: `pr_exec`; Phase 7: `world`).

Commit refreshed output under `translated/` with a note of the c2rust version and the engine commit it was generated from.

## Installing c2rust

c2rust pins a specific LLVM and can be fiddly to build natively (especially on macOS arm64). Options:

- **Native:** `cargo install c2rust` — needs a matching LLVM/Clang dev package (see the c2rust README for the supported version); on macOS: `brew install llvm@<version>` and set `LLVM_CONFIG_PATH`.
- **Container:** there is no maintained official c2rust image (checked 2026-08); build one from the `docker/` directory in the c2rust repo, or `cargo install c2rust` inside an Ubuntu container with `llvm-dev libclang-dev clang`, then run `gen_compile_commands.sh` + `translate.sh` in-container so the include paths match.

## Status

Phase 0 delivered the scaffolding; the first committed translations are deferred until a working c2rust toolchain is available (the oracle's first consumers are Phase 1's `mathlib` port and Phase 6's interpreter port, so nothing blocks on it yet). This deferral is recorded in [ROADMAP.md](../../docs/rust-migration/ROADMAP.md).
