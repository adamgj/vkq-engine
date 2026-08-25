//! stb_image format sniffing, ported from stbi__load_main's probe chain
//! (Quake/stb_image.h). With this tree's STBI_NO_* set the enabled decoders
//! are exactly PNG, JPEG and TGA, probed in that order ("test tga last
//! because it's a crappy test!"). The Rust Image_DecodeSTB dispatches on this
//! classification, so it must reproduce each probe's accept set bit for bit —
//! including the stream semantics stb's probes see: reads past end-of-file
//! return 0 rather than failing.

/// Classification result of the stb probe chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
    Tga,
    /// C: stbi__errpuc("unknown image type", ...)
    Unknown,
}

/// stbi__check_png_header's signature bytes.
const PNG_SIG: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

/// Cursor with stb's stbi__get8 semantics: reading past the end returns 0.
struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Cursor { buf, pos: 0 }
    }

    fn get8(&mut self) -> u8 {
        let b = self.buf.get(self.pos).copied().unwrap_or(0);
        self.pos = self.pos.saturating_add(1);
        b
    }

    fn get16le(&mut self) -> u16 {
        let lo = u16::from(self.get8());
        let hi = u16::from(self.get8());
        lo | (hi << 8)
    }

    fn skip(&mut self, n: usize) {
        self.pos = self.pos.saturating_add(n);
    }
}

/// stbi__png_test: the 8-byte signature.
fn png_test(file: &[u8]) -> bool {
    file.len() >= PNG_SIG.len() && file[..PNG_SIG.len()] == PNG_SIG
}

/// stbi__jpeg_test = stbi__decode_jpeg_header(STBI__SCAN_type): one
/// stbi__get_marker (a 0xFF, any number of 0xFF fill bytes, then the marker
/// byte) that must yield SOI (0xD8). At end-of-file stb's get8 returns 0, so
/// a truncated fill run classifies as marker 0, not SOI.
fn jpeg_test(file: &[u8]) -> bool {
    let mut c = Cursor::new(file);
    let mut x = c.get8();
    if x != 0xFF {
        return false; // stbi__get_marker: not a marker at all
    }
    while x == 0xFF {
        x = c.get8(); // consume repeated 0xFF fill bytes
    }
    x == 0xD8
}

/// stbi__tga_test, ported check for check (Quake/stb_image.h:5821-5850).
fn tga_test(file: &[u8]) -> bool {
    let mut c = Cursor::new(file);
    c.get8(); // discard Offset
    let tga_color_type = c.get8();
    if tga_color_type > 1 {
        return false; // only RGB or indexed allowed
    }
    let sz = c.get8(); // image type
    if tga_color_type == 1 {
        // colormapped (paletted) image
        if sz != 1 && sz != 9 {
            return false; // colortype 1 demands image type 1 or 9
        }
        c.skip(4); // skip index of first colormap entry and number of entries
        let sz = c.get8(); // check bits per palette color entry
        if sz != 8 && sz != 15 && sz != 16 && sz != 24 && sz != 32 {
            return false;
        }
        c.skip(4); // skip image x and y origin
    } else {
        // "normal" image w/o colormap
        if sz != 2 && sz != 3 && sz != 10 && sz != 11 {
            return false; // only RGB or grey allowed, +/- RLE
        }
        c.skip(9); // skip colormap specification and image x/y origin
    }
    if c.get16le() < 1 {
        return false; // test width
    }
    if c.get16le() < 1 {
        return false; // test height
    }
    let sz = c.get8(); // bits per pixel
    if tga_color_type == 1 && sz != 8 && sz != 16 {
        return false; // for colormapped images, bpp is size of an index
    }
    if sz != 8 && sz != 15 && sz != 16 && sz != 24 && sz != 32 {
        return false;
    }
    true
}

/// stbi__load_main's probe order over this tree's enabled decoder set.
pub fn classify(file: &[u8]) -> Format {
    if png_test(file) {
        Format::Png
    } else if jpeg_test(file) {
        Format::Jpeg
    } else if tga_test(file) {
        Format::Tga
    } else {
        Format::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_tiny_inputs_are_unknown() {
        assert_eq!(classify(&[]), Format::Unknown);
        assert_eq!(classify(&[0x00]), Format::Unknown);
        assert_eq!(classify(&[0x89]), Format::Unknown);
    }

    #[test]
    fn png_signature_wins_first() {
        let mut f = PNG_SIG.to_vec();
        f.extend_from_slice(b"garbage");
        assert_eq!(classify(&f), Format::Png);
        // 7 of 8 signature bytes is not PNG (and not TGA: color type 78 > 1)
        assert_eq!(classify(&PNG_SIG[..7]), Format::Unknown);
    }

    #[test]
    fn jpeg_soi_with_fill_bytes() {
        assert_eq!(classify(&[0xFF, 0xD8]), Format::Jpeg);
        assert_eq!(classify(&[0xFF, 0xFF, 0xFF, 0xD8]), Format::Jpeg);
        // EOF inside the fill run reads 0: not SOI
        assert_eq!(classify(&[0xFF]), Format::Unknown);
        assert_eq!(classify(&[0xFF, 0xFF]), Format::Unknown);
        assert_eq!(classify(&[0xFF, 0xD9]), Format::Unknown);
    }

    #[test]
    fn tga_minimal_headers() {
        // uncompressed true-color 1x1 24bpp
        let mut h = vec![0u8; 18];
        h[2] = 2; // image type
        h[12] = 1; // width lo
        h[14] = 1; // height lo
        h[16] = 24; // bpp
        assert_eq!(classify(&h), Format::Tga);
        // zero width rejects
        h[12] = 0;
        assert_eq!(classify(&h), Format::Unknown);
        h[12] = 1;
        // bad bpp rejects
        h[16] = 12;
        assert_eq!(classify(&h), Format::Unknown);
        h[16] = 24;
        // color type > 1 rejects
        h[1] = 2;
        assert_eq!(classify(&h), Format::Unknown);
    }

    #[test]
    fn tga_colormapped_header_rules() {
        // indexed 8-bit palette, 8-bit indexes, type 1
        let mut h = vec![0u8; 18];
        h[1] = 1; // colormap type
        h[2] = 1; // image type (indexed)
        h[7] = 24; // palette entry bits
        h[12] = 1;
        h[14] = 1;
        h[16] = 8; // index bits
        assert_eq!(classify(&h), Format::Tga);
        // colortype 1 with a non-indexed image type rejects
        h[2] = 2;
        assert_eq!(classify(&h), Format::Unknown);
        h[2] = 1;
        // palette entry bits 12 rejects
        h[7] = 12;
        assert_eq!(classify(&h), Format::Unknown);
        h[7] = 24;
        // index bits 24 rejects (colormapped bpp must be 8 or 16)
        h[16] = 24;
        assert_eq!(classify(&h), Format::Unknown);
    }

    #[test]
    fn truncated_tga_header_reads_zeros() {
        // stb's get8 returns 0 past EOF: a plausible prefix that ends before
        // the bpp byte reads bpp 0 and rejects
        let h = [0u8, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0];
        assert_eq!(classify(&h), Format::Unknown);
    }
}
