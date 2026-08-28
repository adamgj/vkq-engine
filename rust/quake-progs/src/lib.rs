//! The QuakeC virtual machine — progs loader, edict arena, interpreter,
//! `ED_Write`/`ED_Parse*`, and builtins (Rust migration Phase 6,
//! `docs/rust-migration/ROADMAP.md`).
//!
//! This crate is deliberately free of `quake-c-sys`: everything the VM needs
//! from the engine is either mirrored in `quake-types` or threaded in by the
//! caller, so the loader and parsers stay fuzzable and the differential suites
//! can drive them over Rust-owned memory. The C-boundary shims live in
//! `quake-capi`.
//!
//! # Unsafe posture (ADR-004)
//!
//! Edicts cannot be Rust structs — their layout is a runtime ABI fixed when
//! `progs.dat` loads (ADR-006) — so the arena indexes an untyped buffer. The
//! progs image itself is the same shape of problem: its interior layout comes
//! from the file header at runtime, so [`image`] is a second island. Unsafe is
//! confined to those two modules; the rest of the crate is
//! `deny(unsafe_code)`.
//!
//! # Re-entrancy (ADR-006 Phase 6 amendment, ADR-008)
//!
//! `PR_ExecuteProgram` dispatches builtins, builtins are C during most of this
//! phase, and C builtins call `PR_ExecuteProgram` again. **No Rust reference
//! into the qcvm or the edict arena may live across a builtin dispatch** — the
//! interpreter re-derives its base pointers per step instead.

#![deny(unsafe_code)]

pub mod alloc;
pub mod arena;
pub mod builtins;
pub mod exec;
pub mod ext;
pub mod image;
pub mod load;
pub mod parse;
pub mod save;
