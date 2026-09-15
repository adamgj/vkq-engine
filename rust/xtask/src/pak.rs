//! `Misc/vq_pak/mkpak.c`: the `vkquake.pak` PACK image from a table of
//! contents, one entry path per line, resolved under a root directory.

use std::fmt::Write as _;
use std::path::Path;

/// `dpackfile_t`: `char name[56]; int filepos, filelen`.
const ENTRY_SIZE: usize = 64;
const NAME_SIZE: usize = 56;
const HEADER_SIZE: usize = 12;

/// C `isspace` in the "C" locale, as `mkpak` trims the toc lines with.
fn is_c_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
}

/// The entry names in `toc` text, in order: one per `fgets` line (a final
/// line without a newline still counts), leading whitespace skipped and the
/// name cut at the first whitespace after it.
pub fn entries(toc: &[u8]) -> Vec<String> {
    toc.split_inclusive(|&b| b == b'\n')
        .map(|line| {
            let start = line
                .iter()
                .position(|&b| !is_c_space(b))
                .unwrap_or(line.len());
            let rest = &line[start..];
            let end = rest
                .iter()
                .position(|&b| is_c_space(b))
                .unwrap_or(rest.len());
            String::from_utf8_lossy(&rest[..end]).into_owned()
        })
        .collect()
}

/// Builds the PACK image for `entries` whose payloads `read` supplies.
pub fn build_with(
    entries: &[String],
    mut read: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<Vec<u8>, String> {
    let directory_size = entries.len() * ENTRY_SIZE;
    let mut out = Vec::new();
    out.extend_from_slice(b"PACK");
    out.extend_from_slice(&(HEADER_SIZE as i32).to_le_bytes());
    out.extend_from_slice(&(directory_size as i32).to_le_bytes());
    out.resize(HEADER_SIZE + directory_size, 0);
    for (index, name) in entries.iter().enumerate() {
        let data = read(name)?;
        if name.len() >= NAME_SIZE {
            return Err(format!(
                "Entry name '{name}' does not fit in the pak directory"
            ));
        }
        let entry = HEADER_SIZE + index * ENTRY_SIZE;
        let filepos = out.len() as i32;
        out[entry..entry + name.len()].copy_from_slice(name.as_bytes());
        out[entry + NAME_SIZE..entry + NAME_SIZE + 4].copy_from_slice(&filepos.to_le_bytes());
        out[entry + NAME_SIZE + 4..entry + ENTRY_SIZE]
            .copy_from_slice(&(data.len() as i32).to_le_bytes());
        out.extend_from_slice(&data);
    }
    Ok(out)
}

/// `mkpak output root toc [depfile]`: the toc's entries under `root`
/// (`root/entry`, as `mkpak` spells the paths). The depfile, when asked for,
/// lists the toc and every entry file as `output`'s dependencies.
pub fn build(output: &Path, root: &Path, toc: &Path, depfile: Option<&Path>) -> Result<(), String> {
    let toc_text = std::fs::read(toc)
        .map_err(|e| format!("Could not open toc_file file '{}': {e}", toc.display()))?;
    let names = entries(&toc_text);
    let mut deps = format!("{}: {}", output.display(), toc.display());
    let pak = build_with(&names, |name| {
        let path = format!("{}/{name}", root.display());
        let _ = write!(deps, " {path}");
        std::fs::read(&path).map_err(|e| format!("Could not open input file '{path}': {e}"))
    })?;
    std::fs::write(output, pak)
        .map_err(|e| format!("Could not open output file '{}': {e}", output.display()))?;
    if let Some(depfile) = depfile {
        deps.push('\n');
        std::fs::write(depfile, deps)
            .map_err(|e| format!("Could not open depfile '{}': {e}", depfile.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toc_lines_trim_like_fgets_plus_isspace() {
        assert_eq!(
            entries(b"gfx/a.lmp\r\n  maps/b.ent trailing\n\ndefault.cfg"),
            ["gfx/a.lmp", "maps/b.ent", "", "default.cfg"]
        );
        assert_eq!(entries(b"x\n"), ["x"]);
        assert!(entries(b"").is_empty());
    }

    #[test]
    fn layout_matches_mkpak() {
        let names = ["a".to_string(), "bb".to_string()];
        let pak = build_with(&names, |name| Ok(name.as_bytes().repeat(3))).unwrap();
        assert_eq!(&pak[..4], b"PACK");
        assert_eq!(i32::from_le_bytes(pak[4..8].try_into().unwrap()), 12);
        assert_eq!(i32::from_le_bytes(pak[8..12].try_into().unwrap()), 128);
        assert_eq!(&pak[12..13], b"a");
        assert_eq!(i32::from_le_bytes(pak[68..72].try_into().unwrap()), 140);
        assert_eq!(i32::from_le_bytes(pak[72..76].try_into().unwrap()), 3);
        assert_eq!(&pak[76..78], b"bb");
        assert_eq!(i32::from_le_bytes(pak[132..136].try_into().unwrap()), 143);
        assert_eq!(i32::from_le_bytes(pak[136..140].try_into().unwrap()), 6);
        assert_eq!(&pak[140..], b"aaabbbbbb");
        let long = ["n".repeat(56)];
        assert!(build_with(&long, |_| Ok(Vec::new())).is_err());
    }
}
