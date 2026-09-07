//! `gl_heap.c` -- the device-memory suballocator (Rust migration Phase 8 M3,
//! ADR-015). Segments of `segment_size` bytes are carved into pages; a block
//! allocation takes a run of pages found through per-size-class free-block
//! bitfields (plus a skip level over 64-word groups), a small allocation
//! takes one power-of-two slot of a page shared through per-bucket free
//! lists, and anything at least `segment_size` bytes gets a dedicated
//! device allocation. Every policy -- first-fit search order, alignment
//! padding left as its own free block, right-hand splits, coalescing with
//! both neighbours on free, the six small buckets from `page_size / 64` up
//! to `page_size / 2` -- is the C one, and the differential test in
//! `quake-ctest/tests/gl_heap_differential.rs` holds offsets, segment
//! indices and `glheapstats_t` bit-identical to it on random traces.
//!
//! The device memory itself comes from a [`DeviceMemoryBackend`]: the C ABI
//! shim in `quake-capi` routes it to the engine's `R_AllocateVulkanMemory`
//! / `R_FreeVulkanMemory` / `GL_SetObjectName`, the tests supply a fake, so
//! this crate makes no Vulkan call.

use quake_types::render::{GlHeapStats, VulkanMemory, VulkanMemoryType};

/// `NUM_SMALL_ALLOC_SIZES`: buckets of 64, 32, 16, 8, 4 and 2 slots per page.
pub const NUM_SMALL_ALLOC_SIZES: usize = 6;
/// `NUM_BLOCK_SIZE_CLASSES`: free-block bitfields for blocks of at least
/// `1 << class` pages.
pub const NUM_BLOCK_SIZE_CLASSES: usize = 8;
/// `MAX_PAGES`: page indices are `uint16_t` with `UINT16_MAX` reserved.
pub const MAX_PAGES: u32 = u16::MAX as u32 - 1;
const INVALID_PAGE_INDEX: u16 = u16::MAX;
const SMALL_SLOTS_PER_PAGE: [u32; NUM_SMALL_ALLOC_SIZES] = [64, 32, 16, 8, 4, 2];
const SLOTS_FULL_MASK: [u64; NUM_SMALL_ALLOC_SIZES] =
    [u64::MAX, 0xFFFF_FFFF, 0xFFFF, 0xFF, 0xF, 0x3];

/// Where segment and dedicated memory comes from. `Counter` is the
/// `atomic_uint32_t *num_allocations` the C API threads through every call;
/// the heap never reads it, so it is whatever the backend needs.
pub trait DeviceMemoryBackend {
    type Counter: Copy;
    /// `R_AllocateVulkanMemory` (+ `GL_SetObjectName` when the handle is
    /// non-null): fill `memory` with `size` bytes of `memory_type_index`.
    /// `device_address` asks for `VK_MEMORY_ALLOCATE_DEVICE_ADDRESS_BIT`,
    /// which the C only requests for segments, never for dedicated blocks.
    fn allocate(
        &mut self,
        memory: &mut VulkanMemory,
        size: u64,
        memory_type_index: u32,
        memory_type: VulkanMemoryType,
        device_address: bool,
        counter: Self::Counter,
    );
    /// `R_FreeVulkanMemory`.
    fn free(&mut self, memory: &mut VulkanMemory, counter: Self::Counter);
}

/// `Q_log2` (`mathlib.h`): `FindLastBitNonZero`, undefined for 0 in C. Every
/// caller here passes a non-zero value (`q_next_pow2` guards `val > 1`, the
/// size-class lookups take a page count of at least 1); 0 would give
/// `u32::MAX`, so it is a debug assertion rather than a defined result.
fn q_log2(val: u32) -> u32 {
    debug_assert!(val > 0, "Q_log2 (0) is undefined");
    31u32.wrapping_sub(val.leading_zeros())
}

/// `Q_nextPow2` (`mathlib.h`).
fn q_next_pow2(val: u32) -> u32 {
    if val > 1 {
        1u32.wrapping_shl(q_log2(val - 1) + 1)
    } else {
        1
    }
}

// COMPAT: `q_align` on two `uint16_t` operands selects the `int` overload
// (integer promotion), so the arithmetic is 32-bit and the result is
// truncated back to `uint16_t` at the assignment (gl_heap.c
// `GL_HeapAllocateBlockFromSegment`). An alignment of 0 pages (only reachable
// when the byte alignment truncates to 0 pages) therefore masks with -1.
fn q_align_u16(size: u16, alignment: u16) -> u16 {
    let size = i32::from(size);
    let alignment = i32::from(alignment);
    let rem = size & alignment.wrapping_sub(1);
    if rem == 0 {
        size as u16
    } else {
        size.wrapping_add(alignment).wrapping_sub(rem) as u16
    }
}

/// `glheappagehdr_t`; all-zero is `EMPTY_PAGE_HDR`.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct PageHdr {
    size_in_pages: u16,
    prev_block_page_index: u16,
}

/// `glheapsmallalloclinks_t`; both `INVALID_PAGE_INDEX` is
/// `EMPTY_SMALL_ALLOC_LINKS`.
#[derive(Clone, Copy, PartialEq, Eq)]
struct SmallAllocLinks {
    prev_small_alloc_page: u16,
    next_small_alloc_page: u16,
}

const EMPTY_SMALL_ALLOC_LINKS: SmallAllocLinks = SmallAllocLinks {
    prev_small_alloc_page: INVALID_PAGE_INDEX,
    next_small_alloc_page: INVALID_PAGE_INDEX,
};

/// `glheapsegment_t`.
struct Segment {
    memory: VulkanMemory,
    page_hdrs: Vec<PageHdr>,
    small_alloc_links: Vec<SmallAllocLinks>,
    small_alloc_masks: Vec<u64>,
    free_blocks_bitfields: [Vec<u64>; NUM_BLOCK_SIZE_CLASSES],
    free_blocks_skip_bitfields: [Vec<u64>; NUM_BLOCK_SIZE_CLASSES],
    small_alloc_free_list_heads: [u16; NUM_SMALL_ALLOC_SIZES],
    num_pages_allocated: u16,
}

impl Segment {
    /// `GL_CreateHeapSegment` minus the device allocation.
    fn new(memory: VulkanMemory, num_pages: u16) -> Self {
        let pages = usize::from(num_pages);
        let mut page_hdrs = vec![PageHdr::default(); pages];
        page_hdrs[0].size_in_pages = num_pages;
        let bitfield = || {
            let mut v = vec![0u64; pages.div_ceil(64)];
            v[0] = 0x1;
            v
        };
        let skip = || {
            let mut v = vec![0u64; pages.div_ceil(4096)];
            v[0] = 0x1;
            v
        };
        Segment {
            memory,
            page_hdrs,
            small_alloc_links: vec![EMPTY_SMALL_ALLOC_LINKS; pages],
            small_alloc_masks: vec![0; pages],
            free_blocks_bitfields: core::array::from_fn(|_| bitfield()),
            free_blocks_skip_bitfields: core::array::from_fn(|_| skip()),
            small_alloc_free_list_heads: [INVALID_PAGE_INDEX; NUM_SMALL_ALLOC_SIZES],
            num_pages_allocated: 0,
        }
    }

    fn get_bit(bitfield: &[u64], index: u32) -> bool {
        (bitfield[(index / 64) as usize] & (1u64 << (index % 64))) != 0
    }

    fn set_bit(bitfield: &mut [u64], index: u32) {
        bitfield[(index / 64) as usize] |= 1u64 << (index % 64);
    }

    fn clear_bit(bitfield: &mut [u64], index: u32) {
        bitfield[(index / 64) as usize] &= !(1u64 << (index % 64));
    }

    /// `GL_HeapMarkBlockFree`
    fn mark_block_free(&mut self, size_in_pages: u16, block_page_index: u32) {
        let size_class = q_log2(u32::from(size_in_pages)).min(NUM_BLOCK_SIZE_CLASSES as u32 - 1);
        for i in 0..=size_class as usize {
            Self::set_bit(&mut self.free_blocks_bitfields[i], block_page_index);
            Self::set_bit(
                &mut self.free_blocks_skip_bitfields[i],
                block_page_index / 64,
            );
        }
    }

    /// `GL_HeapMarkBlockUsed`
    fn mark_block_used(&mut self, block_page_index: u32) {
        for i in 0..NUM_BLOCK_SIZE_CLASSES {
            Self::clear_bit(&mut self.free_blocks_bitfields[i], block_page_index);
            if self.free_blocks_bitfields[i][(block_page_index / 64) as usize] == 0 {
                Self::clear_bit(
                    &mut self.free_blocks_skip_bitfields[i],
                    block_page_index / 64,
                );
            }
        }
    }

    /// `GL_HeapIsBlockFree`
    fn is_block_free(&self, block_page_index: u32) -> bool {
        Self::get_bit(&self.free_blocks_bitfields[0], block_page_index)
    }

    /// `GL_HeapAddPageToSmallFreeList`
    fn add_page_to_small_free_list(&mut self, page_index: u16, bucket: usize) {
        let prev_head_index = self.small_alloc_free_list_heads[bucket];
        if prev_head_index != INVALID_PAGE_INDEX {
            self.small_alloc_links[usize::from(prev_head_index)].prev_small_alloc_page = page_index;
            self.small_alloc_links[usize::from(page_index)].next_small_alloc_page = prev_head_index;
        }
        self.small_alloc_free_list_heads[bucket] = page_index;
    }

    /// `GL_HeapRemovePageFromSmallFreeList`
    fn remove_page_from_small_free_list(&mut self, page_index: u16, bucket: usize) {
        let links = self.small_alloc_links[usize::from(page_index)];
        if links.prev_small_alloc_page != INVALID_PAGE_INDEX {
            self.small_alloc_links[usize::from(links.prev_small_alloc_page)]
                .next_small_alloc_page = links.next_small_alloc_page;
        }
        if links.next_small_alloc_page != INVALID_PAGE_INDEX {
            self.small_alloc_links[usize::from(links.next_small_alloc_page)]
                .prev_small_alloc_page = links.prev_small_alloc_page;
        }
        self.small_alloc_links[usize::from(page_index)] = EMPTY_SMALL_ALLOC_LINKS;
        if self.small_alloc_free_list_heads[bucket] == page_index {
            self.small_alloc_free_list_heads[bucket] = links.next_small_alloc_page;
        }
    }
}

/// `allocinfo_t`
#[derive(Clone, Copy)]
struct AllocInfo {
    is_small_alloc: bool,
    small_alloc_size: u32,
    small_alloc_bucket: usize,
    alloc_size_in_pages: u16,
    alignment_in_pages: u16,
    size_class: usize,
}

/// `ONE_PAGE_ALLOC_INFO`
const ONE_PAGE_ALLOC_INFO: AllocInfo = AllocInfo {
    is_small_alloc: false,
    small_alloc_size: 0,
    small_alloc_bucket: 0,
    alloc_size_in_pages: 1,
    alignment_in_pages: 1,
    size_class: 0,
};

/// `alloc_type_t`; the C encodes the bucket as `ALLOC_TYPE_SMALL_ALLOC + bucket`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AllocKind {
    Pages,
    Dedicated,
    Small(usize),
}

/// `glheapallocation_t`. For page and small allocations `memory` is a copy
/// of the owning segment's (segments are never released before
/// [`Heap::destroy`], and their handle never changes), so the memory query
/// needs no heap; a dedicated allocation owns its memory.
#[derive(Debug)]
pub struct Allocation {
    size: u64,
    offset: u64,
    kind: AllocKind,
    segment: u32,
    memory: VulkanMemory,
}

impl Allocation {
    /// `GL_HeapGetAllocationMemory`
    pub fn memory(&self) -> ash::vk::DeviceMemory {
        self.memory.handle
    }

    /// `GL_HeapGetAllocationOffset`
    pub fn offset(&self) -> u64 {
        self.offset
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn is_dedicated(&self) -> bool {
        self.kind == AllocKind::Dedicated
    }

    /// Index of the owning segment in creation order; `None` when dedicated.
    pub fn segment_index(&self) -> Option<u32> {
        (!self.is_dedicated()).then_some(self.segment)
    }
}

/// The immutable half of `glheap_t`.
#[derive(Clone, Copy)]
struct Config {
    segment_size: u64,
    page_size: u32,
    page_size_shift: u32,
    min_small_alloc_size: u32,
    small_alloc_shift: u32,
    memory_type_index: u32,
    memory_type: VulkanMemoryType,
    device_address: bool,
    num_pages_per_segment: u16,
    num_masks_per_segment: u16,
}

/// `glheap_t`
pub struct Heap<B: DeviceMemoryBackend> {
    backend: B,
    cfg: Config,
    segments: Vec<Segment>,
    dedicated_alloc_bytes: u64,
    stats: GlHeapStats,
}

/// `GL_HeapAllocateBlockFromSegment`: the first-fit page-run search.
fn allocate_block(
    seg: &mut Segment,
    stats: &mut GlHeapStats,
    cfg: &Config,
    info: &AllocInfo,
) -> Option<u16> {
    let num_pages = u32::from(cfg.num_pages_per_segment);
    if u32::from(info.alloc_size_in_pages) > num_pages - u32::from(seg.num_pages_allocated) {
        return None;
    }

    let mut mask_offset: u16 = 0;
    while mask_offset < cfg.num_masks_per_segment {
        let mut skip_mask =
            seg.free_blocks_skip_bitfields[info.size_class][usize::from(mask_offset / 64)];
        while skip_mask != 0 {
            let mask_index = skip_mask.trailing_zeros();
            skip_mask &= !(1u64 << mask_index);

            let mut mask = seg.free_blocks_bitfields[info.size_class]
                [usize::from(mask_offset) + mask_index as usize];
            while mask != 0 {
                let mask_page_index = mask.trailing_zeros();
                mask &= !(1u64 << mask_page_index);

                let block_page_index =
                    (((u32::from(mask_offset) + mask_index) * 64) + mask_page_index) as u16;
                let block_page_index_aligned =
                    q_align_u16(block_page_index, info.alignment_in_pages);

                // COMPAT: both `uint32_t` in C; a 0-page alignment makes the
                // aligned index 0 and the difference wrap, which then fails
                // the fit test below for every block but the first.
                let alignment_pages =
                    u32::from(block_page_index_aligned).wrapping_sub(u32::from(block_page_index));
                let total_pages = alignment_pages.wrapping_add(u32::from(info.alloc_size_in_pages));

                let block_size_in_pages =
                    u32::from(seg.page_hdrs[usize::from(block_page_index)].size_in_pages);

                if total_pages <= block_size_in_pages {
                    if alignment_pages == 0 {
                        stats.num_blocks_free = stats.num_blocks_free.wrapping_sub(1);
                        seg.mark_block_used(u32::from(block_page_index));
                    } else {
                        seg.page_hdrs[usize::from(block_page_index)].size_in_pages =
                            alignment_pages as u16;
                        seg.page_hdrs[usize::from(block_page_index_aligned)]
                            .prev_block_page_index = block_page_index;
                    }

                    let next_block_page_index =
                        block_page_index_aligned.wrapping_add(info.alloc_size_in_pages);
                    if total_pages < block_size_in_pages {
                        let next_size = (block_size_in_pages - total_pages) as u16;
                        let next = usize::from(next_block_page_index);
                        seg.page_hdrs[next].size_in_pages = next_size;
                        stats.num_blocks_free = stats.num_blocks_free.wrapping_add(1);
                        seg.mark_block_free(next_size, u32::from(next_block_page_index));
                        seg.page_hdrs[next].prev_block_page_index = block_page_index_aligned;

                        let nn_block_page_index =
                            u32::from(next_block_page_index) + u32::from(next_size);
                        if nn_block_page_index < num_pages {
                            seg.page_hdrs[nn_block_page_index as usize].prev_block_page_index =
                                next_block_page_index;
                        }
                    } else if u32::from(next_block_page_index) < num_pages {
                        seg.page_hdrs[usize::from(next_block_page_index)].prev_block_page_index =
                            block_page_index_aligned;
                    }

                    seg.page_hdrs[usize::from(block_page_index_aligned)].size_in_pages =
                        info.alloc_size_in_pages;
                    seg.num_pages_allocated = seg
                        .num_pages_allocated
                        .wrapping_add(info.alloc_size_in_pages);
                    stats.num_blocks_used = stats.num_blocks_used.wrapping_add(1);
                    return Some(block_page_index_aligned);
                }
            }
        }
        mask_offset += 64;
    }
    None
}

/// `GL_HeapSmallAllocateFromBlock`
fn small_allocate_from_block(
    allocation: &mut Allocation,
    seg: &mut Segment,
    cfg: &Config,
    block_page_index: u16,
    info: &AllocInfo,
) {
    let bucket = info.small_alloc_bucket;
    if seg.small_alloc_masks[usize::from(block_page_index)] == 0 {
        seg.add_page_to_small_free_list(block_page_index, bucket);
    }
    let mask = &mut seg.small_alloc_masks[usize::from(block_page_index)];
    let slot_index = (!*mask).trailing_zeros();
    *mask |= 1u64 << slot_index;
    if *mask == SLOTS_FULL_MASK[bucket] {
        seg.remove_page_from_small_free_list(block_page_index, bucket);
    }
    allocation.kind = AllocKind::Small(bucket);
    // COMPAT: `uint32_t` arithmetic in C, widened at the assignment
    allocation.offset = u64::from(
        u32::from(block_page_index)
            .wrapping_mul(cfg.page_size)
            .wrapping_add(slot_index.wrapping_mul(info.small_alloc_size)),
    );
}

/// `GL_HeapSmallFreeFromBlock`: true when the page became empty.
fn small_free_from_block(
    seg: &mut Segment,
    cfg: &Config,
    allocation: &Allocation,
    bucket: usize,
) -> bool {
    let block_page_index = (allocation.offset >> cfg.page_size_shift) as u32;
    let offset_in_page = (allocation.offset & u64::from(cfg.page_size - 1)) as u32;
    let slot_index = offset_in_page >> (cfg.small_alloc_shift + bucket as u32);
    let page = block_page_index as u16;

    if seg.small_alloc_masks[block_page_index as usize] == SLOTS_FULL_MASK[bucket] {
        seg.add_page_to_small_free_list(page, bucket);
    }
    let mask = &mut seg.small_alloc_masks[block_page_index as usize];
    *mask &= !(1u64 << slot_index);
    let page_empty = *mask == 0;
    if page_empty {
        seg.remove_page_from_small_free_list(page, bucket);
    }
    page_empty
}

/// `GL_HeapAllocateFromSegment`
fn allocate_from_segment(
    allocation: &mut Allocation,
    seg: &mut Segment,
    stats: &mut GlHeapStats,
    cfg: &Config,
    info: &AllocInfo,
) -> bool {
    if info.is_small_alloc {
        let head = seg.small_alloc_free_list_heads[info.small_alloc_bucket];
        let page_index = if head != INVALID_PAGE_INDEX {
            head
        } else {
            match allocate_block(seg, stats, cfg, &ONE_PAGE_ALLOC_INFO) {
                Some(page) => page,
                None => return false,
            }
        };
        small_allocate_from_block(allocation, seg, cfg, page_index, info);
        true
    } else if let Some(page_index) = allocate_block(seg, stats, cfg, info) {
        // COMPAT: `page_index * heap->page_size` is `uint32_t` in C
        allocation.offset = u64::from(u32::from(page_index).wrapping_mul(cfg.page_size));
        allocation.kind = AllocKind::Pages;
        true
    } else {
        false
    }
}

/// `GL_HeapFreeBlockFromSegment`
fn free_block(seg: &mut Segment, stats: &mut GlHeapStats, cfg: &Config, offset: u64) {
    let num_pages = u32::from(cfg.num_pages_per_segment);
    let mut block_page_index = (offset >> cfg.page_size_shift) as u16 as u32;
    let size = seg.page_hdrs[block_page_index as usize].size_in_pages;
    seg.num_pages_allocated = seg.num_pages_allocated.wrapping_sub(size);
    stats.num_blocks_used = stats.num_blocks_used.wrapping_sub(1);

    if block_page_index > 0 {
        let prev = u32::from(seg.page_hdrs[block_page_index as usize].prev_block_page_index);
        if seg.is_block_free(prev) {
            let size = seg.page_hdrs[block_page_index as usize].size_in_pages;
            seg.page_hdrs[prev as usize].size_in_pages = seg.page_hdrs[prev as usize]
                .size_in_pages
                .wrapping_add(size);
            seg.page_hdrs[block_page_index as usize] = PageHdr::default();
            block_page_index = prev;
        }
    }

    {
        let next = (block_page_index as u16)
            .wrapping_add(seg.page_hdrs[block_page_index as usize].size_in_pages);
        if u32::from(next) < num_pages && seg.is_block_free(u32::from(next)) {
            stats.num_blocks_free = stats.num_blocks_free.wrapping_sub(1);
            seg.mark_block_used(u32::from(next));
            let next_size = seg.page_hdrs[usize::from(next)].size_in_pages;
            seg.page_hdrs[block_page_index as usize].size_in_pages = seg.page_hdrs
                [block_page_index as usize]
                .size_in_pages
                .wrapping_add(next_size);
            seg.page_hdrs[usize::from(next)] = PageHdr::default();
        }
    }

    {
        let next = (block_page_index as u16)
            .wrapping_add(seg.page_hdrs[block_page_index as usize].size_in_pages);
        if u32::from(next) < num_pages {
            seg.page_hdrs[usize::from(next)].prev_block_page_index = block_page_index as u16;
        }
    }

    if !seg.is_block_free(block_page_index) {
        stats.num_blocks_free = stats.num_blocks_free.wrapping_add(1);
    }
    let size = seg.page_hdrs[block_page_index as usize].size_in_pages;
    seg.mark_block_free(size, block_page_index);
}

impl<B: DeviceMemoryBackend> Heap<B> {
    /// `GL_HeapCreate`. The C `assert`s (power-of-two page size of at least
    /// 128 bytes, a segment of one to `MAX_PAGES` whole pages) are hard
    /// checks here: the page tables are sized from them.
    pub fn new(
        backend: B,
        segment_size: u64,
        page_size: u32,
        memory_type_index: u32,
        memory_type: VulkanMemoryType,
        device_address: bool,
    ) -> Self {
        assert!(
            q_next_pow2(page_size) == page_size,
            "page_size must be a power of two"
        );
        assert!(
            page_size >= 1 << (NUM_SMALL_ALLOC_SIZES + 1),
            "page_size too small"
        );
        assert!(
            segment_size >= u64::from(page_size),
            "segment smaller than a page"
        );
        assert!(
            segment_size.is_multiple_of(u64::from(page_size)),
            "segment not a whole number of pages"
        );
        assert!(
            segment_size / u64::from(page_size) <= u64::from(MAX_PAGES),
            "too many pages"
        );
        let num_pages_per_segment = (segment_size / u64::from(page_size)) as u16;
        Heap {
            backend,
            cfg: Config {
                segment_size,
                page_size,
                page_size_shift: q_log2(page_size),
                min_small_alloc_size: page_size / 64,
                small_alloc_shift: q_log2(page_size / (1 << NUM_SMALL_ALLOC_SIZES)),
                memory_type_index,
                memory_type,
                device_address,
                num_pages_per_segment,
                num_masks_per_segment: num_pages_per_segment.div_ceil(64),
            },
            segments: Vec::new(),
            dedicated_alloc_bytes: 0,
            stats: GlHeapStats::default(),
        }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    pub fn num_segments(&self) -> u32 {
        self.segments.len() as u32
    }

    /// `GL_HeapDestroy`: releases every segment's device memory. The C
    /// leaves the heap struct itself allocated; whether this value lives on
    /// is the caller's business.
    ///
    /// Divergence, documented (plan amendment, M3 review): the C frees the
    /// segment structs but leaves `heap->segments` and `num_segments` as
    /// they were, so a destroyed C heap dangles; this drops the segments,
    /// so a destroyed `Heap` is an empty one. No caller touches a heap
    /// after `GL_HeapDestroy` (its only caller is the `_DEBUG` self-test).
    pub fn destroy(&mut self, counter: B::Counter) {
        for seg in &mut self.segments {
            self.backend.free(&mut seg.memory, counter);
        }
        self.segments.clear();
    }

    /// `GL_HeapAllocate`. `None` is the C `Sys_Error ("GL_HeapAllocate
    /// failed to allocate")`: a fresh segment could not hold the request,
    /// which cannot happen for `size < segment_size` with a sane alignment.
    pub fn allocate(
        &mut self,
        size: u64,
        alignment: u64,
        counter: B::Counter,
    ) -> Option<Allocation> {
        let cfg = self.cfg;
        let mut allocation = Allocation {
            size,
            offset: 0,
            kind: AllocKind::Pages,
            segment: 0,
            memory: VulkanMemory::default(),
        };
        self.stats.num_allocations = self.stats.num_allocations.wrapping_add(1);
        self.stats.num_bytes_allocated = self.stats.num_bytes_allocated.wrapping_add(size);

        if size < cfg.segment_size {
            let size_alignment_max = size.max(alignment);
            let is_small_alloc = size_alignment_max <= u64::from(cfg.page_size / 2);
            let info = if is_small_alloc {
                self.stats.num_small_allocations = self.stats.num_small_allocations.wrapping_add(1);
                let small_alloc_size =
                    q_next_pow2(size_alignment_max as u32).max(cfg.min_small_alloc_size);
                AllocInfo {
                    is_small_alloc: true,
                    small_alloc_size,
                    small_alloc_bucket: q_log2(small_alloc_size >> cfg.small_alloc_shift) as usize,
                    alloc_size_in_pages: 0,
                    alignment_in_pages: 0,
                    size_class: 0,
                }
            } else {
                self.stats.num_block_allocations = self.stats.num_block_allocations.wrapping_add(1);
                // COMPAT: both page counts are `page_index_t` (uint16_t) in C
                let alloc_size_in_pages =
                    ((size + u64::from(cfg.page_size) - 1) >> cfg.page_size_shift) as u16;
                let alignment_in_pages =
                    ((alignment + u64::from(cfg.page_size) - 1) >> cfg.page_size_shift) as u16;
                AllocInfo {
                    is_small_alloc: false,
                    small_alloc_size: 0,
                    small_alloc_bucket: 0,
                    alloc_size_in_pages,
                    alignment_in_pages,
                    size_class: q_log2(u32::from(alloc_size_in_pages))
                        .min(NUM_BLOCK_SIZE_CLASSES as u32 - 1)
                        as usize,
                }
            };

            let num_segments = self.segments.len();
            for i in 0..=num_segments {
                if i == num_segments {
                    let mut memory = VulkanMemory::default();
                    self.backend.allocate(
                        &mut memory,
                        cfg.segment_size,
                        cfg.memory_type_index,
                        cfg.memory_type,
                        cfg.device_address,
                        counter,
                    );
                    self.segments
                        .push(Segment::new(memory, cfg.num_pages_per_segment));
                    self.stats.num_blocks_free = self.stats.num_blocks_free.wrapping_add(1);
                }
                let seg = &mut self.segments[i];
                if allocate_from_segment(&mut allocation, seg, &mut self.stats, &cfg, &info) {
                    allocation.segment = i as u32;
                    allocation.memory = seg.memory;
                    return Some(allocation);
                }
            }
            None
        } else {
            self.stats.num_dedicated_allocations =
                self.stats.num_dedicated_allocations.wrapping_add(1);
            self.dedicated_alloc_bytes = self.dedicated_alloc_bytes.wrapping_add(size);
            allocation.kind = AllocKind::Dedicated;
            self.backend.allocate(
                &mut allocation.memory,
                size,
                cfg.memory_type_index,
                cfg.memory_type,
                false,
                counter,
            );
            Some(allocation)
        }
    }

    /// `GL_HeapFree`
    pub fn free(&mut self, mut allocation: Allocation, counter: B::Counter) {
        let cfg = self.cfg;
        self.stats.num_allocations = self.stats.num_allocations.wrapping_sub(1);
        self.stats.num_bytes_allocated =
            self.stats.num_bytes_allocated.wrapping_sub(allocation.size);
        match allocation.kind {
            AllocKind::Pages => {
                self.stats.num_block_allocations = self.stats.num_block_allocations.wrapping_sub(1);
                let seg = &mut self.segments[allocation.segment as usize];
                free_block(seg, &mut self.stats, &cfg, allocation.offset);
            }
            AllocKind::Dedicated => {
                self.stats.num_dedicated_allocations =
                    self.stats.num_dedicated_allocations.wrapping_sub(1);
                self.dedicated_alloc_bytes =
                    self.dedicated_alloc_bytes.wrapping_sub(allocation.size);
                self.backend.free(&mut allocation.memory, counter);
            }
            AllocKind::Small(bucket) => {
                self.stats.num_small_allocations = self.stats.num_small_allocations.wrapping_sub(1);
                let seg = &mut self.segments[allocation.segment as usize];
                if small_free_from_block(seg, &cfg, &allocation, bucket) {
                    free_block(seg, &mut self.stats, &cfg, allocation.offset);
                }
            }
        }
    }

    /// `GL_HeapGetStats` for the C ABI: recomputes the totals like
    /// [`Heap::stats`] and returns a raw pointer to the heap's own stats
    /// block (the C hands out `&heap->stats`). The pointer is derived from
    /// `this` rather than from a `&mut Heap`, so it keeps the provenance of
    /// the heap pointer the caller holds and stays valid until the next
    /// call that touches the stats.
    ///
    /// # Safety
    /// `this` points to a live `Heap` and no reference into it is alive.
    pub unsafe fn stats_ptr(this: *mut Self) -> *mut GlHeapStats {
        // SAFETY: per the contract above; the reference `stats` creates
        // ends before the raw pointer is formed.
        unsafe {
            (*this).stats();
            core::ptr::addr_of_mut!((*this).stats)
        }
    }

    /// `GL_HeapGetStats`: recomputes the page/byte totals and returns the
    /// heap's own stats block (the C hands out a pointer into `glheap_t`).
    pub fn stats(&mut self) -> &GlHeapStats {
        let cfg = self.cfg;
        self.stats.num_pages_allocated = 0;
        let mut num_total_pages: u32 = 0;
        let mut total_allocated_page_bytes: u64 = 0;
        let mut small_alloc_pages_bytes: u32 = 0;
        let mut small_alloc_bytes: u64 = 0;
        for seg in &self.segments {
            num_total_pages = num_total_pages.wrapping_add(u32::from(cfg.num_pages_per_segment));
            self.stats.num_pages_allocated = self
                .stats
                .num_pages_allocated
                .wrapping_add(u32::from(seg.num_pages_allocated));
            // COMPAT: `uint16_t * uint32_t` is a `uint32_t` product in C,
            // widened only at the `uint64_t` accumulation
            total_allocated_page_bytes = total_allocated_page_bytes.wrapping_add(u64::from(
                u32::from(seg.num_pages_allocated).wrapping_mul(cfg.page_size),
            ));
            for (i, &slots_per_page) in SMALL_SLOTS_PER_PAGE.iter().enumerate() {
                let slot_size = cfg.page_size / slots_per_page;
                let mut page = seg.small_alloc_free_list_heads[i];
                while page != INVALID_PAGE_INDEX {
                    small_alloc_pages_bytes = small_alloc_pages_bytes.wrapping_add(cfg.page_size);
                    let mask = seg.small_alloc_masks[usize::from(page)];
                    for slot in 0..slots_per_page {
                        if mask & (1u64 << slot) != 0 {
                            small_alloc_bytes =
                                small_alloc_bytes.wrapping_add(u64::from(slot_size));
                        }
                    }
                    page = seg.small_alloc_links[usize::from(page)].next_small_alloc_page;
                }
            }
        }
        self.stats.num_segments = self.segments.len() as u32;
        self.stats.num_pages_free = num_total_pages.wrapping_sub(self.stats.num_pages_allocated);
        // COMPAT: `uint32_t * uint32_t` product, widened at the assignment
        self.stats.num_bytes_free =
            u64::from(self.stats.num_pages_free.wrapping_mul(cfg.page_size));
        self.stats.num_bytes_wasted = total_allocated_page_bytes
            .wrapping_sub(u64::from(small_alloc_pages_bytes))
            .wrapping_add(self.dedicated_alloc_bytes)
            .wrapping_add(small_alloc_bytes)
            .wrapping_sub(self.stats.num_bytes_allocated);
        &self.stats
    }

    /// `TestHeapConsistency` (gl_heap.c, `_DEBUG`): walks every segment's
    /// block chain and cross-checks it with the free bitfields, the page
    /// counters and the stats. The `Zero-sized block` check is an addition:
    /// the C walk would loop forever on a zero-sized header, this reports
    /// it instead. The alloc-counter sum wraps like the C `uint32_t` sum.
    pub fn check_consistency(&mut self) -> Result<(), &'static str> {
        let num_pages = u32::from(self.cfg.num_pages_per_segment);
        for seg in &self.segments {
            let mut current: u32 = 0;
            let mut prev_block: u16 = 0;
            let mut prev_block_free = false;
            let mut num_allocated_pages: u16 = 0;
            while current < num_pages {
                let block = seg.page_hdrs[current as usize];
                let block_free = seg.is_block_free(current);
                if current > 0 {
                    if block.prev_block_page_index != prev_block {
                        return Err("Invalid prev block");
                    }
                    if prev_block_free && block_free {
                        return Err("Found two consecutive free blocks");
                    }
                }
                if block.size_in_pages == 0 {
                    return Err("Zero-sized block");
                }
                prev_block = current as u16;
                for j in 1..u32::from(block.size_in_pages) {
                    for bitfield in &seg.free_blocks_bitfields {
                        if Segment::get_bit(bitfield, current + j) {
                            return Err("Free bit set for non block page");
                        }
                    }
                }
                if !block_free {
                    num_allocated_pages = num_allocated_pages.wrapping_add(block.size_in_pages);
                }
                prev_block_free = block_free;
                current += u32::from(block.size_in_pages);
            }
            if current != num_pages {
                return Err("Blocks need to add up to num pages");
            }
            if num_allocated_pages != seg.num_pages_allocated {
                return Err("Invalid number of allocated pages found");
            }
        }
        let stats = self.stats();
        if stats.num_allocations
            != stats
                .num_small_allocations
                .wrapping_add(stats.num_block_allocations)
                .wrapping_add(stats.num_dedicated_allocations)
        {
            return Err("Invalid alloc counter");
        }
        Ok(())
    }

    /// `TestHeapCleanState` (gl_heap.c, `_DEBUG`): every segment is one
    /// free block again and every counter is back to zero. The C's first
    /// block check is `=` where `==` is meant (gl_heap.c:841): it assigns
    /// `num_pages_per_segment` into the first header (repairing a wrong
    /// value rather than reporting it) and passes whenever that is
    /// non-zero. This compares and reports, so it is stricter than the C
    /// (and never mutates the heap).
    pub fn check_clean_state(&mut self) -> Result<(), &'static str> {
        let num_pages = self.cfg.num_pages_per_segment;
        for seg in &self.segments {
            if seg.num_pages_allocated != 0 {
                return Err("num_pages_allocated needs to be 0");
            }
            if seg.page_hdrs[0].size_in_pages != num_pages {
                return Err("Empty heap first block needs to fill all pages");
            }
            for j in 0..NUM_BLOCK_SIZE_CLASSES {
                if seg.free_blocks_bitfields[j][0] != 1 {
                    return Err("first bitfield bit needs to be 1");
                }
                if seg.free_blocks_skip_bitfields[j][0] != 1 {
                    return Err("first skip bitfield bit needs to be 1");
                }
            }
            for j in 1..usize::from(num_pages) {
                if seg.page_hdrs[j] != PageHdr::default() {
                    return Err("Page block header needs to be empty");
                }
                if seg.small_alloc_links[j] != EMPTY_SMALL_ALLOC_LINKS {
                    return Err("Page small alloc links need to be empty");
                }
                if seg.small_alloc_masks[j] != 0 {
                    return Err("Page small alloc masks needs to be empty");
                }
            }
            for j in 1..usize::from(num_pages).div_ceil(64) {
                for bitfield in &seg.free_blocks_bitfields {
                    if bitfield[j] != 0 {
                        return Err("bitfield is not 0");
                    }
                }
            }
            for j in 1..usize::from(num_pages).div_ceil(4096) {
                for bitfield in &seg.free_blocks_skip_bitfields {
                    if bitfield[j] != 0 {
                        return Err("skip bitfield is not 0");
                    }
                }
            }
            if seg.small_alloc_free_list_heads != [INVALID_PAGE_INDEX; NUM_SMALL_ALLOC_SIZES] {
                return Err("free list head is not empty");
            }
        }
        let num_segments = self.segments.len() as u32;
        if num_segments != self.stats.num_blocks_free {
            return Err("Invalid number of free blocks");
        }
        let stats = self.stats();
        if stats.num_allocations != 0 {
            return Err("Invalid num_allocations counter");
        }
        if stats.num_small_allocations != 0 {
            return Err("Invalid num_small_allocations counter");
        }
        if stats.num_block_allocations != 0 {
            return Err("Invalid num_block_allocations counter");
        }
        if stats.num_dedicated_allocations != 0 {
            return Err("Invalid num_dedicated_allocations counter");
        }
        if stats.num_blocks_free != num_segments {
            return Err("Invalid num_blocks_free counter");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ash::vk::Handle;

    /// Deterministic stand-in: handle = allocation ordinal, no Vulkan.
    #[derive(Default)]
    struct FakeBackend {
        next_handle: u64,
        live: u32,
    }

    impl DeviceMemoryBackend for FakeBackend {
        type Counter = ();
        fn allocate(
            &mut self,
            memory: &mut VulkanMemory,
            size: u64,
            _memory_type_index: u32,
            memory_type: VulkanMemoryType,
            _device_address: bool,
            (): (),
        ) {
            self.next_handle += 1;
            self.live += 1;
            memory.handle = ash::vk::DeviceMemory::from_raw(self.next_handle);
            memory.size = size as usize;
            memory.type_ = memory_type;
        }
        fn free(&mut self, memory: &mut VulkanMemory, (): ()) {
            self.live -= 1;
            *memory = VulkanMemory::default();
        }
    }

    fn heap(segment_size: u64, page_size: u32) -> Heap<FakeBackend> {
        Heap::new(
            FakeBackend::default(),
            segment_size,
            page_size,
            0,
            VulkanMemoryType::None,
            false,
        )
    }

    #[test]
    fn helpers_match_c() {
        assert_eq!(q_log2(1), 0);
        assert_eq!(q_log2(4096), 12);
        assert_eq!(q_log2(4097), 12);
        assert_eq!(q_next_pow2(0), 1);
        assert_eq!(q_next_pow2(1), 1);
        assert_eq!(q_next_pow2(2), 2);
        assert_eq!(q_next_pow2(3), 4);
        assert_eq!(q_next_pow2(4096), 4096);
        assert_eq!(q_align_u16(0, 4), 0);
        assert_eq!(q_align_u16(5, 4), 8);
        assert_eq!(q_align_u16(8, 4), 8);
        assert_eq!(q_align_u16(5, 1), 5);
        assert_eq!(q_align_u16(5, 0), 0);
        assert_eq!(q_align_u16(65535, 2), 0);
    }

    /// Fills a `MAX_PAGES` segment page by page, then frees it back to one
    /// block.
    #[test]
    fn page_saturation_at_max_pages() {
        let page_size = 128u32;
        let mut h = heap(u64::from(MAX_PAGES) * u64::from(page_size), page_size);
        let mut allocs = Vec::with_capacity(MAX_PAGES as usize);
        for i in 0..MAX_PAGES {
            let a = h.allocate(u64::from(page_size), 1, ()).expect("page");
            assert_eq!(a.offset(), u64::from(i) * u64::from(page_size));
            assert_eq!(a.segment_index(), Some(0));
            allocs.push(a);
        }
        assert_eq!(h.num_segments(), 1);
        assert_eq!(h.stats().num_pages_free, 0);
        let spill = h
            .allocate(u64::from(page_size), 1, ())
            .expect("second segment");
        assert_eq!(spill.segment_index(), Some(1));
        assert_eq!(h.num_segments(), 2);
        h.check_consistency().unwrap();
        h.free(spill, ());
        for a in allocs.drain(..).rev() {
            h.free(a, ());
        }
        h.check_consistency().unwrap();
        h.check_clean_state().unwrap();
        h.destroy(());
        assert_eq!(h.backend().live, 0);
    }

    #[test]
    fn dedicated_and_small_paths() {
        let mut h = heap(1 << 20, 4096);
        h.check_clean_state().unwrap();
        let big = h.allocate(2 << 20, 1, ()).unwrap();
        assert!(big.is_dedicated());
        assert_eq!(big.segment_index(), None);
        assert_eq!(big.memory().as_raw(), 1);
        let small = h.allocate(100, 64, ()).unwrap();
        assert!(!small.is_dedicated());
        assert_eq!(small.offset() % 64, 0);
        let small2 = h.allocate(100, 64, ()).unwrap();
        assert_eq!(small2.offset(), small.offset() + 128);
        h.check_consistency().unwrap();
        let s = *h.stats();
        assert_eq!(s.num_allocations, 3);
        assert_eq!(s.num_small_allocations, 2);
        assert_eq!(s.num_dedicated_allocations, 1);
        assert_eq!(s.num_pages_allocated, 1);
        h.free(small, ());
        h.free(small2, ());
        h.free(big, ());
        h.check_clean_state().unwrap();
    }
}
