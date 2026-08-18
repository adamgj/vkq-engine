//! C ABI shims for `Quake/hash_map.c` (declarations stay in `Quake/hash_map.h`;
//! the header's static-inline hashers survive for C callers).
//!
//! `hash_map_t` is opaque to C, so the map's internals are Rust-owned
//! (`HashMap_Destroy` is the only free path — no cross-boundary buffer
//! ownership, consistent with ADR-013). Lookup/GetKey/GetValue return raw
//! pointers into the map's storage exactly like the C did: valid until the
//! next insert/reserve reallocates, which is the same contract C callers
//! already live with. The storage base comes from Vec<u8> (malloc), which is
//! at least 16-aligned in practice — the same alignment Mem_Alloc gave the C
//! key/value arrays, so C hashers reading pointer-sized keys behave
//! identically.

use core::ffi::{c_int, c_void};
use quake_util::hash_map::{Comp, Hasher, QHashMap};

type CHasher = unsafe extern "C" fn(*const c_void) -> u32;
type CComp = unsafe extern "C" fn(*const c_void, *const c_void) -> bool;

pub struct FfiHashMap {
    core: QHashMap,
}

/// C: `hash_map_t *HashMap_CreateImpl (const uint32_t key_size, const uint32_t
/// value_size, uint32_t (*hasher) (const void *const), qboolean (*comp) (...));`
///
/// # Safety
/// `hasher` must be a valid function pointer (C dereferences it without a
/// NULL check too); `comp` may be NULL (byte-wise comparison, like C memcmp).
#[no_mangle]
pub unsafe extern "C" fn HashMap_CreateImpl(
    key_size: u32,
    value_size: u32,
    hasher: Option<CHasher>,
    comp: Option<CComp>,
) -> *mut FfiHashMap {
    let hasher = hasher.expect("HashMap_CreateImpl: NULL hasher");
    // SAFETY: the hasher contract (hash_map.h) is a pure function over the
    // key bytes; the slice pointer is a valid key slot
    let hasher_box: Hasher =
        Box::new(move |key: &[u8]| unsafe { hasher(key.as_ptr() as *const c_void) });
    let comp_box: Option<Comp> = comp.map(|comp| -> Comp {
        // SAFETY: same contract for the comparator
        Box::new(move |a: &[u8], b: &[u8]| unsafe {
            comp(a.as_ptr() as *const c_void, b.as_ptr() as *const c_void)
        })
    });
    Box::into_raw(Box::new(FfiHashMap {
        core: QHashMap::new(key_size, value_size, hasher_box, comp_box),
    }))
}

/// # Safety
/// `map` must be a live pointer from HashMap_CreateImpl; not used afterwards.
#[no_mangle]
pub unsafe extern "C" fn HashMap_Destroy(map: *mut FfiHashMap) {
    // SAFETY: map came from Box::into_raw in HashMap_CreateImpl
    drop(unsafe { Box::from_raw(map) });
}

/// # Safety
/// `map` must be a live pointer from HashMap_CreateImpl.
#[no_mangle]
pub unsafe extern "C" fn HashMap_Reserve(map: *mut FfiHashMap, capacity: c_int) {
    // SAFETY: live map per the caller contract
    unsafe { &mut *map }.core.reserve(capacity);
}

/// # Safety
/// `map` must be a live pointer from HashMap_CreateImpl.
#[no_mangle]
pub unsafe extern "C" fn HashMap_Clear(map: *mut FfiHashMap) {
    // SAFETY: live map per the caller contract
    unsafe { &mut *map }.core.clear();
}

/// # Safety
/// `key`/`value` must point to key_size/value_size readable bytes matching
/// the map's configured sizes.
#[no_mangle]
pub unsafe extern "C" fn HashMap_InsertImpl(
    map: *mut FfiHashMap,
    key_size: u32,
    value_size: u32,
    key: *const c_void,
    value: *const c_void,
) -> bool {
    // SAFETY: live map; key/value sizes match per the hash_map.h macros
    let m = unsafe { &mut *map };
    debug_assert_eq!(m.core.key_size(), key_size);
    debug_assert_eq!(m.core.value_size(), value_size);
    // SAFETY: caller passes key_size/value_size readable bytes
    let (k, v) = unsafe {
        (
            core::slice::from_raw_parts(key as *const u8, key_size as usize),
            core::slice::from_raw_parts(value as *const u8, value_size as usize),
        )
    };
    m.core.insert(k, v)
}

/// # Safety
/// `key` must point to key_size readable bytes.
#[no_mangle]
pub unsafe extern "C" fn HashMap_EraseImpl(
    map: *mut FfiHashMap,
    key_size: u32,
    key: *const c_void,
) -> bool {
    // SAFETY: live map; key_size readable bytes per the hash_map.h macros
    let m = unsafe { &mut *map };
    debug_assert_eq!(m.core.key_size(), key_size);
    // SAFETY: caller passes key_size readable bytes
    let k = unsafe { core::slice::from_raw_parts(key as *const u8, key_size as usize) };
    m.core.erase(k)
}

/// # Safety
/// `key` must point to key_size readable bytes. The returned pointer aliases
/// the map's value storage (NULL when absent).
#[no_mangle]
pub unsafe extern "C" fn HashMap_LookupImpl(
    map: *mut FfiHashMap,
    key_size: u32,
    key: *const c_void,
) -> *mut c_void {
    // SAFETY: live map; key_size readable bytes per the hash_map.h macros
    let m = unsafe { &mut *map };
    debug_assert_eq!(m.core.key_size(), key_size);
    // SAFETY: caller passes key_size readable bytes
    let k = unsafe { core::slice::from_raw_parts(key as *const u8, key_size as usize) };
    match m.core.lookup(k) {
        // SAFETY: index is in-bounds of the value storage
        Some(index) => unsafe {
            m.core
                .value_storage_ptr()
                .add((m.core.value_size() * index) as usize) as *mut c_void
        },
        None => core::ptr::null_mut(),
    }
}

/// # Safety
/// `map` must be a live pointer from HashMap_CreateImpl.
#[no_mangle]
pub unsafe extern "C" fn HashMap_Size(map: *mut FfiHashMap) -> u32 {
    // SAFETY: live map per the caller contract
    unsafe { &*map }.core.size()
}

/// C computes `keys + key_size * index` without bounds checking; callers pass
/// indices below HashMap_Size.
///
/// # Safety
/// `index` must be within the map's storage.
#[no_mangle]
pub unsafe extern "C" fn HashMap_GetKeyImpl(map: *mut FfiHashMap, index: u32) -> *mut c_void {
    // SAFETY: live map; index within storage per the caller contract
    let m = unsafe { &mut *map };
    // SAFETY: same pointer arithmetic as the C
    unsafe {
        m.core
            .key_storage_ptr()
            .add((m.core.key_size() * index) as usize) as *mut c_void
    }
}

/// # Safety
/// `index` must be within the map's storage.
#[no_mangle]
pub unsafe extern "C" fn HashMap_GetValueImpl(map: *mut FfiHashMap, index: u32) -> *mut c_void {
    // SAFETY: live map; index within storage per the caller contract
    let m = unsafe { &mut *map };
    // SAFETY: same pointer arithmetic as the C
    unsafe {
        m.core
            .value_storage_ptr()
            .add((m.core.value_size() * index) as usize) as *mut c_void
    }
}

// --- test command (C registers it only under _DEBUG; exporting always is
// harmless — the symbol is simply unreferenced in release builds) ---

extern "C" fn hash_int32_c(val: *const c_void) -> u32 {
    // SAFETY: 4 readable bytes per the hasher contract
    let bytes = unsafe { core::slice::from_raw_parts(val as *const u8, 4) };
    quake_util::hash_map::hashers::hash_int32(u32::from_ne_bytes(bytes.try_into().unwrap()))
}

extern "C" fn hash_int64_c(val: *const c_void) -> u32 {
    // SAFETY: 8 readable bytes per the hasher contract
    let bytes = unsafe { core::slice::from_raw_parts(val as *const u8, 8) };
    quake_util::hash_map::hashers::hash_int64(u64::from_ne_bytes(bytes.try_into().unwrap()))
}

fn test_assert(cond: bool, what: &str) {
    if !cond {
        let mut msg: Vec<u8> = what.bytes().chain([0]).collect();
        // SAFETY: NUL-terminated format and string
        unsafe { quake_c_sys::Con_Printf(c"%s".as_ptr(), msg.as_mut_ptr()) };
        std::process::abort();
    }
}

/// Port of the C `TestHashMap_f` debug command, run through the same C ABI
/// the engine uses.
#[no_mangle]
pub extern "C" fn TestHashMap_f() {
    // basic test, without and with reserve
    for reserve in [false, true] {
        const TEST_SIZE: i32 = 1000;
        // SAFETY: shim calls mirror the C test exactly; all pointers are to
        // live locals and the map outlives them
        unsafe {
            let map = HashMap_CreateImpl(4, 8, Some(hash_int32_c), None);
            if reserve {
                HashMap_Reserve(map, TEST_SIZE);
            }
            for i in 0..TEST_SIZE {
                let value = i as i64;
                test_assert(
                    !HashMap_InsertImpl(
                        map,
                        4,
                        8,
                        &i as *const i32 as *const c_void,
                        &value as *const i64 as *const c_void,
                    ),
                    "value should not be overwritten\n",
                );
            }
            for i in 0..TEST_SIZE {
                let v = HashMap_LookupImpl(map, 4, &i as *const i32 as *const c_void);
                test_assert(
                    !v.is_null() && *(v as *const i64) == i as i64,
                    "wrong lookup\n",
                );
            }
            for i in (0..TEST_SIZE).step_by(2) {
                HashMap_EraseImpl(map, 4, &i as *const i32 as *const c_void);
            }
            for i in (1..TEST_SIZE).step_by(2) {
                let v = HashMap_LookupImpl(map, 4, &i as *const i32 as *const c_void);
                test_assert(
                    !v.is_null() && *(v as *const i64) == i as i64,
                    "wrong lookup\n",
                );
            }
            for i in (0..TEST_SIZE).step_by(2) {
                test_assert(
                    HashMap_LookupImpl(map, 4, &i as *const i32 as *const c_void).is_null(),
                    "wrong lookup\n",
                );
            }
            for i in 0..TEST_SIZE {
                HashMap_EraseImpl(map, 4, &i as *const i32 as *const c_void);
            }
            test_assert(HashMap_Size(map) == 0, "map is not empty\n");
            for i in 0..TEST_SIZE {
                test_assert(
                    HashMap_LookupImpl(map, 4, &i as *const i32 as *const c_void).is_null(),
                    "wrong lookup\n",
                );
            }
            HashMap_Destroy(map);
        }
    }

    // stress test, mirroring the C shuffle driven by COM_Rand
    // SAFETY: as above; COM_SeedRand/COM_Rand are engine C functions
    unsafe {
        quake_c_sys::COM_SeedRand(0);
        const TEST_SIZE: usize = 10000;
        let mut keys = vec![0i64; TEST_SIZE];
        let map = HashMap_CreateImpl(8, 4, Some(hash_int64_c), None);
        for _ in 0..10 {
            for (i, k) in keys.iter_mut().enumerate() {
                *k = i as i64;
            }
            for i in (1..TEST_SIZE).rev() {
                let swap_index = (quake_c_sys::COM_Rand() as usize) % (i + 1);
                // the C stores through `const int temp`, truncating i64 to
                // int; values stay below TEST_SIZE so it is lossless
                let temp = keys[swap_index] as i32;
                keys[swap_index] = keys[i];
                keys[i] = temp as i64;
            }
            for (i, k) in keys.iter().enumerate() {
                let value = i as i32;
                HashMap_InsertImpl(
                    map,
                    8,
                    4,
                    k as *const i64 as *const c_void,
                    &value as *const i32 as *const c_void,
                );
            }
            for (i, k) in keys.iter().enumerate() {
                let v = HashMap_LookupImpl(map, 8, k as *const i64 as *const c_void);
                test_assert(
                    !v.is_null() && *(v as *const i32) == i as i32,
                    "wrong lookup\n",
                );
            }
            for k in keys.iter().rev() {
                HashMap_EraseImpl(map, 8, k as *const i64 as *const c_void);
            }
            test_assert(HashMap_Size(map) == 0, "map is not empty\n");
        }
        HashMap_Destroy(map);
    }
}
