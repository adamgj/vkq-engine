# ADR-005: C-printf-compatible float formatter

**Status:** Accepted
**Date:** 2026-08-16
**Tags:** (compat exception) (user decision: byte-identical saves required)

## Context

Savegames (`SAVEGAME_VERSION 5`) are text files whose float values are written with C `"%f"` (6 decimals) via `PR_UglyValueString` (`pr_edict.c`), plus `"%f"` spawn parms and `qcvm->time`; `config.cfg` and console output also use C printf formatting (`PR_FloatFormat`/`PR_DoubleFormat` use `"% 5.0f"`/`"% 7.1f"`). The project owner confirmed the compatibility bar is **byte-identical** output, not merely load-compatible. Rust's `Display`/`format!` for floats (shortest-roundtrip formatting) does not match C `%f` semantics — rounding mode at the 6th decimal, fixed precision, negative zero, inf/nan spellings.

## Decision

Implement a **C-printf-compatible formatting module** in `quake-util` covering the conversions the engine actually uses (`%f`, `%.Nf`, `% 5.0f`-style flag/width combinations, `%i`, `%d`, `%x`, `%g` where used). Savegame/config/console writers call this module, never `format!`, for compat-relevant output. `// COMPAT: ADR-005` at call sites.

Conformance is enforced by tests comparing against the platform C `snprintf` (via a tiny test-only FFI hook): a stratified sample of f32 bit patterns on every CI run, and a scheduled exhaustive sweep of all 2³² f32 patterns per CI OS.

Platform caveat: C `%f` output itself can differ across libc implementations for edge values; the requirement inherits ADR-010's per-platform policy — Rust output matches the C build **on the same platform**.

## Consequences

- Savegame/config byte-diff testing (ADR-019) becomes possible and is the strongest cheap regression signal in the project.
- A small amount of "un-idiomatic" formatting code exists permanently; it is pure, safe, and exhaustively tested.
- If a future libc changes formatting behavior (unlikely for `%f`), goldens regenerate from the C reference tag.

## Amended (Phase 1, 2026-08-17)

Implemented as `quake_util::printf`. Covered conversions: `%f`/`%F` (exact
fixed-point decimal via an in-tree bignum, IEEE round-half-even — verified
against the platform snprintf), `%d`/`%i`/`%u`/`%x`/`%X` with `l`/`ll`/`z`
modifiers, `%s`, `%c`, `%%`, full flag/width/precision handling, per-platform
inf/NaN spellings. `%g`/`%e` are deliberately unimplemented — no engine
writer in the ported set uses them (`%x` users arrive in Phase 6) — and
panic if reached; adding them requires extending the conformance suite first.
The exhaustive 2^32 f32 sweep is green on darwin-arm64 (all patterns x the
engine's three float spec shapes, ~52 minutes); the scheduled CI job runs it
weekly per OS. The test-only snprintf hook lives in `rust/quake-ctest`
(typed non-variadic C wrappers), keeping `quake-util` `forbid(unsafe_code)`.
