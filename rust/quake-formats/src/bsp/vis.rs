//! Mod_DecompressVis. The `mod_decompressed` grow-only cache and its
//! `Mem_Realloc`/`Sys_Error` live in the capi shim; this is the per-call
//! decompression.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisStatus {
    Complete,
    /// C: `Con_Warning ("Mod_DecompressVis: output overrun on model
    /// \"%s\"\n")` once per model (`viswarn`), then early return
    Overrun,
    /// C reads past the end of visdata here (UB); we stop at the slice end
    /// with a partial row
    InputExhausted,
}

/// The row length: `(numleafs + 31) / 8`
pub fn vis_row(numleafs: i32) -> i32 {
    (numleafs + 31) / 8
}

/// Decompress one PVS row. `input` is the slice from the leaf's visofs to
/// the end of visdata; None means no vis info (all-visible fill). Returns
/// the bytes C wrote into `mod_decompressed` (exactly `row` on Complete).
pub fn decompress_vis(input: Option<&[u8]>, row: usize) -> (Vec<u8>, VisStatus) {
    let mut out = Vec::with_capacity(row);

    let Some(mut input) = input else {
        out.resize(row, 0xff);
        return (out, VisStatus::Complete);
    };

    while out.len() < row {
        let Some(&b) = input.first() else {
            return (out, VisStatus::InputExhausted);
        };
        if b != 0 {
            out.push(b);
            input = &input[1..];
            continue;
        }
        let Some(&run) = input.get(1) else {
            return (out, VisStatus::InputExhausted);
        };
        input = &input[2..];
        let mut c = i32::from(run);
        let remaining = (row - out.len()) as i32;
        if c > remaining {
            c = remaining;
        }
        while c > 0 {
            if out.len() == row {
                // unreachable after the clamp above, kept for C parity
                return (out, VisStatus::Overrun);
            }
            out.push(0);
            c -= 1;
        }
    }

    (out, VisStatus::Complete)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_length() {
        assert_eq!(vis_row(1), 4);
        assert_eq!(vis_row(32), 7);
        assert_eq!(vis_row(33), 8);
    }

    #[test]
    fn no_input_fills_all_visible() {
        assert_eq!(
            decompress_vis(None, 3),
            (vec![0xff; 3], VisStatus::Complete)
        );
    }

    #[test]
    fn literals_and_runs() {
        // 0xab, then a run of 3 zeros, then 0xcd
        let (out, st) = decompress_vis(Some(&[0xab, 0, 3, 0xcd]), 5);
        assert_eq!(out, vec![0xab, 0, 0, 0, 0xcd]);
        assert_eq!(st, VisStatus::Complete);
    }

    #[test]
    fn run_clamped_to_row() {
        let (out, st) = decompress_vis(Some(&[0xab, 0, 200]), 4);
        assert_eq!(out, vec![0xab, 0, 0, 0]);
        assert_eq!(st, VisStatus::Complete);
    }

    #[test]
    fn input_exhaustion_is_reported() {
        let (out, st) = decompress_vis(Some(&[0xab]), 3);
        assert_eq!(out, vec![0xab]);
        assert_eq!(st, VisStatus::InputExhausted);
        // dangling run marker
        let (out, st) = decompress_vis(Some(&[0xab, 0]), 3);
        assert_eq!(out, vec![0xab]);
        assert_eq!(st, VisStatus::InputExhausted);
    }
}
