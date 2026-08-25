//! PNG decode behind the stb seam (Phase 3 M8, ADR-012/D8).
//!
//! Architecture: the *acceptance* decisions are a hand-port of
//! stbi__parse_png_file's chunk walk (Quake/stb_image.h:5079-5262) plus
//! stbi__parse_zlib_header — every structural accept/reject and its short
//! failure reason is decided here, bit-for-bit like stb, independent of the
//! `png` crate. The accepted chunks are then re-assembled into a canonical
//! stream (IHDR / PLTE / tRNS / one IDAT / IEND, zlib header normalized to
//! cinfo=7, dummy CRCs) and handed to the `png` crate for
//! inflate + defilter + expand only, with checksum verification off (stb
//! checks neither CRC-32 nor Adler-32). The crate's gray expansion scales by
//! 255/(2^depth-1), identical to stb's depth-scale table, and its tRNS
//! colorkey equals stb's post-scale compare (no scaled-key collisions exist
//! for any depth). Remaining crate-side rejects (malformed deflate bodies,
//! filter bytes >= 5, short pixel data) match stb's reject *decision*; the
//! reason text differs and is masked in the differential (owner-approved
//! policy, task-plan amendment log).
//!
//! COMPAT divergences, all confined to inputs outside well-formed content:
//! - paletted pixels indexing past the palette are UB in C (stb reads
//!   uninitialized stack, stb_image.h:5081); the crate reads opaque black.
//!   Excluded from parity like the PCX RLE-overrun class (M2).
//! - an ancillary chunk whose length has the top bit set makes stb's
//!   stbi__skip jump to the end of its 128-byte read buffer (an internal-
//!   buffering artifact); this walk saturates past the resource instead.
//!   Both reject; the reason may differ.
//! - CgBI (iPhone) PNGs are routed to the C stb fallback wholesale, so
//!   their acceptance and (color-mangled) pixels stay exactly stb's.

/// Result of a successful walk + decode.
#[derive(Debug, PartialEq, Eq)]
pub enum Png {
    Image {
        width: i32,
        height: i32,
        /// width * height * 4 RGBA bytes (stb req_comp = 4)
        rgba: Vec<u8>,
    },
    /// CgBI chunk seen: the shim must decode through the C stb fallback
    Fallback,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// stb-exact short failure reason from the ported structural walk
    Stb(&'static str),
    /// stb's unknown-critical-chunk reason: the 4 raw type bytes prefixing
    /// " PNG chunk not known", truncated at the first NUL by C string rules
    /// (the shim renders it)
    UnknownChunk([u8; 4]),
    /// stb returns 0 *without* setting a reason (the IDAT int-overflow
    /// path), so the C warning prints whatever reason an earlier decode
    /// left in the thread-local: reject with unspecified text
    StaleReason,
    /// the png crate rejected the reconstructed stream (deflate body,
    /// filter bytes, short pixel data): same decision as stb, masked reason
    Crate(String),
}

const STBI_MAX_DIMENSIONS: u32 = 1 << 24;

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

    fn get16be(&mut self) -> u32 {
        let hi = u32::from(self.get8());
        (hi << 8) | u32::from(self.get8())
    }

    fn get32be(&mut self) -> u32 {
        let hi = self.get16be();
        (hi << 16) | self.get16be()
    }

    /// stbi__getn over the callback context: partial prefix kept, success
    /// only when everything was read.
    fn getn(&mut self, dest: &mut [u8]) -> bool {
        let start = self.pos.min(self.buf.len());
        let avail = (self.buf.len() - start).min(dest.len());
        dest[..avail].copy_from_slice(&self.buf[start..start + avail]);
        self.pos = start + avail;
        avail == dest.len()
    }

    fn skip(&mut self, n: u32) {
        self.pos = self.pos.saturating_add(n as usize);
    }
}

const fn png_type(a: u8, b: u8, c: u8, d: u8) -> u32 {
    ((a as u32) << 24) | ((b as u32) << 16) | ((c as u32) << 8) | (d as u32)
}

const TYPE_CGBI: u32 = png_type(b'C', b'g', b'B', b'I');
const TYPE_IHDR: u32 = png_type(b'I', b'H', b'D', b'R');
const TYPE_PLTE: u32 = png_type(b'P', b'L', b'T', b'E');
const TYPE_TRNS: u32 = png_type(b't', b'R', b'N', b'S');
const TYPE_IDAT: u32 = png_type(b'I', b'D', b'A', b'T');
const TYPE_IEND: u32 = png_type(b'I', b'E', b'N', b'D');

struct Ihdr {
    width: u32,
    height: u32,
    depth: u8,
    color: u8,
    interlace: u8,
}

/// The ported stbi__parse_png_file chunk walk (scan = load). Returns the
/// pieces the reconstruction needs.
struct Walked {
    ihdr: Ihdr,
    /// RGB triples as parsed (last PLTE wins, like stb's overwrite)
    palette: Vec<u8>,
    /// tRNS chunk payload verbatim (alpha bytes for paletted, the 2*img_n
    /// key bytes otherwise)
    trns: Option<Vec<u8>>,
    /// concatenated IDAT payload
    idata: Vec<u8>,
}

fn walk(file: &[u8]) -> Result<Result<Walked, ()>, Error> {
    let c = &mut Cursor { buf: file, pos: 8 }; // signature checked by the sniffer

    let mut first = true;
    let mut ihdr: Option<Ihdr> = None;
    let mut img_n: u32 = 0;
    let mut pal_img_n: u32 = 0;
    let mut pal_len: u32 = 0;
    let mut palette: Vec<u8> = Vec::new();
    let mut trns: Option<Vec<u8>> = None;
    let mut idata: Vec<u8> = Vec::new();
    let mut ioff: u32 = 0;

    loop {
        let length = c.get32be();
        let ctype = c.get32be();
        match ctype {
            TYPE_CGBI => return Ok(Err(())), // iPhone PNG: C fallback
            TYPE_IHDR => {
                if !first {
                    return Err(Error::Stb("multiple IHDR"));
                }
                first = false;
                if length != 13 {
                    return Err(Error::Stb("bad IHDR len"));
                }
                let img_x = c.get32be();
                let img_y = c.get32be();
                if img_y > STBI_MAX_DIMENSIONS || img_x > STBI_MAX_DIMENSIONS {
                    return Err(Error::Stb("too large"));
                }
                let depth = c.get8();
                if depth != 1 && depth != 2 && depth != 4 && depth != 8 && depth != 16 {
                    return Err(Error::Stb("1/2/4/8/16-bit only"));
                }
                let color = c.get8();
                if color > 6 || (color == 3 && depth == 16) {
                    return Err(Error::Stb("bad ctype"));
                }
                if color == 3 {
                    pal_img_n = 3;
                } else if color & 1 != 0 {
                    return Err(Error::Stb("bad ctype"));
                }
                let comp = c.get8();
                if comp != 0 {
                    return Err(Error::Stb("bad comp method"));
                }
                let filter = c.get8();
                if filter != 0 {
                    return Err(Error::Stb("bad filter method"));
                }
                let interlace = c.get8();
                if interlace > 1 {
                    return Err(Error::Stb("bad interlace method"));
                }
                if img_x == 0 || img_y == 0 {
                    return Err(Error::Stb("0-pixel image"));
                }
                if pal_img_n == 0 {
                    img_n =
                        (if color & 2 != 0 { 3 } else { 1 }) + (if color & 4 != 0 { 1 } else { 0 });
                    if (1 << 30) / img_x / img_n < img_y {
                        return Err(Error::Stb("too large"));
                    }
                } else {
                    img_n = 1;
                    if (1 << 30) / img_x / 4 < img_y {
                        return Err(Error::Stb("too large"));
                    }
                }
                ihdr = Some(Ihdr {
                    width: img_x,
                    height: img_y,
                    depth,
                    color,
                    interlace,
                });
            }
            TYPE_PLTE => {
                if first {
                    return Err(Error::Stb("first not IHDR"));
                }
                if length > 256 * 3 {
                    return Err(Error::Stb("invalid PLTE"));
                }
                pal_len = length / 3;
                if pal_len * 3 != length {
                    return Err(Error::Stb("invalid PLTE"));
                }
                palette.clear();
                for _ in 0..pal_len * 3 {
                    palette.push(c.get8());
                }
            }
            TYPE_TRNS => {
                if first {
                    return Err(Error::Stb("first not IHDR"));
                }
                if !idata.is_empty() {
                    return Err(Error::Stb("tRNS after IDAT"));
                }
                if pal_img_n != 0 {
                    if pal_len == 0 {
                        return Err(Error::Stb("tRNS before PLTE"));
                    }
                    if length > pal_len {
                        return Err(Error::Stb("bad tRNS len"));
                    }
                    pal_img_n = 4;
                } else {
                    if img_n & 1 == 0 {
                        return Err(Error::Stb("tRNS with alpha"));
                    }
                    if length != img_n * 2 {
                        return Err(Error::Stb("bad tRNS len"));
                    }
                }
                let mut payload = vec![0u8; length as usize];
                // reads past EOF yield zeros, exactly like stb's get8 loops
                let _ = c.getn(&mut payload);
                trns = Some(payload);
            }
            TYPE_IDAT => {
                if first {
                    return Err(Error::Stb("first not IHDR"));
                }
                if pal_img_n != 0 && pal_len == 0 {
                    return Err(Error::Stb("no PLTE"));
                }
                if length > (1 << 30) {
                    return Err(Error::Stb("IDAT size limit"));
                }
                if (ioff.wrapping_add(length) as i32) < (ioff as i32) {
                    // stb `return 0` with no reason set
                    return Err(Error::StaleReason);
                }
                let start = idata.len();
                idata.resize(start + length as usize, 0);
                if !c.getn(&mut idata[start..]) {
                    return Err(Error::Stb("outofdata"));
                }
                ioff = ioff.wrapping_add(length);
            }
            TYPE_IEND => {
                if first {
                    return Err(Error::Stb("first not IHDR"));
                }
                if idata.is_empty() {
                    return Err(Error::Stb("no IDAT"));
                }
                // stb's zlib header checks, so header acceptance never
                // depends on the crate (which is stricter about cinfo).
                // stbi__parse_zlib_header tests zeof AFTER consuming both
                // bytes, so a stream of two or fewer bytes is "bad zlib
                // header" regardless of content
                if idata.len() <= 2 {
                    return Err(Error::Stb("bad zlib header"));
                }
                let (cmf, flg) = (u32::from(idata[0]), u32::from(idata[1]));
                if (cmf * 256 + flg) % 31 != 0 {
                    return Err(Error::Stb("bad zlib header"));
                }
                if flg & 32 != 0 {
                    return Err(Error::Stb("no preset dict"));
                }
                if cmf & 15 != 8 {
                    return Err(Error::Stb("bad compression"));
                }
                return Ok(Ok(Walked {
                    ihdr: ihdr.expect("first checked"),
                    palette,
                    trns,
                    idata,
                }));
            }
            _ => {
                if first {
                    return Err(Error::Stb("first not IHDR"));
                }
                if ctype & (1 << 29) == 0 {
                    return Err(Error::UnknownChunk([
                        (ctype >> 24) as u8,
                        (ctype >> 16) as u8,
                        (ctype >> 8) as u8,
                        ctype as u8,
                    ]));
                }
                c.skip(length);
            }
        }
        // end of PNG chunk, read and skip CRC
        let _ = c.get32be();
    }
}

fn push_chunk(out: &mut Vec<u8>, ctype: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(ctype);
    out.extend_from_slice(data);
    out.extend_from_slice(&[0u8; 4]); // dummy CRC; verification is off
}

/// Reassemble the walked pieces into a canonical PNG for the crate.
fn reconstruct(w: &mut Walked) -> Vec<u8> {
    let mut out = Vec::with_capacity(w.idata.len() + w.palette.len() + 96);
    out.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&w.ihdr.width.to_be_bytes());
    ihdr.extend_from_slice(&w.ihdr.height.to_be_bytes());
    ihdr.extend_from_slice(&[w.ihdr.depth, w.ihdr.color, 0, 0, w.ihdr.interlace]);
    push_chunk(&mut out, b"IHDR", &ihdr);
    // PLTE only where the spec allows it (stb parses-and-ignores it for
    // grayscale; the reconstruction just leaves it out there)
    if !w.palette.is_empty() && (w.ihdr.color == 3 || w.ihdr.color & 2 != 0) {
        push_chunk(&mut out, b"PLTE", &w.palette);
    }
    if let Some(trns) = &w.trns {
        push_chunk(&mut out, b"tRNS", trns);
    }
    // normalize the zlib CMF byte to cinfo=7 (stb ignores the declared
    // window; the crate does not have to): keep CM=8, rewrite FLG so the
    // fcheck holds with FDICT clear. 0x78 0x01 is the canonical pair —
    // (0x7801 % 31) == 0. Moved out of `w` rather than cloned: the IDAT
    // payload runs to the ported ioff i32 bound, so a clone would put a
    // second multi-hundred-MB copy alongside the one push_chunk makes.
    let mut idata = core::mem::take(&mut w.idata);
    idata[0] = 0x78;
    idata[1] = 0x01;
    push_chunk(&mut out, b"IDAT", &idata);
    push_chunk(&mut out, b"IEND", &[]);
    out
}

/// Decode the crate's output (any post-EXPAND layout) to RGBA8, with 16-bit
/// samples reduced by taking the high byte (stb's stbi__convert_16_to_8).
fn to_rgba(info: &png::OutputInfo, buf: &[u8]) -> Vec<u8> {
    use png::{BitDepth, ColorType};
    let pixels = info.width as usize * info.height as usize;
    let mut out = vec![0u8; pixels * 4];
    let wide = info.bit_depth == BitDepth::Sixteen;
    let step = if wide { 2 } else { 1 };
    // sample i of the source row data, MSB for 16-bit
    let sample = |base: usize, i: usize| buf[base + i * step];
    let comp = match info.color_type {
        ColorType::Grayscale => 1,
        ColorType::GrayscaleAlpha => 2,
        ColorType::Rgb => 3,
        ColorType::Rgba => 4,
        ColorType::Indexed => 1, // EXPAND leaves no indexed output
    };
    for (i, px) in out.chunks_exact_mut(4).enumerate() {
        let base = i * comp * step;
        match comp {
            1 => {
                let v = sample(base, 0);
                px.copy_from_slice(&[v, v, v, 255]);
            }
            2 => {
                let v = sample(base, 0);
                px.copy_from_slice(&[v, v, v, sample(base, 1)]);
            }
            3 => px.copy_from_slice(&[sample(base, 0), sample(base, 1), sample(base, 2), 255]),
            _ => {
                px.copy_from_slice(&[
                    sample(base, 0),
                    sample(base, 1),
                    sample(base, 2),
                    sample(base, 3),
                ]);
            }
        }
    }
    out
}

/// Full decode. `file` is the whole resource, already classified as PNG by
/// [`crate::stb_sniff`] (8-byte signature present).
pub fn decode(file: &[u8]) -> Result<Png, Error> {
    let mut walked = match walk(file)? {
        Ok(w) => w,
        Err(()) => return Ok(Png::Fallback),
    };

    // stb bounds every decode stage with stbi__mad3sizes_valid and degrades
    // an overflowing product to a recoverable "outofmem"; the largest stage
    // for req_comp=4 is the RGBA conversion buffer, x*y*4 samples of 1 or 2
    // bytes. Without this gate a small file declaring huge dims would make
    // the Rust side attempt the multi-GiB allocations for real. COMPAT:
    // where the C's *observed* reason differs (it reports whichever stage
    // fails first, e.g. "not enough pixels" when the IDAT data is also
    // short, or overflows an int multiply in convert_format16 — UB), the
    // reject *decision* is what parity covers (compared masked).
    let bps: u64 = if walked.ihdr.depth == 16 { 2 } else { 1 };
    if u64::from(walked.ihdr.width) * u64::from(walked.ihdr.height) * 4 * bps > i32::MAX as u64 {
        return Err(Error::Stb("outofmem"));
    }

    let canonical = reconstruct(&mut walked);
    let mut decoder = png::Decoder::new(std::io::Cursor::new(&canonical));
    decoder.set_transformations(png::Transformations::EXPAND);
    decoder.ignore_checksums(true);
    // acceptance is governed by the stb-mirrored guards in the walk and the
    // output gate above, not by the crate's default 64 MiB budget; the gate
    // bounds every remaining crate allocation to the int range
    decoder.set_limits(png::Limits { bytes: usize::MAX });
    let mut reader = decoder
        .read_info()
        .map_err(|e| Error::Crate(e.to_string()))?;
    let mut buf = vec![
        0u8;
        reader
            .output_buffer_size()
            .ok_or_else(|| Error::Crate("size".into()))?
    ];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| Error::Crate(e.to_string()))?;

    Ok(Png::Image {
        width: walked.ihdr.width as i32,
        height: walked.ihdr.height as i32,
        rgba: to_rgba(&info, &buf),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(ctype: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        push_chunk(&mut v, ctype, data);
        v
    }

    fn sig() -> Vec<u8> {
        vec![137, 80, 78, 71, 13, 10, 26, 10]
    }

    fn ihdr(w: u32, h: u32, depth: u8, color: u8, interlace: u8) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&w.to_be_bytes());
        d.extend_from_slice(&h.to_be_bytes());
        d.extend_from_slice(&[depth, color, 0, 0, interlace]);
        chunk(b"IHDR", &d)
    }

    /// stored-block zlib stream (cmf 0x78, valid fcheck)
    fn stored_zlib(raw: &[u8]) -> Vec<u8> {
        let mut z = vec![0x78, 0x01, 0x01];
        z.extend_from_slice(&(raw.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(raw.len() as u16)).to_le_bytes());
        z.extend_from_slice(raw);
        z.extend_from_slice(&[0, 0, 0, 0]); // adler (ignored)
        z
    }

    #[test]
    fn minimal_gray_decodes() {
        let mut f = sig();
        f.extend(ihdr(2, 1, 8, 0, 0));
        f.extend(chunk(b"IDAT", &stored_zlib(&[0, 10, 200]))); // filter 0 + 2 px
        f.extend(chunk(b"IEND", &[]));
        let out = decode(&f).unwrap();
        assert_eq!(
            out,
            Png::Image {
                width: 2,
                height: 1,
                rgba: vec![10, 10, 10, 255, 200, 200, 200, 255],
            }
        );
    }

    #[test]
    fn structural_rejects_match_stb_reasons() {
        // no chunks after the signature: the zero header hits stb's default
        // arm, whose `first` check fires before the critical-unknown one
        assert_eq!(decode(&sig()), Err(Error::Stb("first not IHDR")));
        // same zero header *after* a valid IHDR (missing IEND): critical
        // unknown chunk with four NUL name bytes
        let mut f = sig();
        f.extend(ihdr(1, 1, 8, 0, 0));
        assert_eq!(decode(&f), Err(Error::UnknownChunk([0, 0, 0, 0])));
        // first chunk not IHDR
        let mut f = sig();
        f.extend(chunk(b"IDAT", &[1, 2, 3]));
        assert_eq!(decode(&f), Err(Error::Stb("first not IHDR")));
        // bad IHDR length
        let mut f = sig();
        f.extend(chunk(b"IHDR", &[0; 12]));
        assert_eq!(decode(&f), Err(Error::Stb("bad IHDR len")));
        // missing IDAT
        let mut f = sig();
        f.extend(ihdr(1, 1, 8, 0, 0));
        f.extend(chunk(b"IEND", &[]));
        assert_eq!(decode(&f), Err(Error::Stb("no IDAT")));
        // unknown critical chunk
        let mut f = sig();
        f.extend(ihdr(1, 1, 8, 0, 0));
        f.extend(chunk(b"AbCd", &[]));
        assert_eq!(decode(&f), Err(Error::UnknownChunk(*b"AbCd")));
        // truncated IDAT payload
        let mut f = sig();
        f.extend(ihdr(1, 1, 8, 0, 0));
        f.extend_from_slice(&8u32.to_be_bytes());
        f.extend_from_slice(b"IDAT");
        f.extend_from_slice(&[1, 2]); // 2 of 8 bytes, then EOF
        assert_eq!(decode(&f), Err(Error::Stb("outofdata")));
    }

    #[test]
    fn oversized_output_rejects_outofmem_before_the_crate_runs() {
        // 16-bit gray 30000x30000: the RGBA output would be 7.2 GB. The
        // gate must reject with stb's "outofmem" (not a crate error, and
        // certainly not by attempting the allocation)
        let mut f = sig();
        f.extend(ihdr(30000, 30000, 16, 0, 0));
        f.extend(chunk(b"IDAT", &stored_zlib(&[0u8; 8])));
        f.extend(chunk(b"IEND", &[]));
        assert_eq!(decode(&f), Err(Error::Stb("outofmem")));
    }

    #[test]
    fn cgbi_routes_to_fallback() {
        let mut f = sig();
        f.extend(chunk(b"CgBI", &[0; 4]));
        f.extend(ihdr(1, 1, 8, 0, 0));
        assert_eq!(decode(&f), Ok(Png::Fallback));
    }

    #[test]
    fn bad_zlib_headers_reject_like_stb() {
        let build = |z: &[u8]| {
            let mut f = sig();
            f.extend(ihdr(1, 1, 8, 0, 0));
            f.extend(chunk(b"IDAT", z));
            f.extend(chunk(b"IEND", &[]));
            f
        };
        assert_eq!(decode(&build(&[0x78])), Err(Error::Stb("bad zlib header")));
        // an exactly-two-byte stream trips stb's post-read zeof check even
        // when the two bytes are themselves a valid header
        assert_eq!(
            decode(&build(&[0x78, 0x01])),
            Err(Error::Stb("bad zlib header"))
        );
        assert_eq!(
            decode(&build(&[0x78, 0x00, 0x00])),
            Err(Error::Stb("bad zlib header"))
        );
        // FDICT set with a valid fcheck (0x7820 % 31 == 0)
        assert_eq!(
            decode(&build(&[0x78, 0x20, 0x00])),
            Err(Error::Stb("no preset dict"))
        );
        // CM != 8 with a valid fcheck (0x7709 % 31 == 0)
        assert_eq!(
            decode(&build(&[0x77, 0x09, 0x00])),
            Err(Error::Stb("bad compression"))
        );
    }
}
