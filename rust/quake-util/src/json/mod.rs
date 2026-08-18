//! Port of `Quake/json.c` (parsing layer over the jsmn hand-port).
//!
//! ADR-012: jsmn's tolerant, non-validating behavior is the spec; acceptance
//! must not change. Numbers go through the platform `strtod` (quake-c-sys)
//! with the same greedy-past-the-token semantics as C — strtod receives the
//! NUL-terminated remainder of the input starting at the token, exactly like
//! the C passes `text + start`.
//!
//! This module produces a pure description of the parse (entry list + packed
//! unescaped string bytes); the C-layout arena (`json_t` + `jsonentry_t[]` +
//! strings in one Mem_Alloc block) is assembled by the quake-capi shim.

pub mod jsmn;

use jsmn::{JsmnParser, JSMN_ARRAY, JSMN_OBJECT, JSMN_PRIMITIVE, JSMN_STRING};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EntryData {
    /// primitive with len == 0 in C leaves the zeroed entry untouched:
    /// type JSON_INVALID, value 0
    Invalid,
    Object,
    Array,
    /// offset into `ParsedJson::strings` of this entry's NUL-terminated bytes
    String(usize),
    Number(f64),
    Boolean(bool),
    Null,
}

#[derive(Debug, Clone, Copy)]
pub struct EntryDesc {
    pub data: EntryData,
    /// parent entry index; C only links when `parent >= 0 && parent < i`
    pub parent: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct ParsedJson {
    pub entries: Vec<EntryDesc>,
    /// concatenated unescaped strings, each NUL-terminated, plus the final
    /// extra NUL the C writes after the loop
    pub strings: Vec<u8>,
    /// the allocation size C would have used for the strings buffer
    /// (`sum(end - start + 1) + 1`) — an upper bound on `strings.len()`
    pub c_strings_alloc_len: usize,
}

fn read_hex_digit(b: u8) -> i32 {
    if b.wrapping_sub(b'0') < 10 {
        return (b - b'0') as i32;
    }
    if b.wrapping_sub(b'a') < 6 {
        return (b - b'a' + 10) as i32;
    }
    if b.wrapping_sub(b'A') < 6 {
        return (b - b'A' + 10) as i32;
    }
    -1
}

fn read_hex_number(s: &[u8]) -> Option<i32> {
    let d0 = read_hex_digit(s[0]);
    let d1 = read_hex_digit(s[1]);
    let d2 = read_hex_digit(s[2]);
    let d3 = read_hex_digit(s[3]);
    if d0 < 0 || d1 < 0 || d2 < 0 || d3 < 0 {
        return None;
    }
    Some((d0 << 12) | (d1 << 8) | (d2 << 4) | d3)
}

/// `UTF8_WriteCodePoint` (common.c) with maxbytes = 4, as json.c calls it.
fn utf8_write_code_point(dst: &mut Vec<u8>, codepoint: u32) -> usize {
    if codepoint < 0x80 {
        dst.push(codepoint as u8);
        1
    } else if codepoint < 0x800 {
        dst.push(0xC0 | (codepoint >> 6) as u8);
        dst.push(0x80 | (codepoint & 63) as u8);
        2
    } else if codepoint < 0x10000 {
        dst.push(0xE0 | (codepoint >> 12) as u8);
        dst.push(0x80 | ((codepoint >> 6) & 63) as u8);
        dst.push(0x80 | (codepoint & 63) as u8);
        3
    } else if codepoint < 0x110000 {
        dst.push(0xF0 | (codepoint >> 18) as u8);
        dst.push(0x80 | ((codepoint >> 12) & 63) as u8);
        dst.push(0x80 | ((codepoint >> 6) & 63) as u8);
        dst.push(0x80 | (codepoint & 63) as u8);
        4
    } else {
        0
    }
}

/// `JSON_Unescape`: appends the unescaped bytes of `src` plus a trailing NUL.
fn unescape(dst: &mut Vec<u8>, src: &[u8]) {
    let mut i = 0;
    let len = src.len();
    while i < len {
        let c = src[i];
        if c != b'\\' {
            dst.push(c);
            i += 1;
            continue;
        }

        i += 1;
        if i == len {
            dst.push(b'\\');
            break;
        }

        match src[i] {
            b'"' => dst.push(b'"'),
            b'\\' => dst.push(b'\\'),
            b'b' => dst.push(0x08),
            b'f' => dst.push(0x0c),
            b'n' => dst.push(b'\n'),
            b'r' => dst.push(b'\r'),
            b't' => dst.push(b'\t'),
            b'u' if len - i > 4 => {
                if let Some(mut codepoint) = read_hex_number(&src[i + 1..i + 5]) {
                    if (0xd800..=0xdbff).contains(&codepoint)
                        && len - i > 10
                        && src[i + 5] == b'\\'
                        && src[i + 6] == b'u'
                    {
                        if let Some(lowsurrogate) = read_hex_number(&src[i + 7..i + 11]) {
                            if (0xdc00..=0xdfff).contains(&lowsurrogate) {
                                codepoint = 0x10000
                                    + ((codepoint - 0xd800) << 10)
                                    + (lowsurrogate - 0xdc00);
                                i += 6;
                            }
                        }
                    }
                    if (0xd800..=0xdfff).contains(&codepoint) {
                        // unpaired surrogate, don't emit invalid UTF-8
                        codepoint = 0xfffd;
                    }
                    utf8_write_code_point(dst, codepoint as u32);
                    i += 5; // the 4 hex digits + loop advance
                    continue;
                }
                dst.push(b'\\');
                dst.push(b'u');
            }
            other => {
                dst.push(b'\\');
                dst.push(other);
            }
        }
        i += 1;
    }
    dst.push(0);
}

/// `JSON_Parse` analysis: BOM handling, two-pass jsmn, per-token entry
/// construction. `text` is the C string's bytes (no interior NULs when called
/// from C; interior NULs terminate parsing like C's `js[pos] != '\0'`
/// checks). Returns `None` exactly when the C returns NULL.
pub fn parse(mut text: &[u8]) -> Option<ParsedJson> {
    // fail on UTF-16 byte order marks; skip a UTF-8 one. C reads text[0..2]
    // of a NUL-terminated string; get() mirrors the short-circuiting.
    let b0 = text.first().copied().unwrap_or(0);
    let b1 = text.get(1).copied().unwrap_or(0);
    if b0 == 0xFF && b1 == 0xFE {
        return None;
    }
    if b0 == 0xFE && b1 == 0xFF {
        return None;
    }
    if b0 == 0xEF && b1 == 0xBB && text.get(2).copied().unwrap_or(0) == 0xBF {
        text = &text[3..];
    }

    let len = text.iter().position(|&b| b == 0).unwrap_or(text.len());
    let mut parser = JsmnParser::new();
    let numtokens = parser.parse(text, len, None);
    if numtokens <= 0 {
        return None;
    }
    let numtokens = numtokens as usize;

    let mut tokens = vec![
        jsmn::JsmnTok {
            type_: 0,
            start: 0,
            end: 0,
            size: 0,
            parent: 0,
        };
        numtokens
    ];
    let mut parser = JsmnParser::new();
    let filled = parser.parse(text, len, Some(&mut tokens));
    if filled != numtokens as i32 {
        return None;
    }

    // C: allocation length for the strings buffer
    let mut alloc_len: usize = 1;
    for t in &tokens {
        if t.type_ == JSMN_STRING {
            alloc_len += (t.end - t.start + 1) as usize;
        }
    }

    // strtod needs the NUL-terminated remainder of the text, like C
    let mut ztext = text[..len].to_vec();
    ztext.push(0);

    let mut entries = Vec::with_capacity(numtokens);
    let mut strings = Vec::with_capacity(alloc_len);
    for (i, t) in tokens.iter().enumerate() {
        let parent = if t.parent >= 0 && (t.parent as usize) < i {
            Some(t.parent as usize)
        } else {
            None
        };

        let data = if t.type_ == JSMN_OBJECT {
            EntryData::Object
        } else if t.type_ == JSMN_ARRAY {
            EntryData::Array
        } else if t.type_ == JSMN_STRING {
            let offset = strings.len();
            unescape(&mut strings, &text[t.start as usize..t.end as usize]);
            EntryData::String(offset)
        } else if t.type_ == JSMN_PRIMITIVE {
            let tok_len = t.end - t.start;
            if tok_len > 0 {
                match text[t.start as usize] {
                    b't' => EntryData::Boolean(true),
                    b'f' => EntryData::Boolean(false),
                    b'n' => EntryData::Null,
                    _ => EntryData::Number(quake_c_sys::libm::strtod(&ztext[t.start as usize..])),
                }
            } else {
                EntryData::Invalid
            }
        } else {
            EntryData::Invalid
        };

        entries.push(EntryDesc { data, parent });
    }
    // C: `*strings++ = '\0'` after the loop
    strings.push(0);

    Some(ParsedJson {
        entries,
        strings,
        c_strings_alloc_len: alloc_len,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_and_shapes() {
        let p = parse(br#"{"a": 1, "b": [true, false, null], "c": "x\ny"}"#).unwrap();
        assert_eq!(p.entries[0].data, EntryData::Object);
        assert!(matches!(p.entries[2].data, EntryData::Number(n) if n == 1.0));

        // tolerant mode: bare primitives and top-level strings are accepted
        assert!(parse(b"hello").is_some());
        assert!(parse(b"\"top\"").is_some());
        assert!(parse(b"{key: value}").is_some());

        // rejects
        assert!(parse(b"").is_none());
        assert!(parse(b"   ").is_none());
        assert!(parse(b"\xFF\xFEx").is_none());
        assert!(parse(b"\xFE\xFFx").is_none());
        assert!(parse(b"{\"unterminated\": \"str").is_none());
        assert!(parse(b"{").is_none());

        // UTF-8 BOM skipped
        assert!(parse(b"\xEF\xBB\xBF{}").is_some());
    }

    #[test]
    fn unescape_semantics() {
        let mut out = Vec::new();
        unescape(&mut out, br"a\nb\t\\\q");
        assert_eq!(out, b"a\nb\t\\\\q\0");

        // \uXXXX incl. surrogate pair, unpaired surrogate -> U+FFFD
        let mut out = Vec::new();
        unescape(&mut out, br"\u0041\uD83D\uDE00\uD800x");
        let mut expect = b"A".to_vec();
        expect.extend_from_slice("😀".as_bytes());
        expect.extend_from_slice("\u{fffd}".as_bytes());
        expect.extend_from_slice(b"x\0");
        assert_eq!(out, expect);

        // trailing lone backslash stays literal
        let mut out = Vec::new();
        unescape(&mut out, br"x\");
        assert_eq!(out, b"x\\\0");

        // invalid \u falls through as literal backslash + u
        let mut out = Vec::new();
        unescape(&mut out, br"\uZZZZ!");
        assert_eq!(out, b"\\uZZZZ!\0");
    }
}
