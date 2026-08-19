//! Port of the Valve KeyValues ("VDB") parser and Steam discovery matching
//! from `Quake/steam.c` (VDB_ParseString/VDB_ParseEntry/VDB_Parse,
//! VDB_OnLibFolderProperty, ACF_OnManifestProperty). File IO and the
//! `steamgame_t` buffer packing stay in the shim; this is the decision logic.
//!
//! Parity notes: the C decodes escapes in place while parsing, so the
//! callback sees decoded bytes; an unclosed node at end of input is silently
//! tolerated, every other malformed shape aborts the walk; and an aborted
//! walk discards any match (Steam_FindGame checks `!VDB_Parse || !result`).

/// `countof (ctx->path)`
pub const MAX_DEPTH: usize = 256;

/// `q_isspace`
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    /// `**buf`, with the slice end acting as the C string's NUL
    fn peek(&self) -> u8 {
        self.buf.get(self.pos).copied().unwrap_or(0)
    }

    fn advance(&mut self) {
        self.pos += 1;
    }

    fn skip_space(&mut self) {
        while is_space(self.peek()) {
            self.advance();
        }
    }

    /// `VDB_ParseString`: quoted string with escape decoding. None (like the
    /// C's NULL) on a missing opening quote, end of buffer inside the string,
    /// or an unsupported escape sequence.
    fn parse_string(&mut self) -> Option<Vec<u8>> {
        self.skip_space();
        if self.peek() != b'"' {
            return None;
        }
        self.advance();

        let mut out = Vec::new();
        loop {
            let c = self.peek();
            if c == 0 {
                // premature end of buffer
                return None;
            }
            if c == b'\\' {
                self.advance();
                let decoded = match self.peek() {
                    b'\'' => b'\'',
                    b'"' => b'"',
                    b'?' => b'?',
                    b'\\' => b'\\',
                    b'a' => 0x07,
                    b'b' => 0x08,
                    b'f' => 0x0c,
                    b'n' => b'\n',
                    b'r' => b'\r',
                    b't' => b'\t',
                    b'v' => 0x0b,
                    _ => return None, // unsupported sequence
                };
                out.push(decoded);
                self.advance();
                continue;
            }
            if c == b'"' {
                self.advance();
                break;
            }
            out.push(c);
            self.advance();
        }
        Some(out)
    }
}

type Callback<'a> = dyn FnMut(&[Vec<u8>], &[u8], &[u8]) + 'a;

/// `VDB_ParseEntry`
fn parse_entry(cur: &mut Cursor, path: &mut Vec<Vec<u8>>, callback: &mut Callback) -> bool {
    cur.skip_space();
    if cur.peek() == 0 {
        // end of buffer
        return true;
    }

    let Some(name) = cur.parse_string() else {
        return false;
    };

    cur.skip_space();

    if cur.peek() == b'"' {
        // key-value pair
        let Some(value) = cur.parse_string() else {
            return false;
        };
        callback(path, &name, &value);
        return true;
    }

    if cur.peek() == b'{' {
        // node
        cur.advance();
        if path.len() == MAX_DEPTH {
            return false;
        }
        path.push(name);

        while cur.peek() != 0 {
            cur.skip_space();

            if cur.peek() == b'}' {
                cur.advance();
                path.pop();
                break;
            }

            if cur.peek() == b'"' {
                if !parse_entry(cur, path, callback) {
                    return false;
                }
                continue;
            }

            if cur.peek() != 0 {
                return false;
            }
        }

        // like the C, end of buffer inside a node is tolerated (no pop)
        return true;
    }

    false
}

/// `VDB_Parse`: walks the buffer, invoking `callback (path, key, value)` for
/// every key/value pair. Returns false when the walk aborted; the C callers
/// discard results in that case.
pub fn parse(text: &[u8], callback: &mut Callback) -> bool {
    // C parses a NUL-terminated string; an interior NUL ends the buffer
    let end = text.iter().position(|&b| b == 0).unwrap_or(text.len());
    let mut cur = Cursor {
        buf: &text[..end],
        pos: 0,
    };
    let mut path = Vec::new();
    while cur.peek() != 0 {
        if !parse_entry(&mut cur, &mut path, callback) {
            return false;
        }
    }
    true
}

/// `sscanf (s, "%d", &idx) == 1`: optional whitespace, optional sign, at
/// least one digit (trailing garbage still counts as a match)
fn scans_as_int(s: &[u8]) -> bool {
    let mut i = 0;
    while i < s.len() && is_space(s[i]) {
        i += 1;
    }
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        i += 1;
    }
    i < s.len() && s[i].is_ascii_digit()
}

/// `VDB_OnLibFolderProperty` over a full parse of libraryfolders.vdf:
/// returns the "path" of the library whose "apps" block lists `appid`.
/// Later matches overwrite earlier ones, and `current` is never reset
/// between blocks, both exactly like the C.
pub fn find_library_for_app(libraryfolders_text: &[u8], appid: i32) -> Option<Vec<u8>> {
    let appidstr = appid.to_string().into_bytes();
    let mut current: Option<Vec<u8>> = None;
    let mut result: Option<Vec<u8>> = None;
    let ok = parse(libraryfolders_text, &mut |path, key, value| {
        if path.len() >= 2 && path[0] == b"libraryfolders" && scans_as_int(&path[1]) {
            if path.len() == 2 {
                if key == b"path" {
                    current = Some(value.to_vec());
                }
            } else if path.len() == 3 && key == appidstr && path[2] == b"apps" {
                // C: `parser->result = parser->current` — even when NULL
                result = current.clone();
            }
        }
    });
    if ok {
        result
    } else {
        None
    }
}

/// `ACF_OnManifestProperty` over a full parse of appmanifest_*.acf:
/// `["AppState"] "installdir"`, last match wins.
pub fn acf_installdir(acf_text: &[u8]) -> Option<Vec<u8>> {
    let mut result: Option<Vec<u8>> = None;
    let ok = parse(acf_text, &mut |path, key, value| {
        if path.len() == 1 && key == b"installdir" && path[0] == b"AppState" {
            result = Some(value.to_vec());
        }
    });
    if ok {
        result
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Event = (Vec<Vec<u8>>, Vec<u8>, Vec<u8>);

    fn run(text: &[u8]) -> (bool, Vec<Event>) {
        let mut events = Vec::new();
        let ok = parse(text, &mut |path, key, value| {
            events.push((path.to_vec(), key.to_vec(), value.to_vec()));
        });
        (ok, events)
    }

    #[test]
    fn basic_pairs_and_nodes() {
        let (ok, ev) =
            run(b"\"a\" \"1\"\n\"node\"\n{\n\t\"b\"\t\t\"2\"\n\t\"sub\" { \"c\" \"3\" }\n}\n");
        assert!(ok);
        assert_eq!(
            ev,
            vec![
                (vec![], b"a".to_vec(), b"1".to_vec()),
                (vec![b"node".to_vec()], b"b".to_vec(), b"2".to_vec()),
                (
                    vec![b"node".to_vec(), b"sub".to_vec()],
                    b"c".to_vec(),
                    b"3".to_vec()
                ),
            ]
        );

        // all q_isspace chars are separators, including \v and \f
        let (ok, ev) = run(b" \t\r\n\x0b\x0c\"k\"\x0b\"v\"");
        assert!(ok);
        assert_eq!(ev, vec![(vec![], b"k".to_vec(), b"v".to_vec())]);

        // empty input and whitespace-only input parse cleanly
        assert_eq!(run(b""), (true, vec![]));
        assert_eq!(run(b" \t\n"), (true, vec![]));
    }

    #[test]
    fn escape_sequences() {
        let (ok, ev) = run(br#""a\tb" "q\"w\\e\n\r\'\?\a\b\f\v""#);
        assert!(ok);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].1, b"a\tb".to_vec());
        assert_eq!(ev[0].2, b"q\"w\\e\n\r'?\x07\x08\x0c\x0b".to_vec());

        // unsupported escape aborts
        assert!(!run(br#""a\xb" "v""#).0);
        // backslash at end of buffer: escape switch sees NUL -> abort
        assert!(!run(br#""a\"#).0);
    }

    #[test]
    fn malformed_inputs() {
        // unclosed quote (NUL mid-string)
        assert!(!run(b"\"key").0);
        // key with no value at end of buffer
        assert!(!run(b"\"key\"").0);
        // bare token instead of a quoted key
        assert!(!run(b"key \"v\"").0);
        // bare value
        assert!(!run(b"\"key\" value").0);
        // stray closing brace at top level
        assert!(!run(b"}").0);
        // garbage inside a node
        assert!(!run(b"\"n\" { junk }").0);
        // interior NUL acts as end of buffer, mid-string -> abort
        assert!(!run(b"\"ke\0y\" \"v\"").0);

        // unclosed node at end of buffer is TOLERATED (C returns true)
        let (ok, ev) = run(b"\"n\" { \"k\" \"v\" ");
        assert!(ok);
        assert_eq!(
            ev,
            vec![(vec![b"n".to_vec()], b"k".to_vec(), b"v".to_vec())]
        );

        // ...even several levels deep
        assert!(run(b"\"a\" { \"b\" { \"c\" {").0);

        // an abort after a successful callback still reports failure
        let (ok, ev) = run(b"\"k\" \"v\" \"bad");
        assert!(!ok);
        assert_eq!(ev.len(), 1);
    }

    #[test]
    fn depth_cap() {
        let nest = |n: usize| {
            let mut s = Vec::new();
            for _ in 0..n {
                s.extend_from_slice(b"\"n\" { ");
            }
            s.extend_from_slice(b"\"k\" \"v\" ");
            s.extend_from_slice(&b"} ".repeat(n));
            s
        };
        let (ok, ev) = run(&nest(MAX_DEPTH));
        assert!(ok);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].0.len(), MAX_DEPTH);

        let (ok, ev) = run(&nest(MAX_DEPTH + 1));
        assert!(!ok);
        assert!(ev.is_empty());
    }

    const LIBFOLDERS: &[u8] = br#"
"libraryfolders"
{
	"contentstatsid"		"-8694234255323545402"
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
		"totalsize"		"0"
		"apps"
		{
			"440"		"25664798439"
			"2310"		"64060"
		}
	}
	"1"
	{
		"path"		"/mnt/games/SteamLibrary"
		"apps"
		{
			"9050"		"1148921217"
		}
	}
}
"#;

    #[test]
    fn library_lookup() {
        // escaped Windows path decodes to single backslashes
        assert_eq!(
            find_library_for_app(LIBFOLDERS, 2310),
            Some(b"C:\\Program Files (x86)\\Steam".to_vec())
        );
        assert_eq!(
            find_library_for_app(LIBFOLDERS, 9050),
            Some(b"/mnt/games/SteamLibrary".to_vec())
        );
        assert_eq!(find_library_for_app(LIBFOLDERS, 22200), None);

        // last match wins when the appid is in several libraries
        let both = br#"
"libraryfolders"
{
	"0" { "path" "/first" "apps" { "2310" "1" } }
	"1" { "path" "/second" "apps" { "2310" "1" } }
}
"#;
        assert_eq!(find_library_for_app(both, 2310), Some(b"/second".to_vec()));

        // sscanf "%d" tolerance: signs and trailing garbage still index
        let odd = br#"
"libraryfolders"
{
	"+7abc" { "path" "/odd" "apps" { "2310" "1" } }
	"x1" { "path" "/never" "apps" { "2310" "1" } }
}
"#;
        assert_eq!(find_library_for_app(odd, 2310), Some(b"/odd".to_vec()));

        // wrong root name is ignored entirely
        let wrongroot = br#""LibraryFolders" { "0" { "path" "/p" "apps" { "2310" "1" } } }"#;
        assert_eq!(find_library_for_app(wrongroot, 2310), None);

        // apps hit before any "path" key: C stores result = NULL
        let no_path_yet = br#"
"libraryfolders"
{
	"0" { "apps" { "2310" "1" } "path" "/late" }
}
"#;
        assert_eq!(find_library_for_app(no_path_yet, 2310), None);

        // a match followed by a parse abort is discarded
        let truncated_escape = br#"
"libraryfolders"
{
	"0" { "path" "/p" "apps" { "2310" "1" } }
	"bad" "\z"
}
"#;
        assert_eq!(find_library_for_app(truncated_escape, 2310), None);

        // unclosed final brace is tolerated, match survives
        let unclosed = br#"
"libraryfolders"
{
	"0" { "path" "/p" "apps" { "2310" "1" } }
"#;
        assert_eq!(find_library_for_app(unclosed, 2310), Some(b"/p".to_vec()));
    }

    const ACF: &[u8] = br#"
"AppState"
{
	"appid"		"2310"
	"universe"		"1"
	"name"		"Quake"
	"StateFlags"		"4"
	"installdir"		"Quake"
	"UserConfig"
	{
		"language"		"english"
		"installdir"		"nested-ignored"
	}
}
"#;

    #[test]
    fn acf_lookup() {
        assert_eq!(acf_installdir(ACF), Some(b"Quake".to_vec()));

        // depth and root are both checked (case-sensitive)
        assert_eq!(acf_installdir(br#""appstate" { "installdir" "x" }"#), None);
        assert_eq!(acf_installdir(br#""installdir" "x""#), None);

        // last match wins
        assert_eq!(
            acf_installdir(br#""AppState" { "installdir" "a" "installdir" "b" }"#),
            Some(b"b".to_vec())
        );

        // abort discards the match
        assert_eq!(
            acf_installdir(br#""AppState" { "installdir" "a" } garbage"#),
            None
        );
    }
}
