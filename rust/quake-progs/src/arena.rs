//! The untyped edict arena and the progs string table (ADR-006).
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

use quake_types::progs::{Edict, EntityState, QBoolean};

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
    /// stay valid and unaliased for the lifetime of the returned arena, and
    /// `stride` must be the `qcvm->edict_size` those edicts were laid out
    /// with. Callers additionally guarantee the host-frame exclusivity
    /// argument of ADR-007/ADR-008.
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
pub struct StringTable<'a> {
    pub strings: *const c_char,
    pub stringssize: c_int,
    pub knownstrings: &'a mut *mut *const c_char,
    pub knownstringsowned: &'a mut *mut QBoolean,
    pub maxknownstrings: &'a mut c_int,
    pub numknownstrings: &'a mut c_int,
    pub progsstrings: c_int,
    pub freeknownstrings: &'a mut c_int,
}

/// `pr_edict.c`
const PR_STRING_ALLOCSLOTS: c_int = 256;

impl StringTable<'_> {
    fn known(&self, i: c_int) -> *const c_char {
        debug_assert!(i >= 0 && i < *self.numknownstrings);
        // SAFETY: callers only pass indices below numknownstrings, and the
        // array is at least maxknownstrings >= numknownstrings entries.
        unsafe { (*self.knownstrings).add(i as usize).read() }
    }

    fn set_known(&mut self, i: c_int, ptr: *const c_char, owned: bool) {
        debug_assert!(i >= 0 && i < *self.maxknownstrings);
        // SAFETY: as above; i < maxknownstrings is the array's true bound.
        unsafe {
            (*self.knownstrings).add(i as usize).write(ptr);
            (*self.knownstringsowned).add(i as usize).write(owned);
        }
    }

    fn is_owned(&self, i: c_int) -> bool {
        // SAFETY: as above.
        unsafe { (*self.knownstringsowned).add(i as usize).read() }
    }

    fn alloc_slots(&mut self, mem: &mut dyn Mem) {
        *self.maxknownstrings += PR_STRING_ALLOCSLOTS;
        mem.note_slot_growth(*self.maxknownstrings);
        let n = *self.maxknownstrings as usize;
        *self.knownstrings = mem
            .realloc(
                (*self.knownstrings).cast::<u8>(),
                n * size_of::<*const c_char>(),
            )
            .cast();
        *self.knownstringsowned = mem
            .realloc(
                (*self.knownstringsowned).cast::<u8>(),
                n * size_of::<QBoolean>(),
            )
            .cast();
    }

    /// `PR_GetString`.
    ///
    /// COMPAT: the invalid-offset arm returns `qcvm->strings` — the empty
    /// string at the head of the blob — and the `Host_Error` after it is dead
    /// code (`pr_edict.c`). Out-of-range handles therefore fail silently, and
    /// that is preserved.
    ///
    /// The one *live* error is a negative handle whose slot is null.
    #[must_use]
    pub fn get(&self, num: c_int) -> Result<*const c_char, StringError> {
        if num >= 0 && num < self.stringssize {
            // SAFETY: 0 <= num < stringssize, inside the blob.
            return Ok(unsafe { self.strings.add(num as usize) });
        }
        if num < 0 && num >= -*self.numknownstrings {
            let slot = -1 - num;
            let p = self.known(slot);
            if p.is_null() {
                return Err(StringError::NonExistent(num));
            }
            return Ok(p);
        }
        Ok(self.strings)
    }

    /// `PR_ClearEngineString`.
    pub fn clear_engine_string(&mut self, num: c_int, mem: &mut dyn Mem) {
        if num < 0 && num >= -*self.numknownstrings {
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
            if *self.freeknownstrings > i {
                *self.freeknownstrings = i;
            }
        }
    }

    /// The `for (i = freeknownstrings;; i++)` slot search shared verbatim by
    /// `PR_SetEngineString` and `PR_AllocString`.
    fn take_slot(&mut self, mem: &mut dyn Mem) -> c_int {
        let mut i = *self.freeknownstrings;
        loop {
            if i < *self.numknownstrings {
                if !self.known(i).is_null() {
                    i += 1;
                    continue;
                }
            } else {
                if i >= *self.maxknownstrings {
                    self.alloc_slots(mem);
                }
                *self.numknownstrings += 1;
            }
            break;
        }
        *self.freeknownstrings = i + 1;
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
        for i in 0..*self.numknownstrings {
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
        for i in self.progsstrings..*self.numknownstrings {
            if self.is_owned(i) {
                mem.free(self.known(i).cast_mut().cast::<u8>());
                self.set_known(i, core::ptr::null(), false);
            }
        }
        if !quake_types::progs::ENGINE_DEBUG {
            *self.freeknownstrings = self.progsstrings;
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
