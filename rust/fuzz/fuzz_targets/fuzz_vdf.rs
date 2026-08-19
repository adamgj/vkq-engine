//! Valve KeyValues (.vdf/.acf) fuzzer: the Steam discovery parser plus the
//! two engine queries built on it (libraryfolders lookup, acf installdir).

#![no_main]

use libfuzzer_sys::fuzz_target;
use quake_fs::vdf;

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }
    let appid = i32::from_le_bytes(data[0..4].try_into().unwrap());
    let text = &data[4..];

    let mut pairs = 0usize;
    let ok = vdf::parse(text, &mut |path, key, value| {
        pairs += 1;
        assert!(path.len() <= vdf::MAX_DEPTH);
        let _ = (key.len(), value.len());
    });
    if !ok {
        // an aborted walk may still have delivered earlier pairs
        let _ = pairs;
    }

    // The Quake appid the engine really uses, plus a fuzz-chosen one.
    let _ = vdf::find_library_for_app(text, 2310);
    let _ = vdf::find_library_for_app(text, appid);
    let _ = vdf::acf_installdir(text);
});
