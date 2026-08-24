//! MD3 (Quake 3 alias model) parsing, ported from `Mod_LoadMD3Model` in
//! `Quake/model_parse.c` (Rust migration Phase 3 M5).
//!
//! Pure layer, same contract as [`crate::mdl`]: decode on-disk records out
//! of byte slices; the `quake-capi` shim owns the single-block
//! `Mem_Alloc`ed `aliashdr_t` array, the `GLMesh_UploadBuffers` handoff and
//! the walk over the file image.
//!
//! `Mod_LoadMD3Model` is handed a bare `const void *` with no length, so the
//! shim cannot bound the image and walks raw pointers exactly as the C does
//! (a malformed .md3 reads past the end on both sides). Everything this
//! module sees is a slice the shim built for one record, so the bounds here
//! are only a backstop.
//!
//! The layout constants duplicate `quake_types::md3` on purpose: this crate
//! has no dependencies (ADR-003), and `tests/mdx_abi.rs` gates the mirrors
//! those constants shadow.

// on-disk record sizes (`gl_model.h`, "MD3 MODELS")
pub const MD3_HEADER_SIZE: usize = 108;
pub const MD3_FRAME_SIZE: usize = 56;
pub const MD3_SURFACE_SIZE: usize = 108;
pub const MD3_TRIANGLE_SIZE: usize = 12;
pub const MD3_ST_SIZE: usize = 8;
pub const MD3_XYZNORMAL_SIZE: usize = 8;

// `offsetof (md3Header_t, ...)`
pub const OFS_HDR_VERSION: usize = 4;
const OFS_HDR_IDENT: usize = 0;
const OFS_HDR_FLAGS: usize = 72;
const OFS_HDR_NUMFRAMES: usize = 76;
const OFS_HDR_NUMSURFACES: usize = 84;
const OFS_HDR_OFSFRAMES: usize = 92;
const OFS_HDR_OFSSURFACES: usize = 100;

/// The header prefix the C has touched by the time it has decided
/// `numFrames`/`numSurfaces` are legal (`numSurfaces` is the last of the
/// three fields the pre-allocation `Sys_Error` gates read).
pub const MD3_HEADER_COUNTS_PREFIX: usize = OFS_HDR_NUMSURFACES + 4;
/// Prefix `parse_flags` needs (the C re-reads `flags` off the header only
/// after the whole surface loop has run).
pub const MD3_HEADER_FLAGS_PREFIX: usize = OFS_HDR_FLAGS + 4;
/// Prefix `parse_header_offsets` needs.
pub const MD3_HEADER_OFFSETS_PREFIX: usize = OFS_HDR_OFSSURFACES + 4;
/// Prefix `parse_surface_ident` needs.
pub const MD3_SURFACE_IDENT_PREFIX: usize = OFS_SURF_IDENT + 4;
/// Prefix `parse_surface_numframes` needs.
pub const MD3_SURFACE_NUMFRAMES_PREFIX: usize = OFS_SURF_NUMFRAMES + 4;

// `offsetof (md3Frame_t, name)` — the loader reads nothing else from a frame
pub const OFS_FRAME_NAME: usize = 40;
pub const MD3_FRAME_NAME_SIZE: usize = 16;

// `offsetof (md3Surface_t, ...)`
const OFS_SURF_IDENT: usize = 0;
pub const OFS_SURF_NAME: usize = 4;
const OFS_SURF_NUMFRAMES: usize = 72;
const OFS_SURF_NUMVERTS: usize = 80;
const OFS_SURF_NUMTRIANGLES: usize = 84;
const OFS_SURF_OFSTRIANGLES: usize = 88;
const OFS_SURF_OFSST: usize = 96;
const OFS_SURF_OFSXYZNORMALS: usize = 100;
const OFS_SURF_OFSEND: usize = 104;

pub const MD3_VERSION: i32 = 15;
/// `('I' << 0) | ('D' << 8) | ('P' << 16) | ('3' << 24)`
pub const IDMD3HEADER: i32 = 0x3350_4449;
/// `gl_model.h`
pub const MD3_XYZ_SCALE: f32 = 1.0 / 64.0;
pub const MAXALIASFRAMES: i32 = 2048;
pub const MAX_SURFACES: i32 = 32;

fn i32_at(b: &[u8], o: usize) -> i32 {
    i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// `LittleLong (pinheader->version)` off the smallest prefix that read
/// touches. Split out so the shim can apply the version `Sys_Error` before
/// materialising anything wider than the C has looked at (the M4 MDL/SPR
/// precedent).
pub fn parse_version(prefix: &[u8]) -> i32 {
    i32_at(prefix, OFS_HDR_VERSION)
}

/// `numSurfaces` / `numFrames`, the two counts the remaining pre-allocation
/// `Sys_Error` gates test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeaderCounts {
    pub num_surfaces: i32,
    pub num_frames: i32,
}

pub fn parse_header_counts(prefix: &[u8]) -> HeaderCounts {
    HeaderCounts {
        num_surfaces: i32_at(prefix, OFS_HDR_NUMSURFACES),
        num_frames: i32_at(prefix, OFS_HDR_NUMFRAMES),
    }
}

/// The two lump offsets the loader walks from, read once every
/// `Sys_Error` gate has passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeaderOffsets {
    pub ofs_frames: i32,
    pub ofs_surfaces: i32,
}

pub fn parse_header_offsets(bytes: &[u8]) -> HeaderOffsets {
    HeaderOffsets {
        ofs_frames: i32_at(bytes, OFS_HDR_OFSFRAMES),
        ofs_surfaces: i32_at(bytes, OFS_HDR_OFSSURFACES),
    }
}

/// `LittleLong (pinheader->flags)`.
///
/// The C reads this *after* the surface loop, and the loop `q_strtrim`s
/// surface names in place, so on a malformed image where a surface name
/// overlaps the header the value can differ from a read taken up front. Kept
/// as a separate late read for that reason.
pub fn parse_flags(prefix: &[u8]) -> i32 {
    i32_at(prefix, OFS_HDR_FLAGS)
}

/// `md3Header_t::ident` — unused by the loader, exposed for tests/fixtures.
pub fn parse_ident(prefix: &[u8]) -> i32 {
    i32_at(prefix, OFS_HDR_IDENT)
}

/// `md3Surface_t` as `Mod_LoadMD3Model` reads it (`numShaders`/`ofsShaders`
/// are skipped: the C leaves the shader block as a TODO).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceHeader {
    pub ident: i32,
    pub num_frames: i32,
    pub num_verts: i32,
    pub num_triangles: i32,
    pub ofs_triangles: i32,
    pub ofs_st: i32,
    pub ofs_xyz_normals: i32,
    pub ofs_end: i32,
}

/// `LittleLong (pinsurface->ident)` — the first thing the C reads off a
/// surface, and the only thing it reads before the corrupt-ident
/// `Sys_Error`.
pub fn parse_surface_ident(prefix: &[u8]) -> i32 {
    i32_at(prefix, OFS_SURF_IDENT)
}

/// `LittleLong (pinsurface->numFrames)` — the second gate.
pub fn parse_surface_numframes(prefix: &[u8]) -> i32 {
    i32_at(prefix, OFS_SURF_NUMFRAMES)
}

pub fn parse_surface_header(bytes: &[u8]) -> SurfaceHeader {
    SurfaceHeader {
        ident: i32_at(bytes, OFS_SURF_IDENT),
        num_frames: i32_at(bytes, OFS_SURF_NUMFRAMES),
        num_verts: i32_at(bytes, OFS_SURF_NUMVERTS),
        num_triangles: i32_at(bytes, OFS_SURF_NUMTRIANGLES),
        ofs_triangles: i32_at(bytes, OFS_SURF_OFSTRIANGLES),
        ofs_st: i32_at(bytes, OFS_SURF_OFSST),
        ofs_xyz_normals: i32_at(bytes, OFS_SURF_OFSXYZNORMALS),
        ofs_end: i32_at(bytes, OFS_SURF_OFSEND),
    }
}

/// `LittleLong (pintriangle->indexes[j])`, narrowed to the `unsigned short`
/// the index buffer holds. The narrowing is the C's:
/// `poutindexes[j] = LittleLong (...)` with no range check.
pub fn parse_triangle_indexes(bytes: &[u8]) -> [u16; 3] {
    [
        i32_at(bytes, 0) as u16,
        i32_at(bytes, 4) as u16,
        i32_at(bytes, 8) as u16,
    ]
}

/// `poutst[j].st[0] = pinst[j].s`.
///
/// COMPAT: the C reads the two floats straight out of the file image with
/// **no** `LittleFloat`, unlike every integer field around it, so an MD3
/// authored on one endianness has swapped texcoords on the other. Native
/// reads keep that bug-for-bug (`from_ne_bytes`, not `from_le_bytes`).
pub fn parse_st_native(bytes: &[u8]) -> [f32; 2] {
    [
        f32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        f32::from_ne_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdr() -> Vec<u8> {
        let mut b = vec![0u8; MD3_HEADER_SIZE];
        b[OFS_HDR_IDENT..][..4].copy_from_slice(&IDMD3HEADER.to_le_bytes());
        b[OFS_HDR_VERSION..][..4].copy_from_slice(&MD3_VERSION.to_le_bytes());
        b[OFS_HDR_FLAGS..][..4].copy_from_slice(&0x1234i32.to_le_bytes());
        b[OFS_HDR_NUMFRAMES..][..4].copy_from_slice(&7i32.to_le_bytes());
        b[OFS_HDR_NUMSURFACES..][..4].copy_from_slice(&3i32.to_le_bytes());
        b[OFS_HDR_OFSFRAMES..][..4].copy_from_slice(&108i32.to_le_bytes());
        b[OFS_HDR_OFSSURFACES..][..4].copy_from_slice(&500i32.to_le_bytes());
        b
    }

    #[test]
    fn header_fields_decode() {
        let b = hdr();
        assert_eq!(parse_version(&b[..8]), MD3_VERSION);
        assert_eq!(
            parse_header_counts(&b[..MD3_HEADER_COUNTS_PREFIX]),
            HeaderCounts {
                num_surfaces: 3,
                num_frames: 7
            }
        );
        assert_eq!(parse_ident(&b[..4]), IDMD3HEADER);
        assert_eq!(parse_flags(&b[..MD3_HEADER_FLAGS_PREFIX]), 0x1234);
        assert_eq!(
            parse_header_offsets(&b[..MD3_HEADER_OFFSETS_PREFIX]),
            HeaderOffsets {
                ofs_frames: 108,
                ofs_surfaces: 500,
            }
        );
    }

    #[test]
    fn surface_header_fields_decode() {
        let mut b = vec![0u8; MD3_SURFACE_SIZE];
        b[OFS_SURF_IDENT..][..4].copy_from_slice(&IDMD3HEADER.to_le_bytes());
        b[OFS_SURF_NUMFRAMES..][..4].copy_from_slice(&7i32.to_le_bytes());
        b[OFS_SURF_NUMVERTS..][..4].copy_from_slice(&11i32.to_le_bytes());
        b[OFS_SURF_NUMTRIANGLES..][..4].copy_from_slice(&5i32.to_le_bytes());
        b[OFS_SURF_OFSTRIANGLES..][..4].copy_from_slice(&108i32.to_le_bytes());
        b[OFS_SURF_OFSST..][..4].copy_from_slice(&200i32.to_le_bytes());
        b[OFS_SURF_OFSXYZNORMALS..][..4].copy_from_slice(&300i32.to_le_bytes());
        b[OFS_SURF_OFSEND..][..4].copy_from_slice(&900i32.to_le_bytes());
        assert_eq!(
            parse_surface_ident(&b[..MD3_SURFACE_IDENT_PREFIX]),
            IDMD3HEADER
        );
        assert_eq!(
            parse_surface_numframes(&b[..MD3_SURFACE_NUMFRAMES_PREFIX]),
            7
        );
        assert_eq!(
            parse_surface_header(&b),
            SurfaceHeader {
                ident: IDMD3HEADER,
                num_frames: 7,
                num_verts: 11,
                num_triangles: 5,
                ofs_triangles: 108,
                ofs_st: 200,
                ofs_xyz_normals: 300,
                ofs_end: 900,
            }
        );
    }

    #[test]
    fn triangle_indexes_narrow_like_the_c() {
        let mut b = vec![0u8; MD3_TRIANGLE_SIZE];
        b[0..4].copy_from_slice(&1i32.to_le_bytes());
        b[4..8].copy_from_slice(&0x0001_2345i32.to_le_bytes());
        b[8..12].copy_from_slice(&(-1i32).to_le_bytes());
        assert_eq!(parse_triangle_indexes(&b), [1, 0x2345, 0xffff]);
    }

    #[test]
    fn st_is_read_without_byteswapping() {
        let v = [0.25f32, -3.5f32];
        let mut b = Vec::new();
        b.extend_from_slice(&v[0].to_ne_bytes());
        b.extend_from_slice(&v[1].to_ne_bytes());
        assert_eq!(parse_st_native(&b), v);
    }
}
