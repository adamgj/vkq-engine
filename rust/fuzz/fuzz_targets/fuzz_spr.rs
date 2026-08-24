//! SPR (sprite model) fuzzer (Phase 3 M7, D11 / AC5): the pure quake-formats
//! sprite decision the `Mod_LoadSpriteModel` shim uses — the version and
//! frame-count gates, the group frame-count and interval fatals, and the
//! truncating model-bounds arithmetic. The C-via-FFI graph differential over
//! synthetic and real .spr files lives in `sprite_differential` and
//! `formats_corpus`.

#![no_main]

use libfuzzer_sys::fuzz_target;
use quake_formats::spr;

fuzz_target!(|data: &[u8]| {
    if data.len() < spr::DSPRITE_T_SIZE {
        return;
    }
    // shim reads version, then numframes, then the full header
    let version = spr::parse_version(&data[..spr::OFS_VERSION + 4]);
    let numframes = spr::parse_numframes(&data[..spr::OFS_NUMFRAMES + 4]);
    let h = spr::parse_header(&data[..spr::DSPRITE_T_SIZE]);
    assert_eq!((h.version, h.numframes), (version, numframes));

    // model bounds are a truncating negate-then-divide; must never panic for
    // any i32 pair (the value parity is checked in sprite_differential)
    let _ = spr::model_bounds(h.width, h.height);

    // the group fatals the shim raises Sys_Error on
    for ft in [spr::SPR_SINGLE, spr::SPR_GROUP, spr::SPR_ANGLED] {
        let _ = spr::group_frame_count_is_fatal(ft, numframes);
    }
    let body = &data[spr::DSPRITE_T_SIZE..];
    if body.len() >= 4 {
        let interval = spr::parse_interval(&body[..4]);
        // interval fatal <=> interval <= 0 (in the f64 domain, catching -0.0)
        assert_eq!(spr::interval_is_fatal(interval), f64::from(interval) <= 0.0);
    }
    if body.len() >= spr::DSPRITEFRAME_T_SIZE {
        let _ = spr::parse_frame(&body[..spr::DSPRITEFRAME_T_SIZE]);
    }
});
