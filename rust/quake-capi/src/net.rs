//! Phase 5 networking wire-layer shims (quake-net).
//!
//! M3: the MSG_*/SZ_* layer. Readers and the error-free SZ entry points
//! export the exact C names; writers and the SZ functions that can
//! `Host_Error`/`Sys_Error` export as `quake_rs_*` status functions that
//! `Quake/net_msg_glue.c` wraps, re-raising in a C frame (ADR-009 -- a
//! longjmp must never unwind a Rust frame). The reader globals
//! (`net_message`, `msg_readcount`, `msg_badread`) stay C-owned (ADR-007
//! net row); this module marshals them around the pure `quake_net` core.

use core::ffi::{c_char, c_int, c_uint};

use quake_c_sys as c;
use quake_net::msg::{self, MsgReader};
use quake_net::sizebuf::{SizeBuf, WireError};

/// status codes shared with net_msg_glue.c (keep in sync with the glue)
const SZ_OK: c_int = 0;
const SZ_ERR_OVERFLOW: c_int = 1;
const SZ_ERR_OVERSIZE: c_int = 2;
const SZ_ERR_RANGE_CHAR: c_int = 3;
const SZ_ERR_RANGE_BYTE: c_int = 4;
const SZ_ERR_RANGE_SHORT: c_int = 5;

fn status_of(e: WireError) -> c_int {
    match e {
        WireError::Overflow => SZ_ERR_OVERFLOW,
        WireError::OversizeWrite => SZ_ERR_OVERSIZE,
        WireError::RangeError(m) => {
            if m.starts_with("MSG_WriteChar") {
                SZ_ERR_RANGE_CHAR
            } else if m.starts_with("MSG_WriteByte") {
                SZ_ERR_RANGE_BYTE
            } else {
                SZ_ERR_RANGE_SHORT
            }
        }
    }
}

/// Runs `f` over a borrowed view of the C sizebuf, writing
/// `cursize`/`overflowed` back afterwards -- also on error, matching the
/// partial effects C leaves behind when Host_Error fires mid-operation.
///
/// # Safety
/// `sb` must point at a live, initialized `sizebuf_t` whose `data` covers
/// `maxsize` bytes (SZ_Alloc guarantees this). Single-threaded host frame.
unsafe fn with_sizebuf(
    sb: *mut c::sizebuf_t,
    f: impl FnOnce(&mut SizeBuf<'_>) -> Result<(), WireError>,
) -> c_int {
    // SAFETY: caller contract above
    let raw = unsafe { &mut *sb };
    let data = if raw.data.is_null() {
        &mut [][..]
    } else {
        // SAFETY: data covers maxsize bytes per SZ_Alloc
        unsafe { core::slice::from_raw_parts_mut(raw.data, raw.maxsize as usize) }
    };
    let mut view = SizeBuf {
        allowoverflow: raw.allowoverflow,
        overflowed: raw.overflowed,
        data,
        cursize: raw.cursize,
        overflow_events: 0,
    };
    let r = f(&mut view);
    raw.cursize = view.cursize;
    raw.overflowed = view.overflowed;
    let events = view.overflow_events;
    let _ = view; // end the sizebuf borrow before touching module state
                  // ADR-009/aliasing: no C is called from this frame. The overflow
                  // diagnostics ("SZ_GetSpace: overflow") are accumulated here and emitted
                  // by net_msg_glue.c's C frame after the export returns -- Con_Printf is
                  // not a leaf (it can reach SCR_UpdateScreen), so it must not run while
                  // Rust holds borrows of the sizebuf.
    if events > 0 {
        // SAFETY: single-threaded host frame (caller contract)
        unsafe {
            PENDING_OVERFLOW_PRINTS = PENDING_OVERFLOW_PRINTS.wrapping_add(events);
        }
    }
    match r {
        Ok(()) => SZ_OK,
        Err(e) => status_of(e),
    }
}

/// pending "SZ_GetSpace: overflow" diagnostics for the glue to print in a C
/// frame; single-threaded host frame like the C original's console path
static mut PENDING_OVERFLOW_PRINTS: c_uint = 0;

/// Drains the pending overflow-diagnostic count; net_msg_glue.c prints one
/// "SZ_GetSpace: overflow" line per event from its own C frame.
#[no_mangle]
pub extern "C" fn quake_rs_sz_take_overflow_events() -> c_uint {
    // SAFETY: single-threaded host frame
    unsafe {
        let n = PENDING_OVERFLOW_PRINTS;
        PENDING_OVERFLOW_PRINTS = 0;
        n
    }
}

/// Runs `f` over a reader view of the C globals, writing the cursor state
/// back afterwards and folding badread events into the harness counter.
///
/// # Safety
/// Single-threaded host frame; `net_message` is initialized (NET_Init).
unsafe fn with_reader<R>(f: impl FnOnce(&mut MsgReader<'_>) -> R) -> R {
    // SAFETY: single-threaded host frame (caller contract)
    unsafe {
        let nm = &raw mut c::net_message;
        let data = if (*nm).data.is_null() {
            &[][..]
        } else {
            core::slice::from_raw_parts((*nm).data, (*nm).maxsize as usize)
        };
        let mut r = MsgReader {
            data,
            cursize: (*nm).cursize,
            readcount: c::msg_readcount,
            badread: c::msg_badread,
            badread_events: 0,
        };
        let out = f(&mut r);
        c::msg_readcount = r.readcount;
        c::msg_badread = r.badread;
        c::harness_badread_count = c::harness_badread_count.wrapping_add(r.badread_events);
        out
    }
}

// ---------------------------------------------------------------------------
// writer/SZ status exports (wrapped by net_msg_glue.c)
// ---------------------------------------------------------------------------

macro_rules! write_shim {
    ($name:ident, $core:path, $ty:ty) => {
        /// # Safety
        /// `sb` is a live sizebuf_t (see `with_sizebuf`).
        #[no_mangle]
        pub unsafe extern "C" fn $name(sb: *mut c::sizebuf_t, v: $ty) -> c_int {
            // SAFETY: forwarded caller contract
            unsafe { with_sizebuf(sb, |b| $core(b, v)) }
        }
    };
}

write_shim!(quake_rs_msg_write_char, msg::write_char, c_int);
write_shim!(quake_rs_msg_write_byte, msg::write_byte, c_int);
write_shim!(quake_rs_msg_write_short, msg::write_short, c_int);
write_shim!(quake_rs_msg_write_long, msg::write_long, c_int);
write_shim!(quake_rs_msg_write_uint64, msg::write_uint64, u64);
write_shim!(quake_rs_msg_write_int64, msg::write_int64, i64);
write_shim!(quake_rs_msg_write_float, msg::write_float, f32);
write_shim!(quake_rs_msg_write_double, msg::write_double, f64);

/// # Safety
/// `sb` live sizebuf_t; `s` NULL or NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_msg_write_string(
    sb: *mut c::sizebuf_t,
    s: *const c_char,
) -> c_int {
    // SAFETY: caller contract
    unsafe {
        let bytes = if s.is_null() {
            None
        } else {
            Some(core::ffi::CStr::from_ptr(s).to_bytes())
        };
        with_sizebuf(sb, |b| msg::write_string(b, bytes))
    }
}

/// # Safety
/// `sb` live sizebuf_t; `s` NUL-terminated (C strlen's it unconditionally).
#[no_mangle]
pub unsafe extern "C" fn quake_rs_msg_write_string_unterminated(
    sb: *mut c::sizebuf_t,
    s: *const c_char,
) -> c_int {
    // SAFETY: caller contract
    unsafe {
        let bytes = core::ffi::CStr::from_ptr(s).to_bytes();
        with_sizebuf(sb, |b| msg::write_string_unterminated(b, bytes))
    }
}

macro_rules! write_flag_shim {
    ($name:ident, $core:path) => {
        /// # Safety
        /// `sb` is a live sizebuf_t (see `with_sizebuf`).
        #[no_mangle]
        pub unsafe extern "C" fn $name(sb: *mut c::sizebuf_t, f: f32, flags: c_uint) -> c_int {
            // SAFETY: forwarded caller contract
            unsafe { with_sizebuf(sb, |b| $core(b, f, flags)) }
        }
    };
}

write_flag_shim!(quake_rs_msg_write_coord, msg::write_coord);
write_flag_shim!(quake_rs_msg_write_angle, msg::write_angle);
write_flag_shim!(quake_rs_msg_write_angle16, msg::write_angle16);

/// # Safety
/// `sb` is a live sizebuf_t (see `with_sizebuf`).
#[no_mangle]
pub unsafe extern "C" fn quake_rs_msg_write_entity(
    sb: *mut c::sizebuf_t,
    entnum: c_uint,
    pext2: c_uint,
) -> c_int {
    // SAFETY: forwarded caller contract
    unsafe { with_sizebuf(sb, |b| msg::write_entity(b, entnum, pext2)) }
}

/// # Safety
/// `sb` live sizebuf_t; `offset` a valid out pointer.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sz_get_space(
    sb: *mut c::sizebuf_t,
    length: c_int,
    offset: *mut c_int,
) -> c_int {
    let mut at = 0i32;
    // SAFETY: forwarded caller contract
    let status = unsafe {
        with_sizebuf(sb, |b| {
            at = b.get_space(length)? as i32;
            Ok(())
        })
    };
    // SAFETY: offset is a valid out pointer
    unsafe { *offset = at };
    status
}

/// # Safety
/// `sb` live sizebuf_t; `data` covers `length` bytes.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sz_write(
    sb: *mut c::sizebuf_t,
    data: *const core::ffi::c_void,
    length: c_int,
) -> c_int {
    // SAFETY: caller contract
    unsafe {
        let bytes = core::slice::from_raw_parts(data.cast::<u8>(), length as usize);
        with_sizebuf(sb, |b| b.write(bytes))
    }
}

/// # Safety
/// `sb` live sizebuf_t; `s` NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_sz_print(sb: *mut c::sizebuf_t, s: *const c_char) -> c_int {
    // SAFETY: caller contract
    unsafe {
        let bytes = core::ffi::CStr::from_ptr(s).to_bytes();
        with_sizebuf(sb, |b| b.print(bytes))
    }
}

// ---------------------------------------------------------------------------
// exact-name exports (no error paths; common.h declares these)
// ---------------------------------------------------------------------------

/// # Safety
/// `buf` is a live sizebuf_t.
#[no_mangle]
pub unsafe extern "C" fn SZ_Clear(buf: *mut c::sizebuf_t) {
    // SAFETY: caller contract
    unsafe {
        (*buf).cursize = 0;
        (*buf).overflowed = false;
    }
}

#[no_mangle]
pub extern "C" fn MSG_BeginReading() {
    // SAFETY: single-threaded host frame globals
    unsafe {
        c::msg_readcount = 0;
        c::msg_badread = false;
    }
}

macro_rules! read_shim {
    ($name:ident, $method:ident, $ret:ty) => {
        #[no_mangle]
        pub extern "C" fn $name() -> $ret {
            // SAFETY: single-threaded host frame (engine reader contract)
            unsafe { with_reader(|r| r.$method()) }
        }
    };
}

read_shim!(MSG_ReadChar, read_char, c_int);
read_shim!(MSG_ReadByte, read_byte, c_int);
read_shim!(MSG_ReadShort, read_short, c_int);
read_shim!(MSG_ReadLong, read_long, c_int);
read_shim!(MSG_ReadUInt64, read_uint64, u64);
read_shim!(MSG_ReadInt64, read_int64, i64);
read_shim!(MSG_ReadFloat, read_float, f32);
// yes, f32: the C original is declared `float MSG_ReadDouble (void)`
read_shim!(MSG_ReadDouble, read_double, f32);

macro_rules! read_flag_shim {
    ($name:ident, $method:ident) => {
        #[no_mangle]
        pub extern "C" fn $name(flags: c_uint) -> f32 {
            // SAFETY: single-threaded host frame (engine reader contract)
            unsafe { with_reader(|r| r.$method(flags)) }
        }
    };
}

read_flag_shim!(MSG_ReadCoord, read_coord);
read_flag_shim!(MSG_ReadAngle, read_angle);
read_flag_shim!(MSG_ReadAngle16, read_angle16);

#[no_mangle]
pub extern "C" fn MSG_ReadEntity(pext2: c_uint) -> c_uint {
    // SAFETY: single-threaded host frame (engine reader contract)
    unsafe { with_reader(|r| r.read_entity(pext2)) }
}

/// C returns a pointer into a function-static 2048-byte buffer; mirrored
/// with a module static (single-threaded host frame, like the original).
#[no_mangle]
pub extern "C" fn MSG_ReadString() -> *const c_char {
    static mut STRING_BUF: [u8; 2048] = [0; 2048];
    // SAFETY: single-threaded host frame; the static mirrors C's
    // function-static lifetime and aliasing (each call overwrites it)
    unsafe {
        let s = with_reader(|r| r.read_string());
        let p = (&raw mut STRING_BUF).cast::<u8>();
        let n = s.len().min(2047);
        core::ptr::copy_nonoverlapping(s.as_ptr(), p, n);
        *p.add(n) = 0;
        p.cast::<c_char>()
    }
}

// ---------------------------------------------------------------------------
// M4: demo file format (cl_demo.c keeps the raw stdio; these own the bytes)
// ---------------------------------------------------------------------------

/// # Safety
/// `viewangles` points at 3 floats, `out` at 16 writable bytes.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_demo_record_header(
    cursize: c_int,
    viewangles: *const f32,
    out: *mut u8,
) {
    // SAFETY: caller contract
    unsafe {
        let angles = [*viewangles, *viewangles.add(1), *viewangles.add(2)];
        let h = quake_net::demo::record_header(cursize, angles);
        core::ptr::copy_nonoverlapping(h.as_ptr(), out, h.len());
    }
}

/// Returns 0 for the `Sys_Error ("Demo message > MAX_MSGLEN")` path, 1 ok.
///
/// # Safety
/// `header` points at 16 bytes, `cursize` and `viewangles` (3 floats) are
/// valid out pointers.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_demo_parse_record_header(
    header: *const u8,
    cursize: *mut c_int,
    viewangles: *mut f32,
) -> c_int {
    // SAFETY: caller contract
    unsafe {
        let mut h = [0u8; quake_net::demo::RECORD_HEADER_SIZE];
        core::ptr::copy_nonoverlapping(header, h.as_mut_ptr(), h.len());
        match quake_net::demo::parse_record_header(&h) {
            Some((len, angles)) => {
                *cursize = len;
                for (i, a) in angles.iter().enumerate() {
                    *viewangles.add(i) = *a;
                }
                1
            }
            None => 0,
        }
    }
}

/// Writes the "%i\n" forcetrack header line; returns its length.
///
/// # Safety
/// `out` covers `outsize` bytes.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_demo_forcetrack_line(
    track: c_int,
    out: *mut c_char,
    outsize: c_int,
) -> c_int {
    let line = quake_net::demo::forcetrack_line(track);
    // The caller writes exactly the returned length to the demo file, so a
    // silent clamp here would emit a truncated (corrupt) header with no
    // diagnostic. cls.forcetrack is an int, so the line is at most 12 bytes
    // and every call site passes >= 32 -- make the contract fatal rather
    // than let that invariant rot quietly. Sys_Error exits (no longjmp), so
    // raising it from this frame is ADR-009-safe.
    if line.len() > outsize.max(0) as usize {
        // SAFETY: plain fatal exit, no unwinding back through this frame
        unsafe { c::Sys_Error(c"quake_rs_demo_forcetrack_line: buffer too small".as_ptr()) };
    }
    // SAFETY: caller contract; length checked against outsize above
    unsafe { core::ptr::copy_nonoverlapping(line.as_ptr().cast::<c_char>(), out, line.len()) };
    line.len() as c_int
}

/// fscanf("%i") + fgetc-newline parse over a prefix chunk of the demo file.
/// Returns 1 with `track`/`consumed` set, or 0 for the invalid-demo path.
///
/// # Safety
/// `buf` covers `len` bytes; `track` and `consumed` are valid out pointers.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_demo_parse_forcetrack(
    buf: *const c_char,
    len: c_int,
    track: *mut c_int,
    consumed: *mut c_int,
) -> c_int {
    // SAFETY: caller contract
    unsafe {
        let bytes = core::slice::from_raw_parts(buf.cast::<u8>(), len.max(0) as usize);
        match quake_net::demo::parse_forcetrack(bytes) {
            Some((t, n)) => {
                *track = t;
                *consumed = n as c_int;
                1
            }
            None => 0,
        }
    }
}

/// `Sys_fseek (cls.demofile, -17, SEEK_END)` offset when resuming a
/// recording (4 length + 12 viewangles + 1 svc_disconnect byte).
#[no_mangle]
pub extern "C" fn quake_rs_demo_resume_seek_offset() -> i64 {
    quake_net::demo::RESUME_RECORD_SEEK_END_OFFSET
}
