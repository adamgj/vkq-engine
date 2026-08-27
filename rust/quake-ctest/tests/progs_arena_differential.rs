//! `quake_progs::{arena, alloc}` vs the C originals in `pr_edict_arena.c`
//! (Phase 6 M2).
//!
//! Entity numbering is observable — it reaches savegames and the wire
//! protocol — so the question is not "does Rust allocate a free edict" but
//! "does it return the *same number* and leave the *same bytes* behind".
//! Every test drives an identical operation sequence against two fixtures (the
//! C-owned VM the oracle dereferences, and a Rust-owned arena of the same
//! stride) and then compares the whole free list, `num_edicts`, and every
//! edict byte-for-byte.

use core::ffi::{c_int, c_void};
use core::mem::offset_of;

use quake_ctest as _;
use quake_progs::alloc::{self, AllocCtx, AllocError, FreeListWarning};
use quake_progs::arena::{EdictArena, EdictId};
use quake_types::progs::{Edict, EntityState, FreeList, QcVm, ENGINE_DEBUG, MAX_EDICTS};

extern "C" {
    fn ctest_progs_reset_vm(max_edicts: c_int, entityfields: c_int) -> *mut c_void;
    fn ctest_progs_edict_size() -> usize;
    fn ctest_progs_edicts() -> *mut c_void;
    fn ctest_progs_set_time(t: f64);
    fn ctest_progs_sort_by_freetime(nums: *mut c_int, n: usize, freetimes: *const f32);
    fn ctest_try_host(f: extern "C" fn(*mut c_void), arg: *mut c_void) -> c_int;

    fn c_ref_ED_Alloc() -> *mut c_void;
    fn c_ref_ED_Free(ed: *mut c_void);
    fn c_ref_ED_RemoveFromFreeList(ed: *mut c_void);
    fn c_ref_ED_RebuildFreeList(force_free_reuse: bool);

    static c_ref_qcvm: *mut c_void;
    static c_ref_nullentitystate: EntityState;
}

/// The oracle's ambient `qcvm` is one process-wide instance.
static VM_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    VM_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn c_vm() -> *mut QcVm {
    // SAFETY: ctest_progs_reset_vm published a real qcvm_t here, and
    // tests/progs_abi.rs proves the mirror matches the C layout.
    unsafe { c_ref_qcvm.cast() }
}

fn c_free_list() -> &'static FreeList {
    // SAFETY: as above; the caller holds VM_LOCK, and the oracle is not
    // running concurrently.
    unsafe { &(*c_vm()).free_list }
}

fn c_num_edicts() -> c_int {
    // SAFETY: as above.
    unsafe { (*c_vm()).num_edicts }
}

fn c_edict_ptr(num: c_int) -> *mut c_void {
    // SAFETY: callers stay below max_edicts, so the offset is inside the
    // `max_edicts * edict_size` block the fixture allocated.
    unsafe {
        (ctest_progs_edicts() as *mut u8)
            .add(num as usize * ctest_progs_edict_size())
            .cast()
    }
}

fn c_edict_num(p: *mut c_void) -> c_int {
    // SAFETY: both are plain reads of fixture scalars; the caller holds
    // VM_LOCK, and `p` came out of this same array.
    let base = unsafe { ctest_progs_edicts() } as usize;
    // SAFETY: as above.
    let stride = unsafe { ctest_progs_edict_size() };
    ((p as usize - base) / stride) as c_int
}

fn c_edict_bytes(num: c_int) -> &'static [u8] {
    // SAFETY: a plain read of a fixture scalar; the caller holds VM_LOCK.
    let stride = unsafe { ctest_progs_edict_size() };
    // SAFETY: num < max_edicts, and the block is `max_edicts * stride` bytes.
    unsafe { core::slice::from_raw_parts(c_edict_ptr(num).cast::<u8>(), stride) }
}

/// Under `engine-debug` the first two header fields are `edict_ptr` and
/// `qcvm_owner`, which point into whichever fixture owns them and so differ by
/// construction. Everything from `edict_num` on is comparable.
fn cmp_from() -> usize {
    if ENGINE_DEBUG {
        offset_of!(Edict, area) - core::mem::size_of::<u64>()
    } else {
        0
    }
}

struct Fixture {
    arena: EdictArena,
    free_list: Box<FreeList>,
    num_edicts: c_int,
    max_edicts: c_int,
    entityfields: c_int,
    time: f64,
    null_state: EntityState,
    unlinked: Vec<EdictId>,
}

fn new_fixture(max_edicts: c_int, entityfields: c_int) -> Fixture {
    // SAFETY: the caller holds VM_LOCK.
    unsafe { ctest_progs_reset_vm(max_edicts, entityfields) };
    // SAFETY: a plain read of a fixture scalar just published above.
    let stride = unsafe { ctest_progs_edict_size() };
    // SAFETY: the fixture initialises it via COM_SetupNullState's values.
    let null_state = unsafe { c_ref_nullentitystate };

    Fixture {
        arena: EdictArena::owned(stride, max_edicts as usize),
        // freelist_t is 64 KB of circular buffer; box it rather than stack it
        free_list: Box::new(FreeList {
            size: 0,
            head_index: 0,
            circular_buffer: [0u16; MAX_EDICTS],
        }),
        num_edicts: 0,
        max_edicts,
        entityfields,
        time: 0.0,
        null_state,
        unlinked: Vec::new(),
    }
}

impl Fixture {
    fn set_time(&mut self, t: f64) {
        self.time = t;
        // SAFETY: the caller holds VM_LOCK.
        unsafe { ctest_progs_set_time(t) };
    }

    /// Rust `ED_Alloc` beside the oracle's. Both sides must either succeed
    /// with the *same* edict number or fail together — C by `Host_Error`,
    /// Rust by returning the condition (ADR-009).
    fn try_alloc_both(&mut self) -> Option<EdictId> {
        let mut ctx = AllocCtx {
            free_list: &mut self.free_list,
            num_edicts: &mut self.num_edicts,
            max_edicts: self.max_edicts,
            time: self.time,
            entityfields: self.entityfields,
        };
        let rust = alloc::ed_alloc(
            &mut ctx,
            &mut self.arena,
            &self.null_state,
            core::ptr::null_mut(),
        );
        match rust {
            Ok(id) => {
                // SAFETY: the caller holds VM_LOCK; the fixture has room.
                let c = unsafe { c_ref_ED_Alloc() };
                assert_eq!(
                    id.0 as c_int,
                    c_edict_num(c),
                    "ED_Alloc returned a different edict"
                );
                Some(id)
            }
            Err(AllocError::NoFreeEdicts { max_edicts }) => {
                assert_eq!(max_edicts, self.max_edicts);
                extern "C" fn call_alloc(_: *mut c_void) {
                    // SAFETY: run under the armed Host_Error trap.
                    unsafe { c_ref_ED_Alloc() };
                }
                // SAFETY: the trap is armed for exactly this call, and it
                // longjmps out before the oracle mutates the fixture.
                let raised = unsafe { ctest_try_host(call_alloc, core::ptr::null_mut()) };
                assert_eq!(
                    raised, 1,
                    "Rust reported exhaustion but C ED_Alloc succeeded"
                );
                None
            }
        }
    }

    fn alloc_both(&mut self) -> EdictId {
        self.try_alloc_both().expect("ED_Alloc had room")
    }

    fn free_both(&mut self, id: EdictId) {
        let unlinked = &mut self.unlinked;
        alloc::ed_free(
            &mut self.free_list,
            &mut self.arena,
            id,
            self.time,
            &mut |e| unlinked.push(e),
        );
        // SAFETY: as above.
        unsafe { c_ref_ED_Free(c_edict_ptr(id.0 as c_int)) };
    }

    fn remove_from_free_list_both(&mut self, id: EdictId) {
        alloc::remove_from_free_list(&mut self.free_list, id);
        // SAFETY: as above.
        unsafe { c_ref_ED_RemoveFromFreeList(c_edict_ptr(id.0 as c_int)) };
    }

    fn rebuild_both(&mut self, force_free_reuse: bool) {
        let freetimes: Vec<f32> = (0..self.num_edicts)
            .map(|n| self.arena.freetime(EdictId(n as u32)))
            .collect();
        alloc::rebuild_free_list(
            &mut self.free_list,
            &mut self.arena,
            self.num_edicts,
            force_free_reuse,
            &mut |nums| {
                // the same libc qsort and the same never-zero comparator the C
                // build uses, so tie ordering matches by construction
                // SAFETY: `nums` and `freetimes` are live for the call, and
                // every entry of `nums` indexes `freetimes`.
                unsafe {
                    ctest_progs_sort_by_freetime(nums.as_mut_ptr(), nums.len(), freetimes.as_ptr())
                };
            },
        );
        // SAFETY: as above.
        unsafe { c_ref_ED_RebuildFreeList(force_free_reuse) };
    }

    fn assert_same(&self, label: &str) {
        assert_eq!(self.num_edicts, c_num_edicts(), "{label}: num_edicts");
        let c = c_free_list();
        assert_eq!(self.free_list.size, c.size, "{label}: free_list.size");
        assert_eq!(
            self.free_list.head_index, c.head_index,
            "{label}: free_list.head_index"
        );
        for i in 0..c.size {
            let idx = (c.head_index + i) % MAX_EDICTS;
            assert_eq!(
                self.free_list.circular_buffer[idx], c.circular_buffer[idx],
                "{label}: free_list position {i} (slot {idx})"
            );
        }
        let from = cmp_from();
        for n in 0..self.num_edicts {
            let rust = &self.arena.edict_bytes(EdictId(n as u32))[from..];
            let cc = &c_edict_bytes(n)[from..];
            assert_eq!(rust, cc, "{label}: edict {n} bytes");
        }
    }
}

/// A deterministic xorshift, so a failure is reproducible from the seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: u32) -> u32 {
        (self.next() % u64::from(n)) as u32
    }
}

#[test]
fn fresh_allocs_number_and_zero_identically() {
    let _g = lock();
    let mut f = new_fixture(64, 128);
    f.set_time(10.0);
    for expect in 0..32 {
        let id = f.alloc_both();
        assert_eq!(id.0, expect);
    }
    f.assert_same("fresh allocs");
}

/// The FIFO reuse rule: `freetime < MAX_EDICT_FREETIME_ALWAYS_REUSE ||
/// (time - freetime) > MIN_EDICT_AGE_FOR_REUSE`. Both thresholds are 2.0, so
/// this walks the time axis across each of them.
#[test]
fn reuse_policy_matches_across_both_age_thresholds() {
    for &(free_at, alloc_at) in &[
        (0.0, 0.5),   // freetime < 2.0 -> always reuse
        (1.9, 2.0),   // still below the always-reuse threshold
        (5.0, 5.0),   // age 0 -> too young, must extend instead
        (5.0, 7.0),   // age exactly 2.0 -> NOT > 2.0, still too young
        (5.0, 7.001), // age just over -> reuse
        (5.0, 60.0),  // long past
    ] {
        let _g = lock();
        let mut f = new_fixture(64, 128);
        f.set_time(free_at);
        let a = f.alloc_both();
        let b = f.alloc_both();
        f.free_both(a);
        f.free_both(b);
        f.assert_same("after frees");

        f.set_time(alloc_at);
        let got = f.alloc_both();
        f.assert_same(&format!("realloc at {alloc_at} after free at {free_at}"));
        let _ = got;
    }
}

#[test]
fn free_then_alloc_cycles_match_over_a_random_walk() {
    for seed in 1..=8u64 {
        let _g = lock();
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut f = new_fixture(96, 137);
        let mut live: Vec<EdictId> = Vec::new();

        for step in 0..400 {
            f.set_time(f64::from(step) * 0.05);
            let roll = rng.below(100);
            if roll < 55 || live.is_empty() {
                // exhaustion is reachable here (the free list can hold only
                // edicts too young to reuse); try_alloc_both asserts the two
                // sides agree about it rather than skipping the case
                if let Some(id) = f.try_alloc_both() {
                    live.push(id);
                }
            } else if roll < 90 {
                let i = rng.below(live.len() as u32) as usize;
                let id = live.swap_remove(i);
                f.free_both(id);
            } else {
                // ED_RemoveFromFreeList on an edict that is in the list
                if f.free_list.size > 0 {
                    let pos = rng.below(f.free_list.size as u32) as usize;
                    let slot = (f.free_list.head_index + pos) % MAX_EDICTS;
                    let num = f.free_list.circular_buffer[slot];
                    f.remove_from_free_list_both(EdictId(u32::from(num)));
                }
            }
            f.assert_same(&format!("seed {seed} step {step}"));
        }
    }
}

/// `ED_RemoveFromFreeList` overwrites the found slot with the *head* entry and
/// advances the head, so the FIFO's relative order changes. Removing an edict
/// that is not in the list must be a no-op.
#[test]
fn remove_from_free_list_matches_including_absent_entries() {
    let _g = lock();
    let mut f = new_fixture(32, 105);
    f.set_time(1.0);
    let ids: Vec<EdictId> = (0..8).map(|_| f.alloc_both()).collect();
    for &id in &ids[..6] {
        f.free_both(id);
    }
    f.assert_same("six freed");

    // middle of the list, then the head, then one that was never freed
    f.remove_from_free_list_both(ids[3]);
    f.assert_same("removed middle");
    f.remove_from_free_list_both(ids[0]);
    f.assert_same("removed old head");
    f.remove_from_free_list_both(ids[7]);
    f.assert_same("removed absent");
}

#[test]
fn rebuild_free_list_matches_with_and_without_forced_reuse() {
    for force in [false, true] {
        let _g = lock();
        let mut rng = Rng(0xDEAD_BEEF ^ u64::from(force));
        let mut f = new_fixture(64, 111);
        let ids: Vec<EdictId> = (0..40)
            .map(|i| {
                // staggered times so freetimes both tie and differ
                f.set_time(f64::from(i / 4) * 3.0);
                f.alloc_both()
            })
            .collect();
        for &id in &ids {
            if rng.below(2) == 0 {
                f.free_both(id);
            }
        }
        f.assert_same("before rebuild");
        f.rebuild_both(force);
        f.assert_same(&format!("after rebuild(force={force})"));
    }
}

/// Double-free is C's early return, and the freed edict's bytes must not move.
#[test]
fn double_free_is_a_no_op_on_both_sides() {
    let _g = lock();
    let mut f = new_fixture(16, 105);
    f.set_time(3.0);
    let a = f.alloc_both();
    f.free_both(a);
    f.assert_same("first free");
    f.free_both(a);
    f.assert_same("second free");
    assert_eq!(f.free_list.size, 1, "double free must not re-enqueue");
}

/// `ED_Alloc` past `max_edicts` is a `Host_Error` in C; the Rust port returns
/// the condition instead so the raise happens in a C frame (ADR-009).
#[test]
fn exhausting_max_edicts_reports_rather_than_raises() {
    let _g = lock();
    let max = 8;
    let mut f = new_fixture(max, 105);
    f.set_time(100.0);
    for _ in 0..max {
        f.alloc_both();
    }
    f.assert_same("full");

    let mut ctx = AllocCtx {
        free_list: &mut f.free_list,
        num_edicts: &mut f.num_edicts,
        max_edicts: f.max_edicts,
        time: f.time,
        entityfields: f.entityfields,
    };
    let err = alloc::ed_alloc(&mut ctx, &mut f.arena, &f.null_state, core::ptr::null_mut());
    assert_eq!(err, Err(AllocError::NoFreeEdicts { max_edicts: max }));

    extern "C" fn overflow(_: *mut c_void) {
        // SAFETY: the fixture is full, so this takes the Host_Error path.
        unsafe { c_ref_ED_Alloc() };
    }
    // SAFETY: the trap is armed for exactly this call.
    let raised = unsafe { ctest_try_host(overflow, core::ptr::null_mut()) };
    assert_eq!(raised, 1, "C ED_Alloc must Host_Error when full");
}

/// `ED_CheckFreeList` is a pure cross-check in Rust: it returns the warnings C
/// would print instead of printing them (`Con_Warning` is not a leaf).
#[test]
fn check_free_list_reports_the_same_inconsistencies() {
    let _g = lock();
    let mut f = new_fixture(32, 105);
    f.set_time(1.0);
    let ids: Vec<EdictId> = (0..6).map(|_| f.alloc_both()).collect();
    for &id in &ids[..3] {
        f.free_both(id);
    }
    assert!(
        alloc::check_free_list(&f.free_list, &f.arena, f.num_edicts).is_empty(),
        "a consistent list must produce no warnings"
    );

    // corrupt one way: mark a listed edict as live
    f.arena.set_free(ids[0], false);
    let w = alloc::check_free_list(&f.free_list, &f.arena, f.num_edicts);
    assert!(
        w.contains(&FreeListWarning::InListButNotFree(ids[0].0 as c_int)),
        "{w:?}"
    );
    assert!(
        w.contains(&FreeListWarning::NotFreeButInList(ids[0].0 as c_int)),
        "{w:?}"
    );
    f.arena.set_free(ids[0], true);

    // and the other way: mark an unlisted edict as free
    f.arena.set_free(ids[5], true);
    let w = alloc::check_free_list(&f.free_list, &f.arena, f.num_edicts);
    assert_eq!(
        w,
        vec![FreeListWarning::FreeButNotInList(ids[5].0 as c_int)]
    );
}

/// `ED_Free` calls `SV_UnlinkEdict` exactly once per transition to free, and
/// not at all on a double free.
#[test]
fn unlink_is_called_once_per_free_transition() {
    let _g = lock();
    let mut f = new_fixture(16, 105);
    f.set_time(1.0);
    let a = f.alloc_both();
    let b = f.alloc_both();
    f.free_both(a);
    f.free_both(a);
    f.free_both(b);
    assert_eq!(f.unlinked, vec![a, b]);
}
