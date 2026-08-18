//! `json_t` / `jsonentry_t` mirrors (`Quake/json.h`). Compat-critical ABI:
//! C callers (host_cmd.c, steam.c) walk `json->root`, `->firstchild`,
//! `->next` and read the union through these layouts, and the whole tree
//! lives in one `Mem_Alloc` block that `JSON_Free` hands to `Mem_Free`.

use core::ffi::{c_char, c_int};

#[repr(C)]
#[derive(Clone, Copy)]
pub union JsonValue {
    pub string: *const c_char,
    pub number: f64,
    /// qboolean
    pub boolean: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct JsonEntry {
    pub value: JsonValue,
    /// jsontype_t
    pub type_: c_int,
    pub parent: *mut JsonEntry,
    pub firstchild: *mut JsonEntry,
    pub lastchild: *mut JsonEntry,
    pub next: *mut JsonEntry,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Json {
    pub numentries: c_int,
    pub root: *mut JsonEntry,
    pub strings: *const c_char,
}

pub const JSON_INVALID: c_int = 0;
pub const JSON_OBJECT: c_int = 1;
pub const JSON_ARRAY: c_int = 2;
pub const JSON_STRING: c_int = 3;
pub const JSON_NUMBER: c_int = 4;
pub const JSON_BOOLEAN: c_int = 5;
pub const JSON_NULL: c_int = 6;

const _: () = assert!(std::mem::size_of::<JsonValue>() == 8);
const _: () = assert!(std::mem::size_of::<JsonEntry>() == 48);
const _: () = assert!(std::mem::offset_of!(JsonEntry, value) == 0);
const _: () = assert!(std::mem::offset_of!(JsonEntry, type_) == 8);
const _: () = assert!(std::mem::offset_of!(JsonEntry, parent) == 16);
const _: () = assert!(std::mem::offset_of!(JsonEntry, firstchild) == 24);
const _: () = assert!(std::mem::offset_of!(JsonEntry, lastchild) == 32);
const _: () = assert!(std::mem::offset_of!(JsonEntry, next) == 40);
const _: () = assert!(std::mem::size_of::<Json>() == 24);
const _: () = assert!(std::mem::offset_of!(Json, numentries) == 0);
const _: () = assert!(std::mem::offset_of!(Json, root) == 8);
const _: () = assert!(std::mem::offset_of!(Json, strings) == 16);
