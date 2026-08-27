# ADR-006: Edict/progs memory as an untyped arena with typed accessors

**Status:** Accepted
**Date:** 2026-08-16
**Tags:** (compat exception)

## Context

Edicts cannot be Rust structs, because their layout is a **runtime ABI** decided when `progs.dat` loads:

- `edict_size = progs->entityfields*4 + sizeof(edict_t) - sizeof(entvars_t)`, rounded up to pointer alignment; mod-defined fields extend past `entvars_t`.
- `PR_MergeEngineFieldDefs` **mutates** `entityfields` at load, appending engine fields (`alpha`, `scale`, `emiteffectnum`, `traileffectnum`, `tag_entity`, `tag_index`, `modelflags`, `colormod` + synthesized `_x/_y/_z` defs) — changing layout, the fielddef table, and savegame output.
- `EDICT_TO_PROG` yields **byte offsets** into the edict array; these values are what QuakeC sees in entity variables and flow into savegames and the wire protocol.
- Debug builds add three extra header fields — layout differs per build profile.
- All QC field access is offset arithmetic (`(float*)&ed->v + ofs`); `string_t`/`func_t`/entity values are raw `i32` including negative engine-string indices; `PR_EnterFunction`/`PR_LeaveFunction` copy locals to `localstack` as raw ints.

A fully type-safe representation is impossible without changing observable behavior. This is the project's flagship documented exception to full type safety.

## Decision

- `EdictArena` owns a single aligned byte buffer (`Box<[u32]>`-backed) holding all edicts at `edict_size` stride; `EdictId(u32)` newtype indices replace raw pointers in Rust code.
- Typed accessors are generated from the **merged** fielddef table at load: `arena.f32(id, fld)`, `arena.vec3(id, fld)`, and an `EntVars` getter/setter view for engine-known fields. Field handles are validated-once `FieldOfs` newtypes.
- No reference into the arena escapes a VM step; the interpreter borrows the arena `&mut` for the duration of `PR_ExecuteProgram`.
- Raw `i32` semantics for string/function/entity slots, negative engine strings, byte-offset `EDICT_TO_PROG`, the `STRINGTEMP` 1024×1024 ring (with observable wraparound), the localstack raw-copy behavior, and the FIFO `ED_Alloc` free-list (entity numbering is observable) are preserved exactly.
- Interpreter arithmetic is transliterated: `OP_DIV_F` raw division (no zero guard), C float→int truncation in `OP_BITAND`/`OP_BITOR`, C comparison semantics.
- Unsafe is confined to the arena's indexing internals; the public API is safe (`// SAFETY:` per ADR-004).

## Consequences

- QC-observable behavior, savegames, and wire values are preserved by construction; the trace oracle (ADR-019) verifies it.
- The arena API is less ergonomic than idiomatic structs; engine-known fields get typed sugar via `EntVars` to compensate.
- Bounds and alignment are checked at arena construction (progs load), making per-access cost negligible; hot-path accessors may use unchecked indexing after validation, with SAFETY comments.

## Amended (Phase 6 M1, 2026-08-27)

Two clarifications forced by the transition shape, neither of which changes
the end state described above.

**1. The arena is a *view* before it is an owner.** This ADR describes
`EdictArena` as owning a `Box<[u32]>`-backed buffer. That is the post-loader
state. `qcvm->edicts` is allocated and freed by C (`PR_LoadProgs` /
`PR_ClearProgs`) until the loader itself flips, and the `qcvm_t` holding it is
C storage embedded in `sv`/`cl` for the whole of Phase 6 (ADR-007). So the
arena has two constructors: a borrowing one over a caller-supplied base
pointer and stride (the engine path), and an owning one over a Rust
allocation (the differential-test and fuzz path). Both enforce the same
construction-time bounds and alignment validation, so the unchecked hot-path
indexing argument is unchanged.

**2. "No reference escapes a VM step" is tightened to "no reference lives
across a builtin dispatch."** The original wording assumed the interpreter
could hold `&mut EdictArena` for the duration of `PR_ExecuteProgram`. It
cannot: `OP_CALL*` dispatches into `qcvm->builtins[]`, those builtins are C
for most of Phase 6, and C builtins call `PR_ExecuteProgram` re-entrantly
(`pr_exec.c`). A live Rust `&mut` across that boundary would be aliasing UB
the moment the nested call touched the same arena. The interpreter therefore
re-derives its base pointers per step rather than holding a borrow, and the
re-entrancy shape is modelled under Miri. This is the concrete mechanism
behind ADR-008's "a Rust builtin must never re-enter the VM through the
ambient path while holding `&mut QcVm`" care point.
