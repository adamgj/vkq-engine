//! CalcSurfaceExtents.
//!
//! COMPAT: ADR-010 — the per-vertex projection is computed with f64
//! intermediates exactly like the C code's explicit `(double)` casts (which
//! exist to reproduce x87 light-compiler rounding), then rounded through f32
//! `val`/`mins`/`maxs`, and the results wrap through i16 stores before the
//! `> 2000` check.

/// C: `Sys_Error ("Bad surface extents")`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadExtents;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SurfaceExtents {
    pub texturemins: [i16; 2],
    pub extents: [i16; 2],
}

/// The surfedge -> vertex resolution of CalcSurfaceExtents. Out-of-range
/// indices are UB in C; here they yield None.
pub fn surface_points<'a>(
    surfedges: &'a [i32],
    edges: &'a [[u32; 2]],
    vertexes: &'a [[f32; 3]],
    firstedge: i32,
    numedges: i32,
) -> impl Iterator<Item = Option<[f32; 3]>> + 'a {
    (0..numedges.max(0)).map(move |i| {
        let e = *surfedges.get(usize::try_from(firstedge.checked_add(i)?).ok()?)?;
        let v = if e >= 0 {
            edges.get(usize::try_from(e).ok()?)?[0]
        } else {
            edges.get(usize::try_from(e.checked_neg()?).ok()?)?[1]
        };
        vertexes.get(usize::try_from(v).ok()?).copied()
    })
}

/// CalcSurfaceExtents over resolved points. `special` is
/// `tex->flags & TEX_SPECIAL`. The error is per-axis: axis 0 failing means C
/// died before writing axis 1 (irrelevant in practice — Sys_Error aborts).
pub fn calc_surface_extents(
    points: impl Iterator<Item = [f32; 3]>,
    vecs: &[[f32; 4]; 2],
    special: bool,
) -> Result<SurfaceExtents, BadExtents> {
    let mut mins = [f32::MAX; 2];
    let mut maxs = [f32::MIN; 2];

    // COMPAT: ADR-010 — vecs promoted to f64 once, per the C tex_vecs array
    let tex_vecs = [
        [
            f64::from(vecs[0][0]),
            f64::from(vecs[0][1]),
            f64::from(vecs[0][2]),
            f64::from(vecs[0][3]),
        ],
        [
            f64::from(vecs[1][0]),
            f64::from(vecs[1][1]),
            f64::from(vecs[1][2]),
            f64::from(vecs[1][3]),
        ],
    ];

    for p in points {
        for j in 0..2 {
            let val = (f64::from(p[0]) * tex_vecs[j][0]
                + f64::from(p[1]) * tex_vecs[j][1]
                + f64::from(p[2]) * tex_vecs[j][2]
                + tex_vecs[j][3]) as f32;
            // COMPAT: q_min_f/q_max_f are `(a < b) ? a : b`, so a NaN operand
            // propagates into the accumulator, where f32::min/f32::max would
            // silently discard it (Quake/q_minmax.h)
            mins[j] = if mins[j] < val { mins[j] } else { val };
            maxs[j] = if maxs[j] > val { maxs[j] } else { val };
        }
    }

    let mut out = SurfaceExtents::default();
    for i in 0..2 {
        // C: floor()/ceil() take the f32 quotient through double
        let bmins = f64::from(mins[i] / 16.0).floor() as i32;
        let bmaxs = f64::from(maxs[i] / 16.0).ceil() as i32;

        out.texturemins[i] = (bmins.wrapping_mul(16)) as i16;
        out.extents[i] = (bmaxs.wrapping_sub(bmins).wrapping_mul(16)) as i16;

        if !special && out.extents[i] > 2000 {
            return Err(BadExtents);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENT_VECS: [[f32; 4]; 2] = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]];

    #[test]
    fn quad_extents() {
        let pts = [
            [0.0, 0.0, 0.0],
            [64.0, 0.0, 0.0],
            [64.0, 32.0, 0.0],
            [0.0, 32.0, 0.0],
        ];
        let e = calc_surface_extents(pts.into_iter(), &IDENT_VECS, false).unwrap();
        assert_eq!(e.texturemins, [0, 0]);
        assert_eq!(e.extents, [64, 32]);
    }

    #[test]
    fn negative_mins_floor() {
        let pts = [[-1.0, -17.0, 0.0], [30.0, 15.0, 0.0]];
        let e = calc_surface_extents(pts.into_iter(), &IDENT_VECS, false).unwrap();
        // floor(-1/16) = -1 -> -16; ceil(30/16) = 2 -> extents 48
        assert_eq!(e.texturemins, [-16, -32]);
        assert_eq!(e.extents, [48, 48]);
    }

    #[test]
    fn special_skips_the_size_check() {
        let pts = [[0.0, 0.0, 0.0], [4096.0, 0.0, 0.0]];
        assert_eq!(
            calc_surface_extents(pts.into_iter(), &IDENT_VECS, false),
            Err(BadExtents)
        );
        let e = calc_surface_extents(pts.into_iter(), &IDENT_VECS, true).unwrap();
        assert_eq!(e.extents[0], 4096);
    }

    #[test]
    fn extents_wrap_through_i16_before_the_check() {
        // 65536-wide surface: (bmaxs-bmins)*16 = 65536 wraps to 0 as i16,
        // passing the > 2000 check exactly like C
        let pts = [[0.0, 0.0, 0.0], [65536.0, 0.0, 0.0]];
        let e = calc_surface_extents(pts.into_iter(), &IDENT_VECS, false).unwrap();
        assert_eq!(e.extents[0], 0);
    }

    #[test]
    fn nan_propagates_like_q_min_q_max() {
        // C: q_min/q_max are ternaries, so a NaN val is *stored* and the next
        // comparison then falls through to that operand. mins/maxs therefore
        // end up as the last point's value, not the true extrema.
        let pts = [[3.0, 0.0, 0.0], [f32::NAN, 0.0, 0.0], [1.0, 0.0, 0.0]];
        let e = calc_surface_extents(pts.into_iter(), &IDENT_VECS, false).unwrap();
        // mins: MAX -> 3 -> NaN -> 1 ; maxs: MIN -> 3 -> NaN -> 1
        assert_eq!(e.texturemins, [0, 0]);
        assert_eq!(e.extents, [16, 0]);
    }

    #[test]
    fn point_resolution_follows_edge_sign() {
        let vertexes = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]];
        let edges = [[0u32, 1], [1, 2]];
        let surfedges = [1i32, -1];
        let pts: Vec<_> = surface_points(&surfedges, &edges, &vertexes, 0, 2).collect();
        // +1 -> edges[1].v[0] = vertex 1; -1 -> edges[1].v[1] = vertex 2
        assert_eq!(pts, vec![Some([1.0, 0.0, 0.0]), Some([2.0, 0.0, 0.0])]);
        let pts: Vec<_> = surface_points(&surfedges, &edges, &vertexes, 1, 2).collect();
        assert_eq!(pts[1], None); // surfedge index 2 out of range
    }
}
