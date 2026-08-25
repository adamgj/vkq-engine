//! Synthetic PCX/LMP/TGA images for the image differentials. Same builders
//! as quake-image's own unit tests, shared by `image_differential` (byte
//! parity), `image_crate_differential` (the M8 stb seam) and
//! `threaded_parse` (concurrent decode) so the suites cannot drift apart.

/// Minimal valid PCX with a deterministic non-trivial palette. `rle` is the
/// raw encoded scanline data; `bytes_per_line` may exceed `w` to exercise the
/// padding write.
pub fn build_pcx(w: u16, h: u16, bytes_per_line: u16, rle: &[u8]) -> Vec<u8> {
    let mut f = vec![0u8; 128];
    f[0] = 0x0A;
    f[1] = 5;
    f[2] = 1;
    f[3] = 8;
    f[8..10].copy_from_slice(&(w - 1).to_le_bytes());
    f[10..12].copy_from_slice(&(h - 1).to_le_bytes());
    f[65] = 1;
    f[66..68].copy_from_slice(&bytes_per_line.to_le_bytes());
    f.extend_from_slice(rle);
    let mut palette = [0u8; 768];
    for (i, b) in palette.iter_mut().enumerate() {
        *b = (i % 251) as u8;
    }
    f.extend_from_slice(&palette);
    f
}

pub fn build_lmp(w: u32, h: u32, pixels: &[u8]) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(&w.to_le_bytes());
    f.extend_from_slice(&h.to_le_bytes());
    f.extend_from_slice(pixels);
    f
}

/// The 18-byte TGA header, every field addressable (malformed headers
/// included — the sniff/reject differentials need them).
#[derive(Clone, Copy, Default)]
pub struct TgaHeader {
    pub offset: u8,
    pub colormap_type: u8,
    pub image_type: u8,
    pub palette_start: u16,
    pub palette_len: u16,
    pub palette_bits: u8,
    pub x_origin: u16,
    pub y_origin: u16,
    pub width: u16,
    pub height: u16,
    pub bpp: u8,
    pub descriptor: u8,
}

/// Header + raw payload (offset junk, palette-start junk, palette bytes and
/// pixel/RLE data laid out by the caller in stream order).
pub fn build_tga(h: &TgaHeader, payload: &[u8]) -> Vec<u8> {
    let mut f = vec![0u8; 18];
    f[0] = h.offset;
    f[1] = h.colormap_type;
    f[2] = h.image_type;
    f[3..5].copy_from_slice(&h.palette_start.to_le_bytes());
    f[5..7].copy_from_slice(&h.palette_len.to_le_bytes());
    f[7] = h.palette_bits;
    f[8..10].copy_from_slice(&h.x_origin.to_le_bytes());
    f[10..12].copy_from_slice(&h.y_origin.to_le_bytes());
    f[12..14].copy_from_slice(&h.width.to_le_bytes());
    f[14..16].copy_from_slice(&h.height.to_le_bytes());
    f[16] = h.bpp;
    f[17] = h.descriptor;
    f.extend_from_slice(payload);
    f
}

/// Deterministic byte stream (LCG) for pixel/palette payloads.
pub fn lcg_bytes(seed: u32, n: usize) -> Vec<u8> {
    let mut state = seed | 1;
    (0..n)
        .map(|_| {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            (state >> 16) as u8
        })
        .collect()
}

/// TGA RLE stream: alternating run and literal packets of deterministic
/// pixel values, covering at least `pixels` pixels of `px_size` bytes each.
pub fn tga_rle_stream(px_size: usize, pixels: usize, seed: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let mut produced = 0usize;
    let mut i = 0u32;
    while produced < pixels {
        let vals = lcg_bytes(seed.wrapping_add(i), px_size * 3);
        if i % 2 == 0 {
            // run of 3 of one pixel
            out.push(0x80 | 2);
            out.extend_from_slice(&vals[..px_size]);
            produced += 3;
        } else {
            // literal packet of 3 pixels
            out.push(2);
            out.extend_from_slice(&vals);
            produced += 3;
        }
        i += 1;
    }
    out
}
