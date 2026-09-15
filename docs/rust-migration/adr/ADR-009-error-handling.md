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

### Post-guard invariant (added at the Phase 6 M5 review)

One consequence of "`Host_Error` does its work before it jumps" was not spelled
out above and is easy to get wrong: **after `Host_Guard` returns non-zero, the
Rust frame must not touch the state the raise was about.** By that point
`Host_Error` has already run `PR_SwitchQCVM (NULL)`, `SCR_EndLoadingPlaque`,
`Host_ShutdownServer` and `CL_Disconnect`. The guard makes the *jump* safe; it
does not make the *world* unchanged.

Concretely for Phase 6: `ExecSys::call_builtin` returning non-zero means the
interpreter returns `Err` immediately, without reading `VmRaw`/`EdictArena` and
without emitting the builtin-return trace record. The lumps happen to survive
today — `Host_ShutdownServer` does not call `PR_ClearProgs`, only
`Host_ClearMemory` does — so this is currently a discipline, not a live
use-after-free. It is written down here and in the trait's doc comment because
one added line on that path (a trace emit, a diagnostic drain) would make it
live.

Any future `Host_Guard` caller inherits the same rule.

## Amended (Phase 9 M7, 2026-09-15)

The host loop is now Rust (`quake-platform::main_sdl`, Phase 9 M6), which
reaches the "end state" paragraph above only in part; the rest is deferred
on purpose and this section records how far the transition got and why.

- **`HostError` exists but carries no message yet.** `quake-host::error`
  defines `HostError { AbortServer, ScreenError, Unknown(c_int) }` -- one
  variant per jump `Host_Guard` reports, not the `Error(String)` /
  `EndGame(String)` / `Abort` shape the Decision names. During the transition
  the C `Host_Error`/`Host_EndGame` print, shut the server down and disconnect
  the client *before* they jump, so by the time the loop sees the status the
  message has already reached the console and the loop has nothing left to
  do with it. The `String`-carrying variants arrive when `Host_Error` itself
  is ported (Phase 10), at which point the raise site stops printing and the
  loop takes over the longjmp-target behaviour it describes.
- **The loop consumes the status as a `Result`.** `host_frame` maps the
  `Host_Guard` `int` through `HostError::from_guard_status`; `recover` is the
  sole owner of frame-abort recovery under `USE_RUST_PLATFORM` and, by the
  post-guard invariant, touches none of the state the raise was about --
  the loop just goes round again. `ScreenError` at the loop is the one case
  the invariant does not describe: `Host_Error` takes the `screen_error`
  jump *before* `Host_ShutdownServer`/`CL_Disconnect`, and in the C it only
  ever had a target inside `SCR_DrawGUI` (a jump with no such frame landed
  on a dead buffer). The loop continues with the server and client as they
  were, which is what `SCR_DrawGUI`'s own recovery does. A status `Host_Guard` does not define has
  no C precedent (`setjmp` returns 1 on both buffers) and is reported through
  `Sys_Error` instead of ignored. A raise during `Sys_Init`/`Host_Init` is
  a `Sys_Error` too: the C `main` had no `setjmp` around them, so such a
  jump landed in an unset buffer.
- **`setjmp`/`longjmp` is not yet deleted.** The Rust-main frame path has no
  `setjmp` of its own -- `Host_Glue_FrameInner`'s is compiled out under
  `USE_RUST_PLATFORM`, where the frame is instead a `Host_Guard` whose status
  `quake_rs_host_frame` hands back to `Host_Frame`'s `Host_Reraise`, so the
  re-raise never crosses that Rust frame (rule 3; the Phase 9 review fix, and
  the reason `Host_Frame`'s `serverprofile` tail runs after a raised frame as
  it did in the C) -- but the trampoline pair, the two `jmp_buf`s and the
  `longjmp`s in `Host_Error`/`Host_EndGame`/`SCR_DrawGUI` stay while any C
  caller can still raise past a Rust frame (rule 3). Every remaining site is
  classified in `docs/rust-migration/setjmp-inventory.md`, generated by
  `scripts/setjmp_inventory.py`; `--check` fails on an unclassified site or a
  stale document, so the inventory cannot silently grow. The C-oracle copies
  (`host.c`, `gl_screen.c`) go with the Phase 9 soak exit; the `host_glue.c`
  / `gl_screen_glue.c` sites convert at Phase 10 (`quakedef.h` holds only
  the `Host_Guard`/`Host_Reraise` declarations, no `jmp_buf`), where
  `screen_error` becomes an error path of the render frame function as the
  end state says.
