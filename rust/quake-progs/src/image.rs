//! Raw access to a `progs.dat` image while it is being loaded, and to the
//! `qcvm_t` fields the loader fills in (`pr_edict_load.c` `PR_LoadProgs`).
//!
//! # Unsafe posture (ADR-004)
//!
//! This is the crate's second unsafe island, alongside [`crate::arena`]. It
//! exists for the same reason: the progs image is untyped C memory whose
//! interior layout is decided by the file header at runtime, so nothing here
//! can be expressed as a Rust struct. [`crate::load`] stays
//! `deny(unsafe_code)` and drives the image only through this view.
//!
//! Every lump read goes through `read_unaligned`. C dereferences
//! `(dstatement_t *)((byte *)progs + ofs_statements)` directly, so a hostile
//! `progs.dat` with an unaligned lump offset is UB in C and merely slow here;
//! the loader must not add alignment UB of its own on top of the bounds
//! checks it already performs.

#![allow(unsafe_code)]

use core::ffi::{c_char, c_int, c_void};

use quake_types::progs::{
    BuiltinT, DDef, DFunction, DPrograms, DStatement, Edict, PrExtFields, QBoolean, QcVm,
    MAX_BUILTINS,
};

/// Number of `int`s in `dprograms_t` — the header byteswap's loop bound,
/// `sizeof (*qcvm->progs) / 4` in C.
pub const DPROGRAMS_INTS: usize = core::mem::size_of::<DPrograms>() / 4;

/// A loaded `progs.dat` file image.
///
/// A view, not an owner: the buffer comes from `COM_LoadFile` and is released
/// by `PR_ClearProgs`.
pub struct ProgsImage {
    base: *mut u8,
    len: usize,
}

impl ProgsImage {
    /// # Safety
    ///
    /// `base` must point at `len` writable, initialised bytes that outlive the
    /// view, and no other Rust reference may alias them.
    #[must_use]
    pub unsafe fn new(base: *mut u8, len: usize) -> Self {
        Self { base, len }
    }

    #[must_use]
    pub fn base(&self) -> *mut u8 {
        self.base
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The whole file, for the CRC and the folded MD4 — both of which C takes
    /// **before** the in-place byteswap.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        // SAFETY: the constructor's contract.
        unsafe { core::slice::from_raw_parts(self.base, self.len) }
    }

    fn read_i32(&self, byte_ofs: usize) -> i32 {
        debug_assert!(byte_ofs + 4 <= self.len);
        // SAFETY: bounds are the caller's responsibility and are checked in
        // debug; every call site derives `byte_ofs` from a lump bound the
        // loader has already validated against `len`.
        unsafe { self.base.add(byte_ofs).cast::<i32>().read_unaligned() }
    }

    fn write_i32(&mut self, byte_ofs: usize, v: i32) {
        debug_assert!(byte_ofs + 4 <= self.len);
        // SAFETY: as `read_i32`.
        unsafe { self.base.add(byte_ofs).cast::<i32>().write_unaligned(v) }
    }

    fn read_u16(&self, byte_ofs: usize) -> u16 {
        debug_assert!(byte_ofs + 2 <= self.len);
        // SAFETY: as `read_i32`.
        unsafe { self.base.add(byte_ofs).cast::<u16>().read_unaligned() }
    }

    fn write_u16(&mut self, byte_ofs: usize, v: u16) {
        debug_assert!(byte_ofs + 2 <= self.len);
        // SAFETY: as `read_i32`.
        unsafe { self.base.add(byte_ofs).cast::<u16>().write_unaligned(v) }
    }

    /// `for (i = 0; i < sizeof (*qcvm->progs) / 4; i++) ((int *)progs)[i] = LittleLong (...)`
    ///
    /// Runs before any bound is known, so it is the one place that must clamp
    /// against a short file rather than trust the header.
    pub fn swap_header(&mut self) {
        for i in 0..DPROGRAMS_INTS {
            if (i + 1) * 4 > self.len {
                break;
            }
            let v = self.read_i32(i * 4);
            self.write_i32(i * 4, i32::from_le(v));
        }
    }

    /// The header as values. Zero-filled beyond a short file, matching what a
    /// truncated `COM_LoadFile` buffer would leave C reading past its end —
    /// except that here the read is defined.
    #[must_use]
    pub fn header(&self) -> DPrograms {
        let f = |i: usize| -> c_int {
            if (i + 1) * 4 <= self.len {
                self.read_i32(i * 4)
            } else {
                0
            }
        };
        DPrograms {
            version: f(0),
            crc: f(1),
            ofs_statements: f(2),
            numstatements: f(3),
            ofs_globaldefs: f(4),
            numglobaldefs: f(5),
            ofs_fielddefs: f(6),
            numfielddefs: f(7),
            ofs_functions: f(8),
            numfunctions: f(9),
            ofs_strings: f(10),
            numstrings: f(11),
            ofs_globals: f(12),
            numglobals: f(13),
            entityfields: f(14),
        }
    }

    /// `qcvm->progs->numfielddefs++` — the merge grows the header in place.
    pub fn set_numfielddefs(&mut self, v: c_int) {
        self.write_i32(7 * 4, v);
    }

    /// `qcvm->progs->entityfields = maxofs` — this is what fixes `edict_size`
    /// and therefore the savegame layout.
    pub fn set_entityfields(&mut self, v: c_int) {
        self.write_i32(14 * 4, v);
    }

    /// True when the whole `[ofs, ofs + count * stride)` range is inside the
    /// file. C performs no such test; the loader uses it to refuse a lump
    /// that would walk off the buffer instead of reproducing the read.
    #[must_use]
    pub fn lump_fits(&self, ofs: c_int, count: c_int, stride: usize) -> bool {
        let Ok(ofs) = usize::try_from(ofs) else {
            return false;
        };
        let Ok(count) = usize::try_from(count) else {
            return false;
        };
        count
            .checked_mul(stride)
            .and_then(|n| n.checked_add(ofs))
            .is_some_and(|end| end <= self.len)
    }

    /// The four shorts of every `dstatement_t`.
    pub fn swap_statements(&mut self, ofs: c_int, count: c_int) {
        let base = ofs as usize;
        for i in 0..count as usize {
            for w in 0..4 {
                let at = base + i * 8 + w * 2;
                let v = self.read_u16(at);
                self.write_u16(at, u16::from_le(v));
            }
        }
    }

    /// `first_statement`, `parm_start`, `s_name`, `s_file`, `numparms` and
    /// `locals` — C swaps exactly these six and leaves `profile` and
    /// `parm_size[]` alone.
    pub fn swap_functions(&mut self, ofs: c_int, count: c_int) {
        const WORDS: [usize; 6] = [0, 1, 2, 4, 5, 6];
        let base = ofs as usize;
        for i in 0..count as usize {
            for w in WORDS {
                let at = base + i * 36 + w * 4;
                let v = self.read_i32(at);
                self.write_i32(at, i32::from_le(v));
            }
        }
    }

    /// The two shorts and the `s_name` of every `ddef_t`.
    pub fn swap_defs(&mut self, ofs: c_int, count: c_int) {
        let base = ofs as usize;
        for i in 0..count as usize {
            let at = base + i * 8;
            let t = self.read_u16(at);
            self.write_u16(at, u16::from_le(t));
            let o = self.read_u16(at + 2);
            self.write_u16(at + 2, u16::from_le(o));
            let n = self.read_i32(at + 4);
            self.write_i32(at + 4, i32::from_le(n));
        }
    }

    /// `for (i = 0; i < numglobals; i++) ((int *)globals)[i] = LittleLong (...)`
    pub fn swap_globals(&mut self, ofs: c_int, count: c_int) {
        let base = ofs as usize;
        for i in 0..count as usize {
            let at = base + i * 4;
            let v = self.read_i32(at);
            self.write_i32(at, i32::from_le(v));
        }
    }

    /// The `type` half of a `ddef_t`, read after [`swap_defs`](Self::swap_defs).
    #[must_use]
    pub fn def_type(&self, ofs: c_int, index: c_int) -> u16 {
        self.read_u16(ofs as usize + index as usize * 8)
    }

    /// A whole `ddef_t`.
    #[must_use]
    pub fn def(&self, ofs: c_int, index: c_int) -> DDef {
        let at = ofs as usize + index as usize * 8;
        DDef {
            type_: self.read_u16(at),
            ofs: self.read_u16(at + 2),
            s_name: self.read_i32(at + 4),
        }
    }

    /// Address of a `ddef_t` inside the image — the value the hash maps store.
    #[must_use]
    pub fn def_ptr(&self, ofs: c_int, index: c_int) -> *const DDef {
        // SAFETY: the caller has validated the lump with `lump_fits`.
        unsafe { self.base.add(ofs as usize + index as usize * 8).cast() }
    }

    /// `qcvm->functions[i].s_name`, read after the function byteswap.
    #[must_use]
    pub fn function_s_name(&self, ofs: c_int, index: c_int) -> c_int {
        self.read_i32(ofs as usize + index as usize * 36 + 4 * 4)
    }

    /// `qcvm->functions[i].first_statement`.
    #[must_use]
    pub fn function_first_statement(&self, ofs: c_int, index: c_int) -> c_int {
        self.read_i32(ofs as usize + index as usize * 36)
    }

    /// `f->first_statement = ex->patch_statement` (re-release patching).
    pub fn set_function_first_statement(&mut self, ofs: c_int, index: c_int, v: c_int) {
        self.write_i32(ofs as usize + index as usize * 36, v);
    }

    /// Address of a `dfunction_t` inside the image.
    #[must_use]
    pub fn function_ptr(&self, ofs: c_int, index: c_int) -> *const DFunction {
        // SAFETY: the caller has validated the lump with `lump_fits`.
        unsafe { self.base.add(ofs as usize + index as usize * 36).cast() }
    }

    /// `(char *)qcvm->progs + ofs_strings`.
    #[must_use]
    pub fn strings_ptr(&self, ofs: c_int) -> *mut c_char {
        // SAFETY: `ofs` is inside the file — the loader checks
        // `ofs_strings + numstrings >= com_filesize` exactly as C does.
        unsafe { self.base.add(ofs as usize).cast() }
    }

    /// The last byte of the strings blob, or `None` if the lump is empty or
    /// does not fit.
    #[must_use]
    pub fn strings_last_byte(&self, ofs: c_int, count: c_int) -> Option<u8> {
        let (ofs, count) = (usize::try_from(ofs).ok()?, usize::try_from(count).ok()?);
        let end = ofs.checked_add(count)?;
        if count == 0 || end > self.len {
            return None;
        }
        Some(self.bytes()[end - 1])
    }

    /// A raw byte pointer into the image.
    #[must_use]
    pub fn at(&self, ofs: c_int) -> *mut u8 {
        // SAFETY: the caller has validated the offset.
        unsafe { self.base.add(ofs as usize) }
    }
}

/// A `ddef_t` array that may live either inside the progs image or in the
/// `Mem_Alloc` block `PR_MergeEngineFieldDefs` reallocates it into.
///
/// The distinction is load-bearing: `PR_ClearProgs` frees the array only when
/// it is *not* the image's own lump, and it tells the two apart by pointer
/// comparison.
pub struct DefTable {
    base: *mut DDef,
    count: usize,
}

impl DefTable {
    /// # Safety
    ///
    /// `base` must point at `count` readable, writable `ddef_t`s that outlive
    /// the view.
    #[must_use]
    pub unsafe fn new(base: *mut DDef, count: usize) -> Self {
        Self { base, count }
    }

    /// `PR_MergeEngineFieldDefs`' reallocation: a fresh `Mem_Alloc` block of
    /// `count` entries with `src`'s first `copy` entries memcpy'd in.
    ///
    /// Safe because both bounds come from tables this module vouches for —
    /// which is what keeps [`crate::load`] free of `unsafe`.
    #[must_use]
    pub fn realloc_from(
        mem: &mut dyn crate::arena::Mem,
        count: usize,
        src: &DefTable,
        copy: usize,
    ) -> Self {
        assert!(copy <= src.count && copy <= count);
        let bytes = mem.alloc(count * core::mem::size_of::<DDef>());
        // The copy is over **bytes**, not `DDef`s, because `src` may be the
        // image's own lump at whatever `ofs_fielddefs` says -- and nothing
        // requires that to be 4-aligned. C's `memcpy` does not care; a typed
        // `copy_nonoverlapping` would be UB. Found by `fuzz_progs_load`.
        //
        // SAFETY: `bytes` is a fresh allocation of `count * size_of::<DDef>()`
        // (Mem_Alloc aborts rather than returning null, ADR-013), and `src` is
        // either the progs image or an older block, so the two cannot overlap.
        unsafe {
            core::ptr::copy_nonoverlapping(
                src.base.cast::<u8>().cast_const(),
                bytes,
                copy * core::mem::size_of::<DDef>(),
            );
        }
        Self {
            base: bytes.cast::<DDef>(),
            count,
        }
    }

    #[must_use]
    pub fn base(&self) -> *mut DDef {
        self.base
    }

    #[must_use]
    pub fn count(&self) -> usize {
        self.count
    }

    #[must_use]
    pub fn get(&self, i: usize) -> DDef {
        assert!(i < self.count);
        // SAFETY: bounds-checked; `ddef_t` is 8-byte aligned inside a
        // `Mem_Alloc` block but only 4-byte aligned inside the image, so the
        // read stays unaligned.
        unsafe { self.base.add(i).read_unaligned() }
    }

    pub fn set(&mut self, i: usize, def: DDef) {
        assert!(i < self.count);
        // SAFETY: as `get`.
        unsafe { self.base.add(i).write_unaligned(def) }
    }

    #[must_use]
    pub fn ptr(&self, i: usize) -> *const DDef {
        assert!(i < self.count);
        // SAFETY: bounds-checked.
        unsafe { self.base.add(i) }
    }
}

/// The `qcvm_t` fields `PR_LoadProgs` and `PR_ClearProgs` write.
///
/// [`crate::arena::VmRaw`] deliberately cannot be built until the lumps are
/// loaded, so the loader needs its own view of the same object. It is
/// separate rather than merged so that nothing on the *execution* path can
/// reach a setter that only makes sense mid-load.
pub struct VmLoad {
    vm: *mut QcVm,
}

macro_rules! field {
    ($get:ident, $set:ident, $ty:ty, $name:ident) => {
        #[must_use]
        pub fn $get(&self) -> $ty {
            // SAFETY: a plain field read of the live qcvm_t (constructor
            // contract).
            unsafe { (*self.vm).$name }
        }
        pub fn $set(&mut self, v: $ty) {
            // SAFETY: as above.
            unsafe { (*self.vm).$name = v }
        }
    };
}

impl VmLoad {
    /// # Safety
    ///
    /// `vm` must point at a live `qcvm_t` that stays valid and
    /// unaliased-by-Rust for the lifetime of the view. During `PR_LoadProgs`
    /// that is the ambient VM the host frame selected (ADR-007, ADR-008).
    #[must_use]
    pub unsafe fn new(vm: *mut QcVm) -> Self {
        assert!(!vm.is_null());
        Self { vm }
    }

    #[must_use]
    pub fn as_ptr(&self) -> *mut QcVm {
        self.vm
    }

    field!(progs, set_progs, *mut DPrograms, progs);
    field!(functions, set_functions, *mut DFunction, functions);
    field!(statements, set_statements, *mut DStatement, statements);
    field!(globaldefs, set_globaldefs, *mut DDef, globaldefs);
    field!(fielddefs, set_fielddefs, *mut DDef, fielddefs);
    field!(globals, set_globals, *mut f32, globals);
    field!(strings, set_strings, *mut c_char, strings);
    field!(stringssize, set_stringssize, c_int, stringssize);
    field!(progssize, set_progssize, u32, progssize);
    field!(progscrc, set_progscrc, u16, progscrc);
    field!(progshash, set_progshash, u32, progshash);
    field!(edict_size, set_edict_size, c_int, edict_size);
    field!(numbuiltins, set_numbuiltins, c_int, numbuiltins);
    field!(numknownstrings, set_numknownstrings, c_int, numknownstrings);
    field!(progsstrings, set_progsstrings, c_int, progsstrings);
    field!(function_map, set_function_map, *mut c_void, function_map);
    field!(
        globaldefs_map,
        set_globaldefs_map,
        *mut c_void,
        globaldefs_map
    );
    field!(fielddefs_map, set_fielddefs_map, *mut c_void, fielddefs_map);
    field!(edicts, set_edicts_raw, *mut Edict, edicts);
    field!(
        knownstrings,
        set_knownstrings,
        *mut *const c_char,
        knownstrings
    );
    field!(
        knownstringsowned,
        set_knownstringsowned,
        *mut QBoolean,
        knownstringsowned
    );

    /// `qcvm->extfields.<name> = ED_FindFieldOffset ("<name>")`
    pub fn set_extfields(&mut self, v: PrExtFields) {
        // SAFETY: a plain field write of the live qcvm_t.
        unsafe { (*self.vm).extfields = v }
    }

    /// `memcpy (qcvm->builtins, builtins, numbuiltins * sizeof (...))`
    pub fn copy_builtins(&mut self, src: &[BuiltinT]) {
        assert!(src.len() <= MAX_BUILTINS);
        // SAFETY: `builtins` is a fixed MAX_BUILTINS array in the live
        // qcvm_t and `src` is a Rust slice, so both ranges are valid and
        // cannot overlap.
        unsafe {
            core::ptr::copy_nonoverlapping(
                src.as_ptr(),
                (*self.vm).builtins.as_mut_ptr(),
                src.len(),
            )
        }
    }

    /// `qcvm->fielddefs` as a bounded table.
    #[must_use]
    pub fn fielddef_table(&self, count: usize) -> DefTable {
        // SAFETY: the constructor's contract; `count` is the loader's
        // `numfielddefs`, which is what the array was sized with.
        unsafe { DefTable::new(self.fielddefs(), count) }
    }

    /// `(ddef_t *)((byte *)qcvm->progs + qcvm->progs->ofs_fielddefs)` — the
    /// pointer `PR_ClearProgs` compares against to decide whether
    /// `qcvm->fielddefs` is the image's own lump or a `Mem_Alloc` block.
    ///
    /// `None` when the image is too short to hold a header, which happens when
    /// `PR_LoadProgs` raised before it got that far and the VM is torn down
    /// afterwards.
    ///
    /// COMPAT (accepted divergence): C reads `qcvm->progs->ofs_fielddefs`
    /// unconditionally, so a truncated `progs.dat` makes `PR_ClearProgs` read
    /// past the buffer. Found by `fuzz_progs_load` under ASan.
    #[must_use]
    pub fn image_fielddefs(&self) -> Option<*mut u8> {
        let progs = self.progs();
        if progs.is_null() || (self.progssize() as usize) < core::mem::size_of::<DPrograms>() {
            return None;
        }
        // SAFETY: the image is at least a whole header long (checked above),
        // and its header has been byteswapped, so `ofs_fielddefs` is the
        // in-file offset C uses here.
        let ofs = unsafe { (*progs).ofs_fielddefs };
        // SAFETY: as above; the offset is only compared, never dereferenced.
        Some(unsafe { progs.cast::<u8>().add(ofs.max(0) as usize) })
    }

    /// `PR_GetString` restricted to the strings blob, which is all the loader
    /// needs: nothing has allocated an engine string yet when the symbol names
    /// are hashed.
    ///
    /// COMPAT: the out-of-range arm returns `qcvm->strings`, the empty string
    /// at the head of the blob — the `Host_Error` after it in `PR_GetString`
    /// sits behind a `return` and is dead code.
    #[must_use]
    pub fn blob_string(&self, num: c_int, numstrings: c_int) -> *const c_char {
        let strings = self.strings();
        if num >= 0 && num < numstrings {
            // SAFETY: bounds-checked against the strings lump, which the
            // loader has already confirmed lies inside the file.
            unsafe { strings.add(num as usize) }
        } else {
            strings
        }
    }

    /// `memset (qcvm, 0, sizeof (*qcvm))` — `PR_ClearProgs`' last act.
    pub fn zero(&mut self) {
        // SAFETY: the constructor's contract; `qcvm_t` is a POD C struct and
        // all-zeroes is the state `PR_SwitchQCVM (NULL)` expects to find.
        unsafe { core::ptr::write_bytes(self.vm, 0, 1) }
    }

    /// `qcvm->knownstrings[i]` / `qcvm->knownstringsowned[i]`, for
    /// `PR_ClearProgs`' free loop.
    #[must_use]
    pub fn known_string(&self, i: c_int) -> (*const c_char, bool) {
        // SAFETY: the caller iterates `0..numknownstrings`, which is the
        // array's populated prefix.
        unsafe {
            (
                self.knownstrings().add(i as usize).read(),
                self.knownstringsowned().add(i as usize).read(),
            )
        }
    }
}
