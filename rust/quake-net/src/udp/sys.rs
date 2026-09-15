//! The ADR-004 unsafe island of the UDP landriver: one arm per C driver.
//! [`unix`] wraps the net_udp.c OS calls (socket2 + libc); [`windows`]
//! wraps the net_wins.c Winsock calls (windows-sys, Phase 9 M2). No engine
//! state in here; quake-capi owns that.

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;
