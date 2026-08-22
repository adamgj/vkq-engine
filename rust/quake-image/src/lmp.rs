//! QPIC/.lmp decode, ported from Image_DecodeLMP (Quake/image_decode.c)

/// sizeof(lmpheader_t)
pub const HEADER_SIZE: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// C: Sys_Error ("'%s' is not a valid LMP file", image_name)
    NotValid,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Lmp<'a> {
    Image {
        width: u32,
        height: u32,
        /// 8-bit palette-indexed pixels, width * height bytes
        pixels: &'a [u8],
    },
    /// C: file_size != 8 + width * height — COM_CloseFile and return NULL
    SizeMismatch,
}

/// `file` is the whole resource; `file_size` is com_filesize truncated to
/// int by the caller, exactly like the C local.
pub fn decode(file: &[u8], file_size: i32) -> Result<Lmp<'_>, Error> {
    if file.len() < HEADER_SIZE {
        return Err(Error::NotValid);
    }
    let width = u32::from_le_bytes([file[0], file[1], file[2], file[3]]);
    let height = u32::from_le_bytes([file[4], file[5], file[6], file[7]]);

    // COMPAT: C computes `pix = qpic.width * qpic.height` in unsigned int
    // (wrapping), then compares `file_size != 8 + pix` with both sides
    // promoted to size_t — the int sign-extends. Engine targets have 64-bit
    // size_t, so 8 + pix cannot wrap
    let pix = width.wrapping_mul(height);
    if file_size as i64 as u64 != 8 + u64::from(pix) {
        return Ok(Lmp::SizeMismatch);
    }

    // the size check bounds pix to file_size - 8; a short slice here means
    // the underlying read came up short, which is the C Sys_Error path
    let pixels = file
        .get(HEADER_SIZE..HEADER_SIZE + pix as usize)
        .ok_or(Error::NotValid)?;
    Ok(Lmp::Image {
        width,
        height,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_lmp(w: u32, h: u32, pixels: &[u8]) -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(&w.to_le_bytes());
        f.extend_from_slice(&h.to_le_bytes());
        f.extend_from_slice(pixels);
        f
    }

    #[test]
    fn valid_image() {
        let f = build_lmp(2, 3, &[1, 2, 3, 4, 5, 6]);
        assert_eq!(
            decode(&f, f.len() as i32),
            Ok(Lmp::Image {
                width: 2,
                height: 3,
                pixels: &[1, 2, 3, 4, 5, 6],
            })
        );
    }

    #[test]
    fn size_mismatch_returns_null_path() {
        let f = build_lmp(2, 3, &[1, 2, 3, 4, 5]);
        assert_eq!(decode(&f, f.len() as i32), Ok(Lmp::SizeMismatch));
    }

    #[test]
    fn short_header_rejected() {
        assert_eq!(decode(&[0u8; 4], 4), Err(Error::NotValid));
    }

    #[test]
    fn zero_size_wrap_mismatch() {
        // width * height wraps in unsigned int; the wrapped product cannot
        // match the real byte count, so the C NULL path is taken
        let f = build_lmp(0x1000_0000, 0x10, &[9; 16]);
        assert_eq!(decode(&f, f.len() as i32), Ok(Lmp::SizeMismatch));
    }
}
