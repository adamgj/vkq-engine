//! COM_* searchpath/pak/gamedir layer (Phase 2); WAD2 parse logic (Phase 1)
//!
//! Rust migration: Phase 1 populates the pure wad.c decision logic here
//! (PLAN.md §5 assigns WAD2 to this crate); the searchpath/pak layer follows
//! in Phase 2.

#![forbid(unsafe_code)] // ADR-004: pure crate

pub mod wad;
