//! Phase 4 M7: codec framework differential -- the quake-capi S_Codec*
//! registry, native WAV/UMX codecs and mp3_skiptags vs the c_ref-compiled
//! snd_codec.c/snd_wave.c/snd_umx.c/snd_mp3tag.c, over synthetic fixture
//! files (self-generated, license-clean per ADR-019). Both sides register
//! the same codec set: UMX + WAV natives plus an identical dummy MP3 vtable.

use core::ffi::{c_char, c_int, c_uint, c_void};

use quake_ctest::fs as ctfs;
use quake_ctest::fs::Side;

use quake_c_sys as sys;
use sys::{fshandle_t, snd_stream_t};

extern "C" {
    fn c_ref_S_CodecInit();
    fn c_ref_S_CodecShutdown();
    fn c_ref_S_CodecIsAvailable(type_: c_uint) -> c_int;
    fn c_ref_S_CodecOpenStreamAny(filename: *const c_char, loop_: bool) -> *mut snd_stream_t;
    fn c_ref_S_CodecOpenStreamExt(filename: *const c_char, loop_: bool) -> *mut snd_stream_t;
    fn c_ref_S_CodecReadStream(
        stream: *mut snd_stream_t,
        bytes: c_int,
        buffer: *mut c_void,
    ) -> c_int;
    fn c_ref_S_CodecRewindStream(stream: *mut snd_stream_t) -> c_int;
    fn c_ref_S_CodecCloseStream(stream: *mut snd_stream_t);
    fn c_ref_mp3_skiptags(stream: *mut snd_stream_t) -> c_int;

    fn ctest_open_fshandle(path: *const c_char, fh: *mut fshandle_t) -> c_int;
}

fn cstr(s: &str) -> std::ffi::CString {
    std::ffi::CString::new(s).unwrap()
}

// ---------------------------------------------------------------------------
// fixtures

fn wav_file(
    rate: u32,
    bits: u16,
    channels: u16,
    nsamples: usize,
    extra_fmt: usize,
    junk_chunk: bool,
) -> Vec<u8> {
    let width = (bits / 8) as usize;
    let mut data = Vec::new();
    let mut state = 0x1357u32;
    for _ in 0..nsamples * channels as usize {
        state = state.wrapping_mul(1103515245).wrapping_add(12345);
        if width == 1 {
            data.push((state >> 16) as u8);
        } else {
            data.extend_from_slice(&((state >> 8) as i16).to_le_bytes());
        }
    }
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&1u16.to_le_bytes());
    fmt.extend_from_slice(&channels.to_le_bytes());
    fmt.extend_from_slice(&rate.to_le_bytes());
    fmt.extend_from_slice(&(rate * channels as u32 * width as u32).to_le_bytes());
    fmt.extend_from_slice(&((channels as usize * width) as u16).to_le_bytes());
    fmt.extend_from_slice(&bits.to_le_bytes());
    fmt.extend(std::iter::repeat_n(0u8, extra_fmt));

    let chunk = |id: &[u8; 4], body: &[u8]| {
        let mut v = Vec::new();
        v.extend_from_slice(id);
        v.extend_from_slice(&(body.len() as u32).to_le_bytes());
        v.extend_from_slice(body);
        if body.len() % 2 == 1 {
            v.push(0);
        }
        v
    };

    let mut inner = Vec::new();
    inner.extend_from_slice(b"WAVE");
    if junk_chunk {
        inner.extend_from_slice(&chunk(b"JUNK", &[0xAA; 13]));
    }
    inner.extend_from_slice(&chunk(b"fmt ", &fmt));
    inner.extend_from_slice(&chunk(b"data", &data));

    let mut v = Vec::new();
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(inner.len() as u32).to_le_bytes());
    v.extend_from_slice(&inner);
    v
}

/// Minimal upkg container: header (v61 or v68 layout), a name table with the
/// music-type name, one export entry pointing at the embedded object.
fn umx_file(version: i32, typename: &[u8], object: &[u8]) -> Vec<u8> {
    // FCompactIndex encode (non-negative values)
    fn fci(v: i32) -> Vec<u8> {
        assert!(v >= 0);
        let mut out = Vec::new();
        let mut rest = (v as u32) >> 6;
        out.push(((v as u32) & 0x3f) as u8 | if rest != 0 { 0x40 } else { 0 });
        while rest != 0 {
            let more = rest >> 7;
            out.push((rest & 0x7f) as u8 | if more != 0 { 0x80 } else { 0 });
            rest = more;
        }
        out
    }

    let mut f = vec![0u8; 64]; // header

    // name table at 64: entry 0 = typename
    let name_offset = 64usize;
    let mut names = Vec::new();
    if version >= 64 {
        names.push((typename.len() + 1) as u8);
        names.extend_from_slice(typename);
        names.push(0);
        names.extend_from_slice(&[0; 4]); // name_flags
    } else {
        names.extend_from_slice(typename);
        names.push(0);
        names.extend_from_slice(&[0; 4]);
    }

    // export table
    let export_offset = name_offset + names.len();
    let mut export = Vec::new();
    export.extend_from_slice(&fci(0)); // class_index
    export.extend_from_slice(&fci(0)); // super_index
    if version >= 60 {
        export.extend_from_slice(&[0; 4]); // package_index
    }
    export.extend_from_slice(&fci(0)); // object_name
    export.extend_from_slice(&[0; 4]); // object_flags
    export.extend_from_slice(&fci(object.len() as i32)); // serial_size

    // the serialized object record begins with the same prelude read_export
    // parses: junk fci, type_name fci, (v>61: export size int32), objsize fci
    let mut record = Vec::new();
    record.extend_from_slice(&fci(0)); // junk
    record.extend_from_slice(&fci(0)); // type_name -> name index 0
    if version > 61 {
        record.extend_from_slice(&[0; 4]); // export size
    }
    record.extend_from_slice(&fci(object.len() as i32)); // objsize

    let serial_offset = export_offset + export.len() + 1 /* serial_offset fci */;
    export.extend_from_slice(&fci(serial_offset as i32));

    f.extend_from_slice(&names);
    f.extend_from_slice(&export);
    f.extend_from_slice(&record);
    f.extend_from_slice(object);
    // read_export wants 40 readable bytes at serial_offset and probe wants
    // serial_offset <= fsiz - 40; pad the tail
    f.extend(std::iter::repeat_n(0u8, 64));

    let le = |v: i32| v.to_le_bytes();
    f[0..4].copy_from_slice(&0x9e2a83c1u32.to_le_bytes()); // tag
    f[4..8].copy_from_slice(&le(version));
    f[12..16].copy_from_slice(&le(1)); // name_count
    f[16..20].copy_from_slice(&le(name_offset as i32));
    f[20..24].copy_from_slice(&le(1)); // export_count
    f[24..28].copy_from_slice(&le(export_offset as i32));
    f[28..32].copy_from_slice(&le(0)); // import_count
    f[32..36].copy_from_slice(&le(export_offset as i32)); // import_offset (unused)
    f
}

// ---------------------------------------------------------------------------
// mp3_skiptags fixtures

fn id3v2(len: usize, footer: bool, padding: usize) -> Vec<u8> {
    // len = payload size of the tag (synchsafe), excluding the 10-byte header
    let mut v = Vec::new();
    v.extend_from_slice(b"ID3");
    v.push(4);
    v.push(0);
    v.push(if footer { 0x10 } else { 0 });
    v.push(((len >> 21) & 0x7f) as u8);
    v.push(((len >> 14) & 0x7f) as u8);
    v.push(((len >> 7) & 0x7f) as u8);
    v.push((len & 0x7f) as u8);
    v.extend(std::iter::repeat_n(0x41u8, len));
    if footer {
        v.extend_from_slice(b"3DI");
        v.extend(std::iter::repeat_n(0u8, 7));
    }
    v.extend(std::iter::repeat_n(0u8, padding));
    v
}

fn id3v1() -> Vec<u8> {
    let mut v = vec![0u8; 128];
    v[0..3].copy_from_slice(b"TAG");
    for (i, b) in v.iter_mut().enumerate().skip(3) {
        *b = b'a' + (i % 20) as u8;
    }
    v
}

fn apetag(version: u32, items_len: usize, header_flag: bool, with_header: bool) -> Vec<u8> {
    // footer (and optional identical header): size covers items + footer
    let size = (items_len + 32) as u32;
    let mut footer = Vec::new();
    footer.extend_from_slice(b"APETAGEX");
    footer.extend_from_slice(&version.to_le_bytes());
    footer.extend_from_slice(&size.to_le_bytes());
    footer.extend_from_slice(&0u32.to_le_bytes()); // item count
    footer.extend_from_slice(&(if header_flag { 1u32 << 31 } else { 0 }).to_le_bytes());
    footer.extend_from_slice(&[0; 8]); // reserved

    let mut v = Vec::new();
    if with_header {
        v.extend_from_slice(&footer); // header copy (close enough for the probe)
    }
    v.extend(std::iter::repeat_n(0x42u8, items_len));
    v.extend_from_slice(&footer);
    v
}

fn lyrics3v2(body_len: usize) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"LYRICSBEGIN");
    v.extend(std::iter::repeat_n(b'x', body_len));
    let size = v.len();
    v.extend_from_slice(format!("{size:06}").as_bytes());
    v.extend_from_slice(b"LYRICS200");
    v
}

fn lyrics3v1(body_len: usize) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"LYRICSBEGIN");
    v.extend(std::iter::repeat_n(b'y', body_len));
    v.extend_from_slice(b"LYRICSEND");
    v
}

fn musicmatch(img_len: usize, with_header: bool) -> Vec<u8> {
    let metasize = 7868usize;
    let syncstr: [u8; 10] = *b"18273645\0\0";

    let mut version_info = vec![b' '; 256];
    version_info[0..10].copy_from_slice(&syncstr);
    // bytes 10..30 are unconstrained; leave spaces

    let mut footer = Vec::new();
    footer.extend_from_slice(b"Brava Software Inc.             ");
    footer.extend_from_slice(b"3.00");
    footer.extend_from_slice(&[b' '; 12]);
    assert_eq!(footer.len(), 48);

    let imgext_ofs: i32 = 1000; // arbitrary positive
    let version_ofs: i32 = imgext_ofs + img_len as i32 + 12;

    let mut offsets = vec![0u8; 20];
    offsets[0..4].copy_from_slice(&imgext_ofs.to_le_bytes());
    offsets[12..16].copy_from_slice(&version_ofs.to_le_bytes());

    let mut v = Vec::new();
    if with_header {
        v.extend_from_slice(&version_info); // 256-byte header (same shape)
    }
    v.extend_from_slice(&[0u8; 4]); // image ext
    v.extend_from_slice(&(img_len as i32).to_le_bytes()); // image size field
    v.extend(std::iter::repeat_n(0x43u8, img_len.saturating_sub(4))); // image binary
    v.extend_from_slice(&[0u8; 4]); // unused (must be zero)
    v.extend_from_slice(&version_info);
    v.extend(std::iter::repeat_n(0x44u8, metasize));
    v.extend_from_slice(&offsets);
    v.extend_from_slice(&footer);
    v
}

// ---------------------------------------------------------------------------

fn write_temp(dir: &std::path::Path, name: &str, data: &[u8]) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, data).unwrap();
    p
}

fn snap_fh(fh: &fshandle_t) -> (i64, i64, i64) {
    (fh.start, fh.length, fh.pos)
}

#[test]
fn mp3_skiptags_differential() {
    let _guard = ctfs::lock();
    let dir = std::env::temp_dir().join(format!("vkqr_mp3tag_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let payload: Vec<u8> = (0..3000u32).map(|i| (i * 37 + 5) as u8).collect();

    let mut cases: Vec<(&str, Vec<u8>)> = Vec::new();
    cases.push(("plain", payload.clone()));
    cases.push(("id3v2", {
        let mut v = id3v2(200, false, 0);
        v.extend(&payload);
        v
    }));
    cases.push(("id3v2-footer", {
        let mut v = id3v2(64, true, 0);
        v.extend(&payload);
        v
    }));
    cases.push(("id3v2-pad", {
        let mut v = id3v2(64, false, 30);
        v.extend(&payload);
        v
    }));
    cases.push(("id3v1", {
        let mut v = payload.clone();
        v.extend(id3v1());
        v
    }));
    cases.push(("ape2-end", {
        let mut v = payload.clone();
        v.extend(apetag(2000, 100, true, true));
        v
    }));
    cases.push(("ape1-end", {
        let mut v = payload.clone();
        v.extend(apetag(1000, 60, false, false));
        v
    }));
    cases.push(("ape-start", {
        let mut v = apetag(2000, 80, false, false);
        v.extend(&payload);
        v
    }));
    cases.push(("lyrics3v2", {
        let mut v = payload.clone();
        v.extend(lyrics3v2(500));
        v
    }));
    cases.push(("lyrics3v1", {
        let mut v = payload.clone();
        v.extend(lyrics3v1(700));
        v
    }));
    cases.push(("musicmatch", {
        let mut v = payload.clone();
        v.extend(musicmatch(500, false));
        v
    }));
    cases.push(("musicmatch-hdr", {
        let mut v = payload.clone();
        v.extend(musicmatch(500, true));
        v
    }));
    cases.push(("id3v1+ape", {
        let mut v = payload.clone();
        v.extend(apetag(2000, 100, true, true));
        v.extend(id3v1());
        v
    }));
    cases.push(("id3v1+lyr+ape", {
        let mut v = payload.clone();
        v.extend(apetag(1000, 40, false, false));
        v.extend(lyrics3v2(120));
        v.extend(id3v1());
        v
    }));
    cases.push(("mm+id3v1", {
        let mut v = payload.clone();
        v.extend(musicmatch(300, false));
        v.extend(id3v1());
        v
    }));
    cases.push(("hostile-id3v2-too-big", id3v2(5000, false, 0)));
    cases.push(("hostile-ape-huge", {
        let mut v = payload[..100].to_vec();
        v.extend(apetag(2000, 100000, true, false));
        v
    }));
    cases.push(("tiny", payload[..10].to_vec()));
    cases.push(("empty", Vec::new()));

    for (tag, data) in &cases {
        let path = write_temp(&dir, &format!("{tag}.bin"), data);
        let cpath = cstr(path.to_str().unwrap());

        let run = |func: unsafe extern "C" fn(*mut snd_stream_t) -> c_int| {
            // SAFETY: fixture file exists; stream is zeroed then given a
            // fresh OS handle; single-threaded under the fs lock
            unsafe {
                let mut stream: snd_stream_t = core::mem::zeroed();
                assert_eq!(
                    ctest_open_fshandle(cpath.as_ptr(), &mut stream.fh),
                    0,
                    "{tag}: open"
                );
                ctfs::clear_logs();
                let rc = func(&mut stream);
                let log = ctfs::con_log();
                let snap = snap_fh(&stream.fh);
                sys::stdio::fclose(stream.fh.file);
                (rc, snap, log)
            }
        };

        let c = run(c_ref_mp3_skiptags);
        let r = run(sys_mp3_skiptags);
        assert_eq!(c, r, "{tag}: mp3_skiptags (rc, fh, console)");
    }

    std::fs::remove_dir_all(&dir).ok();
}

// the capi export (linked via quake-capi's snd + codec-mp3 features)
extern "C" {
    #[link_name = "mp3_skiptags"]
    fn sys_mp3_skiptags(stream: *mut snd_stream_t) -> c_int;
}

// ---------------------------------------------------------------------------
// framework + wav/umx codec parity over mounted gamedirs

const CODECTYPE_WAV: c_uint = 1 << 3;
const CODECTYPE_MP3: c_uint = 1 << 4;
const CODECTYPE_UMX: c_uint = 1 << 7;
const CODECTYPE_VORBIS: c_uint = 1 << 5;

fn read_all(
    read: unsafe extern "C" fn(*mut snd_stream_t, c_int, *mut c_void) -> c_int,
    stream: *mut snd_stream_t,
    chunks: &[usize],
) -> (Vec<u8>, Vec<i32>) {
    let mut out = Vec::new();
    let mut rets = Vec::new();
    let mut i = 0;
    loop {
        let want = chunks[i % chunks.len()];
        i += 1;
        let mut buf = vec![0u8; want];
        // SAFETY: buf sized to want
        let got = unsafe { read(stream, want as c_int, buf.as_mut_ptr().cast()) };
        rets.push(got);
        if got <= 0 || rets.len() > 4096 {
            break;
        }
        out.extend_from_slice(&buf[..got as usize]);
    }
    (out, rets)
}

#[test]
fn codec_framework_differential() {
    let guard = ctfs::lock();
    let root = std::env::temp_dir().join(format!("vkqr_codec_{}", std::process::id()));
    let music = root.join("testgame").join("music");
    std::fs::create_dir_all(&music).unwrap();

    write_temp(&music, "good.wav", &wav_file(11025, 16, 2, 3000, 0, false));
    write_temp(&music, "mono8.wav", &wav_file(22050, 8, 1, 2777, 0, false));
    write_temp(&music, "bigfmt.wav", &wav_file(44100, 16, 2, 500, 24, true));
    write_temp(
        &music,
        "trunc.wav",
        &wav_file(11025, 16, 2, 3000, 0, false)[..100],
    );
    write_temp(&music, "notpcm.wav", {
        let mut v = wav_file(11025, 16, 1, 100, 0, false);
        v[20] = 2; // format 2
        &v.clone()
    });
    write_temp(&music, "noext", &wav_file(11025, 16, 2, 64, 0, false));
    write_temp(&music, "song.xyz", b"not audio");
    write_temp(
        &music,
        "wrapped61.umx",
        &umx_file(61, b"wav", &wav_file(11025, 16, 1, 400, 0, false)),
    );
    write_temp(
        &music,
        "wrapped68.umx",
        &umx_file(68, b"WAV", &wav_file(22050, 8, 1, 900, 0, false)),
    );
    write_temp(
        &music,
        "badumx.umx",
        b"\xc1\x83\x2a\x9egarbage_not_long_enough",
    );

    ctfs::setup(Side::C, &[&root], 0, &cstr("testgame"));
    ctfs::setup(Side::Rust, &[&root], 0, &cstr("testgame"));

    // SAFETY: both registries built once; single-threaded under the lock
    unsafe {
        c_ref_S_CodecInit();
        sys_S_CodecInit();
    }

    for t in [
        CODECTYPE_WAV,
        CODECTYPE_MP3,
        CODECTYPE_UMX,
        CODECTYPE_VORBIS,
    ] {
        // SAFETY: plain queries
        let (c, r) = unsafe { (c_ref_S_CodecIsAvailable(t), sys_S_CodecIsAvailable(t)) };
        assert_eq!(c, r, "S_CodecIsAvailable({t})");
    }

    let names = [
        "music/good.wav",
        "music/mono8.wav",
        "music/bigfmt.wav",
        "music/trunc.wav",
        "music/notpcm.wav",
        "music/noext",
        "music/song.xyz",
        "music/missing.wav",
        "music/wrapped61.umx",
        "music/wrapped68.umx",
        "music/badumx.umx",
    ];

    for name in names {
        let cname = cstr(name);
        for use_any in [false, true] {
            // SAFETY: mounted searchpaths per side; streams closed before the
            // sides swap; single-threaded under the lock
            unsafe {
                ctfs::clear_logs();
                let cs = if use_any {
                    c_ref_S_CodecOpenStreamAny(cname.as_ptr(), false)
                } else {
                    c_ref_S_CodecOpenStreamExt(cname.as_ptr(), false)
                };
                let c_log = ctfs::con_log();
                let c_open = !cs.is_null();
                let c_result = if c_open {
                    let info = (*cs).info;
                    let (bytes, rets) = read_all(c_ref_S_CodecReadStream, cs, &[1, 7, 4096, 333]);
                    let rw = c_ref_S_CodecRewindStream(cs);
                    let (bytes2, _) = read_all(c_ref_S_CodecReadStream, cs, &[512]);
                    c_ref_S_CodecCloseStream(cs);
                    Some((info, bytes, rets, rw, bytes2))
                } else {
                    None
                };

                ctfs::clear_logs();
                let rs = if use_any {
                    sys_S_CodecOpenStreamAny(cname.as_ptr(), false)
                } else {
                    sys_S_CodecOpenStreamExt(cname.as_ptr(), false)
                };
                let r_log = ctfs::con_log();
                let r_open = !rs.is_null();
                let r_result = if r_open {
                    let info = (*rs).info;
                    let (bytes, rets) = read_all(sys_S_CodecReadStream, rs, &[1, 7, 4096, 333]);
                    let rw = sys_S_CodecRewindStream(rs);
                    let (bytes2, _) = read_all(sys_S_CodecReadStream, rs, &[512]);
                    sys_S_CodecCloseStream(rs);
                    Some((info, bytes, rets, rw, bytes2))
                } else {
                    None
                };

                assert_eq!(c_log, r_log, "{name} any={use_any}: console");
                assert_eq!(c_open, r_open, "{name} any={use_any}: open success");
                match (c_result, r_result) {
                    (Some((ci, cb, crets, crw, cb2)), Some((ri, rb, rrets, rrw, rb2))) => {
                        assert_eq!(
                            (
                                ci.rate,
                                ci.bits,
                                ci.width,
                                ci.channels,
                                ci.samples,
                                ci.size,
                                ci.dataofs
                            ),
                            (
                                ri.rate,
                                ri.bits,
                                ri.width,
                                ri.channels,
                                ri.samples,
                                ri.size,
                                ri.dataofs
                            ),
                            "{name} any={use_any}: snd_info_t"
                        );
                        assert_eq!(cb, rb, "{name} any={use_any}: streamed bytes");
                        assert_eq!(crets, rrets, "{name} any={use_any}: read returns");
                        assert_eq!(crw, rrw, "{name} any={use_any}: rewind rc");
                        assert_eq!(cb2, rb2, "{name} any={use_any}: post-rewind bytes");
                    }
                    (None, None) => {}
                    _ => unreachable!(),
                }
            }
        }
    }

    // SAFETY: teardown
    unsafe {
        c_ref_S_CodecShutdown();
        sys_S_CodecShutdown();
    }
    drop(guard);
    std::fs::remove_dir_all(&root).ok();
}

extern "C" {
    #[link_name = "S_CodecInit"]
    fn sys_S_CodecInit();
    #[link_name = "S_CodecShutdown"]
    fn sys_S_CodecShutdown();
    #[link_name = "S_CodecIsAvailable"]
    fn sys_S_CodecIsAvailable(type_: c_uint) -> c_int;
    #[link_name = "S_CodecOpenStreamAny"]
    fn sys_S_CodecOpenStreamAny(filename: *const c_char, loop_: bool) -> *mut snd_stream_t;
    #[link_name = "S_CodecOpenStreamExt"]
    fn sys_S_CodecOpenStreamExt(filename: *const c_char, loop_: bool) -> *mut snd_stream_t;
    #[link_name = "S_CodecReadStream"]
    fn sys_S_CodecReadStream(stream: *mut snd_stream_t, bytes: c_int, buffer: *mut c_void)
        -> c_int;
    #[link_name = "S_CodecRewindStream"]
    fn sys_S_CodecRewindStream(stream: *mut snd_stream_t) -> c_int;
    #[link_name = "S_CodecCloseStream"]
    fn sys_S_CodecCloseStream(stream: *mut snd_stream_t);
}
