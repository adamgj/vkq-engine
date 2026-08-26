//! MP3 tag-skipper fuzzer (Phase 4): drives the pure predicates and length
//! computations of `quake_snd::mp3tag` over arbitrary bytes, asserting no
//! panics.
//!
//! Why this matters more than the other sound parsers: `mp3_skiptags` is
//! compiled into every default build (`USE_CODEC_MP3` is on whenever mpg123
//! or mad is found), and it runs over arbitrary user-supplied music files
//! *before* any decoder validates them. Unlike the C original, a slice
//! mis-index here is a panic-abort rather than a stray read, so an
//! out-of-bounds guess is a denial of service on a file the C would have
//! survived.
//!
//! Pure predicates only, matching the Phase 3 format targets: the c_ref
//! oracle's `Sys_Error` cannot be trapped across a Rust frame, so the
//! C-vs-Rust differential lives in quake-ctest's `snd_codec_differential`
//! instead.

#![no_main]

use libfuzzer_sys::fuzz_target;
use quake_snd::mp3tag;

fuzz_target!(|data: &[u8]| {
    // Each predicate does its own length check; feed the whole buffer and
    // every prefix boundary the real caller can produce. mp3_skiptags reads
    // into a 128-byte stack buffer and slices it at fixed offsets, so those
    // exact window sizes are the interesting ones.
    let _ = mp3tag::is_id3v1(data);
    let _ = mp3tag::is_id3v2(data);
    let _ = mp3tag::is_apetag(data);
    let _ = mp3tag::is_lyrics3tag(data);
    let _ = mp3tag::verify_lyrics3v2(data);
    let _ = mp3tag::is_musicmatch(data);

    // the length computations are only reached once the matching predicate
    // accepted -- honour that contract, the same way the shim does
    if mp3tag::is_id3v2(data) {
        // the C passes the bytes actually read (<= 128) as `length`
        let len = data.len().min(128) as core::ffi::c_long;
        let _ = mp3tag::get_id3v2_len(data, len);
    }
    if mp3tag::is_apetag(data) {
        let _ = mp3tag::get_ape_len(data);
    }
    // get_lyrics3v2_len is called with exactly the 6-byte size field
    if data.len() >= 6 {
        let _ = mp3tag::get_lyrics3v2_len(&data[..6], 6);
    }
    let _ = mp3tag::get_lyrics3v2_len(data, data.len() as core::ffi::c_long);

    // the 128-byte probe windows mp3_skiptags carves out of its read buffer
    if data.len() >= 128 {
        let buf = &data[..128];
        if mp3tag::is_id3v1(buf) {
            let _ = mp3tag::is_musicmatch(&buf[128 - 48..]);
            let _ = mp3tag::is_apetag(&buf[128 - 32..]);
            let _ = mp3tag::is_lyrics3tag(&buf[128 - 15..]);
        }
    }
});
