//! Savegame/entity-text value parser fuzzer (Phase 6, ADR-019 gate 5).
//!
//! `ED_ParseEpair` is where untrusted savegame and `.ent` text turns into
//! writes through an edict's field block: the key's `type`/`ofs` come from the
//! progs, the value from the file, and the `ev_entity` arm additionally
//! allocates edicts and mutates the free list. The M1–M5 compatibility review
//! found two live defects in exactly this function (a negative entity number
//! that marked the world entity allocated, and an over-wide destination slice)
//! and named the absence of this target as the gap that let them through.
//!
//! Both the arena and the parser run over Rust-owned memory here — no engine
//! is linked in — so every write is inside an allocation ASan can police. The
//! C-vs-Rust comparison is `progs_parse_differential` and the savegame
//! byte-diff gate.

#![no_main]

use core::ffi::{c_int, CStr};
use std::ffi::CString;

use libfuzzer_sys::fuzz_target;
use quake_progs::arena::{EdictArena, EdictId, Mem, VmRaw};
use quake_progs::parse::{self, ParseSys};
use quake_progs::save::value_words;
use quake_types::progs::{etype, DPrograms, FreeList, QcVm, DEF_SAVEGLOBAL};

/// Every `etype_t` `ED_ParseEpair` has an arm for, plus one it does not (so
/// the `default: break` path is reached too).
const TYPES: [c_int; 12] = [
    etype::EV_STRING,
    etype::EV_FLOAT,
    etype::EV_VECTOR,
    etype::EV_ENTITY,
    etype::EV_FIELD,
    etype::EV_FUNCTION,
    etype::EV_POINTER,
    etype::EV_EXT_INTEGER,
    etype::EV_EXT_UINT32,
    etype::EV_EXT_SINT64,
    etype::EV_EXT_DOUBLE,
    etype::EV_VOID,
];

const ENTITYFIELDS: c_int = 128;
const MAX_EDICTS: c_int = 64;

#[derive(Default)]
struct FuzzSys {
    live: Vec<(*mut u8, std::alloc::Layout)>,
    unlinked: Vec<EdictId>,
}

impl Drop for FuzzSys {
    fn drop(&mut self) {
        for (p, layout) in core::mem::take(&mut self.live) {
            // SAFETY: `p` came from this allocator with `layout`.
            unsafe { std::alloc::dealloc(p, layout) };
        }
    }
}

impl Mem for FuzzSys {
    fn alloc(&mut self, size: usize) -> *mut u8 {
        let layout = std::alloc::Layout::from_size_align(size.max(1), 8).unwrap();
        // SAFETY: non-zero size, valid alignment; zeroed like Mem_Alloc.
        let p = unsafe { std::alloc::alloc_zeroed(layout) };
        assert!(!p.is_null());
        self.live.push((p, layout));
        p
    }

    fn realloc(&mut self, ptr: *mut u8, size: usize) -> *mut u8 {
        let fresh = self.alloc(size);
        if let Some(i) = self.live.iter().position(|&(p, _)| p == ptr) {
            let n = self.live[i].1.size().min(size);
            // SAFETY: both blocks are live and at least `n` bytes long.
            unsafe { core::ptr::copy_nonoverlapping(ptr, fresh, n) };
            self.free(ptr);
        }
        fresh
    }

    fn free(&mut self, ptr: *mut u8) {
        if let Some(i) = self.live.iter().position(|&(p, _)| p == ptr) {
            let (p, layout) = self.live.remove(i);
            // SAFETY: `p` came from this allocator with `layout`.
            unsafe { std::alloc::dealloc(p, layout) };
        }
    }

    fn note_slot_growth(&mut self, _maxknownstrings: c_int) {}
}

/// COMPAT (ADR-010): the engine calls the platform conversions, and their
/// rounding is the savegame round-trip's contract. The fuzzer is not a
/// conformance oracle, so it calls the same libc through `std`'s CStr →
/// `strtod`-equivalent parse only where a Rust parse cannot diverge in a way
/// that matters here (a panic is what is being hunted, not a last-digit
/// difference).
impl ParseSys for FuzzSys {
    fn atof(&mut self, s: &CStr) -> f64 {
        prefix_parse::<f64>(s).unwrap_or(0.0)
    }

    fn atoi(&mut self, s: &CStr) -> c_int {
        prefix_parse::<i64>(s).unwrap_or(0) as c_int
    }

    fn strtoll(&mut self, s: &CStr) -> i64 {
        prefix_parse::<i64>(s).unwrap_or(0)
    }

    fn strtoull(&mut self, s: &CStr) -> u64 {
        prefix_parse::<u64>(s).unwrap_or(0)
    }

    fn find_field_ofs(&mut self, name: &CStr) -> Option<c_int> {
        (!name.to_bytes().is_empty()).then(|| c_int::from(name.to_bytes()[0]) % ENTITYFIELDS)
    }

    fn find_function(&mut self, name: &CStr) -> Option<c_int> {
        (!name.to_bytes().is_empty()).then(|| c_int::from(name.to_bytes()[0]))
    }

    fn unlink_edict(&mut self, id: EdictId) {
        self.unlinked.push(id);
    }

    fn dprint(&mut self, _p: &str, _a: &[u8], _s: &str) {}
    fn print(&mut self, _p: &str, _a: &[u8], _s: &str) {}
    fn dwarn(&mut self, _p: &str, _a: &[u8], _m: &str, _a2: &[u8], _s: &str) {}
}

/// The leading-numeric-prefix behaviour `strtod`/`strtoll` have and Rust's
/// `parse` does not; good enough to keep the fuzzer exercising real values
/// rather than always taking the zero path.
fn prefix_parse<T: core::str::FromStr>(s: &CStr) -> Option<T> {
    let text = core::str::from_utf8(s.to_bytes()).ok()?;
    let trimmed = text.trim_start();
    let mut end = 0;
    for (i, ch) in trimmed.char_indices() {
        let ok = ch.is_ascii_digit()
            || (i == 0 && (ch == '-' || ch == '+'))
            || ch == '.'
            || ch == 'e'
            || ch == 'E';
        if !ok {
            break;
        }
        end = i + ch.len_utf8();
    }
    trimmed[..end].parse().ok()
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }
    let ty = TYPES[(data[0] as usize) % TYPES.len()];
    let saveglobal = data[0] & 0x80 != 0;
    // a word offset anywhere in the field block, including the very last
    // word, where an over-wide destination slice would leave the allocation
    let word_ofs = c_int::from(data[1]) % ENTITYFIELDS;
    let zoned = data[2] & 1 != 0;
    let target = c_int::from(data[2] >> 1) % MAX_EDICTS;

    let Ok(value) = CString::new(data[3..].to_vec()) else {
        return; // an embedded NUL: not a token COM_Parse could produce
    };

    let mut sys = FuzzSys::default();
    // SAFETY: qcvm_t is a POD C struct; all-zeroes is the state sv/cl start in.
    let mut vm: Box<QcVm> = unsafe { Box::new(core::mem::zeroed()) };

    let stride = ENTITYFIELDS as usize * 4 + 512;
    let mut arena = EdictArena::owned(stride, MAX_EDICTS as usize);
    let mut strings = vec![0u8; 64];

    // entityfields lives in the progs header, which the parser reads through
    // VmRaw when it bounds an edict field write
    let mut header = Box::new(DPrograms {
        version: 0,
        crc: 0,
        ofs_statements: 0,
        numstatements: 0,
        ofs_globaldefs: 0,
        numglobaldefs: 0,
        ofs_fielddefs: 0,
        numfielddefs: 0,
        ofs_functions: 0,
        numfunctions: 0,
        ofs_strings: 0,
        numstrings: strings.len() as c_int,
        ofs_globals: 0,
        numglobals: 0,
        entityfields: ENTITYFIELDS,
    });

    vm.progs = &mut *header;
    vm.edicts = arena.base().cast();
    vm.edict_size = stride as c_int;
    vm.max_edicts = MAX_EDICTS;
    vm.num_edicts = 1;
    vm.strings = strings.as_mut_ptr().cast();
    vm.stringssize = strings.len() as c_int;

    let vm_ptr: *mut QcVm = &mut *vm;
    // SAFETY: every lump the parser touches is set above, and the arena owns
    // `max_edicts * edict_size` bytes.
    let mut raw = unsafe { VmRaw::new(vm_ptr) };

    // the destination is the parsed edict's own field block, exactly as
    // ED_ParseEdict passes `&ed->v`
    let words = value_words(ty);
    let dest_base = arena.base().wrapping_add(quake_progs::arena::EDICT_V_OFFSET);
    if (word_ofs as usize + words) * 4 > ENTITYFIELDS as usize * 4 {
        return;
    }
    // SAFETY: `dest` is `words` i32s inside edict 0's field block, which the
    // bound above keeps within `entityfields`.
    let dest = unsafe {
        core::slice::from_raw_parts_mut(
            dest_base.wrapping_add(word_ofs as usize * 4).cast::<i32>(),
            words,
        )
    };

    let key_type = if saveglobal {
        ty | c_int::from(DEF_SAVEGLOBAL)
    } else {
        ty
    };
    let mut free_list = Box::new(FreeList {
        size: 0,
        head_index: 0,
        circular_buffer: [0; quake_types::progs::MAX_EDICTS],
    });
    let _ = target;

    let _ = parse::ed_parse_epair(
        &mut raw,
        &mut arena,
        &mut free_list,
        &mut sys,
        dest,
        key_type,
        0,
        &value,
        zoned,
    );

    // Whatever happened, the VM must still be internally consistent: an
    // ev_entity value may only have raised num_edicts within max_edicts, and
    // the free list may only hold as many entries as there are edicts.
    // SAFETY: `vm` is still live; the parser took a raw view of it, not a
    // borrow, and that view has been dropped.
    let (num_edicts, max_edicts) = unsafe { ((*vm_ptr).num_edicts, (*vm_ptr).max_edicts) };
    assert!(num_edicts >= 1 && num_edicts <= max_edicts);
    assert!(free_list.size <= quake_types::progs::MAX_EDICTS);
});
