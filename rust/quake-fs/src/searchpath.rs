//! Searchpath and gamedir decision logic (common_fs.c). The list threading,
//! allocation and file IO stay in the FFI shim; the string semantics that
//! other code observes (gamedir name lists, path_id assignment, mod-name
//! validation) live here.

/// C: `GAMENAME` (quakedef.h)
pub const GAMENAME: &[u8] = b"id1";
/// `com_gamenames` capacity: truncation boundary for q_strlcat.
pub const GAMENAMES_SIZE: usize = 1024;

fn lower(b: u8) -> u8 {
    // q_tolower: ASCII-only, locale-independent
    if b.is_ascii_uppercase() {
        b + 32
    } else {
        b
    }
}

fn eq_ignore_case(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| lower(x) == lower(y))
}

/// C: path_id assignment in COM_AddGameDirectory — the head searchpath's id
/// doubled, 1 for the first game directory. Deliberately no overflow guard,
/// like the C (`<<` on unsigned wraps the same way).
pub fn next_path_id(head_path_id: Option<u32>) -> u32 {
    match head_path_id {
        Some(id) => id << 1,
        None => 1,
    }
}

/// C: `COM_ModForbiddenChars`
pub fn mod_forbidden_chars(p: &[u8]) -> bool {
    fn contains(hay: &[u8], needle: &[u8]) -> bool {
        hay.windows(needle.len()).any(|w| w == needle)
    }
    p.is_empty()
        || p == b"."
        || contains(p, b"..")
        || contains(p, b"/")
        || contains(p, b"\\")
        || contains(p, b":")
        || contains(p, b"\"")
        || contains(p, b";")
}

/// C: `COM_GetGameNames (true)` — "id1" or "id1;<names>".
pub fn full_game_names(gamenames: &[u8]) -> Vec<u8> {
    if gamenames.is_empty() {
        GAMENAME.to_vec()
    } else {
        let mut v = GAMENAME.to_vec();
        v.push(b';');
        v.extend_from_slice(gamenames);
        v
    }
}

/// C: q_strlcat of ";" + dir onto com_gamenames[1024] in COM_AddGameDirectory,
/// with strlcat's truncate-at-capacity behavior.
pub fn append_game_name(gamenames: &mut Vec<u8>, dir: &[u8]) {
    fn add(gamenames: &mut Vec<u8>, bytes: &[u8]) {
        for &b in bytes {
            if gamenames.len() < GAMENAMES_SIZE - 1 {
                gamenames.push(b);
            }
        }
    }
    if !gamenames.is_empty() {
        add(gamenames, b";");
    }
    add(gamenames, dir);
}

fn skip_core(mut dirs: &[u8]) -> &[u8] {
    // ignore a leading GAMENAME component. Case-SENSITIVE: the C strips it
    // with strncmp, and tdirs is server-controlled (MSG_ReadString via
    // cl_parse.c), so "ID1;..." must reach the comparison unstripped
    let gnl = GAMENAME.len();
    if dirs.len() >= gnl && &dirs[..gnl] == GAMENAME && (dirs.len() == gnl || dirs[gnl] == b';') {
        dirs = &dirs[gnl..];
        if dirs.first() == Some(&b';') {
            dirs = &dirs[1..];
        }
    }
    // skip a leading quakeworld "qw" component (case-sensitive in C: strncmp)
    if dirs.starts_with(b"qw;") || dirs == b"qw" {
        dirs = &dirs[2..];
        if dirs.first() == Some(&b';') {
            dirs = &dirs[1..];
        }
    }
    dirs
}

/// C: `COM_GameDirMatches` — compares a requested dir list against the
/// current one, ignoring leading id1/qw components on both sides.
/// The final comparison is case-SENSITIVE strcmp, like the C.
pub fn game_dir_matches(current_gamenames: &[u8], tdirs: &[u8]) -> bool {
    skip_core(current_gamenames) == skip_core(tdirs)
}

/// C: the gamedir-list walk in COM_ResetGameDirectories — ';'-separated
/// tokens, skipping GAMENAME and any token seen earlier in the list
/// (case-insensitive; earlier tokens count even if themselves skipped).
/// Returns the directories to mount, in order.
pub fn parse_new_gamedirs(newdirs: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut seen: Vec<&[u8]> = Vec::new();
    let mut rest = newdirs;
    while !rest.is_empty() {
        let (tok, next) = match rest.iter().position(|&b| b == b';') {
            Some(p) => (&rest[..p], &rest[p + 1..]),
            None => (rest, &rest[rest.len()..]),
        };
        let dup = eq_ignore_case(tok, GAMENAME) || seen.iter().any(|s| eq_ignore_case(s, tok));
        if !dup {
            out.push(tok);
        }
        seen.push(tok);
        rest = next;
        if rest.is_empty() {
            break;
        }
    }
    out
}

/// C: `COM_IsPathPrefix` — case-insensitive, '/' and '\\' equivalent,
/// component-boundary aware.
pub fn is_path_prefix(prefix: &[u8], path: &[u8]) -> bool {
    let norm = |b: u8| lower(if b == b'\\' { b'/' } else { b });
    if path.len() < prefix.len() {
        // C compares byte-wise and hits path's NUL (which can't match a
        // prefix byte; prefix has no interior NUL)
        return false;
    }
    if !prefix.iter().zip(path).all(|(&a, &b)| norm(a) == norm(b)) {
        return false;
    }
    matches!(path.get(prefix.len()), None | Some(&b'/') | Some(&b'\\'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_ids_double() {
        assert_eq!(next_path_id(None), 1);
        assert_eq!(next_path_id(Some(1)), 2);
        assert_eq!(next_path_id(Some(4)), 8);
        // wraps without a guard, like the C
        assert_eq!(next_path_id(Some(0x8000_0000)), 0);
    }

    #[test]
    fn forbidden_chars() {
        assert!(mod_forbidden_chars(b""));
        assert!(mod_forbidden_chars(b"."));
        assert!(mod_forbidden_chars(b".."));
        assert!(mod_forbidden_chars(b"a/b"));
        assert!(mod_forbidden_chars(b"a\\b"));
        assert!(mod_forbidden_chars(b"c:"));
        assert!(mod_forbidden_chars(b"a\"b"));
        assert!(mod_forbidden_chars(b"a;b"));
        assert!(!mod_forbidden_chars(b"hipnotic"));
        assert!(!mod_forbidden_chars(b".hidden")); // single leading dot is allowed
    }

    #[test]
    fn game_names() {
        assert_eq!(full_game_names(b""), b"id1");
        assert_eq!(full_game_names(b"rogue;warp"), b"id1;rogue;warp");

        let mut names = Vec::new();
        append_game_name(&mut names, b"rogue");
        assert_eq!(names, b"rogue");
        append_game_name(&mut names, b"warp");
        assert_eq!(names, b"rogue;warp");

        // strlcat truncation at the 1024-byte buffer
        let mut long = vec![b'x'; GAMENAMES_SIZE - 3];
        append_game_name(&mut long, b"abcdef");
        assert_eq!(long.len(), GAMENAMES_SIZE - 1);
        assert_eq!(&long[GAMENAMES_SIZE - 3..], b";a");
    }

    #[test]
    fn dir_matching() {
        assert!(game_dir_matches(b"", b""));
        assert!(game_dir_matches(b"", b"id1"));
        assert!(game_dir_matches(b"rogue", b"id1;rogue"));
        assert!(game_dir_matches(b"rogue", b"qw;rogue"));
        assert!(game_dir_matches(b"qw;rogue", b"rogue"));
        assert!(!game_dir_matches(b"rogue", b"hipnotic"));
        // final compare is case-sensitive in the C
        assert!(!game_dir_matches(b"Rogue", b"rogue"));
        // ...and so is the leading id1/qw strip (strncmp): a server sending
        // "ID1;rogue" does NOT get the prefix stripped, so it compares
        // unequal to a local "rogue" and the gamedir-mismatch warning fires
        assert!(!game_dir_matches(b"rogue", b"ID1;rogue"));
        assert!(!game_dir_matches(b"ID1;rogue", b"rogue"));
        assert!(!game_dir_matches(b"rogue", b"QW;rogue"));
    }

    #[test]
    fn new_gamedir_parsing() {
        assert_eq!(parse_new_gamedirs(b""), Vec::<&[u8]>::new());
        assert_eq!(parse_new_gamedirs(b"rogue"), vec![&b"rogue"[..]]);
        assert_eq!(parse_new_gamedirs(b"id1;rogue"), vec![&b"rogue"[..]]);
        assert_eq!(
            parse_new_gamedirs(b"rogue;ROGUE;warp"),
            vec![&b"rogue"[..], &b"warp"[..]]
        );
        // a trailing ';' leaves newpath at the terminating NUL: loop exits
        assert_eq!(parse_new_gamedirs(b"rogue;"), vec![&b"rogue"[..]]);
        // but leading/middle empty tokens ARE processed and mounted by the C
        // (*newpath is ';' there, so the loop runs and splits off "")
        assert_eq!(parse_new_gamedirs(b";rogue"), vec![&b""[..], &b"rogue"[..]]);
        // ...and deduped like any other token on repeat
        assert_eq!(
            parse_new_gamedirs(b"a;;b;;c"),
            vec![&b"a"[..], &b""[..], &b"b"[..], &b"c"[..]]
        );
    }

    #[test]
    fn path_prefix() {
        assert!(is_path_prefix(
            b"C:/Games/Steam",
            b"C:\\games\\steam\\steamapps"
        ));
        assert!(is_path_prefix(b"/opt/quake", b"/opt/quake"));
        assert!(!is_path_prefix(b"/opt/quake", b"/opt/quake2"));
        assert!(!is_path_prefix(b"/opt/quake/id1", b"/opt/quake"));
        assert!(is_path_prefix(b"", b"/anything"));
    }
}
