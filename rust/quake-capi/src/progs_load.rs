//! C-ABI shim for the progs loader (`pr_edict_load.c` → `quake-progs`).
//!
//! `Quake/pr_edict_load_glue.c` keeps `PR_SwitchQCVM` — the selector every
//! other translation unit reaches the ambient VM through — plus the engine
//! lookups and `va` that the loader must call rather than reimplement, and the
//! four `Host_Error` raises (ADR-009).
//!
//! M9g: the two pointers that selector assigns, `qcvm` and `pr_global_struct`,
//! are Rust-owned storage, defined below as `#[no_mangle] static mut` exactly
//! as `net.rs` defines the net reader globals (ADR-007 qcvm row closed).

use core::ffi::{c_char, c_int, c_void, CStr};

use quake_c_sys as c;
use quake_progs::image::{ProgsImage, VmLoad};
use quake_progs::load::{self, LoadError, LoadSys};
use quake_types::progs::{BuiltinT, DDef, QcVm};

// ---------------------------------------------------------------------------
// ADR-007 qcvm row: the two ambient VM pointers, Rust-owned from Phase 7 M9g.
//
// `progs.h:433-435` keeps declaring both, so the 14 files outside the progs
// sources that dereference them resolve to the definitions below without a
// source change, and `Quake/pr_edict_load_glue.c` still writes them through
// `PR_SwitchQCVM`/`PRLoad_Glue_DeselectQCVM`/`PRLoad_Glue_SetPrGlobalStruct`
// exactly as before. `Quake/pr_edict_load.c` keeps its own copies for the
// -Duse_rust_progs=disabled oracle leg; that file and this module are never
// compiled into the same binary.

/// `pr_edict_load.c:34` -- `qcvm_t *qcvm;`, the ambient VM (ADR-008).
///
/// SAFETY: a null pointer is the C definition's own initial value, and
/// `PR_SwitchQCVM (NULL)` restores it; every reader on both sides already
/// treats null as "no VM selected".
#[no_mangle]
pub static mut qcvm: *mut c::qcvm_s = core::ptr::null_mut();

/// `pr_edict_load.c:35` -- `globalvars_t *pr_global_struct;`.
///
/// `globalvars_t` has no bindgen mirror, so this is typed the way
/// `quake_c_sys::sv_main` already declares it: an opaque pointer. C's
/// `progs.h:433` declaration is the authoritative type for every C reader.
#[no_mangle]
pub static mut pr_global_struct: *mut c_void = core::ptr::null_mut();

/// Status codes shared with `Quake/pr_edict_load_glue.c` (keep in sync).
const PRLOAD_OK: c_int = 0;
const PRLOAD_FALSE: c_int = 1;
const PRLOAD_ERR_VERSION: c_int = 2;
const PRLOAD_ERR_CRC: c_int = 3;
const PRLOAD_ERR_STRINGS_PAST_END: c_int = 4;
const PRLOAD_ERR_SAVEGLOBAL: c_int = 5;
const PRLOAD_ERR_LUMP_RANGE: c_int = 6;
const PRLOAD_ERR_ENTITYFIELDS: c_int = 7;
const PRLOAD_ERR_TOO_SHORT: c_int = 8;
const PRLOAD_ERR_UNTERMINATED_STRINGS: c_int = 9;

/// Deferred console output. `Con_Printf` is not a leaf (it can reach
/// `SCR_UpdateScreen`), so nothing is printed while the loader holds its views
/// of the VM and the image.
struct EngineLoad {
    /// `(level, bytes)` with level 0 = `Con_Printf`, 1 = `Con_DPrintf`,
    /// 2 = `Con_DPrintf2`.
    pending: Vec<(u8, Vec<u8>)>,
}

impl EngineLoad {
    fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    fn drain(&mut self) {
        for (level, mut msg) in core::mem::take(&mut self.pending) {
            msg.retain(|&b| b != 0);
            msg.push(0);
            // SAFETY: `msg` is NUL-terminated and the console takes a plain
            // `%s`, so progs bytes reach it unmodified.
            unsafe {
                match level {
                    0 => c::Con_Printf(c"%s".as_ptr(), msg.as_ptr()),
                    1 => c::Con_DPrintf(c"%s".as_ptr(), msg.as_ptr()),
                    _ => c::Con_DPrintf2(c"%s".as_ptr(), msg.as_ptr()),
                }
            }
        }
    }
}

impl quake_progs::arena::Mem for EngineLoad {
    fn alloc(&mut self, size: usize) -> *mut u8 {
        // SAFETY: Mem_Alloc returns zeroed memory or aborts (ADR-013).
        unsafe { c::Mem_Alloc(size).cast() }
    }

    fn realloc(&mut self, ptr: *mut u8, size: usize) -> *mut u8 {
        // SAFETY: `ptr` is null or came from this allocator.
        unsafe { c::Mem_Realloc(ptr.cast(), size).cast() }
    }

    fn free(&mut self, ptr: *mut u8) {
        // SAFETY: `ptr` is null or came from this allocator; Mem_Free
        // tolerates null.
        unsafe { c::Mem_Free(ptr.cast()) }
    }

    fn note_slot_growth(&mut self, maxknownstrings: c_int) {
        self.pending.push((
            2,
            format!("PR_AllocStringSlots: realloc'ing for {maxknownstrings} slots\n").into_bytes(),
        ));
    }
}

impl LoadSys for EngineLoad {
    fn map_create(&mut self) -> *mut c_void {
        // SAFETY: a leaf call into hash_map.
        unsafe { c::PRLoad_Glue_MapCreate() }
    }

    fn map_reserve(&mut self, map: *mut c_void, capacity: c_int) {
        // SAFETY: `map` came from `map_create`.
        unsafe { c::PRLoad_Glue_MapReserve(map, capacity) }
    }

    fn map_insert(&mut self, map: *mut c_void, key: *const c_char, value: *const c_void) {
        // SAFETY: `map` came from `map_create`; the map stores both pointers
        // by value and dereferences `key` only during a lookup.
        unsafe { c::PRLoad_Glue_MapInsert(map, key, value) }
    }

    fn map_destroy(&mut self, map: *mut c_void) {
        // SAFETY: `map` came from `map_create` (or is null, which
        // HashMap_Destroy tolerates the same way C's PR_ClearProgs relies on).
        unsafe { c::PRLoad_Glue_MapDestroy(map) }
    }

    fn ed_new_string(&mut self, s: *const c_char) -> c_int {
        // SAFETY: `s` is NUL-terminated and the ambient VM's string table is
        // live — `PR_MergeEngineFieldDefs` runs inside `PR_LoadProgs`, which
        // is exactly where C calls `ED_NewString` from too.
        unsafe { crate::progs_parse::quake_rs_ed_new_string(s) }
    }

    fn set_empty_engine_string(&mut self) {
        // SAFETY: a leaf call; the string table is live.
        unsafe { c::PRLoad_Glue_SetEmptyEngineString() }
    }

    fn find_field_ofs(&mut self, name: &CStr) -> c_int {
        // SAFETY: `ED_FindFieldOffset` is a hash lookup over the map this
        // loader has already built.
        unsafe { c::PRLoad_Glue_FindFieldOfs(name.as_ptr()) }
    }

    fn global_float(&mut self, name: &CStr) -> Option<f32> {
        let mut out = 0.0f32;
        // SAFETY: as above, over `globaldefs_map`.
        let found = unsafe { c::PRLoad_Glue_GlobalFloat(name.as_ptr(), &mut out) };
        found.then_some(out)
    }

    fn find_function(&mut self, name: &CStr) -> Option<c_int> {
        // SAFETY: as above, over `function_map`.
        let index = unsafe { c::PRLoad_Glue_FindFunction(name.as_ptr()) };
        (index >= 0).then_some(index)
    }

    fn va_component_name(&mut self, name: &CStr, component: u8) -> *const c_char {
        // SAFETY: `va` returns one of its rotating static buffers; keeping
        // that storage is the whole point (see the trait's COMPAT note).
        unsafe { c::PRLoad_Glue_VaComponent(name.as_ptr(), c_int::from(component)) }
    }

    fn flush_console(&mut self) {
        self.drain();
    }

    fn shutdown_extensions(&mut self) {
        // SAFETY: `PR_ShutdownExtensions` runs against the ambient VM, which
        // `clear_progs` has just selected.
        unsafe { c::PRLoad_Glue_ShutdownExtensions() }
    }

    fn enable_extensions(&mut self, globaldefs: *mut DDef) {
        // SAFETY: `globaldefs` is the lump this loader just installed.
        unsafe { c::PRLoad_Glue_EnableExtensions(globaldefs.cast()) }
    }

    fn switch_qcvm(&mut self, vm: *mut QcVm) {
        // SAFETY: `PR_SwitchQCVM` only assigns two globals (and Sys_Errors if
        // a VM is already active, which is the assertion being preserved).
        unsafe { c::PRLoad_Glue_SwitchQCVM(vm.cast()) }
    }

    fn deselect_qcvm(&mut self) {
        // SAFETY: `qcvm = NULL` written directly, exactly as C's
        // `PR_ClearProgs` does to get past the already-active assertion.
        unsafe { c::PRLoad_Glue_DeselectQCVM() }
    }

    fn current_qcvm(&mut self) -> *mut QcVm {
        // SAFETY: reading the ambient global.
        unsafe { c::qcvm.cast() }
    }

    fn set_pr_global_struct(&mut self, globals: *mut f32) {
        // SAFETY: assigns the C global the engine reads `pr_global_struct`
        // through; `globals` is the image's globals lump.
        unsafe { c::PRLoad_Glue_SetPrGlobalStruct(globals) }
    }

    fn is_server_vm(&mut self, vm: *mut QcVm) -> bool {
        // SAFETY: a pointer comparison against `&sv.qcvm` in C.
        unsafe { c::PRLoad_Glue_IsServerVM(vm.cast()) }
    }

    fn set_effects_mask(&mut self, mask: c_int) {
        // SAFETY: assigns `sv.effectsmask`.
        unsafe { c::PRLoad_Glue_SetEffectsMask(mask) }
    }

    fn print(&mut self, msg: &[u8]) {
        self.pending.push((0, msg.to_vec()));
    }

    fn dprint(&mut self, msg: &[u8]) {
        self.pending.push((1, msg.to_vec()));
    }
}

fn encode(err: LoadError, detail: &mut c_int) -> c_int {
    match err {
        LoadError::WrongVersion(v) => {
            *detail = v;
            PRLOAD_ERR_VERSION
        }
        LoadError::CrcMismatch => PRLOAD_ERR_CRC,
        LoadError::StringsPastEnd => PRLOAD_ERR_STRINGS_PAST_END,
        LoadError::FieldDefSaveGlobal => PRLOAD_ERR_SAVEGLOBAL,
        LoadError::LumpOutOfRange => PRLOAD_ERR_LUMP_RANGE,
        LoadError::UnterminatedStrings => PRLOAD_ERR_UNTERMINATED_STRINGS,
        LoadError::TooShort(n) => {
            *detail = n as c_int;
            PRLOAD_ERR_TOO_SHORT
        }
        LoadError::BadEntityFields(n) => {
            *detail = n;
            PRLOAD_ERR_ENTITYFIELDS
        }
    }
}

/// C: `void PR_ClearProgs (qcvm_t *vm);`
///
/// # Safety
///
/// `vm` must point at a live `qcvm_t`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pr_clear_progs(vm: *mut c_void) {
    if vm.is_null() {
        return;
    }
    let mut sys = EngineLoad::new();
    // SAFETY: the caller's contract.
    let mut view = unsafe { VmLoad::new(vm.cast()) };
    load::clear_progs(&mut view, &mut sys);
    sys.drain();
}

/// C: `qboolean PR_LoadProgs (...)`, from the point where the glue's
/// `COM_LoadFile` has returned `data`/`len`.
///
/// Returns one of the `PRLOAD_*` codes; the glue raises for the error ones.
///
/// # Safety
///
/// `vm` must be the ambient `qcvm_t`, `data` the `COM_LoadFile` block of `len`
/// bytes that the VM takes ownership of, `filename` NUL-terminated, and
/// `builtins` an array of at least `numbuiltins` `builtin_t` entries.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn quake_rs_pr_load_progs(
    vm: *mut c_void,
    data: *mut u8,
    len: usize,
    filename: *const c_char,
    fatal: bool,
    needcrc: c_int,
    builtins: *const c_void,
    numbuiltins: usize,
    detail: *mut c_int,
) -> c_int {
    let mut sys = EngineLoad::new();
    // SAFETY: the caller's contract.
    let (mut view, mut image, name, table) = unsafe {
        (
            VmLoad::new(vm.cast()),
            ProgsImage::new(data, len),
            CStr::from_ptr(filename),
            core::slice::from_raw_parts(builtins.cast::<BuiltinT>(), numbuiltins),
        )
    };

    let result = load::load_progs(&mut view, &mut image, name, fatal, needcrc, table, &mut sys);
    sys.drain();

    match result {
        Ok(true) => PRLOAD_OK,
        Ok(false) => PRLOAD_FALSE,
        Err(err) => {
            // SAFETY: the glue always passes a live `int`.
            encode(err, unsafe { &mut *detail })
        }
    }
}
