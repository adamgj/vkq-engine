//! Pure line-parsing core of `Quake/cfgfile.c` (early config-file cvar
//! reads). The file-handle/Cvar_Set orchestration lives in quake-capi.
//!
//! The exact early-parse line format is compat-relevant (PLAN §6.2): a line
//! qualifies only if — after nulling `\r`/`\n`, turning tabs into spaces and
//! trimming trailing spaces — its last character is `"`; the cvar must match
//! as `<name><space>` at the very start (C: `strstr(buff, "<name> ") ==
//! buff`), and the value is everything after the *first* `"` on the line.

/// One CFG_ReadCvars line, over the raw FS_fgets buffer (fgets guarantees a
/// terminating NUL; the C memsets the buffer each iteration). Returns the
/// first matching var index and the value bytes.
pub fn process_line(raw: &[u8; 1024], vars: &[&[u8]]) -> Option<(usize, Vec<u8>)> {
    let mut buff = *raw;

    // remove end-of-line characters; replace tabs with spaces (scan runs to
    // the first pre-existing NUL, continuing past nulled \r/\n like the C)
    let mut i = 0usize;
    while buff[i] != 0 {
        if buff[i] == b'\r' || buff[i] == b'\n' {
            buff[i] = 0;
        }
        if buff[i] == b'\t' {
            buff[i] = b' ';
        }
        i += 1;
    }
    // go to the last character
    while buff[i] == 0 && i > 0 {
        i -= 1;
    }
    // remove trailing spaces
    while i > 0 {
        if buff[i] == b' ' {
            buff[i] = 0;
            i -= 1;
        } else {
            break;
        }
    }

    // the line must end with a quotation mark
    if buff[i] != b'"' {
        return None;
    }
    buff[i] = 0;

    for (vi, var) in vars.iter().enumerate() {
        // C: strstr (buff, va ("%s ", var)) == buff — name-plus-one-space
        // prefix at the very start
        let prefix_matches =
            buff.len() > var.len() && buff[..var.len()] == **var && buff[var.len()] == b' ';
        if !prefix_matches {
            continue;
        }
        // locate the first quotation mark
        let cstr_end = buff.iter().position(|&b| b == 0).unwrap_or(buff.len());
        if let Some(q) = buff[..cstr_end].iter().position(|&b| b == b'"') {
            let value_end = buff[q + 1..]
                .iter()
                .position(|&b| b == 0)
                .map_or(buff.len(), |p| q + 1 + p);
            return Some((vi, buff[q + 1..value_end].to_vec()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(s: &[u8]) -> [u8; 1024] {
        let mut b = [0u8; 1024];
        b[..s.len()].copy_from_slice(s);
        b
    }

    #[test]
    fn parses_cvar_write_format() {
        let vars: &[&[u8]] = &[b"vid_width", b"vid_height"];
        assert_eq!(
            process_line(&line(b"vid_width \"1920\"\r\n"), vars),
            Some((0, b"1920".to_vec()))
        );
        assert_eq!(
            process_line(&line(b"vid_height\t\"1080\"  \n"), vars),
            Some((1, b"1080".to_vec()))
        );
        // seta prefix does not match (name must be at line start)
        assert_eq!(process_line(&line(b"seta vid_width \"1\"\n"), vars), None);
        // no trailing quote
        assert_eq!(process_line(&line(b"vid_width \"1920\n"), vars), None);
        // value is everything after the FIRST quote
        assert_eq!(
            process_line(&line(b"vid_width \"19\"20\"\n"), vars),
            Some((0, b"19\"20".to_vec()))
        );
        // empty value
        assert_eq!(
            process_line(&line(b"vid_width \"\"\n"), vars),
            Some((0, Vec::new()))
        );
        // no space after name
        assert_eq!(process_line(&line(b"vid_width\"1\"\n"), vars), None);
        // empty/garbage lines
        assert_eq!(process_line(&line(b"\n"), vars), None);
        assert_eq!(process_line(&line(b"// comment\n"), vars), None);
    }
}
