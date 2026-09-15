//! `HostError` (ADR-009): the typed outcome of a guarded host call.
//!
//! Rust migration Phase 9 M7. While the C oracle is built from the same
//! tree the C side still raises with `longjmp`, and `Host_Guard`
//! (`Quake/host.c`, `quakedef.h:472-477`) reports the jump it caught as an
//! `int` status. This type is what the Rust host loop
//! (`quake-platform::main_sdl`) consumes instead of that raw status. It
//! carries no message: `Host_Error`/`Host_EndGame` print theirs before they
//! jump, and ADR-009's post-guard invariant forbids reading the state the
//! raise was about. ADR-009's end-state sketch (`Error(String)` /
//! `EndGame(String)`) is reached when the raise itself moves to Rust
//! (Phase 10); `docs/rust-migration/setjmp-inventory.md` lists what is left.

use core::ffi::c_int;
use core::fmt;

/// `HOST_GUARD_OK` (`quakedef.h:475`): `Host_Guard` returned normally.
pub const GUARD_OK: c_int = 0;
/// `HOST_GUARD_ABORTSERVER` (`quakedef.h:476`).
pub const GUARD_ABORTSERVER: c_int = 1;
/// `HOST_GUARD_SCREEN_ERROR` (`quakedef.h:477`).
pub const GUARD_SCREEN_ERROR: c_int = 2;

/// A raise that `Host_Guard` caught on the C side of a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostError {
    /// `longjmp (host_abortserver, 1)`: `Host_Error` or `Host_EndGame`. The
    /// server is already shut down and the client disconnected.
    AbortServer,
    /// `longjmp (screen_error, 1)`: `Host_Error` while CSQC was drawing the
    /// HUD (`in_update_screen`). In the C this jump only ever had a target
    /// inside `SCR_DrawGUI`; one that reaches the host loop was raised with
    /// no such frame on the stack.
    ScreenError,
    /// A status `Host_Guard` does not define: a C-side contract violation.
    Unknown(c_int),
}

impl HostError {
    /// The `Host_Guard` status as the `Result` the loop consumes.
    pub fn from_guard_status(status: c_int) -> Result<(), HostError> {
        match status {
            GUARD_OK => Ok(()),
            GUARD_ABORTSERVER => Err(HostError::AbortServer),
            GUARD_SCREEN_ERROR => Err(HostError::ScreenError),
            other => Err(HostError::Unknown(other)),
        }
    }

    /// The status `Host_Reraise` would take to re-issue this jump.
    pub fn guard_status(self) -> c_int {
        match self {
            HostError::AbortServer => GUARD_ABORTSERVER,
            HostError::ScreenError => GUARD_SCREEN_ERROR,
            HostError::Unknown(status) => status,
        }
    }
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostError::AbortServer => f.write_str("Host_Error (host_abortserver)"),
            HostError::ScreenError => f.write_str("Host_Error (screen_error)"),
            HostError::Unknown(status) => write!(f, "Host_Guard status {status}"),
        }
    }
}

impl std::error::Error for HostError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_statuses_round_trip() {
        assert_eq!(HostError::from_guard_status(0), Ok(()));
        assert_eq!(HostError::from_guard_status(1), Err(HostError::AbortServer));
        assert_eq!(HostError::from_guard_status(2), Err(HostError::ScreenError));
        assert_eq!(HostError::from_guard_status(7), Err(HostError::Unknown(7)));
        for status in [1, 2, 7, -1] {
            let err = HostError::from_guard_status(status).unwrap_err();
            assert_eq!(err.guard_status(), status);
        }
    }

    #[test]
    fn display_names_the_jump() {
        assert_eq!(
            HostError::AbortServer.to_string(),
            "Host_Error (host_abortserver)"
        );
        assert_eq!(
            HostError::ScreenError.to_string(),
            "Host_Error (screen_error)"
        );
        assert_eq!(HostError::Unknown(3).to_string(), "Host_Guard status 3");
    }
}
