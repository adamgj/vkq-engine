//! C-ABI shim for the edict arena and the progs string table
//! (`pr_edict_arena.c` → `quake-progs`), Rust migration Phase 7 M9d.
//!
//! `Quake/pr_edict_arena_glue.c` owns the `ED_ALLOC_HOOK` slot, the platform
//! `qsort` the free-list rebuild must keep using, and every `Host_Error`
//! (ADR-009 rule 3: no longjmp may cross a Rust frame, so the cores below
//! return a status and the glue issues the original message from a C frame).
//!
//! # Allocator
//!
//! COMPAT (ADR-013): the string table keeps allocating through
//! `Mem_Alloc`/`Mem_Realloc`/`Mem_Free`, and the edict array is *borrowed*
//! ([`EdictArena::borrowed`]) -- [`EdictArena::owned`] must never be used
//! here. `qcvm->edicts` is allocated by Rust
//! (`quake-capi/src/host.rs:1415`, `quake-capi/src/sv_main.rs:1584`, both
//! `Mem_Alloc`) but freed by C at `Quake/pr_edict_load.c:65`, and the two
//! sides are gated on *different* meson switches (`-Duse_rust_host` vs
//! `-Duse_rust_progs`), so a configuration exists in which a Rust-owned
//! allocation would be handed to C's `Mem_Free`. `Mem_*` on both ends is the
//! only shape that is correct in all six configurations.
//!
//! # Hoisted bounds checks
//!
//! `EDICT_NUM` and `NUM_FOR_EDICT` (`Quake/pr_edict.c:1156-1198`) stay C and
//! `Host_Error` on an out-of-range number -- unconditionally, not
//! debug-only. [`EdictArena`] instead *asserts*, and every profile sets
//! `panic = "abort"`, which would turn C's recoverable raise into a crash.
//! Each entry point therefore range-checks before it can reach an arena
//! accessor and returns [`PRARENA_RAISE_EDICT_NUM`] /
//! [`PRARENA_RAISE_NUM_FOR_EDICT`] at the point C would have raised.

use core::ffi::{c_char, c_int, c_void};

use quake_c_sys as c;
use quake_c_sys::pr_edict_arena as g;
use quake_progs::alloc::{self, AllocCtx, AllocError, FreeListOverflow, FreeListWarning};
use quake_progs::arena::{EdictArena, EdictId, Mem, StringError, VmRaw};
use quake_types::progs::{EntityState, FreeList, QcVm, ENGINE_DEBUG, MAX_EDICTS};

/// Status codes shared with `Quake/pr_edict_arena_glue.c` (keep in sync).
const PRARENA_OK: c_int = 0;
/// `ED_Alloc: no free edicts (max_edicts is %i)` (`pr_edict_arena.c:76`).
const PRARENA_RAISE_NO_FREE_EDICTS: c_int = 1;
/// `EDICT_NUM: bad edict_num %i` (`pr_edict.c:1160`).
const PRARENA_RAISE_EDICT_NUM: c_int = 2;
/// `NUM_FOR_EDICT: bad pointer` (`pr_edict.c:1186`).
const PRARENA_RAISE_NUM_FOR_EDICT: c_int = 3;
/// `ED_AddToFreeList : is full (qcvm 0x%p)` (`pr_edict_arena.c:110`, debug).
const PRARENA_RAISE_FREELIST_FULL: c_int = 4;
/// `ED_AddToFreeList : has more than max_edicts >= %i (qcvm 0x%p)`
/// (`pr_edict_arena.c:112`, debug).
const PRARENA_RAISE_FREELIST_OVER_MAX: c_int = 5;
/// `PR_GetString: attempt to get a non-existant string %d`
/// (`pr_edict_arena.c:315`).
const PRARENA_RAISE_STRING_MISSING: c_int = 6;

extern "C" {
    /// `Quake/protocol.c` -- `entity_state_t nullentitystate;` (note: not
    /// all-zero). `ED_Alloc` copies it into a fresh edict's baseline.
    static nullentitystate: EntityState;
}

/// The engine's `Mem_*` allocator for the string table (ADR-013).
struct EngineArena {
    /// `PR_AllocStringSlots`'s `Con_DPrintf2`, deferred rather than printed
    /// inline: the console is not a leaf (it can reach `SCR_UpdateScreen`),
    /// so it must not run while a view of the VM is live. Same treatment as
    /// `progs_parse::EngineParse`.
    pending_slots: Vec<c_int>,
}

impl Mem for EngineArena {
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
        // tolerates null, like C's SAFE_FREE.
        unsafe { c::Mem_Free(ptr.cast()) }
    }

    fn note_slot_growth(&mut self, maxknownstrings: c_int) {
        self.pending_slots.push(maxknownstrings);
    }
}

impl EngineArena {
    fn new() -> Self {
        Self {
            pending_slots: Vec::new(),
        }
    }

    /// Drain the deferred `Con_DPrintf2`. Called only after every view of the
    /// VM has been dropped.
    ///
    /// COMPAT: C prints *before* the two `Mem_Realloc`s
    /// (`pr_edict_arena.c:301`); deferring moves the line after the whole
    /// call. That is the established treatment of non-leaf console output on
    /// this boundary (`progs_parse::EngineParse::flush`), and nothing between
    /// the two points reads the console.
    fn flush(&mut self) {
        for n in self.pending_slots.drain(..) {
            // SAFETY: the original format string with its one int argument;
            // no borrow of the VM is live here.
            unsafe {
                c::Con_DPrintf2(
                    c"PR_AllocStringSlots: realloc'ing for %d slots\n".as_ptr(),
                    n,
                );
            }
        }
    }
}

/// # Safety
///
/// The ambient `qcvm` must be selected.
unsafe fn ambient_vm() -> VmRaw {
    // SAFETY: the ambient qcvm is the VM the host frame selected (ADR-008).
    unsafe { VmRaw::new(c::qcvm.cast::<QcVm>()) }
}

/// An arena over the ambient VM's edict array.
///
/// # Safety
///
/// The ambient `qcvm` must be loaded, i.e. past the point in `PR_LoadProgs`
/// where `edict_size` is computed and `edicts` allocated.
unsafe fn ambient_arena(vm: &VmRaw) -> EdictArena {
    let stride = vm.edict_stride() as usize;
    let count = vm.max_edicts().max(0) as usize;
    // SAFETY: the edict array is `max_edicts * edict_size` bytes, allocated
    // by PR_LoadProgs and live for the VM's lifetime. Borrowed, never owned
    // -- see the module's allocator note.
    unsafe { EdictArena::borrowed(vm.edicts_base(), stride, count) }
}

/// `NUM_FOR_EDICT` without its `Host_Error`:
/// `((byte *)e - (byte *)qcvm->edicts) / qcvm->edict_size`, truncating toward
/// zero exactly like C's `/`.
///
/// `None` means the pointer does not land inside the edict array at all.
/// COMPAT (unreproducible, ADR-006): C never bounds-checks before reading
/// `ed->free`, so such a pointer is already an out-of-bounds access there and
/// there is no defined behaviour to preserve; the callers report it as
/// `NUM_FOR_EDICT`'s own raise, the nearest C diagnostic.
fn num_for_edict(vm: &VmRaw, ed: *mut c_void) -> Option<c_int> {
    let stride = vm.edict_stride() as isize;
    if stride <= 0 {
        return None;
    }
    let delta = (ed as isize).wrapping_sub(vm.edicts_base() as isize);
    let num = delta / stride;
    if num < 0 || num >= isize::try_from(vm.max_edicts()).unwrap_or(0) {
        return None;
    }
    Some(num as c_int)
}

/// `ED_RebuildFreeList`'s sort, handed straight back to the platform `qsort`
/// in the glue -- see [`g::PREdictArena_Glue_SortFreeEdicts`].
fn sort_free_edicts(nums: &mut [c_int]) {
    // SAFETY: `nums` is a live slice of `len` ints, and every number in it is
    // below `num_edicts <= max_edicts`, which the callers range-check before
    // they can reach here, so the comparator's EDICT_NUM_NO_CHECK is in
    // bounds.
    unsafe { g::PREdictArena_Glue_SortFreeEdicts(nums.as_mut_ptr(), nums.len()) }
}

/// `SV_UnlinkEdict`, through the seam `pr_edict_parse_glue.c` already
/// defines: both glue translation units are compiled by the same
/// `-Duse_rust_progs` leg, so no second seam is needed.
fn unlink_edict(id: EdictId) {
    // SAFETY: `id` is below max_edicts (the caller's num_for_edict check), so
    // EDICT_NUM_NO_CHECK is in bounds. SV_UnlinkEdict is world.c's area-tree
    // removal and does not raise (audited at pr_edict_parse_glue.c:73).
    unsafe { c::PRParse_Glue_UnlinkEdict(id.0 as c_int) }
}

/// One of `ED_CheckFreeList`'s three `Con_Warning`s.
///
/// Printed inline rather than deferred: `Con_Warning` is a plain leaf call by
/// project policy, and printing here (after both loops, before the rebuild)
/// is the same order C observes -- nothing happens between C's last warning
/// and its `ED_RebuildFreeList (false)`.
fn warn(w: FreeListWarning) {
    let (fmt, n) = match w {
        FreeListWarning::InListButNotFree(n) => (
            c"ED_CheckFreeList: edict %i is in free-list but is NOT free\n",
            n,
        ),
        FreeListWarning::FreeButNotInList(n) => (
            c"ED_CheckFreeList: edict %i is free, but is NOT in free-list\n",
            n,
        ),
        FreeListWarning::NotFreeButInList(n) => (
            c"ED_CheckFreeList: edict %i is NOT free, but is in free-list\n",
            n,
        ),
    };
    // SAFETY: the original format string with its one int argument; no borrow
    // of the VM is live here.
    unsafe { c::Con_Warning(fmt.as_ptr(), n) }
}

/// `ED_Alloc`'s core. Hands back the edict *number*; the glue converts it to
/// an `edict_t *` and calls `ED_ALLOC_HOOK`, which is where C called it (the
/// last statement of both branches), keeping the C function pointer out of
/// any Rust frame.
///
/// # Safety
///
/// The ambient `qcvm` must be loaded; `out_num` and `detail` must point at
/// writable `int`s.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ed_alloc(out_num: *mut c_int, detail: *mut c_int) -> c_int {
    // SAFETY: the caller's frame owns both.
    let (out_num, detail) = unsafe { (&mut *out_num, &mut *detail) };
    *out_num = 0;
    *detail = 0;

    // SAFETY: see ambient_vm()/ambient_arena(); ED_Alloc only runs on a
    // loaded VM.
    let vm = unsafe { ambient_vm() };
    // SAFETY: as above.
    let mut arena = unsafe { ambient_arena(&vm) };
    let max_edicts = vm.max_edicts();
    let time = vm.time();
    let entityfields = vm.entityfields();
    let vmp = vm.as_ptr();
    // SAFETY: two disjoint fields of the live qcvm_t; no other Rust reference
    // to either is live (`vm` is not touched again).
    let free_list: &mut FreeList = unsafe { &mut (*vmp).free_list };
    // SAFETY: as above.
    let num_edicts: &mut c_int = unsafe { &mut (*vmp).num_edicts };

    // the two EDICT_NUM calls C makes -- see the module's hoisting note. The
    // first is in C's own position (nothing observable precedes it).
    if free_list.size > 0 {
        let head = c_int::from(free_list.circular_buffer[free_list.head_index]);
        if head >= max_edicts {
            *detail = head;
            return PRARENA_RAISE_EDICT_NUM;
        }
    }
    // COMPAT (carve-out): C reaches `EDICT_NUM (qcvm->num_edicts++)` only
    // after the reuse path declines, so hoisting this check ahead of it
    // raises where C would have returned a reused edict -- but only when
    // `num_edicts > max_edicts`, which no writer produces: `PR_LoadProgs`
    // (`pr_edict_load.c`) sets `num_edicts` no higher than `max_edicts`, and
    // the only increment is this function's, guarded by the `==` test below.
    // Hoisting is what turns a corrupt state into C's raise instead of an
    // `EdictArena` assert, which `panic = "abort"` would make fatal.
    if *num_edicts < 0 || *num_edicts > max_edicts {
        *detail = *num_edicts;
        return PRARENA_RAISE_EDICT_NUM;
    }

    let mut ctx = AllocCtx {
        free_list,
        num_edicts,
        max_edicts,
        time,
        entityfields,
    };
    // SAFETY: `nullentitystate` is protocol.c storage, written once at
    // startup and read-only afterwards.
    let null_state = unsafe { &*core::ptr::addr_of!(nullentitystate) };

    match alloc::ed_alloc(&mut ctx, &mut arena, null_state, vmp.cast::<c_void>()) {
        Ok(id) => {
            *out_num = id.0 as c_int;
            PRARENA_OK
        }
        Err(AllocError::NoFreeEdicts { max_edicts }) => {
            *detail = max_edicts;
            PRARENA_RAISE_NO_FREE_EDICTS
        }
    }
}

/// `ED_Free`'s core.
///
/// `ED_AddToFreeList` is inlined here rather than in `quake-progs` because
/// its two debug-only raises and `NUM_FOR_EDICT`'s raise are evaluated
/// *between* the field writes and the free-list add, so the halves have to be
/// driven separately (hence `alloc::ed_free_fields`).
///
/// # Safety
///
/// The ambient `qcvm` must be loaded; `ed` must be an `edict_t *` and
/// `detail` must point at a writable `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ed_free(ed: *mut c_void, detail: *mut c_int) -> c_int {
    // SAFETY: the caller's frame owns it.
    let detail = unsafe { &mut *detail };
    *detail = 0;

    // SAFETY: see ambient_vm()/ambient_arena().
    let vm = unsafe { ambient_vm() };
    // SAFETY: as above.
    let mut arena = unsafe { ambient_arena(&vm) };
    let max_edicts = vm.max_edicts();
    let time = vm.time();
    let Some(num) = num_for_edict(&vm, ed) else {
        return PRARENA_RAISE_NUM_FOR_EDICT;
    };
    let id = EdictId(num as u32);

    if !alloc::ed_free_fields(&mut arena, id, time, &mut unlink_edict) {
        // C's `if (ed->free)` early return
        return PRARENA_OK;
    }

    let vmp = vm.as_ptr();
    // SAFETY: the free list lives inside the same qcvm_t; no other Rust
    // reference to it is live, and SV_UnlinkEdict (above) does not touch it.
    let free_list: &mut FreeList = unsafe { &mut (*vmp).free_list };

    // ED_AddToFreeList's own body, in C's order: the debug preconditions
    // first, then NUM_FOR_EDICT, then the add.
    if ENGINE_DEBUG {
        match alloc::free_list_would_overflow(free_list, max_edicts) {
            Some(FreeListOverflow::Full) => return PRARENA_RAISE_FREELIST_FULL,
            Some(FreeListOverflow::OverMaxEdicts) => {
                *detail = max_edicts;
                return PRARENA_RAISE_FREELIST_OVER_MAX;
            }
            None => {}
        }
    }
    // SAFETY: a plain field read of the live qcvm_t.
    if num >= unsafe { (*vmp).num_edicts } {
        return PRARENA_RAISE_NUM_FOR_EDICT;
    }

    alloc::add_to_free_list(free_list, id);
    PRARENA_OK
}

/// `ED_RemoveFromFreeList`'s core.
///
/// # Safety
///
/// The ambient `qcvm` must be loaded and `ed` must be an `edict_t *`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ed_remove_from_free_list(ed: *mut c_void) -> c_int {
    // SAFETY: see ambient_vm().
    let vm = unsafe { ambient_vm() };
    let Some(num) = num_for_edict(&vm, ed) else {
        return PRARENA_RAISE_NUM_FOR_EDICT;
    };
    let vmp = vm.as_ptr();
    // SAFETY: a plain field read of the live qcvm_t.
    if num >= unsafe { (*vmp).num_edicts } {
        return PRARENA_RAISE_NUM_FOR_EDICT;
    }
    // SAFETY: the free list lives inside the same qcvm_t; no other Rust
    // reference to it is live.
    let free_list: &mut FreeList = unsafe { &mut (*vmp).free_list };
    alloc::remove_from_free_list(free_list, EdictId(num as u32));
    PRARENA_OK
}

/// `ED_CheckFreeList`'s core -- the `edicts` console command's cross-check.
///
/// # Safety
///
/// The ambient `qcvm` must be loaded; `detail` must point at a writable
/// `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ed_check_free_list(detail: *mut c_int) -> c_int {
    // SAFETY: the caller's frame owns it.
    let detail = unsafe { &mut *detail };
    *detail = 0;

    // SAFETY: see ambient_vm()/ambient_arena().
    let vm = unsafe { ambient_vm() };
    // SAFETY: as above.
    let mut arena = unsafe { ambient_arena(&vm) };
    let max_edicts = vm.max_edicts();
    let num_edicts = vm.num_edicts();
    let vmp = vm.as_ptr();

    let warnings = {
        // SAFETY: the free list lives inside the same qcvm_t; nothing under
        // check_free_list mutates it.
        let free_list: &FreeList = unsafe { &(*vmp).free_list };

        // Both loops' EDICT_NUM calls, hoisted -- see the module's note. The
        // scratch table C indexes as `free_list_edicts[edict_num]` is
        // MAX_EDICTS bytes and so is `check_free_list`'s `in_list`, and
        // `max_edicts` is CLAMPed to MAX_EDICTS at every assignment
        // (`sv_main.c:951`, `host.c:963`, `cl_main.c:146`), so these two
        // checks also keep the Rust indexing in bounds.
        //
        // COMPAT (carve-out): C's first loop prints nothing, so hoisting its
        // check is invisible; hoisting the *second* loop's check ahead of the
        // first loop's warnings would drop those warnings on a raise. That
        // needs `num_edicts > max_edicts`, which no writer produces (see
        // `quake_rs_ed_alloc`).
        let mut idx = free_list.head_index;
        for _ in 0..free_list.size {
            let n = c_int::from(free_list.circular_buffer[idx]);
            if n >= max_edicts {
                *detail = n;
                return PRARENA_RAISE_EDICT_NUM;
            }
            idx = (idx + 1) % MAX_EDICTS;
        }
        if num_edicts > max_edicts {
            *detail = max_edicts;
            return PRARENA_RAISE_EDICT_NUM;
        }

        alloc::check_free_list(free_list, &arena, num_edicts)
    };

    for w in &warnings {
        warn(*w);
    }

    if !warnings.is_empty() {
        // C's `has_errors` branch. COMPAT: the rebuild's own
        // ED_AddToFreeList calls cannot hit either debug precondition -- it
        // adds at most one entry per free edict, so `size` peaks at
        // `nb_free_edicts - 1 <= num_edicts - 1 < max_edicts <= MAX_EDICTS`,
        // with `num_edicts <= max_edicts` established immediately above.
        // SAFETY: the free list lives inside the same qcvm_t; the shared
        // borrow above has ended.
        let free_list: &mut FreeList = unsafe { &mut (*vmp).free_list };
        alloc::rebuild_free_list(
            free_list,
            &mut arena,
            num_edicts,
            false,
            &mut sort_free_edicts,
        );
    }

    PRARENA_OK
}

/// `ED_RebuildFreeList`'s core.
///
/// # Safety
///
/// The ambient `qcvm` must be loaded; `detail` must point at a writable
/// `int`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ed_rebuild_free_list(
    force_free_reuse: bool,
    detail: *mut c_int,
) -> c_int {
    // SAFETY: the caller's frame owns it.
    let detail = unsafe { &mut *detail };
    *detail = 0;

    // SAFETY: see ambient_vm()/ambient_arena().
    let vm = unsafe { ambient_vm() };
    // SAFETY: as above.
    let mut arena = unsafe { ambient_arena(&vm) };
    let max_edicts = vm.max_edicts();
    let num_edicts = vm.num_edicts();

    // the enumeration loop's EDICT_NUM (i) -- see the module's hoisting note.
    // With this established, the rebuild's own ED_AddToFreeList calls cannot
    // hit either debug precondition; the proof is in
    // quake_rs_ed_check_free_list.
    if num_edicts > max_edicts {
        *detail = max_edicts;
        return PRARENA_RAISE_EDICT_NUM;
    }

    let vmp = vm.as_ptr();
    // SAFETY: the free list lives inside the same qcvm_t; no other Rust
    // reference to it is live.
    let free_list: &mut FreeList = unsafe { &mut (*vmp).free_list };
    alloc::rebuild_free_list(
        free_list,
        &mut arena,
        num_edicts,
        force_free_reuse,
        &mut sort_free_edicts,
    );
    PRARENA_OK
}

/// `PR_GetString`'s core.
///
/// COMPAT: the invalid-offset arm returns `qcvm->strings` and never raises --
/// `pr_edict_arena.c:322`'s `return` precedes its `Host_Error`, so that arm
/// is dead code. Only a negative handle whose slot is null is live.
///
/// # Safety
///
/// The ambient `qcvm` must be selected; `out` must point at a writable
/// `const char *`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pr_get_string(num: c_int, out: *mut *const c_char) -> c_int {
    // SAFETY: the caller's frame owns it.
    let out = unsafe { &mut *out };
    // SAFETY: see ambient_vm(). No arena is built: PR_GetString also runs
    // during PR_LoadProgs, before the edict array exists.
    let vm = unsafe { ambient_vm() };
    match vm.get_string(num) {
        Ok(p) => {
            *out = p;
            PRARENA_OK
        }
        Err(StringError::NonExistent(_)) => {
            *out = core::ptr::null();
            PRARENA_RAISE_STRING_MISSING
        }
    }
}

/// `PR_ClearEngineString`.
///
/// # Safety
///
/// The ambient `qcvm` must be selected.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pr_clear_engine_string(num: c_int) {
    let mut sys = EngineArena::new();
    {
        // SAFETY: see ambient_vm().
        let mut vm = unsafe { ambient_vm() };
        vm.string_table().clear_engine_string(num, &mut sys);
    }
    sys.flush();
}

/// `PR_SetEngineString`.
///
/// Not raise-capable: `pr_edict_arena.c:351-353` wraps its only `Host_Error`
/// in `#if 0`, and the live `#else` arm just returns an offset into the progs
/// string blob. It therefore needs no status and no C trampoline.
///
/// # Safety
///
/// The ambient `qcvm` must be selected; `s` is null or a pointer whose
/// lifetime outlives the handle (C stores it in `knownstrings`).
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pr_set_engine_string(s: *const c_char) -> c_int {
    let mut sys = EngineArena::new();
    let handle = {
        // SAFETY: see ambient_vm().
        let mut vm = unsafe { ambient_vm() };
        vm.string_table().set_engine_string(s, &mut sys)
    };
    sys.flush();
    handle
}

/// `PR_AllocString`.
///
/// # Safety
///
/// The ambient `qcvm` must be selected; `ptr` is null or points at a writable
/// `char *`.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pr_alloc_string(size: c_int, ptr: *mut *mut c_char) -> c_int {
    // C's `if (!size) return 0;` -- before `*ptr` is ever written
    if size == 0 {
        return 0;
    }
    let mut sys = EngineArena::new();
    let (handle, buf) = {
        // SAFETY: see ambient_vm().
        let mut vm = unsafe { ambient_vm() };
        vm.string_table().alloc_string(size, &mut sys)
    };
    sys.flush();
    if !ptr.is_null() {
        // SAFETY: the caller's frame owns it.
        unsafe { *ptr = buf };
    }
    handle
}

/// `PR_ClearEdictStrings`.
///
/// # Safety
///
/// The ambient `qcvm` must be selected.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_pr_clear_edict_strings() {
    let mut sys = EngineArena::new();
    {
        // SAFETY: see ambient_vm().
        let mut vm = unsafe { ambient_vm() };
        vm.string_table().clear_edict_strings(&mut sys);
    }
    sys.flush();
}
