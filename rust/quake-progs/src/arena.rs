//! The untyped edict arena, the progs string table, and the raw `qcvm_t`
//! view (ADR-006).
//!
//! This is the crate's single `#[allow(unsafe_code)]` island (ADR-004).
//! Everything in it exists because two pieces of progs state cannot be Rust
//! types:
//!
//! * **Edicts.** Their layout is a runtime ABI decided when `progs.dat` loads:
//!   the stride is `qcvm->edict_size`, not `size_of::<Edict>()`, and
//!   mod-defined fields extend past [`EntVars`]. All QC field access is offset
//!   arithmetic.
//! * **The string table.** `knownstrings` is a C `const char **` of borrowed
//!   and owned pointers, indexed by the negative half of a `string_t`, and
//!   `PR_SetEngineString` decides identity by raw pointer comparison.
//! * **The `qcvm_t` itself.** It is C storage embedded in `sv`/`cl`, and its
//!   progs lumps, global block and stacks are untyped word arrays.
//!
//! Both are C-owned during Phase 6 (`qcvm_t` lives inside `sv`/`cl`), so the
//! arena has two constructors: [`EdictArena::borrowed`] over engine memory and
//! [`EdictArena::owned`] over a Rust allocation for tests and fuzzing.
//!
//! # Re-entrancy
//!
//! No value here holds a Rust reference into the arena. Accessors take
//! `&self`/`&mut self` and re-derive a raw pointer per call, because the
//! interpreter dispatches C builtins that re-enter the VM and touch the same
//! memory (ADR-006 Phase 6 amendment).
#![allow(unsafe_code)]

use core::ffi::{c_char, c_int, c_void};
use core::mem::{offset_of, size_of};

use quake_types::progs::{
    DDef, DFunction, DStatement, Edict, EntityState, PrStack, QBoolean, QcVm,
};

/// Index of an edict within the arena. Replaces the raw `edict_t *` that C
/// passes around; `EDICT_TO_PROG` is `id * edict_size` (ADR-006).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EdictId(pub u32);

/// Byte offset of the progs-visible field block within an edict. The debug
/// build's three extra header fields move it, which is why it is derived from
/// the mirror rather than written down.
pub const EDICT_V_OFFSET: usize = offset_of!(Edict, v);

/// `Mem_Alloc`/`Mem_Free`/`Mem_Realloc` — the mandatory cross-boundary
/// ownership API (ADR-013). The string table hands pointers to C and takes
/// pointers back from it, so it must allocate the way C does.
pub trait Mem {
    /// Must return zeroed memory, like the engine's `Mem_Alloc`.
    fn alloc(&mut self, size: usize) -> *mut u8;
    fn realloc(&mut self, ptr: *mut u8, size: usize) -> *mut u8;
    /// Must tolerate a null pointer, like `Mem_Free`.
    fn free(&mut self, ptr: *mut u8);
    /// `Con_DPrintf2` from `PR_AllocStringSlots`. Deferred, not called
    /// inline: the console is not a leaf (it can reach `SCR_UpdateScreen`).
    fn note_slot_growth(&mut self, maxknownstrings: c_int);
}

/// A view over the edict array: `count` edicts of `stride` bytes each,
/// starting at `base`.
pub struct EdictArena {
    base: *mut u8,
    stride: usize,
    count: usize,
    /// Backing storage for [`EdictArena::owned`]; `None` when the engine owns
    /// the memory. `u64` so the block is pointer-aligned like `Mem_Alloc`'s.
    /// Never read — it exists to keep `base` alive.
    #[allow(dead_code)]
    owned: Option<Box<[u64]>>,
}

impl EdictArena {
    /// Borrow the engine's edict array.
    ///
    /// # Safety
    ///
    /// `base` must point at `count * stride` writable, initialised bytes that
    /// stay valid for the lifetime of the returned arena, and `stride` must be
    /// the `qcvm->edict_size` those edicts were laid out with. Callers
    /// additionally guarantee the host-frame exclusivity argument of
    /// ADR-007/ADR-008.
    ///
    /// The arena keeps a raw base pointer and re-derives every access from it,
    /// so another live reference *into* the same array is permitted provided
    /// it does not overlap an edict the arena writes. `quake_rs_ed_parse_epair`
    /// relies on exactly that: its `dest` slice points into the edict being
    /// parsed, while the arena only ever touches edicts at higher indices (see
    /// the invariant recorded at that shim).
    pub unsafe fn borrowed(base: *mut u8, stride: usize, count: usize) -> Self {
        assert!(
            stride >= EDICT_V_OFFSET,
            "edict_size smaller than the edict header"
        );
        assert_eq!(
            stride % size_of::<*mut c_void>(),
            0,
            "edict_size is not pointer-aligned"
        );
        assert!(!base.is_null());
        Self {
            base,
            stride,
            count,
            owned: None,
        }
    }

    /// Allocate a Rust-owned arena — the differential-test and fuzz path.
    #[must_use]
    pub fn owned(stride: usize, count: usize) -> Self {
        assert!(
            stride >= EDICT_V_OFFSET,
            "edict_size smaller than the edict header"
        );
        assert_eq!(
            stride % size_of::<*mut c_void>(),
            0,
            "edict_size is not pointer-aligned"
        );
        // u64-backed so the block is pointer-aligned the way Mem_Alloc's is
        let mut buf = vec![0u64; (stride * count).div_ceil(8)].into_boxed_slice();
        let base = buf.as_mut_ptr().cast::<u8>();
        Self {
            base,
            stride,
            count,
            owned: Some(buf),
        }
    }

    #[must_use]
    pub fn stride(&self) -> usize {
        self.stride
    }

    #[must_use]
    pub fn count(&self) -> usize {
        self.count
    }

    #[must_use]
    pub fn base(&self) -> *mut u8 {
        self.base
    }

    fn edict(&self, id: EdictId) -> *mut u8 {
        let idx = id.0 as usize;
        assert!(
            idx < self.count,
            "edict {idx} out of range (count {})",
            self.count
        );
        // SAFETY: idx < count and the whole count*stride block is valid by the
        // constructor's contract, so the offset stays inside the allocation.
        unsafe { self.base.add(idx * self.stride) }
    }

    /// `EDICT_TO_PROG` — the byte offset QuakeC sees, and what savegames and
    /// the wire protocol carry (ADR-006).
    #[must_use]
    pub fn to_prog(&self, id: EdictId) -> c_int {
        (id.0 as usize * self.stride) as c_int
    }

    /// `PROG_TO_EDICT`. C does pointer arithmetic and never validates the
    /// remainder, so a `prog` that is not a multiple of the stride rounds
    /// down — preserved.
    #[must_use]
    pub fn from_prog(&self, prog: c_int) -> EdictId {
        EdictId((prog as usize / self.stride) as u32)
    }

    fn header(&self, id: EdictId) -> *mut Edict {
        self.edict(id).cast::<Edict>()
    }

    #[must_use]
    pub fn free(&self, id: EdictId) -> bool {
        // SAFETY: `free` sits in the fixed header, which every edict has in
        // full (the constructor rejects a stride shorter than it).
        unsafe { core::ptr::addr_of!((*self.header(id)).free).read() }
    }

    pub fn set_free(&mut self, id: EdictId, value: bool) {
        // SAFETY: as above.
        unsafe { core::ptr::addr_of_mut!((*self.header(id)).free).write(value as QBoolean) }
    }

    #[must_use]
    pub fn freetime(&self, id: EdictId) -> f32 {
        // SAFETY: as above.
        unsafe { core::ptr::addr_of!((*self.header(id)).freetime).read() }
    }

    pub fn set_freetime(&mut self, id: EdictId, value: f32) {
        // SAFETY: as above.
        unsafe { core::ptr::addr_of_mut!((*self.header(id)).freetime).write(value) }
    }

    pub fn set_alpha(&mut self, id: EdictId, value: u8) {
        // SAFETY: as above.
        unsafe { core::ptr::addr_of_mut!((*self.header(id)).alpha).write(value) }
    }

    /// `ED_Free`'s `assert (!ed->area.prev)` on an already-free edict.
    #[must_use]
    pub fn area_prev_is_null(&self, id: EdictId) -> bool {
        // SAFETY: as above.
        unsafe {
            core::ptr::addr_of!((*self.header(id)).area.prev)
                .read()
                .is_null()
        }
    }

    pub fn set_baseline(&mut self, id: EdictId, state: &EntityState) {
        // SAFETY: as above; `state` is a separate object, so no overlap.
        unsafe { core::ptr::addr_of_mut!((*self.header(id)).baseline).write(*state) }
    }

    /// `memset (e, 0, qcvm->edict_size)` — the whole edict, header included.
    pub fn clear_edict(&mut self, id: EdictId) {
        let p = self.edict(id);
        // SAFETY: `stride` bytes from `p` is exactly one edict.
        unsafe { core::ptr::write_bytes(p, 0, self.stride) }
    }

    /// COMPAT: `ED_Alloc` clears `progs->entityfields * 4` bytes starting at
    /// `&e->v` — **not** `edict_size - EDICT_V_OFFSET`. The two differ by the
    /// pointer-alignment padding `edict_size` is rounded up with, and those
    /// padding bytes are deliberately left as they were.
    pub fn clear_fields(&mut self, id: EdictId, entityfields: c_int) {
        let bytes = (entityfields.max(0) as usize) * 4;
        let avail = self.stride - EDICT_V_OFFSET;
        assert!(
            bytes <= avail,
            "entityfields ({entityfields}) overruns edict_size"
        );
        let p = self.edict(id);
        // SAFETY: EDICT_V_OFFSET + bytes <= stride by the assert above.
        unsafe { core::ptr::write_bytes(p.add(EDICT_V_OFFSET), 0, bytes) }
    }

    /// Restores the `DEBUG`/`_DEBUG` bookkeeping fields that `clear_edict`
    /// wipes. A no-op unless the mirror carries them.
    pub fn set_debug_header(&mut self, id: EdictId, qcvm: *mut c_void, num: u64) {
        let _ = (id, qcvm, num);
        #[cfg(feature = "engine-debug")]
        {
            let p = self.header(id);
            // SAFETY: the three fields are the first members of the header
            // under this cfg, and the mirror is asserted against the C layout
            // by quake-ctest/tests/progs_abi.rs.
            unsafe {
                core::ptr::addr_of_mut!((*p).edict_ptr).write(p);
                core::ptr::addr_of_mut!((*p).qcvm_owner).write(qcvm.cast());
                core::ptr::addr_of_mut!((*p).edict_num).write(num);
            }
        }
    }

    fn field(&self, id: EdictId, byte_ofs: usize, width: usize) -> *mut u8 {
        let end = EDICT_V_OFFSET
            .checked_add(byte_ofs)
            .and_then(|s| s.checked_add(width))
            .expect("field offset overflow");
        assert!(
            end <= self.stride,
            "field offset {byte_ofs} (+{width}) overruns edict_size"
        );
        // SAFETY: bounds checked immediately above.
        unsafe { self.edict(id).add(EDICT_V_OFFSET + byte_ofs) }
    }

    /// Write an `f32` field at `byte_ofs` within the progs field block.
    pub fn set_field_f32(&mut self, id: EdictId, byte_ofs: usize, value: f32) {
        // SAFETY: `field` bounds-checks; progs fields are 4-byte aligned
        // because the block starts pointer-aligned and offsets are multiples
        // of 4, but write_unaligned costs nothing here and needs no proof.
        unsafe {
            self.field(id, byte_ofs, 4)
                .cast::<f32>()
                .write_unaligned(value)
        }
    }

    /// Write an `i32`-shaped field (`string_t`, `func_t`, entity, field).
    pub fn set_field_i32(&mut self, id: EdictId, byte_ofs: usize, value: i32) {
        // SAFETY: as above.
        unsafe {
            self.field(id, byte_ofs, 4)
                .cast::<i32>()
                .write_unaligned(value)
        }
    }

    pub fn set_field_vec3(&mut self, id: EdictId, byte_ofs: usize, value: [f32; 3]) {
        // SAFETY: as above, with a 12-byte width.
        unsafe {
            self.field(id, byte_ofs, 12)
                .cast::<[f32; 3]>()
                .write_unaligned(value)
        }
    }

    #[must_use]
    pub fn field_f32(&self, id: EdictId, byte_ofs: usize) -> f32 {
        // SAFETY: as above.
        unsafe { self.field(id, byte_ofs, 4).cast::<f32>().read_unaligned() }
    }

    /// Raw bytes of one edict — the differential suites compare these.
    pub fn edict_bytes(&self, id: EdictId) -> &[u8] {
        let p = self.edict(id);
        // SAFETY: `stride` bytes from `p` is one edict, and `&self` bounds
        // the borrow so nothing mutates it meanwhile.
        unsafe { core::slice::from_raw_parts(p, self.stride) }
    }
}

/// Byte offsets of the [`EntVars`] fields `ED_Free` resets, so the callers
/// stay free of `offset_of!` and of `EntVars` itself.
pub mod entvars_ofs {
    use core::mem::offset_of;

    use quake_types::progs::EntVars;

    pub const MODEL: usize = offset_of!(EntVars, model);
    pub const TAKEDAMAGE: usize = offset_of!(EntVars, takedamage);
    pub const MODELINDEX: usize = offset_of!(EntVars, modelindex);
    pub const COLORMAP: usize = offset_of!(EntVars, colormap);
    pub const SKIN: usize = offset_of!(EntVars, skin);
    pub const FRAME: usize = offset_of!(EntVars, frame);
    pub const ORIGIN: usize = offset_of!(EntVars, origin);
    pub const ANGLES: usize = offset_of!(EntVars, angles);
    pub const NEXTTHINK: usize = offset_of!(EntVars, nextthink);
    pub const SOLID: usize = offset_of!(EntVars, solid);
}

/// The progs string table: the `strings` blob plus the `knownstrings` array of
/// engine strings addressed by negative `string_t` values.
///
/// A view, not an owner — every field lives in the C `qcvm_t`.
/// The fields are raw pointers rather than `&mut` references so the table can
/// be built from a live `qcvm_t` without taking a borrow that would alias
/// [`VmRaw`]'s view of the same object.
pub struct StringTable<'a> {
    pub strings: *const c_char,
    pub stringssize: c_int,
    pub knownstrings: *mut *mut *const c_char,
    pub knownstringsowned: *mut *mut QBoolean,
    pub maxknownstrings: *mut c_int,
    pub numknownstrings: *mut c_int,
    pub progsstrings: c_int,
    pub freeknownstrings: *mut c_int,
    pub(crate) _marker: core::marker::PhantomData<&'a mut ()>,
}

impl<'a> StringTable<'a> {
    /// Build a table from the addresses of the fields it mutates, for callers
    /// that keep the string table outside a `qcvm_t` — the differential
    /// suites, which drive it beside the C original.
    ///
    /// # Safety
    ///
    /// Every pointer must stay valid, and exclusively owned by this table,
    /// for `'a`.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn from_parts(
        strings: *const c_char,
        stringssize: c_int,
        knownstrings: *mut *mut *const c_char,
        knownstringsowned: *mut *mut QBoolean,
        maxknownstrings: *mut c_int,
        numknownstrings: *mut c_int,
        progsstrings: c_int,
        freeknownstrings: *mut c_int,
    ) -> Self {
        Self {
            strings,
            stringssize,
            knownstrings,
            knownstringsowned,
            maxknownstrings,
            numknownstrings,
            progsstrings,
            freeknownstrings,
            _marker: core::marker::PhantomData,
        }
    }
}

impl StringTable<'_> {
    fn known_base(&self) -> *mut *const c_char {
        // SAFETY: the constructor's contract keeps the field live.
        unsafe { *self.knownstrings }
    }

    fn owned_base(&self) -> *mut QBoolean {
        // SAFETY: as above.
        unsafe { *self.knownstringsowned }
    }

    fn num(&self) -> c_int {
        // SAFETY: as above.
        unsafe { *self.numknownstrings }
    }

    fn max(&self) -> c_int {
        // SAFETY: as above.
        unsafe { *self.maxknownstrings }
    }

    fn free_slot(&self) -> c_int {
        // SAFETY: as above.
        unsafe { *self.freeknownstrings }
    }
}

/// `pr_edict.c`
const PR_STRING_ALLOCSLOTS: c_int = 256;

/// `PR_GetString`'s lookup, over plain values so the owning [`StringTable`]
/// view and the borrow-free [`VmRaw`] path cannot drift apart.
///
/// COMPAT: the invalid-offset arm returns `qcvm->strings` — the empty string
/// at the head of the blob — and the `Host_Error` after it in `pr_edict.c`
/// sits behind a `return` and is dead code. Out-of-range handles therefore
/// fail silently, and that is preserved. The one live error is a negative
/// handle whose slot has been cleared.
///
/// # Safety
///
/// `knownstrings` must have at least `numknownstrings` readable entries when
/// `numknownstrings > 0`.
unsafe fn resolve_string(
    strings: *const c_char,
    stringssize: c_int,
    knownstrings: *mut *const c_char,
    numknownstrings: c_int,
    num: c_int,
) -> Result<*const c_char, StringError> {
    if num >= 0 && num < stringssize {
        // SAFETY: 0 <= num < stringssize, inside the blob.
        return Ok(unsafe { strings.add(num as usize) });
    }
    if num < 0 && num >= -numknownstrings {
        // SAFETY: -1 - num is in 0..numknownstrings by the test above.
        let p = unsafe { knownstrings.add((-1 - num) as usize).read() };
        if p.is_null() {
            return Err(StringError::NonExistent(num));
        }
        return Ok(p);
    }
    Ok(strings)
}

impl StringTable<'_> {
    fn known(&self, i: c_int) -> *const c_char {
        debug_assert!(i >= 0 && i < self.num());
        // SAFETY: callers only pass indices below numknownstrings, and the
        // array is at least maxknownstrings >= numknownstrings entries.
        unsafe { self.known_base().add(i as usize).read() }
    }

    fn set_known(&mut self, i: c_int, ptr: *const c_char, owned: bool) {
        debug_assert!(i >= 0 && i < self.max());
        // SAFETY: as above; i < maxknownstrings is the array's true bound.
        unsafe {
            self.known_base().add(i as usize).write(ptr);
            self.owned_base().add(i as usize).write(owned);
        }
    }

    fn is_owned(&self, i: c_int) -> bool {
        // SAFETY: as above.
        unsafe { self.owned_base().add(i as usize).read() }
    }

    fn alloc_slots(&mut self, mem: &mut dyn Mem) {
        // SAFETY: the constructor's contract keeps every field of the table
        // live for its lifetime; these are the qcvm_t's own counters.
        unsafe {
            *self.maxknownstrings += PR_STRING_ALLOCSLOTS;
            let n = *self.maxknownstrings as usize;
            mem.note_slot_growth(*self.maxknownstrings);
            *self.knownstrings = mem
                .realloc(
                    self.known_base().cast::<u8>(),
                    n * size_of::<*const c_char>(),
                )
                .cast();
            *self.knownstringsowned = mem
                .realloc(self.owned_base().cast::<u8>(), n * size_of::<QBoolean>())
                .cast();
        }
    }

    /// `PR_GetString`.
    ///
    /// COMPAT: the invalid-offset arm returns `qcvm->strings` — the empty
    /// string at the head of the blob — and the `Host_Error` after it is dead
    /// code (`pr_edict.c`). Out-of-range handles therefore fail silently, and
    /// that is preserved.
    ///
    /// The one *live* error is a negative handle whose slot is null.
    pub fn get(&self, num: c_int) -> Result<*const c_char, StringError> {
        // SAFETY: the array holds maxknownstrings >= numknownstrings entries.
        unsafe {
            resolve_string(
                self.strings,
                self.stringssize,
                self.known_base(),
                self.num(),
                num,
            )
        }
    }

    /// `PR_ClearEngineString`.
    pub fn clear_engine_string(&mut self, num: c_int, mem: &mut dyn Mem) {
        if num < 0 && num >= -self.num() {
            let i = -1 - num;
            if self.is_owned(i) {
                // C is an unconditional SAFE_FREE; Mem_Free tolerates NULL
                mem.free(self.known(i).cast_mut().cast::<u8>());
                self.set_known(i, core::ptr::null(), false);
            } else {
                // COMPAT: the non-owned arm clears only the pointer and
                // leaves `knownstringsowned` alone (it is already false).
                // SAFETY: i < numknownstrings <= maxknownstrings.
                unsafe {
                    (*self.knownstrings)
                        .add(i as usize)
                        .write(core::ptr::null())
                }
            }
            // SAFETY: the field is live for the table's lifetime.
            unsafe {
                if *self.freeknownstrings > i {
                    *self.freeknownstrings = i;
                }
            }
        }
    }

    /// The `for (i = freeknownstrings;; i++)` slot search shared verbatim by
    /// `PR_SetEngineString` and `PR_AllocString`.
    fn take_slot(&mut self, mem: &mut dyn Mem) -> c_int {
        let mut i = self.free_slot();
        loop {
            if i < self.num() {
                if !self.known(i).is_null() {
                    i += 1;
                    continue;
                }
            } else {
                if i >= self.max() {
                    self.alloc_slots(mem);
                }
                // SAFETY: the field is live for the table's lifetime.
                unsafe { *self.numknownstrings += 1 };
            }
            break;
        }
        // SAFETY: as above.
        unsafe { *self.freeknownstrings = i + 1 };
        i
    }

    /// `PR_SetEngineString`.
    ///
    /// COMPAT: the in-blob test is `s <= strings + stringssize - 2`, two short
    /// of the blob's end. `pr_edict.c` carries a `#if 0` explaining that
    /// `sv.model_precache`/`sv.sound_precache` point into `pr_strings`, so the
    /// stricter `Host_Error` form is unusable; the off-by-two is preserved.
    pub fn set_engine_string(&mut self, s: *const c_char, mem: &mut dyn Mem) -> c_int {
        if s.is_null() {
            return 0;
        }
        if !self.strings.is_null() && s >= self.strings {
            // SAFETY: pointer arithmetic on the blob's own range; the
            // comparison mirrors C's and never dereferences.
            let end = unsafe {
                self.strings
                    .add((self.stringssize as usize).saturating_sub(2))
            };
            if self.stringssize >= 2 && s <= end {
                return (s as usize - self.strings as usize) as c_int;
            }
        }
        for i in 0..self.num() {
            if self.known(i) == s {
                return -1 - i;
            }
        }
        let i = self.take_slot(mem);
        self.set_known(i, s, false);
        -1 - i
    }

    /// `PR_AllocString`. Returns the handle and the buffer C hands back
    /// through `**ptr`.
    pub fn alloc_string(&mut self, size: c_int, mem: &mut dyn Mem) -> (c_int, *mut c_char) {
        if size == 0 {
            return (0, core::ptr::null_mut());
        }
        let i = self.take_slot(mem);
        let buf = mem.alloc(size.max(0) as usize).cast::<c_char>();
        self.set_known(i, buf.cast_const(), true);
        (-1 - i, buf)
    }

    /// `PR_ClearEdictStrings`.
    ///
    /// COMPAT: the `freeknownstrings` reset is `#ifndef _DEBUG` — debug builds
    /// deliberately never reuse a slot, to catch stale references.
    pub fn clear_edict_strings(&mut self, mem: &mut dyn Mem) {
        for i in self.progsstrings..self.num() {
            if self.is_owned(i) {
                mem.free(self.known(i).cast_mut().cast::<u8>());
                self.set_known(i, core::ptr::null(), false);
            }
        }
        if !quake_types::progs::ENGINE_DEBUG {
            // SAFETY: the field is live for the table's lifetime.
            unsafe { *self.freeknownstrings = self.progsstrings };
        }
    }
}

/// The one reachable error in the string table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringError {
    /// `PR_GetString: attempt to get a non-existant string %d`
    NonExistent(c_int),
}

#[cfg(test)]
mod tests {
    use super::*;
    use quake_types::progs::{FreeList, MAX_EDICTS};

    /// Models the phase's central aliasing hazard under Miri: the interpreter
    /// is inside a Rust call that holds `&mut EdictArena`, it dispatches a
    /// builtin, and that builtin is C code writing the *same* edict memory
    /// through its own pointer before returning.
    ///
    /// The layout mirrors the engine's: one allocation owned outside Rust,
    /// with both the arena and the "C" alias derived from the same raw
    /// pointer. If any accessor ever retained a reference into the buffer
    /// across the callback, Stacked Borrows would reject the write below.
    #[test]
    fn builtin_reentry_does_not_invalidate_the_arena() {
        // room for the header plus a full entvars_t (105 words), the way
        // PR_LoadProgs sizes it
        let stride = (EDICT_V_OFFSET + 105 * 4).next_multiple_of(8);
        const COUNT: usize = 4;

        let layout = std::alloc::Layout::from_size_align(stride * COUNT, 8).unwrap();
        // SAFETY: non-zero size, valid alignment.
        let base = unsafe { std::alloc::alloc_zeroed(layout) };
        assert!(!base.is_null());

        // SAFETY: `base` owns STRIDE * COUNT zeroed bytes for the whole test,
        // and nothing else derives from the allocation except `alias` below,
        // which shares this same provenance.
        let mut arena = unsafe { EdictArena::borrowed(base, stride, COUNT) };
        let alias = base;

        let mut free_list = Box::new(FreeList {
            size: 0,
            head_index: 0,
            circular_buffer: [0u16; MAX_EDICTS],
        });

        let id = EdictId(2);
        arena.set_free(id, false);
        arena.set_field_f32(id, 0, 1.0);

        crate::alloc::ed_free(&mut free_list, &mut arena, id, 7.5, &mut |freed| {
            // stand-in for SV_UnlinkEdict: C writing the same edict through
            // its own pointer while the Rust call is still on the stack
            let ofs = freed.0 as usize * stride;
            // SAFETY: same allocation, same provenance, in bounds.
            unsafe { alias.add(ofs).write(0xAB) };
        });

        assert!(arena.free(id));
        assert_eq!(arena.freetime(id), 7.5);
        assert_eq!(free_list.size, 1);
        // the callback's byte survived everything the Rust side wrote after it
        assert_eq!(arena.edict_bytes(id)[0], 0xAB);

        // SAFETY: same pointer and layout the allocation was made with.
        unsafe { std::alloc::dealloc(base, layout) };
    }

    /// The owned constructor must produce a pointer-aligned block, because
    /// `edict_size` is rounded up to pointer alignment and the engine relies
    /// on edicts being aligned.
    #[test]
    fn owned_arena_is_pointer_aligned() {
        let arena = EdictArena::owned(EDICT_V_OFFSET.next_multiple_of(8) + 64, 3);
        assert_eq!(
            arena.base() as usize % core::mem::align_of::<*mut c_void>(),
            0
        );
    }
}

/// A function in the loaded functions lump.
///
/// A newtype rather than a bare `*mut DFunction` so callers outside this
/// module cannot fabricate one: every value comes from
/// [`VmRaw::function_ptr`] or [`VmRaw::stack_slot`], both of which derive it
/// from the VM's own lump.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FuncRef(*mut DFunction);

impl FuncRef {
    /// The null `xfunction` the outermost stack frame carries.
    #[must_use]
    pub fn is_null(self) -> bool {
        self.0.is_null()
    }
}

/// A raw view over the C-owned `qcvm_t`: the progs image lumps, the global
/// block, and the call/local stacks.
///
/// Like [`EdictArena`] this holds no Rust reference into any of it — every
/// accessor re-derives a pointer from `vm`, because the interpreter dispatches
/// C builtins that re-enter the VM and mutate the same state (ADR-006 Phase 6
/// amendment).
pub struct VmRaw {
    vm: *mut QcVm,
    /// `qcvm->edicts` as bytes, with the total size the arena was built with,
    /// so `OP_STOREP_*`'s unchecked byte offsets can be range-tested.
    edicts: *mut u8,
    edicts_bytes: usize,
}

impl VmRaw {
    /// # Safety
    ///
    /// `vm` must point at a live `qcvm_t` whose `progs`, `statements`,
    /// `functions` and `globals` lumps are loaded, and whose `edicts` array is
    /// `edicts_bytes` long. It must stay valid and unaliased-by-Rust for the
    /// lifetime of the view; the host frame's `PR_SwitchQCVM` discipline is
    /// the exclusivity argument (ADR-007, ADR-008).
    pub unsafe fn new(vm: *mut QcVm) -> Self {
        assert!(!vm.is_null());
        // SAFETY: the caller guarantees `vm` is a live qcvm_t.
        let (edicts, bytes) = unsafe {
            let stride = (*vm).edict_size.max(0) as usize;
            let count = (*vm).max_edicts.max(0) as usize;
            ((*vm).edicts.cast::<u8>(), stride * count)
        };
        Self {
            vm,
            edicts,
            edicts_bytes: bytes,
        }
    }

    fn vm(&self) -> *mut QcVm {
        self.vm
    }

    /// `qcvm->progs->numfunctions`
    #[must_use]
    pub fn numfunctions(&self) -> c_int {
        // SAFETY: `progs` is loaded by the constructor's contract.
        unsafe { (*(*self.vm).progs).numfunctions }
    }

    #[must_use]
    pub fn numbuiltins(&self) -> c_int {
        // SAFETY: a plain field read of the live qcvm_t.
        unsafe { (*self.vm).numbuiltins }
    }

    /// `qcvm->trace` — the `Con_Printf` single-step flag a builtin can set.
    #[must_use]
    pub fn trace_flag(&self) -> bool {
        // SAFETY: as above.
        unsafe { (*self.vm).trace }
    }

    pub fn set_trace_flag(&mut self, on: bool) {
        // SAFETY: as above.
        unsafe { (*self.vm).trace = on }
    }

    #[must_use]
    pub fn depth(&self) -> c_int {
        // SAFETY: as above.
        unsafe { (*self.vm).depth }
    }

    pub fn set_depth(&mut self, d: c_int) {
        // SAFETY: as above.
        unsafe { (*self.vm).depth = d }
    }

    #[must_use]
    pub fn argc(&self) -> c_int {
        // SAFETY: as above.
        unsafe { (*self.vm).argc }
    }

    pub fn set_argc(&mut self, n: c_int) {
        // SAFETY: as above.
        unsafe { (*self.vm).argc = n }
    }

    #[must_use]
    pub fn xstatement(&self) -> c_int {
        // SAFETY: as above.
        unsafe { (*self.vm).xstatement }
    }

    pub fn set_xstatement(&mut self, pc: c_int) {
        // SAFETY: as above.
        unsafe { (*self.vm).xstatement = pc }
    }

    /// `PR_GetString` without taking any borrow of the VM, so the interpreter
    /// can resolve a string across a builtin dispatch.
    pub fn get_string(&self, num: c_int) -> Result<*const c_char, StringError> {
        // SAFETY: `knownstrings` holds at least `numknownstrings` entries;
        // both are read from the live qcvm_t.
        unsafe {
            resolve_string(
                (*self.vm).strings,
                (*self.vm).stringssize,
                (*self.vm).knownstrings,
                (*self.vm).numknownstrings,
                num,
            )
        }
    }

    // ---- def tables ------------------------------------------------------

    /// `qcvm->progs->numfielddefs`
    #[must_use]
    pub fn numfielddefs(&self) -> c_int {
        // SAFETY: `progs` is loaded by the constructor's contract.
        unsafe { (*(*self.vm).progs).numfielddefs }
    }

    /// `qcvm->progs->numglobaldefs`
    #[must_use]
    pub fn numglobaldefs(&self) -> c_int {
        // SAFETY: as above.
        unsafe { (*(*self.vm).progs).numglobaldefs }
    }

    /// `qcvm->progs->entityfields`
    #[must_use]
    pub fn entityfields(&self) -> c_int {
        // SAFETY: as above.
        unsafe { (*(*self.vm).progs).entityfields }
    }

    /// `qcvm->fielddefs[i]`
    #[must_use]
    pub fn fielddef(&self, i: c_int) -> DDef {
        debug_assert!(i >= 0 && i < self.numfielddefs());
        // SAFETY: the caller stays within numfielddefs, and the table is at
        // least that long (PR_MergeEngineFieldDefs only grows it).
        unsafe { (*self.vm).fielddefs.add(i as usize).read() }
    }

    /// `qcvm->globaldefs[i]`
    #[must_use]
    pub fn globaldef(&self, i: c_int) -> DDef {
        debug_assert!(i >= 0 && i < self.numglobaldefs());
        // SAFETY: as above.
        unsafe { (*self.vm).globaldefs.add(i as usize).read() }
    }

    /// `qcvm->functions[i].s_name`, for `PR_UglyValueString`'s `ev_function`.
    #[must_use]
    pub fn function_name_handle(&self, i: c_int) -> Option<c_int> {
        if i < 0 || i >= self.numfunctions() {
            return None;
        }
        // SAFETY: bounds-checked immediately above.
        Some(unsafe { (*self.vm).functions.add(i as usize).read() }.s_name)
    }

    /// `qcvm->max_edicts`
    #[must_use]
    pub fn max_edicts(&self) -> c_int {
        // SAFETY: a plain field read of the live qcvm_t.
        unsafe { (*self.vm).max_edicts }
    }

    /// `qcvm->num_edicts`
    #[must_use]
    pub fn num_edicts(&self) -> c_int {
        // SAFETY: as above.
        unsafe { (*self.vm).num_edicts }
    }

    pub fn set_num_edicts(&mut self, n: c_int) {
        // SAFETY: as above.
        unsafe { (*self.vm).num_edicts = n }
    }

    /// `qcvm->extfields.alpha` — `ED_Write`'s manual-alpha fallback.
    #[must_use]
    pub fn extfield_alpha(&self) -> c_int {
        // SAFETY: as above.
        unsafe { (*self.vm).extfields.alpha }
    }

    /// The progs-visible field words of one edict, addressed as C does:
    /// `(int *)((char *)&ed->v + ofs * 4)`.
    #[must_use]
    pub fn edict_field_words(&self, num: c_int, word_ofs: c_int, count: usize) -> Option<Vec<i32>> {
        let byteofs = self.field_byte_offset(num * self.edict_stride(), word_ofs);
        (0..count)
            .map(|i| self.ed_i32(byteofs + (i as i32) * 4))
            .collect()
    }

    /// `ed->alpha`, the engine-side byte `ED_Write` falls back to.
    #[must_use]
    pub fn edict_alpha(&self, num: c_int) -> u8 {
        let stride = self.edict_stride() as usize;
        // SAFETY: num < max_edicts, and `alpha` sits in the fixed header.
        unsafe {
            core::ptr::addr_of!((*self.edicts.add(num as usize * stride).cast::<Edict>()).alpha)
                .read()
        }
    }

    /// `ed->free`
    #[must_use]
    pub fn edict_free(&self, num: c_int) -> bool {
        let stride = self.edict_stride() as usize;
        // SAFETY: as above.
        unsafe {
            core::ptr::addr_of!((*self.edicts.add(num as usize * stride).cast::<Edict>()).free)
                .read()
        }
    }

    /// `qcvm->time`
    #[must_use]
    pub fn time(&self) -> f64 {
        // SAFETY: a plain field read of the live qcvm_t.
        unsafe { (*self.vm).time }
    }

    /// Copy `bytes` into a buffer `PR_AllocString`/`Mem_Alloc` handed back.
    ///
    /// A no-op on a null buffer, matching `ED_NewString`'s behaviour when
    /// `PR_AllocString` returned handle 0 for a zero length.
    pub fn write_engine_string(&mut self, buf: *mut c_char, bytes: &[u8]) {
        if buf.is_null() {
            return;
        }
        // SAFETY: `buf` is an allocation of at least `bytes.len()` bytes --
        // both callers size it from the same string they copy in.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), buf.cast::<u8>(), bytes.len()) };
    }

    /// `qcvm->knownzone[id >> 3] & (1u << (id & 7))` — is this engine string
    /// one `ED_RezoneString`/`PF_strzone` allocated?
    #[must_use]
    pub fn knownzone_test(&self, id: usize) -> bool {
        // SAFETY: guarded by the size check, matching C's own.
        unsafe {
            id < (*self.vm).knownzonesize
                && ((*self.vm).knownzone.add(id >> 3).read() & (1u8 << (id & 7))) != 0
        }
    }

    pub fn knownzone_clear(&mut self, id: usize) {
        // SAFETY: callers test knownzone_test first, which bounds `id`.
        unsafe {
            let p = (*self.vm).knownzone.add(id >> 3);
            p.write(p.read() & !(1u8 << (id & 7)));
        }
    }

    pub fn knownzone_set(&mut self, id: usize) {
        debug_assert!(self.knownzone_capacity() > id);
        // SAFETY: knownzone_grow_to has made the byte addressable.
        unsafe {
            let p = (*self.vm).knownzone.add(id >> 3);
            p.write(p.read() | (1u8 << (id & 7)));
        }
    }

    #[must_use]
    fn knownzone_capacity(&self) -> usize {
        // SAFETY: a plain field read of the live qcvm_t.
        unsafe { (*self.vm).knownzonesize }
    }

    /// `ED_RezoneString`'s bitmap growth, verbatim: the new size is
    /// `(id + 32) & ~7` bits, and only the bytes past the old size are zeroed.
    pub fn knownzone_grow_to(&mut self, id: usize, mem: &mut dyn Mem) {
        // SAFETY: the fields belong to the live qcvm_t.
        unsafe {
            if id >= (*self.vm).knownzonesize {
                let old_size = ((*self.vm).knownzonesize + 7) >> 3;
                (*self.vm).knownzonesize = (id + 32) & !7;
                let new_size = ((*self.vm).knownzonesize + 7) >> 3;
                (*self.vm).knownzone = mem.realloc((*self.vm).knownzone, new_size);
                core::ptr::write_bytes((*self.vm).knownzone.add(old_size), 0, new_size - old_size);
            }
        }
    }

    /// The VM's string table, built from the live `qcvm_t`'s own fields.
    ///
    /// Takes `&mut self` so it cannot coexist with another mutable view of
    /// the same VM, but holds only raw pointers, so nothing it returns aliases
    /// a Rust reference across a builtin dispatch.
    pub fn string_table(&mut self) -> StringTable<'_> {
        // SAFETY: every field below belongs to the live qcvm_t the
        // constructor was given.
        unsafe {
            StringTable {
                strings: (*self.vm).strings,
                stringssize: (*self.vm).stringssize,
                knownstrings: core::ptr::addr_of_mut!((*self.vm).knownstrings),
                knownstringsowned: core::ptr::addr_of_mut!((*self.vm).knownstringsowned),
                maxknownstrings: core::ptr::addr_of_mut!((*self.vm).maxknownstrings),
                numknownstrings: core::ptr::addr_of_mut!((*self.vm).numknownstrings),
                progsstrings: (*self.vm).progsstrings,
                freeknownstrings: core::ptr::addr_of_mut!((*self.vm).freeknownstrings),
                _marker: core::marker::PhantomData,
            }
        }
    }

    /// `PR_GetString` as bytes up to the NUL, for writers that format it.
    ///
    /// The returned slice borrows the progs string blob or a `knownstrings`
    /// entry, both of which outlive any single VM step.
    pub fn get_string_bytes(&self, num: c_int) -> Result<&[u8], StringError> {
        let p = self.get_string(num)?;
        // SAFETY: resolve_string only ever returns a pointer into the progs
        // string blob or a live knownstrings entry; both are NUL-terminated.
        Ok(unsafe { core::ffi::CStr::from_ptr(p) }.to_bytes())
    }

    /// `!*PR_GetString (handle)` — whether a progs string is empty, which is
    /// the only dereference `OP_NOT_S` performs.
    pub fn string_is_empty(&self, num: c_int) -> Result<bool, StringError> {
        let p = self.get_string(num)?;
        // SAFETY: resolve_string returns either a pointer into the progs
        // string blob or a live knownstrings entry; both are NUL-terminated,
        // so the first byte is readable.
        Ok(unsafe { p.read() } == 0)
    }

    /// `pr_global_struct->self` — `qcvm->globals` viewed as `globalvars_t`.
    #[must_use]
    pub fn global_self(&self) -> i32 {
        self.g_i32(offset_of!(quake_types::progs::GlobalVars, self_) / 4)
    }

    /// `pr_global_struct->time`
    #[must_use]
    pub fn global_time(&self) -> f32 {
        self.g_f32(offset_of!(quake_types::progs::GlobalVars, time) / 4)
    }

    #[must_use]
    pub fn xfunction(&self) -> FuncRef {
        // SAFETY: as above.
        FuncRef(unsafe { (*self.vm).xfunction })
    }

    pub fn set_xfunction(&mut self, f: FuncRef) {
        // SAFETY: as above.
        unsafe { (*self.vm).xfunction = f.0 }
    }

    /// `&qcvm->functions[i]`, as the raw pointer C stores in `xfunction` and
    /// on the call stack.
    #[must_use]
    pub fn function_ptr(&self, index: c_int) -> FuncRef {
        // SAFETY: callers check `index` against numfunctions first.
        FuncRef(unsafe { (*self.vm).functions.add(index as usize) })
    }

    /// Index of a function pointer, i.e. C's `f - qcvm->functions`.
    #[must_use]
    pub fn function_index(&self, f: FuncRef) -> c_int {
        // SAFETY: `f` came from `function_ptr`, so it is inside the array.
        let base = unsafe { (*self.vm).functions };
        (((f.0 as usize) - (base as usize)) / size_of::<DFunction>()) as c_int
    }

    #[must_use]
    pub fn function(&self, f: FuncRef) -> DFunction {
        // SAFETY: a FuncRef only ever points into the loaded functions lump.
        unsafe { f.0.read() }
    }

    /// `qcvm->xfunction->profile += delta`
    pub fn add_profile(&mut self, f: FuncRef, delta: c_int) {
        // SAFETY: as above.
        unsafe {
            let p = core::ptr::addr_of_mut!((*f.0).profile);
            p.write(p.read().wrapping_add(delta));
        }
    }

    #[must_use]
    pub fn statement(&self, pc: c_int) -> DStatement {
        // SAFETY: `pc` is produced by the interpreter's own control flow over
        // a lump the loader validated; a progs whose jumps leave the lump is
        // out-of-domain for C too (it has no check either).
        unsafe { (*self.vm).statements.add(pc as usize).read() }
    }

    // ---- global block -----------------------------------------------------

    fn globals(&self) -> *mut f32 {
        // SAFETY: `globals` is loaded by the constructor's contract.
        unsafe { (*self.vm).globals }
    }

    #[must_use]
    pub fn g_f32(&self, ofs: usize) -> f32 {
        // SAFETY: offsets come from `(unsigned short)st->a`-style operands,
        // bounded by the globals lump the loader sized.
        unsafe { self.globals().add(ofs).read() }
    }

    pub fn set_g_f32(&mut self, ofs: usize, v: f32) {
        // SAFETY: as above.
        unsafe { self.globals().add(ofs).write(v) }
    }

    #[must_use]
    pub fn g_i32(&self, ofs: usize) -> i32 {
        // SAFETY: as above; the global block is an untyped word array.
        unsafe { self.globals().add(ofs).cast::<i32>().read() }
    }

    pub fn set_g_i32(&mut self, ofs: usize, v: i32) {
        // SAFETY: as above.
        unsafe { self.globals().add(ofs).cast::<i32>().write(v) }
    }

    #[must_use]
    pub fn g_vec3(&self, ofs: usize) -> [f32; 3] {
        [self.g_f32(ofs), self.g_f32(ofs + 1), self.g_f32(ofs + 2)]
    }

    pub fn set_g_vec3(&mut self, ofs: usize, v: [f32; 3]) {
        self.set_g_f32(ofs, v[0]);
        self.set_g_f32(ofs + 1, v[1]);
        self.set_g_f32(ofs + 2, v[2]);
    }

    /// Raw words of the global block, for the trace's `B` and `R` records.
    #[must_use]
    pub fn g_words(&self, ofs: usize, count: usize) -> Vec<i32> {
        (0..count).map(|i| self.g_i32(ofs + i)).collect()
    }

    // ---- edict field access ----------------------------------------------

    /// Byte offset within the edict array is in range.
    ///
    /// COMPAT (accepted divergence, ADR-006): C's `OP_STOREP_*`, `OP_LOAD_*`
    /// and `OP_STATE`'s `ed->v.frame`/`nextthink`/`think` writes do no bounds
    /// check at all — a progs with an out-of-range field offset silently reads
    /// or corrupts whatever follows the edict array. The port range-tests
    /// instead and raises, because the alternative is an arbitrary-write
    /// primitive reachable from mod data. No progs in the corpus reaches it,
    /// so trace parity is unaffected.
    fn edict_byte_ok(&self, byteofs: i32, width: usize) -> bool {
        byteofs >= 0 && (byteofs as usize).saturating_add(width) <= self.edicts_bytes
    }

    #[must_use]
    pub fn ed_i32(&self, byteofs: i32) -> Option<i32> {
        if !self.edict_byte_ok(byteofs, 4) {
            return None;
        }
        // SAFETY: range-checked immediately above.
        Some(unsafe {
            self.edicts
                .add(byteofs as usize)
                .cast::<i32>()
                .read_unaligned()
        })
    }

    pub fn set_ed_i32(&mut self, byteofs: i32, v: i32) -> Option<()> {
        if !self.edict_byte_ok(byteofs, 4) {
            return None;
        }
        // SAFETY: range-checked immediately above.
        unsafe {
            self.edicts
                .add(byteofs as usize)
                .cast::<i32>()
                .write_unaligned(v)
        };
        Some(())
    }

    /// The three raw words at `byteofs`, for the trace's `P` records.
    #[must_use]
    pub fn ed_words3(&self, byteofs: i32) -> Option<[i32; 3]> {
        if !self.edict_byte_ok(byteofs, 12) {
            return None;
        }
        // SAFETY: range-checked immediately above.
        Some(unsafe {
            self.edicts
                .add(byteofs as usize)
                .cast::<[i32; 3]>()
                .read_unaligned()
        })
    }

    #[must_use]
    pub fn ed_vec3(&self, byteofs: i32) -> Option<[f32; 3]> {
        if !self.edict_byte_ok(byteofs, 12) {
            return None;
        }
        // SAFETY: range-checked immediately above.
        Some(unsafe {
            self.edicts
                .add(byteofs as usize)
                .cast::<[f32; 3]>()
                .read_unaligned()
        })
    }

    pub fn set_ed_vec3(&mut self, byteofs: i32, v: [f32; 3]) -> Option<()> {
        if !self.edict_byte_ok(byteofs, 12) {
            return None;
        }
        // SAFETY: range-checked immediately above.
        unsafe {
            self.edicts
                .add(byteofs as usize)
                .cast::<[f32; 3]>()
                .write_unaligned(v);
        }
        Some(())
    }

    /// `PROG_TO_EDICT (p) == qcvm->edicts`, i.e. the world entity.
    #[must_use]
    pub fn is_world(&self, prog: i32) -> bool {
        prog == 0
    }

    /// `(byte *)((int *)&ed->v + ofs) - (byte *)qcvm->edicts` for `OP_ADDRESS`,
    /// and the base for the `OP_LOAD_*` family. `prog` is an `EDICT_TO_PROG`
    /// byte offset; `ofs` is a **word** offset into the field block.
    #[must_use]
    pub fn field_byte_offset(&self, prog: i32, word_ofs: i32) -> i32 {
        prog.wrapping_add(EDICT_V_OFFSET as i32)
            .wrapping_add(word_ofs.wrapping_mul(4))
    }

    // ---- call and local stacks -------------------------------------------

    pub fn push_stack(&mut self, depth: c_int, s: c_int, f: FuncRef) {
        // SAFETY: the caller checked 0 <= depth < MAX_STACK_DEPTH.
        unsafe {
            let slot = core::ptr::addr_of_mut!((*self.vm).stack)
                .cast::<PrStack>()
                .add(depth as usize);
            slot.write(PrStack { s, f: f.0 });
        }
    }

    /// `(qcvm->stack[depth].s, qcvm->stack[depth].f)`
    #[must_use]
    pub fn stack_slot(&self, depth: c_int) -> (c_int, FuncRef) {
        // SAFETY: the caller checked 0 <= depth < MAX_STACK_DEPTH.
        let slot = unsafe {
            core::ptr::addr_of!((*self.vm).stack)
                .cast::<PrStack>()
                .add(depth as usize)
                .read()
        };
        (slot.s, FuncRef(slot.f))
    }

    #[must_use]
    pub fn localstack_used(&self) -> c_int {
        // SAFETY: a plain field read of the live qcvm_t.
        unsafe { (*self.vm).localstack_used }
    }

    pub fn set_localstack_used(&mut self, n: c_int) {
        // SAFETY: as above.
        unsafe { (*self.vm).localstack_used = n }
    }

    pub fn localstack_write(&mut self, index: c_int, v: c_int) {
        // SAFETY: the caller checked index < LOCALSTACK_SIZE.
        unsafe {
            core::ptr::addr_of_mut!((*self.vm).localstack)
                .cast::<c_int>()
                .add(index as usize)
                .write(v);
        }
    }

    #[must_use]
    pub fn localstack_read(&self, index: c_int) -> c_int {
        // SAFETY: the caller checked index < LOCALSTACK_SIZE.
        unsafe {
            core::ptr::addr_of!((*self.vm).localstack)
                .cast::<c_int>()
                .add(index as usize)
                .read()
        }
    }

    /// `qcvm->edict_size` — the stride between edicts in the arena, used to
    /// convert between prog offsets and edict numbers.
    #[must_use]
    pub fn edict_stride(&self) -> i32 {
        // SAFETY: a plain field read of the live qcvm_t.
        unsafe { (*self.vm).edict_size }
    }

    /// `qcvm->edicts` as a byte pointer, for building an [`EdictArena`] over
    /// the same array this view was constructed from.
    #[must_use]
    pub fn edicts_base(&self) -> *mut u8 {
        self.edicts
    }

    /// The whole edict array as bytes — the differential suites compare it.
    #[must_use]
    pub fn edicts_bytes(&self) -> &[u8] {
        // SAFETY: the constructor recorded the array's true byte length, and
        // `&self` bounds the borrow.
        unsafe { core::slice::from_raw_parts(self.edicts, self.edicts_bytes) }
    }

    /// The raw `qcvm_t` pointer, for shims that must hand it back to C.
    #[must_use]
    pub fn as_ptr(&self) -> *mut QcVm {
        self.vm()
    }
}
