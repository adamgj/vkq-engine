//! Localization (.loc / loc_english.txt) fuzzer: the parsing half of
//! LOC_LoadFile plus lookups and LOC_Format over the parsed data.

#![no_main]

use libfuzzer_sys::fuzz_target;
use quake_fs::loc;

fuzz_target!(|data: &[u8]| {
    let Ok(parsed) = loc::parse(data, &mut |_| {}) else {
        return;
    };

    // Fixed keys the engine really asks for, plus one derived from the
    // input head so mutated files can hit the found path.
    let _ = parsed.get_raw(b"$qc_ammo_shells");
    let _ = parsed.get_raw_str(b"$m_gameoptions");
    let head = &data[..data.len().min(24)];
    if let Some(keyish) = head.split(|&b| b == 0).next() {
        let mut key = Vec::with_capacity(keyish.len() + 1);
        key.push(b'$');
        key.extend_from_slice(keyish);
        if let Some(off) = parsed.get_raw(&key) {
            // A found value offset must be a valid NUL-terminated string in
            // the text image.
            let _ = parsed.str_at(off);
        }
    }

    // Every parsed entry's key/value offsets must resolve.
    for e in parsed.entries() {
        let _ = parsed.str_at(e.key);
        let _ = parsed.str_at(e.value);
    }

    // Format each entry's value as the format string, and the raw head as a
    // fallback, echoing placeholder indices back as arguments.
    let mut out = [0u8; 256];
    let mut fmt_inputs: Vec<&[u8]> = parsed
        .entries()
        .iter()
        .take(8)
        .map(|e| parsed.str_at(e.value))
        .collect();
    fmt_inputs.push(head);
    for fmt in fmt_inputs {
        let _ = parsed.has_placeholders(fmt);
        let written = loc::format(
            fmt,
            &mut |i| if i % 2 == 0 { b"arg".as_slice() } else { b"" },
            &mut out,
            &mut |_| {},
        );
        assert!(written < out.len());
        assert_eq!(out[written], 0);
    }
});
