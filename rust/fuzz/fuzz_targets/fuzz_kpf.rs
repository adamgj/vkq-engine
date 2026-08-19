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
//! a valid-enough archive can legally demand a multi-GB allocation from a
//! hundred-byte input. That is engine-inherited behavior (the C is a zip
//! bomb here too), not something this phase may change, so the cap below
//! keeps it from masquerading as a fuzz finding.

#![no_main]

use std::alloc::{GlobalAlloc, Layout, System};

use libfuzzer_sys::fuzz_target;
use quake_fs::zipdir;

/// Per-allocation cap for this target only. `extract_to_heap` requests the
/// claimed size through `Vec::try_reserve_exact`, so refusing an absurd
/// request here surfaces as the library's own `ZipError::AllocFailed` --
/// the path we actually want fuzzed -- instead of an OOM abort that says
/// nothing about parser correctness. Fuzz inputs are a few KB, so no
/// legitimate allocation comes close.
const ALLOC_CAP: usize = 256 << 20;

struct CappedAlloc;

// SAFETY: every request under the cap is forwarded verbatim to the system
// allocator, which upholds the GlobalAlloc contract; oversized requests
// return null, which is the documented "allocation failed" signal.
unsafe impl GlobalAlloc for CappedAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() > ALLOC_CAP {
            return std::ptr::null_mut();
        }
        // SAFETY: forwarding an unmodified layout to the system allocator.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if layout.size() > ALLOC_CAP {
            return std::ptr::null_mut();
        }
        // SAFETY: as above.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: ptr came from this allocator, i.e. from System, with the
        // same layout.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size > ALLOC_CAP {
            return std::ptr::null_mut();
        }
        // SAFETY: ptr came from System with `layout`; new_size is non-zero
        // and within the cap.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: CappedAlloc = CappedAlloc;

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
