//! SDL2+SDL3 platform glue, input, video, sys/pl layers (ADR-017).
//!
//! Phase 4 M9 populated the SDL3 audio backend. Phase 9 adds the rest of
//! the platform layer under the `platform` feature, with `sdl2`/`sdl3`
//! selecting the SDL generation the C engine was configured for. The SDL2
//! *audio* backend stays C (ADR-017); only the event/input/sys layers carry
//! an SDL2 arm.

#[cfg(all(feature = "sdl2", feature = "sdl3"))]
compile_error!("quake-platform: the `sdl2` and `sdl3` features are mutually exclusive (ADR-017)");

#[cfg(all(feature = "platform", not(any(feature = "sdl2", feature = "sdl3"))))]
compile_error!("quake-platform: the `platform` feature needs exactly one of `sdl2`/`sdl3`");

#[cfg(feature = "sdl3")]
pub mod snd_sdl3;

/// Phase 9 M3: `in_sdl.c`/`in_sdl2.c`/`in_sdl3.c`.
#[cfg(feature = "platform")]
pub mod input;

/// Phase 9 M4: `pl_win.c`/`pl_linux.c`.
#[cfg(feature = "platform")]
pub mod pl;

/// Phase 9 M5: `sys_sdl.c`/`sys_sdl_win.c`/`sys_sdl_unix.c`.
#[cfg(feature = "platform")]
pub mod sys;

/// Phase 9 M6: `main_sdl.c`.
#[cfg(feature = "platform")]
pub mod main_sdl;
