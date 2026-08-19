//! Port of the EGS manifest matching in `Quake/steam.c` (EGS_FindGame):
//! phase 1 scans LauncherInstalled.dat's InstallationList, phase 2 checks a
//! single *.item manifest (the directory walk and file IO stay in the shim).
//!
//! JSON navigation reproduces json.c's JSON_Find/JSON_FindString/
//! JSON_FindBoolean over the Phase-1 `quake_util::json` parse: a key must be
//! a string entry with a child, and a duplicate key whose value has the
//! wrong type is skipped ("keep looking in case of duplicate keys").

use quake_util::json::{parse, EntryData, ParsedJson};

/// C strings stop at the first NUL (strcmp semantics for the parameters)
fn c_bytes(s: &[u8]) -> &[u8] {
    &s[..s.iter().position(|&b| b == 0).unwrap_or(s.len())]
}

struct Doc {
    parsed: ParsedJson,
    children: Vec<Vec<usize>>,
}

impl Doc {
    /// `JSON_Parse` + the firstchild/next links; entry 0 is `json->root`
    fn parse(text: &[u8]) -> Option<Doc> {
        let parsed = parse(text)?;
        let mut children = vec![Vec::new(); parsed.entries.len()];
        for (i, e) in parsed.entries.iter().enumerate() {
            if let Some(parent) = e.parent {
                children[parent].push(i);
            }
        }
        Some(Doc { parsed, children })
    }

    fn c_str(&self, offset: usize) -> &[u8] {
        c_bytes(&self.parsed.strings[offset..])
    }

    /// `JSON_Find`. The C skips children with a NULL `string` (everything
    /// but string keys, given zeroed entries) and children without a child;
    /// non-string keys with children would read union garbage in C (UB), so
    /// only string keys can ever match.
    fn find(&self, entry: usize, name: &[u8], want: fn(&EntryData) -> bool) -> Option<usize> {
        for &child in &self.children[entry] {
            let EntryData::String(offset) = self.parsed.entries[child].data else {
                continue;
            };
            let Some(&value) = self.children[child].first() else {
                continue;
            };
            if self.c_str(offset) != name {
                continue;
            }
            if !want(&self.parsed.entries[value].data) {
                continue; // keep looking in case of duplicate keys
            }
            return Some(value);
        }
        None
    }

    /// `JSON_FindString`
    fn find_string(&self, entry: usize, name: &[u8]) -> Option<&[u8]> {
        let value = self.find(entry, name, |d| matches!(d, EntryData::String(_)))?;
        match self.parsed.entries[value].data {
            EntryData::String(offset) => Some(self.c_str(offset)),
            _ => unreachable!(),
        }
    }

    /// `JSON_FindBoolean`
    fn find_boolean(&self, entry: usize, name: &[u8]) -> Option<bool> {
        let value = self.find(entry, name, |d| matches!(d, EntryData::Boolean(_)))?;
        match self.parsed.entries[value].data {
            EntryData::Boolean(b) => Some(b),
            _ => unreachable!(),
        }
    }
}

/// EGS_FindGame phase 1: LauncherInstalled.dat. Walks InstallationList in
/// order and returns the first entry whose NamespaceId/ItemId/AppName all
/// match and whose InstallLocation is present and non-empty.
pub fn find_in_launcher_data(
    text: &[u8],
    nspace: &[u8],
    itemid: &[u8],
    appname: &[u8],
) -> Option<Vec<u8>> {
    let (nspace, itemid, appname) = (c_bytes(nspace), c_bytes(itemid), c_bytes(appname));
    let doc = Doc::parse(text)?;
    let list = doc.find(0, b"InstallationList", |d| matches!(d, EntryData::Array))?;
    for &item in &doc.children[list] {
        let cur_nspace = doc.find_string(item, b"NamespaceId");
        let cur_itemid = doc.find_string(item, b"ItemId");
        let cur_appname = doc.find_string(item, b"AppName");
        let location = doc.find_string(item, b"InstallLocation");

        if let (Some(location), Some(cur_nspace), Some(cur_itemid), Some(cur_appname)) =
            (location, cur_nspace, cur_itemid, cur_appname)
        {
            // C: `location && *location && ... && strcmp (...) == 0`
            if !location.is_empty()
                && cur_nspace == nspace
                && cur_itemid == itemid
                && cur_appname == appname
            {
                return Some(location.to_vec());
            }
        }
    }
    None
}

/// EGS_FindGame phase 2, one *.item manifest: CatalogNamespace/
/// CatalogItemId/AppName must match, InstallLocation must be present and
/// non-empty, and bIsIncompleteInstall must be absent, non-boolean, or false
/// (C: `!incomplete || !*incomplete`).
pub fn manifest_matches(
    text: &[u8],
    nspace: &[u8],
    itemid: &[u8],
    appname: &[u8],
) -> Option<Vec<u8>> {
    let (nspace, itemid, appname) = (c_bytes(nspace), c_bytes(itemid), c_bytes(appname));
    let doc = Doc::parse(text)?;
    let cur_nspace = doc.find_string(0, b"CatalogNamespace");
    let cur_itemid = doc.find_string(0, b"CatalogItemId");
    let cur_appname = doc.find_string(0, b"AppName");
    let location = doc.find_string(0, b"InstallLocation");
    let incomplete = doc.find_boolean(0, b"bIsIncompleteInstall");

    match (location, cur_nspace, cur_itemid, cur_appname) {
        (Some(location), Some(cur_nspace), Some(cur_itemid), Some(cur_appname))
            if !location.is_empty()
                && incomplete != Some(true)
                && cur_nspace == nspace
                && cur_itemid == itemid
                && cur_appname == appname =>
        {
            Some(location.to_vec())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAUNCHER: &[u8] = br#"
{
	"InstallationList": [
		{
			"InstallLocation": "C:\\Games\\OtherGame",
			"NamespaceId": "othernspace",
			"ItemId": "otheritem",
			"ArtifactId": "OtherArtifact",
			"AppVersion": "1.0.2",
			"AppName": "OtherApp"
		},
		{
			"InstallLocation": "C:\\Games\\Quake",
			"NamespaceId": "quakenspace",
			"ItemId": "quakeitem",
			"ArtifactId": "QuakeArtifact",
			"AppVersion": "1.0.0",
			"AppName": "QuakeApp"
		}
	]
}
"#;

    #[test]
    fn launcher_data_matching() {
        assert_eq!(
            find_in_launcher_data(LAUNCHER, b"quakenspace", b"quakeitem", b"QuakeApp"),
            Some(b"C:\\Games\\Quake".to_vec())
        );
        // every field must match
        assert_eq!(
            find_in_launcher_data(LAUNCHER, b"quakenspace", b"quakeitem", b"WrongApp"),
            None
        );
        assert_eq!(
            find_in_launcher_data(LAUNCHER, b"wrong", b"quakeitem", b"QuakeApp"),
            None
        );
        // missing list, wrong list type, malformed JSON
        assert_eq!(find_in_launcher_data(b"{}", b"n", b"i", b"a"), None);
        assert_eq!(
            find_in_launcher_data(br#"{"InstallationList": "nope"}"#, b"n", b"i", b"a"),
            None
        );
        assert_eq!(
            find_in_launcher_data(b"{\"InstallationList\": [", b"n", b"i", b"a"),
            None
        );
    }

    #[test]
    fn launcher_data_edge_cases() {
        // empty InstallLocation is rejected (`location && *location`)
        let empty_loc = br#"
{
	"InstallationList": [
		{"InstallLocation": "", "NamespaceId": "n", "ItemId": "i", "AppName": "a"}
	]
}
"#;
        assert_eq!(find_in_launcher_data(empty_loc, b"n", b"i", b"a"), None);

        // an item missing a field is skipped, not fatal
        let partial = br#"
{
	"InstallationList": [
		{"InstallLocation": "/x", "NamespaceId": "n", "ItemId": "i"},
		{"InstallLocation": "/y", "NamespaceId": "n", "ItemId": "i", "AppName": "a"}
	]
}
"#;
        assert_eq!(
            find_in_launcher_data(partial, b"n", b"i", b"a"),
            Some(b"/y".to_vec())
        );

        // first match wins
        let dup = br#"
{
	"InstallationList": [
		{"InstallLocation": "/first", "NamespaceId": "n", "ItemId": "i", "AppName": "a"},
		{"InstallLocation": "/second", "NamespaceId": "n", "ItemId": "i", "AppName": "a"}
	]
}
"#;
        assert_eq!(
            find_in_launcher_data(dup, b"n", b"i", b"a"),
            Some(b"/first".to_vec())
        );

        // duplicate key with wrong type is skipped by JSON_Find
        let dupkey = br#"
{
	"InstallationList": [
		{"InstallLocation": 5, "InstallLocation": "/typed",
		 "NamespaceId": "n", "ItemId": "i", "AppName": "a"}
	]
}
"#;
        assert_eq!(
            find_in_launcher_data(dupkey, b"n", b"i", b"a"),
            Some(b"/typed".to_vec())
        );

        // non-object entries in the list are skipped
        let mixed = br#"
{
	"InstallationList": [
		"junk", 42, null,
		{"InstallLocation": "/z", "NamespaceId": "n", "ItemId": "i", "AppName": "a"}
	]
}
"#;
        assert_eq!(
            find_in_launcher_data(mixed, b"n", b"i", b"a"),
            Some(b"/z".to_vec())
        );
    }

    fn item(incomplete: &str) -> Vec<u8> {
        format!(
            r#"
{{
	"FormatVersion": 0,
	"bIsIncompleteInstall": {},
	"AppVersionString": "1.0.0",
	"LaunchExecutable": "quake.exe",
	"InstallLocation": "D:\\Epic\\Quake",
	"AppName": "QuakeApp",
	"CatalogNamespace": "quakenspace",
	"CatalogItemId": "quakeitem"
}}
"#,
            incomplete
        )
        .into_bytes()
    }

    #[test]
    fn item_manifest_matching() {
        let expect = Some(b"D:\\Epic\\Quake".to_vec());

        // bIsIncompleteInstall false or absent matches; true does not
        assert_eq!(
            manifest_matches(&item("false"), b"quakenspace", b"quakeitem", b"QuakeApp"),
            expect
        );
        assert_eq!(
            manifest_matches(&item("true"), b"quakenspace", b"quakeitem", b"QuakeApp"),
            None
        );
        let absent = br#"
{
	"InstallLocation": "D:\\Epic\\Quake",
	"AppName": "QuakeApp",
	"CatalogNamespace": "quakenspace",
	"CatalogItemId": "quakeitem"
}
"#;
        assert_eq!(
            manifest_matches(absent, b"quakenspace", b"quakeitem", b"QuakeApp"),
            expect
        );
        // a non-boolean value means JSON_FindBoolean returns NULL -> matches
        assert_eq!(
            manifest_matches(&item("\"true\""), b"quakenspace", b"quakeitem", b"QuakeApp"),
            expect
        );

        // field mismatches
        assert_eq!(
            manifest_matches(&item("false"), b"othernspace", b"quakeitem", b"QuakeApp"),
            None
        );
        assert_eq!(
            manifest_matches(&item("false"), b"quakenspace", b"otheritem", b"QuakeApp"),
            None
        );
        assert_eq!(
            manifest_matches(&item("false"), b"quakenspace", b"quakeitem", b"OtherApp"),
            None
        );

        // empty or missing InstallLocation
        let empty_loc = br#"
{
	"InstallLocation": "",
	"AppName": "a", "CatalogNamespace": "n", "CatalogItemId": "i"
}
"#;
        assert_eq!(manifest_matches(empty_loc, b"n", b"i", b"a"), None);
        let no_loc = br#"{"AppName": "a", "CatalogNamespace": "n", "CatalogItemId": "i"}"#;
        assert_eq!(manifest_matches(no_loc, b"n", b"i", b"a"), None);

        // malformed JSON
        assert_eq!(manifest_matches(b"{", b"n", b"i", b"a"), None);
        assert_eq!(manifest_matches(b"", b"n", b"i", b"a"), None);
    }
}
