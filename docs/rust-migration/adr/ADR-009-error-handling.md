# ADR-009: `Result`/`HostError` replaces setjmp/longjmp; no-unwind FFI rules

**Status:** Accepted
**Date:** 2026-08-16
**Tags:** —

## Context

The engine's error architecture is non-local control flow: `Host_Error`/`Host_EndGame` (`host.c`) `longjmp` to `host_abortserver` (set each `_Host_Frame`), and a second buffer `screen_error` is set in `SCR_UpdateScreen`. A `longjmp` across a Rust frame is undefined behavior (Rust frames are not trivially skippable — destructors, unwind metadata), and a Rust panic crossing into C is likewise unsound. During the migration, C and Rust frames interleave on the same call stacks.

## Decision

**Hard rules, enforced by construction:**

1. A `longjmp` never unwinds a Rust frame. A Rust panic never crosses into C.
2. Every Rust function exported to C (`quake-capi`) returns a status code; a small C macro at the call site re-raises via `Host_Error` when needed — so every longjmp originates and lands entirely within C frames.
3. Every C function Rust calls that can `Host_Error` is invoked through a C trampoline — `int Host_Guard(void (*fn)(void*), void *arg)` — that `setjmp`s locally and returns an error code instead of jumping past Rust. The trampoline list is kept small by porting error-raising leaf code early.
4. Release builds use `panic = "abort"`; transition-period export shims additionally `catch_unwind` in debug builds to convert bugs into diagnostics rather than UB.

**Rust-side architecture:** layered error enums (`ParseError`, `NetError`, `ProgsError`, `SndError`, …) convert into

```rust
enum HostError { Error(String), EndGame(String), Abort }
```

propagated by `Result` to the host frame loop, which performs today's longjmp-target behavior (abort server frame, disconnect, drop to console). Panics are reserved for engine bugs, never game-state conditions.

**End state (Phase 9):** the host loop is Rust; `setjmp`/`longjmp` is deleted with its last C caller; `screen_error` becomes an error path of the render frame function.

## Consequences

- Soundness across the boundary is guaranteed by two mechanical patterns (status shims, trampolines) rather than per-call-site vigilance.
- Transition cost: some double bookkeeping (C raises → trampoline → Rust `Err` → shim status → C re-raise) on error paths; error paths are cold, so the cost is code, not speed.
- The final `Result`-based architecture is idiomatic and makes error provenance explicit — a net improvement over longjmp once the transition ends.

## Amended (Phase 6 M3, 2026-08-27)

`Host_Guard` now exists (`Quake/host.c`, declared in `quakedef.h`). Rule 3
said the trampoline "`setjmp`s locally and returns an error code instead of
jumping past Rust"; implementing it surfaced a detail worth writing down.

`Host_Error` does most of its work — `PR_SwitchQCVM (NULL)`, the stack-trace
print, `Host_ShutdownServer`, `CL_Disconnect` — *before* it jumps. A guard that
caught the jump and then called `Host_Error` again to re-raise would run all
of that twice, which is observable. So the trampoline is a **pair**:

- `Host_Guard (fn, arg)` installs its own `host_abortserver` *and*
  `screen_error` buffers (the CSQC-drawing path jumps to the latter), runs
  `fn`, restores both, and returns which jump it caught;
- `Host_Reraise (result)` re-issues that same jump from a pure C frame once
  the Rust frames above have returned normally.

Nesting is one level at a time: an inner guard restores the outer's buffers on
the way out, so each re-issued jump is caught by the next guard out until the
outermost reaches `host.c`'s own `setjmp`. `Quake/pr_exec_glue.c` is the first
consumer, wrapping every `qcvm->builtins[i]()` dispatch.

The ADR's "error paths are cold, so the cost is code, not speed" holds here
only because the guard is cheap relative to how often it runs: the e1m1
gameplay trace shows ~13 builtin calls per frame, so two `jmp_buf` copies and
a `setjmp` per call are not measurable against a 13.9 ms frame. If a later
milestone puts a guard somewhere genuinely hot, that measurement has to be
redone rather than assumed.
