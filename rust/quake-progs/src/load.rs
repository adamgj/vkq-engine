//! The progs loader: `PR_LoadProgs`, `PR_MergeEngineFieldDefs`,
//! `PR_ClearProgs` and their helpers (`pr_edict_load.c`).
//!
//! Everything downstream of this module depends on decisions taken here: the
//! CRC and folded MD4 are computed **before** the in-place byteswap, all three
//! hash maps are built in reverse so a duplicate symbol resolves the way a
//! linear search would, the engine fielddef merge fixes `entityfields` and so
//! `edict_size` and so the savegame layout, and the re-release builtin patch
//! runs *after* `PR_EnableExtensions` and can therefore undo an extension
//! binding.
//!
//! The lookups that stay C — `ED_FindField`, `ED_FindGlobal`,
//! `ED_FindFunction` — read the hash maps this module builds, so the maps are
//! created through [`LoadSys`] rather than by `quake_util::hash_map` directly:
//! the object C dereferences must be the object C's `HashMap_Lookup` was
//! written against.

use core::ffi::{c_char, c_int, c_void, CStr};

use quake_types::progs::{
    etype, BuiltinT, DDef, DPrograms, Edict, EntVars, PrExtFields, QcVm, DEF_SAVEGLOBAL,
    PROGHEADER_CRC, PROG_VERSION,
};
use quake_util::crc::crc_block;
use quake_util::mdfour::block_checksum;

use crate::arena::Mem;
use crate::image::{DefTable, ProgsImage, VmLoad};

/// `server.h` — the 2021 re-release effect bits `PR_FindSupportedEffects`
/// masks off when the progs does not define them.
pub const EF_QEX_QUADLIGHT: c_int = 16;
pub const EF_QEX_PENTALIGHT: c_int = 32;
pub const EF_QEX_CANDLELIGHT: c_int = 64;

/// `pr_edict.c` `type_size[]`, indexed by `etype_t`.
const TYPE_SIZE: [c_int; 8] = [1, 1, 1, 3, 1, 1, 1, 1];

/// What the loader needs from code that has not moved yet.
///
/// The hash-map methods are deliberately opaque `*mut c_void` handles: the
/// maps are the engine's `hash_map_t`, and `ED_FindField` and friends look
/// them up from C.
pub trait LoadSys: Mem {
    /// `HashMap_Create (const char *, <ptr>, &HashStr, &HashStrCmp)`.
    fn map_create(&mut self) -> *mut c_void;
    /// `HashMap_Reserve`.
    fn map_reserve(&mut self, map: *mut c_void, capacity: c_int);
    /// `HashMap_Insert (map, &key, &value)` — both slots are pointer-sized.
    ///
    /// COMPAT: the map stores the *key pointer*, not a copy of the string, and
    /// compares by dereferencing it. Every caller here must therefore pass a
    /// pointer whose lifetime matches what C passed.
    fn map_insert(&mut self, map: *mut c_void, key: *const c_char, value: *const c_void);
    /// `HashMap_Destroy`.
    fn map_destroy(&mut self, map: *mut c_void);

    /// `ED_NewString`.
    fn ed_new_string(&mut self, s: *const c_char) -> c_int;
    /// `PR_SetEngineString ("")`.
    ///
    /// The literal stays C-side: `PR_SetEngineString` keys the known-string
    /// table on the *pointer*, so the empty string every later
    /// `PR_SetEngineString ("")` in the engine can match must be the C one.
    fn set_empty_engine_string(&mut self);
    /// `ED_FindFieldOffset` — `-1` when absent.
    fn find_field_ofs(&mut self, name: &CStr) -> c_int;
    /// `ED_FindGlobal` plus `PR_HasGlobal`'s type test, returning `G_FLOAT
    /// (g->ofs)` when the global exists and is an `ev_float`.
    fn global_float(&mut self, name: &CStr) -> Option<f32>;
    /// `ED_FindFunction` — the function's index, or `None`.
    fn find_function(&mut self, name: &CStr) -> Option<c_int>;

    /// `va ("%s_%c", name, 'x' + component)`.
    ///
    /// COMPAT (bug preserved deliberately): `PR_MergeEngineFieldDefs` inserts
    /// the vector-component fielddefs into `fielddefs_map` keyed on `va`'s
    /// **rotating temporary buffer**, so once eight more `va` calls have gone
    /// by, `ED_FindField ("colormod_x")` compares against whatever now
    /// occupies that buffer and misses. Reproducing that needs the engine's
    /// own `va`, not a Rust string: the divergence is not in *what* is
    /// inserted but in *which storage* the key points at.
    fn va_component_name(&mut self, name: &CStr, component: u8) -> *const c_char;

    /// Drain whatever `print`/`dprint`/`dwarn` have queued to the real
    /// console *now*.
    ///
    /// COMPAT: the loader defers console output because `Con_Printf` can reach
    /// `SCR_UpdateScreen` (Phase 5's lesson), but C emits its messages inline —
    /// so a bare drain-at-the-end reorders them past anything the C seams print
    /// mid-load, `PR_EnableExtensions` above all. Flushing where C prints keeps
    /// the `developer 1` transcript identical.
    fn flush_console(&mut self);

    /// `PR_ShutdownExtensions`.
    fn shutdown_extensions(&mut self);
    /// `PR_EnableExtensions (qcvm->globaldefs)`.
    fn enable_extensions(&mut self, globaldefs: *mut DDef);

    /// `PR_SwitchQCVM` — including its already-active `Sys_Error`.
    fn switch_qcvm(&mut self, vm: *mut QcVm);
    /// `qcvm = NULL;` written *directly*, which is how `PR_ClearProgs` gets
    /// past `PR_SwitchQCVM`'s already-active assertion. It leaves
    /// `pr_global_struct` alone, exactly as the C assignment does.
    fn deselect_qcvm(&mut self);
    /// The ambient `qcvm`.
    fn current_qcvm(&mut self) -> *mut QcVm;
    /// `pr_global_struct = (globalvars_t *)qcvm->globals;`
    fn set_pr_global_struct(&mut self, globals: *mut f32);

    /// `qcvm == &sv.qcvm` — `PR_FindSupportedEffects` only touches the server.
    fn is_server_vm(&mut self, vm: *mut QcVm) -> bool;
    /// `sv.effectsmask = ...`
    fn set_effects_mask(&mut self, mask: c_int);

    /// `Con_Printf`, deferred: the console is not a leaf (it can reach
    /// `SCR_UpdateScreen`), so it must not run while the loader holds its
    /// views of the VM.
    fn print(&mut self, msg: &[u8]);
    /// `Con_DPrintf`, deferred.
    fn dprint(&mut self, msg: &[u8]);
}

/// The `Host_Error`s `PR_LoadProgs` raises. Reported rather than raised so the
/// jump happens in a C frame (ADR-009).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// `%s has wrong version number (%i should be %i)`
    WrongVersion(c_int),
    /// `%s system vars have been modified, progdefs.h is out of date`
    CrcMismatch,
    /// `%s strings go past end of file\n`
    StringsPastEnd,
    /// `PR_LoadProgs: pr_fielddefs[i].type & DEF_SAVEGLOBAL`
    FieldDefSaveGlobal,
    /// A strings lump whose last byte is not a NUL, or an empty one.
    ///
    /// COMPAT (accepted divergence): every symbol name the loader hashes is a
    /// `PR_GetString` pointer into this blob, and `HashStr` runs `strlen` on
    /// it. C's only guard is `ofs_strings + numstrings >= com_filesize`, which
    /// says nothing about a terminator — so a blob whose tail has no NUL makes
    /// the very first hash-map insert read past the file. Reachable from mod
    /// data; found by `fuzz_progs_load`. A `progs.dat` from any compiler ends
    /// its string table with a NUL.
    ///
    /// `numstrings == 0` takes this arm too: with an empty blob and any
    /// symbol to hash, C reads off the end for the same reason. C itself
    /// loads such a file — `stringssize` is 0, every `PR_GetString` takes the
    /// out-of-range arm and returns `qcvm->strings` — so refusing it is a
    /// deliberate divergence, not a reproduction.
    UnterminatedStrings,
    /// A file shorter than a `dprograms_t` header.
    ///
    /// COMPAT (accepted divergence): C's very first act after `COM_LoadFile`
    /// is to byteswap `sizeof (*qcvm->progs) / 4` words **in place**, before
    /// any validation, so a file shorter than 60 bytes is a heap overflow
    /// before the version check runs — and `PR_MergeEngineFieldDefs` later
    /// writes `numfielddefs` and `entityfields` back into the same header.
    /// Reachable from mod data. Found by `fuzz_progs_load`.
    TooShort(usize),
    /// `entityfields` so large that `entityfields * 4 + sizeof (edict_t) -
    /// sizeof (entvars_t)` does not fit in an `int`.
    ///
    /// COMPAT (accepted divergence): C computes `edict_size` with signed
    /// `int` arithmetic and lets it overflow — UB, wrapping in practice — and
    /// then allocates `max_edicts * edict_size` from the wrapped value.
    /// Reachable from mod data: `entityfields` is a header field of an
    /// untrusted `progs.dat`. The port refuses instead. Found by
    /// `fuzz_progs_load` on its first run.
    BadEntityFields(c_int),
    /// A lump whose offset and count do not fit inside the file.
    ///
    /// COMPAT (accepted divergence): C computes every lump base as
    /// `(byte *)progs + ofs` and walks `count` entries with no validation, so
    /// a malformed `progs.dat` is an out-of-bounds read and an in-place
    /// byteswap *write* past the buffer — reachable from mod data. The port
    /// refuses instead. No shipping progs reaches it (trace parity over
    /// eight game/map combinations is byte-identical), so the divergence is
    /// unobservable for valid input.
    LumpOutOfRange,
}

/// The engine fields `PR_MergeEngineFieldDefs` adds when a progs does not
/// already define them, in the order C's `extrafields[]` lists them — the
/// order decides the offsets, and therefore `edict_size`.
const EXTRAFIELDS: [(&CStr, c_int); 8] = [
    (c"alpha", etype::EV_FLOAT),
    (c"scale", etype::EV_FLOAT),
    (c"emiteffectnum", etype::EV_FLOAT),
    (c"traileffectnum", etype::EV_FLOAT),
    (c"tag_entity", etype::EV_FLOAT),
    (c"tag_index", etype::EV_FLOAT),
    (c"modelflags", etype::EV_FLOAT),
    (c"colormod", etype::EV_VECTOR),
];

/// `HashMap_Reserve (fielddefs_map, numfielddefs + countof (extrafields) * 3)`
/// — the margin assumes every autofield could be a vector.
const EXTRAFIELDS_RESERVE_MARGIN: c_int = EXTRAFIELDS.len() as c_int * 3;

/// The most words `PR_MergeEngineFieldDefs` can add to `entityfields`: one per
/// float autofield and three for `colormod`. Headroom for the `edict_size`
/// overflow bound.
const EXTRAFIELDS_MAX_WORDS: i64 = EXTRAFIELDS.len() as i64 * 3;

/// The `QCEXTFIELD` names, in `progs.h` declaration order. The order matters
/// only for the console — every entry is an independent `ED_FindFieldOffset`
/// — but it is kept so the mirror and the macro list can be diffed by eye.
const EXTFIELD_NAMES: [&CStr; 27] = [
    c"alpha",
    c"scale",
    c"colormod",
    c"tag_entity",
    c"tag_index",
    c"modelflags",
    c"origin",
    c"angles",
    c"frame",
    c"skin",
    c"customphysics",
    c"gravity",
    c"items2",
    c"movement",
    c"nodrawtoclient",
    c"drawonlytoclient",
    c"traileffectnum",
    c"emiteffectnum",
    c"button3",
    c"button4",
    c"button5",
    c"button6",
    c"button7",
    c"button8",
    c"viewzoom",
    c"SendEntity",
    c"SendFlags",
];

fn extfields_from(v: [c_int; 27]) -> PrExtFields {
    PrExtFields {
        alpha: v[0],
        scale: v[1],
        colormod: v[2],
        tag_entity: v[3],
        tag_index: v[4],
        modelflags: v[5],
        origin: v[6],
        angles: v[7],
        frame: v[8],
        skin: v[9],
        customphysics: v[10],
        gravity: v[11],
        items2: v[12],
        movement: v[13],
        nodrawtoclient: v[14],
        drawonlytoclient: v[15],
        traileffectnum: v[16],
        emiteffectnum: v[17],
        button3: v[18],
        button4: v[19],
        button5: v[20],
        button6: v[21],
        button7: v[22],
        button8: v[23],
        viewzoom: v[24],
        send_entity: v[25],
        send_flags: v[26],
    }
}

/// `PR_ClearProgs`.
///
/// The caller has already established that `vm->progs` is non-NULL — the
/// early `if (!vm->progs) return;` lives in the shim, which is also where the
/// `edict_size`/`sizeof (edict_t)` arithmetic that would otherwise need a
/// loaded VM is avoided.
pub fn clear_progs(load: &mut VmLoad, sys: &mut dyn LoadSys) {
    let oldvm = sys.current_qcvm();
    let vm = load.as_ptr();
    if load.progs().is_null() {
        return;
    }

    sys.deselect_qcvm();
    sys.switch_qcvm(vm);
    sys.shutdown_extensions();

    if !load.knownstrings().is_null() {
        for i in 0..load.numknownstrings() {
            let (p, owned) = load.known_string(i);
            if owned {
                sys.free(p.cast_mut().cast());
            }
        }
        sys.free(load.knownstrings().cast());
        sys.free(load.knownstringsowned().cast());
    }
    sys.free(load.edicts().cast());

    // The merge reallocates `fielddefs` away from the image; C tells the two
    // ownership states apart by comparing against the image's own lump. When
    // the image is too short to hold a header the merge cannot have run, so
    // `fielddefs` is never an allocator block and nothing is freed here.
    if let Some(lump) = load.image_fielddefs() {
        if load.fielddefs().cast::<u8>() != lump {
            sys.free(load.fielddefs().cast());
        }
    }
    sys.free(load.progs().cast());

    sys.map_destroy(load.function_map());
    sys.map_destroy(load.fielddefs_map());
    sys.map_destroy(load.globaldefs_map());
    load.zero();

    sys.deselect_qcvm();
    sys.switch_qcvm(oldvm);
}

/// `PR_LoadProgs`, from the point where `COM_LoadFile` has returned a buffer.
///
/// `Ok(true)` is C's `return true`; `Ok(false)` is one of the non-fatal arms,
/// whose console message has already been queued on `sys`.
///
/// COMPAT: the non-fatal arms set `qcvm->progs = NULL` and return without
/// freeing the buffer — a leak in C, preserved rather than fixed so a
/// sanitizer run over the mixed build reports the same thing the C oracle
/// does.
///
/// `load` must be the ambient, `PR_SwitchQCVM`-selected VM, and `image` the
/// `COM_LoadFile` block it takes ownership of.
#[allow(clippy::too_many_arguments)]
pub fn load_progs(
    load: &mut VmLoad,
    image: &mut ProgsImage,
    filename: &CStr,
    fatal: bool,
    needcrc: c_int,
    builtins: &[BuiltinT],
    sys: &mut dyn LoadSys,
) -> Result<bool, LoadError> {
    let vm = load.as_ptr();
    let len = image.len();

    load.set_progs(image.base().cast());
    load.set_progssize(len as u32);
    if len < core::mem::size_of::<DPrograms>() {
        return Err(LoadError::TooShort(len));
    }
    load.set_progscrc(crc_block(image.bytes()));
    load.set_progshash(block_checksum(image.bytes()));

    image.swap_header();
    let hdr = image.header();

    if hdr.version != PROG_VERSION {
        if fatal {
            return Err(LoadError::WrongVersion(hdr.version));
        }
        sys.print(&join(&[filename.to_bytes(), b" ABI set not supported\n"]));
        load.set_progs(core::ptr::null_mut());
        return Ok(false);
    }

    if hdr.crc != needcrc {
        if fatal {
            return Err(LoadError::CrcMismatch);
        }
        sys.print(&join(&[filename.to_bytes(), foreign_crc_message(hdr.crc)]));
        load.set_progs(core::ptr::null_mut());
        return Ok(false);
    }

    let mut kb = itoa(len as u64 / 1024);
    let mut occupies = filename.to_bytes().to_vec();
    occupies.extend_from_slice(b" occupies ");
    occupies.append(&mut kb);
    occupies.extend_from_slice(b"K.\n");
    sys.dprint(&occupies);
    // C prints this inline, before `PR_EnableExtensions` emits any of its own
    // messages; drain here so the ordering survives the deferral.
    sys.flush_console();

    // C's own check, kept ahead of the added bounds pass so a truncated
    // progs.dat still reports the message C reports. C sums two `int`s and
    // compares against `com_filesize`; the sum is widened here because the
    // overflow C would have is not a behaviour worth reproducing.
    if i64::from(hdr.ofs_strings) + i64::from(hdr.numstrings) >= len as i64 {
        return Err(LoadError::StringsPastEnd);
    }

    // C computes every lump base by pointer arithmetic and never validates it
    // (see LoadError::LumpOutOfRange).
    // `numstrings` is included here purely to reject a *negative* count: C's
    // own `ofs_strings + numstrings >= com_filesize` test above already
    // rejects everything that runs past the end, and a negative sum sails
    // through it into `qcvm->stringssize`, which is a bound.
    if !image.lump_fits(hdr.ofs_strings, hdr.numstrings, 1)
        || !image.lump_fits(hdr.ofs_statements, hdr.numstatements, 8)
        || !image.lump_fits(hdr.ofs_functions, hdr.numfunctions, 36)
        || !image.lump_fits(hdr.ofs_globaldefs, hdr.numglobaldefs, 8)
        || !image.lump_fits(hdr.ofs_fielddefs, hdr.numfielddefs, 8)
        || !image.lump_fits(hdr.ofs_globals, hdr.numglobals, 4)
    {
        return Err(LoadError::LumpOutOfRange);
    }

    // `edict_size` is `entityfields * 4 + header`, rounded up, in `int`
    // arithmetic; the merge can add ten more words. Bound `entityfields` here
    // so none of that can overflow (see LoadError::BadEntityFields), and so
    // the arena's stride stays a sane positive number.
    let edict_header = (core::mem::size_of::<Edict>() - core::mem::size_of::<EntVars>()) as i64;
    let ptr_align = core::mem::size_of::<*const c_void>() as i64;
    let max_entityfields =
        (i64::from(c_int::MAX) - edict_header - ptr_align + 1) / 4 - EXTRAFIELDS_MAX_WORDS;
    if hdr.entityfields < 0 || i64::from(hdr.entityfields) > max_entityfields {
        return Err(LoadError::BadEntityFields(hdr.entityfields));
    }

    // Every symbol name reaching the hash maps is `strings + s_name`, and the
    // engine's `HashStr` calls `strlen` on it (see
    // LoadError::UnterminatedStrings).
    if image.strings_last_byte(hdr.ofs_strings, hdr.numstrings) != Some(0) {
        return Err(LoadError::UnterminatedStrings);
    }

    load.set_functions(image.at(hdr.ofs_functions).cast());
    load.set_strings(image.strings_ptr(hdr.ofs_strings));

    load.set_globaldefs(image.at(hdr.ofs_globaldefs).cast());
    load.set_fielddefs(image.at(hdr.ofs_fielddefs).cast());
    load.set_statements(image.at(hdr.ofs_statements).cast());

    let globals = image.at(hdr.ofs_globals).cast::<f32>();
    load.set_globals(globals);
    sys.set_pr_global_struct(globals);

    load.set_stringssize(hdr.numstrings);

    image.swap_statements(hdr.ofs_statements, hdr.numstatements);
    image.swap_functions(hdr.ofs_functions, hdr.numfunctions);

    // Reverse insert, all three: there can be duplicate symbols, and C wants
    // hash lookup to return what a linear search would (the first match).
    let function_map = sys.map_create();
    sys.map_reserve(function_map, hdr.numfunctions);
    load.set_function_map(function_map);
    for i in (0..hdr.numfunctions).rev() {
        let name = load.blob_string(image.function_s_name(hdr.ofs_functions, i), hdr.numstrings);
        let ptr = image.function_ptr(hdr.ofs_functions, i);
        sys.map_insert(function_map, name, ptr.cast());
    }

    image.swap_defs(hdr.ofs_globaldefs, hdr.numglobaldefs);
    let globaldefs_map = sys.map_create();
    sys.map_reserve(globaldefs_map, hdr.numglobaldefs);
    load.set_globaldefs_map(globaldefs_map);
    for i in (0..hdr.numglobaldefs).rev() {
        let name = load.blob_string(image.def(hdr.ofs_globaldefs, i).s_name, hdr.numstrings);
        sys.map_insert(
            globaldefs_map,
            name,
            image.def_ptr(hdr.ofs_globaldefs, i).cast(),
        );
    }

    image.swap_defs(hdr.ofs_fielddefs, hdr.numfielddefs);
    for i in 0..hdr.numfielddefs {
        if image.def_type(hdr.ofs_fielddefs, i) & DEF_SAVEGLOBAL != 0 {
            return Err(LoadError::FieldDefSaveGlobal);
        }
    }
    let fielddefs_map = sys.map_create();
    sys.map_reserve(fielddefs_map, hdr.numfielddefs + EXTRAFIELDS_RESERVE_MARGIN);
    load.set_fielddefs_map(fielddefs_map);
    for i in (0..hdr.numfielddefs).rev() {
        let name = load.blob_string(image.def(hdr.ofs_fielddefs, i).s_name, hdr.numstrings);
        sys.map_insert(
            fielddefs_map,
            name,
            image.def_ptr(hdr.ofs_fielddefs, i).cast(),
        );
    }

    image.swap_globals(hdr.ofs_globals, hdr.numglobals);

    load.copy_builtins(builtins);
    load.set_numbuiltins(builtins.len() as c_int);

    merge_engine_field_defs(load, image, sys);

    let mut ofs = [0; 27];
    for (slot, name) in ofs.iter_mut().zip(EXTFIELD_NAMES) {
        *slot = sys.find_field_ofs(name);
    }
    load.set_extfields(extfields_from(ofs));

    // `entityfields * 4 + sizeof (edict_t) - sizeof (entvars_t)`, rounded up
    // to pointer alignment so engine data in the edict stays aligned.
    //
    // The header size is `sizeof (edict_t) - sizeof (entvars_t)`, not
    // `offsetof (edict_t, v)`: `entvars_t` is 4-aligned while `edict_t`
    // contains pointers, so trailing padding can make the two differ.
    let header = (core::mem::size_of::<Edict>() - core::mem::size_of::<EntVars>()) as c_int;
    let align = core::mem::size_of::<*const c_void>() as c_int;
    let size = image.header().entityfields * 4 + header + align - 1;
    load.set_edict_size(size & !(align - 1));

    sys.set_empty_engine_string();
    sys.enable_extensions(load.globaldefs());
    patch_rerelease_builtins(image, hdr, sys);
    find_supported_effects(vm, sys);

    load.set_progsstrings(load.numknownstrings());
    Ok(true)
}

/// `PR_MergeEngineFieldDefs` — registers the engine's extension fields so a
/// map can use `.alpha`/`.scale` without the mod declaring them.
///
/// This is what makes `edict_size` mod-dependent, and therefore what decides
/// savegame layout.
fn merge_engine_field_defs(load: &mut VmLoad, image: &mut ProgsImage, sys: &mut dyn LoadSys) {
    let hdr = image.header();
    let mut maxofs = hdr.entityfields;
    let mut maxdefs = hdr.numfielddefs;

    // `(offset, is_new)`. C does not keep the flag: its second loop re-derives
    // "is this one new?" from `newidx >= entityfields && newidx < maxofs`.
    //
    // COMPAT (accepted divergence): that re-derivation is wrong for a progs
    // that *declares* one of these field names at an offset at or past
    // `entityfields` — C then emits a def it did not count, and writes past
    // the `Mem_Alloc (maxdefs * sizeof (ddef_t))` block it just allocated. A
    // heap overflow reachable from mod data. Carrying the flag keeps the two
    // loops in agreement by construction. Unreachable from a well-formed
    // progs, where every fielddef offset is below `entityfields`; found by
    // `fuzz_progs_load`.
    let mut newidx = [(0, false); EXTRAFIELDS.len()];
    for (slot, (name, ty)) in newidx.iter_mut().zip(EXTRAFIELDS) {
        slot.0 = sys.find_field_ofs(name);
        if slot.0 < 0 {
            *slot = (maxofs, true);
            maxdefs += 1;
            if ty == etype::EV_VECTOR {
                maxdefs += 3;
            }
            maxofs += TYPE_SIZE[ty as usize];
        }
    }

    if maxdefs == hdr.numfielddefs {
        return;
    }

    let olddefs = load.fielddefs();
    let old = load.fielddef_table(hdr.numfielddefs as usize);
    let mut defs = DefTable::realloc_from(sys, maxdefs as usize, &old, hdr.numfielddefs as usize);
    if olddefs.cast::<u8>() != image.at(hdr.ofs_fielddefs) {
        sys.free(olddefs.cast());
    }
    load.set_fielddefs(defs.base());

    let map = load.fielddefs_map();
    let mut numfielddefs = hdr.numfielddefs;
    for (&(idx, is_new), (name, ty)) in newidx.iter().zip(EXTRAFIELDS) {
        if !is_new {
            continue;
        }
        debug_assert!(idx >= hdr.entityfields && idx < maxofs, "C's own test");
        defs.set(
            numfielddefs as usize,
            DDef {
                type_: ty as u16,
                ofs: idx as u16,
                s_name: sys.ed_new_string(name.as_ptr()),
            },
        );
        sys.map_insert(map, name.as_ptr(), defs.ptr(numfielddefs as usize).cast());
        numfielddefs += 1;
        image.set_numfielddefs(numfielddefs);

        if ty != etype::EV_VECTOR {
            continue;
        }
        for a in 0..3u8 {
            // COMPAT: the key stored in the map is this `va` pointer, into a
            // rotating temporary buffer — see `LoadSys::va_component_name`.
            let component = sys.va_component_name(name, a);
            defs.set(
                numfielddefs as usize,
                DDef {
                    type_: etype::EV_FLOAT as u16 | DEF_SAVEGLOBAL,
                    ofs: (idx + c_int::from(a)) as u16,
                    s_name: sys.ed_new_string(component),
                },
            );
            sys.map_insert(map, component, defs.ptr(numfielddefs as usize).cast());
            numfielddefs += 1;
            image.set_numfielddefs(numfielddefs);
        }
    }
    image.set_entityfields(maxofs);
}

/// `PR_PatchRereleaseBuiltins` — Update-1 of the 2021 re-release moved three
/// builtins to new ordinals; they are patched back **only** when the
/// `first_statement` matches exactly, and this runs *after*
/// `PR_EnableExtensions`, so it can undo an extension binding.
fn patch_rerelease_builtins(image: &mut ProgsImage, hdr: DPrograms, sys: &mut dyn LoadSys) {
    const EXBUILTINS: [(&CStr, c_int, c_int); 3] = [
        (c"centerprint", -90, -73),
        (c"bprint", -91, -23),
        (c"sprint", -92, -24),
    ];
    for (name, from, to) in EXBUILTINS {
        let Some(index) = sys.find_function(name) else {
            continue;
        };
        if index < 0 || index >= hdr.numfunctions {
            continue;
        }
        if image.function_first_statement(hdr.ofs_functions, index) == from {
            image.set_function_first_statement(hdr.ofs_functions, index, to);
        }
    }
}

/// `PR_FindSupportedEffects` — disables the 2021 re-release effect bits when
/// the progs does not define them, because Arcane Dimensions uses the same
/// bits for its own effects.
fn find_supported_effects(vm: *mut QcVm, sys: &mut dyn LoadSys) {
    if !sys.is_server_vm(vm) {
        return;
    }
    let has = |sys: &mut dyn LoadSys, name: &CStr, value: f32| {
        sys.global_float(name).is_some_and(|v| v == value)
    };
    let isqex = has(sys, c"EF_QUADLIGHT", EF_QEX_QUADLIGHT as f32)
        && (has(sys, c"EF_PENTLIGHT", EF_QEX_PENTALIGHT as f32)
            || has(sys, c"EF_PENTALIGHT", EF_QEX_PENTALIGHT as f32));
    // C writes `-1 & ~(...)`; the mask is all-ones with those three bits off.
    let mask = if isqex {
        -1
    } else {
        !(EF_QEX_QUADLIGHT | EF_QEX_PENTALIGHT | EF_QEX_CANDLELIGHT)
    };
    sys.set_effects_mask(mask);
}

/// `PR_LoadProgs`' hardcoded foreign-CRC diagnostic switch. The text must
/// match byte for byte: it is what a user sees when a mod ships gamecode this
/// engine cannot run.
fn foreign_crc_message(crc: c_int) -> &'static [u8] {
    match crc {
        22390 => b" - full csqc is not supported\n".as_slice(),
        52195 => b" - obsolete csqc is not supported\n",
        54730 => b" - quakeworld gamecode is not supported\n",
        26940 => b" - prerelease gamecode is not supported\n",
        32401 => b" - tenebrae gamecode is not supported\n",
        // hexen2 release, mission pack and demo share one message
        38488 | 26905 | 14046 => b" - hexen2 gamecode is not supported\n",
        // 5927 is PROGHEADER_CRC itself and cannot reach here
        _ => b" system vars are not supported\n",
    }
}

const _: () = assert!(PROGHEADER_CRC == 5927);

fn join(parts: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(parts.iter().map(|p| p.len()).sum());
    for p in parts {
        out.extend_from_slice(p);
    }
    out
}

/// `%u` of a `size_t` — the only conversion the loader's console output needs
/// beyond `%s`.
fn itoa(mut v: u64) -> Vec<u8> {
    if v == 0 {
        return vec![b'0'];
    }
    let mut out = Vec::new();
    while v != 0 {
        out.push(b'0' + (v % 10) as u8);
        v /= 10;
    }
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreign_crc_messages_match_the_c_switch() {
        assert_eq!(
            foreign_crc_message(22390),
            b" - full csqc is not supported\n"
        );
        assert_eq!(
            foreign_crc_message(26905),
            foreign_crc_message(38488),
            "the three hexen2 CRCs share one arm"
        );
        assert_eq!(
            foreign_crc_message(0),
            b" system vars are not supported\n",
            "the default arm has no leading dash"
        );
    }

    #[test]
    fn itoa_matches_printf_u() {
        assert_eq!(itoa(0), b"0");
        assert_eq!(itoa(1), b"1");
        assert_eq!(itoa(1024), b"1024");
        assert_eq!(itoa(u64::from(u32::MAX)), b"4294967295");
    }

    #[test]
    fn extrafields_sizes_match_type_size() {
        assert_eq!(TYPE_SIZE[etype::EV_FLOAT as usize], 1);
        assert_eq!(TYPE_SIZE[etype::EV_VECTOR as usize], 3);
        assert_eq!(EXTRAFIELDS_RESERVE_MARGIN, 24);
    }
}
