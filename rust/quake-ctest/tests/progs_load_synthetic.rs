//! `quake_progs::load` over synthetic `progs.dat` images (Phase 6 M6).
//!
//! Unlike the other Phase 6 suites this one has **no C oracle**, and that is a
//! deliberate, recorded deviation from the milestone plan rather than an
//! oversight. `PR_LoadProgs` reaches `COM_LoadFile`, `PR_EnableExtensions` and
//! `PR_ShutdownExtensions`; the first is renamed to `c_ref_COM_LoadFile` by the
//! differential prelude and would need a staged gamedir, and the other two live
//! in `pr_ext.c`, which is not an oracle file. Standing an oracle up would
//! therefore mean either a filesystem fixture per case or stubbing out the
//! extension layer both sides call — neither of which compares more than the
//! engine gates already do.
//!
//! What *does* compare the loader against C is the engine level, and it is
//! strong: `trace_diff.py` is byte-identical over id1 e1m1/e2m1/e3m1 and
//! hipnotic/rogue/rerelease `start` (223k–489k records each), which cannot
//! hold unless the globals block, the statement and function lumps and all
//! three hash maps came out identical; `save_diff.py` covers `edict_size` and
//! the fielddef merge, which decide savegame layout.
//!
//! This suite covers what the corpus cannot reach: the diagnostic arms, the
//! merge arithmetic in isolation, and malformed images. `fuzz_progs_load`
//! covers the rest of the malformed-input space.

use core::ffi::{c_char, c_int, c_void, CStr};
use std::ffi::CString;

use quake_ctest as _;
use quake_progs::arena::Mem;
use quake_progs::image::{ProgsImage, VmLoad};
use quake_progs::load::{self, LoadError, LoadSys};
use quake_types::progs::{
    etype, BuiltinT, DDef, Edict, EntVars, QcVm, DEF_SAVEGLOBAL, PROGHEADER_CRC, PROG_VERSION,
};

// ---------------------------------------------------------------------------
// synthetic progs images

#[derive(Clone, Default)]
struct ProgsSpec {
    version: i32,
    crc: i32,
    /// `(name, first_statement)`
    functions: Vec<(&'static str, i32)>,
    /// `(name, type, ofs)`
    globaldefs: Vec<(&'static str, u16, u16)>,
    fielddefs: Vec<(&'static str, u16, u16)>,
    numstatements: i32,
    globals: Vec<i32>,
    entityfields: i32,
    /// Overrides written into the header *after* it is laid out, for the
    /// malformed-image cases.
    header_patch: Vec<(usize, i32)>,
}

impl ProgsSpec {
    fn new() -> Self {
        Self {
            version: PROG_VERSION,
            crc: PROGHEADER_CRC,
            numstatements: 1,
            entityfields: 4,
            globals: vec![0; 32],
            ..Default::default()
        }
    }

    fn build(&self) -> Vec<u8> {
        let mut strings: Vec<u8> = vec![0];
        let handle = |s: &str, blob: &mut Vec<u8>| -> i32 {
            let at = blob.len() as i32;
            blob.extend_from_slice(s.as_bytes());
            blob.push(0);
            at
        };
        let fn_names: Vec<i32> = self
            .functions
            .iter()
            .map(|(n, _)| handle(n, &mut strings))
            .collect();
        let gd_names: Vec<i32> = self
            .globaldefs
            .iter()
            .map(|(n, _, _)| handle(n, &mut strings))
            .collect();
        let fd_names: Vec<i32> = self
            .fielddefs
            .iter()
            .map(|(n, _, _)| handle(n, &mut strings))
            .collect();

        let mut out = vec![0u8; 60];
        let push_ofs = |out: &mut Vec<u8>| -> i32 {
            while !out.len().is_multiple_of(4) {
                out.push(0);
            }
            out.len() as i32
        };

        let ofs_strings = push_ofs(&mut out);
        out.extend_from_slice(&strings);

        let ofs_statements = push_ofs(&mut out);
        out.extend(std::iter::repeat_n(
            0u8,
            self.numstatements.max(0) as usize * 8,
        ));

        let ofs_functions = push_ofs(&mut out);
        for (i, (_, first)) in self.functions.iter().enumerate() {
            let mut f = [0i32; 7];
            f[0] = *first;
            f[4] = fn_names[i];
            for w in f {
                out.extend_from_slice(&w.to_le_bytes());
            }
            out.extend_from_slice(&[0u8; 8]); // parm_size
        }

        let ofs_globaldefs = push_ofs(&mut out);
        for (i, (_, ty, ofs)) in self.globaldefs.iter().enumerate() {
            out.extend_from_slice(&ty.to_le_bytes());
            out.extend_from_slice(&ofs.to_le_bytes());
            out.extend_from_slice(&gd_names[i].to_le_bytes());
        }

        let ofs_fielddefs = push_ofs(&mut out);
        for (i, (_, ty, ofs)) in self.fielddefs.iter().enumerate() {
            out.extend_from_slice(&ty.to_le_bytes());
            out.extend_from_slice(&ofs.to_le_bytes());
            out.extend_from_slice(&fd_names[i].to_le_bytes());
        }

        let ofs_globals = push_ofs(&mut out);
        for g in &self.globals {
            out.extend_from_slice(&g.to_le_bytes());
        }

        let header = [
            self.version,
            self.crc,
            ofs_statements,
            self.numstatements,
            ofs_globaldefs,
            self.globaldefs.len() as i32,
            ofs_fielddefs,
            self.fielddefs.len() as i32,
            ofs_functions,
            self.functions.len() as i32,
            ofs_strings,
            strings.len() as i32,
            ofs_globals,
            self.globals.len() as i32,
            self.entityfields,
        ];
        for (i, v) in header.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        for &(word, value) in &self.header_patch {
            out[word * 4..word * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        out
    }
}

// ---------------------------------------------------------------------------
// the LoadSys mock

/// Which map an insert landed in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MapKind {
    Function,
    GlobalDef,
    FieldDef,
}

struct Insert {
    map: usize,
    key: Vec<u8>,
    value: *const c_void,
}

#[derive(Default)]
struct MockSys {
    live: Vec<(*mut u8, std::alloc::Layout)>,
    freed: usize,
    maps: Vec<usize>,
    reserves: Vec<(usize, c_int)>,
    inserts: Vec<Insert>,
    destroyed: Vec<usize>,
    printed: Vec<Vec<u8>>,
    dprinted: Vec<Vec<u8>>,
    /// `ED_NewString` results, in call order.
    new_strings: Vec<CString>,
    va_bufs: Vec<CString>,
    empty_string_calls: usize,
    enable_calls: usize,
    shutdown_calls: usize,
    effects_mask: Option<c_int>,
    is_server: bool,
    qex_globals: Vec<(&'static str, f32)>,
    current: *mut QcVm,
    switched: Vec<*mut QcVm>,
    /// Base and count of the functions lump, so `find_function` can turn a
    /// recorded map value back into an index.
    functions_base: *const u8,
}

impl MockSys {
    fn map_kind(&self, map: usize) -> MapKind {
        // creation order in PR_LoadProgs: functions, globaldefs, fielddefs
        match self.maps.iter().position(|&m| m == map) {
            Some(0) => MapKind::Function,
            Some(1) => MapKind::GlobalDef,
            _ => MapKind::FieldDef,
        }
    }

    fn last_insert(&self, kind: MapKind, key: &str) -> Option<&Insert> {
        self.inserts
            .iter()
            .rfind(|i| self.map_kind(i.map) == kind && i.key == key.as_bytes())
    }

    fn keys(&self, kind: MapKind) -> Vec<String> {
        self.inserts
            .iter()
            .filter(|i| self.map_kind(i.map) == kind)
            .map(|i| String::from_utf8_lossy(&i.key).into_owned())
            .collect()
    }

    fn console(&self) -> String {
        self.printed
            .iter()
            .map(|m| String::from_utf8_lossy(m).into_owned())
            .collect()
    }
}

fn key_bytes(p: *const c_char) -> Vec<u8> {
    // SAFETY: every key the loader inserts is a NUL-terminated C string —
    // either a progs symbol, a static literal, or a `va` buffer.
    unsafe { CStr::from_ptr(p) }.to_bytes().to_vec()
}

impl Mem for MockSys {
    fn alloc(&mut self, size: usize) -> *mut u8 {
        let layout = std::alloc::Layout::from_size_align(size.max(1), 8).unwrap();
        // SAFETY: non-zero size, valid alignment; zeroed like Mem_Alloc.
        let p = unsafe { std::alloc::alloc_zeroed(layout) };
        self.live.push((p, layout));
        p
    }

    fn realloc(&mut self, ptr: *mut u8, size: usize) -> *mut u8 {
        let fresh = self.alloc(size);
        if !ptr.is_null() {
            if let Some(i) = self.live.iter().position(|&(p, _)| p == ptr) {
                let (_, old) = self.live[i];
                let n = old.size().min(size);
                // SAFETY: both blocks are live and at least `n` bytes.
                unsafe { core::ptr::copy_nonoverlapping(ptr, fresh, n) };
            }
            self.free(ptr);
        }
        fresh
    }

    fn free(&mut self, ptr: *mut u8) {
        if ptr.is_null() {
            return;
        }
        self.freed += 1;
        if let Some(i) = self.live.iter().position(|&(p, _)| p == ptr) {
            let (p, layout) = self.live.remove(i);
            // SAFETY: `p` came from this allocator with `layout`.
            unsafe { std::alloc::dealloc(p, layout) };
        }
    }

    fn note_slot_growth(&mut self, _maxknownstrings: c_int) {}
}

impl LoadSys for MockSys {
    fn map_create(&mut self) -> *mut c_void {
        let id = self.maps.len() + 1;
        self.maps.push(id);
        id as *mut c_void
    }

    fn map_reserve(&mut self, map: *mut c_void, capacity: c_int) {
        self.reserves.push((map as usize, capacity));
    }

    fn map_insert(&mut self, map: *mut c_void, key: *const c_char, value: *const c_void) {
        self.inserts.push(Insert {
            map: map as usize,
            key: key_bytes(key),
            value,
        });
    }

    fn map_destroy(&mut self, map: *mut c_void) {
        self.destroyed.push(map as usize);
    }

    fn ed_new_string(&mut self, s: *const c_char) -> c_int {
        // SAFETY: the loader only passes NUL-terminated names.
        let owned = unsafe { CStr::from_ptr(s) }.to_owned();
        self.new_strings.push(owned);
        -(self.new_strings.len() as c_int)
    }

    fn flush_console(&mut self) {}

    fn set_empty_engine_string(&mut self) {
        self.empty_string_calls += 1;
    }

    fn find_field_ofs(&mut self, name: &CStr) -> c_int {
        let name = String::from_utf8_lossy(name.to_bytes()).into_owned();
        match self.last_insert(MapKind::FieldDef, &name) {
            // SAFETY: the recorded value is the ddef_t pointer the loader
            // inserted, which is live for the VM's lifetime.
            Some(i) => c_int::from(unsafe { i.value.cast::<DDef>().read_unaligned() }.ofs),
            None => -1,
        }
    }

    fn global_float(&mut self, name: &CStr) -> Option<f32> {
        let name = String::from_utf8_lossy(name.to_bytes()).into_owned();
        self.qex_globals
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| *v)
    }

    fn find_function(&mut self, name: &CStr) -> Option<c_int> {
        let name = String::from_utf8_lossy(name.to_bytes()).into_owned();
        let base = self.functions_base as usize;
        self.last_insert(MapKind::Function, &name)
            .map(|i| ((i.value as usize - base) / 36) as c_int)
    }

    fn va_component_name(&mut self, name: &CStr, component: u8) -> *const c_char {
        let mut s = name.to_bytes().to_vec();
        s.push(b'_');
        s.push(b'x' + component);
        self.va_bufs.push(CString::new(s).unwrap());
        self.va_bufs.last().unwrap().as_ptr()
    }

    fn shutdown_extensions(&mut self) {
        self.shutdown_calls += 1;
    }

    fn enable_extensions(&mut self, _globaldefs: *mut DDef) {
        self.enable_calls += 1;
    }

    fn switch_qcvm(&mut self, vm: *mut QcVm) {
        self.switched.push(vm);
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
        self.is_server
    }

    fn set_effects_mask(&mut self, mask: c_int) {
        self.effects_mask = Some(mask);
    }

    fn print(&mut self, msg: &[u8]) {
        self.printed.push(msg.to_vec());
    }

    fn dprint(&mut self, msg: &[u8]) {
        self.dprinted.push(msg.to_vec());
    }
}

// ---------------------------------------------------------------------------
// fixture

struct Fixture {
    vm: Box<QcVm>,
    /// The file as it arrived, for the CRC/hash comparison — the loader
    /// byteswaps its copy in place.
    raw: Vec<u8>,
    /// The image the loader owns, allocated through the mock allocator so
    /// `PR_ClearProgs`' frees are observable.
    image: *mut u8,
    len: usize,
    sys: MockSys,
}

impl Fixture {
    fn new(spec: &ProgsSpec) -> Self {
        // SAFETY: qcvm_t is a POD C struct; all-zeroes is the state
        // PR_SwitchQCVM (NULL) leaves behind, and what C's `sv`/`cl` start in.
        let vm: Box<QcVm> = unsafe { Box::new(core::mem::zeroed()) };
        let raw = spec.build();
        let mut sys = MockSys::default();
        let image = sys.alloc(raw.len());
        // SAFETY: `image` is a fresh block of `raw.len()` bytes.
        unsafe { core::ptr::copy_nonoverlapping(raw.as_ptr(), image, raw.len()) };
        Self {
            vm,
            len: raw.len(),
            raw,
            image,
            sys,
        }
    }

    /// The loader's copy of the image, after any in-place mutation.
    fn image_bytes(&self) -> &[u8] {
        // SAFETY: the block is live for the fixture's lifetime.
        unsafe { core::slice::from_raw_parts(self.image, self.len) }
    }

    fn header_word(&self, i: usize) -> i32 {
        i32::from_le_bytes(self.raw[i * 4..i * 4 + 4].try_into().unwrap())
    }

    fn function_first_statement(&self, index: usize) -> i32 {
        let at = self.header_word(8) as usize + index * 36;
        i32::from_le_bytes(self.image_bytes()[at..at + 4].try_into().unwrap())
    }

    fn run(&mut self, fatal: bool, needcrc: c_int) -> Result<bool, LoadError> {
        self.sys.functions_base = self.image.wrapping_add(self.header_word(8) as usize);

        let vm_ptr: *mut QcVm = &mut *self.vm;
        // SAFETY: `vm` is a live zeroed qcvm_t owned by this fixture, and
        // `image` is the block it takes ownership of.
        let (mut view, mut image) =
            unsafe { (VmLoad::new(vm_ptr), ProgsImage::new(self.image, self.len)) };
        self.sys.current = vm_ptr;
        let builtins: [BuiltinT; 3] = [None; 3];
        load::load_progs(
            &mut view,
            &mut image,
            c"progs.dat",
            fatal,
            needcrc,
            &builtins,
            &mut self.sys,
        )
    }

    fn clear(&mut self) {
        let vm_ptr: *mut QcVm = &mut *self.vm;
        // SAFETY: the fixture's VM is live and loaded.
        let mut view = unsafe { VmLoad::new(vm_ptr) };
        self.sys.current = vm_ptr;
        load::clear_progs(&mut view, &mut self.sys);
    }
}

fn minimal() -> ProgsSpec {
    let mut s = ProgsSpec::new();
    s.functions = vec![("main", 1)];
    s.globaldefs = vec![("time", etype::EV_FLOAT as u16, 4)];
    s.fielddefs = vec![("origin", etype::EV_VECTOR as u16, 0)];
    s
}

const EDICT_HEADER: usize = core::mem::size_of::<Edict>() - core::mem::size_of::<EntVars>();

// ---------------------------------------------------------------------------

#[test]
fn a_minimal_progs_loads_and_fills_the_vm() {
    let spec = minimal();
    let mut f = Fixture::new(&spec);
    assert_eq!(f.run(true, PROGHEADER_CRC), Ok(true));

    assert_eq!(
        f.vm.progscrc,
        quake_util::crc::crc_block(&f.raw),
        "the CRC is taken over the file as it arrived, before the byteswap"
    );
    assert_eq!(f.vm.progshash, quake_util::mdfour::block_checksum(&f.raw));
    assert_eq!(f.vm.progssize as usize, f.len);
    assert_eq!(f.vm.numbuiltins, 3);
    assert_eq!(f.vm.stringssize, f.header_word(11));
    assert_eq!(f.sys.empty_string_calls, 1);
    assert_eq!(f.sys.enable_calls, 1);
    assert!(!f.vm.functions.is_null() && !f.vm.globals.is_null());
}

#[test]
fn edict_size_is_the_rounded_entityfields_expression() {
    // `alpha scale emiteffectnum traileffectnum tag_entity tag_index
    //  modelflags` are seven floats, `colormod` a vector: +10 words.
    let spec = minimal();
    let mut f = Fixture::new(&spec);
    assert_eq!(f.run(true, PROGHEADER_CRC), Ok(true));

    let entityfields = spec.entityfields + 10;
    let align = core::mem::size_of::<*const c_void>() as c_int;
    let expected = (entityfields * 4 + EDICT_HEADER as c_int + align - 1) & !(align - 1);
    assert_eq!(f.vm.edict_size, expected);
    assert_eq!(f.vm.edict_size % align, 0, "must be pointer aligned");
}

#[test]
fn hash_maps_are_built_in_reverse_so_duplicates_resolve_to_the_first() {
    let mut spec = minimal();
    spec.functions = vec![("dup", 1), ("dup", 2), ("other", 3)];
    let mut f = Fixture::new(&spec);
    assert_eq!(f.run(true, PROGHEADER_CRC), Ok(true));

    let order = f.sys.keys(MapKind::Function);
    assert_eq!(
        order,
        vec!["other", "dup", "dup"],
        "C inserts functions back-to-front so the *last* write is index 0"
    );
    assert_eq!(
        f.sys.find_function(c"dup"),
        Some(0),
        "a duplicate symbol must resolve the way a linear search would"
    );
}

#[test]
fn every_map_is_reserved_and_the_fielddefs_map_carries_the_autofield_margin() {
    let mut spec = minimal();
    spec.fielddefs = vec![
        ("origin", etype::EV_VECTOR as u16, 0),
        ("health", etype::EV_FLOAT as u16, 3),
    ];
    spec.globaldefs = vec![("time", etype::EV_FLOAT as u16, 4)];
    let mut f = Fixture::new(&spec);
    assert_eq!(f.run(true, PROGHEADER_CRC), Ok(true));

    let by_kind: Vec<(MapKind, c_int)> = f
        .sys
        .reserves
        .iter()
        .map(|&(m, n)| (f.sys.map_kind(m), n))
        .collect();
    assert_eq!(
        by_kind,
        vec![
            (MapKind::Function, 1),
            (MapKind::GlobalDef, 1),
            // countof (extrafields) * 3 == 24, "assume size of vectors for
            // all engine autofields, for margin"
            (MapKind::FieldDef, 2 + 24),
        ]
    );
}

#[test]
fn the_engine_fields_are_appended_at_the_end_of_the_entity_block() {
    let mut spec = minimal();
    spec.entityfields = 8;
    let mut f = Fixture::new(&spec);
    assert_eq!(f.run(true, PROGHEADER_CRC), Ok(true));

    let mut ofs = |name: &str| f.sys.find_field_ofs(&CString::new(name).unwrap());
    assert_eq!(ofs("alpha"), 8);
    assert_eq!(ofs("scale"), 9);
    assert_eq!(ofs("emiteffectnum"), 10);
    assert_eq!(ofs("traileffectnum"), 11);
    assert_eq!(ofs("tag_entity"), 12);
    assert_eq!(ofs("tag_index"), 13);
    assert_eq!(ofs("modelflags"), 14);
    assert_eq!(ofs("colormod"), 15);
    assert_eq!(f.vm.extfields.alpha, 8);
    assert_eq!(f.vm.extfields.colormod, 15);
    // and the ones the engine names but this progs does not define
    assert_eq!(f.vm.extfields.gravity, -1);
}

#[test]
fn a_field_the_progs_already_defines_is_not_re_added() {
    let mut spec = minimal();
    spec.entityfields = 8;
    spec.fielddefs = vec![
        ("origin", etype::EV_VECTOR as u16, 0),
        ("alpha", etype::EV_FLOAT as u16, 3),
    ];
    let mut f = Fixture::new(&spec);
    assert_eq!(f.run(true, PROGHEADER_CRC), Ok(true));

    assert_eq!(
        f.sys.find_field_ofs(c"alpha"),
        3,
        "the progs' own .alpha wins; the merge must not shadow it"
    );
    assert_eq!(
        f.sys.find_field_ofs(c"scale"),
        8,
        "and the next autofield takes the slot .alpha would have had"
    );
    assert!(
        !f.sys.new_strings.iter().any(|s| s.to_bytes() == b"alpha"),
        "no engine string is allocated for a field that already exists"
    );
}

#[test]
fn colormod_components_are_keyed_on_the_rotating_va_buffer() {
    let spec = minimal();
    let mut f = Fixture::new(&spec);
    assert_eq!(f.run(true, PROGHEADER_CRC), Ok(true));

    let names: Vec<String> = f
        .sys
        .va_bufs
        .iter()
        .map(|s| String::from_utf8_lossy(s.to_bytes()).into_owned())
        .collect();
    assert_eq!(names, vec!["colormod_x", "colormod_y", "colormod_z"]);

    // COMPAT: the map key is the `va` pointer itself, which is why these
    // three are unfindable in C once the buffer ring wraps. What is asserted
    // here is that the port routes them through `va` rather than through a
    // stable Rust string, i.e. that the quirk is preserved.
    for (i, name) in names.iter().enumerate() {
        let insert = f.sys.last_insert(MapKind::FieldDef, name).unwrap();
        assert_eq!(insert.key, name.as_bytes());
        // SAFETY: the recorded value is the ddef_t the loader wrote.
        let def = unsafe { insert.value.cast::<DDef>().read_unaligned() };
        assert_eq!(def.type_, etype::EV_FLOAT as u16 | DEF_SAVEGLOBAL);
        assert_eq!(
            c_int::from(def.ofs),
            f.sys.find_field_ofs(c"colormod") + i as c_int
        );
    }
}

#[test]
fn a_wrong_version_is_fatal_or_prints_depending_on_the_flag() {
    let mut spec = minimal();
    spec.version = 7;

    let mut f = Fixture::new(&spec);
    assert_eq!(f.run(true, PROGHEADER_CRC), Err(LoadError::WrongVersion(7)));

    let mut f = Fixture::new(&spec);
    assert_eq!(f.run(false, PROGHEADER_CRC), Ok(false));
    assert_eq!(f.sys.console(), "progs.dat ABI set not supported\n");
    assert!(f.vm.progs.is_null(), "the non-fatal arm clears qcvm->progs");
}

#[test]
fn the_foreign_crc_switch_matches_the_c_text() {
    let cases: [(i32, &str); 9] = [
        (22390, "progs.dat - full csqc is not supported\n"),
        (52195, "progs.dat - obsolete csqc is not supported\n"),
        (54730, "progs.dat - quakeworld gamecode is not supported\n"),
        (26940, "progs.dat - prerelease gamecode is not supported\n"),
        (32401, "progs.dat - tenebrae gamecode is not supported\n"),
        (38488, "progs.dat - hexen2 gamecode is not supported\n"),
        (26905, "progs.dat - hexen2 gamecode is not supported\n"),
        (14046, "progs.dat - hexen2 gamecode is not supported\n"),
        (999, "progs.dat system vars are not supported\n"),
    ];
    for (crc, text) in cases {
        let mut spec = minimal();
        spec.crc = crc;
        let mut f = Fixture::new(&spec);
        assert_eq!(f.run(false, PROGHEADER_CRC), Ok(false));
        assert_eq!(f.sys.console(), text, "crc {crc}");
    }

    let mut spec = minimal();
    spec.crc = 22390;
    let mut f = Fixture::new(&spec);
    assert_eq!(
        f.run(true, PROGHEADER_CRC),
        Err(LoadError::CrcMismatch),
        "with fatal set the switch is never reached"
    );
}

#[test]
fn a_fielddef_carrying_def_saveglobal_raises() {
    let mut spec = minimal();
    spec.fielddefs = vec![("origin", etype::EV_VECTOR as u16 | DEF_SAVEGLOBAL, 0)];
    let mut f = Fixture::new(&spec);
    assert_eq!(
        f.run(true, PROGHEADER_CRC),
        Err(LoadError::FieldDefSaveGlobal)
    );
}

#[test]
fn strings_running_past_the_end_of_the_file_raise() {
    let mut spec = minimal();
    // numstrings (header word 11) far beyond the file
    spec.header_patch = vec![(11, 1 << 20)];
    let mut f = Fixture::new(&spec);
    assert_eq!(f.run(true, PROGHEADER_CRC), Err(LoadError::StringsPastEnd));
}

#[test]
fn a_lump_that_leaves_the_file_is_refused_rather_than_walked() {
    // COMPAT (accepted divergence): C walks it and byteswaps past the buffer.
    for word in [3usize, 9, 5, 7, 13] {
        let mut spec = minimal();
        spec.header_patch = vec![(word, 1 << 24)];
        let mut f = Fixture::new(&spec);
        assert_eq!(
            f.run(true, PROGHEADER_CRC),
            Err(LoadError::LumpOutOfRange),
            "header word {word}"
        );
    }
    // a negative offset is refused too
    let mut spec = minimal();
    spec.header_patch = vec![(2, -4)];
    let mut f = Fixture::new(&spec);
    assert_eq!(f.run(true, PROGHEADER_CRC), Err(LoadError::LumpOutOfRange));

    // and so is a negative `numstrings`, which C's own strings-past-end test
    // lets through (the sum only gets smaller) straight into
    // `qcvm->stringssize`, where it is used as a bound. Found by
    // fuzz_progs_load.
    let mut spec = minimal();
    spec.header_patch = vec![(11, -1)];
    let mut f = Fixture::new(&spec);
    assert_eq!(f.run(true, PROGHEADER_CRC), Err(LoadError::LumpOutOfRange));
}

#[test]
fn an_unterminated_string_table_is_refused() {
    // Every symbol name the loader hashes is `strings + s_name`, and the
    // engine's HashStr runs strlen on it. C's only guard is
    // `ofs_strings + numstrings >= com_filesize`, which says nothing about a
    // terminator, so a blob whose tail has no NUL makes the first hash-map
    // insert read past the file. Found by fuzz_progs_load under ASan.
    let spec = minimal();
    let raw = spec.build();
    let numstrings = i32::from_le_bytes(raw[44..48].try_into().unwrap());

    // shortening the blob by one byte puts its last byte inside a name
    let mut spec2 = minimal();
    spec2.header_patch = vec![(11, numstrings - 1)];
    let mut f = Fixture::new(&spec2);
    assert_eq!(
        f.run(true, PROGHEADER_CRC),
        Err(LoadError::UnterminatedStrings)
    );

    // an empty table has no terminator either
    let mut spec3 = minimal();
    spec3.header_patch = vec![(11, 0)];
    let mut f = Fixture::new(&spec3);
    assert_eq!(
        f.run(true, PROGHEADER_CRC),
        Err(LoadError::UnterminatedStrings)
    );

    // and the well-formed one still loads
    let mut f = Fixture::new(&spec);
    assert_eq!(f.run(true, PROGHEADER_CRC), Ok(true));
}

#[test]
fn an_autofield_declared_past_entityfields_does_not_overrun_the_merged_table() {
    // COMPAT (accepted divergence): PR_MergeEngineFieldDefs counts how many
    // defs to allocate in one loop and decides what to emit in another, and
    // the second loop re-derives "is this new?" from
    // `newidx >= entityfields && newidx < maxofs`. A progs that *declares*
    // one of the autofield names at an offset at or past `entityfields`
    // satisfies that test without having been counted, so C emits a def it
    // did not allocate room for -- a heap overflow, reachable from mod data.
    //
    // The port carries an explicit is-new flag instead, so the two loops
    // agree by construction. Found by fuzz_progs_load.
    let mut spec = minimal();
    spec.entityfields = 4;
    spec.fielddefs = vec![
        ("origin", etype::EV_VECTOR as u16, 0),
        // declared at exactly entityfields, which C's re-derivation
        // misreads as "newly assigned"
        ("scale", etype::EV_FLOAT as u16, 4),
    ];
    let mut f = Fixture::new(&spec);
    assert_eq!(f.run(true, PROGHEADER_CRC), Ok(true));

    // .scale keeps the offset the progs gave it, and is not re-emitted
    assert_eq!(f.sys.find_field_ofs(c"scale"), 4);
    assert!(
        !f.sys.new_strings.iter().any(|s| s.to_bytes() == b"scale"),
        "no engine string is allocated for a field that already exists"
    );
    // and the fields that really are new still land after it
    assert_eq!(f.sys.find_field_ofs(c"alpha"), 4);
    assert_eq!(f.sys.find_field_ofs(c"emiteffectnum"), 5);
}

#[test]
fn clear_progs_survives_an_image_too_short_to_hold_a_header() {
    // PR_LoadProgs sets qcvm->progs before it validates anything, so a
    // truncated file that raises still leaves the pointer installed, and the
    // teardown that follows compares against progs->ofs_fielddefs.
    //
    // COMPAT (accepted divergence): C reads that field unconditionally, i.e.
    // past the end of the buffer. Found by fuzz_progs_load under ASan.
    let spec = minimal();
    let raw = spec.build();
    for len in [1usize, 4, 16, 59] {
        let mut f = Fixture::new(&spec);
        f.len = len.min(raw.len());
        assert_eq!(
            f.run(true, PROGHEADER_CRC),
            Err(LoadError::TooShort(len.min(raw.len()))),
            "a {len}-byte image must not load"
        );
        // the teardown must not read the header it does not have
        f.clear();
        assert!(f.sys.live.is_empty(), "{len}-byte image");
    }
}

#[test]
fn an_unaligned_fielddefs_lump_is_copied_the_way_memcpy_would() {
    // Nothing requires ofs_fielddefs to be 4-aligned, and PR_MergeEngineFieldDefs
    // memcpy's the lump into its fresh table. The port's first version used a
    // *typed* copy_nonoverlapping, whose alignment precondition a lump at an
    // odd offset violates -- UB that C's memcpy does not have.
    //
    // Found by fuzz_progs_load. The image builder pads every lump to a word,
    // so the offset is perturbed through the header patch instead, which is
    // exactly what a hostile progs.dat would do.
    let spec = minimal();
    let raw = spec.build();
    let aligned_ofs = i32::from_le_bytes(raw[24..28].try_into().unwrap());

    for skew in [1i32, 2, 3] {
        let mut spec = minimal();
        // shift the lump back by `skew` bytes and drop the count to one so it
        // still lies inside the file
        spec.header_patch = vec![(6, aligned_ofs - skew)];
        let mut f = Fixture::new(&spec);
        assert_eq!(
            f.run(true, PROGHEADER_CRC),
            Ok(true),
            "ofs_fielddefs {} (skew {skew})",
            aligned_ofs - skew
        );
        // the merge ran, so the table was copied out of the unaligned lump
        assert_ne!(
            f.vm.fielddefs.cast::<u8>(),
            f.image.wrapping_add((aligned_ofs - skew) as usize)
        );
    }
}

#[test]
fn an_absurd_entityfields_is_refused_rather_than_overflowing_edict_size() {
    // COMPAT (accepted divergence): C computes edict_size with signed int
    // arithmetic and lets it overflow, then allocates max_edicts * edict_size
    // from the wrapped value. `entityfields` is a header field of an
    // untrusted progs.dat, so this is reachable from mod data.
    //
    // Found by fuzz_progs_load on its first run: the port panicked on the
    // multiplication instead, and a panic in the engine is an abort (ADR-009).
    for bad in [c_int::MAX, c_int::MAX / 4, 0x2000_0000, -1] {
        let mut spec = minimal();
        spec.header_patch = vec![(14, bad)];
        let mut f = Fixture::new(&spec);
        assert_eq!(
            f.run(true, PROGHEADER_CRC),
            Err(LoadError::BadEntityFields(bad)),
            "entityfields {bad}"
        );
    }

    // and a large-but-addressable value still loads
    let mut spec = minimal();
    spec.entityfields = 1 << 20;
    let mut f = Fixture::new(&spec);
    assert_eq!(f.run(true, PROGHEADER_CRC), Ok(true));
    assert!(f.vm.edict_size > 0);
}

#[test]
fn rerelease_builtins_are_patched_only_on_an_exact_match() {
    let mut spec = minimal();
    spec.functions = vec![
        ("centerprint", -90),
        ("bprint", -91),
        ("sprint", -92),
        ("other", -90),
    ];
    let mut f = Fixture::new(&spec);
    assert_eq!(f.run(true, PROGHEADER_CRC), Ok(true));
    assert_eq!(f.function_first_statement(0), -73);
    assert_eq!(f.function_first_statement(1), -23);
    assert_eq!(f.function_first_statement(2), -24);
    assert_eq!(
        f.function_first_statement(3),
        -90,
        "only the three named functions are patched"
    );

    // and a centerprint that is not at -90 is left alone
    let mut spec = minimal();
    spec.functions = vec![("centerprint", -60)];
    let mut f = Fixture::new(&spec);
    assert_eq!(f.run(true, PROGHEADER_CRC), Ok(true));
    assert_eq!(f.function_first_statement(0), -60);
}

#[test]
fn the_effects_mask_only_moves_for_the_server_vm() {
    let spec = minimal();

    let mut f = Fixture::new(&spec);
    f.sys.is_server = false;
    assert_eq!(f.run(true, PROGHEADER_CRC), Ok(true));
    assert_eq!(
        f.sys.effects_mask, None,
        "csqc never touches sv.effectsmask"
    );

    // no QEX globals: the three re-release bits are masked off, because
    // Arcane Dimensions uses the same bits for its own effects
    let mut f = Fixture::new(&spec);
    f.sys.is_server = true;
    assert_eq!(f.run(true, PROGHEADER_CRC), Ok(true));
    assert_eq!(f.sys.effects_mask, Some(!(16 | 32 | 64)));

    // EF_QUADLIGHT plus either spelling of the pentagram light
    for pent in ["EF_PENTLIGHT", "EF_PENTALIGHT"] {
        let mut f = Fixture::new(&spec);
        f.sys.is_server = true;
        f.sys.qex_globals = vec![("EF_QUADLIGHT", 16.0), (pent, 32.0)];
        assert_eq!(f.run(true, PROGHEADER_CRC), Ok(true));
        assert_eq!(f.sys.effects_mask, Some(-1), "{pent}");
    }

    // a QEX global with the wrong value does not count
    let mut f = Fixture::new(&spec);
    f.sys.is_server = true;
    f.sys.qex_globals = vec![("EF_QUADLIGHT", 8.0), ("EF_PENTLIGHT", 32.0)];
    assert_eq!(f.run(true, PROGHEADER_CRC), Ok(true));
    assert_eq!(f.sys.effects_mask, Some(!(16 | 32 | 64)));
}

#[test]
fn clear_progs_frees_the_merged_fielddefs_and_leaves_the_image_lump_alone() {
    let spec = minimal();
    let mut f = Fixture::new(&spec);
    assert_eq!(f.run(true, PROGHEADER_CRC), Ok(true));

    // the merge moved fielddefs out of the image into a Mem_Alloc block
    let merged = f.vm.fielddefs.cast::<u8>();
    assert!(
        f.sys.live.iter().any(|&(p, _)| p == merged),
        "the merged fielddef table is allocator-owned"
    );
    assert_ne!(merged, f.image, "and is no longer the image's own lump");

    f.clear();
    assert_eq!(f.sys.shutdown_calls, 1);
    assert_eq!(
        f.sys.destroyed.len(),
        3,
        "function_map, fielddefs_map and globaldefs_map"
    );
    assert!(
        f.sys.live.is_empty(),
        "both the merged fielddefs and the image are freed"
    );
    assert_eq!(f.vm.edict_size, 0, "the qcvm is memset to zero");
    // PR_ClearProgs restores whatever VM was ambient on entry -- here the one
    // it just wiped, which is exactly what PR_LoadProgs' opening
    // `PR_ClearProgs (qcvm)` leaves selected before it loads the new image.
    let vm_ptr: *mut QcVm = &mut *f.vm;
    assert_eq!(f.sys.current, vm_ptr);
}

#[test]
fn clear_progs_leaves_the_image_lump_alone_when_the_merge_did_not_run() {
    // A progs that already declares every autofield: maxdefs == numfielddefs,
    // so PR_MergeEngineFieldDefs returns early and qcvm->fielddefs is still
    // the image's own lump. PR_ClearProgs must not free it separately.
    let mut spec = minimal();
    spec.entityfields = 16;
    spec.fielddefs = vec![
        ("alpha", etype::EV_FLOAT as u16, 0),
        ("scale", etype::EV_FLOAT as u16, 1),
        ("emiteffectnum", etype::EV_FLOAT as u16, 2),
        ("traileffectnum", etype::EV_FLOAT as u16, 3),
        ("tag_entity", etype::EV_FLOAT as u16, 4),
        ("tag_index", etype::EV_FLOAT as u16, 5),
        ("modelflags", etype::EV_FLOAT as u16, 6),
        ("colormod", etype::EV_VECTOR as u16, 7),
    ];
    let mut f = Fixture::new(&spec);
    assert_eq!(f.run(true, PROGHEADER_CRC), Ok(true));
    assert_eq!(
        f.vm.fielddefs.cast::<u8>(),
        f.image.wrapping_add(f.header_word(6) as usize),
        "the merge must have been skipped entirely"
    );
    assert!(f.sys.new_strings.is_empty());

    f.clear();
    assert_eq!(f.sys.freed, 1, "only the image itself");
    assert!(f.sys.live.is_empty());
}
