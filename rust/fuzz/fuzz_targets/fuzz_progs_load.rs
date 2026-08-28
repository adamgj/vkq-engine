//! `progs.dat` loader fuzzer (Phase 6 M6, ADR-019 gate 5).
//!
//! `PR_LoadProgs` is the engine's largest attack surface reachable from mod
//! data: a `progs.dat` is an untrusted file whose header decides where five
//! lumps live and how many entries each has, and C walks all five with no
//! validation at all. The port refuses an out-of-range lump instead
//! (`LoadError::LumpOutOfRange`); this target is what says that refusal is
//! total — every malformed header must come back as an error or a clean load,
//! never a panic, an out-of-bounds access or a wild pointer write.
//!
//! `quake-progs` is deliberately free of `quake-c-sys`, so the whole loader
//! runs here behind a mock `LoadSys` with no engine linked in. The true
//! C-vs-Rust comparison is the engine-level trace and savegame gates, which
//! run real game code (same design decision as the Phase 3/4/5 targets).

#![no_main]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::ffi::CString;

use libfuzzer_sys::fuzz_target;
use quake_progs::arena::Mem;
use quake_progs::image::{ProgsImage, VmLoad};
use quake_progs::load::{self, LoadSys};
use quake_types::progs::{BuiltinT, DDef, QcVm, PROGHEADER_CRC};

#[derive(Default)]
struct FuzzSys {
    live: Vec<(*mut u8, std::alloc::Layout)>,
    /// Every `(key, ddef)` the loader inserted into the fielddefs map, so
    /// `ED_FindFieldOffset` answers the way the engine's would.
    fields: Vec<(Vec<u8>, *const DDef)>,
    va: Vec<CString>,
    next_map: usize,
    fielddefs_map: *mut c_void,
    current: *mut QcVm,
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
        assert!(!p.is_null(), "Mem_Alloc aborts rather than returning null");
        self.live.push((p, layout));
        p
    }

    fn realloc(&mut self, ptr: *mut u8, size: usize) -> *mut u8 {
        let fresh = self.alloc(size);
        if let Some(i) = self.live.iter().position(|&(p, _)| p == ptr) {
            let old = self.live[i].1.size().min(size);
            // SAFETY: both blocks are live and at least `old` bytes long.
            unsafe { core::ptr::copy_nonoverlapping(ptr, fresh, old) };
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

impl LoadSys for FuzzSys {
    fn map_create(&mut self) -> *mut c_void {
        self.next_map += 1;
        let handle = self.next_map as *mut c_void;
        // the fielddefs map is the third and last one PR_LoadProgs creates
        if self.next_map == 3 {
            self.fielddefs_map = handle;
        }
        handle
    }

    fn map_reserve(&mut self, _map: *mut c_void, _capacity: c_int) {}

    fn map_insert(&mut self, map: *mut c_void, key: *const c_char, value: *const c_void) {
        // SAFETY: every key is a NUL-terminated C string inside either the
        // image's strings blob, a static literal, or a `va` buffer.
        let bytes = unsafe { CStr::from_ptr(key) }.to_bytes().to_vec();
        if map == self.fielddefs_map {
            self.fields.push((bytes, value.cast()));
        }
    }

    fn map_destroy(&mut self, _map: *mut c_void) {}

    fn ed_new_string(&mut self, _s: *const c_char) -> c_int {
        -1
    }

    fn flush_console(&mut self) {}
    fn set_empty_engine_string(&mut self) {}

    fn find_field_ofs(&mut self, name: &CStr) -> c_int {
        // last insert wins, exactly as HashMap_Insert overwrites -- which is
        // why PR_LoadProgs builds the map back to front
        self.fields
            .iter()
            .rev()
            .find(|(k, _)| k == name.to_bytes())
            // SAFETY: the recorded pointer is the ddef_t the loader inserted,
            // live for as long as the image and the merged table are.
            .map(|(_, d)| c_int::from(unsafe { d.read_unaligned() }.ofs))
            .unwrap_or(-1)
    }

    fn global_float(&mut self, _name: &CStr) -> Option<f32> {
        None
    }

    fn find_function(&mut self, _name: &CStr) -> Option<c_int> {
        None
    }

    fn va_component_name(&mut self, name: &CStr, component: u8) -> *const c_char {
        let mut s = name.to_bytes().to_vec();
        s.push(b'_');
        s.push(b'x' + component);
        self.va.push(CString::new(s).unwrap());
        self.va.last().unwrap().as_ptr()
    }

    fn shutdown_extensions(&mut self) {}
    fn enable_extensions(&mut self, _globaldefs: *mut DDef) {}

    fn switch_qcvm(&mut self, vm: *mut QcVm) {
        self.current = vm;
    }

    fn deselect_qcvm(&mut self) {
        self.current = core::ptr::null_mut();
    }

    fn current_qcvm(&mut self) -> *mut QcVm {
        self.current
    }

    fn set_pr_global_struct(&mut self, _globals: *mut f32) {}

    fn is_server_vm(&mut self, _vm: *mut QcVm) -> bool {
        true
    }

    fn set_effects_mask(&mut self, _mask: c_int) {}
    fn print(&mut self, _msg: &[u8]) {}
    fn dprint(&mut self, _msg: &[u8]) {}
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 2 {
        return;
    }
    // byte 0 picks `fatal`; byte 1 picks the expected CRC, so both the
    // "system vars modified" raise and the foreign-CRC console switch are
    // reachable without the corpus having to guess 5927
    let fatal = data[0] & 1 != 0;
    let needcrc = if data[0] & 2 != 0 {
        PROGHEADER_CRC
    } else {
        c_int::from(data[1])
    };

    let mut image_bytes = data[2..].to_vec();
    if image_bytes.is_empty() {
        return;
    }

    let mut sys = FuzzSys::default();
    // SAFETY: qcvm_t is a POD C struct; all-zeroes is what PR_SwitchQCVM
    // (NULL) leaves behind and what sv/cl start in.
    let mut vm: Box<QcVm> = unsafe { Box::new(core::mem::zeroed()) };
    let vm_ptr: *mut QcVm = &mut *vm;
    sys.current = vm_ptr;

    let builtins: [BuiltinT; 1] = [None];
    {
        // SAFETY: `vm` is a live zeroed qcvm_t and `image_bytes` a live buffer;
        // both outlive the views, which are dropped at the end of this block.
        let (mut view, mut image) = unsafe {
            (
                VmLoad::new(vm_ptr),
                ProgsImage::new(image_bytes.as_mut_ptr(), image_bytes.len()),
            )
        };
        let result = load::load_progs(
            &mut view,
            &mut image,
            c"progs.dat",
            fatal,
            needcrc,
            &builtins,
            &mut sys,
        );

        if result == Ok(true) {
            // A successful load must leave a self-consistent VM: every lump
            // pointer inside the image, and edict_size positive and pointer
            // aligned (it is the arena stride, ADR-006).
            //
            // The bound is `<= end`, not `< end`: a lump with a zero count
            // sitting exactly at the end of the file yields a legal
            // one-past-the-end pointer that is never dereferenced. The first
            // version of this assertion used `<` and the fuzzer produced that
            // shape within minutes.
            let base = image_bytes.as_ptr() as usize;
            let end = base + image_bytes.len();
            for p in [
                view.functions() as usize,
                view.statements() as usize,
                view.globaldefs() as usize,
                view.globals() as usize,
                view.strings() as usize,
            ] {
                assert!(p >= base && p <= end, "lump pointer escaped the image");
            }
            let align = core::mem::size_of::<*const c_void>() as c_int;
            assert!(view.edict_size() > 0);
            assert_eq!(view.edict_size() % align, 0);
            assert!(view.stringssize() >= 0);
        }
    }

    // Tear down through PR_ClearProgs so its free path -- in particular the
    // pointer comparison that decides whether `fielddefs` is the image's own
    // lump or a Mem_Alloc block -- runs on every input too. The image is a
    // Rust `Vec` rather than an allocator block, so `FuzzSys::free` ignores
    // it; everything the loader really allocated is released here, and
    // `FuzzSys::drop` frees whatever is left, so a double free or a missed
    // free shows up under ASan.
    // SAFETY: `vm` is still the live qcvm_t the load ran against.
    let mut view = unsafe { VmLoad::new(vm_ptr) };
    load::clear_progs(&mut view, &mut sys);
});
