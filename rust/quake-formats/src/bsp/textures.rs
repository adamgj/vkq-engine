//! Mod_ParseTextures / Mod_TextureTypeFromName. The wad lookup, texture_t
//! allocation, and Con_* messages are shim-side; this layer turns the
//! texture lump into per-slot work records.

use super::lumps::{
    TEXTYPE_CUTOUT, TEXTYPE_LAVA, TEXTYPE_SKY, TEXTYPE_SLIME, TEXTYPE_TELE, TEXTYPE_WATER,
};
use super::{i32_at, u16_at, u32_at};

pub const TEXTYPE_DEFAULT: i32 = 0;
pub const MIPTEX_SIZE: usize = 40;
pub const MIPTEX64_SIZE: usize = 44;

/// Mod_TextureTypeFromName over the raw 16-byte name field
pub fn texture_type_from_name(name: &[u8; 16]) -> i32 {
    if name[0] == b'*' || name[0] == b'!' {
        if name[1..].starts_with(b"lava") {
            return TEXTYPE_LAVA;
        }
        if name[1..].starts_with(b"slime") {
            return TEXTYPE_SLIME;
        }
        if name[1..].starts_with(b"tele") {
            return TEXTYPE_TELE;
        }
        return TEXTYPE_WATER;
    }
    if name[0] == b'{' {
        return TEXTYPE_CUTOUT;
    }
    if name[..3].eq_ignore_ascii_case(b"sky") {
        return TEXTYPE_SKY;
    }
    TEXTYPE_DEFAULT
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TexWork {
    /// dataofs == -1 (or, divergently, a dataofs whose 40-byte header would
    /// leave the lump — UB in C): slot stays NULL
    Skip,
    /// C: `Con_Warning ("Zero sized texture %s in %s!\n", ...)`; slot stays
    /// NULL
    ZeroSized {
        name: [u8; 16],
    },
    Tex(Box<TexRec>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TexRec {
    pub name: [u8; 16],
    pub width: u32,
    pub height: u32,
    /// Raw file offsets from the 40-byte header read. COMPAT: for Q64 the C
    /// memcpy of `sizeof (miptex_t)` bytes over a miptex64_t record puts
    /// `shift` in offsets[0] and shifts the rest — preserved as-is; the shim
    /// adds `sizeof (texture_t) - sizeof (miptex_t)` when storing.
    pub offsets: [u32; 4],
    /// offsets[0] == 0: try Mod_LoadWadTexture first; on failure fall
    /// through to this record's internal load
    pub external_candidate: bool,
    pub textype: i32,
    /// The C `int pixels` at allocation time: `width * height / 64 * 85`
    /// (u32 arithmetic reinterpreted as int), plus the Valve palette bytes
    pub alloc_pixels: i32,
    pub palette: bool,
    /// Q64 only, read separately from dataofs + 24; 0 otherwise
    pub shift: i32,
    /// lump-relative `pixels_p` (dataofs + 40): source_offset base, clamp
    /// base, and non-Q64 copy source
    pub pixels_ofs: usize,
    /// Q64 copy source (dataofs + 44); equals pixels_ofs otherwise
    pub copy_ofs: usize,
    /// post-clamp byte count to copy after the allocated texture_t. COMPAT:
    /// the Q64 clamp is computed against pixels_ofs but the copy starts 4
    /// bytes later, so C can read past the lump end (UB) — the shim must
    /// additionally bound the read to the lump slice.
    pub copy_len: i32,
    /// C: `Con_DPrintf ("Texture %s extends past end of lump\n", ...)`
    pub truncated: bool,
}

/// Mod_ParseTextures over the texture lump. Returns the C `nummiptex` and
/// one work record per slot (`mod->numtextures` is nummiptex + 2; the two
/// dummy slots stay NULL). An empty lump also triggers
/// `Con_Printf ("Mod_LoadTextures: no textures in bsp file\n")` shim-side.
pub fn parse_textures(lump: &[u8], valve: bool, q64: bool) -> (i32, Vec<TexWork>) {
    if lump.is_empty() {
        return (0, Vec::new());
    }
    let nummiptex = i32_at(lump, 0);
    let mut work = Vec::new();
    for i in 0..nummiptex.max(0) {
        let dir_ofs = 4 + 4 * i as usize;
        if dir_ofs + 4 > lump.len() {
            work.push(TexWork::Skip); // directory past lump end: UB in C
            continue;
        }
        let dataofs = i32_at(lump, dir_ofs);
        if dataofs == -1 {
            work.push(TexWork::Skip);
            continue;
        }
        let Ok(dataofs) = usize::try_from(dataofs) else {
            work.push(TexWork::Skip); // negative dataofs: UB in C
            continue;
        };
        if dataofs + MIPTEX_SIZE > lump.len() {
            work.push(TexWork::Skip); // header past lump end: UB in C
            continue;
        }
        let hdr = &lump[dataofs..dataofs + MIPTEX_SIZE];
        let mut name = [0u8; 16];
        name.copy_from_slice(&hdr[..16]);
        let width = u32_at(hdr, 16);
        let height = u32_at(hdr, 20);
        let offsets = [
            u32_at(hdr, 24),
            u32_at(hdr, 28),
            u32_at(hdr, 32),
            u32_at(hdr, 36),
        ];

        if width == 0 || height == 0 {
            work.push(TexWork::ZeroSized { name });
            continue;
        }

        let mut pixels = (width.wrapping_mul(height) / 64).wrapping_mul(85) as i32;
        let pixels_ofs = dataofs + MIPTEX_SIZE;
        if valve {
            // palette check uses the pre-+2 pixel count
            if let Some(color_ofs) =
                pixels_ofs.checked_add(usize::try_from(pixels).unwrap_or(usize::MAX))
            {
                if color_ofs + 2 <= lump.len() {
                    let colors = i32::from(u16_at(lump, color_ofs));
                    pixels = pixels.wrapping_add(colors.wrapping_mul(3));
                }
            }
            pixels = pixels.wrapping_add(2);
        }
        let alloc_pixels = pixels;

        // post-alloc clamp: pixels_p + pixels vs lump end
        let mut truncated = false;
        let past_end = match usize::try_from(pixels) {
            Ok(p) => pixels_ofs + p > lump.len(),
            Err(_) => true, // negative pixels: UB pointer math in C
        };
        if past_end {
            truncated = true;
            pixels = (lump.len() as i64 - pixels_ofs as i64).max(0) as i32;
        }

        let (shift, copy_ofs) = if q64 {
            (i32_at(lump, dataofs + 24), dataofs + MIPTEX64_SIZE)
        } else {
            (0, pixels_ofs)
        };

        work.push(TexWork::Tex(Box::new(TexRec {
            name,
            width,
            height,
            offsets,
            external_candidate: offsets[0] == 0,
            textype: texture_type_from_name(&name),
            alloc_pixels,
            palette: valve,
            shift,
            pixels_ofs,
            copy_ofs,
            copy_len: pixels,
            truncated,
        })));
    }
    (nummiptex, work)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn miptex(name: &str, w: u32, h: u32, ofs0: u32) -> Vec<u8> {
        let mut r = vec![0u8; MIPTEX_SIZE];
        r[..name.len()].copy_from_slice(name.as_bytes());
        r[16..20].copy_from_slice(&w.to_le_bytes());
        r[20..24].copy_from_slice(&h.to_le_bytes());
        r[24..28].copy_from_slice(&ofs0.to_le_bytes());
        r
    }

    fn lump_with(entries: &[Vec<u8>]) -> Vec<u8> {
        let n = entries.len() as i32;
        let dir = 4 + 4 * entries.len();
        let mut lump = n.to_le_bytes().to_vec();
        let mut ofs = dir;
        for e in entries {
            lump.extend_from_slice(&(ofs as i32).to_le_bytes());
            ofs += e.len();
        }
        for e in entries {
            lump.extend_from_slice(e);
        }
        lump
    }

    #[test]
    fn texture_types() {
        let n = |s: &str| {
            let mut b = [0u8; 16];
            b[..s.len()].copy_from_slice(s.as_bytes());
            b
        };
        assert_eq!(texture_type_from_name(&n("*lava1")), TEXTYPE_LAVA);
        assert_eq!(texture_type_from_name(&n("!slime0")), TEXTYPE_SLIME);
        assert_eq!(texture_type_from_name(&n("*teleport")), TEXTYPE_TELE);
        assert_eq!(texture_type_from_name(&n("*water")), TEXTYPE_WATER);
        assert_eq!(texture_type_from_name(&n("{fence")), TEXTYPE_CUTOUT);
        assert_eq!(texture_type_from_name(&n("SKY1")), TEXTYPE_SKY);
        assert_eq!(texture_type_from_name(&n("wall")), TEXTYPE_DEFAULT);
    }

    #[test]
    fn basic_lump() {
        let mut tex = miptex("wall", 8, 8, MIPTEX_SIZE as u32);
        tex.extend(std::iter::repeat_n(7u8, 85)); // 8*8/64*85
        let lump = lump_with(&[tex]);
        let (n, work) = parse_textures(&lump, false, false);
        assert_eq!(n, 1);
        let TexWork::Tex(r) = &work[0] else { panic!() };
        assert_eq!((r.width, r.height, r.alloc_pixels), (8, 8, 85));
        assert!(!r.external_candidate && !r.truncated);
        assert_eq!(r.copy_ofs, r.pixels_ofs);
        assert_eq!(r.copy_len, 85);
    }

    #[test]
    fn skip_zero_and_external() {
        let zero = miptex("empty", 0, 8, 4);
        let ext = miptex("fromwad", 8, 8, 0);
        let mut lump = lump_with(&[zero, ext]);
        // patch entry 0's dataofs to -1 exercise Skip separately below
        let (_, work) = parse_textures(&lump, false, false);
        assert!(matches!(&work[0], TexWork::ZeroSized { name } if name.starts_with(b"empty")));
        let TexWork::Tex(r) = &work[1] else { panic!() };
        assert!(r.external_candidate);
        assert!(r.truncated); // no pixel data in the lump

        lump[4..8].copy_from_slice(&(-1i32).to_le_bytes());
        let (_, work) = parse_textures(&lump, false, false);
        assert_eq!(work[0], TexWork::Skip);
    }

    #[test]
    fn truncated_pixels_clamped() {
        let mut tex = miptex("wall", 8, 8, MIPTEX_SIZE as u32);
        tex.extend(std::iter::repeat_n(7u8, 40)); // only 40 of 85 bytes
        let lump = lump_with(&[tex]);
        let (_, work) = parse_textures(&lump, false, false);
        let TexWork::Tex(r) = &work[0] else { panic!() };
        assert_eq!(r.alloc_pixels, 85);
        assert!(r.truncated);
        assert_eq!(r.copy_len, 40);
    }

    #[test]
    fn valve_palette_sizing() {
        let mut tex = miptex("wall", 8, 8, MIPTEX_SIZE as u32);
        tex.extend(std::iter::repeat_n(7u8, 85));
        tex.extend_from_slice(&4u16.to_le_bytes()); // 4-color palette
        tex.extend(std::iter::repeat_n(0u8, 12));
        let lump = lump_with(&[tex]);
        let (_, work) = parse_textures(&lump, true, false);
        let TexWork::Tex(r) = &work[0] else { panic!() };
        assert!(r.palette);
        assert_eq!(r.alloc_pixels, 85 + 4 * 3 + 2);
        assert!(!r.truncated);

        // palette count missing: only the +2 is added, then clamped
        let mut tex = miptex("wall", 8, 8, MIPTEX_SIZE as u32);
        tex.extend(std::iter::repeat_n(7u8, 85));
        let lump = lump_with(&[tex]);
        let (_, work) = parse_textures(&lump, true, false);
        let TexWork::Tex(r) = &work[0] else { panic!() };
        assert_eq!(r.alloc_pixels, 87);
        assert!(r.truncated);
        assert_eq!(r.copy_len, 85);
    }

    #[test]
    fn q64_header_aliasing() {
        // miptex64: name, w, h, shift@24, offsets@28 — the 40-byte read sees
        // shift as offsets[0]
        let mut tex = vec![0u8; MIPTEX64_SIZE];
        tex[..4].copy_from_slice(b"wall");
        tex[16..20].copy_from_slice(&8u32.to_le_bytes());
        tex[20..24].copy_from_slice(&8u32.to_le_bytes());
        tex[24..28].copy_from_slice(&2u32.to_le_bytes()); // shift
        tex[28..32].copy_from_slice(&44u32.to_le_bytes()); // real offsets[0]
        tex.extend(std::iter::repeat_n(9u8, 85 + 4));
        let lump = lump_with(&[tex]);
        let (_, work) = parse_textures(&lump, false, true);
        let TexWork::Tex(r) = &work[0] else { panic!() };
        assert_eq!(r.shift, 2);
        assert_eq!(r.offsets[0], 2); // aliased shift, preserved
        assert!(!r.external_candidate); // shift != 0 masks the check
        assert_eq!(r.copy_ofs, r.pixels_ofs + 4);
        assert!(!r.truncated);

        // shift == 0 makes a Q64 texture an external candidate (compat bug)
        let mut tex2 = lump.clone();
        let shift_at = 4 + 4 + 24;
        tex2[shift_at..shift_at + 4].copy_from_slice(&0u32.to_le_bytes());
        let (_, work) = parse_textures(&tex2, false, true);
        let TexWork::Tex(r) = &work[0] else { panic!() };
        assert!(r.external_candidate);
    }
}
