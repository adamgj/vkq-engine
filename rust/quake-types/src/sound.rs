//! Sound-engine ABI mirrors (`Quake/q_sound.h`). Compat-critical: under
//! `-Duse_rust_snd` the Rust mixer walks `channel_t` arrays and `sfxcache_t`
//! blocks that C code (cl_demo.c, menu.c readers) also touches, so layout
//! drift is silent memory corruption. Verified per-platform by
//! `quake-ctest/tests/snd_abi.rs` against the engine's own headers.

use core::ffi::{c_char, c_int, c_uchar};

pub const MAX_QPATH: usize = 64;
pub const MAX_CHANNELS: usize = 1024;
pub const MAX_DYNAMIC_CHANNELS: usize = 128;
pub const MAX_RAW_SAMPLES: usize = 8192;
/// quakedef.h MAX_SOUNDS; known_sfx holds twice this (snd_dma.c)
pub const MAX_SOUNDS: usize = 2048;
pub const NUM_AMBIENTS: usize = 2;
pub const PAINTBUFFER_SIZE: usize = 2048;
pub const WAV_FORMAT_PCM: i32 = 1;

/// `portable_samplepair_t`
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct SamplePair {
    pub left: c_int,
    pub right: c_int,
}

/// `sfxcache_t`. The C struct ends in `byte data[1]` (variable sized); this
/// mirror keeps the one-byte array so `size_of` matches C and the PCM data
/// begins at `DATA_OFFSET`.
#[repr(C)]
pub struct SfxCache {
    pub length: c_int,
    pub loopstart: c_int,
    pub speed: c_int,
    pub width: c_int,
    pub stereo: c_int,
    pub data: [c_uchar; 1],
}

impl SfxCache {
    pub const DATA_OFFSET: usize = core::mem::offset_of!(SfxCache, data);
}

/// `sfx_t`
#[repr(C)]
pub struct Sfx {
    pub name: [c_char; MAX_QPATH],
    pub cache: *mut SfxCache,
}

/// `dma_t`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Dma {
    pub channels: c_int,
    /// mono samples in buffer
    pub samples: c_int,
    /// don't mix less than this #
    pub submission_chunk: c_int,
    /// in mono samples
    pub samplepos: c_int,
    pub samplebits: c_int,
    /// device opened for S8 format?
    pub signed8: c_int,
    pub speed: c_int,
    pub buffer: *mut c_uchar,
}

/// `channel_t`
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Channel {
    pub sfx: *mut Sfx,
    /// 0-255 volume
    pub leftvol: c_int,
    /// 0-255 volume
    pub rightvol: c_int,
    /// end time in global paintsamples
    pub end: c_int,
    /// sample position in sfx
    pub pos: c_int,
    /// where to loop, -1 = no looping
    pub looping: c_int,
    /// to allow overriding a specific sound
    pub entnum: c_int,
    pub entchannel: c_int,
    /// origin of sound effect
    pub origin: [f32; 3],
    /// distance multiplier (attenuation/clipK)
    pub dist_mult: f32,
    /// 0-255 master volume
    pub master_vol: c_int,
}

/// `wavinfo_t`
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct WavInfo {
    pub rate: c_int,
    pub width: c_int,
    pub channels: c_int,
    pub loopstart: c_int,
    pub samples: c_int,
    /// chunk starts this many bytes from file start
    pub dataofs: c_int,
}

// 64-bit layout pins; the per-platform gate is tests/snd_abi.rs
const _: () = {
    assert!(core::mem::size_of::<SamplePair>() == 8);
    assert!(core::mem::size_of::<SfxCache>() == 24);
    assert!(SfxCache::DATA_OFFSET == 20);
    assert!(core::mem::size_of::<Sfx>() == 72);
    assert!(core::mem::offset_of!(Sfx, cache) == 64);
    assert!(core::mem::size_of::<Dma>() == 40);
    assert!(core::mem::offset_of!(Dma, buffer) == 32);
    assert!(core::mem::size_of::<Channel>() == 56);
    assert!(core::mem::offset_of!(Channel, origin) == 36);
    assert!(core::mem::offset_of!(Channel, dist_mult) == 48);
    assert!(core::mem::offset_of!(Channel, master_vol) == 52);
    assert!(core::mem::size_of::<WavInfo>() == 24);
};
