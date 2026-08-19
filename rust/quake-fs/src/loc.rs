//! Pure core of the LOCALIZATION block of `Quake/common_fs.c`: the parsing
//! half of LOC_LoadFile, COM_HashString/COM_HashBlock, LOC_GetRawString,
//! LOC_ParseArg, LOC_HasPlaceholders and LOC_Format. File acquisition (kpf
//! lookup), Con_Printf/Sys_Error and the `localization` global stay in the
//! shim.
//!
//! The C mutates one flat text buffer in place and keeps `char *` key/value
//! pointers into it; here they are byte offsets into an identical buffer
//! image (same in-place NULs), so a later FFI shim can hand out stable
//! pointers. Replicated quirks (all bug-for-bug from this repo's C):
//! - the unquoted right-trim writes each NUL *after* the blank it tested, so
//!   one trailing blank always survives — and when nothing shrank the value
//!   the first write lands on the line's `\n`, which stops the entire parse
//!   at that line;
//! - unquoted values shrunk by escapes get no terminator at the new end, so
//!   stale tail bytes up to the line's NUL stay part of the value;
//! - any unescaped `"` ends the value, leading quote or not, and `\\` is an
//!   "unrecognized escape" that still decodes to one backslash;
//! - LOC_Format's `numargs` is never incremented, so the per-argument
//!   overflow warning always says `#0`.

/// COM_HashString: FNV-1a, but `hash ^= *str++` goes through plain `char`,
/// whose signedness is implementation-defined — signed on x86-64 and Apple
/// arm64, unsigned on AArch64/ARM Linux — so a byte >= 0x80 XORs in
/// sign-extended on some targets and zero-extended on others. Casting
/// through `c_char` reproduces whichever the C compiler for this target
/// picked, so the hashes agree per platform rather than only where `char`
/// happens to be signed. (`COM_HashBlock` reads `const byte *`, i.e. always
/// unsigned — see `hash_block`.) `s` is the key's NUL-free bytes.
pub fn hash_string(s: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c9dc5;
    for &b in s {
        hash ^= b as core::ffi::c_char as i32 as u32;
        hash = hash.wrapping_mul(0x01000193);
    }
    hash
}

/// COM_HashBlock: FNV-1a over unsigned bytes (no sign extension here — the
/// C walks a `const byte *`).
pub fn hash_block(data: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c9dc5;
    for &b in data {
        hash ^= u32::from(b);
        hash = hash.wrapping_mul(0x01000193);
    }
    hash
}

fn is_blank(c: u8) -> bool {
    c == b' ' || c == b'\t'
}

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0c | 0x0b)
}

fn c_str(text: &[u8], offset: usize) -> &[u8] {
    let end = text[offset..].iter().position(|&b| b == 0).unwrap() + offset;
    &text[offset..end]
}

/// locentry_t, with the `char *`s as byte offsets into [`LocData::text`].
/// Each key and value is NUL-terminated in place there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocEntry {
    pub key: usize,
    pub value: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocWarning {
    /// C: `Con_DPrintf ("LOC_LoadFile: malformed comment on line %d\n", lineno)`
    MalformedComment { line: i32 },
    /// C: `Con_Printf ("LOC_LoadFile: unrecognized escape sequence \\%c on line %d\n", c, lineno)`
    /// (a plain Con_Printf, unlike the comment warning)
    UnrecognizedEscape { c: u8, line: i32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocError {
    /// C: `Sys_Error ("LOC_LoadFile failed")` — the index probe wrapped.
    /// Unreachable at the fixed 50% load factor, kept for fidelity.
    IndexFull,
}

/// localization_t minus the engine-global lifecycle.
#[derive(Debug)]
pub struct LocData {
    text: Vec<u8>,
    entries: Vec<LocEntry>,
    index: Vec<u32>,
}

impl LocData {
    /// The mutated buffer image: file bytes with the in-place NULs, plus the
    /// trailing NUL COM_LoadFile appends. Entry offsets index into it; its
    /// base address is what the shim's stable pointers hang off.
    pub fn text(&self) -> &[u8] {
        &self.text
    }

    pub fn entries(&self) -> &[LocEntry] {
        &self.entries
    }

    /// The NUL-terminated string starting at `offset` (an entry key/value
    /// offset) in the text image, without the NUL.
    pub fn str_at(&self, offset: usize) -> &[u8] {
        c_str(&self.text, offset)
    }

    /// LOC_GetRawString, returning the value's byte offset into the text
    /// image. `key` is the C string's NUL-free bytes: it must begin with
    /// `'$'`, which is stripped before hashing and comparing (stored keys
    /// carry no `'$'`).
    pub fn get_raw(&self, key: &[u8]) -> Option<usize> {
        if self.index.is_empty() || key.first() != Some(&b'$') {
            return None;
        }
        let key = &key[1..];
        let n = self.index.len();
        let mut pos = (hash_string(key) as usize) % n;
        let end = pos;
        loop {
            let idx = self.index[pos];
            if idx == 0 {
                return None;
            }
            let entry = &self.entries[idx as usize - 1];
            if self.str_at(entry.key) == key {
                return Some(entry.value);
            }
            pos += 1;
            if pos == n {
                pos = 0;
            }
            if pos == end {
                return None;
            }
        }
    }

    /// [`Self::get_raw`] as string bytes (LOC_GetString's found case; the
    /// key-echo fallback is the shim's).
    pub fn get_raw_str(&self, key: &[u8]) -> Option<&[u8]> {
        self.get_raw(key).map(|off| self.str_at(off))
    }

    /// LOC_HasPlaceholders bails first when nothing is loaded
    /// (`!localization.numindices`).
    pub fn has_placeholders(&self, s: &[u8]) -> bool {
        !self.index.is_empty() && has_placeholders(s)
    }
}

/// The parsing portion of LOC_LoadFile, over the raw file bytes (a NUL is
/// appended like COM_LoadFile does; an embedded NUL ends the parse early
/// like the C's `while (*cursor)`).
pub fn parse(file_bytes: &[u8], warn: &mut dyn FnMut(LocWarning)) -> Result<LocData, LocError> {
    let mut text = Vec::with_capacity(file_bytes.len() + 1);
    text.extend_from_slice(file_bytes);
    text.push(0);

    let mut entries: Vec<LocEntry> = Vec::new();
    let mut maxnumentries = 0usize;

    let mut cursor = 0usize;
    if text.starts_with(&[0xef, 0xbb, 0xbf]) {
        cursor = 3;
    }

    let mut lineno: i32 = 0;
    while text[cursor] != 0 {
        lineno = lineno.wrapping_add(1);

        while is_blank(text[cursor]) {
            cursor += 1;
        }

        let line = cursor;
        let mut equals: Option<usize> = None;
        while text[cursor] != 0 && text[cursor] != b'\n' {
            if text[cursor] == b'=' && equals.is_none() {
                equals = Some(cursor);
            }
            cursor += 1;
        }

        if text[line] == b'/' {
            if text[line + 1] != b'/' {
                warn(LocWarning::MalformedComment { line: lineno });
            }
        } else if let Some(eq) = equals {
            let mut key_end = eq;
            while key_end != line && is_space(text[key_end - 1]) {
                key_end -= 1;
            }
            text[key_end] = 0;

            let mut value = eq + 1;
            while value != cursor && is_space(text[value]) {
                value += 1;
            }

            let leading_quote = text[value] == b'"';
            let mut trailing_quote = false;
            value += usize::from(leading_quote);

            let mut src = value;
            let mut dst = value;
            while src != cursor {
                if text[src] == b'\\' && src + 1 != cursor {
                    let c = text[src + 1];
                    src += 2;
                    text[dst] = match c {
                        b'n' => b'\n',
                        b't' => b'\t',
                        b'v' => 0x0b,
                        b'b' => 0x08,
                        b'f' => 0x0c,
                        b'"' | b'\'' => c,
                        _ => {
                            warn(LocWarning::UnrecognizedEscape { c, line: lineno });
                            c
                        }
                    };
                    dst += 1;
                    continue;
                }

                if text[src] == b'"' {
                    trailing_quote = true;
                    text[dst] = 0;
                    break;
                }

                text[dst] = text[src];
                dst += 1;
                src += 1;
            }

            // LOC_LoadFile's unquoted right-trim: zeroes at dst, not dst-1,
            // keeping one blank; when dst == cursor the first write clobbers
            // the '\n' and the outer loop below stops the whole parse
            if !trailing_quote {
                while dst != value && is_blank(text[dst - 1]) {
                    text[dst] = 0;
                    dst -= 1;
                }
            }

            if entries.len() == maxnumentries {
                // grow by 50%, minimum 32 (allocation only, not observable)
                maxnumentries += maxnumentries >> 1;
                maxnumentries = maxnumentries.max(32);
                entries.reserve_exact(maxnumentries - entries.len());
            }

            entries.push(LocEntry { key: line, value });
        }

        if text[cursor] != 0 {
            text[cursor] = 0;
            cursor += 1;
        }
    }

    // hash all entries: numentries*2 slots (50% load factor), open
    // addressing with linear probe, slot value i+1, 0 = empty
    let numindices = entries.len() * 2;
    if numindices == 0 {
        // the C Con_Printf's "No localized strings in file '%s'" and leaves
        // numindices 0; lookups then fail — an empty index does the same
        return Ok(LocData {
            text,
            entries,
            index: Vec::new(),
        });
    }

    let mut index = vec![0u32; numindices];
    for (i, entry) in entries.iter().enumerate() {
        let mut pos = (hash_string(c_str(&text, entry.key)) as usize) % numindices;
        let end = pos;
        loop {
            if index[pos] == 0 {
                index[pos] = (i + 1) as u32;
                break;
            }
            pos += 1;
            if pos == numindices {
                pos = 0;
            }
            if pos == end {
                return Err(LocError::IndexFull);
            }
        }
    }

    Ok(LocData {
        text,
        entries,
        index,
    })
}

/// LOC_ParseArg over C-string semantics (out of range reads as NUL). On a
/// syntactic match `*pos` advances past the `}` and the accumulated index is
/// returned — the C sums `arg * 10 + digit` in a (practically wrapping)
/// int, so an overflowing `{N}` still advances and can return a wrapped
/// negative value. Otherwise returns -1 with `*pos` untouched.
pub fn parse_arg(s: &[u8], pos: &mut usize) -> i32 {
    let mut p = *pos;
    if s.get(p) != Some(&b'{') {
        return -1;
    }
    p += 1;

    let mut arg: i32 = 0;
    while let Some(&c) = s.get(p) {
        if !c.is_ascii_digit() {
            break;
        }
        arg = arg.wrapping_mul(10).wrapping_add(i32::from(c - b'0'));
        p += 1;
    }

    if s.get(p) != Some(&b'}') {
        return -1;
    }
    *pos = p + 1;
    arg
}

/// LOC_HasPlaceholders' scan half; the C's numindices guard lives on
/// [`LocData::has_placeholders`]. (A wrapped-negative `{N}` flush at the end
/// of the string makes the C read past the terminator — UB it can't share;
/// the scan just ends at the slice here.)
pub fn has_placeholders(s: &[u8]) -> bool {
    let mut pos = 0usize;
    while pos < s.len() {
        if parse_arg(s, &mut pos) >= 0 {
            return true;
        }
        pos += 1;
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatWarning {
    /// C: `Con_DPrintf ("LOC_Format: no output space\n")`
    NoOutputSpace,
    /// C: `Con_DPrintf ("LOC_Format: overflow at argument #%d\n", numargs)`
    /// — the C never increments numargs, so it is always 0
    OverflowAtArgument { numargs: i32 },
    /// C: `Con_DPrintf ("LOC_Format: overflow\n")`
    Overflow,
}

/// LOC_Format. `out.len()` plays the C's `len`: the last byte is reserved
/// for the NUL, which is written at `out[written]` whenever `out` is
/// non-empty. `getarg` gets the placeholder index (the C's `getarg_fn`
/// without the userdata pointer) and must return NUL-free bytes — the C
/// strlen's the argument. Returns the written count, excluding the NUL.
pub fn format<'a>(
    fmt: &[u8],
    getarg: &mut dyn FnMut(i32) -> &'a [u8],
    out: &mut [u8],
    warn: &mut dyn FnMut(FormatWarning),
) -> usize {
    let mut written = 0usize;
    let numargs: i32 = 0;

    if out.is_empty() {
        warn(FormatWarning::NoOutputSpace);
        return 0;
    }
    let len = out.len() - 1;

    let mut pos = 0usize;
    while pos < fmt.len() && written < len {
        let argindex = parse_arg(fmt, &mut pos);

        if argindex < 0 {
            // a wrapped-negative {N} flush at the end would have the C copy
            // the terminator and read on past it (UB); the slice just ends
            let Some(&c) = fmt.get(pos) else { break };
            out[written] = c;
            written += 1;
            pos += 1;
            continue;
        }

        let insert = getarg(argindex);
        let space_left = len - written;
        let mut insert_len = insert.len();

        if insert_len > space_left {
            warn(FormatWarning::OverflowAtArgument { numargs });
            insert_len = space_left;
        }

        out[written..written + insert_len].copy_from_slice(&insert[..insert_len]);
        written += insert_len;
    }

    if pos < fmt.len() {
        warn(FormatWarning::Overflow);
    }

    out[written] = 0;
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(src: &[u8]) -> (LocData, Vec<LocWarning>) {
        let mut warns = Vec::new();
        let data = parse(src, &mut |w| warns.push(w)).unwrap();
        (data, warns)
    }

    fn value(data: &LocData, key: &[u8]) -> Option<Vec<u8>> {
        data.get_raw_str(key).map(<[u8]>::to_vec)
    }

    #[test]
    fn fnv1a_vectors() {
        assert_eq!(hash_string(b""), 0x811c9dc5);
        assert_eq!(hash_string(b"a"), 0xe40c292c);
        assert_eq!(hash_string(b"foobar"), 0xbf9cf968);
        assert_eq!(hash_block(b"a"), 0xe40c292c);
        assert_eq!(hash_block(b"foobar"), 0xbf9cf968);
    }

    #[test]
    fn hash_string_sign_extends_high_bytes() {
        // COM_HashString xors the sign-extended char; COM_HashBlock the byte
        assert_eq!(
            hash_string(&[0xc4]),
            (0x811c9dc5u32 ^ 0xffff_ffc4).wrapping_mul(0x01000193)
        );
        assert_eq!(
            hash_block(&[0xc4]),
            (0x811c9dc5u32 ^ 0xc4).wrapping_mul(0x01000193)
        );
        assert_ne!(hash_string(&[0xc4]), hash_block(&[0xc4]));
    }

    #[test]
    fn basic_entries_and_lookup() {
        let (data, warns) = parse_ok(b"KEY=value\nK2=\"quoted\"\n");
        assert!(warns.is_empty());
        assert_eq!(data.entries().len(), 2);
        assert_eq!(value(&data, b"$KEY").as_deref(), Some(&b"value"[..]));
        assert_eq!(value(&data, b"$K2").as_deref(), Some(&b"quoted"[..]));
        // LOC_GetRawString requires the '$' prefix and a loaded index
        assert_eq!(data.get_raw(b"KEY"), None);
        assert_eq!(data.get_raw(b""), None);
        assert_eq!(data.get_raw(b"$MISSING"), None);
    }

    #[test]
    fn buffer_image_matches_c() {
        // key NUL replaces '=', line NUL replaces '\n', plus the trailing NUL
        let (data, _) = parse_ok(b"A=1\n");
        assert_eq!(data.text(), b"A\x001\x00\x00");
        // closing quote gets the NUL; junk after it stays in the image
        let (data, _) = parse_ok(b"A=\"v\"!\n");
        assert_eq!(data.text(), b"A\x00\"v\x00!\x00\x00");
        assert_eq!(value(&data, b"$A").as_deref(), Some(&b"v"[..]));
    }

    #[test]
    fn bom_and_blank_lines() {
        let (data, _) = parse_ok(b"\xef\xbb\xbfK=v\n");
        assert_eq!(value(&data, b"$K").as_deref(), Some(&b"v"[..]));
        let (data, warns) = parse_ok(b"\n   \n\r\nno equals here\n");
        assert!(warns.is_empty());
        assert_eq!(data.entries().len(), 0);
        assert_eq!(data.get_raw(b"$x"), None);
    }

    #[test]
    fn comments() {
        let (data, warns) = parse_ok(b"// a=b\nK=v\n");
        assert_eq!(data.entries().len(), 1);
        assert!(warns.is_empty());
        // single slash: comment still skipped, DPrintf warning
        let (data, warns) = parse_ok(b"//ok\n/bad=x\nK=v\n");
        assert_eq!(data.entries().len(), 1);
        assert_eq!(warns, vec![LocWarning::MalformedComment { line: 2 }]);
        // '//' later in the line is not a comment
        let (data, _) = parse_ok(b"x //y=z\n");
        assert_eq!(data.str_at(data.entries()[0].key), b"x //y");
        assert_eq!(value(&data, b"$x //y").as_deref(), Some(&b"z"[..]));
    }

    #[test]
    fn trim_and_quote_rules() {
        // leading blanks before key, isspace trim around '='
        let (data, _) = parse_ok(b"  A \t= \t v\n");
        assert_eq!(data.str_at(data.entries()[0].key), b"A");
        assert_eq!(value(&data, b"$A").as_deref(), Some(&b"v"[..]));
        // unescaped quote ends the value even without a leading quote
        let (data, _) = parse_ok(b"A=ab\"cd\nB=b\n");
        assert_eq!(value(&data, b"$A").as_deref(), Some(&b"ab"[..]));
        assert_eq!(value(&data, b"$B").as_deref(), Some(&b"b"[..]));
        // quoted value keeps inner blanks; junk after the close is ignored
        let (data, _) = parse_ok(b"A=\"hi there \" junk\nB=b\n");
        assert_eq!(value(&data, b"$A").as_deref(), Some(&b"hi there "[..]));
        assert_eq!(value(&data, b"$B").as_deref(), Some(&b"b"[..]));
        // unterminated leading quote: quote skipped, no right-trim issue
        let (data, _) = parse_ok(b"A=\"v\n");
        assert_eq!(value(&data, b"$A").as_deref(), Some(&b"v"[..]));
        // CR is not q_isblank: unquoted value keeps a trailing '\r'
        let (data, _) = parse_ok(b"A=v\r\nB=w\r\n");
        assert_eq!(value(&data, b"$A").as_deref(), Some(&b"v\r"[..]));
        assert_eq!(value(&data, b"$B").as_deref(), Some(&b"w\r"[..]));
        // ...but the key-side trim is q_isspace, which eats '\r'
        let (data, _) = parse_ok(b"A\r=v\n");
        assert_eq!(data.str_at(data.entries()[0].key), b"A");
        // empty key and empty value both produce entries
        let (data, _) = parse_ok(b"=v\nA=\n");
        assert_eq!(value(&data, b"$").as_deref(), Some(&b"v"[..]));
        assert_eq!(value(&data, b"$A").as_deref(), Some(&b""[..]));
    }

    #[test]
    fn trailing_blank_trim_stops_the_parse() {
        // the trim's first NUL lands on the '\n': one blank survives in the
        // value and every later line is dropped, exactly like the C
        let (data, warns) = parse_ok(b"A=x \nB=y\n");
        assert!(warns.is_empty());
        assert_eq!(data.entries().len(), 1);
        assert_eq!(value(&data, b"$A").as_deref(), Some(&b"x "[..]));
        assert_eq!(data.get_raw(b"$B"), None);
        // multiple blanks: still exactly one survivor
        let (data, _) = parse_ok(b"A=x  \t\nB=y\n");
        assert_eq!(data.entries().len(), 1);
        assert_eq!(value(&data, b"$A").as_deref(), Some(&b"x "[..]));
    }

    #[test]
    fn escape_decoding() {
        let (data, warns) = parse_ok(b"A=\"1\\n2\\t3\\v4\\b5\\f6\\\"7\\'8\"\n");
        assert!(warns.is_empty());
        assert_eq!(
            value(&data, b"$A").as_deref(),
            Some(&b"1\n2\t3\x0b4\x085\x0c6\"7'8"[..])
        );
        // unknown escape: literal char plus warning
        let (data, warns) = parse_ok(b"A=\"x\\qy\"\n");
        assert_eq!(value(&data, b"$A").as_deref(), Some(&b"xqy"[..]));
        assert_eq!(
            warns,
            vec![LocWarning::UnrecognizedEscape { c: b'q', line: 1 }]
        );
        // backslash-backslash is itself "unrecognized" but yields one '\'
        let (data, warns) = parse_ok(b"A=\"x\\\\y\"\n");
        assert_eq!(value(&data, b"$A").as_deref(), Some(&b"x\\y"[..]));
        assert_eq!(
            warns,
            vec![LocWarning::UnrecognizedEscape { c: b'\\', line: 1 }]
        );
        // lone backslash at line end: copied literally, no warning
        let (data, warns) = parse_ok(b"A=x\\\n");
        assert!(warns.is_empty());
        assert_eq!(value(&data, b"$A").as_deref(), Some(&b"x\\"[..]));
    }

    #[test]
    fn unquoted_escape_leaves_stale_tail() {
        // decode shrinks in place but writes no terminator: the last source
        // byte reappears ("a\nb" + stale 'b'), and parsing continues
        let (data, _) = parse_ok(b"K=a\\nb\nX=y\n");
        assert_eq!(data.entries().len(), 2);
        assert_eq!(value(&data, b"$K").as_deref(), Some(&b"a\nbb"[..]));
        assert_eq!(value(&data, b"$X").as_deref(), Some(&b"y"[..]));
    }

    #[test]
    fn embedded_nul_ends_parse() {
        let (data, _) = parse_ok(b"A=1\n\0B=2\n");
        assert_eq!(data.entries().len(), 1);
        assert_eq!(data.get_raw(b"$B"), None);
    }

    #[test]
    fn index_probe_and_duplicates() {
        // 'A' and 'E' collide mod 4 (two entries -> four slots)
        assert_eq!(hash_string(b"A") % 4, hash_string(b"E") % 4);
        let (data, _) = parse_ok(b"A=1\nE=2\n");
        assert_eq!(value(&data, b"$A").as_deref(), Some(&b"1"[..]));
        assert_eq!(value(&data, b"$E").as_deref(), Some(&b"2"[..]));
        // duplicate keys: both inserted, probe finds the first
        let (data, _) = parse_ok(b"A=1\nA=2\n");
        assert_eq!(data.entries().len(), 2);
        assert_eq!(value(&data, b"$A").as_deref(), Some(&b"1"[..]));
    }

    #[test]
    fn parse_arg_cases() {
        let mut pos = 0;
        assert_eq!(parse_arg(b"{}", &mut pos), 0);
        assert_eq!(pos, 2);
        let mut pos = 0;
        assert_eq!(parse_arg(b"{5}", &mut pos), 5);
        assert_eq!(pos, 3);
        let mut pos = 0;
        assert_eq!(parse_arg(b"{12}x", &mut pos), 12);
        assert_eq!(pos, 4);
        // failures leave pos untouched
        for s in [&b"{a}"[..], b"{12", b"x{}", b"", b"{"] {
            let mut pos = 0;
            assert_eq!(parse_arg(s, &mut pos), -1);
            assert_eq!(pos, 0);
        }
        // int wrap: the C still advances and returns the wrapped value
        let mut pos = 0;
        assert_eq!(parse_arg(b"{2147483648}", &mut pos), i32::MIN);
        assert_eq!(pos, 12);
    }

    #[test]
    fn has_placeholders_cases() {
        assert!(has_placeholders(b"{}"));
        assert!(has_placeholders(b"a {3} b"));
        assert!(!has_placeholders(b"{x} {12"));
        assert!(!has_placeholders(b""));
        assert!(!has_placeholders(b"{2147483648}"));
        // the LocData method adds the C's numindices guard
        let (empty, _) = parse_ok(b"");
        assert!(!empty.has_placeholders(b"{0}"));
        let (loaded, _) = parse_ok(b"A=1\n");
        assert!(loaded.has_placeholders(b"{0}"));
    }

    fn run_format(
        fmt: &[u8],
        args: &[&'static [u8]],
        out_len: usize,
    ) -> (Vec<u8>, usize, Vec<FormatWarning>) {
        let args = args.to_vec();
        let mut out = vec![0xaau8; out_len];
        let mut warns = Vec::new();
        let written = format(
            fmt,
            &mut |i| args.get(i as usize).copied().unwrap_or(b""),
            &mut out,
            &mut |w| warns.push(w),
        );
        (out, written, warns)
    }

    #[test]
    fn format_substitution() {
        let (out, written, warns) = run_format(b"Hi {0}, {1}{}", &[b"A", b"BB"], 64);
        assert!(warns.is_empty());
        assert_eq!(written, 9);
        assert_eq!(&out[..written], b"Hi A, BBA");
        assert_eq!(out[written], 0);
        // non-placeholder braces are copied literally
        let (out, written, _) = run_format(b"a{x}b{12", &[], 16);
        assert_eq!(&out[..written], b"a{x}b{12");
        // getarg sees the parsed index
        let mut out = [0u8; 16];
        let mut seen = Vec::new();
        format(
            b"{7}{}",
            &mut |i| {
                seen.push(i);
                b"z"
            },
            &mut out,
            &mut |_| {},
        );
        assert_eq!(seen, vec![7, 0]);
    }

    #[test]
    fn format_overflow() {
        // literal truncation: trailing "LOC_Format: overflow"
        let (out, written, warns) = run_format(b"abcdef", &[], 4);
        assert_eq!(written, 3);
        assert_eq!(&out[..4], b"abc\x00");
        assert_eq!(warns, vec![FormatWarning::Overflow]);
        // argument truncation: numargs is the C's never-incremented 0
        let (out, written, warns) = run_format(b"{0}", &[b"12345"], 4);
        assert_eq!(written, 3);
        assert_eq!(&out[..4], b"123\x00");
        assert_eq!(
            warns,
            vec![FormatWarning::OverflowAtArgument { numargs: 0 }]
        );
        // both warnings when format bytes remain after the clipped argument
        let (_, _, warns) = run_format(b"{0}z", &[b"12345"], 4);
        assert_eq!(
            warns,
            vec![
                FormatWarning::OverflowAtArgument { numargs: 0 },
                FormatWarning::Overflow
            ]
        );
        // exact fit: no warnings
        let (out, written, warns) = run_format(b"ab", &[], 3);
        assert_eq!((written, &out[..3]), (2, &b"ab\x00"[..]));
        assert!(warns.is_empty());
        // empty output buffer
        let (out, written, warns) = run_format(b"x", &[], 0);
        assert_eq!(written, 0);
        assert!(out.is_empty());
        assert_eq!(warns, vec![FormatWarning::NoOutputSpace]);
        // wrapped-negative {N}: consumed, then the next char is copied
        let (out, written, warns) = run_format(b"{2147483648}x", &[], 8);
        assert!(warns.is_empty());
        assert_eq!(&out[..written], b"x");
        assert_eq!(written, 1);
    }
}
