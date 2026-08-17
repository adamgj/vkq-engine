# ADR-008: Ambient `qcvm` at the C boundary; explicit `&mut QcVm` internally

**Status:** Accepted
**Date:** 2026-08-16
**Tags:** (compat exception)

## Context

The progs API uses a global implicit receiver: `qcvm_t *qcvm`, swapped by `PR_SwitchQCVM()` between the two live VMs (`sv.qcvm` for server progs, `cl.qcvm` for CSQC), with an assertion that no VM is active when switching. Every field-access macro (`G_FLOAT`, `EDICT_NUM`, `PROG_TO_EDICT`, …) dereferences the global. `_Host_Frame` switches VMs around server frames and CSQC physics. This ambient pattern is engine-visible: builtins, savegame code, and host code all rely on "the current VM."

Idiomatic Rust would thread `&mut QcVm` explicitly — but the C boundary cannot change mid-migration, and the switch-discipline (including its assertion) is itself observable behavior worth preserving.

## Decision

- **Rust internals** thread `&mut QcVm` (and `&mut EdictArena`) explicitly. No Rust-internal ambient global.
- **The C boundary** keeps the ambient contract: while any C code calls progs functionality, the C global `qcvm` remains authoritative; Rust boundary shims resolve it exactly once per entry (`current_qcvm()` → `&mut QcVm`, with a SAFETY argument: the host frame's switch discipline guarantees exclusivity).
- After the host loop is Rust (Phase 9), `PR_SwitchQCVM` semantics are reproduced by an RAII **`QcVmGuard`**: acquiring a guard for `sv` or `cl` VM asserts none is active (same error as C), and host-loop code structure mirrors today's switch points exactly.
- The "no switch while active" assertion, the `PR_SwitchQCVM(NULL)` clear, and the ordering of VM switches in the frame (`CL_SendCmd` → sv frame under sv.qcvm → CSQC physics under cl.qcvm → …) are preserved as behavior.

## Consequences

- Pure-Rust call paths get compile-time receiver correctness; the ambient hazard is confined to boundary shims during transition and eliminated with the last C caller.
- The guard adds a tiny runtime check exactly where C had an assertion — no behavioral change.
- Care point: a Rust builtin must never re-enter the VM through the ambient path while holding `&mut QcVm`; the shim design (resolve once per entry) makes this structurally impossible.
