//! PCX decode, ported from Image_DecodePCX (Quake/image_decode.c)
//!
//! The C decoder streams through Sys_fgetc and trusts the input: an RLE
//! stream that overruns the pixel buffer, hits end-of-file mid-run, or a
//! resource too small to hold the 768-byte tail palette all reach undefined
//! behavior (out-of-bounds reads/writes), and inside a pak the OS-level
//! reads can leave the resource bounds entirely. This port bounds every
//! access to the resource slice and reports `Error::NotValid` on those
//! inputs instead; well-formed inputs decode byte-identically.
//! COMPAT: divergence is confined to UB/out-of-resource inputs (task plan
//! amendment log, docs/ai/plans/rust-conversion-phase-3.md).

use core::ffi::c_char;

/// sizeof(pcxheader_t)
pub const HEADER_SIZE: usize = 128;
pub const PALETTE_SIZE: usize = 768;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// C: Sys_Error ("'%s' is not a valid PCX file", image_name)
    NotValid,
    /// C: Sys_Error ("'%s' is version %i, should be 5", image_name, pcx.version)
    /// COMPAT: c_char, not i8 — the C field is a plain `char`, which is
    /// unsigned on aarch64 Linux, so a version byte >= 0x80 must print as a
    /// positive number there just like the C Sys_Error does
    BadVersion(c_char),
    /// C: Sys_Error ("'%s' has wrong encoding or bit depth", image_name)
    BadEncoding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub width: i32,
    pub height: i32,
    /// (w * h + 1) * 4 in wrapping i32 arithmetic — the exact byte count the
    /// C decoder passes to Mem_Alloc (the +1 pixel absorbs the padding byte
    /// on the last line). COMPAT: kept in the int domain so the shim's
    /// size_t conversion sign-extends exactly like the C call site.
    pub alloc_size: i32,
    pub bytes_per_line: u16,
}

/// Validation and dimension logic of Image_DecodePCX up to the Mem_Alloc
/// call. `file` is the whole resource (header + RLE data + tail palette).
pub fn parse_header(file: &[u8]) -> Result<Header, Error> {
    if file.len() < HEADER_SIZE {
        return Err(Error::NotValid);
    }
    let signature = file[0];
    let version = file[1];
    let encoding = file[2];
    let bits_per_pixel = file[3];
    let xmin = u16::from_le_bytes([file[4], file[5]]);
    let ymin = u16::from_le_bytes([file[6], file[7]]);
    let xmax = u16::from_le_bytes([file[8], file[9]]);
    let ymax = u16::from_le_bytes([file[10], file[11]]);
    let color_planes = file[65];
    let bytes_per_line = u16::from_le_bytes([file[66], file[67]]);

    if signature != 0x0A {
        return Err(Error::NotValid);
    }
    if version != 5 {
        // from_ne_bytes, not `as`: c_char is u8 on aarch64 Linux, where an
        // `as c_char` would be a same-type cast clippy rejects
        return Err(Error::BadVersion(c_char::from_ne_bytes([version])));
    }
    if encoding != 1 || bits_per_pixel != 8 || color_planes != 1 {
        return Err(Error::BadEncoding);
    }

    let w = i32::from(xmax) - i32::from(xmin) + 1;
    let h = i32::from(ymax) - i32::from(ymin) + 1;
    Ok(Header {
        width: w,
        height: h,
        alloc_size: w.wrapping_mul(h).wrapping_add(1).wrapping_mul(4),
        bytes_per_line,
    })
}

/// RLE decode into `out` (RGBA, zero-initialized by the caller like
/// Mem_Alloc/calloc). Mirrors the C loop exactly: rows restart at
/// y * w * 4, runs may spill past the row end, and the last line may write
/// one pixel of padding into the +1 slot.
pub fn decode(file: &[u8], header: &Header, out: &mut [u8]) -> Result<(), Error> {
    // C: Sys_FileSeek (file_handle, start + file_size - 768) then a 768-byte
    // read; for a plain file a resource shorter than the palette makes that
    // read come up short and Sys_Error. COMPAT: inside a pak the C seek
    // lands before the resource start and reads neighboring pak bytes; this
    // port reports NotValid instead (out-of-resource input)
    if file.len() < PALETTE_SIZE {
        return Err(Error::NotValid);
    }
    let palette = &file[file.len() - PALETTE_SIZE..];
    let rle = &file[HEADER_SIZE..];

    let w = i64::from(header.width);
    let mut pos = 0usize;
    for y in 0..i64::from(header.height.max(0)) {
        let mut p = y * w * 4;
        let mut x: i32 = 0;
        while x < i32::from(header.bytes_per_line) {
            // COMPAT: at end-of-input the C Sys_fgetc returns EOF (-1) and
            // the decoder indexes palette[-3] (UB); report NotValid instead
            let mut readbyte = i32::from(*rle.get(pos).ok_or(Error::NotValid)?);
            pos += 1;
            let mut runlength = 1;
            if readbyte >= 0xC0 {
                runlength = readbyte & 0x3F;
                readbyte = i32::from(*rle.get(pos).ok_or(Error::NotValid)?);
                pos += 1;
            }
            while runlength > 0 {
                runlength -= 1;
                // COMPAT: the C decoder writes unchecked through
                // p = data + y*w*4 (heap overflow UB when the RLE stream
                // overruns the (w*h+1)-pixel buffer, negative offsets when
                // w < 0); report NotValid instead
                let dst = usize::try_from(p)
                    .ok()
                    .filter(|&d| d + 4 <= out.len())
                    .ok_or(Error::NotValid)?;
                let idx = (readbyte * 3) as usize;
                out[dst] = palette[idx];
                out[dst + 1] = palette[idx + 1];
                out[dst + 2] = palette[idx + 2];
                out[dst + 3] = 255;
                p += 4;
                x += 1;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_pcx(w: u16, h: u16, bytes_per_line: u16, rle: &[u8]) -> Vec<u8> {
        let mut f = vec![0u8; HEADER_SIZE];
        f[0] = 0x0A;
        f[1] = 5;
        f[2] = 1;
        f[3] = 8;
        f[8..10].copy_from_slice(&(w - 1).to_le_bytes());
        f[10..12].copy_from_slice(&(h - 1).to_le_bytes());
        f[65] = 1;
        f[66..68].copy_from_slice(&bytes_per_line.to_le_bytes());
        f.extend_from_slice(rle);
        let mut palette = [0u8; PALETTE_SIZE];
        for (i, b) in palette.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        f.extend_from_slice(&palette);
        f
    }

    #[test]
    fn literal_and_run_pixels() {
        // 2x2: literal indices 1 and 2, then a 2-length run of index 3
        let file = build_pcx(2, 2, 2, &[1, 2, 0xC2, 3]);
        let header = parse_header(&file).unwrap();
        assert_eq!((header.width, header.height), (2, 2));
        assert_eq!(header.alloc_size, (2 * 2 + 1) * 4);
        let mut out = vec![0u8; header.alloc_size as usize];
        decode(&file, &header, &mut out).unwrap();
        assert_eq!(&out[0..4], &[3, 4, 5, 255]);
        assert_eq!(&out[4..8], &[6, 7, 8, 255]);
        assert_eq!(&out[8..12], &[9, 10, 11, 255]);
        assert_eq!(&out[12..16], &[9, 10, 11, 255]);
        assert_eq!(&out[16..20], &[0, 0, 0, 0]); // +1 padding pixel untouched
    }

    #[test]
    fn last_line_padding_write() {
        // bytes_per_line one wider than the image: the pad byte lands in the
        // +1 slot on the last row
        let file = build_pcx(1, 1, 2, &[7, 8]);
        let header = parse_header(&file).unwrap();
        let mut out = vec![0u8; header.alloc_size as usize];
        decode(&file, &header, &mut out).unwrap();
        assert_eq!(&out[0..4], &[21, 22, 23, 255]);
        assert_eq!(&out[4..8], &[24, 25, 26, 255]);
    }

    #[test]
    fn rejects() {
        assert_eq!(parse_header(&[0u8; 12]), Err(Error::NotValid));
        let mut f = build_pcx(1, 1, 1, &[0]);
        f[0] = 0;
        assert_eq!(parse_header(&f), Err(Error::NotValid));
        let mut f = build_pcx(1, 1, 1, &[0]);
        f[1] = 4;
        assert_eq!(parse_header(&f), Err(Error::BadVersion(4)));
        let mut f = build_pcx(1, 1, 1, &[0]);
        f[3] = 24;
        assert_eq!(parse_header(&f), Err(Error::BadEncoding));
    }

    #[test]
    fn truncated_rle_and_overrun_rejected() {
        // no RLE bytes of its own: the decoder consumes the palette region
        // as pixel data (like C), so input only runs out once the demand
        // exceeds the whole resource
        let file = build_pcx(100, 100, 100, &[]);
        let header = parse_header(&file).unwrap();
        let mut out = vec![0u8; header.alloc_size as usize];
        assert_eq!(decode(&file, &header, &mut out), Err(Error::NotValid));

        // a run long past the buffer end
        let file = build_pcx(1, 1, 1, &[0xFF, 1]);
        let header = parse_header(&file).unwrap();
        let mut out = vec![0u8; header.alloc_size as usize];
        assert_eq!(decode(&file, &header, &mut out), Err(Error::NotValid));
    }
}
