//! Rust global allocator bound to the engine's vendored mimalloc.
//!
//! COMPAT: ADR-013 — one allocator on both sides of the boundary: this
//! forwards every Rust allocation to the *same* compiled mimalloc that
//! mem.c amalgamates (`#include "mimalloc/static.c"`), not a second copy
//! (libmimalloc-sys was rejected because it builds its own). Buffers whose
//! ownership crosses the language boundary still go through the `Mem_*`
//! C API; this only puts Rust-internal allocations on the shared heap.
//!
//! Compiled only with the `engine-alloc` cargo feature, which the Meson
//! build sets: plain `cargo test`/`cargo build` binaries do not link
//! mem.o, so the `mi_*` symbols are absent there and the default system
//! allocator is used instead.

use core::alloc::{GlobalAlloc, Layout};
use quake_c_sys::mi;

pub struct EngineMiMalloc;

// SAFETY: forwards the GlobalAlloc contract onto mimalloc's aligned API,
// which provides malloc-compatible semantics for any size and any
// power-of-two alignment; a pointer returned by one of these functions is
// valid until passed to mi_free/mi_realloc_aligned exactly once.
unsafe impl GlobalAlloc for EngineMiMalloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: Layout guarantees a nonzero power-of-two alignment;
        // mi_malloc_aligned accepts any such size/alignment pair.
        unsafe { mi::mi_malloc_aligned(layout.size(), layout.align()).cast() }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: as alloc, and mi_zalloc_aligned zeroes the block.
        unsafe { mi::mi_zalloc_aligned(layout.size(), layout.align()).cast() }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        // SAFETY: GlobalAlloc guarantees ptr was returned by this allocator,
        // i.e. by one of the mi_* functions above, and is freed only once.
        unsafe { mi::mi_free(ptr.cast()) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: ptr came from this allocator with layout.align(), which
        // GlobalAlloc requires the reallocated block to keep; mimalloc
        // preserves the contents up to min(old, new) size.
        unsafe { mi::mi_realloc_aligned(ptr.cast(), new_size, layout.align()).cast() }
    }
}

#[global_allocator]
static GLOBAL: EngineMiMalloc = EngineMiMalloc;
