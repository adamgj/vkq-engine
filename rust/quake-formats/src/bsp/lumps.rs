//! Fixed-record lump parsers: vertexes, edges, surfedges, planes, texinfo,
//! faces, nodes, leafs, clipnodes, marksurfaces, submodels.

use super::{f32_at, i16_at, i32_at, u16_at, u32_at, Bsp2, FunnySize};

pub const DVERTEX_SIZE: usize = 12;
pub const DSEDGE_SIZE: usize = 4;
pub const DLEDGE_SIZE: usize = 8;
pub const DPLANE_SIZE: usize = 20;
pub const TEXINFO_SIZE: usize = 40;
pub const DSFACE_SIZE: usize = 20;
pub const DLFACE_SIZE: usize = 28;
pub const DSNODE_SIZE: usize = 24;
pub const DL1NODE_SIZE: usize = 32;
pub const DL2NODE_SIZE: usize = 44;
pub const DSLEAF_SIZE: usize = 28;
pub const DL1LEAF_SIZE: usize = 32;
pub const DL2LEAF_SIZE: usize = 44;
pub const DSCLIPNODE_SIZE: usize = 8;
pub const DLCLIPNODE_SIZE: usize = 12;
pub const DMODEL_SIZE: usize = 64;

pub const TEX_SPECIAL: i32 = 1;
pub const TEX_MISSING: i32 = 2;
pub const MAX_LIGHTSTYLES: i32 = 64;

/// Mod_LoadVertexes
pub fn parse_vertexes(lump: &[u8]) -> Result<Vec<[f32; 3]>, FunnySize> {
    if !lump.len().is_multiple_of(DVERTEX_SIZE) {
        return Err(FunnySize);
    }
    Ok(lump
        .chunks_exact(DVERTEX_SIZE)
        .map(|r| [f32_at(r, 0), f32_at(r, 4), f32_at(r, 8)])
        .collect())
}

/// Mod_LoadEdges. The shim allocates `count + 1` records like C.
pub fn parse_edges(lump: &[u8], bsp2: bool) -> Result<Vec<[u32; 2]>, FunnySize> {
    if bsp2 {
        if !lump.len().is_multiple_of(DLEDGE_SIZE) {
            return Err(FunnySize);
        }
        Ok(lump
            .chunks_exact(DLEDGE_SIZE)
            .map(|r| [u32_at(r, 0), u32_at(r, 4)])
            .collect())
    } else {
        if !lump.len().is_multiple_of(DSEDGE_SIZE) {
            return Err(FunnySize);
        }
        Ok(lump
            .chunks_exact(DSEDGE_SIZE)
            .map(|r| [u32::from(u16_at(r, 0)), u32::from(u16_at(r, 2))])
            .collect())
    }
}

/// Mod_LoadSurfedges
pub fn parse_surfedges(lump: &[u8]) -> Result<Vec<i32>, FunnySize> {
    if !lump.len().is_multiple_of(4) {
        return Err(FunnySize);
    }
    Ok(lump.chunks_exact(4).map(|r| i32_at(r, 0)).collect())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaneRec {
    pub normal: [f32; 3],
    pub dist: f32,
    /// C stores the int lump value into the byte `type` field (truncates)
    pub type_: u8,
    pub signbits: u8,
}

/// Mod_LoadPlanes. The shim allocates `count * 2` records like C.
pub fn parse_planes(lump: &[u8]) -> Result<Vec<PlaneRec>, FunnySize> {
    if !lump.len().is_multiple_of(DPLANE_SIZE) {
        return Err(FunnySize);
    }
    Ok(lump
        .chunks_exact(DPLANE_SIZE)
        .map(|r| {
            let normal = [f32_at(r, 0), f32_at(r, 4), f32_at(r, 8)];
            let mut bits = 0u8;
            for (j, n) in normal.iter().enumerate() {
                if *n < 0.0 {
                    bits |= 1 << j;
                }
            }
            PlaneRec {
                normal,
                dist: f32_at(r, 12),
                type_: i32_at(r, 16) as u8,
                signbits: bits,
            }
        })
        .collect())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TexInfoRec {
    pub vecs: [[f32; 4]; 2],
    pub miptex: i32,
    pub flags: i32,
}

/// Mod_LoadTexinfo, raw records
pub fn parse_texinfo(lump: &[u8]) -> Result<Vec<TexInfoRec>, FunnySize> {
    if !lump.len().is_multiple_of(TEXINFO_SIZE) {
        return Err(FunnySize);
    }
    Ok(lump
        .chunks_exact(TEXINFO_SIZE)
        .map(|r| TexInfoRec {
            vecs: [
                [f32_at(r, 0), f32_at(r, 4), f32_at(r, 8), f32_at(r, 12)],
                [f32_at(r, 16), f32_at(r, 20), f32_at(r, 24), f32_at(r, 28)],
            ],
            miptex: i32_at(r, 32),
            flags: i32_at(r, 36),
        })
        .collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TexInfoResolve {
    /// index into `mod->textures` the record points at
    pub texture_slot: i32,
    /// flags with TEX_MISSING OR'd in where applicable
    pub flags: i32,
    pub tex_idx: i32,
    pub missing: bool,
}

/// The texture-assignment logic of Mod_LoadTexinfo. `slot_present(i)`
/// reports whether `mod->textures[i]` is non-NULL for `0 <= i`.
///
/// A negative `miptex` indexes `mod->textures` out of bounds in C (UB);
/// this port treats it as a missing texture.
pub fn resolve_texinfo(
    miptex: i32,
    flags: i32,
    numtextures: i32,
    slot_present: impl Fn(i32) -> bool,
) -> TexInfoResolve {
    if miptex < 0 || miptex >= numtextures - 1 || !slot_present(miptex) {
        TexInfoResolve {
            texture_slot: if flags & TEX_SPECIAL != 0 {
                numtextures - 1
            } else {
                numtextures - 2
            },
            flags: flags | TEX_MISSING,
            tex_idx: -1,
            missing: true,
        }
    } else {
        TexInfoResolve {
            texture_slot: miptex,
            flags,
            tex_idx: miptex,
            missing: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaceRec {
    pub firstedge: i32,
    pub numedges: i32,
    pub planenum: i32,
    pub side: i32,
    pub texinfo: i32,
    pub styles: [u8; 4],
    /// Computed from zero. C ORs into a `Mem_AllocNonZero` field
    /// (uninitialized-memory bug); see the plan amendment log.
    pub styles_bitmap: u32,
    /// Style values that tripped `Con_Warning ("Invalid lightstyle %d\n")`,
    /// in emission order
    pub warned_styles: Vec<u8>,
    pub lightofs: i32,
}

/// Mod_ParseFaces, raw records (flag classification is [`classify_face`])
pub fn parse_faces(lump: &[u8], bsp2: bool) -> Result<Vec<FaceRec>, FunnySize> {
    let rec_size = if bsp2 { DLFACE_SIZE } else { DSFACE_SIZE };
    if !lump.len().is_multiple_of(rec_size) {
        return Err(FunnySize);
    }
    Ok(lump
        .chunks_exact(rec_size)
        .map(|r| {
            let (planenum, side, firstedge, numedges, texinfo, styles_ofs, lightofs) = if bsp2 {
                (
                    i32_at(r, 0),
                    i32_at(r, 4),
                    i32_at(r, 8),
                    i32_at(r, 12),
                    i32_at(r, 16),
                    20,
                    i32_at(r, 24),
                )
            } else {
                (
                    i32::from(i16_at(r, 0)),
                    i32::from(i16_at(r, 2)),
                    i32_at(r, 4),
                    i32::from(i16_at(r, 8)),
                    i32::from(i16_at(r, 10)),
                    12,
                    i32_at(r, 16),
                )
            };
            let mut styles = [0u8; 4];
            // Parity here depends on Mod_ParseFaces zeroing styles_bitmap
            // before its OR loop (model_parse.c, the M7 RA12 fix in
            // docs/ai/plans/rust-conversion-phase-3.md) — reverting that C
            // line reopens the nondeterministic-oracle divergence this site
            // used to carry a COMPAT note for.
            let mut styles_bitmap = 0u32;
            let mut warned_styles = Vec::new();
            for i in 0..4 {
                let mut s = r[styles_ofs + i];
                if i32::from(s) >= MAX_LIGHTSTYLES && s != 255 {
                    warned_styles.push(s);
                    s = 0;
                }
                styles[i] = s;
                if s < 255 {
                    let j = u32::from(s);
                    styles_bitmap |= 1 << (if j < 16 { j } else { j % 16 + 16 });
                }
            }
            if styles_bitmap == 0 {
                styles_bitmap = 1;
            }
            FaceRec {
                firstedge,
                numedges,
                planenum,
                side,
                texinfo,
                styles,
                styles_bitmap,
                warned_styles,
                lightofs,
            }
        })
        .collect())
}

/// The lighting-offset transform of Mod_ParseFaces: byte offset into
/// `mod->lightdata`, or None for no samples. COMPAT: the Q64 halving runs
/// before the -1 check, exactly like C (`lofs = -1` becomes offset 0).
pub fn face_samples_offset(lofs: i32, q64: bool, valve: bool) -> Option<i64> {
    let mut lofs = lofs;
    if q64 {
        lofs /= 2;
    }
    if lofs == -1 {
        None
    } else if valve {
        Some(i64::from(lofs))
    } else {
        Some(i64::from(lofs) * 3)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaceClassify {
    /// flags to OR onto SURF_PLANEBACK (already applied by the shim)
    pub flags: i32,
    pub lightmaptexturenum: i32,
    /// Con_Warning ("Mod_LoadFaces: TEX_MISSING without TEX_SPECIAL missing lightmap samples")
    pub warn_missing_samples: bool,
}

pub const SURF_DRAWSKY: i32 = 4;
pub const SURF_DRAWTURB: i32 = 0x10;
pub const SURF_DRAWTILED: i32 = 0x20;
pub const SURF_NOTEXTURE: i32 = 0x100;
pub const SURF_DRAWFENCE: i32 = 0x200;
pub const SURF_DRAWLAVA: i32 = 0x400;
pub const SURF_DRAWSLIME: i32 = 0x800;
pub const SURF_DRAWTELE: i32 = 0x1000;
pub const SURF_DRAWWATER: i32 = 0x2000;

pub const TEXTYPE_CUTOUT: i32 = 1;
pub const TEXTYPE_SKY: i32 = 2;
pub const TEXTYPE_LAVA: i32 = 3;
pub const TEXTYPE_SLIME: i32 = 4;
pub const TEXTYPE_TELE: i32 = 5;
pub const TEXTYPE_WATER: i32 = 6;

fn textype_is_liquid(t: i32) -> bool {
    (t.wrapping_sub(TEXTYPE_LAVA) as u32) < 4
}

/// The flag/lightmap classification tail of Mod_ParseFaces
pub fn classify_face(
    textype: i32,
    texinfo_flags: i32,
    has_samples: bool,
    style0: u8,
) -> FaceClassify {
    let mut flags = 0;
    let mut lightmaptexturenum = -1;
    let mut warn_missing_samples = false;

    if textype == TEXTYPE_SKY {
        flags |= SURF_DRAWSKY | SURF_DRAWTILED;
    } else if textype_is_liquid(textype) {
        flags |= SURF_DRAWTURB;
        if texinfo_flags & TEX_SPECIAL != 0 {
            flags |= SURF_DRAWTILED;
        }
        if textype == TEXTYPE_LAVA {
            flags |= SURF_DRAWLAVA;
        } else if textype == TEXTYPE_SLIME {
            flags |= SURF_DRAWSLIME;
        } else if textype == TEXTYPE_TELE {
            flags |= SURF_DRAWTELE;
        } else {
            flags |= SURF_DRAWWATER;
        }
    } else if textype == TEXTYPE_CUTOUT {
        flags |= SURF_DRAWFENCE;
    } else if texinfo_flags & TEX_MISSING != 0 {
        flags |= SURF_NOTEXTURE;
        let missing_samples = !has_samples && style0 != 255;
        let unlit_texture = texinfo_flags & TEX_SPECIAL != 0;
        if !unlit_texture && missing_samples {
            warn_missing_samples = true;
            lightmaptexturenum = 0;
        }
        if unlit_texture || missing_samples {
            flags |= SURF_DRAWTILED;
        }
    }

    FaceClassify {
        flags,
        lightmaptexturenum,
        warn_missing_samples,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeChild {
    Node(i32),
    Leaf(i32),
    /// Con_Printf ("Mod_LoadNodes: invalid leaf index %i (file has only %i
    /// leafs)\n") then leaf 0
    InvalidLeaf(i32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeRec {
    pub minmaxs: [f32; 6],
    pub planenum: i32,
    pub firstsurface: u32,
    pub numsurfaces: u32,
    pub children: [NodeChild; 2],
}

/// Mod_LoadNodes (all three dialects)
pub fn parse_nodes(lump: &[u8], dialect: Bsp2, numleafs: i32) -> Result<Vec<NodeRec>, FunnySize> {
    let rec_size = match dialect {
        Bsp2::No => DSNODE_SIZE,
        Bsp2::L1 => DL1NODE_SIZE,
        Bsp2::L2 => DL2NODE_SIZE,
    };
    if !lump.len().is_multiple_of(rec_size) {
        return Err(FunnySize);
    }
    let count = (lump.len() / rec_size) as i32;
    Ok(lump
        .chunks_exact(rec_size)
        .map(|r| {
            let mut minmaxs = [0f32; 6];
            match dialect {
                Bsp2::No => {
                    for j in 0..3 {
                        minmaxs[j] = f32::from(i16_at(r, 8 + 2 * j));
                        minmaxs[3 + j] = f32::from(i16_at(r, 14 + 2 * j));
                    }
                }
                Bsp2::L1 => {
                    for j in 0..3 {
                        minmaxs[j] = f32::from(i16_at(r, 12 + 2 * j));
                        minmaxs[3 + j] = f32::from(i16_at(r, 18 + 2 * j));
                    }
                }
                Bsp2::L2 => {
                    for j in 0..3 {
                        minmaxs[j] = f32_at(r, 12 + 4 * j);
                        minmaxs[3 + j] = f32_at(r, 24 + 4 * j);
                    }
                }
            }
            let planenum = i32_at(r, 0);
            let (firstsurface, numsurfaces) = match dialect {
                Bsp2::No => (u32::from(u16_at(r, 20)), u32::from(u16_at(r, 22))),
                Bsp2::L1 => (u32_at(r, 24), u32_at(r, 28)),
                Bsp2::L2 => (u32_at(r, 36), u32_at(r, 40)),
            };
            let mut children = [NodeChild::Node(0); 2];
            for (j, child) in children.iter_mut().enumerate() {
                *child = match dialect {
                    Bsp2::No => {
                        let p = i32::from(u16_at(r, 4 + 2 * j));
                        if p < count {
                            NodeChild::Node(p)
                        } else {
                            let p = 65535 - p; // -1 is leaf 0
                            if p < numleafs {
                                NodeChild::Leaf(p)
                            } else {
                                NodeChild::InvalidLeaf(p)
                            }
                        }
                    }
                    Bsp2::L1 => {
                        let p = i32_at(r, 4 + 4 * j);
                        if p >= 0 && p < count {
                            NodeChild::Node(p)
                        } else {
                            let p = 0xffff_ffffu32.wrapping_sub(p as u32) as i32;
                            if p >= 0 && p < numleafs {
                                NodeChild::Leaf(p)
                            } else {
                                NodeChild::InvalidLeaf(p)
                            }
                        }
                    }
                    Bsp2::L2 => {
                        let p = i32_at(r, 4 + 4 * j);
                        // COMPAT: strictly greater — node 0 as a child falls
                        // through to the leaf path, exactly like C
                        if p > 0 && p < count {
                            NodeChild::Node(p)
                        } else {
                            let p = 0xffff_ffffu32.wrapping_sub(p as u32) as i32;
                            if p >= 0 && p < numleafs {
                                NodeChild::Leaf(p)
                            } else {
                                NodeChild::InvalidLeaf(p)
                            }
                        }
                    }
                };
            }
            NodeRec {
                minmaxs,
                planenum,
                firstsurface,
                numsurfaces,
                children,
            }
        })
        .collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeafError {
    /// Sys_Error ("Mod_ProcessLeafs: funny lump size in %s")
    FunnySize,
    /// Host_Error ("Mod_LoadLeafs: %i leafs exceeds limit of 32767.") —
    /// BSP29 dialect only
    TooMany(i32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LeafRec {
    pub minmaxs: [f32; 6],
    pub contents: i32,
    pub firstmarksurface: i32,
    pub nummarksurfaces: i32,
    pub visofs: i32,
    pub ambient: [u8; 4],
}

/// Mod_ProcessLeafs_{S,L1,L2} record parsing
pub fn parse_leafs(buf: &[u8], dialect: Bsp2) -> Result<Vec<LeafRec>, LeafError> {
    let rec_size = match dialect {
        Bsp2::No => DSLEAF_SIZE,
        Bsp2::L1 => DL1LEAF_SIZE,
        Bsp2::L2 => DL2LEAF_SIZE,
    };
    if !buf.len().is_multiple_of(rec_size) {
        return Err(LeafError::FunnySize);
    }
    let count = (buf.len() / rec_size) as i32;
    if dialect == Bsp2::No && count > 32767 {
        return Err(LeafError::TooMany(count));
    }
    Ok(buf
        .chunks_exact(rec_size)
        .map(|r| {
            let mut minmaxs = [0f32; 6];
            match dialect {
                Bsp2::No | Bsp2::L1 => {
                    for j in 0..3 {
                        minmaxs[j] = f32::from(i16_at(r, 8 + 2 * j));
                        minmaxs[3 + j] = f32::from(i16_at(r, 14 + 2 * j));
                    }
                }
                Bsp2::L2 => {
                    for j in 0..3 {
                        minmaxs[j] = f32_at(r, 8 + 4 * j);
                        minmaxs[3 + j] = f32_at(r, 20 + 4 * j);
                    }
                }
            }
            let (firstmarksurface, nummarksurfaces, ambient_ofs) = match dialect {
                Bsp2::No => (i32::from(u16_at(r, 20)), i32::from(u16_at(r, 22)), 24),
                Bsp2::L1 => (i32_at(r, 20), i32_at(r, 24), 28),
                Bsp2::L2 => (i32_at(r, 32), i32_at(r, 36), 40),
            };
            LeafRec {
                minmaxs,
                contents: i32_at(r, 0),
                firstmarksurface,
                nummarksurfaces,
                visofs: i32_at(r, 4),
                ambient: [
                    r[ambient_ofs],
                    r[ambient_ofs + 1],
                    r[ambient_ofs + 2],
                    r[ambient_ofs + 3],
                ],
            }
        })
        .collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipnodeRec {
    pub planenum: i32,
    pub children: [i32; 2],
}

/// Mod_LoadClipnodes record parsing. The planenum bounds check
/// (Host_Error) is replayed by the shim during the write loop so partial
/// state matches C.
pub fn parse_clipnodes(lump: &[u8], bsp2: bool) -> Result<Vec<ClipnodeRec>, FunnySize> {
    let rec_size = if bsp2 {
        DLCLIPNODE_SIZE
    } else {
        DSCLIPNODE_SIZE
    };
    if !lump.len().is_multiple_of(rec_size) {
        return Err(FunnySize);
    }
    let count = (lump.len() / rec_size) as i32;
    Ok(lump
        .chunks_exact(rec_size)
        .map(|r| {
            if bsp2 {
                ClipnodeRec {
                    planenum: i32_at(r, 0),
                    children: [i32_at(r, 4), i32_at(r, 8)],
                }
            } else {
                let mut children = [i32::from(u16_at(r, 4)), i32::from(u16_at(r, 6))];
                for c in &mut children {
                    if *c >= count {
                        *c -= 65536;
                    }
                }
                ClipnodeRec {
                    planenum: i32_at(r, 0),
                    children,
                }
            }
        })
        .collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkError {
    /// Host_Error ("Mod_LoadMarksurfaces: funny lump size in %s") — both
    /// dialects
    FunnySize,
    /// "Mod_LoadMarksurfaces: bad surface number" — Host_Error for BSP2,
    /// Sys_Error for BSP29
    BadSurface,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkResult {
    /// Validated prefix — on BadSurface, the entries C wrote before the
    /// fatal call
    pub entries: Vec<i32>,
    pub error: Option<MarkError>,
}

/// Mod_LoadMarksurfaces
pub fn parse_marksurfaces(lump: &[u8], bsp2: bool, numsurfaces: i32) -> MarkResult {
    let rec_size = if bsp2 { 4 } else { 2 };
    if !lump.len().is_multiple_of(rec_size) {
        return MarkResult {
            entries: Vec::new(),
            error: Some(MarkError::FunnySize),
        };
    }
    let mut entries = Vec::with_capacity(lump.len() / rec_size);
    for r in lump.chunks_exact(rec_size) {
        let j = if bsp2 {
            i32_at(r, 0)
        } else {
            i32::from(u16_at(r, 0))
        };
        if j >= numsurfaces {
            return MarkResult {
                entries,
                error: Some(MarkError::BadSurface),
            };
        }
        entries.push(j);
    }
    MarkResult {
        entries,
        error: None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SubmodelRec {
    /// spread by a pixel: file mins - 1
    pub mins: [f32; 3],
    /// spread by a pixel: file maxs + 1
    pub maxs: [f32; 3],
    pub origin: [f32; 3],
    pub headnode: [i32; 4],
    pub visleafs: i32,
    pub firstface: i32,
    pub numfaces: i32,
}

/// Mod_LoadSubmodels. The world-visleafs warning (`visleafs > 8192` on
/// record 0) is emitted by the shim.
pub fn parse_submodels(lump: &[u8]) -> Result<Vec<SubmodelRec>, FunnySize> {
    if !lump.len().is_multiple_of(DMODEL_SIZE) {
        return Err(FunnySize);
    }
    Ok(lump
        .chunks_exact(DMODEL_SIZE)
        .map(|r| {
            let mut mins = [0f32; 3];
            let mut maxs = [0f32; 3];
            let mut origin = [0f32; 3];
            for j in 0..3 {
                mins[j] = f32_at(r, 4 * j) - 1.0;
                maxs[j] = f32_at(r, 12 + 4 * j) + 1.0;
                origin[j] = f32_at(r, 24 + 4 * j);
            }
            let mut headnode = [0i32; 4];
            for (j, h) in headnode.iter_mut().enumerate() {
                *h = i32_at(r, 36 + 4 * j);
            }
            SubmodelRec {
                mins,
                maxs,
                origin,
                headnode,
                visleafs: i32_at(r, 52),
                firstface: i32_at(r, 56),
                numfaces: i32_at(r, 60),
            }
        })
        .collect())
}

/// RadiusFromBounds: |corner| via float dot + double sqrt, exactly like
/// the C expression tree (no FMA per ADR-010's -ffp-contract=off).
pub fn radius_from_bounds(mins: &[f32; 3], maxs: &[f32; 3]) -> f32 {
    let mut corner = [0f32; 3];
    for i in 0..3 {
        corner[i] = if mins[i].abs() > maxs[i].abs() {
            mins[i].abs()
        } else {
            maxs[i].abs()
        };
    }
    let dot = corner[0] * corner[0] + corner[1] * corner[1] + corner[2] * corner[2];
    f64::from(dot).sqrt() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertexes_roundtrip_and_funny_size() {
        let mut lump = Vec::new();
        for f in [1.0f32, -2.5, 3.25, 0.0, 4.0, -8.0] {
            lump.extend_from_slice(&f.to_le_bytes());
        }
        let v = parse_vertexes(&lump).unwrap();
        assert_eq!(v, vec![[1.0, -2.5, 3.25], [0.0, 4.0, -8.0]]);
        assert_eq!(parse_vertexes(&lump[..13]), Err(FunnySize));
    }

    #[test]
    fn edges_both_dialects() {
        let mut s = Vec::new();
        s.extend_from_slice(&5u16.to_le_bytes());
        s.extend_from_slice(&65535u16.to_le_bytes());
        assert_eq!(parse_edges(&s, false).unwrap(), vec![[5, 65535]]);

        let mut l = Vec::new();
        l.extend_from_slice(&(-1i32).to_le_bytes()); // wraps like C int->unsigned
        l.extend_from_slice(&70000i32.to_le_bytes());
        assert_eq!(parse_edges(&l, true).unwrap(), vec![[0xffff_ffff, 70000]]);
    }

    #[test]
    fn planes_signbits_and_type_truncation() {
        let mut lump = Vec::new();
        for f in [-0.5f32, 0.0, 0.75] {
            lump.extend_from_slice(&f.to_le_bytes());
        }
        lump.extend_from_slice(&12.5f32.to_le_bytes());
        lump.extend_from_slice(&0x1_02i32.to_le_bytes()); // type 258 -> byte 2
        let p = parse_planes(&lump).unwrap();
        assert_eq!(p[0].signbits, 1); // only normal[0] < 0 (-0.0 is not < 0)
        assert_eq!(p[0].type_, 2);
    }

    #[test]
    fn texinfo_resolution() {
        // numtextures = 5 (3 real + 2 dummies), slot 1 is NULL
        let present = |i: i32| i != 1;
        let r = resolve_texinfo(0, 0, 5, present);
        assert_eq!((r.texture_slot, r.tex_idx, r.missing), (0, 0, false));
        // NULL slot -> non-special dummy
        let r = resolve_texinfo(1, 0, 5, present);
        assert_eq!((r.texture_slot, r.tex_idx), (3, -1));
        assert_eq!(r.flags, TEX_MISSING);
        // out of range + special -> special dummy
        let r = resolve_texinfo(4, TEX_SPECIAL, 5, present);
        assert_eq!(r.texture_slot, 4);
        assert_eq!(r.flags, TEX_SPECIAL | TEX_MISSING);
    }

    fn dsface(
        planenum: i16,
        side: i16,
        firstedge: i32,
        numedges: i16,
        texinfo: i16,
        styles: [u8; 4],
        lightofs: i32,
    ) -> Vec<u8> {
        let mut r = Vec::new();
        r.extend_from_slice(&planenum.to_le_bytes());
        r.extend_from_slice(&side.to_le_bytes());
        r.extend_from_slice(&firstedge.to_le_bytes());
        r.extend_from_slice(&numedges.to_le_bytes());
        r.extend_from_slice(&texinfo.to_le_bytes());
        r.extend_from_slice(&styles);
        r.extend_from_slice(&lightofs.to_le_bytes());
        r
    }

    #[test]
    fn faces_styles_fixup_and_bitmap() {
        let lump = dsface(1, 1, 4, 4, 0, [0, 70, 255, 17], 12);
        let f = &parse_faces(&lump, false).unwrap()[0];
        assert_eq!(f.styles, [0, 0, 255, 17]); // 70 is invalid -> 0
        assert_eq!(f.warned_styles, vec![70]);
        // style 0 -> bit 0, style 17 -> bit 17 % 16 + 16 = 17
        assert_eq!(f.styles_bitmap, (1 << 0) | (1 << 17));
        assert_eq!((f.planenum, f.side, f.lightofs), (1, 1, 12));

        // all styles 255 -> empty bitmap forced to 1
        let lump = dsface(0, 0, 0, 0, 0, [255; 4], -1);
        assert_eq!(parse_faces(&lump, false).unwrap()[0].styles_bitmap, 1);
    }

    #[test]
    fn samples_offset_transforms() {
        assert_eq!(face_samples_offset(-1, false, false), None);
        assert_eq!(face_samples_offset(6, false, false), Some(18));
        assert_eq!(face_samples_offset(6, false, true), Some(6));
        // COMPAT: Q64 halves before the -1 check
        assert_eq!(face_samples_offset(-1, true, false), Some(0));
        assert_eq!(face_samples_offset(6, true, false), Some(9));
    }

    #[test]
    fn face_classification() {
        let c = classify_face(TEXTYPE_SKY, 0, true, 0);
        assert_eq!(c.flags, SURF_DRAWSKY | SURF_DRAWTILED);
        let c = classify_face(TEXTYPE_LAVA, TEX_SPECIAL, true, 0);
        assert_eq!(c.flags, SURF_DRAWTURB | SURF_DRAWTILED | SURF_DRAWLAVA);
        let c = classify_face(TEXTYPE_CUTOUT, 0, true, 0);
        assert_eq!(c.flags, SURF_DRAWFENCE);
        // missing texture, lit, no samples -> warning + lightmap 0
        let c = classify_face(0, TEX_MISSING, false, 0);
        assert!(c.warn_missing_samples);
        assert_eq!(c.lightmaptexturenum, 0);
        assert_eq!(c.flags, SURF_NOTEXTURE | SURF_DRAWTILED);
        // missing texture with samples -> no tiling
        let c = classify_face(0, TEX_MISSING, true, 0);
        assert_eq!(c.flags, SURF_NOTEXTURE);
        assert_eq!(c.lightmaptexturenum, -1);
    }

    fn dsnode(planenum: i32, children: [i16; 2]) -> Vec<u8> {
        let mut r = Vec::new();
        r.extend_from_slice(&planenum.to_le_bytes());
        r.extend_from_slice(&children[0].to_le_bytes());
        r.extend_from_slice(&children[1].to_le_bytes());
        for v in [-8i16, -8, -8, 8, 8, 8] {
            r.extend_from_slice(&v.to_le_bytes());
        }
        r.extend_from_slice(&3u16.to_le_bytes());
        r.extend_from_slice(&2u16.to_le_bytes());
        r
    }

    #[test]
    fn nodes_s_child_resolution() {
        let mut lump = dsnode(0, [1, -1]); // node 1, leaf 0
        lump.extend_from_slice(&dsnode(1, [-3, -9])); // leaf 2, invalid (numleafs 4 -> 65535-65527=8)
        let n = parse_nodes(&lump, Bsp2::No, 4).unwrap();
        assert_eq!(n[0].children, [NodeChild::Node(1), NodeChild::Leaf(0)]);
        assert_eq!(n[0].minmaxs, [-8.0, -8.0, -8.0, 8.0, 8.0, 8.0]);
        assert_eq!((n[0].firstsurface, n[0].numsurfaces), (3, 2));
        assert_eq!(
            n[1].children,
            [NodeChild::Leaf(2), NodeChild::InvalidLeaf(8)]
        );
    }

    fn dl2node(planenum: i32, children: [i32; 2]) -> Vec<u8> {
        let mut r = Vec::new();
        r.extend_from_slice(&planenum.to_le_bytes());
        r.extend_from_slice(&children[0].to_le_bytes());
        r.extend_from_slice(&children[1].to_le_bytes());
        for v in [-8f32, -8.0, -8.0, 8.0, 8.0, 8.0] {
            r.extend_from_slice(&v.to_le_bytes());
        }
        r.extend_from_slice(&3u32.to_le_bytes());
        r.extend_from_slice(&2u32.to_le_bytes());
        r
    }

    #[test]
    fn nodes_l2_zero_child_is_leaf_path() {
        // COMPAT: L2 uses p > 0, so a 0 child resolves through the leaf path
        // to 0xffffffff - 0 = -1 -> invalid -> solid leaf
        let mut lump = dl2node(0, [0, -1]);
        lump.extend_from_slice(&dl2node(0, [1, -2]));
        let n = parse_nodes(&lump, Bsp2::L2, 4).unwrap();
        assert_eq!(n[0].children[0], NodeChild::InvalidLeaf(-1));
        assert_eq!(n[0].children[1], NodeChild::Leaf(0));
        assert_eq!(n[1].children, [NodeChild::Node(1), NodeChild::Leaf(1)]);
    }

    fn dsleaf(contents: i32, visofs: i32, first: u16, num: u16, ambient: [u8; 4]) -> Vec<u8> {
        let mut r = Vec::new();
        r.extend_from_slice(&contents.to_le_bytes());
        r.extend_from_slice(&visofs.to_le_bytes());
        for v in [-16i16, -16, -16, 16, 16, 16] {
            r.extend_from_slice(&v.to_le_bytes());
        }
        r.extend_from_slice(&first.to_le_bytes());
        r.extend_from_slice(&num.to_le_bytes());
        r.extend_from_slice(&ambient);
        r
    }

    #[test]
    fn leafs_s_parse_and_limit() {
        let lump = dsleaf(-2, 4, 1, 2, [10, 20, 30, 40]);
        let l = parse_leafs(&lump, Bsp2::No).unwrap();
        assert_eq!(l[0].contents, -2);
        assert_eq!(l[0].visofs, 4);
        assert_eq!((l[0].firstmarksurface, l[0].nummarksurfaces), (1, 2));
        assert_eq!(l[0].ambient, [10, 20, 30, 40]);

        let big = vec![0u8; DSLEAF_SIZE * 32768];
        assert_eq!(parse_leafs(&big, Bsp2::No), Err(LeafError::TooMany(32768)));
        // L1 has no limit
        let big = vec![0u8; DL1LEAF_SIZE * 32768];
        assert!(parse_leafs(&big, Bsp2::L1).is_ok());
    }

    #[test]
    fn clipnodes_short_children_wrap() {
        // count = 2; child 65535 -> 65535 - 65536 = -1 (contents)
        let mut lump = Vec::new();
        lump.extend_from_slice(&0i32.to_le_bytes());
        lump.extend_from_slice(&1u16.to_le_bytes());
        lump.extend_from_slice(&65535u16.to_le_bytes());
        lump.extend_from_slice(&1i32.to_le_bytes());
        lump.extend_from_slice(&2u16.to_le_bytes()); // >= count -> -65534
        lump.extend_from_slice(&0u16.to_le_bytes());
        let c = parse_clipnodes(&lump, false).unwrap();
        assert_eq!(c[0].children, [1, -1]);
        assert_eq!(c[1].children, [2 - 65536, 0]);
    }

    #[test]
    fn marksurfaces_validated_prefix() {
        let mut lump = Vec::new();
        for v in [0u16, 2, 9, 1] {
            lump.extend_from_slice(&v.to_le_bytes());
        }
        let r = parse_marksurfaces(&lump, false, 3);
        assert_eq!(r.entries, vec![0, 2]);
        assert_eq!(r.error, Some(MarkError::BadSurface));

        let r = parse_marksurfaces(&lump[..6], false, 10);
        assert_eq!(r.entries, vec![0, 2, 9]);
        assert_eq!(r.error, None);

        let r = parse_marksurfaces(&lump[..3], false, 10);
        assert_eq!(r.error, Some(MarkError::FunnySize));
    }

    #[test]
    fn submodels_spread_bounds() {
        let mut r = Vec::new();
        for v in [0f32, 0.0, 0.0, 32.0, 32.0, 32.0, 1.0, 2.0, 3.0] {
            r.extend_from_slice(&v.to_le_bytes());
        }
        for v in [7i32, 8, 9, 10, 5, 0, 6] {
            r.extend_from_slice(&v.to_le_bytes());
        }
        let s = parse_submodels(&r).unwrap();
        assert_eq!(s[0].mins, [-1.0, -1.0, -1.0]);
        assert_eq!(s[0].maxs, [33.0, 33.0, 33.0]);
        assert_eq!(s[0].origin, [1.0, 2.0, 3.0]);
        assert_eq!(s[0].headnode, [7, 8, 9, 10]);
        assert_eq!((s[0].visleafs, s[0].firstface, s[0].numfaces), (5, 0, 6));
    }

    #[test]
    fn radius_matches_reference() {
        let r = radius_from_bounds(&[-3.0, -4.0, 0.0], &[1.0, 2.0, 0.0]);
        assert_eq!(r, 5.0);
    }
}
