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
    /// Two COMPAT deviations, both confined to states where the C original
    /// has already left the allocation (undefined behavior it survives by
    /// luck) and neither reachable from an engine call site -- every
    /// `SZ_Print` caller passes a short string into a >= 256-byte buffer:
    ///
    /// * with `cursize == 0` C reads `data[-1]` to pick the branch; this
    ///   port takes the "no trailing 0" append branch instead;
    /// * in the trailing-NUL branch C writes starting at
    ///   `SZ_GetSpace (len - 1) - 1`, so when the allowed-overflow path
    ///   inside `get_space` has just reset `cursize` to 0 that start is
    ///   `data[-1]`. This port clamps the start to 0 and the tail to the
    ///   allocation, dropping the bytes C would have written outside it --
    ///   the alternative would be a Rust panic (process abort) standing in
    ///   for C's silent one-byte scribble, a louder change than the bug.
    pub fn print(&mut self, s: &[u8]) -> Result<(), WireError> {
        let len = s.len() as i32 + 1;
        let trailing_nul = self.cursize > 0 && self.data[self.cursize as usize - 1] == 0;
        if !trailing_nul {
            let at = self.get_space(len)?;
            self.data[at..at + s.len()].copy_from_slice(s);
            self.data[at + s.len()] = 0;
        } else {
            let at = self.get_space(len - 1)?.saturating_sub(1);
            // clamped per the COMPAT note above; in every reachable state
            // `end` is `at + s.len()` and the terminator lands in bounds
            let end = (at + s.len()).min(self.data.len());
            self.data[at..end].copy_from_slice(&s[..end - at]);
            if end < self.data.len() {
                self.data[end] = 0;
            }
        }
        Ok(())
    }

    pub fn written(&self) -> &[u8] {
        &self.data[..self.cursize as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression guard for the COMPAT clamp in `print`: the allowed-overflow
    /// reset state must not turn C's out-of-allocation scribble into a Rust
    /// panic. Shape: allowoverflow, a trailing NUL, and `s.len() == maxsize`,
    /// so `get_space` resets cursize to 0 and the naive terminator write
    /// would land one past the slice.
    #[test]
    fn print_overflow_reset_does_not_panic() {
        let mut store = vec![0u8; 256];
        let mut sb = SizeBuf::new(&mut store);
        sb.allowoverflow = true;
        // establish a trailing NUL and a non-zero cursize
        sb.write(b"hi\0").unwrap();
        assert!(sb.cursize > 0);

        let s = vec![b'x'; 256]; // == maxsize
        sb.print(&s).unwrap();

        assert!(sb.overflowed, "the allowed-overflow path should have fired");
        assert!(sb.cursize <= sb.maxsize());
    }

    /// The normal branch still terminates in place (no clamp interference)
    #[test]
    fn print_overwrites_trailing_nul() {
        let mut store = vec![0u8; 256];
        let mut sb = SizeBuf::new(&mut store);
        sb.write(b"ab\0").unwrap();
        sb.print(b"cd").unwrap();
        assert_eq!(sb.written(), b"abcd\0");
    }
}
