//! Differential tests: the Rust hash_map (through the quake-capi shims — the
//! exact C ABI the engine links) vs the original hash_map.c compiled as
//! c_ref_*. Both sides are driven through identical operation sequences and
//! compared after every op: return values, size, lookups over the whole key
//! space, and the full index-ordered key/value iteration (which is observable
//! and must match, including erase's swap-remove reordering).

use core::ffi::{c_int, c_void};
use proptest::prelude::*;
use quake_ctest as _; // links the cc-built c_ref_* archive

#[allow(non_camel_case_types)]
type c_hash_map = c_void;

extern "C" {
    fn c_ref_HashMap_CreateImpl(
        key_size: u32,
        value_size: u32,
        hasher: Option<unsafe extern "C" fn(*const c_void) -> u32>,
        comp: Option<unsafe extern "C" fn(*const c_void, *const c_void) -> bool>,
    ) -> *mut c_hash_map;
    fn c_ref_HashMap_Destroy(map: *mut c_hash_map);
    fn c_ref_HashMap_Reserve(map: *mut c_hash_map, capacity: c_int);
    fn c_ref_HashMap_Clear(map: *mut c_hash_map);
    fn c_ref_HashMap_InsertImpl(
        map: *mut c_hash_map,
        key_size: u32,
        value_size: u32,
        key: *const c_void,
        value: *const c_void,
    ) -> bool;
    fn c_ref_HashMap_EraseImpl(map: *mut c_hash_map, key_size: u32, key: *const c_void) -> bool;
    fn c_ref_HashMap_LookupImpl(
        map: *mut c_hash_map,
        key_size: u32,
        key: *const c_void,
    ) -> *mut c_void;
    fn c_ref_HashMap_Size(map: *mut c_hash_map) -> u32;
    fn c_ref_HashMap_GetKeyImpl(map: *mut c_hash_map, index: u32) -> *mut c_void;
    fn c_ref_HashMap_GetValueImpl(map: *mut c_hash_map, index: u32) -> *mut c_void;

    fn c_ref_HashInt32(val: *const c_void) -> u32;
    fn c_ref_HashInt64(val: *const c_void) -> u32;
    fn c_ref_HashFloat(val: *const c_void) -> u32;
    fn c_ref_HashCombine(a: u32, b: u32) -> u32;
    fn c_ref_HashVec2(val: *const c_void) -> u32;
    fn c_ref_HashVec3(val: *const c_void) -> u32;
    fn c_ref_HashStr(val: *const c_void) -> u32;
}

unsafe extern "C" fn collide_hasher(_val: *const c_void) -> u32 {
    7
}

#[derive(Debug, Clone)]
enum Op {
    Insert(u32, u64),
    Erase(u32),
    Lookup,
    Clear,
    Reserve(i32),
}

fn op_strategy() -> impl Strategy<Value = Op> {
    // a small key space forces duplicate inserts, collisions and erase hits
    prop_oneof![
        4 => (0u32..48, any::<u64>()).prop_map(|(k, v)| Op::Insert(k, v)),
        2 => (0u32..48).prop_map(Op::Erase),
        2 => Just(Op::Lookup),
        1 => Just(Op::Clear),
        1 => (0i32..96).prop_map(Op::Reserve),
    ]
}

/// Drives identical op sequences through the C reference map and the Rust
/// shim map, asserting identical observable state throughout.
fn run_differential(
    ops: &[Op],
    hasher_c: unsafe extern "C" fn(*const c_void) -> u32,
) -> Result<(), TestCaseError> {
    // SAFETY: both maps are used with consistent key/value sizes and
    // destroyed at the end; all key/value pointers are to live locals
    unsafe {
        let c_map = c_ref_HashMap_CreateImpl(4, 8, Some(hasher_c), None);
        let r_map = quake_rs::hash_map::HashMap_CreateImpl(4, 8, Some(hasher_c), None);

        for op in ops {
            match *op {
                Op::Insert(k, v) => {
                    let c = c_ref_HashMap_InsertImpl(
                        c_map,
                        4,
                        8,
                        &k as *const u32 as *const c_void,
                        &v as *const u64 as *const c_void,
                    );
                    let r = quake_rs::hash_map::HashMap_InsertImpl(
                        r_map,
                        4,
                        8,
                        &k as *const u32 as *const c_void,
                        &v as *const u64 as *const c_void,
                    );
                    prop_assert_eq!(c, r, "insert({}, {}) return", k, v);
                }
                Op::Erase(k) => {
                    let c = c_ref_HashMap_EraseImpl(c_map, 4, &k as *const u32 as *const c_void);
                    let r = quake_rs::hash_map::HashMap_EraseImpl(
                        r_map,
                        4,
                        &k as *const u32 as *const c_void,
                    );
                    prop_assert_eq!(c, r, "erase({}) return", k);
                }
                Op::Lookup | Op::Clear | Op::Reserve(_) => match *op {
                    Op::Clear => {
                        c_ref_HashMap_Clear(c_map);
                        quake_rs::hash_map::HashMap_Clear(r_map);
                    }
                    Op::Reserve(cap) => {
                        c_ref_HashMap_Reserve(c_map, cap);
                        quake_rs::hash_map::HashMap_Reserve(r_map, cap);
                    }
                    _ => {}
                },
            }

            // full observable-state comparison after every op
            let c_size = c_ref_HashMap_Size(c_map);
            let r_size = quake_rs::hash_map::HashMap_Size(r_map);
            prop_assert_eq!(c_size, r_size, "size after {:?}", op);

            for k in 0u32..48 {
                let c_val = c_ref_HashMap_LookupImpl(c_map, 4, &k as *const u32 as *const c_void);
                let r_val = quake_rs::hash_map::HashMap_LookupImpl(
                    r_map,
                    4,
                    &k as *const u32 as *const c_void,
                );
                prop_assert_eq!(
                    c_val.is_null(),
                    r_val.is_null(),
                    "lookup({}) presence after {:?}",
                    k,
                    op
                );
                if !c_val.is_null() {
                    prop_assert_eq!(
                        *(c_val as *const u64),
                        *(r_val as *const u64),
                        "lookup({}) value after {:?}",
                        k,
                        op
                    );
                }
            }

            // index-ordered iteration must match exactly (observable via
            // HashMap_GetKey/GetValue, incl. erase's swap-remove order)
            for i in 0..c_size {
                let ck = *(c_ref_HashMap_GetKeyImpl(c_map, i) as *const u32);
                let rk = *(quake_rs::hash_map::HashMap_GetKeyImpl(r_map, i) as *const u32);
                prop_assert_eq!(ck, rk, "key at index {} after {:?}", i, op);
                let cv = *(c_ref_HashMap_GetValueImpl(c_map, i) as *const u64);
                let rv = *(quake_rs::hash_map::HashMap_GetValueImpl(r_map, i) as *const u64);
                prop_assert_eq!(cv, rv, "value at index {} after {:?}", i, op);
            }
        }

        c_ref_HashMap_Destroy(c_map);
        quake_rs::hash_map::HashMap_Destroy(r_map);
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn op_sequences_match(ops in proptest::collection::vec(op_strategy(), 1..120)) {
        run_differential(&ops, c_ref_HashInt32)?;
    }

    #[test]
    fn op_sequences_match_all_collisions(ops in proptest::collection::vec(op_strategy(), 1..80)) {
        // a degenerate hasher chains everything in one bucket, stressing the
        // chain-order semantics
        run_differential(&ops, collide_hasher)?;
    }

    #[test]
    fn hashers_match(
        i32v in any::<u32>(),
        i64v in any::<u64>(),
        f in any::<f32>(),
        a in any::<u32>(),
        b in any::<u32>(),
        v2 in [any::<f32>(); 2],
        v3 in [any::<f32>(); 3],
        s in proptest::collection::vec(1u8..=255, 0..64),
    ) {
        use quake_util::hash_map::hashers;
        // SAFETY: pointers to live locals with the sizes each hasher reads
        unsafe {
            prop_assert_eq!(hashers::hash_int32(i32v), c_ref_HashInt32(&i32v as *const u32 as *const c_void));
            prop_assert_eq!(hashers::hash_int64(i64v), c_ref_HashInt64(&i64v as *const u64 as *const c_void));
            prop_assert_eq!(hashers::hash_float(f), c_ref_HashFloat(&f as *const f32 as *const c_void));
            prop_assert_eq!(hashers::hash_combine(a, b), c_ref_HashCombine(a, b));
            prop_assert_eq!(hashers::hash_vec2(&v2), c_ref_HashVec2(v2.as_ptr() as *const c_void));
            prop_assert_eq!(hashers::hash_vec3(&v3), c_ref_HashVec3(v3.as_ptr() as *const c_void));

            let mut s_z = s.clone();
            s_z.push(0);
            let ptr = s_z.as_ptr();
            prop_assert_eq!(
                hashers::hash_str(&s),
                c_ref_HashStr(&ptr as *const *const u8 as *const c_void)
            );
        }
    }
}

// string-keyed maps with HashStr/HashStrCmp: duplicates by content behind
// distinct pointers, the exact shape pr_edict's symbol maps use
#[test]
fn string_keys_first_match_semantics() {
    extern "C" {
        fn c_ref_HashStrCmp(a: *const c_void, b: *const c_void) -> bool;
        fn c_ref_HashStr(val: *const c_void) -> u32;
    }

    let strings: Vec<std::ffi::CString> = ["alpha", "beta", "alpha", "gamma", "beta", "alpha"]
        .iter()
        .map(|s| std::ffi::CString::new(*s).unwrap())
        .collect();
    let ptrs: Vec<*const i8> = strings.iter().map(|s| s.as_ptr()).collect();

    // SAFETY: maps hold char* keys pointing at the CStrings above, which
    // outlive both maps; hasher/comp are the C reference implementations
    unsafe {
        let c_map = c_ref_HashMap_CreateImpl(8, 4, Some(c_ref_HashStr), Some(c_ref_HashStrCmp));
        let r_map = quake_rs::hash_map::HashMap_CreateImpl(
            8,
            4,
            Some(c_ref_HashStr),
            Some(c_ref_HashStrCmp),
        );

        // reverse insertion, like PR_LoadProgs building its symbol maps
        for (i, p) in ptrs.iter().enumerate().rev() {
            let v = i as i32;
            let c = c_ref_HashMap_InsertImpl(
                c_map,
                8,
                4,
                p as *const *const i8 as *const c_void,
                &v as *const i32 as *const c_void,
            );
            let r = quake_rs::hash_map::HashMap_InsertImpl(
                r_map,
                8,
                4,
                p as *const *const i8 as *const c_void,
                &v as *const i32 as *const c_void,
            );
            assert_eq!(c, r, "insert of index {i}");
        }

        for p in &ptrs {
            let c = c_ref_HashMap_LookupImpl(c_map, 8, p as *const *const i8 as *const c_void);
            let r = quake_rs::hash_map::HashMap_LookupImpl(
                r_map,
                8,
                p as *const *const i8 as *const c_void,
            );
            assert!(!c.is_null() && !r.is_null());
            assert_eq!(*(c as *const i32), *(r as *const i32));
        }

        c_ref_HashMap_Destroy(c_map);
        quake_rs::hash_map::HashMap_Destroy(r_map);
    }
}
