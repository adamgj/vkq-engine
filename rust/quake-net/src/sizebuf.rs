//! `SZ_*` port (Quake/net_msg.c, split from common.c). Bit-exact transliteration:
//! overflow semantics, silent release-build truncations, and error routing all
//! mirror the C original. Errors that C raises via `Host_Error`/`Sys_Error`
//! surface as `Err`; the M3 capi glue re-raises them in a C frame (ADR-009).

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

/// Owned mirror of `sizebuf_t` semantics for the pure crate; the buffer is
/// allocated at `maxsize` like `SZ_Alloc` and `cursize` tracks the write
/// position. The C-storage-backed variant used by the capi shims runs the
/// same `get_space_raw` core.
#[derive(Debug)]
pub struct SizeBuf {
    pub allowoverflow: bool,
    pub overflowed: bool,
    pub data: Vec<u8>,
    pub cursize: i32,
}

/// The `SZ_GetSpace` core over raw parts: returns the write offset, applying
/// the C overflow rules (clear + set overflowed when allowed). Shared by the
/// owned `SizeBuf` and the C-backed shim path.
pub fn get_space_raw(
    allowoverflow: bool,
    overflowed: &mut bool,
    cursize: &mut i32,
    maxsize: i32,
    length: i32,
) -> Result<i32, WireError> {
    if *cursize + length > maxsize {
        if !allowoverflow {
            return Err(WireError::Overflow);
        }
        if length > maxsize {
            return Err(WireError::OversizeWrite);
        }
        // Con_Printf ("SZ_GetSpace: overflow\n") is diagnostics-only; the
        // shim layer prints. SZ_Clear also resets overflowed, then it is
        // re-set -- net effect below.
        *cursize = 0;
        *overflowed = true;
    }
    let at = *cursize;
    *cursize += length;
    Ok(at)
}

impl SizeBuf {
    /// `SZ_Alloc`: minimum size 256, zero-filled (Mem_Alloc zeroes)
    pub fn alloc(startsize: i32) -> SizeBuf {
        let startsize = startsize.max(256);
        SizeBuf {
            allowoverflow: false,
            overflowed: false,
            data: vec![0u8; startsize as usize],
            cursize: 0,
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
        let maxsize = self.maxsize();
        let at = get_space_raw(
            self.allowoverflow,
            &mut self.overflowed,
            &mut self.cursize,
            maxsize,
            length,
        )?;
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
