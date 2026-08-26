//! `SZ_*` port (Quake/net_msg.c, split from common.c). Bit-exact transliteration:
//! overflow semantics, silent release-build truncations, and error routing all
//! mirror the C original. Errors that C raises via `Host_Error`/`Sys_Error`
//! surface as `Err`; the M3 capi glue re-raises them in a C frame (ADR-009).
//!
//! `SizeBuf` borrows its backing store, so the capi shims can wrap the
//! engine's own `sizebuf_t` allocation (data/maxsize from the C struct) and
//! write `cursize`/`overflowed` back after each call; pure tests wrap a Vec.

/// Error paths of `SZ_GetSpace` (net_msg.c). C maps `Overflow` to
/// `Host_Error` and `OversizeWrite` to `Sys_Error`; `RangeError` is the
/// debug-only writer range check (engine-debug feature = C's DEBUG/_DEBUG).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireError {
    /// "SZ_GetSpace: overflow without allowoverflow set"
    Overflow,
    /// "SZ_GetSpace: %i is > full buffer size"
    OversizeWrite,
    /// debug-only MSG_Write{Char,Byte,Short} range check message
    RangeError(&'static str),
}

/// Borrowed view of a `sizebuf_t`: `data` is the full allocation (`maxsize`
/// bytes), `cursize` the write position.
#[derive(Debug)]
pub struct SizeBuf<'a> {
    pub allowoverflow: bool,
    pub overflowed: bool,
    pub data: &'a mut [u8],
    pub cursize: i32,
    /// times the allowed-overflow path fired, so the shim can emit C's
    /// `Con_Printf ("SZ_GetSpace: overflow\n")` per event
    pub overflow_events: u32,
}

impl<'a> SizeBuf<'a> {
    /// fresh buffer over `data` (like just-`SZ_Alloc`ed storage)
    pub fn new(data: &'a mut [u8]) -> SizeBuf<'a> {
        SizeBuf {
            allowoverflow: false,
            overflowed: false,
            data,
            cursize: 0,
            overflow_events: 0,
        }
    }

    pub fn maxsize(&self) -> i32 {
        self.data.len() as i32
    }

    /// `SZ_Clear`
    pub fn clear(&mut self) {
        self.cursize = 0;
        self.overflowed = false;
    }

    /// `SZ_GetSpace`: reserves `length` bytes, returning their offset
    pub fn get_space(&mut self, length: i32) -> Result<usize, WireError> {
        if self.cursize + length > self.maxsize() {
            if !self.allowoverflow {
                return Err(WireError::Overflow);
            }
            if length > self.maxsize() {
                return Err(WireError::OversizeWrite);
            }
            // C: Con_Printf + SZ_Clear (resets overflowed), then re-set
            self.overflow_events += 1;
            self.cursize = 0;
            self.overflowed = true;
        }
        let at = self.cursize;
        self.cursize += length;
        Ok(at as usize)
    }

    /// `SZ_Write`
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), WireError> {
        let at = self.get_space(bytes.len() as i32)?;
        self.data[at..at + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }

    /// `SZ_Print`: strcats `s` (no NUL in the slice) plus a terminator onto
    /// the buffer, overwriting an existing trailing NUL.
    ///
    /// COMPAT: with `cursize == 0` the C original reads `data[-1]` (heap
    /// garbage before the allocation) to decide the branch -- undefined
    /// behavior it survives by luck. This port takes the "no trailing 0"
    /// append branch in that case; no engine call site hits it.
    pub fn print(&mut self, s: &[u8]) -> Result<(), WireError> {
        let len = s.len() as i32 + 1;
        let trailing_nul = self.cursize > 0 && self.data[self.cursize as usize - 1] == 0;
        if !trailing_nul {
            let at = self.get_space(len)?;
            self.data[at..at + s.len()].copy_from_slice(s);
            self.data[at + s.len()] = 0;
        } else {
            // COMPAT: if the overflow path inside get_space just reset
            // cursize to 0, C would write at data[-1] (UB); clamp to 0
            let at = self.get_space(len - 1)?.saturating_sub(1);
            self.data[at..at + s.len()].copy_from_slice(s);
            self.data[at + s.len()] = 0;
        }
        Ok(())
    }

    pub fn written(&self) -> &[u8] {
        &self.data[..self.cursize as usize]
    }
}
