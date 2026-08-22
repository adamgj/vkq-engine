//! SPR (sprite model) parsing, ported from the sprite range of
//! `Quake/model_parse.c` (Rust migration Phase 3 M4).
//!
//! Pure layer, same contract as [`crate::mdl`]: the `quake-capi` shim owns
//! the `Mem_Alloc`ed `msprite_t`/`mspriteframe_t` tree, the walk over the
//! (unbounded, see [`crate::mdl`]) file image and the `TexMgr_LoadImage`
//! calls; everything decoded or computed from the file lives here.

// on-disk record sizes (`spritegn.h`)
pub const DSPRITE_T_SIZE: usize = 36;
pub const DSPRITEFRAME_T_SIZE: usize = 16;
pub const DSPRITEGROUP_T_SIZE: usize = 4;
pub const DSPRITEINTERVAL_T_SIZE: usize = 4;
pub const DSPRITEFRAMETYPE_T_SIZE: usize = 4;

// `offsetof (dsprite_t, ...)`
const OFS_IDENT: usize = 0;
pub const OFS_VERSION: usize = 4;
const OFS_TYPE: usize = 8;
const OFS_BOUNDINGRADIUS: usize = 12;
const OFS_WIDTH: usize = 16;
const OFS_HEIGHT: usize = 20;
pub const OFS_NUMFRAMES: usize = 24;
const OFS_BEAMLENGTH: usize = 28;
const OFS_SYNCTYPE: usize = 32;

pub const SPRITE_VERSION: i32 = 1;

/// `spriteframetype_t`
pub const SPR_SINGLE: i32 = 0;
pub const SPR_GROUP: i32 = 1;
pub const SPR_ANGLED: i32 = 2;

fn i32_at(b: &[u8], o: usize) -> i32 {
    i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn f32_at(b: &[u8], o: usize) -> f32 {
    f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// `dsprite_t` as `Mod_LoadSpriteModel` reads it
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpriteHeader {
    pub ident: i32,
    pub version: i32,
    pub type_: i32,
    pub boundingradius: f32,
    pub width: i32,
    pub height: i32,
    pub numframes: i32,
    pub beamlength: f32,
    /// `synctype_t`
    pub synctype: i32,
}

/// Panics if `b` is shorter than [`DSPRITE_T_SIZE`].
/// The two fields the C checks before it is entitled to the rest of the
/// header, for a caller holding an unbounded file image.
///
/// Panics if `b` is shorter than `OFS_VERSION + 4`.
pub fn parse_version(b: &[u8]) -> i32 {
    assert!(b.len() >= OFS_VERSION + 4);
    i32_at(b, OFS_VERSION)
}

/// Panics if `b` is shorter than `OFS_NUMFRAMES + 4`.
pub fn parse_numframes(b: &[u8]) -> i32 {
    assert!(b.len() >= OFS_NUMFRAMES + 4);
    i32_at(b, OFS_NUMFRAMES)
}

pub fn parse_header(b: &[u8]) -> SpriteHeader {
    assert!(b.len() >= DSPRITE_T_SIZE);
    SpriteHeader {
        ident: i32_at(b, OFS_IDENT),
        version: i32_at(b, OFS_VERSION),
        type_: i32_at(b, OFS_TYPE),
        boundingradius: f32_at(b, OFS_BOUNDINGRADIUS),
        width: i32_at(b, OFS_WIDTH),
        height: i32_at(b, OFS_HEIGHT),
        numframes: i32_at(b, OFS_NUMFRAMES),
        beamlength: f32_at(b, OFS_BEAMLENGTH),
        synctype: i32_at(b, OFS_SYNCTYPE),
    }
}

/// `qmodel_t` mins/maxs for a sprite.
///
/// COMPAT: the C is `mod->mins[0] = -psprite->maxwidth / 2;` -- unary minus
/// binds tighter than the division, so this is a *truncating integer*
/// negate-then-divide, and only the result becomes a float. An odd
/// `maxwidth` therefore gives an asymmetric box (5 -> -2 .. 2).
pub fn model_bounds(maxwidth: i32, maxheight: i32) -> ([f32; 3], [f32; 3]) {
    let minx = (maxwidth.wrapping_neg() / 2) as f32;
    let maxx = (maxwidth / 2) as f32;
    let minz = (maxheight.wrapping_neg() / 2) as f32;
    let maxz = (maxheight / 2) as f32;
    ([minx, minx, minz], [maxx, maxx, maxz])
}

/// The `mspriteframe_t` a `dspriteframe_t` produces, plus the pixel-data
/// size the walk advances by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameGeom {
    pub width: i32,
    pub height: i32,
    /// `width * height`, the number of `SRC_INDEXED` bytes that follow
    pub size: i32,
    pub up: f32,
    pub down: f32,
    pub left: f32,
    pub right: f32,
    pub smax: f32,
    pub tmax: f32,
}

/// Panics if `b` is shorter than [`DSPRITEFRAME_T_SIZE`].
pub fn parse_frame(b: &[u8]) -> FrameGeom {
    assert!(b.len() >= DSPRITEFRAME_T_SIZE);
    let origin = [i32_at(b, 0), i32_at(b, 4)];
    let width = i32_at(b, 8);
    let height = i32_at(b, 12);
    FrameGeom {
        width,
        height,
        size: width.wrapping_mul(height),
        up: origin[1] as f32,
        down: origin[1].wrapping_sub(height) as f32,
        left: origin[0] as f32,
        right: width.wrapping_add(origin[0]) as f32,
        smax: 1.0,
        tmax: 1.0,
    }
}

/// `dspritegroup_t::numframes` / `dspriteframetype_t::type`
pub fn parse_i32(b: &[u8]) -> i32 {
    assert!(b.len() >= 4);
    i32_at(b, 0)
}

/// `dspriteinterval_t::interval`
pub fn parse_interval(b: &[u8]) -> f32 {
    assert!(b.len() >= DSPRITEINTERVAL_T_SIZE);
    f32_at(b, 0)
}

/// C: `if (*poutintervals <= 0.0) Sys_Error ("Mod_LoadSpriteGroup: interval<=0");`
///
/// COMPAT: ADR-010 -- the comparison is against a **double** literal, so the
/// float is widened first. That is exact for every float, so the predicate
/// is the same either way; kept explicit to document the C.
pub fn interval_is_fatal(interval: f32) -> bool {
    f64::from(interval) <= 0.0
}

/// C: `if (type == SPR_ANGLED && numframes != 8)`
pub fn group_frame_count_is_fatal(frametype: i32, numframes: i32) -> bool {
    frametype == SPR_ANGLED && numframes != 8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_fields_decode() {
        let mut b = vec![0u8; DSPRITE_T_SIZE];
        b[OFS_IDENT..OFS_IDENT + 4].copy_from_slice(b"IDSP");
        b[OFS_VERSION..OFS_VERSION + 4].copy_from_slice(&1i32.to_le_bytes());
        b[OFS_TYPE..OFS_TYPE + 4].copy_from_slice(&2i32.to_le_bytes());
        b[OFS_BOUNDINGRADIUS..OFS_BOUNDINGRADIUS + 4].copy_from_slice(&3.5f32.to_le_bytes());
        b[OFS_WIDTH..OFS_WIDTH + 4].copy_from_slice(&16i32.to_le_bytes());
        b[OFS_HEIGHT..OFS_HEIGHT + 4].copy_from_slice(&24i32.to_le_bytes());
        b[OFS_NUMFRAMES..OFS_NUMFRAMES + 4].copy_from_slice(&3i32.to_le_bytes());
        b[OFS_BEAMLENGTH..OFS_BEAMLENGTH + 4].copy_from_slice(&10.0f32.to_le_bytes());
        b[OFS_SYNCTYPE..OFS_SYNCTYPE + 4].copy_from_slice(&1i32.to_le_bytes());

        let h = parse_header(&b);
        assert_eq!(h.version, SPRITE_VERSION);
        assert_eq!(h.type_, 2);
        assert_eq!(h.boundingradius, 3.5);
        assert_eq!(h.width, 16);
        assert_eq!(h.height, 24);
        assert_eq!(h.numframes, 3);
        assert_eq!(h.beamlength, 10.0);
        assert_eq!(h.synctype, 1);
    }

    #[test]
    fn bounds_use_truncating_integer_division() {
        assert_eq!(
            model_bounds(16, 24),
            ([-8.0, -8.0, -12.0], [8.0, 8.0, 12.0])
        );
        // odd sizes truncate toward zero on both ends, so the box is skewed
        assert_eq!(model_bounds(5, 7), ([-2.0, -2.0, -3.0], [2.0, 2.0, 3.0]));
    }

    #[test]
    fn frame_geometry_matches_the_c_arithmetic() {
        let mut b = vec![0u8; DSPRITEFRAME_T_SIZE];
        b[0..4].copy_from_slice(&(-3i32).to_le_bytes());
        b[4..8].copy_from_slice(&5i32.to_le_bytes());
        b[8..12].copy_from_slice(&6i32.to_le_bytes());
        b[12..16].copy_from_slice(&9i32.to_le_bytes());
        assert_eq!(
            parse_frame(&b),
            FrameGeom {
                width: 6,
                height: 9,
                size: 54,
                up: 5.0,
                down: -4.0,
                left: -3.0,
                right: 3.0,
                smax: 1.0,
                tmax: 1.0,
            }
        );
    }

    #[test]
    fn fatal_predicates() {
        assert!(interval_is_fatal(0.0));
        assert!(interval_is_fatal(-0.0));
        assert!(interval_is_fatal(-1.0));
        assert!(!interval_is_fatal(f32::MIN_POSITIVE));
        assert!(!interval_is_fatal(f32::NAN));

        assert!(group_frame_count_is_fatal(SPR_ANGLED, 7));
        assert!(!group_frame_count_is_fatal(SPR_ANGLED, 8));
        assert!(!group_frame_count_is_fatal(SPR_GROUP, 7));
    }
}
