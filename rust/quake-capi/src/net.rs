//! Phase 5 networking wire-layer shims (quake-net).
//!
//! M1 scaffolding: the module exists so the `net` cargo feature and the Meson
//! `-Duse_rust_net` switch are wired end-to-end before any code flips; the
//! MSG_*/SZ_* exports land in M3, the driver entry points from M5 onward.
