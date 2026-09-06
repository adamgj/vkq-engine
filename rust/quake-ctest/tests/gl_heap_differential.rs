//! Differential for `quake_render::heap` vs the C `gl_heap.c` oracle
//! (Rust migration Phase 8 M3, ADR-015: "gl_heap property-tested vs C").
//!
//! Both sides run the same allocate/free trace against a fake memory
//! backend that hands out sequential handles (`stubs/gl_heap_ref.c` for C),
//! so after every operation the differential can compare not just the
//! offset but the memory an allocation landed in -- segment identity and
//! dedicated-ness -- plus the full `glheapstats_t`. The traces are proptest
//! generated with `GL_HeapTest_f`'s distributions (exponential sizes up to
//! 64 KiB, power-of-two alignments up to 2^14) and never touch the stub
//! `COM_Rand` (its missing `& COM_RAND_MAX` is a separate carry item).
//!
//! The C `num_allocations` counter is passed as NULL here: it is a plain
//! pass-through to the seams on both sides (`gl_heap.rs` forwards it
//! untouched), and the fakes tolerate NULL.

use ash::vk::Handle;
use core::ffi::{c_char, c_int, c_void};
use proptest::prelude::*;
use quake_ctest as _;
use quake_render::heap::{DeviceMemoryBackend, Heap};
use quake_types::render::{GlHeapStats, VulkanMemory, VulkanMemoryType};

extern "C" {
    fn c_ref_GL_HeapCreate(
        segment_size: u64,
        page_size: u32,
        memory_type_index: u32,
        memory_type: c_int,
        device_address: bool,
        heap_name: *const c_char,
    ) -> *mut c_void;
    fn c_ref_GL_HeapDestroy(heap: *mut c_void, num_allocations: *mut c_void);
    fn c_ref_GL_HeapAllocate(
        heap: *mut c_void,
        size: u64,
        alignment: u64,
        num_allocations: *mut c_void,
    ) -> *mut c_void;
    fn c_ref_GL_HeapFree(heap: *mut c_void, allocation: *mut c_void, num_allocations: *mut c_void);
    fn c_ref_GL_HeapGetAllocationMemory(allocation: *mut c_void) -> *mut c_void;
    fn c_ref_GL_HeapGetAllocationOffset(allocation: *mut c_void) -> u64;
    fn c_ref_GL_HeapGetStats(heap: *mut c_void) -> *const GlHeapStats;

    static mut c_ref_heap_next_handle: u64;
    static mut c_ref_heap_num_live: u32;
    static mut c_ref_heap_num_device_address_allocs: u32;
    static mut c_ref_heap_num_named: u32;
}

/// The Rust twin of `gl_heap_ref.c`'s fake seams.
#[derive(Default)]
struct FakeBackend {
    next_handle: u64,
    live: u32,
    device_address_allocs: u32,
}

impl DeviceMemoryBackend for FakeBackend {
    type Counter = ();
    fn allocate(
        &mut self,
        memory: &mut VulkanMemory,
        size: u64,
        _memory_type_index: u32,
        memory_type: VulkanMemoryType,
        device_address: bool,
        (): (),
    ) {
        self.next_handle += 1;
        self.live += 1;
        if device_address {
            self.device_address_allocs += 1;
        }
        memory.handle = ash::vk::DeviceMemory::from_raw(self.next_handle);
        memory.size = size as usize;
        memory.type_ = memory_type;
    }
    fn free(&mut self, memory: &mut VulkanMemory, (): ()) {
        self.live -= 1;
        *memory = VulkanMemory::default();
    }
}

/// One C heap; the oracle's fake-seam globals are per process, so tests
/// serialize on this lock and reset the globals at open.
struct CHeap(*mut c_void);

static C_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl CHeap {
    fn create(segment_size: u64, page_size: u32, device_address: bool) -> Self {
        // SAFETY: the globals are only touched under C_LOCK (held by the
        // caller for the whole test body); the name is a static literal.
        unsafe {
            c_ref_heap_next_handle = 0;
            c_ref_heap_num_live = 0;
            c_ref_heap_num_device_address_allocs = 0;
            c_ref_heap_num_named = 0;
            CHeap(c_ref_GL_HeapCreate(
                segment_size,
                page_size,
                0,
                VulkanMemoryType::None as c_int,
                device_address,
                c"Test Heap".as_ptr(),
            ))
        }
    }
    fn allocate(&self, size: u64, alignment: u64) -> *mut c_void {
        // SAFETY: live heap from `create`; the counter may be NULL.
        unsafe { c_ref_GL_HeapAllocate(self.0, size, alignment, core::ptr::null_mut()) }
    }
    fn free(&self, allocation: *mut c_void) {
        // SAFETY: `allocation` came from `allocate` on this heap, once.
        unsafe { c_ref_GL_HeapFree(self.0, allocation, core::ptr::null_mut()) }
    }
    fn stats(&self) -> GlHeapStats {
        // SAFETY: the pointer is into the live heap struct.
        unsafe { *c_ref_GL_HeapGetStats(self.0) }
    }
    fn destroy(&self) {
        // SAFETY: every allocation has been freed (asserted by the callers).
        unsafe { c_ref_GL_HeapDestroy(self.0, core::ptr::null_mut()) }
    }
    fn globals() -> (u64, u32, u32, u32) {
        // SAFETY: under C_LOCK.
        unsafe {
            (
                c_ref_heap_next_handle,
                c_ref_heap_num_live,
                c_ref_heap_num_device_address_allocs,
                c_ref_heap_num_named,
            )
        }
    }
}

fn c_alloc_memory(allocation: *mut c_void) -> u64 {
    // SAFETY: live allocation.
    unsafe { c_ref_GL_HeapGetAllocationMemory(allocation) as usize as u64 }
}

fn c_alloc_offset(allocation: *mut c_void) -> u64 {
    // SAFETY: live allocation.
    unsafe { c_ref_GL_HeapGetAllocationOffset(allocation) }
}

#[derive(Clone, Debug)]
enum Op {
    Alloc { size: u64, alignment: u64 },
    Free { index: usize },
}

/// `GL_HeapTest_f`'s distributions: `powf(r, 5)` sizes in `1..=64 KiB`,
/// `powf(r, 10)` alignment exponents in `0..14`; plus a rare
/// segment-or-larger size to reach the dedicated path.
fn op_strategy(segment_size: u64) -> impl Strategy<Value = Op> {
    const MAX_ALLOC_SIZE: f64 = 64.0 * 1024.0;
    const MAX_ALIGNMENT_POW2: f64 = 14.0;
    prop_oneof![
        6 => (0.0f64..1.0, 0.0f64..1.0).prop_map(|(r1, r2)| Op::Alloc {
            size: ((MAX_ALLOC_SIZE - 1.0) * r1.powi(5)) as u64 + 1,
            alignment: 1u64 << ((r2.powi(10) * MAX_ALIGNMENT_POW2) as u32),
        }),
        1 => (0u64..3).prop_map(move |extra| Op::Alloc {
            size: segment_size + extra * 4096,
            alignment: 1,
        }),
        5 => (0usize..1024).prop_map(|index| Op::Free { index }),
    ]
}

fn config_strategy() -> impl Strategy<Value = (u32, u64, bool)> {
    (
        prop_oneof![Just(128u32), Just(4096u32), Just(65536u32)],
        prop_oneof![Just(16u64), Just(256u64), Just(4096u64)],
        any::<bool>(),
    )
        .prop_map(|(page_size, pages, device_address)| {
            (page_size, u64::from(page_size) * pages, device_address)
        })
}

/// Runs one trace on both sides and compares after every operation; returns
/// the oracle's final seam counters (read under the lock).
fn run_trace(
    page_size: u32,
    segment_size: u64,
    device_address: bool,
    ops: &[Op],
) -> (u64, u32, u32, u32) {
    let _guard = C_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let c = CHeap::create(segment_size, page_size, device_address);
    let mut rs: Heap<FakeBackend> = Heap::new(
        FakeBackend::default(),
        segment_size,
        page_size,
        0,
        VulkanMemoryType::None,
        device_address,
    );
    let mut live: Vec<(*mut c_void, quake_render::heap::Allocation)> = Vec::new();

    for (step, op) in ops.iter().enumerate() {
        match *op {
            Op::Alloc { size, alignment } => {
                let ca = c.allocate(size, alignment);
                let ra = rs
                    .allocate(size, alignment, ())
                    .unwrap_or_else(|| panic!("step {step}: Rust failed to allocate {op:?}"));
                assert_eq!(
                    c_alloc_offset(ca),
                    ra.offset(),
                    "step {step} offset for {op:?}"
                );
                assert_eq!(
                    c_alloc_memory(ca),
                    ra.memory().as_raw(),
                    "step {step} memory handle for {op:?}"
                );
                assert_eq!(ra.offset() % alignment, 0, "step {step} alignment");
                live.push((ca, ra));
            }
            Op::Free { index } => {
                if live.is_empty() {
                    continue;
                }
                let (ca, ra) = live.swap_remove(index % live.len());
                c.free(ca);
                rs.free(ra, ());
            }
        }
        assert_eq!(c.stats(), *rs.stats(), "step {step} stats after {op:?}");
        rs.check_consistency().unwrap();
        let (next_handle, num_live, num_da, _) = CHeap::globals();
        assert_eq!(
            next_handle,
            rs.backend().next_handle,
            "step {step} handle sequence"
        );
        assert_eq!(num_live, rs.backend().live, "step {step} live segments");
        assert_eq!(
            num_da,
            rs.backend().device_address_allocs,
            "step {step} device-address"
        );
    }

    for (ca, ra) in live.drain(..) {
        c.free(ca);
        rs.free(ra, ());
    }
    assert_eq!(c.stats(), *rs.stats(), "final stats");
    // TestHeapCleanState expects bit 0 set in all eight size-class
    // bitfields; GL_HeapCreate sets them all (gl_heap.c:227) but a coalesced
    // free block only re-sets classes up to log2(size_in_pages) (:159-162),
    // so the C's own check holds only for segments of >= 2^7 pages. The Rust
    // reproduces that exactly; the stats comparison above covers the rest.
    if segment_size / u64::from(page_size) >= 128 {
        rs.check_clean_state().unwrap();
    }
    c.destroy();
    rs.destroy(());
    let (next_handle, num_live, num_da, num_named) = CHeap::globals();
    assert_eq!(num_live, 0);
    assert_eq!(rs.backend().live, 0);
    assert_eq!(next_handle, rs.backend().next_handle);
    // gl_heap.c names every segment and dedicated block it gets a handle for
    assert_eq!(num_named, next_handle as u32);
    (next_handle, num_live, num_da, num_named)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: if cfg!(debug_assertions) { 500 } else { 10_000 },
        max_shrink_iters: 2000,
        ..ProptestConfig::default()
    })]

    #[test]
    fn heap_traces_match_c(
        (page_size, segment_size, device_address) in config_strategy(),
        ops in prop::collection::vec(op_strategy(1u64 << 20), 1..200),
    ) {
        // the dedicated-path sizes were drawn against 1 MiB; scale them to
        // this configuration so the trace still reaches that path
        let ops: Vec<Op> = ops
            .into_iter()
            .map(|op| match op {
                Op::Alloc { size, alignment } if size >= 1 << 20 => Op::Alloc {
                    size: segment_size + (size - (1 << 20)),
                    alignment,
                },
                op => op,
            })
            .collect();
        run_trace(page_size, segment_size, device_address, &ops);
    }
}

/// `GL_HeapTest_f`'s loop shape (1 MiB / 4096, stride-3 alloc/free waves,
/// exponential sizes) with a fixed LCG instead of `COM_Rand`, reduced to a
/// handful of iterations; the in-engine command is the full 10000.
#[test]
fn heap_test_f_shape_matches_c() {
    const TEST_HEAP_SIZE: u64 = 1024 * 1024;
    const TEST_HEAP_PAGE_SIZE: u32 = 4096;
    const NUM_ITERATIONS: usize = if cfg!(debug_assertions) { 2 } else { 8 };
    const NUM_ALLOCS_PER_ITERATION: usize = 500;
    const MAX_ALLOC_SIZE: f64 = 64.0 * 1024.0;
    const MAX_ALIGNMENT_POW2: f64 = 14.0;
    const STRIDE: usize = 3;

    let mut state: u32 = 0x1234_5678;
    let mut rand = move || {
        state = state.wrapping_mul(1_103_515_245).wrapping_add(12345);
        f64::from((state >> 8) & 0x7fff) / 32767.0
    };
    let mut ops = Vec::new();
    for _ in 0..NUM_ITERATIONS {
        // the trace is replayed against a live-list, so express the stride
        // waves as explicit alloc/free ops with stable indices
        let mut sizes = vec![None; NUM_ALLOCS_PER_ITERATION];
        for k in 0..=STRIDE {
            if k < STRIDE {
                for i in (k..NUM_ALLOCS_PER_ITERATION).step_by(STRIDE) {
                    let size = ((MAX_ALLOC_SIZE - 1.0) * rand().powi(5)) as u64 + 1;
                    let alignment = 1u64 << ((rand().powi(10) * MAX_ALIGNMENT_POW2) as u32);
                    sizes[i] = Some((size, alignment));
                    ops.push(Wave::Alloc(i, size, alignment));
                }
            }
            if k > 0 {
                for i in ((k - 1)..NUM_ALLOCS_PER_ITERATION).step_by(STRIDE) {
                    sizes[i] = None;
                    ops.push(Wave::Free(i));
                }
            }
        }
        assert!(sizes.iter().all(Option::is_none));
    }

    let _guard = C_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let c = CHeap::create(TEST_HEAP_SIZE, TEST_HEAP_PAGE_SIZE, false);
    let mut rs: Heap<FakeBackend> = Heap::new(
        FakeBackend::default(),
        TEST_HEAP_SIZE,
        TEST_HEAP_PAGE_SIZE,
        0,
        VulkanMemoryType::None,
        false,
    );
    rs.check_clean_state().unwrap();
    let mut slots: Vec<Option<(*mut c_void, quake_render::heap::Allocation)>> =
        (0..NUM_ALLOCS_PER_ITERATION).map(|_| None).collect();
    for (step, op) in ops.iter().enumerate() {
        match *op {
            Wave::Alloc(i, size, alignment) => {
                assert!(slots[i].is_none());
                let ca = c.allocate(size, alignment);
                let ra = rs.allocate(size, alignment, ()).unwrap();
                assert_eq!(c_alloc_offset(ca), ra.offset(), "step {step} offset");
                assert_eq!(
                    c_alloc_memory(ca),
                    ra.memory().as_raw(),
                    "step {step} memory"
                );
                assert_eq!(ra.offset() % alignment, 0, "step {step} alignment");
                slots[i] = Some((ca, ra));
            }
            Wave::Free(i) => {
                let (ca, ra) = slots[i].take().unwrap();
                c.free(ca);
                rs.free(ra, ());
            }
        }
        assert_eq!(c.stats(), *rs.stats(), "step {step} stats");
        rs.check_consistency().unwrap();
    }
    assert!(slots.iter().all(Option::is_none));
    rs.check_clean_state().unwrap();

    // the trailing dedicated allocation of GL_HeapTest_f
    let ca = c.allocate(TEST_HEAP_SIZE * 2, 1);
    let ra = rs.allocate(TEST_HEAP_SIZE * 2, 1, ()).unwrap();
    assert!(ra.is_dedicated());
    assert_eq!(c_alloc_offset(ca), ra.offset());
    assert_eq!(c_alloc_memory(ca), ra.memory().as_raw());
    assert_eq!(c.stats(), *rs.stats(), "dedicated stats");
    c.free(ca);
    rs.free(ra, ());
    assert_eq!(c.stats(), *rs.stats(), "post-dedicated stats");
    rs.check_clean_state().unwrap();
    c.destroy();
    rs.destroy(());
    let (next_handle, num_live, _, num_named) = CHeap::globals();
    assert_eq!(num_live, 0);
    assert_eq!(next_handle, rs.backend().next_handle);
    assert_eq!(num_named, next_handle as u32);
}

#[derive(Clone, Copy, Debug)]
enum Wave {
    Alloc(usize, u64, u64),
    Free(usize),
}

/// Device-address heaps request the flag for segments only, never for
/// dedicated blocks (`gl_heap.c`); both fakes count the pNext chain.
#[test]
fn device_address_flag_matches_c() {
    let ops = vec![
        Op::Alloc {
            size: 100,
            alignment: 16,
        },
        Op::Alloc {
            size: 3 << 20,
            alignment: 1,
        },
        Op::Alloc {
            size: 70_000,
            alignment: 4096,
        },
        Op::Free { index: 0 },
        Op::Alloc {
            size: 1 << 20,
            alignment: 1,
        },
    ];
    let (_, _, num_da, _) = run_trace(4096, 1 << 20, true, &ops);
    assert_eq!(num_da, 1, "one segment, two dedicated blocks");
}
