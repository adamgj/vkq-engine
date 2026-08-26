//! `MSG_Write*` / `MSG_Read*` port (Quake/net_msg.c, split from common.c).
//! Bit-exact transliteration, including the C original's quirks:
//!
//! * the release build performs **silent truncation** in WriteChar/Byte/Short
//!   (the range Host_Errors are DEBUG-only -- mirrored by the `engine-debug`
//!   cargo feature, which Meson sets alongside `-D_DEBUG`);
//! * the angle byte path is asymmetric (write `& 255` unsigned, read as
//!   signed char);
//! * `MSG_ReadFloat`/`MSG_ReadDouble` have **no bounds check**: they read
//!   past `cursize` into the allocated buffer without setting badread;
//! * `MSG_ReadUInt64` shifts an `int` by up to 56 bits -- UB that every
//!   supported compiler resolves as a 31-masked shift, with the signed
//!   result sign-extended into the u64 (see `read_uint64`); the writer's
//!   matching UB is stable everywhere except the ten-byte lead form, whose
//!   accepted divergence is spelled out in `write_uint64`.
//!
//! Reader state (`msg_readcount`/`msg_badread`/the `net_message` buffer) is
//! ambient C state in the engine; here it is an explicit `MsgReader` and the
//! capi shims marshal the globals (ADR-007 net row).

use crate::protocol::{
    PEXT2_REPLACEMENTDELTAS, PRFL_24BITCOORD, PRFL_FLOATANGLE, PRFL_FLOATCOORD, PRFL_INT32COORD,
    PRFL_SHORTANGLE,
};
use crate::sizebuf::{SizeBuf, WireError};

/// COMPAT (ADR-010): C's float->int conversion is UB for NaN/out-of-range,
/// and the two supported architectures resolve it differently at runtime:
/// x86-64's cvttss2si/cvttsd2si produce INT_MIN (the "integer indefinite"
/// value, NaN included), arm64's fcvtzs saturates (NaN -> 0) -- which is
/// exactly what Rust's `as` does. The engine's C build therefore already
/// differs across platforms here; this helper reproduces each platform's C
/// behavior so C-vs-Rust parity holds per platform. Reachable only with
/// out-of-domain inputs (e.g. NaN viewangles from buggy QC).
#[inline]
fn c_cast_i32(x: f64) -> i32 {
    if cfg!(target_arch = "x86_64") {
        if !(-2147483648.0..2147483648.0).contains(&x) {
            i32::MIN
        } else {
            x as i32
        }
    } else {
        x as i32
    }
}

/// `Q_rint` (q_minmax.h) applied to a double argument: the macro adds the
/// double literal 0.5, so every call site promotes to double before the
/// truncating cast (see `c_cast_i32` for the out-of-domain behavior).
#[inline]
fn q_rint(x: f64) -> i32 {
    if x > 0.0 {
        c_cast_i32(x + 0.5)
    } else {
        c_cast_i32(x - 0.5)
    }
}

#[cfg(feature = "engine-debug")]
macro_rules! debug_range_check {
    ($cond:expr, $msg:literal) => {
        if $cond {
            return Err(WireError::RangeError($msg));
        }
    };
}
#[cfg(not(feature = "engine-debug"))]
macro_rules! debug_range_check {
    ($cond:expr, $msg:literal) => {};
}

//
// writing functions
//

pub fn write_char(sb: &mut SizeBuf<'_>, c: i32) -> Result<(), WireError> {
    debug_range_check!(!(-128..=127).contains(&c), "MSG_WriteChar: range error");
    let at = sb.get_space(1)?;
    sb.data[at] = c as u8;
    Ok(())
}

pub fn write_byte(sb: &mut SizeBuf<'_>, c: i32) -> Result<(), WireError> {
    debug_range_check!(!(0..=255).contains(&c), "MSG_WriteByte: range error");
    let at = sb.get_space(1)?;
    sb.data[at] = c as u8;
    Ok(())
}

pub fn write_short(sb: &mut SizeBuf<'_>, c: i32) -> Result<(), WireError> {
    debug_range_check!(
        c < i16::MIN as i32 || c > u16::MAX as i32,
        "MSG_WriteShort: range error"
    );
    let at = sb.get_space(2)?;
    sb.data[at] = (c & 0xff) as u8;
    sb.data[at + 1] = (c >> 8) as u8;
    Ok(())
}

pub fn write_long(sb: &mut SizeBuf<'_>, c: i32) -> Result<(), WireError> {
    let at = sb.get_space(4)?;
    sb.data[at] = (c & 0xff) as u8;
    sb.data[at + 1] = ((c >> 8) & 0xff) as u8;
    sb.data[at + 2] = ((c >> 16) & 0xff) as u8;
    sb.data[at + 3] = (c >> 24) as u8;
    Ok(())
}

/// 0*, 10*+1, 110*+2 ... lead-byte length prefix, up to 0xff + 8 continuation
/// bytes (net_msg.c MSG_WriteUInt64)
pub fn write_uint64(sb: &mut SizeBuf<'_>, c: u64) -> Result<(), WireError> {
    let mut b: u32 = 0;
    let mut l: u64 = 128;
    while c > l.wrapping_sub(1) {
        b += 1;
        l = l.wrapping_shl(7); // 8 bits gained per byte, one spent on length
    }
    let at = sb.get_space(1 + b as i32)?;
    // COMPAT: for b >= 8 (c >= 2^56; b == 9 is reachable via
    // MSG_WriteInt64 extremes) the C shift amounts leave [0,31]/[0,63] --
    // UB the supported targets' variable-shift instructions resolve by
    // masking, which wrapping_shl/shr reproduce. At b == 8 the UB only
    // feeds the lead byte, which is OR'd with 0xff, so C and Rust agree
    // whatever the compiler does. At b == 9 (c >= 2^63) it lands in the
    // first continuation byte and C has no stable answer: clang emits the
    // masked shift at -O0/-O1 (= this port) but folds `c >> 64` to 0 at
    // -O2/-O3, so an optimized C build writes 0 there where this writes
    // `c & 0xff`. ACCEPTED DIVERGENCE -- see quake-ctest's
    // `uint64_ub_domain_variants`. Unreachable from the engine itself: only
    // the QC `WriteUInt64`/`WriteInt64` builtins (pr_ext.c) can reach it,
    // and the ten-byte form does not round-trip through MSG_ReadUInt64
    // anyway (the reader consumes nine bytes).
    let lead = ((0xffu32.wrapping_shl((8i32.wrapping_sub(b as i32)) as u32) as u64)
        | c.wrapping_shr(b * 8)) as u8;
    let mut out = at;
    sb.data[out] = lead;
    out += 1;
    while b > 0 {
        b -= 1;
        sb.data[out] = c.wrapping_shr(b * 8) as u8;
        out += 1;
    }
    Ok(())
}

/// sign bit moved to the low bit to avoid sign extension (net_msg.c)
pub fn write_int64(sb: &mut SizeBuf<'_>, c: i64) -> Result<(), WireError> {
    if c < 0 {
        write_uint64(sb, (((-1 - c) as u64) << 1) | 1)
    } else {
        write_uint64(sb, (c as u64).wrapping_shl(1))
    }
}

pub fn write_float(sb: &mut SizeBuf<'_>, f: f32) -> Result<(), WireError> {
    sb.write(&f.to_le_bytes())
}

pub fn write_double(sb: &mut SizeBuf<'_>, f: f64) -> Result<(), WireError> {
    let at = sb.get_space(8)?;
    sb.data[at..at + 8].copy_from_slice(&f.to_le_bytes());
    Ok(())
}

/// `s` is the string bytes WITHOUT the terminator; `None` mirrors a NULL
/// `const char *` (C writes just the terminator)
pub fn write_string(sb: &mut SizeBuf<'_>, s: Option<&[u8]>) -> Result<(), WireError> {
    match s {
        None => sb.write(&[0u8]),
        Some(bytes) => {
            let at = sb.get_space(bytes.len() as i32 + 1)?;
            sb.data[at..at + bytes.len()].copy_from_slice(bytes);
            sb.data[at + bytes.len()] = 0;
            Ok(())
        }
    }
}

pub fn write_string_unterminated(sb: &mut SizeBuf<'_>, s: &[u8]) -> Result<(), WireError> {
    sb.write(s)
}

/// 13.3 fixed point, max range +-4096 (the f*8 multiply is C float
/// arithmetic before Q_rint's double rounding)
pub fn write_coord16(sb: &mut SizeBuf<'_>, f: f32) -> Result<(), WireError> {
    write_short(sb, q_rint((f * 8.0f32) as f64))
}

/// 16.8 fixed point, max range +-32768. COMPAT: the fraction byte is
/// `(int)(f * 255) % 255` exactly as in C -- including the odd `% 255` (not
/// 256) and the truncating cast, negative results reaching write_byte's
/// silent truncation.
pub fn write_coord24(sb: &mut SizeBuf<'_>, f: f32) -> Result<(), WireError> {
    write_short(sb, c_cast_i32(f as f64))?;
    write_byte(sb, c_cast_i32((f * 255.0f32) as f64) % 255)
}

pub fn write_coord32f(sb: &mut SizeBuf<'_>, f: f32) -> Result<(), WireError> {
    write_float(sb, f)
}

pub fn write_coord(sb: &mut SizeBuf<'_>, f: f32, flags: u32) -> Result<(), WireError> {
    if flags & PRFL_FLOATCOORD != 0 {
        write_float(sb, f)
    } else if flags & PRFL_INT32COORD != 0 {
        write_long(sb, q_rint((f * 16.0f32) as f64))
    } else if flags & PRFL_24BITCOORD != 0 {
        write_coord24(sb, f)
    } else {
        write_coord16(sb, f)
    }
}

pub fn write_angle(sb: &mut SizeBuf<'_>, f: f32, flags: u32) -> Result<(), WireError> {
    if flags & PRFL_FLOATANGLE != 0 {
        write_float(sb, f)
    } else if flags & PRFL_SHORTANGLE != 0 {
        write_short(sb, q_rint(f as f64 * 65536.0 / 360.0) & 65535)
    } else {
        write_byte(sb, q_rint(f as f64 * 256.0 / 360.0) & 255)
    }
}

pub fn write_angle16(sb: &mut SizeBuf<'_>, f: f32, flags: u32) -> Result<(), WireError> {
    if flags & PRFL_FLOATANGLE != 0 {
        write_float(sb, f)
    } else {
        write_short(sb, q_rint(f as f64 * 65536.0 / 360.0) & 65535)
    }
}

/// PEXT2_REPLACEMENTDELTAS: entnums > 0x7fff go as high short + low byte
pub fn write_entity(sb: &mut SizeBuf<'_>, entnum: u32, pext2: u32) -> Result<(), WireError> {
    if entnum > 0x7fff && (pext2 & PEXT2_REPLACEMENTDELTAS) != 0 {
        write_short(sb, (0x8000 | (entnum >> 8)) as i32)?;
        write_byte(sb, (entnum & 0xff) as i32)
    } else {
        write_short(sb, entnum as i32)
    }
}

//
// reading functions
//

/// The reader over a received message. `data` is the FULL allocated buffer
/// (`net_message.maxsize` bytes in the engine), not just the valid prefix:
/// `read_float`/`read_double` deliberately read past `cursize` like the C
/// original. Reads past the allocation itself (UB in C, unreachable in the
/// engine) yield zero bytes here.
#[derive(Debug)]
pub struct MsgReader<'a> {
    pub data: &'a [u8],
    pub cursize: i32,
    pub readcount: i32,
    pub badread: bool,
    /// number of underrun events, mirrored into `harness_badread_count`
    pub badread_events: u32,
}

impl<'a> MsgReader<'a> {
    /// `MSG_BeginReading` over a message occupying `data[..cursize]`
    pub fn begin(data: &'a [u8], cursize: i32) -> MsgReader<'a> {
        MsgReader {
            data,
            cursize,
            readcount: 0,
            badread: false,
            badread_events: 0,
        }
    }

    #[inline]
    fn byte_at(&self, idx: i32) -> u8 {
        self.data.get(idx as usize).copied().unwrap_or(0)
    }

    fn underrun(&mut self) -> i32 {
        self.badread = true;
        self.badread_events += 1;
        -1
    }

    pub fn read_char(&mut self) -> i32 {
        if self.readcount + 1 > self.cursize {
            return self.underrun();
        }
        let c = self.byte_at(self.readcount) as i8 as i32;
        self.readcount += 1;
        c
    }

    pub fn read_byte(&mut self) -> i32 {
        if self.readcount + 1 > self.cursize {
            return self.underrun();
        }
        let c = self.byte_at(self.readcount) as i32;
        self.readcount += 1;
        c
    }

    pub fn read_short(&mut self) -> i32 {
        if self.readcount + 2 > self.cursize {
            return self.underrun();
        }
        let c = (self.byte_at(self.readcount) as i32
            + ((self.byte_at(self.readcount + 1) as i32) << 8)) as i16 as i32;
        self.readcount += 2;
        c
    }

    pub fn read_long(&mut self) -> i32 {
        if self.readcount + 4 > self.cursize {
            return self.underrun();
        }
        let c = (self.byte_at(self.readcount) as u32)
            | ((self.byte_at(self.readcount + 1) as u32) << 8)
            | ((self.byte_at(self.readcount + 2) as u32) << 16)
            | ((self.byte_at(self.readcount + 3) as u32) << 24);
        self.readcount += 4;
        c as i32
    }

    /// COMPAT (net_msg.c MSG_ReadUInt64): the C `r |= MSG_ReadByte() << (b*8)`
    /// shifts an `int` by up to 56 bits -- UB that clang/gcc/MSVC on every
    /// supported target compile as a 31-masked shift, whose signed result is
    /// then sign-extended into the unsigned long long. Reproduced exactly;
    /// values needing >= 4 continuation bytes therefore do NOT round-trip,
    /// on either side. Do not "fix" without a cross-version protocol plan.
    pub fn read_uint64(&mut self) -> u64 {
        let mut l: u8 = 0x80;
        let mut b: u32 = 0;
        let mut v = self.read_byte() as u8; // -1 truncates to 0xff like C
        while l != 0 && (v & l) != 0 {
            v -= l;
            l >>= 1;
            b += 1;
        }
        let mut r = ((v as i32).wrapping_shl(b * 8) as i64) as u64;
        while b > 0 {
            b -= 1;
            r |= ((self.read_byte()).wrapping_shl(b * 8) as i64) as u64;
        }
        r
    }

    pub fn read_int64(&mut self) -> i64 {
        let c = self.read_uint64();
        if c & 1 != 0 {
            -1 - ((c >> 1) as i64)
        } else {
            (c >> 1) as i64
        }
    }

    /// COMPAT: no bounds check, no badread -- reads whatever sits in the
    /// allocated buffer past cursize, exactly like the C original
    pub fn read_float(&mut self) -> f32 {
        let b = [
            self.byte_at(self.readcount),
            self.byte_at(self.readcount + 1),
            self.byte_at(self.readcount + 2),
            self.byte_at(self.readcount + 3),
        ];
        self.readcount += 4;
        f32::from_le_bytes(b)
    }

    /// COMPAT: no bounds check, like `read_float` -- and the C original is
    /// declared `float MSG_ReadDouble (void)`: it assembles the 8-byte
    /// double, then narrows it to float at the return. Mirrored exactly.
    pub fn read_double(&mut self) -> f32 {
        let mut b = [0u8; 8];
        for (i, o) in b.iter_mut().enumerate() {
            *o = self.byte_at(self.readcount + i as i32);
        }
        self.readcount += 8;
        f64::from_le_bytes(b) as f32
    }

    /// `MSG_ReadString`: stops at NUL or underrun, capped at 2047 bytes like
    /// the C static buffer. Returns the bytes without the terminator.
    pub fn read_string(&mut self) -> Vec<u8> {
        let mut s = Vec::new();
        while s.len() < 2047 {
            let c = self.read_byte();
            if c == -1 || c == 0 {
                break;
            }
            s.push(c as u8);
        }
        s
    }

    pub fn read_coord16(&mut self) -> f32 {
        (self.read_short() as f64 * (1.0 / 8.0)) as f32
    }

    /// COMPAT: the C `MSG_ReadShort () + MSG_ReadByte () * (1.0 / 255)` has
    /// unspecified evaluation order; every supported compiler evaluates the
    /// short first, which this port fixes in stone (differentially verified)
    pub fn read_coord24(&mut self) -> f32 {
        let s = self.read_short();
        let b = self.read_byte();
        (s as f64 + b as f64 * (1.0 / 255.0)) as f32
    }

    pub fn read_coord32f(&mut self) -> f32 {
        self.read_float()
    }

    pub fn read_coord(&mut self, flags: u32) -> f32 {
        if flags & PRFL_FLOATCOORD != 0 {
            self.read_float()
        } else if flags & PRFL_INT32COORD != 0 {
            (self.read_long() as f64 * (1.0 / 16.0)) as f32
        } else if flags & PRFL_24BITCOORD != 0 {
            self.read_coord24()
        } else {
            self.read_coord16()
        }
    }

    pub fn read_angle(&mut self, flags: u32) -> f32 {
        if flags & PRFL_FLOATANGLE != 0 {
            self.read_float()
        } else if flags & PRFL_SHORTANGLE != 0 {
            (self.read_short() as f64 * (360.0 / 65536.0)) as f32
        } else {
            // asymmetric with the write side: read as SIGNED char
            (self.read_char() as f64 * (360.0 / 256.0)) as f32
        }
    }

    pub fn read_angle16(&mut self, flags: u32) -> f32 {
        if flags & PRFL_FLOATANGLE != 0 {
            self.read_float()
        } else {
            (self.read_short() as f64 * (360.0 / 65536.0)) as f32
        }
    }

    pub fn read_entity(&mut self, pext2: u32) -> u32 {
        let mut e = (self.read_short() as u16) as u32;
        if pext2 & PEXT2_REPLACEMENTDELTAS != 0 && e & 0x8000 != 0 {
            e = (e & 0x7fff) << 8;
            e |= self.read_byte() as u32;
        }
        e
    }
}
