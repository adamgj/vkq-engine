//! Differential test: `quake_net::{msg, sizebuf}` vs the C originals in
//! `Quake/net_msg.c` (compiled as `c_ref_*`). Phase 5 M2.
//!
//! The C readers run over the stub-owned `c_ref_net_message` +
//! `c_ref_msg_readcount`/`c_ref_msg_badread` globals; the Rust side is the
//! explicit `MsgReader`. Every comparison checks the returned value AND the
//! reader cursor/badread state, so cursor-order bugs (e.g. the coord24
//! evaluation-order landmine) cannot hide.
//!
//! All tests share the C globals, so they serialize on one mutex.

use core::ffi::{c_char, c_int, c_uint};
use std::sync::{Mutex, MutexGuard};

use quake_ctest as _; // links the cc-built c_ref_* archive
use quake_net::msg::{self, MsgReader};
use quake_net::protocol::*;
use quake_net::sizebuf::{SizeBuf, WireError};
use quake_types::net::SizeBuf as CSizeBuf;

extern "C" {
    fn c_ref_MSG_WriteChar(sb: *mut CSizeBuf, c: c_int);
    fn c_ref_MSG_WriteByte(sb: *mut CSizeBuf, c: c_int);
    fn c_ref_MSG_WriteShort(sb: *mut CSizeBuf, c: c_int);
    fn c_ref_MSG_WriteLong(sb: *mut CSizeBuf, c: c_int);
    fn c_ref_MSG_WriteUInt64(sb: *mut CSizeBuf, c: u64);
    fn c_ref_MSG_WriteInt64(sb: *mut CSizeBuf, c: i64);
    fn c_ref_MSG_WriteFloat(sb: *mut CSizeBuf, f: f32);
    fn c_ref_MSG_WriteDouble(sb: *mut CSizeBuf, f: f64);
    fn c_ref_MSG_WriteString(sb: *mut CSizeBuf, s: *const c_char);
    fn c_ref_MSG_WriteStringUnterminated(sb: *mut CSizeBuf, s: *const c_char);
    fn c_ref_MSG_WriteCoord(sb: *mut CSizeBuf, f: f32, flags: c_uint);
    fn c_ref_MSG_WriteAngle(sb: *mut CSizeBuf, f: f32, flags: c_uint);
    fn c_ref_MSG_WriteAngle16(sb: *mut CSizeBuf, f: f32, flags: c_uint);
    fn c_ref_MSG_WriteEntity(sb: *mut CSizeBuf, entnum: c_uint, pext2: c_uint);

    fn c_ref_MSG_BeginReading();
    fn c_ref_MSG_ReadChar() -> c_int;
    fn c_ref_MSG_ReadByte() -> c_int;
    fn c_ref_MSG_ReadShort() -> c_int;
    fn c_ref_MSG_ReadLong() -> c_int;
    fn c_ref_MSG_ReadUInt64() -> u64;
    fn c_ref_MSG_ReadInt64() -> i64;
    fn c_ref_MSG_ReadFloat() -> f32;
    // yes, float: the C original is declared `float MSG_ReadDouble (void)`
    fn c_ref_MSG_ReadDouble() -> f32;
    fn c_ref_MSG_ReadString() -> *const c_char;
    fn c_ref_MSG_ReadCoord(flags: c_uint) -> f32;
    fn c_ref_MSG_ReadAngle(flags: c_uint) -> f32;
    fn c_ref_MSG_ReadAngle16(flags: c_uint) -> f32;
    fn c_ref_MSG_ReadEntity(pext2: c_uint) -> c_uint;

    fn c_ref_SZ_Print(sb: *mut CSizeBuf, s: *const c_char);

    static mut c_ref_net_message: CSizeBuf;
    static mut c_ref_msg_readcount: c_int;
    static mut c_ref_msg_badread: bool;

    fn ctest_try_host(
        f: unsafe extern "C" fn(*mut core::ffi::c_void),
        arg: *mut core::ffi::c_void,
    ) -> c_int;
}

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

/// deterministic xorshift64* for sweep values
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
}

/// A C sizebuf over a Rust-owned backing store
struct CBuf {
    store: Vec<u8>,
    sb: CSizeBuf,
}

impl CBuf {
    fn new(maxsize: usize, allowoverflow: bool) -> CBuf {
        let mut store = vec![0u8; maxsize];
        let sb = CSizeBuf {
            allowoverflow,
            overflowed: false,
            data: store.as_mut_ptr(),
            maxsize: maxsize as c_int,
            cursize: 0,
        };
        CBuf { store, sb }
    }
    fn written(&self) -> &[u8] {
        &self.store[..self.sb.cursize as usize]
    }
}

/// One writer op applied identically to a C sizebuf and a Rust SizeBuf
#[derive(Debug, Clone)]
enum WOp {
    Char(i32),
    Byte(i32),
    Short(i32),
    Long(i32),
    U64(u64),
    I64(i64),
    Float(f32),
    Double(f64),
    Str(Vec<u8>),
    StrUnterminated(Vec<u8>),
    Coord(f32, u32),
    Angle(f32, u32),
    Angle16(f32, u32),
    Entity(u32, u32),
}

fn apply_c(sb: &mut CSizeBuf, op: &WOp) {
    // SAFETY: sb points at a live CBuf whose store outlives the call; string
    // ops pass NUL-terminated owned bytes
    unsafe {
        match op {
            WOp::Char(c) => c_ref_MSG_WriteChar(sb, *c),
            WOp::Byte(c) => c_ref_MSG_WriteByte(sb, *c),
            WOp::Short(c) => c_ref_MSG_WriteShort(sb, *c),
            WOp::Long(c) => c_ref_MSG_WriteLong(sb, *c),
            WOp::U64(c) => c_ref_MSG_WriteUInt64(sb, *c),
            WOp::I64(c) => c_ref_MSG_WriteInt64(sb, *c),
            WOp::Float(f) => c_ref_MSG_WriteFloat(sb, *f),
            WOp::Double(f) => c_ref_MSG_WriteDouble(sb, *f),
            WOp::Str(s) => {
                let cs = std::ffi::CString::new(s.clone()).unwrap();
                c_ref_MSG_WriteString(sb, cs.as_ptr());
            }
            WOp::StrUnterminated(s) => {
                let cs = std::ffi::CString::new(s.clone()).unwrap();
                c_ref_MSG_WriteStringUnterminated(sb, cs.as_ptr());
            }
            WOp::Coord(f, flags) => c_ref_MSG_WriteCoord(sb, *f, *flags),
            WOp::Angle(f, flags) => c_ref_MSG_WriteAngle(sb, *f, *flags),
            WOp::Angle16(f, flags) => c_ref_MSG_WriteAngle16(sb, *f, *flags),
            WOp::Entity(e, pext2) => c_ref_MSG_WriteEntity(sb, *e, *pext2),
        }
    }
}

fn apply_rust(sb: &mut SizeBuf<'_>, op: &WOp) -> Result<(), WireError> {
    match op {
        WOp::Char(c) => msg::write_char(sb, *c),
        WOp::Byte(c) => msg::write_byte(sb, *c),
        WOp::Short(c) => msg::write_short(sb, *c),
        WOp::Long(c) => msg::write_long(sb, *c),
        WOp::U64(c) => msg::write_uint64(sb, *c),
        WOp::I64(c) => msg::write_int64(sb, *c),
        WOp::Float(f) => msg::write_float(sb, *f),
        WOp::Double(f) => msg::write_double(sb, *f),
        WOp::Str(s) => msg::write_string(sb, Some(s)),
        WOp::StrUnterminated(s) => msg::write_string_unterminated(sb, s),
        WOp::Coord(f, flags) => msg::write_coord(sb, *f, *flags),
        WOp::Angle(f, flags) => msg::write_angle(sb, *f, *flags),
        WOp::Angle16(f, flags) => msg::write_angle16(sb, *f, *flags),
        WOp::Entity(e, pext2) => msg::write_entity(sb, *e, *pext2),
    }
}

fn compare_write_ops(ops: &[WOp], ctx: &str) {
    let mut cbuf = CBuf::new(65536, false);
    let mut rstore = vec![0u8; 65536];
    let mut rbuf = SizeBuf::new(&mut rstore);
    for op in ops {
        apply_c(&mut cbuf.sb, op);
        apply_rust(&mut rbuf, op).unwrap_or_else(|e| panic!("{ctx}: rust err {e:?} on {op:?}"));
    }
    assert_eq!(cbuf.sb.cursize, rbuf.cursize, "{ctx}: cursize");
    assert_eq!(cbuf.written(), rbuf.written(), "{ctx}: bytes");
}

const FLAG_SETS: [u32; 8] = [
    0,
    PRFL_SHORTANGLE,
    PRFL_FLOATANGLE,
    PRFL_24BITCOORD,
    PRFL_FLOATCOORD,
    PRFL_INT32COORD,
    PRFL_SHORTANGLE | PRFL_24BITCOORD,
    PRFL_FLOATANGLE | PRFL_FLOATCOORD | PRFL_INT32COORD,
];

#[allow(clippy::excessive_precision)]
fn interesting_floats() -> Vec<f32> {
    let mut v: Vec<f32> = vec![
        0.0,
        -0.0,
        1.0,
        -1.0,
        0.49,
        0.5,
        0.51,
        -0.49,
        -0.5,
        -0.51,
        7.9375,
        -7.9375,
        63.499,
        4095.96875,
        -4095.96875,
        4096.0,
        -4096.0,
        4097.5,
        32767.984,
        -32767.984,
        32768.0,
        -32768.0,
        65536.5,
        359.999,
        360.0,
        -359.999,
        720.25,
        -720.25,
        1e6,
        -1e6,
        1e30,
        -1e30,
        f32::MIN_POSITIVE,
        -f32::MIN_POSITIVE,
        f32::EPSILON,
        1e-40, // denormal
    ];
    let mut rng = Rng(0x9E3779B97F4A7C15);
    for _ in 0..4000 {
        let bits = rng.next() as u32;
        let f = f32::from_bits(bits);
        // NaN/inf trip the UB-on-cast paths differently per C compiler even
        // on one platform (constant folding vs runtime); the engine never
        // sends them through coord/angle encoders. Keep the sweep to reals.
        if f.is_finite() {
            v.push(f);
        }
    }
    v
}

#[test]
fn writers_match_c() {
    let _g = lock();

    // integer writers: boundaries + sweep (silent truncation domain included)
    let mut ints: Vec<i32> = vec![
        0,
        1,
        -1,
        127,
        128,
        -128,
        -129,
        255,
        256,
        -255,
        -256,
        32767,
        32768,
        -32768,
        -32769,
        65535,
        65536,
        i32::MAX,
        i32::MIN,
    ];
    let mut rng = Rng(0xDEADBEEFCAFEF00D);
    for _ in 0..2000 {
        ints.push(rng.next() as i32);
    }
    let ops: Vec<WOp> = ints
        .iter()
        .flat_map(|&c| [WOp::Char(c), WOp::Byte(c), WOp::Short(c), WOp::Long(c)])
        .collect();
    compare_write_ops(&ops, "int writers");

    // varints: the full length-prefix ladder incl. the b>=8 masked-shift
    // domain (reachable via WriteInt64 extremes)
    let mut u64s: Vec<u64> = vec![0, 1, 127, 128, 16383, 16384, u64::MAX];
    let mut i64s: Vec<i64> = vec![0, 1, -1, 63, 64, -64, -65, i64::MAX, i64::MIN];
    for b in 0..64 {
        u64s.push(1u64 << b);
        u64s.push((1u64 << b) - 1);
        u64s.push((1u64 << b) + 1);
        i64s.push(1i64.wrapping_shl(b));
        i64s.push(1i64.wrapping_shl(b).wrapping_neg());
    }
    for _ in 0..2000 {
        u64s.push(rng.next());
        i64s.push(rng.next() as i64);
    }
    compare_write_ops(
        &u64s.iter().map(|&c| WOp::U64(c)).collect::<Vec<_>>(),
        "u64",
    );
    compare_write_ops(
        &i64s.iter().map(|&c| WOp::I64(c)).collect::<Vec<_>>(),
        "i64",
    );

    // float/double raw writers (bit patterns incl. NaN/inf are byte moves)
    let mut fops = Vec::new();
    for _ in 0..2000 {
        fops.push(WOp::Float(f32::from_bits(rng.next() as u32)));
        fops.push(WOp::Double(f64::from_bits(rng.next())));
    }
    fops.push(WOp::Float(f32::NAN));
    fops.push(WOp::Double(f64::INFINITY));
    compare_write_ops(&fops, "float/double");

    // coord/angle across every flag set
    for &flags in &FLAG_SETS {
        let ops: Vec<WOp> = interesting_floats()
            .iter()
            .flat_map(|&f| {
                [
                    WOp::Coord(f, flags),
                    WOp::Angle(f, flags),
                    WOp::Angle16(f, flags),
                ]
            })
            .collect();
        compare_write_ops(&ops, &format!("coord/angle flags={flags:#x}"));
    }

    // entity encoding across the pext2 gate and the 0x7fff boundary
    let mut eops = Vec::new();
    for &pext2 in &[0u32, PEXT2_REPLACEMENTDELTAS, PEXT2_PREDINFO] {
        for e in [0u32, 1, 0x7ffe, 0x7fff, 0x8000, 0x8001, 0xffff, 0x7fffff] {
            eops.push(WOp::Entity(e, pext2));
        }
    }
    compare_write_ops(&eops, "entity");

    // strings incl. empty and NULL
    let sops = vec![
        WOp::Str(b"".to_vec()),
        WOp::Str(b"maps/e1m1.bsp".to_vec()),
        WOp::Str(vec![b'x'; 900]),
        WOp::StrUnterminated(b"unterminated".to_vec()),
    ];
    compare_write_ops(&sops, "strings");
    {
        let mut cbuf = CBuf::new(65536, false);
        let mut rstore = vec![0u8; 65536];
        let mut rbuf = SizeBuf::new(&mut rstore);
        // SAFETY: NULL string pointer is an accepted input of MSG_WriteString
        unsafe { c_ref_MSG_WriteString(&mut cbuf.sb, core::ptr::null()) };
        msg::write_string(&mut rbuf, None).unwrap();
        assert_eq!(cbuf.written(), rbuf.written(), "NULL string");
    }
}

#[test]
fn sz_overflow_semantics_match_c() {
    let _g = lock();

    // allowoverflow=true: overflow clears the buffer, sets overflowed, and
    // keeps writing from offset 0
    let mut cbuf = CBuf::new(256, true);
    let mut rstore = vec![0u8; 256];
    let mut rbuf = SizeBuf::new(&mut rstore);
    rbuf.allowoverflow = true;
    for i in 0..100 {
        let op = WOp::Long(i);
        apply_c(&mut cbuf.sb, &op);
        apply_rust(&mut rbuf, &op).unwrap();
    }
    assert_eq!(cbuf.sb.cursize, rbuf.cursize, "overflow cursize");
    assert!(cbuf.sb.overflowed && rbuf.overflowed, "overflowed flags");
    assert_eq!(cbuf.written(), rbuf.written(), "overflow bytes");

    // allowoverflow=false: C Host_Errors (trapped), Rust returns Overflow
    unsafe extern "C" fn fill(arg: *mut core::ffi::c_void) {
        // SAFETY: arg is the CSizeBuf passed below
        let sb = arg.cast::<CSizeBuf>();
        for i in 0..100 {
            // SAFETY: sb stays valid for the whole loop
            unsafe { c_ref_MSG_WriteLong(sb, i) };
        }
    }
    let mut cbuf = CBuf::new(256, false);
    // SAFETY: ctest_try_host arms the Host_Error trap around fill
    let c_failed = unsafe { ctest_try_host(fill, (&mut cbuf.sb as *mut CSizeBuf).cast()) };
    assert_eq!(c_failed, 1, "C Host_Error fired");
    let mut rstore = vec![0u8; 256];
    let mut rbuf = SizeBuf::new(&mut rstore);
    let mut rust_err = None;
    for i in 0..100 {
        if let Err(e) = msg::write_long(&mut rbuf, i) {
            rust_err = Some(e);
            break;
        }
    }
    assert_eq!(rust_err, Some(WireError::Overflow));
    // both sides stopped at the same fill level
    assert_eq!(cbuf.sb.cursize, rbuf.cursize, "pre-overflow cursize");
    assert_eq!(cbuf.written(), rbuf.written(), "pre-overflow bytes");
}

#[test]
fn sz_print_matches_c() {
    let _g = lock();
    // both branch shapes: after an unterminated write (no trailing NUL) and
    // after a terminated one (write over trailing NUL)
    for lead in [
        Some(WOp::StrUnterminated(b"cmd".to_vec())),
        Some(WOp::Str(b"cmd".to_vec())),
        None,
    ] {
        if lead.is_none() {
            // COMPAT: SZ_Print on an empty buffer reads data[-1] in C (UB);
            // skipped -- no engine call site does it (see sizebuf.rs)
            continue;
        }
        let mut cbuf = CBuf::new(4096, false);
        let mut rstore = vec![0u8; 4096];
        let mut rbuf = SizeBuf::new(&mut rstore);
        let lead = lead.unwrap();
        apply_c(&mut cbuf.sb, &lead);
        apply_rust(&mut rbuf, &lead).unwrap();
        let cs = std::ffi::CString::new("echo hello\n").unwrap();
        // SAFETY: live sizebuf + NUL-terminated string
        unsafe { c_ref_SZ_Print(&mut cbuf.sb, cs.as_ptr()) };
        rbuf.print(b"echo hello\n").unwrap();
        assert_eq!(cbuf.sb.cursize, rbuf.cursize);
        assert_eq!(cbuf.written(), rbuf.written());
    }
}

// ---------------------------------------------------------------------------
// readers
// ---------------------------------------------------------------------------

/// One reader op; applied to the C globals and the Rust MsgReader, comparing
/// value + cursor + badread after each step
#[derive(Debug, Clone, Copy)]
enum ROp {
    Char,
    Byte,
    Short,
    Long,
    U64,
    I64,
    Float,
    Double,
    Str,
    Coord(u32),
    Angle(u32),
    Angle16(u32),
    Entity(u32),
}

const ALL_ROPS: [ROp; 13] = [
    ROp::Char,
    ROp::Byte,
    ROp::Short,
    ROp::Long,
    ROp::U64,
    ROp::I64,
    ROp::Float,
    ROp::Double,
    ROp::Str,
    ROp::Coord(PRFL_24BITCOORD),
    ROp::Angle(PRFL_SHORTANGLE),
    ROp::Angle16(0),
    ROp::Entity(PEXT2_REPLACEMENTDELTAS),
];

/// Runs `ops` over `buf[..cursize]` on both sides (the whole `buf` is the
/// allocation, so past-cursize reads see identical overhang bytes).
fn compare_read_ops(buf: &mut [u8], cursize: i32, ops: &[ROp], ctx: &str) {
    // SAFETY: the C globals are ours under TEST_LOCK; data points into buf,
    // which outlives the calls
    unsafe {
        c_ref_net_message.data = buf.as_mut_ptr();
        c_ref_net_message.maxsize = buf.len() as c_int;
        c_ref_net_message.cursize = cursize;
        c_ref_net_message.allowoverflow = false;
        c_ref_net_message.overflowed = false;
        c_ref_MSG_BeginReading();
    }
    let mut r = MsgReader::begin(buf, cursize);
    for (i, op) in ops.iter().enumerate() {
        // SAFETY: reader calls under TEST_LOCK with globals set up above
        unsafe {
            match op {
                ROp::Char => assert_eq!(c_ref_MSG_ReadChar(), r.read_char(), "{ctx}[{i}] char"),
                ROp::Byte => assert_eq!(c_ref_MSG_ReadByte(), r.read_byte(), "{ctx}[{i}] byte"),
                ROp::Short => assert_eq!(c_ref_MSG_ReadShort(), r.read_short(), "{ctx}[{i}] short"),
                ROp::Long => assert_eq!(c_ref_MSG_ReadLong(), r.read_long(), "{ctx}[{i}] long"),
                ROp::U64 => assert_eq!(c_ref_MSG_ReadUInt64(), r.read_uint64(), "{ctx}[{i}] u64"),
                ROp::I64 => assert_eq!(c_ref_MSG_ReadInt64(), r.read_int64(), "{ctx}[{i}] i64"),
                ROp::Float => assert_eq!(
                    c_ref_MSG_ReadFloat().to_bits(),
                    r.read_float().to_bits(),
                    "{ctx}[{i}] float"
                ),
                ROp::Double => assert_eq!(
                    c_ref_MSG_ReadDouble().to_bits(),
                    r.read_double().to_bits(),
                    "{ctx}[{i}] double"
                ),
                ROp::Str => {
                    let cs = std::ffi::CStr::from_ptr(c_ref_MSG_ReadString());
                    assert_eq!(cs.to_bytes(), r.read_string().as_slice(), "{ctx}[{i}] str");
                }
                ROp::Coord(f) => assert_eq!(
                    c_ref_MSG_ReadCoord(*f).to_bits(),
                    r.read_coord(*f).to_bits(),
                    "{ctx}[{i}] coord({f:#x})"
                ),
                ROp::Angle(f) => assert_eq!(
                    c_ref_MSG_ReadAngle(*f).to_bits(),
                    r.read_angle(*f).to_bits(),
                    "{ctx}[{i}] angle({f:#x})"
                ),
                ROp::Angle16(f) => assert_eq!(
                    c_ref_MSG_ReadAngle16(*f).to_bits(),
                    r.read_angle16(*f).to_bits(),
                    "{ctx}[{i}] angle16({f:#x})"
                ),
                ROp::Entity(p) => assert_eq!(
                    c_ref_MSG_ReadEntity(*p),
                    r.read_entity(*p),
                    "{ctx}[{i}] entity({p:#x})"
                ),
            }
            let (c_count, c_bad) = (
                core::ptr::read(&raw const c_ref_msg_readcount),
                core::ptr::read(&raw const c_ref_msg_badread),
            );
            assert_eq!(c_count, r.readcount, "{ctx}[{i}] readcount");
            assert_eq!(c_bad, r.badread, "{ctx}[{i}] badread");
        }
    }
}

#[test]
fn readers_match_c() {
    let _g = lock();
    let mut rng = Rng(0x1234567890ABCDEF);

    // random buffers x random op sequences, incl. short cursize with a
    // nonzero overhang (exercises the unchecked float/double reads and the
    // badread sequencing)
    for round in 0..200 {
        let alloc = 512usize;
        let mut buf: Vec<u8> = (0..alloc).map(|_| rng.next() as u8).collect();
        let cursize = (rng.next() % 96) as i32;
        let ops: Vec<ROp> = (0..24)
            .map(|_| ALL_ROPS[(rng.next() % ALL_ROPS.len() as u64) as usize])
            .collect();
        compare_read_ops(&mut buf, cursize, &ops, &format!("rand{round}"));
    }

    // every flag set over structured coord/angle payloads
    for &flags in &FLAG_SETS {
        let mut buf: Vec<u8> = (0..256).map(|_| rng.next() as u8).collect();
        let ops = [
            ROp::Coord(flags),
            ROp::Angle(flags),
            ROp::Angle16(flags),
            ROp::Coord(flags),
            ROp::Angle(flags),
        ];
        compare_read_ops(&mut buf, 200, &ops, &format!("flags{flags:#x}"));
    }

    // the ReadUInt64 masked-shift bug domain: write values with >=4
    // continuation bytes with the C writer, read back on both sides
    let mut cbuf = CBuf::new(4096, false);
    let mut values = vec![
        (1u64 << 28) - 1,
        1u64 << 28,
        1u64 << 35,
        1u64 << 42,
        1u64 << 49,
        1u64 << 56,
        u64::MAX,
    ];
    for _ in 0..64 {
        values.push(rng.next());
    }
    for &v in &values {
        // SAFETY: live sizebuf under TEST_LOCK
        unsafe { c_ref_MSG_WriteUInt64(&mut cbuf.sb, v) };
    }
    let cursize = cbuf.sb.cursize;
    let ops: Vec<ROp> = values.iter().map(|_| ROp::U64).collect();
    compare_read_ops(&mut cbuf.store, cursize, &ops, "u64 bug domain");

    // underrun tail: keep reading far past the end
    let mut buf = vec![0xAAu8; 64];
    let ops: Vec<ROp> = (0..40)
        .flat_map(|_| [ROp::Long, ROp::Char, ROp::U64])
        .collect();
    compare_read_ops(&mut buf, 9, &ops, "underrun");
}

#[test]
fn roundtrip_rust_write_c_read() {
    let _g = lock();
    // Rust writer bytes fed to the C reader (cross-direction coverage): the
    // decoded values must agree with the Rust reader's
    let mut rstore = vec![0u8; 4096];
    let mut rbuf = SizeBuf::new(&mut rstore);
    let flags = PRFL_24BITCOORD | PRFL_SHORTANGLE;
    msg::write_coord(&mut rbuf, 123.456, flags).unwrap();
    msg::write_angle(&mut rbuf, -179.5, flags).unwrap();
    msg::write_entity(&mut rbuf, 0x8123, PEXT2_REPLACEMENTDELTAS).unwrap();
    msg::write_uint64(&mut rbuf, 0x1234_5678).unwrap();
    msg::write_string(&mut rbuf, Some(b"quake")).unwrap();
    let cursize = rbuf.cursize;
    let ops = [
        ROp::Coord(flags),
        ROp::Angle(flags),
        ROp::Entity(PEXT2_REPLACEMENTDELTAS),
        ROp::U64,
        ROp::Str,
    ];
    compare_read_ops(rbuf.data, cursize, &ops, "rust->c roundtrip");
}

/// Golden pins for the ReadUInt64/WriteUInt64 COMPAT bug domain (values
/// needing >= 4 continuation bytes). The C original's `int` shifts are UB
/// that both supported architectures' variable-shift instructions resolve
/// by 31-masking; these vectors pin the observed encode bytes and (buggy)
/// decode values on BOTH the Rust port and the c_ref oracle, so an
/// optimization- or platform-dependent codegen change in either breaks the
/// gate loudly instead of silently shifting the wire format.
#[test]
fn uint64_bug_domain_goldens() {
    let _g = lock();
    const VALS: [u64; 6] = [
        (1u64 << 28) - 1,
        1u64 << 28,
        1u64 << 35,
        1u64 << 56,
        u64::MAX,
        0x123456789ABCDEF0,
    ];
    const GOLDEN_BYTES: [u8; 43] = [
        239, 255, 255, 255, 240, 16, 0, 0, 0, 248, 8, 0, 0, 0, 0, 255, 1, 0, 0, 0, 0, 0, 0, 0, 255,
        255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 18, 52, 86, 120, 154, 188, 222, 240,
    ];
    const GOLDEN_DECODE: [u64; 6] = [
        0xfffffff,
        0x10000000,
        0x8,       // 1<<35: masked-shift truncation
        0x1000000, // 1<<56: masked-shift truncation
        0xffffffffffffffff,
        0xffffffffff9abcde, // sign-extension of the shifted int
    ];

    // Rust writer + reader
    let mut store = vec![0u8; 4096];
    let mut sb = SizeBuf::new(&mut store);
    for &v in &VALS {
        msg::write_uint64(&mut sb, v).unwrap();
    }
    assert_eq!(sb.written(), GOLDEN_BYTES, "rust encode golden");
    let cursize = sb.cursize;
    let mut rd = quake_net::msg::MsgReader::begin(sb.data, cursize);
    for (i, &want) in GOLDEN_DECODE.iter().enumerate() {
        assert_eq!(rd.read_uint64(), want, "rust decode golden [{i}]");
    }

    // c_ref oracle writer + reader over the same vectors
    let mut cbuf = CBuf::new(4096, false);
    // SAFETY: serialized under TEST_LOCK; live sizebuf/globals
    unsafe {
        for &v in &VALS {
            c_ref_MSG_WriteUInt64(&mut cbuf.sb, v);
        }
        assert_eq!(cbuf.written(), GOLDEN_BYTES, "c_ref encode golden");
        c_ref_net_message.data = cbuf.store.as_mut_ptr();
        c_ref_net_message.maxsize = cbuf.store.len() as c_int;
        c_ref_net_message.cursize = cbuf.sb.cursize;
        c_ref_MSG_BeginReading();
        for (i, &want) in GOLDEN_DECODE.iter().enumerate() {
            assert_eq!(c_ref_MSG_ReadUInt64(), want, "c_ref decode golden [{i}]");
        }
    }
}
