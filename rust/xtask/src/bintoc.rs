//! `Shaders/bintoc.c`: a binary file as a C `const unsigned char` array,
//! optionally raw-deflated with miniz's `TDEFL_MAX_PROBES_MASK` flags.

use std::path::Path;

use miniz_oxide::deflate::core::{compress, CompressorOxide, TDEFLFlush, TDEFLStatus};

/// `tdefl_compress_mem_to_heap(..., TDEFL_MAX_PROBES_MASK)`: no zlib header,
/// 4095 probes, non-greedy parsing. `miniz_oxide` derives the same probe
/// counts from the same flag bits as `tdefl_init`.
const TDEFL_MAX_PROBES_MASK: u32 = 0xFFF;

/// Raw-deflates `input` like `bintoc -c` does. Byte-identical to the vendored
/// miniz except for one heuristic: `miniz_oxide` never replaces a coded block
/// of 32 bytes or fewer with a stored block, miniz does whenever the coded
/// block is not smaller (quake-ctest's `xtask_differential` pins the
/// boundary; `scripts/harness/xtask_diff.py` proves `Misc/vq_pak` is not
/// affected, and either output inflates to the same bytes).
pub fn deflate_raw(input: &[u8]) -> Vec<u8> {
    let mut compressor = CompressorOxide::new(TDEFL_MAX_PROBES_MASK);
    let mut output = vec![0u8; (input.len() / 2).max(64)];
    let mut input = input;
    let mut out_pos = 0;
    loop {
        let (status, bytes_in, bytes_out) = compress(
            &mut compressor,
            input,
            &mut output[out_pos..],
            TDEFLFlush::Finish,
        );
        out_pos += bytes_out;
        match status {
            TDEFLStatus::Done => {
                output.truncate(out_pos);
                return output;
            }
            TDEFLStatus::Okay => {
                input = &input[bytes_in..];
                if output.len() - out_pos < 30 {
                    output.resize(output.len() * 2, 0);
                }
            }
            status => panic!("deflate failed: {status:?}"),
        }
    }
}

/// `bintoc` replaces every '.' in the symbol name with '_' (`basic.frag_spv`
/// becomes `basic_frag_spv`).
pub fn symbol(name: &str) -> String {
    name.replace('.', "_")
}

/// The C text `bintoc` writes for `bytes` under `symbol`; `decompressed` is
/// the original length when `bytes` are the `-c` deflate output.
pub fn render(symbol: &str, bytes: &[u8], decompressed: Option<usize>) -> String {
    let mut out = String::with_capacity(bytes.len() * 6 + 256);
    out.push_str("// clang-format off\n");
    out.push_str(&format!("const unsigned char {symbol}[] = {{\n"));
    for (i, b) in bytes.iter().enumerate() {
        out.push_str(&format!("0x{b:02X}, "));
        if (i + 1) % 10 == 0 {
            out.push('\n');
        }
    }
    out.push_str("};\n");
    out.push_str(&format!("const int {symbol}_size = {};\n", bytes.len()));
    if let Some(len) = decompressed {
        out.push_str(&format!("const int {symbol}_decompressed_size = {len};\n"));
    }
    out
}

/// `bintoc [-c] input symbol output`. The C tool opens the output in text
/// mode, so its file carries the platform newline; this always writes `\n`.
pub fn convert(input: &Path, name: &str, output: &Path, compress: bool) -> Result<(), String> {
    let data = std::fs::read(input).map_err(|e| format!("{}: {e}", input.display()))?;
    let symbol = symbol(name);
    let text = if compress {
        render(&symbol, &deflate_raw(&data), Some(data.len()))
    } else {
        render(&symbol, &data, None)
    };
    std::fs::write(output, text).map_err(|e| format!("{}: {e}", output.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_matches_bintoc_layout() {
        let bytes: Vec<u8> = (0u8..23).collect();
        let text = render("x_spv", &bytes, None);
        let expected = "// clang-format off\n\
const unsigned char x_spv[] = {\n\
0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, \n\
0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x11, 0x12, 0x13, \n\
0x14, 0x15, 0x16, };\n\
const int x_spv_size = 23;\n";
        assert_eq!(text, expected);
        assert_eq!(render("p", &[], Some(7)), "// clang-format off\nconst unsigned char p[] = {\n};\nconst int p_size = 0;\nconst int p_decompressed_size = 7;\n");
        assert_eq!(symbol("basic.frag_spv"), "basic_frag_spv");
    }

    #[test]
    fn deflate_round_trips() {
        let input: Vec<u8> = (0..100_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let packed = deflate_raw(&input);
        assert!(packed.len() < input.len());
        let unpacked = miniz_oxide::inflate::decompress_to_vec(&packed).unwrap();
        assert_eq!(unpacked, input);
        assert_eq!(
            miniz_oxide::inflate::decompress_to_vec(&deflate_raw(&[])).unwrap(),
            Vec::<u8>::new()
        );
    }
}
