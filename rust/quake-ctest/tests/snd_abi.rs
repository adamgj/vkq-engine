//! ABI cross-check: the `quake_types::sound` mirrors vs what the engine's own
//! `q_sound.h` says on this platform (Phase 4). Under `-Duse_rust_snd` the
//! Rust mixer walks `channel_t` arrays and `sfxcache_t` blocks that C code
//! also touches, so mirror drift is silent memory corruption rather than a
//! link error.
//!
//! Name-keyed like the Phase 3 probes so this consumer and the C table can't
//! drift by index; an unknown key returns usize::MAX and fails the assert.

use core::mem::{offset_of, size_of};

use quake_ctest as _;
use quake_types::sound::{Channel, Dma, SamplePair, Sfx, SfxCache, WavInfo};

extern "C" {
    fn ctest_abi_snd_lookup(key: *const core::ffi::c_char) -> usize;
}

fn c_abi(key: &str) -> usize {
    let cstr = std::ffi::CString::new(key).unwrap();
    // SAFETY: the probe only strcmp's the key against a compile-time table.
    let v = unsafe { ctest_abi_snd_lookup(cstr.as_ptr()) };
    assert_ne!(v, usize::MAX, "key {key:?} missing from the C probe table");
    v
}

macro_rules! check_size {
    ($rust:ty, $ctag:literal) => {
        assert_eq!(
            size_of::<$rust>(),
            c_abi(concat!("sizeof.", $ctag)),
            concat!("sizeof ", $ctag)
        );
    };
}

macro_rules! check_offsets {
    ($rust:ty, $ctag:literal, [$($field:ident),+ $(,)?]) => {
        $(
            assert_eq!(
                offset_of!($rust, $field),
                c_abi(concat!($ctag, ".", stringify!($field))),
                concat!($ctag, ".", stringify!($field))
            );
        )+
    };
}

#[test]
fn snd_mirrors_match_engine_headers() {
    check_size!(SamplePair, "portable_samplepair_t");
    check_offsets!(SamplePair, "portable_samplepair_t", [left, right]);

    check_size!(SfxCache, "sfxcache_t");
    check_offsets!(
        SfxCache,
        "sfxcache_t",
        [length, loopstart, speed, width, stereo, data]
    );

    check_size!(Sfx, "sfx_t");
    check_offsets!(Sfx, "sfx_t", [name, cache]);

    check_size!(Dma, "dma_t");
    check_offsets!(
        Dma,
        "dma_t",
        [channels, samples, submission_chunk, samplepos, samplebits, signed8, speed, buffer]
    );

    check_size!(Channel, "channel_t");
    check_offsets!(
        Channel,
        "channel_t",
        [sfx, leftvol, rightvol, end, pos, looping, entnum, entchannel, origin, dist_mult, master_vol]
    );

    check_size!(WavInfo, "wavinfo_t");
    check_offsets!(
        WavInfo,
        "wavinfo_t",
        [rate, width, channels, loopstart, samples, dataofs]
    );
}

#[test]
fn snd_consts_match_engine_headers() {
    use quake_types::sound;
    assert_eq!(sound::MAX_CHANNELS, c_abi("const.MAX_CHANNELS"));
    assert_eq!(
        sound::MAX_DYNAMIC_CHANNELS,
        c_abi("const.MAX_DYNAMIC_CHANNELS")
    );
    assert_eq!(sound::MAX_RAW_SAMPLES, c_abi("const.MAX_RAW_SAMPLES"));
    assert_eq!(sound::MAX_QPATH, c_abi("const.MAX_QPATH"));
    assert_eq!(sound::NUM_AMBIENTS, c_abi("const.NUM_AMBIENTS"));
    assert_eq!(sound::MAX_SOUNDS, c_abi("const.MAX_SOUNDS"));
}
