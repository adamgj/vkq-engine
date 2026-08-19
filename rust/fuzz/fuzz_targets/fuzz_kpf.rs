//! .kpf (zip) fuzzer: quake_fs::zipdir::ZipArchive over an arbitrary
//! archive image, plus the raw embedded-pak inflate path.
//!
//! Extraction is attempted for the one name the engine actually asks a kpf
//! for (localization/loc_english.txt) and for a name sliced from the input's
//! tail, so mutated archives can still hit the locate/extract paths with
//! matching entry names.
//!
//! Note: extract() allocates the central directory's claimed uncomp_size up
//! front, exactly like miniz/the C (mz_zip_reader_extract_file_to_heap), so
//! a valid-enough archive can legally demand a multi-GB allocation. That is
//! engine-inherited behavior, not a harness bug; -rss_limit_mb bounds it at
//! runtime.

#![no_main]

use libfuzzer_sys::fuzz_target;
use quake_fs::zipdir;

fuzz_target!(|data: &[u8]| {
    if let Ok(archive) = zipdir::ZipArchive::open(data) {
        let _ = archive.extract(b"localization/loc_english.txt");
        // A name candidate from the tail (where zip filenames live, right
        // before the EOCD): up to 32 bytes, NUL-free like a C string.
        let tail_start = data.len().saturating_sub(54).saturating_sub(32);
        let tail = &data[tail_start..data.len().saturating_sub(54).max(tail_start)];
        if let Some(name) = tail.split(|&b| b == 0).next() {
            let _ = archive.extract(name);
        }
        // And the head, for archives whose first local header's name mutated.
        let head = &data[..data.len().min(64)];
        if let Some(name) = head.get(30..).map(|h| h.split(|&b| b == 0).next().unwrap_or(b"")) {
            let _ = archive.extract(name);
        }
    }

    // The embedded vkquake.pak path: raw tinfl over the input with a
    // HARNESS-CAPPED output size (first 3 bytes, max 1 MiB) purely to keep
    // the by-design "allocate whatever the caller asks for" behavior from
    // reading as an OOM finding; the library itself takes any size.
    if data.len() >= 3 {
        let size = usize::from(data[0])
            | (usize::from(data[1]) << 8)
            | (usize::from(data[2]) << 16);
        let size = size.min(1 << 20);
        let _ = zipdir::inflate_embedded(&data[3..], size);
    }
});
