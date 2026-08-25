//! TGA decode, hand-ported from stb_image v2.30's stbi__tga_load
//! (Quake/stb_image.h:5872-6075) per ADR-012/D8: the port reproduces stb's
//! accept/reject decisions, pixel output and failure reasons bit for bit,
//! quirks included.
//!
//! Stream model: stb reads the engine's Sys_File handle through a 128-byte
//! buffered context whose get8 returns 0 past end-of-file. The TGA decoder
//! never consults at_eof and never seeks backward, and its only skips are
//! non-negative, so a direct cursor over the resource slice — get8 = 0 past
//! the end, getn keeps the partial prefix, skip advances the position — is
//! byte-for-byte equivalent to the buffered callback pipeline for this
//! format. (Formats that do consult at_eof mid-stream cannot use this
//! shortcut.)

/// stb's short failure reasons (no STBI_FAILURE_USERMSG in image_stb.c), fed
/// verbatim into `Con_Warning ("couldn't load %s (%s)\n", ...)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// "too large" — STBI_MAX_DIMENSIONS (unreachable: 16-bit fields) or the
    /// stbi__mad3sizes_valid int-overflow check (reachable, dims published)
    TooLarge,
    /// "bad format" — stbi__tga_get_comp returned 0 (unreachable after
    /// stbi__tga_test, ported for fidelity)
    BadFormat,
    /// "bad palette" — zero palette entries, or a truncated non-rgb16
    /// palette read
    BadPalette,
    /// "outofmem" — stbi__convert_format's stbi__malloc_mad3(4, x, y, 0)
    /// returns NULL when 4*w*h overflows int (reachable: a grey image with
    /// w*h between 2^29 and 2^31 passes the first mad3 gate but not this
    /// one). The C then warns and recovers; without this gate the Rust
    /// side would attempt a multi-GiB allocation instead. True allocation
    /// failure (calloc NULL on an in-range size) remains unreproducible:
    /// Rust Vec aborts where C warns — COMPAT, real-OOM-only divergence.
    OutOfMem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Error {
    pub reason: Reason,
    /// stb writes `*x`/`*y` before the mad3/palette failures but after the
    /// MAX_DIMENSIONS/bad-format ones; the shim replicates the write points
    pub dims: Option<(i32, i32)>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Tga {
    pub width: i32,
    pub height: i32,
    /// width * height * 4 RGBA bytes (stb req_comp = 4)
    pub rgba: Vec<u8>,
}

const STBI_MAX_DIMENSIONS: i32 = 1 << 24;

/// Direct cursor with stb's past-EOF-reads-zero semantics (see module doc).
struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn get8(&mut self) -> u8 {
        let b = self.buf.get(self.pos).copied().unwrap_or(0);
        self.pos = self.pos.saturating_add(1);
        b
    }

    fn get16le(&mut self) -> i32 {
        let lo = i32::from(self.get8());
        let hi = i32::from(self.get8());
        lo | (hi << 8)
    }

    /// stbi__getn: copies what is available (the callback context keeps a
    /// partial prefix), reports whether all `n` bytes were read. The cursor
    /// advances by what was read, like Sys_FileRead; once short, every later
    /// read returns nothing.
    fn getn(&mut self, dest: &mut [u8]) -> bool {
        let start = self.pos.min(self.buf.len());
        let avail = (self.buf.len() - start).min(dest.len());
        dest[..avail].copy_from_slice(&self.buf[start..start + avail]);
        self.pos = start + avail;
        avail == dest.len()
    }

    fn skip(&mut self, n: i32) {
        // stbi__skip is only reached with n >= 0 here (offset and
        // palette_start are unsigned header fields)
        self.pos = self.pos.saturating_add(n as usize);
    }
}

/// stbi__tga_get_comp.
fn get_comp(bits_per_pixel: i32, is_grey: bool, is_rgb16: &mut i32) -> i32 {
    *is_rgb16 = 0;
    match bits_per_pixel {
        8 => 1,             // STBI_grey
        16 if is_grey => 2, // STBI_grey_alpha
        15 | 16 => {
            *is_rgb16 = 1;
            3 // STBI_rgb
        }
        24 | 32 => bits_per_pixel / 8,
        _ => 0,
    }
}

/// stbi__mul2sizes_valid.
fn mul2_valid(a: i32, b: i32) -> bool {
    if a < 0 || b < 0 {
        return false;
    }
    if b == 0 {
        return true; // mul-by-0 is always safe
    }
    a <= i32::MAX / b
}

/// stbi__mad3sizes_valid(a, b, c, 0).
fn mad3_valid(a: i32, b: i32, c: i32) -> bool {
    mul2_valid(a, b) && mul2_valid(a * b, c)
}

/// stbi__tga_read_rgb16: 555 unpack to RGB, no alpha ("treat all 15 and
/// 16bit TGAs as RGB with no alpha").
fn read_rgb16(c: &mut Cursor, out: &mut [u8]) {
    let px = c.get16le();
    let r = (px >> 10) & 31;
    let g = (px >> 5) & 31;
    let b = px & 31;
    // saved in RGB order: no swap later
    out[0] = ((r * 255) / 31) as u8;
    out[1] = ((g * 255) / 31) as u8;
    out[2] = ((b * 255) / 31) as u8;
}

/// stbi__convert_format(data, img_n, 4, x, y): the three source layouts the
/// TGA path can produce, each to RGBA.
fn convert_to_rgba(data: &[u8], comp: usize) -> Vec<u8> {
    let pixels = data.len() / comp;
    let mut out = vec![0u8; pixels * 4];
    match comp {
        1 => {
            for (i, px) in out.chunks_exact_mut(4).enumerate() {
                let v = data[i];
                px.copy_from_slice(&[v, v, v, 255]);
            }
        }
        2 => {
            for (i, px) in out.chunks_exact_mut(4).enumerate() {
                let v = data[i * 2];
                px.copy_from_slice(&[v, v, v, data[i * 2 + 1]]);
            }
        }
        3 => {
            for (i, px) in out.chunks_exact_mut(4).enumerate() {
                px.copy_from_slice(&[data[i * 3], data[i * 3 + 1], data[i * 3 + 2], 255]);
            }
        }
        _ => out.copy_from_slice(data),
    }
    out
}

/// stbi__tga_load with req_comp = 4 (image_stb.c always requests RGBA).
/// `file` is the whole resource; the caller has already classified it as
/// TGA via [`crate::stb_sniff`].
pub fn decode(file: &[u8]) -> Result<Tga, Error> {
    let c = &mut Cursor { buf: file, pos: 0 };

    let tga_offset = i32::from(c.get8());
    let tga_indexed = i32::from(c.get8());
    let mut tga_image_type = i32::from(c.get8());
    let mut tga_is_rle = false;
    let tga_palette_start = c.get16le();
    let tga_palette_len = c.get16le();
    let tga_palette_bits = i32::from(c.get8());
    let _tga_x_origin = c.get16le(); // ignored by stb (no horizontal flip)
    let _tga_y_origin = c.get16le();
    let tga_width = c.get16le();
    let tga_height = c.get16le();
    let tga_bits_per_pixel = i32::from(c.get8());
    let mut tga_rgb16 = 0;
    let tga_descriptor = i32::from(c.get8());

    let fail = |reason: Reason, published: bool| Error {
        reason,
        dims: published.then_some((tga_width, tga_height)),
    };

    // COMPAT: unreachable for TGA (16-bit dimension fields), ported because
    // stb checks them before publishing the dims
    if tga_height > STBI_MAX_DIMENSIONS || tga_width > STBI_MAX_DIMENSIONS {
        return Err(fail(Reason::TooLarge, false));
    }

    if tga_image_type >= 8 {
        tga_image_type -= 8;
        tga_is_rle = true;
    }
    let tga_inverted = 1 - ((tga_descriptor >> 5) & 1);

    let tga_comp = if tga_indexed != 0 {
        get_comp(tga_palette_bits, false, &mut tga_rgb16)
    } else {
        get_comp(tga_bits_per_pixel, tga_image_type == 3, &mut tga_rgb16)
    };
    if tga_comp == 0 {
        // COMPAT: unreachable after stbi__tga_test, ported for fidelity
        return Err(fail(Reason::BadFormat, false));
    }

    // *x/*y are published here — before the size-overflow check
    if !mad3_valid(tga_width, tga_height, tga_comp) {
        return Err(fail(Reason::TooLarge, true));
    }
    // stb only discovers this at the stbi__convert_format stage — after
    // decoding into the comp-sized buffer — but the observable outcome
    // (reject, "outofmem", dims published) is identical, and checking
    // before the allocation avoids the C's transient multi-hundred-MB
    // buffer for an 18-byte file
    if tga_comp != 4 && !mad3_valid(4, tga_width, tga_height) {
        return Err(fail(Reason::OutOfMem, true));
    }

    let comp = tga_comp as usize;
    let (w, h) = (tga_width as usize, tga_height as usize);
    // Mem_Alloc zero-fills; the zeros are observable wherever a read comes
    // up short (truncated raw rows, truncated rgb16 palettes)
    let mut tga_data = vec![0u8; w * h * comp];

    // skip to the data's starting position (offset usually = 0)
    c.skip(tga_offset);

    if tga_indexed == 0 && !tga_is_rle && tga_rgb16 == 0 {
        // fast path: raw rows, placed inverted as they are read
        for i in 0..h {
            let row = if tga_inverted != 0 { h - i - 1 } else { i };
            // stb ignores the getn result: a short row keeps its prefix and
            // the zero tail, and later rows read nothing
            c.getn(&mut tga_data[row * w * comp..(row + 1) * w * comp]);
        }
    } else {
        let mut tga_palette = Vec::new();
        if tga_indexed != 0 {
            if tga_palette_len == 0 {
                // you have to have at least one entry!
                return Err(fail(Reason::BadPalette, true));
            }
            c.skip(tga_palette_start);
            tga_palette = vec![0u8; tga_palette_len as usize * comp];
            if tga_rgb16 != 0 {
                // COMPAT: no failure path — a truncated rgb16 palette reads
                // zeros, exactly like stb's per-entry read_rgb16 loop
                for entry in tga_palette.chunks_exact_mut(comp) {
                    read_rgb16(c, entry);
                }
            } else if !c.getn(&mut tga_palette) {
                return Err(fail(Reason::BadPalette, true));
            }
        }

        let mut raw_data = [0u8; 4];
        let mut rle_count: i32 = 0;
        let mut rle_repeating = 0;
        let mut read_next_pixel = true;
        for i in 0..w * h {
            if tga_is_rle {
                if rle_count == 0 {
                    let rle_cmd = i32::from(c.get8());
                    rle_count = 1 + (rle_cmd & 127);
                    rle_repeating = rle_cmd >> 7;
                    read_next_pixel = true;
                } else if rle_repeating == 0 {
                    read_next_pixel = true;
                }
            } else {
                read_next_pixel = true;
            }
            if read_next_pixel {
                if tga_indexed != 0 {
                    // read in index, then perform the lookup
                    let mut pal_idx = if tga_bits_per_pixel == 8 {
                        i32::from(c.get8())
                    } else {
                        c.get16le()
                    };
                    if pal_idx >= tga_palette_len {
                        pal_idx = 0; // invalid index
                    }
                    let pal_idx = pal_idx as usize * comp;
                    raw_data[..comp].copy_from_slice(&tga_palette[pal_idx..pal_idx + comp]);
                } else if tga_rgb16 != 0 {
                    read_rgb16(c, &mut raw_data);
                } else {
                    for slot in raw_data[..comp].iter_mut() {
                        *slot = c.get8();
                    }
                }
                read_next_pixel = false;
            }
            tga_data[i * comp..(i + 1) * comp].copy_from_slice(&raw_data[..comp]);
            rle_count -= 1;
        }
        // do I need to invert the image?
        if tga_inverted != 0 {
            for j in 0..h / 2 {
                let (top, rest) = tga_data.split_at_mut((j + 1) * w * comp);
                let row1 = &mut top[j * w * comp..];
                let i2 = (h - 1 - j) * w * comp - (j + 1) * w * comp;
                row1.swap_with_slice(&mut rest[i2..i2 + w * comp]);
            }
        }
    }

    // swap RGB - if the source data was RGB16, it already is in the right order
    if tga_comp >= 3 && tga_rgb16 == 0 {
        for px in tga_data.chunks_exact_mut(comp) {
            px.swap(0, 2);
        }
    }

    Ok(Tga {
        width: tga_width,
        height: tga_height,
        rgba: convert_to_rgba(&tga_data, comp),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(image_type: u8, w: u16, h: u16, bpp: u8, descriptor: u8) -> Vec<u8> {
        let mut f = vec![0u8; 18];
        f[2] = image_type;
        f[12..14].copy_from_slice(&w.to_le_bytes());
        f[14..16].copy_from_slice(&h.to_le_bytes());
        f[16] = bpp;
        f[17] = descriptor;
        f
    }

    #[test]
    fn raw_bgr_bottom_origin_flips_and_swaps() {
        let mut f = header(2, 2, 2, 24, 0);
        f.extend_from_slice(&[255, 0, 0, 0, 255, 0, 0, 0, 255, 10, 20, 30]);
        let t = decode(&f).unwrap();
        // bottom-origin: file row 1 becomes output row 0; BGR -> RGB
        assert_eq!(&t.rgba[0..4], &[255, 0, 0, 255]);
        assert_eq!(&t.rgba[4..8], &[30, 20, 10, 255]);
        assert_eq!(&t.rgba[8..12], &[0, 0, 255, 255]);
        assert_eq!(&t.rgba[12..16], &[0, 255, 0, 255]);
    }

    #[test]
    fn top_origin_keeps_row_order() {
        let mut f = header(2, 1, 2, 24, 0x20);
        f.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        let t = decode(&f).unwrap();
        assert_eq!(&t.rgba[0..4], &[3, 2, 1, 255]);
        assert_eq!(&t.rgba[4..8], &[6, 5, 4, 255]);
    }

    #[test]
    fn grey_replicates_channels() {
        let mut f = header(3, 2, 1, 8, 0x20);
        f.extend_from_slice(&[7, 200]);
        let t = decode(&f).unwrap();
        assert_eq!(&t.rgba, &[7, 7, 7, 255, 200, 200, 200, 255]);
    }

    #[test]
    fn rgb16_555_unpack() {
        let mut f = header(2, 1, 1, 16, 0x20);
        // R=31, G=0, B=15 -> 0b0111110000001111
        f.extend_from_slice(&0b0111_1100_0000_1111u16.to_le_bytes());
        let t = decode(&f).unwrap();
        assert_eq!(&t.rgba, &[255, 0, ((15u32 * 255) / 31) as u8, 255]);
    }

    #[test]
    fn rle_run_spills_across_rows() {
        // 2x2 8bpp grey RLE: one run of 3 (value 9), then 1 literal (5)
        let mut f = header(11, 2, 2, 8, 0x20);
        f.extend_from_slice(&[0x82, 9, 0x00, 5]);
        let t = decode(&f).unwrap();
        assert_eq!(
            &t.rgba,
            &[9, 9, 9, 255, 9, 9, 9, 255, 9, 9, 9, 255, 5, 5, 5, 255]
        );
    }

    #[test]
    fn rle_count_can_exceed_pixels() {
        // run of 128 into a 1x1 image: loop is bounded by w*h
        let mut f = header(11, 1, 1, 8, 0x20);
        f.extend_from_slice(&[0xFF, 42]);
        let t = decode(&f).unwrap();
        assert_eq!(&t.rgba, &[42, 42, 42, 255]);
    }

    #[test]
    fn palette_lookup_with_oob_index() {
        // 2 palette entries of 24 bits; index 7 is out of range -> entry 0
        let mut f = header(1, 2, 1, 8, 0x20);
        f[1] = 1; // colormap type
        f[5..7].copy_from_slice(&2u16.to_le_bytes()); // palette length
        f[7] = 24; // palette entry bits
        f.extend_from_slice(&[10, 20, 30, 40, 50, 60]); // 2 BGR entries
        f.extend_from_slice(&[1, 7]); // indexes
        let t = decode(&f).unwrap();
        assert_eq!(&t.rgba[0..4], &[60, 50, 40, 255]);
        assert_eq!(&t.rgba[4..8], &[30, 20, 10, 255]);
    }

    #[test]
    fn empty_palette_rejects_with_dims_published() {
        let mut f = header(1, 2, 1, 8, 0x20);
        f[1] = 1;
        f[7] = 24;
        assert_eq!(
            decode(&f),
            Err(Error {
                reason: Reason::BadPalette,
                dims: Some((2, 1)),
            })
        );
    }

    #[test]
    fn truncated_palette_rejects() {
        let mut f = header(1, 1, 1, 8, 0x20);
        f[1] = 1;
        f[5..7].copy_from_slice(&4u16.to_le_bytes());
        f[7] = 24;
        f.extend_from_slice(&[1, 2, 3]); // 3 of 12 palette bytes
        assert_eq!(decode(&f).unwrap_err().reason, Reason::BadPalette);
    }

    #[test]
    fn size_overflow_rejects_after_publishing_dims() {
        let f = header(2, 65535, 65535, 32, 0x20);
        assert_eq!(
            decode(&f),
            Err(Error {
                reason: Reason::TooLarge,
                dims: Some((65535, 65535)),
            })
        );
    }

    #[test]
    fn conversion_overflow_rejects_outofmem_with_dims_published() {
        // grey 23171x23171: w*h passes mad3(w,h,1) but 4*w*h overflows int,
        // stb's stbi__convert_format failure ("outofmem", dims published)
        let f = header(3, 23171, 23171, 8, 0x20);
        assert_eq!(
            decode(&f),
            Err(Error {
                reason: Reason::OutOfMem,
                dims: Some((23171, 23171)),
            })
        );
    }

    #[test]
    fn truncated_raw_keeps_partial_row_and_zero_tail() {
        let mut f = header(2, 2, 2, 24, 0x20);
        f.extend_from_slice(&[1, 2, 3, 4]); // 4 of 12 bytes
        let t = decode(&f).unwrap();
        // row 0 keeps the partial prefix (BGR->RGB swapped), rest zeros
        assert_eq!(&t.rgba[0..4], &[3, 2, 1, 255]);
        assert_eq!(&t.rgba[4..8], &[0, 0, 4, 255]);
        assert_eq!(&t.rgba[8..16], &[0, 0, 0, 255, 0, 0, 0, 255]);
    }

    #[test]
    fn nonzero_offset_skips_into_data() {
        let mut f = header(3, 1, 1, 8, 0x20);
        f[0] = 2; // offset
        f.extend_from_slice(&[99, 99, 77]);
        let t = decode(&f).unwrap();
        assert_eq!(&t.rgba, &[77, 77, 77, 255]);
    }
}
