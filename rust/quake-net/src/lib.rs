//! MSG_* readers/writers, protocols 15/666/999+PRFL+PEXT, net drivers, demo IO
//!
//! Rust migration Phase 5 (ROADMAP.md): the networking wire layer. Protocol
//! *logic* (cl_parse.c svc dispatch, sv_main.c delta writer) stays C until
//! Phase 7; this crate owns byte-level encode/decode, the drivers, and demo
//! file IO.

// ADR-004 (Phase 5 amendment): deny rather than forbid -- the M7 UDP
// landriver adds one #[allow(unsafe_code)] `sys` module for the socket
// syscall boundary; everything else in this crate stays unsafe-free.
#![deny(unsafe_code)]

pub mod demo;
pub mod msg;
pub mod protocol;
pub mod sizebuf;
