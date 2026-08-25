//! Synthetic PCX/LMP images for the image differentials. Same builders as
//! quake-image's own unit tests, shared by `image_differential` (byte parity)
//! and `threaded_parse` (concurrent decode) so the two cannot drift apart.

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
