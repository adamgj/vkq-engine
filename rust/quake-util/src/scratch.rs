//! Scratch buffers: a fixed inline capacity used when it suffices, spilling
//! to the heap for larger requests. This is the Phase 2 stand-in for the C
//! `TEMP_ALLOC` macro family (mem.h) that ROADMAP Phase 2 calls for.
//!
//! It is a near, not exact, counterpart, and deliberately so:
//!
//! - the C macros `alloca` into the *caller's* frame; the inline storage
//!   here is a struct field, so it lives wherever the `ScratchBuf` does
//!   (the caller's frame for a local, which is the intended use) and costs
//!   `N * size_of::<T>()` of stack even on the heap path;
//! - the C tracks a per-thread `alloca` budget and falls back to
//!   `Mem_Alloc` past it; the spill threshold here is the const `N`. That
//!   budget only bounds C stack usage and is not observable across the FFI
//!   boundary (see the COMPAT note below).
//!
//! No Phase 2 code uses it — the ported filesystem has no `TEMP_ALLOC` call
//! sites. Its consumers are the `TEMP_ALLOC`-heavy modules that arrive with
//! the formats and renderer phases (`gl_model.c` alone has 65).
//!
//! COMPAT: ADR-013 — the C macros guard a per-thread `alloca` budget
//! (`thread_stack_alloc_size` vs `max_thread_stack_alloc_size`) before
//! falling back to `Mem_Alloc`. That budget only bounds C stack usage and is
//! not observable across the FFI boundary, so the Rust equivalent keeps the
//! same shape (stack first, heap past a fixed capacity) without mirroring
//! the thread-local counter.

/// A scratch buffer of `len` `T`s: inline storage up to `N`, heap beyond.
///
/// The buffer is fully initialized with `T::default()` (the `TEMP_ALLOC`
/// zeroed variants; the non-zeroed C variants exist only to skip a memset,
/// which is not an observable difference).
pub struct ScratchBuf<T: Copy + Default, const N: usize> {
    inline: [T; N],
    heap: Vec<T>,
    len: usize,
}

impl<T: Copy + Default, const N: usize> ScratchBuf<T, N> {
    /// C: `TEMP_ALLOC_ZEROED (type, var, len)` / `TEMP_ALLOC (type, var, len)`
    pub fn new(len: usize) -> Self {
        Self {
            inline: [T::default(); N],
            heap: if len > N {
                vec![T::default(); len]
            } else {
                Vec::new()
            },
            len,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// True when the request spilled past the inline capacity (the
    /// `temp_alloc_*_on_heap` flag of the C macros).
    pub fn on_heap(&self) -> bool {
        self.len > N
    }
}

impl<T: Copy + Default, const N: usize> core::ops::Deref for ScratchBuf<T, N> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        if self.len > N {
            &self.heap
        } else {
            &self.inline[..self.len]
        }
    }
}

impl<T: Copy + Default, const N: usize> core::ops::DerefMut for ScratchBuf<T, N> {
    fn deref_mut(&mut self) -> &mut [T] {
        if self.len > N {
            &mut self.heap
        } else {
            &mut self.inline[..self.len]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_within_capacity() {
        let mut buf = ScratchBuf::<u8, 16>::new(8);
        assert_eq!(buf.len(), 8);
        assert!(!buf.on_heap());
        assert!(buf.iter().all(|&b| b == 0));
        buf[7] = 42;
        assert_eq!(buf[7], 42);
    }

    #[test]
    fn heap_past_capacity() {
        let mut buf = ScratchBuf::<u32, 4>::new(100);
        assert_eq!(buf.len(), 100);
        assert!(buf.on_heap());
        assert!(buf.iter().all(|&v| v == 0));
        buf[99] = 7;
        assert_eq!(buf[99], 7);
    }

    #[test]
    fn empty() {
        let buf = ScratchBuf::<u8, 4>::new(0);
        assert!(buf.is_empty());
        assert_eq!(buf.iter().count(), 0);
    }
}
