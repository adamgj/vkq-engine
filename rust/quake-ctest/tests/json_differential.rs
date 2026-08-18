//! Differential tests: the Rust JSON_Parse/JSON_Find (quake-capi shims) vs
//! the original json.c compiled as c_ref_*. Both trees are serialized to a
//! canonical form and compared; acceptance (NULL vs non-NULL) must match on
//! every input (ADR-012: jsmn's tolerant behavior is the spec).
// The c_ref_* symbols are compiled C (build.rs), which Miri cannot execute;
// the shims themselves get Miri coverage in miri_capi.rs instead.
#![cfg(not(miri))]

use core::ffi::{c_char, c_int, CStr};
use proptest::prelude::*;
use quake_ctest as _; // links the cc-built c_ref_* archive
use quake_types::json::{
    Json, JsonEntry, JSON_BOOLEAN, JSON_NULL, JSON_NUMBER, JSON_OBJECT, JSON_STRING,
};

extern "C" {
    fn c_ref_JSON_Parse(text: *const c_char) -> *mut Json;
    fn c_ref_JSON_Free(json: *mut Json);
    fn c_ref_JSON_Find(
        entry: *const JsonEntry,
        name: *const c_char,
        type_: c_int,
    ) -> *const JsonEntry;
    fn c_ref_JSON_FindString(entry: *const JsonEntry, name: *const c_char) -> *const c_char;
    fn c_ref_JSON_FindNumber(entry: *const JsonEntry, name: *const c_char) -> *const f64;
    fn c_ref_JSON_FindBoolean(entry: *const JsonEntry, name: *const c_char) -> *const bool;
}

/// Canonical serialization of a parse tree (both sides share the
/// quake-types layout, which its const asserts pin to the C layout).
fn serialize(entry: *const JsonEntry, out: &mut String) {
    if entry.is_null() {
        out.push('ـ');
        return;
    }
    // SAFETY: valid tree node from JSON_Parse; union read is gated on type_
    unsafe {
        let e = &*entry;
        match e.type_ {
            t if t == JSON_STRING => {
                let s = if e.value.string.is_null() {
                    b"<null>".as_slice()
                } else {
                    CStr::from_ptr(e.value.string).to_bytes()
                };
                out.push_str(&format!("s{:?}", s));
            }
            t if t == JSON_NUMBER => out.push_str(&format!("n{:016x}", e.value.number.to_bits())),
            t if t == JSON_BOOLEAN => out.push_str(if e.value.boolean { "bt" } else { "bf" }),
            t if t == JSON_NULL => out.push('z'),
            t if t == JSON_OBJECT => out.push('o'),
            2 => out.push('a'), // JSON_ARRAY
            _ => out.push('i'), // JSON_INVALID
        }
        out.push('(');
        let mut child = e.firstchild;
        while !child.is_null() {
            serialize(child, out);
            out.push(',');
            child = (*child).next;
        }
        out.push(')');
    }
}

fn compare_parse(input: &[u8]) -> Result<(), TestCaseError> {
    let mut z = input.to_vec();
    z.push(0);
    // SAFETY: NUL-terminated inputs; trees freed after comparison
    unsafe {
        let c = c_ref_JSON_Parse(z.as_ptr() as *const c_char);
        let r = quake_rs::json::JSON_Parse(z.as_ptr() as *const c_char);
        prop_assert_eq!(
            c.is_null(),
            r.is_null(),
            "acceptance mismatch for {:?}",
            String::from_utf8_lossy(input)
        );
        if !c.is_null() {
            prop_assert_eq!(
                (*c).numentries,
                (*r).numentries,
                "entry count for {:?}",
                String::from_utf8_lossy(input)
            );
            let mut cs = String::new();
            serialize((*c).root, &mut cs);
            let mut rs = String::new();
            serialize((*r).root, &mut rs);
            prop_assert_eq!(
                cs,
                rs,
                "tree mismatch for {:?}",
                String::from_utf8_lossy(input)
            );
            c_ref_JSON_Free(c);
            quake_rs::json::JSON_Free(r);
        }
    }
    Ok(())
}

#[test]
fn corpus_matches() {
    let corpus: &[&[u8]] = &[
        b"{}",
        b"[]",
        b"{\"a\":1}",
        b"{\"a\": 1, \"b\": [true, false, null], \"c\": \"x\\ny\"}",
        b"{\"nested\": {\"deep\": {\"deeper\": [1,2,3]}}}",
        b"\"top level string\"",
        b"42",
        b"-17.5e3",
        b"0x1A",
        b"infinity",
        b"nan",
        b"true",
        b"false",
        b"null",
        b"bareword",
        b"{key: value}",
        b"{\"dup\": 1, \"dup\": \"two\", \"dup\": true}",
        b"[1, [2, [3, [4]]]]",
        b"{\"esc\": \"\\u0041\\uD83D\\uDE00\\uD800\\uFFFD\\q\\\"",
        b"{\"esc\": \"tab\\there \\u0041 pair \\uD83D\\uDE00 lone \\uD800 bad \\uZZZZ\"}",
        b"\xEF\xBB\xBF{\"bom\": true}",
        b"  \t\r\n  {\"ws\": 0}  ",
        b"{\"a\":}",
        b"{,}",
        b"[,]",
        b"[1 2 3]",
        b"{\"a\" \"b\"}",
        b"}",
        b"]",
        b"{]",
        b"[}",
        b"\"unterminated",
        b"{\"x\": \"trailing backslash\\\\\"}",
        b"{\"mapdb\": {\"episodes\": [{\"name\": \"ep1\", \"maps\": [\"e1m1\"]}]}}",
        b"{\"InstallationList\": [{\"InstallLocation\": \"C:\\\\Games\\\\Quake\", \"AppName\": \"quake\"}]}",
        b"1e999",
        b"-1e-999",
        b"0.00000000000000000000000000001",
        b"{\"n\": 179769313486231570000000000000000000000000000000000000000000000}",
    ];
    for input in corpus {
        compare_parse(input).unwrap();
    }
}

#[test]
fn find_matches() {
    let doc = b"{\"s\": \"str\", \"n\": 3.5, \"b\": true, \"dup\": 1, \"dup\": \"x\", \"obj\": {\"inner\": 1}}\0";
    // SAFETY: NUL-terminated doc; both parses freed at the end
    unsafe {
        let c = c_ref_JSON_Parse(doc.as_ptr() as *const c_char);
        let r = quake_rs::json::JSON_Parse(doc.as_ptr() as *const c_char);
        assert!(!c.is_null() && !r.is_null());

        for name in [c"s", c"n", c"b", c"dup", c"obj", c"missing"] {
            let cs = c_ref_JSON_FindString((*c).root, name.as_ptr());
            let rs = quake_rs::json::JSON_FindString((*r).root, name.as_ptr());
            assert_eq!(cs.is_null(), rs.is_null(), "FindString({name:?})");
            if !cs.is_null() {
                assert_eq!(
                    CStr::from_ptr(cs),
                    CStr::from_ptr(rs),
                    "FindString({name:?})"
                );
            }

            let cn = c_ref_JSON_FindNumber((*c).root, name.as_ptr());
            let rn = quake_rs::json::JSON_FindNumber((*r).root, name.as_ptr());
            assert_eq!(cn.is_null(), rn.is_null(), "FindNumber({name:?})");
            if !cn.is_null() {
                assert_eq!((*cn).to_bits(), (*rn).to_bits(), "FindNumber({name:?})");
            }

            let cb = c_ref_JSON_FindBoolean((*c).root, name.as_ptr());
            let rb = quake_rs::json::JSON_FindBoolean((*r).root, name.as_ptr());
            assert_eq!(cb.is_null(), rb.is_null(), "FindBoolean({name:?})");
            if !cb.is_null() {
                assert_eq!(*cb, *rb, "FindBoolean({name:?})");
            }

            // duplicate-key: first-of-right-type
            let cf = c_ref_JSON_Find((*c).root, name.as_ptr(), JSON_STRING);
            let rf = quake_rs::json::JSON_Find((*r).root, name.as_ptr(), JSON_STRING);
            assert_eq!(cf.is_null(), rf.is_null(), "Find({name:?}, STRING)");
        }

        c_ref_JSON_Free(c);
        quake_rs::json::JSON_Free(r);
    }
}

/// JSON-ish text generator: mixes structured documents and arbitrary noise
fn jsonish() -> impl Strategy<Value = Vec<u8>> {
    let scalar = prop_oneof![
        Just("1".to_string()),
        Just("-2.5e10".to_string()),
        Just("true".to_string()),
        Just("false".to_string()),
        Just("null".to_string()),
        Just("bare".to_string()),
        "[a-z]{0,6}",
        Just("\"str\\\\ing\"".to_string()),
        Just("\"\\u0041\\uD83D\\uDE00\"".to_string()),
    ];
    let doc = scalar.prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..4)
                .prop_map(|items| format!("[{}]", items.join(","))),
            proptest::collection::vec(("[a-z]{1,4}", inner), 0..4).prop_map(|kvs| {
                let body: Vec<String> = kvs
                    .into_iter()
                    .map(|(k, v)| format!("\"{k}\":{v}"))
                    .collect();
                format!("{{{}}}", body.join(","))
            }),
        ]
    });
    prop_oneof![
        4 => doc.prop_map(|s| s.into_bytes()),
        // arbitrary NUL-free noise: acceptance must still match
        1 => proptest::collection::vec(1u8..=255, 0..64),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]

    #[test]
    fn random_inputs_match(input in jsonish()) {
        compare_parse(&input)?;
    }
}
