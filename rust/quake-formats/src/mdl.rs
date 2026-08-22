//! MDL (alias model) parsing, ported from the alias range of
//! `Quake/model_parse.c` (Rust migration Phase 3 M4).
//!
//! Pure layer, same contract as [`crate::bsp`]: decode on-disk records out
//! of byte slices, compute the derived values, report the diagnostics in the
//! order the C emits them. The `quake-capi` shim owns the `Mem_Alloc`ed
//! `aliashdr_t`, the `stverts`/`triangles`/`poseverts` engine globals and
//! the walk over the file image.
//!
//! Unlike the BSP lumps, `Mod_ParseAliasModel` is handed a bare `void *`
//! with no length, so the shim cannot bound the image; it walks raw
//! pointers exactly as the C does (a malformed .mdl reads past the end on
//! both sides). Everything this module sees is a slice the shim copied out
//! of that image, so the bounds here are only a backstop.
//!
//! The layout constants duplicate `quake_types::modelgen` on purpose: this
//! crate has no dependencies (ADR-003), and `tests/alias_abi.rs` gates the
//! mirrors those constants shadow.

// on-disk record sizes (`modelgen.h`)
pub const MDL_T_SIZE: usize = 84;
pub const STVERT_T_SIZE: usize = 12;
pub const DTRIANGLE_T_SIZE: usize = 16;
pub const TRIVERTX_T_SIZE: usize = 4;
pub const DALIASFRAME_T_SIZE: usize = 24;
pub const DALIASGROUP_T_SIZE: usize = 12;
pub const DALIASINTERVAL_T_SIZE: usize = 4;
pub const DALIASFRAMETYPE_T_SIZE: usize = 4;

// `offsetof (mdl_t, ...)`
const OFS_IDENT: usize = 0;
pub const OFS_VERSION: usize = 4;
const OFS_SCALE: usize = 8;
const OFS_SCALE_ORIGIN: usize = 20;
const OFS_BOUNDINGRADIUS: usize = 32;
const OFS_EYEPOSITION: usize = 36;
const OFS_NUMSKINS: usize = 48;
const OFS_SKINWIDTH: usize = 52;
const OFS_SKINHEIGHT: usize = 56;
const OFS_NUMVERTS: usize = 60;
const OFS_NUMTRIS: usize = 64;
const OFS_NUMFRAMES: usize = 68;
const OFS_SYNCTYPE: usize = 72;
const OFS_FLAGS: usize = 76;
const OFS_SIZE: usize = 80;

pub const ALIAS_VERSION: i32 = 6;
/// `aliasframetype_t`: anything that is not `ALIAS_SINGLE` loads as a group
pub const ALIAS_SINGLE: i32 = 0;

pub const MAXALIASVERTS: i32 = 0x7fff;
pub const MAXALIASVERTS_QS: i32 = 2000;
pub const MAXALIASTRIS_QS: i32 = 4096;
pub const MAXALIASFRAMES: i32 = 2048;
pub const MAX_LBM_HEIGHT: i32 = 480;

/// `glquake.h`: a **double**, and the C multiplies the float `size` by it
/// before narrowing back to float
pub const ALIAS_BASE_SIZE_RATIO: f64 = 1.0 / 11.0;
/// `gl_model.h`
pub const MD3_XYZ_SCALE: f32 = 1.0 / 64.0;

fn i32_at(b: &[u8], o: usize) -> i32 {
    i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn f32_at(b: &[u8], o: usize) -> f32 {
    f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn vec3_at(b: &[u8], o: usize) -> [f32; 3] {
    [f32_at(b, o), f32_at(b, o + 4), f32_at(b, o + 8)]
}

/// `mdl_t` as `Mod_ParseAliasModel` reads it
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MdlHeader {
    pub ident: i32,
    pub version: i32,
    pub scale: [f32; 3],
    pub scale_origin: [f32; 3],
    /// COMPAT: `Quake/model_parse.c` reads this **float** field with
    /// `ReadLongUnaligned` and assigns the result to the float
    /// `aliashdr_t::boundingradius`, so what lands in the header is the
    /// integer reinterpretation of the bits converted to float, not the
    /// float that was written. Preserved bug-for-bug; the field is never
    /// read back by the engine.
    pub boundingradius_as_long: i32,
    pub eyeposition: [f32; 3],
    pub numskins: i32,
    pub skinwidth: i32,
    pub skinheight: i32,
    pub numverts: i32,
    pub numtris: i32,
    pub numframes: i32,
    /// `synctype_t`
    pub synctype: i32,
    pub flags: i32,
    /// the raw on-disk value; see [`scaled_size`]
    pub size: f32,
}

/// `offsetof (daliasframe_t, name)`
pub const OFS_FRAME_NAME: usize = 8;

/// The version field alone, so a caller with an unbounded file image can do
/// the C's version check before it is entitled to the other 76 bytes.
///
/// Panics if `b` is shorter than `OFS_VERSION + 4`.
pub fn parse_version(b: &[u8]) -> i32 {
    assert!(b.len() >= OFS_VERSION + 4);
    i32_at(b, OFS_VERSION)
}

/// Panics if `b` is shorter than [`MDL_T_SIZE`].
pub fn parse_header(b: &[u8]) -> MdlHeader {
    assert!(b.len() >= MDL_T_SIZE);
    MdlHeader {
        ident: i32_at(b, OFS_IDENT),
        version: i32_at(b, OFS_VERSION),
        scale: vec3_at(b, OFS_SCALE),
        scale_origin: vec3_at(b, OFS_SCALE_ORIGIN),
        boundingradius_as_long: i32_at(b, OFS_BOUNDINGRADIUS),
        eyeposition: vec3_at(b, OFS_EYEPOSITION),
        numskins: i32_at(b, OFS_NUMSKINS),
        skinwidth: i32_at(b, OFS_SKINWIDTH),
        skinheight: i32_at(b, OFS_SKINHEIGHT),
        numverts: i32_at(b, OFS_NUMVERTS),
        numtris: i32_at(b, OFS_NUMTRIS),
        numframes: i32_at(b, OFS_NUMFRAMES),
        synctype: i32_at(b, OFS_SYNCTYPE),
        flags: i32_at(b, OFS_FLAGS),
        size: f32_at(b, OFS_SIZE),
    }
}

/// C: `pheader->size = ReadFloatUnaligned (...) * ALIAS_BASE_SIZE_RATIO;`
///
/// COMPAT: ADR-010 -- `float * double` promotes to double, and only the
/// assignment narrows back, so the rounding differs from an all-float
/// multiply. Kept exactly.
pub fn scaled_size(raw: f32) -> f32 {
    (f64::from(raw) * ALIAS_BASE_SIZE_RATIO) as f32
}

/// A `Con_DWarning` or `Sys_Error` from the `Mod_ParseAliasModel` header
/// checks, in the order the C reaches them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Diag {
    SkinTooTall { skinheight: i32 },
    NoVertices,
    TooManyVertices { numverts: i32 },
    VertsExceedQsLimit { numverts: i32 },
    NoTriangles,
    TrisExceedQsLimit { numtris: i32 },
    InvalidFrameCount { numframes: i32 },
}

impl Diag {
    /// `true` for the `Sys_Error` cases; the shim stops at the first one.
    pub fn is_fatal(self) -> bool {
        matches!(
            self,
            Diag::NoVertices
                | Diag::TooManyVertices { .. }
                | Diag::NoTriangles
                | Diag::InvalidFrameCount { .. }
        )
    }
}

/// The header checks after the version test, in C order. The caller emits
/// each warning and aborts on the first fatal, so nothing after a fatal is
/// reported.
///
/// The C runs `check_tris_size` between the triangle-count warning and the
/// frame-count check; the shim runs it after this list instead. The only
/// diagnostic in between is fatal, and `Sys_Error` does not return, so the
/// reordering is unobservable.
pub fn validate(h: &MdlHeader) -> Vec<Diag> {
    let mut out = Vec::new();
    if h.skinheight > MAX_LBM_HEIGHT {
        out.push(Diag::SkinTooTall {
            skinheight: h.skinheight,
        });
    }
    if h.numverts <= 0 {
        out.push(Diag::NoVertices);
        return out;
    }
    if h.numverts > MAXALIASVERTS {
        out.push(Diag::TooManyVertices {
            numverts: h.numverts,
        });
        return out;
    }
    if h.numverts > MAXALIASVERTS_QS {
        out.push(Diag::VertsExceedQsLimit {
            numverts: h.numverts,
        });
    }
    if h.numtris <= 0 {
        out.push(Diag::NoTriangles);
        return out;
    }
    if h.numtris > MAXALIASTRIS_QS {
        out.push(Diag::TrisExceedQsLimit { numtris: h.numtris });
    }
    if h.numframes < 1 {
        out.push(Diag::InvalidFrameCount {
            numframes: h.numframes,
        });
    }
    out
}

/// `stvert_t`: onseam, s, t
pub fn parse_stvert(b: &[u8]) -> [i32; 3] {
    assert!(b.len() >= STVERT_T_SIZE);
    [i32_at(b, 0), i32_at(b, 4), i32_at(b, 8)]
}

/// `dtriangle_t` -> the `mtriangle_t` the C fills in
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Triangle {
    pub facesfront: i32,
    pub vertindex: [i32; 3],
}

pub fn parse_triangle(b: &[u8]) -> Triangle {
    assert!(b.len() >= DTRIANGLE_T_SIZE);
    Triangle {
        facesfront: i32_at(b, 0),
        vertindex: [i32_at(b, 4), i32_at(b, 8), i32_at(b, 12)],
    }
}

/// `daliasframetype_t::type` / `daliasgroup_t::numframes` / any leading int
pub fn parse_i32(b: &[u8]) -> i32 {
    assert!(b.len() >= 4);
    i32_at(b, 0)
}

/// `daliasinterval_t::interval`
pub fn parse_interval(b: &[u8]) -> f32 {
    assert!(b.len() >= DALIASINTERVAL_T_SIZE);
    f32_at(b, 0)
}

/// The `maliasframedesc_t` fields a single `daliasframe_t` supplies
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    /// `trivertx_t::v`, byte values -- no endian swap
    pub bboxmin: [u8; 3],
    pub bboxmax: [u8; 3],
}

/// The `name` field is deliberately not decoded here: the C copies it with
/// `q_strlcpy`, which walks the source past the 16-byte field when it holds no
/// NUL, so the C-ABI shim reads it in place at [`OFS_FRAME_NAME`] instead.
pub fn parse_frame_header(b: &[u8]) -> FrameHeader {
    assert!(b.len() >= DALIASFRAME_T_SIZE);
    FrameHeader {
        bboxmin: [b[0], b[1], b[2]],
        bboxmax: [b[4], b[5], b[6]],
    }
}

/// The `maliasframedesc_t` fields a `daliasgroup_t` supplies
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupHeader {
    pub numframes: i32,
    pub bboxmin: [u8; 3],
    pub bboxmax: [u8; 3],
}

pub fn parse_group_header(b: &[u8]) -> GroupHeader {
    assert!(b.len() >= DALIASGROUP_T_SIZE);
    GroupHeader {
        numframes: i32_at(b, 0),
        bboxmin: [b[4], b[5], b[6]],
        bboxmax: [b[8], b[9], b[10]],
    }
}

/// `Mod_CalcAliasBounds` output: the six bounding boxes on `qmodel_t`
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AliasBounds {
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub ymins: [f32; 3],
    pub ymaxs: [f32; 3],
    pub rmins: [f32; 3],
    pub rmaxs: [f32; 3],
}

/// C `q_min_f`/`q_max_f`: plain ternaries, so a NaN operand propagates the
/// *second* argument rather than being ignored the way `f32::min` would.
fn q_min(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}

fn q_max(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}

/// `poseverts[i][j].v[k] * a->scale[k] + a->scale_origin[k]`
///
/// COMPAT: ADR-010 -- the byte widens to float and the multiply/add stay in
/// float, unfused (the C is built with `-ffp-contract=off`).
pub fn quake1_vert(v: [u8; 3], scale: [f32; 3], scale_origin: [f32; 3]) -> [f32; 3] {
    [
        f32::from(v[0]) * scale[0] + scale_origin[0],
        f32::from(v[1]) * scale[1] + scale_origin[1],
        f32::from(v[2]) * scale[2] + scale_origin[2],
    ]
}

/// `pv[j].xyz[k] * MD3_XYZ_SCALE`
pub fn quake3_vert(xyz: [i16; 3]) -> [f32; 3] {
    [
        f32::from(xyz[0]) * MD3_XYZ_SCALE,
        f32::from(xyz[1]) * MD3_XYZ_SCALE,
        f32::from(xyz[2]) * MD3_XYZ_SCALE,
    ]
}

/// `Mod_CalcAliasBounds` over an already-transformed vertex stream. All four
/// `poseverttype_t` branches of the C differ only in how they produce `v`.
///
/// An empty stream leaves `mins` at `FLT_MAX` and `maxs` at `-FLT_MAX` and
/// both radii at zero, exactly as the C does.
pub fn calc_bounds<I: IntoIterator<Item = [f32; 3]>>(verts: I) -> AliasBounds {
    let mut mins = [f32::MAX; 3];
    let mut maxs = [-f32::MAX; 3];
    let mut radius = 0.0f32;
    let mut yawradius = 0.0f32;

    for v in verts {
        for k in 0..3 {
            mins[k] = q_min(mins[k], v[k]);
            maxs[k] = q_max(maxs[k], v[k]);
        }
        let mut dist = v[0] * v[0] + v[1] * v[1];
        if yawradius < dist {
            yawradius = dist;
        }
        dist += v[2] * v[2];
        if radius < dist {
            radius = dist;
        }
    }

    let radius = radius.sqrt();
    let yawradius = yawradius.sqrt();

    AliasBounds {
        mins,
        maxs,
        ymins: [-yawradius, -yawradius, mins[2]],
        ymaxs: [yawradius, yawradius, maxs[2]],
        rmins: [-radius; 3],
        rmaxs: [radius; 3],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header_bytes() -> Vec<u8> {
        let mut b = vec![0u8; MDL_T_SIZE];
        b[OFS_IDENT..OFS_IDENT + 4].copy_from_slice(b"IDPO");
        b[OFS_VERSION..OFS_VERSION + 4].copy_from_slice(&6i32.to_le_bytes());
        for i in 0..3 {
            b[OFS_SCALE + i * 4..OFS_SCALE + i * 4 + 4]
                .copy_from_slice(&(1.0f32 + i as f32).to_le_bytes());
            b[OFS_SCALE_ORIGIN + i * 4..OFS_SCALE_ORIGIN + i * 4 + 4]
                .copy_from_slice(&(-2.0f32 * i as f32).to_le_bytes());
            b[OFS_EYEPOSITION + i * 4..OFS_EYEPOSITION + i * 4 + 4]
                .copy_from_slice(&(0.5f32 * i as f32).to_le_bytes());
        }
        b[OFS_BOUNDINGRADIUS..OFS_BOUNDINGRADIUS + 4].copy_from_slice(&7.5f32.to_le_bytes());
        b[OFS_NUMSKINS..OFS_NUMSKINS + 4].copy_from_slice(&1i32.to_le_bytes());
        b[OFS_SKINWIDTH..OFS_SKINWIDTH + 4].copy_from_slice(&4i32.to_le_bytes());
        b[OFS_SKINHEIGHT..OFS_SKINHEIGHT + 4].copy_from_slice(&8i32.to_le_bytes());
        b[OFS_NUMVERTS..OFS_NUMVERTS + 4].copy_from_slice(&3i32.to_le_bytes());
        b[OFS_NUMTRIS..OFS_NUMTRIS + 4].copy_from_slice(&1i32.to_le_bytes());
        b[OFS_NUMFRAMES..OFS_NUMFRAMES + 4].copy_from_slice(&2i32.to_le_bytes());
        b[OFS_SYNCTYPE..OFS_SYNCTYPE + 4].copy_from_slice(&1i32.to_le_bytes());
        b[OFS_FLAGS..OFS_FLAGS + 4].copy_from_slice(&0x18i32.to_le_bytes());
        b[OFS_SIZE..OFS_SIZE + 4].copy_from_slice(&11.0f32.to_le_bytes());
        b
    }

    #[test]
    fn header_fields_decode() {
        let h = parse_header(&header_bytes());
        assert_eq!(h.version, 6);
        assert_eq!(h.scale, [1.0, 2.0, 3.0]);
        assert_eq!(h.scale_origin, [-0.0, -2.0, -4.0]);
        assert_eq!(h.numskins, 1);
        assert_eq!(h.numverts, 3);
        assert_eq!(h.numtris, 1);
        assert_eq!(h.numframes, 2);
        assert_eq!(h.synctype, 1);
        assert_eq!(h.flags, 0x18);
        assert_eq!(h.size, 11.0);
        // the float 7.5 read back as a long
        assert_eq!(h.boundingradius_as_long, 7.5f32.to_bits() as i32);
    }

    #[test]
    fn size_scales_through_a_double() {
        assert_eq!(scaled_size(11.0), 1.0);
        // 0.1f * (1.0/11.0) in double, narrowed once
        let expect = (f64::from(0.1f32) * (1.0f64 / 11.0)) as f32;
        assert_eq!(scaled_size(0.1), expect);
    }

    #[test]
    fn validate_reports_in_c_order_and_stops_at_the_first_fatal() {
        let mut h = parse_header(&header_bytes());
        assert_eq!(validate(&h), vec![]);

        h.skinheight = MAX_LBM_HEIGHT + 1;
        h.numverts = MAXALIASVERTS_QS + 1;
        h.numtris = MAXALIASTRIS_QS + 1;
        assert_eq!(
            validate(&h),
            vec![
                Diag::SkinTooTall {
                    skinheight: MAX_LBM_HEIGHT + 1
                },
                Diag::VertsExceedQsLimit {
                    numverts: MAXALIASVERTS_QS + 1
                },
                Diag::TrisExceedQsLimit {
                    numtris: MAXALIASTRIS_QS + 1
                },
            ]
        );

        h.numverts = 0;
        let d = validate(&h);
        assert_eq!(d.last(), Some(&Diag::NoVertices));
        assert!(d.last().unwrap().is_fatal());

        h.numverts = MAXALIASVERTS + 1;
        assert!(matches!(
            validate(&h).last(),
            Some(Diag::TooManyVertices { .. })
        ));

        h.numverts = 3;
        h.numtris = 0;
        assert!(matches!(validate(&h).last(), Some(Diag::NoTriangles)));

        h.numtris = 1;
        h.numframes = 0;
        assert!(matches!(
            validate(&h).last(),
            Some(Diag::InvalidFrameCount { numframes: 0 })
        ));
    }

    #[test]
    fn records_decode() {
        let mut b = vec![0u8; DTRIANGLE_T_SIZE];
        b[0..4].copy_from_slice(&1i32.to_le_bytes());
        b[4..8].copy_from_slice(&7i32.to_le_bytes());
        b[8..12].copy_from_slice(&8i32.to_le_bytes());
        b[12..16].copy_from_slice(&9i32.to_le_bytes());
        assert_eq!(
            parse_triangle(&b),
            Triangle {
                facesfront: 1,
                vertindex: [7, 8, 9],
            }
        );

        let mut f = vec![0u8; DALIASFRAME_T_SIZE];
        f[0..4].copy_from_slice(&[1, 2, 3, 0xff]);
        f[4..8].copy_from_slice(&[4, 5, 6, 0xff]);
        f[8..12].copy_from_slice(b"cog1");
        let fh = parse_frame_header(&f);
        assert_eq!(fh.bboxmin, [1, 2, 3]);
        assert_eq!(fh.bboxmax, [4, 5, 6]);
        assert_eq!(&f[OFS_FRAME_NAME..OFS_FRAME_NAME + 4], b"cog1");

        let mut g = vec![0u8; DALIASGROUP_T_SIZE];
        g[0..4].copy_from_slice(&4i32.to_le_bytes());
        g[4..8].copy_from_slice(&[9, 8, 7, 0]);
        g[8..12].copy_from_slice(&[6, 5, 4, 0]);
        assert_eq!(
            parse_group_header(&g),
            GroupHeader {
                numframes: 4,
                bboxmin: [9, 8, 7],
                bboxmax: [6, 5, 4],
            }
        );
    }

    #[test]
    fn bounds_over_an_empty_stream_match_the_c_clear_state() {
        let b = calc_bounds(core::iter::empty());
        assert_eq!(b.mins, [f32::MAX; 3]);
        assert_eq!(b.maxs, [-f32::MAX; 3]);
        assert_eq!(b.rmins, [-0.0f32; 3]);
        assert_eq!(b.rmaxs, [0.0f32; 3]);
        assert_eq!(b.ymins, [-0.0, -0.0, f32::MAX]);
        assert_eq!(b.ymaxs, [0.0, 0.0, -f32::MAX]);
    }

    #[test]
    fn bounds_track_radius_and_yawradius() {
        let verts = [[3.0f32, 4.0, 12.0], [-1.0, 0.0, 0.0]];
        let b = calc_bounds(verts);
        assert_eq!(b.mins, [-1.0, 0.0, 0.0]);
        assert_eq!(b.maxs, [3.0, 4.0, 12.0]);
        // yawradius^2 = 9 + 16 = 25, radius^2 = 25 + 144 = 169
        assert_eq!(b.ymins, [-5.0, -5.0, 0.0]);
        assert_eq!(b.ymaxs, [5.0, 5.0, 12.0]);
        assert_eq!(b.rmins, [-13.0; 3]);
        assert_eq!(b.rmaxs, [13.0; 3]);
    }

    #[test]
    fn quake1_and_quake3_vertex_transforms() {
        assert_eq!(
            quake1_vert([2, 0, 255], [0.5, 1.0, 2.0], [1.0, -1.0, 0.0]),
            [2.0, -1.0, 510.0]
        );
        assert_eq!(quake3_vert([64, -64, 0]), [1.0, -1.0, 0.0]);
    }
}
