//! C-ABI shim for the savegame/entity-text value parser
//! (`pr_edict_parse.c` → `quake-progs`).
//!
//! `Quake/pr_edict_parse_glue.c` owns the platform libc conversions, the
//! engine lookups that are still C, and the one raise (ADR-009).

use core::ffi::{c_char, c_int, CStr};

use quake_c_sys as c;
use quake_progs::alloc::{AllocError, FreeListOverflow};
use quake_progs::arena::{EdictArena, EdictId, Mem, VmRaw};
use quake_progs::parse::{self, ParseError, ParseSys};
use quake_progs::save::value_words;
use quake_types::progs::{FreeList, QcVm, DEF_SAVEGLOBAL};

/// Status codes shared with `Quake/pr_edict_parse_glue.c` (keep in sync).
const PRPARSE_OK: c_int = 0;
const PRPARSE_FALSE: c_int = 1;
const PRPARSE_ERR_ENTITY_RANGE: c_int = 2;
const PRPARSE_ERR_BAD_EDICT_NUM: c_int = 3;
const PRPARSE_ERR_FREELIST_FULL: c_int = 4;
const PRPARSE_ERR_FREELIST_OVER_MAX: c_int = 5;

/// Concatenate the pieces of a console message. Kept as raw bytes, not
/// `String`: Quake strings carry high-bit bytes (the coloured-text charset)
/// and a lossy UTF-8 round trip would print different bytes than C's `%s`.
fn join(parts: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(parts.iter().map(|p| p.len()).sum::<usize>() + 1);
    for p in parts {
        // an embedded NUL would truncate the C string; drop them, as no
        // progs string can contain one anyway
        out.extend(p.iter().copied().filter(|&b| b != 0));
    }
    out
}

struct EngineParse {
    /// Console output accumulated rather than printed inline: `Con_Printf` is
    /// not a leaf (it can reach `SCR_UpdateScreen`), so it must not run while
    /// the parser holds its views of the VM.
    pending: Vec<(u8, Vec<u8>)>,
}

impl Mem for EngineParse {
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
        self.pending.push((
            2,
            format!("PR_AllocStringSlots: realloc'ing for {maxknownstrings} slots\n").into_bytes(),
        ));
    }
}

impl ParseSys for EngineParse {
    fn atof(&mut self, s: &CStr) -> f64 {
        // SAFETY: a leaf libc call on a NUL-terminated string.
        unsafe { c::PRParse_Glue_Atof(s.as_ptr()) }
    }

    fn atoi(&mut self, s: &CStr) -> c_int {
        // SAFETY: as above.
        unsafe { c::PRParse_Glue_Atoi(s.as_ptr()) }
    }

    fn strtoll(&mut self, s: &CStr) -> i64 {
        // SAFETY: as above.
        unsafe { c::PRParse_Glue_Strtoll(s.as_ptr()) }
    }

    fn strtoull(&mut self, s: &CStr) -> u64 {
        // SAFETY: as above.
        unsafe { c::PRParse_Glue_Strtoull(s.as_ptr()) }
    }

    fn find_field_ofs(&mut self, name: &CStr) -> Option<c_int> {
        // SAFETY: a hash lookup in pr_edict.c over the loaded fielddefs.
        let ofs = unsafe { c::PRParse_Glue_FindFieldOfs(name.as_ptr()) };
        (ofs >= 0).then_some(ofs)
    }

    fn find_function(&mut self, name: &CStr) -> Option<c_int> {
        // SAFETY: as above, over the loaded functions.
        let i = unsafe { c::PRParse_Glue_FindFunction(name.as_ptr()) };
        (i >= 0).then_some(i)
    }

    fn unlink_edict(&mut self, id: EdictId) {
        // SAFETY: `id` is below num_edicts, which the parser just raised to
        // cover it, so EDICT_NUM's bounds check passes. SV_UnlinkEdict is
        // world.c's area-tree removal and does not raise.
        unsafe { c::PRParse_Glue_UnlinkEdict(id.0 as c_int) }
    }

    fn dprint(&mut self, prefix: &str, arg: &[u8], suffix: &str) {
        self.pending
            .push((1, join(&[prefix.as_bytes(), arg, suffix.as_bytes()])));
    }

    fn print(&mut self, prefix: &str, arg: &[u8], suffix: &str) {
        self.pending
            .push((0, join(&[prefix.as_bytes(), arg, suffix.as_bytes()])));
    }

    fn dwarn(&mut self, prefix: &str, arg: &[u8], mid: &str, arg2: &[u8], suffix: &str) {
        self.pending.push((
            3,
            join(&[
                prefix.as_bytes(),
                arg,
                mid.as_bytes(),
                arg2,
                suffix.as_bytes(),
            ]),
        ));
    }
}

impl EngineParse {
    fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    /// Drain the accumulated console output. Called only after every view of
    /// the VM has been dropped.
    fn flush(&mut self) {
        for (kind, msg) in self.pending.drain(..) {
            let Ok(c_msg) = std::ffi::CString::new(msg) else {
                continue;
            };
            // SAFETY: "%s" plus a NUL-terminated argument; no borrow of the
            // VM is live here.
            unsafe {
                match kind {
                    1 => c::Con_DPrintf(c"%s".as_ptr(), c_msg.as_ptr()),
                    2 => c::Con_DPrintf2(c"%s".as_ptr(), c_msg.as_ptr()),
                    3 => c::Con_DWarning(c"%s".as_ptr(), c_msg.as_ptr()),
                    _ => c::Con_Printf(c"%s".as_ptr(), c_msg.as_ptr()),
                }
            }
        }
    }
}

/// # Safety
///
/// The ambient `qcvm` must be selected.
unsafe fn ambient_vm() -> VmRaw {
    // SAFETY: the ambient qcvm is the VM the host frame selected.
    unsafe { VmRaw::new(c::qcvm.cast::<QcVm>()) }
}

/// An arena over the ambient VM's edict array.
///
/// Only the `ev_entity` branch of `ED_ParseEpair` needs one. It is built
/// separately from [`ambient_vm`] because `ED_NewString` also runs *during*
/// `PR_LoadProgs` — `PR_MergeEngineFieldDefs` calls it before `edict_size` and
/// `edicts` exist — and must not require an edict array to be there yet.
///
/// # Safety
///
/// The ambient `qcvm` must be loaded, i.e. past the point in `PR_LoadProgs`
/// where `edict_size` is computed and `edicts` allocated.
unsafe fn ambient_arena(vm: &VmRaw) -> EdictArena {
    let stride = vm.edict_stride() as usize;
    let count = vm.max_edicts().max(0) as usize;
    // SAFETY: the edict array is `max_edicts * edict_size` bytes, allocated
    // by PR_LoadProgs and live for the VM's lifetime.
    unsafe { EdictArena::borrowed(vm.edicts_base(), stride, count) }
}

/// `ED_NewString`.
///
/// # Safety
///
/// `s` must be NUL-terminated; the ambient `qcvm` must be loaded.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ed_new_string(s: *const c_char) -> c_int {
    // SAFETY: the caller passes a NUL-terminated progs/parser token.
    let s = unsafe { CStr::from_ptr(s) };
    // SAFETY: see ambient_vm(). No arena is built: this runs during
    // PR_LoadProgs too, before the edict array exists.
    let mut vm = unsafe { ambient_vm() };
    let mut sys = EngineParse::new();
    let handle = parse::ed_new_string(&mut vm, &mut sys, s);
    drop(vm);
    sys.flush();
    handle
}

/// `ED_ParseEpair`.
///
/// # Safety
///
/// `base` must point at the block `key->ofs` indexes (the globals or an
/// edict's field block), `s` must be NUL-terminated, and the ambient `qcvm`
/// must be loaded.
#[no_mangle]
pub unsafe extern "C" fn quake_rs_ed_parse_epair(
    base: *mut c_int,
    key_type: u16,
    key_ofs: u16,
    key_s_name: c_int,
    s: *const c_char,
    zoned: bool,
    detail: *mut c_int,
) -> c_int {
    // SAFETY: the caller's frame owns it.
    let detail = unsafe { &mut *detail };
    *detail = 0;

    // SAFETY: the caller passes a NUL-terminated token.
    let s = unsafe { CStr::from_ptr(s) };

    // Exactly the words this type occupies: an over-wide slice would reach
    // past the globals block (or the last edict) for a def at its tail, which
    // `slice::from_raw_parts_mut` forbids even without a write.
    let words = value_words(c_int::from(key_type));

    // SAFETY: `base + key_ofs` is where C's `(int *)base + key->ofs` points,
    // and the block has room for the key's type -- `words` is exactly that.
    let dest = unsafe { core::slice::from_raw_parts_mut(base.add(usize::from(key_ofs)), words) };

    // SAFETY: see ambient_vm()/ambient_arena(); ED_ParseEpair only runs from
    // ED_ParseEdict/ED_ParseGlobals, i.e. after the VM is fully loaded.
    //
    // `dest` and `arena` cover the same allocation when the caller is
    // ED_ParseEdict, whose `base` is `&ed->v`. They cannot overlap, and the
    // invariant that guarantees it lives in the callers rather than here:
    // both `ED_LoadFromFile` (host_cmd.c, via SV_SpawnServer) and the
    // savegame loader raise `qcvm->num_edicts` above the edict they are about
    // to parse *before* calling ED_ParseEdict. The only arena writes this
    // function performs are in the `ev_entity` arm, which touches edicts in
    // `num_edicts .. loaded_ent_num` and `loaded_ent_num` itself -- all at
    // indices >= num_edicts, hence strictly above the parsed edict. See the
    // aliasing note on `EdictArena::borrowed`.
    let mut vm = unsafe { ambient_vm() };
    let mut arena = unsafe { ambient_arena(&vm) };
    // SAFETY: the free list lives inside the same qcvm_t; no other Rust
    // reference to it is live.
    let free_list: &mut FreeList = unsafe { &mut (*vm.as_ptr()).free_list };

    let mut sys = EngineParse::new();
    let result = parse::ed_parse_epair(
        &mut vm,
        &mut arena,
        free_list,
        &mut sys,
        dest,
        c_int::from(key_type),
        key_s_name,
        s,
        zoned,
    );

    drop(arena);
    drop(vm);
    sys.flush();

    match result {
        Ok(true) => PRPARSE_OK,
        Ok(false) => PRPARSE_FALSE,
        Err(ParseError::EntityTooLarge { num, .. }) => {
            *detail = num;
            PRPARSE_ERR_ENTITY_RANGE
        }
        Err(ParseError::BadEdictNum(n)) => {
            *detail = n;
            PRPARSE_ERR_BAD_EDICT_NUM
        }
        Err(ParseError::FreeListFull { kind, max_edicts }) => match kind {
            FreeListOverflow::Full => {
                *detail = 0;
                PRPARSE_ERR_FREELIST_FULL
            }
            FreeListOverflow::OverMaxEdicts => {
                *detail = max_edicts;
                PRPARSE_ERR_FREELIST_OVER_MAX
            }
        },
        Err(ParseError::Alloc(AllocError::NoFreeEdicts { max_edicts })) => {
            *detail = max_edicts;
            PRPARSE_ERR_ENTITY_RANGE
        }
    }
}
