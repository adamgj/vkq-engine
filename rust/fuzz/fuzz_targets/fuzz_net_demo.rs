//! Demo file-format fuzzer (Phase 5 M4): the forcetrack header-line parse
//! (fscanf "%i" + newline semantics) and the record-header decode, over
//! hostile inputs -- truncation, oversize/negative lengths, non-decimal
//! forms. Asserts no panics plus write->parse roundtrips of the canonical
//! encodings. The engine-level byte gate is scripts/harness/record_diff.py.

#![no_main]

use libfuzzer_sys::fuzz_target;
use quake_net::demo;

fuzz_target!(|data: &[u8]| {
    // hostile forcetrack lines never panic; accepted ones end in '\n'
    if let Some((_, consumed)) = demo::parse_forcetrack(data) {
        assert!(consumed <= data.len());
        assert_eq!(data[consumed - 1], b'\n');
    }

    // hostile record headers never panic; accepted lengths are <= MAX_MSGLEN
    if data.len() >= demo::RECORD_HEADER_SIZE {
        let h: [u8; demo::RECORD_HEADER_SIZE] =
            data[..demo::RECORD_HEADER_SIZE].try_into().unwrap();
        if let Some((len, _)) = demo::parse_record_header(&h) {
            assert!(len <= demo::MAX_MSGLEN);
        }
    }

    // canonical roundtrips
    if data.len() >= 16 {
        let track = i32::from_le_bytes(data[0..4].try_into().unwrap());
        let line = demo::forcetrack_line(track);
        assert_eq!(demo::parse_forcetrack(&line), Some((track, line.len())));

        let len = i32::from_le_bytes(data[4..8].try_into().unwrap()) & 0x7fff;
        let ang = [
            f32::from_le_bytes(data[8..12].try_into().unwrap()),
            f32::from_le_bytes(data[12..16].try_into().unwrap()),
            0.0,
        ];
        let h = demo::record_header(len, ang);
        let (l2, a2) = demo::parse_record_header(&h).unwrap();
        assert_eq!(l2, len);
        assert_eq!(a2[0].to_bits(), ang[0].to_bits());
        assert_eq!(a2[1].to_bits(), ang[1].to_bits());
    }
});
