//! C ABI shims for `Quake/json.c` (declarations stay in `Quake/json.h`).
//!
//! The parse analysis is pure (quake_util::json); this shim assembles the
//! exact C arena: one Mem_Alloc block holding `json_t` + `jsonentry_t[n]` +
//! the unescaped strings, with parent/child/next pointers into the same
//! block, so C callers walk the tree through the json.h layout and
//! `JSON_Free` remains a single `Mem_Free` (ADR-013: the block crosses the
//! FFI boundary and must be engine-allocator memory).

use core::ffi::{c_char, c_int, c_void, CStr};
use quake_types::json::{
    Json, JsonEntry, JSON_ARRAY, JSON_BOOLEAN, JSON_NULL, JSON_NUMBER, JSON_OBJECT, JSON_STRING,
};
use quake_util::json::{parse, EntryData};

/// Appends `entry` to `parent`'s child list (json.h's firstchild/lastchild/next).
///
/// # Safety
/// Both must point at `JsonEntry` slots in the same arena; `parent`'s links
/// must already be initialised (the arena starts zeroed).
unsafe fn link_child(parent: *mut JsonEntry, entry: *mut JsonEntry) {
    // SAFETY: caller contract above; lastchild is non-NULL exactly when
    // firstchild is, since both are set together below
    unsafe {
        if (*parent).firstchild.is_null() {
            (*parent).firstchild = entry;
        } else {
            (*(*parent).lastchild).next = entry;
        }
        (*parent).lastchild = entry;
    }
}

/// Stamps one parsed entry's type tag and union payload into an arena slot.
///
/// This is the only place the `jsonentry_t` union is written, so the "which
/// member is live for this type tag?" argument is made once (ADR-004) instead
/// of once per match arm.
///
/// # Safety
/// `entry` must point at a zeroed `JsonEntry` in the arena, and `strings` at
/// that arena's string block, so `EntryData::String` offsets land inside it.
unsafe fn write_entry(entry: *mut JsonEntry, data: &EntryData, strings: *mut c_char) {
    // SAFETY: caller contract above
    unsafe {
        match *data {
            EntryData::Invalid => {} // stays zeroed: JSON_INVALID
            EntryData::Object => (*entry).type_ = JSON_OBJECT,
            EntryData::Array => (*entry).type_ = JSON_ARRAY,
            EntryData::String(offset) => {
                (*entry).type_ = JSON_STRING;
                (*entry).value.string = strings.add(offset);
            }
            EntryData::Number(n) => {
                (*entry).type_ = JSON_NUMBER;
                (*entry).value.number = n;
            }
            EntryData::Boolean(b) => {
                (*entry).type_ = JSON_BOOLEAN;
                (*entry).value.boolean = b;
            }
            EntryData::Null => {
                (*entry).type_ = JSON_NULL;
                (*entry).value.boolean = false;
            }
        }
    }
}

/// C: `json_t *JSON_Parse (const char *text);`
///
/// # Safety
/// `text` must be NULL or a NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn JSON_Parse(text: *const c_char) -> *mut Json {
    if text.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: NUL-terminated per the json.h contract
    let bytes = unsafe { CStr::from_ptr(text) }.to_bytes();
    let Some(parsed) = parse(bytes) else {
        return core::ptr::null_mut();
    };

    let numtokens = parsed.entries.len();
    let total = core::mem::size_of::<Json>()
        + core::mem::size_of::<JsonEntry>() * numtokens
        + parsed.c_strings_alloc_len;

    // SAFETY: Mem_Alloc returns zeroed memory (like the C json.c relies on);
    // NULL is propagated like the C
    let block = unsafe { quake_c_sys::Mem_Alloc(total) };
    if block.is_null() {
        return core::ptr::null_mut();
    }

    // SAFETY: the block is `total` bytes, so the json_t header, the numtokens
    // entries and the string bytes are carved out of it in that order; layout
    // matches quake-types' asserted mirrors. The strings are copied verbatim
    // (each NUL-terminated, plus the final NUL) — offsets in parsed.strings
    // are the same offsets in the block.
    let (json, entries, strings) = unsafe {
        let json = block as *mut Json;
        let entries = json.add(1) as *mut JsonEntry;
        let strings = entries.add(numtokens) as *mut c_char;

        (*json).numentries = numtokens as c_int;
        (*json).root = entries;
        (*json).strings = strings;

        core::ptr::copy_nonoverlapping(
            parsed.strings.as_ptr(),
            strings as *mut u8,
            parsed.strings.len(),
        );

        (json, entries, strings)
    };

    for (i, desc) in parsed.entries.iter().enumerate() {
        // SAFETY: i < numtokens, so this is one of the arena's entry slots
        let entry = unsafe { entries.add(i) };
        // link into the parent's child list, in token order
        if let Some(p) = desc.parent {
            // SAFETY: parse() only ever names an already-emitted token as a
            // parent, so p < i and that slot is an initialised arena entry
            unsafe { link_child(entries.add(p), entry) };
        }
        // SAFETY: `entry` is a zeroed arena slot and `strings` is this arena's
        // string block, which the offsets in `desc` index
        unsafe { write_entry(entry, &desc.data, strings) };
    }

    json
}

/// C: `void JSON_Free (json_t *json);`
///
/// # Safety
/// `json` must come from JSON_Parse (or be NULL, which Mem_Free tolerates).
#[no_mangle]
pub unsafe extern "C" fn JSON_Free(json: *mut Json) {
    // SAFETY: single-block allocation from Mem_Alloc
    unsafe { quake_c_sys::Mem_Free(json as *const c_void) };
}

/// C: `const jsonentry_t *JSON_Find (const jsonentry_t *entry, const char
/// *name, jsontype_t type);` — first child value of the right name AND type
/// (wrong-type duplicates are skipped).
///
/// # Safety
/// `entry` must be NULL or a valid tree node; `name` a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn JSON_Find(
    entry: *const JsonEntry,
    name: *const c_char,
    type_: c_int,
) -> *const JsonEntry {
    if entry.is_null() {
        return core::ptr::null();
    }
    // SAFETY: valid NUL-terminated name per the json.h contract
    let name = unsafe { CStr::from_ptr(name) };
    // SAFETY: `entry` is a tree node, so its links are readable
    let mut child = unsafe { (*entry).firstchild };

    while !child.is_null() {
        // SAFETY: tree pointers all point into the parse arena
        let e = unsafe { &*child };
        // C reads `entry->string` from the union without a type check:
        // for OBJECT/ARRAY/false/null children the union is zero and C
        // skips them; for number/true children C would strcmp through
        // garbage union bits (UB, crash in practice). The type check here
        // skips those safely — divergent only where the C is UB.
        if e.type_ == JSON_STRING {
            // SAFETY: a JSON_STRING entry's union holds a pointer into the
            // arena's string block, and firstchild (when non-NULL) is an
            // arena node; the conditions run in the C's order
            let matched = unsafe {
                !e.value.string.is_null()
                    && !e.firstchild.is_null()
                    && CStr::from_ptr(e.value.string) == name
                    && (*e.firstchild).type_ == type_
            };
            if matched {
                return e.firstchild;
            }
        }
        child = e.next;
    }
    core::ptr::null()
}

/// # Safety
/// As JSON_Find.
#[no_mangle]
pub unsafe extern "C" fn JSON_FindString(
    entry: *const JsonEntry,
    name: *const c_char,
) -> *const c_char {
    // SAFETY: forwarded contracts
    let found = unsafe { JSON_Find(entry, name, JSON_STRING) };
    if found.is_null() {
        return core::ptr::null();
    }
    // SAFETY: a JSON_STRING entry's union holds the string pointer
    unsafe { (*found).value.string }
}

/// # Safety
/// As JSON_Find.
#[no_mangle]
pub unsafe extern "C" fn JSON_FindNumber(
    entry: *const JsonEntry,
    name: *const c_char,
) -> *const f64 {
    // SAFETY: forwarded contracts
    let found = unsafe { JSON_Find(entry, name, JSON_NUMBER) };
    if found.is_null() {
        return core::ptr::null();
    }
    // SAFETY: a JSON_NUMBER entry's union holds the number
    unsafe { core::ptr::addr_of!((*found).value.number) }
}

/// # Safety
/// As JSON_Find.
#[no_mangle]
pub unsafe extern "C" fn JSON_FindBoolean(
    entry: *const JsonEntry,
    name: *const c_char,
) -> *const bool {
    // SAFETY: forwarded contracts
    let found = unsafe { JSON_Find(entry, name, JSON_BOOLEAN) };
    if found.is_null() {
        return core::ptr::null();
    }
    // SAFETY: a JSON_BOOLEAN entry's union holds the qboolean
    unsafe { core::ptr::addr_of!((*found).value.boolean) }
}
