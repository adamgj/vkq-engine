//! `ED_Alloc`/`ED_Free` and the FIFO free list (`pr_edict_arena.c`).
//!
//! Entity numbering is observable — it reaches savegames and the wire
//! protocol — so this is bit-exact or nothing (ADR-006). Everything here is
//! safe code over [`EdictArena`]'s checked accessors.

use core::ffi::c_int;

use quake_types::progs::{
    EntityState, FreeList, MAX_EDICTS, MAX_EDICT_FREETIME_ALWAYS_REUSE, MIN_EDICT_AGE_FOR_REUSE,
};

use crate::arena::{entvars_ofs as ev, EdictArena, EdictId};

/// The parts of `qcvm_t` the allocator touches, borrowed for one call so no
/// reference into the VM outlives it (ADR-006 Phase 6 amendment).
pub struct AllocCtx<'a> {
    pub free_list: &'a mut FreeList,
    pub num_edicts: &'a mut c_int,
    pub max_edicts: c_int,
    pub time: f64,
    /// `qcvm->progs->entityfields`.
    pub entityfields: c_int,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AllocError {
    /// `ED_Alloc: no free edicts (max_edicts is %i)`
    NoFreeEdicts { max_edicts: c_int },
}

/// Diagnostics `ED_CheckFreeList` would have printed. Accumulated rather than
/// printed inline: `Con_Warning` is not a leaf (it can reach
/// `SCR_UpdateScreen`), so the caller drains these once its borrows are gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreeListWarning {
    /// `edict %i is in free-list but is NOT free`
    InListButNotFree(c_int),
    /// `edict %i is free, but is NOT in free-list`
    FreeButNotInList(c_int),
    /// `edict %i is NOT free, but is in free-list`
    NotFreeButInList(c_int),
}

/// `ED_AddToFreeList`.
///
/// The debug-build overflow checks live in the caller's C frame (they
/// `Host_Error`), as does the free-list-full condition.
pub fn add_to_free_list(free_list: &mut FreeList, id: EdictId) {
    let add_index = (free_list.head_index + free_list.size) % MAX_EDICTS;
    free_list.circular_buffer[add_index] = id.0 as u16;
    free_list.size += 1;
}

/// Which of `ED_AddToFreeList`'s two debug-only preconditions a free list
/// violates. C raises a *distinct* `Host_Error` for each, so they are reported
/// separately rather than collapsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreeListOverflow {
    /// `qcvm->free_list.size >= MAX_EDICTS`
    Full,
    /// `qcvm->free_list.size >= qcvm->max_edicts`
    OverMaxEdicts,
}

/// Debug-only precondition of `ED_AddToFreeList`, reported rather than raised
/// so the `Host_Error` happens in a C frame (ADR-009). C tests `MAX_EDICTS`
/// first, so that arm wins when both hold.
#[must_use]
pub fn free_list_would_overflow(
    free_list: &FreeList,
    max_edicts: c_int,
) -> Option<FreeListOverflow> {
    if free_list.size >= MAX_EDICTS {
        Some(FreeListOverflow::Full)
    } else if free_list.size >= max_edicts.max(0) as usize {
        Some(FreeListOverflow::OverMaxEdicts)
    } else {
        None
    }
}

/// `ED_Alloc`. Returns the edict; the caller invokes `ED_ALLOC_HOOK`, which
/// stays C-side state read by `sv_phys.c`.
///
/// COMPAT: the reuse test is `freetime < MAX_EDICT_FREETIME_ALWAYS_REUSE ||
/// (time - freetime) > MIN_EDICT_AGE_FOR_REUSE`, evaluated in `double` because
/// `qcvm->time` is a double and `freetime` promotes.
pub fn ed_alloc(
    ctx: &mut AllocCtx<'_>,
    arena: &mut EdictArena,
    null_state: &EntityState,
    qcvm: *mut core::ffi::c_void,
) -> Result<EdictId, AllocError> {
    if ctx.free_list.size > 0 {
        let head = EdictId(u32::from(
            ctx.free_list.circular_buffer[ctx.free_list.head_index],
        ));
        let freetime = arena.freetime(head);
        if f64::from(freetime) < f64::from(MAX_EDICT_FREETIME_ALWAYS_REUSE)
            || (ctx.time - f64::from(freetime)) > f64::from(MIN_EDICT_AGE_FOR_REUSE)
        {
            debug_assert!(arena.free(head));
            arena.clear_fields(head, ctx.entityfields);
            arena.set_free(head, false);
            ctx.free_list.head_index = (ctx.free_list.head_index + 1) % MAX_EDICTS;
            ctx.free_list.size -= 1;
            return Ok(head);
        }
    }

    if *ctx.num_edicts == ctx.max_edicts {
        return Err(AllocError::NoFreeEdicts {
            max_edicts: ctx.max_edicts,
        });
    }

    let id = EdictId(*ctx.num_edicts as u32);
    *ctx.num_edicts += 1;

    // 'new' free edicts are not necessarily clean after a load/fastload, so
    // the whole edict is reset rather than just the field block.
    arena.clear_edict(id);
    arena.set_free(id, false);
    arena.set_baseline(id, null_state);
    arena.set_debug_header(id, qcvm, u64::from(id.0));

    Ok(id)
}

/// `ED_Free`. `unlink` is `SV_UnlinkEdict` (world.c, C until Phase 7).
///
/// Returns `false` when the edict was already free, which is C's early
/// return — including its `assert (!ed->area.prev)`.
pub fn ed_free(
    free_list: &mut FreeList,
    arena: &mut EdictArena,
    id: EdictId,
    time: f64,
    unlink: &mut dyn FnMut(EdictId),
) -> bool {
    if arena.free(id) {
        debug_assert!(arena.area_prev_is_null(id));
        return false;
    }

    unlink(id);

    arena.set_free(id, true);
    arena.set_field_i32(id, ev::MODEL, 0);
    arena.set_field_f32(id, ev::TAKEDAMAGE, 0.0);
    arena.set_field_f32(id, ev::MODELINDEX, 0.0);
    arena.set_field_f32(id, ev::COLORMAP, 0.0);
    arena.set_field_f32(id, ev::SKIN, 0.0);
    arena.set_field_f32(id, ev::FRAME, 0.0);
    arena.set_field_vec3(id, ev::ORIGIN, [0.0; 3]);
    arena.set_field_vec3(id, ev::ANGLES, [0.0; 3]);
    arena.set_field_f32(id, ev::NEXTTHINK, -1.0);
    arena.set_field_f32(id, ev::SOLID, 0.0);
    // ENTALPHA_DEFAULT (protocol.h) -- must be zero so zeroed memory works
    arena.set_alpha(id, 0);

    arena.set_freetime(id, time as f32);

    add_to_free_list(free_list, id);
    true
}

/// `ED_RemoveFromFreeList`.
///
/// COMPAT: the removal is not a compaction — the found slot is overwritten
/// with the *head* entry and the head advances, so the FIFO's relative order
/// changes in a way callers can observe through later `ED_Alloc` numbering.
pub fn remove_from_free_list(free_list: &mut FreeList, id: EdictId) {
    let num = id.0 as u16;
    let head_index = free_list.head_index;
    for i in 0..free_list.size {
        let found_index = (head_index + i) % MAX_EDICTS;
        if free_list.circular_buffer[found_index] == num {
            free_list.circular_buffer[found_index] = free_list.circular_buffer[head_index];
            free_list.head_index = (head_index + 1) % MAX_EDICTS;
            free_list.size -= 1;
            break;
        }
    }
}

/// `ED_CheckFreeList` — the debugging cross-check. Returns the warnings C
/// would have printed; a non-empty result is C's `has_errors`, which makes it
/// call [`rebuild_free_list`] with `force_free_reuse = false`.
#[must_use]
pub fn check_free_list(
    free_list: &FreeList,
    arena: &EdictArena,
    num_edicts: c_int,
) -> Vec<FreeListWarning> {
    let mut warnings = Vec::new();
    let mut in_list = vec![0u8; MAX_EDICTS];

    let mut current_index = free_list.head_index;
    for _ in 0..free_list.size {
        let edict_num = c_int::from(free_list.circular_buffer[current_index]);
        if !arena.free(EdictId(edict_num as u32)) {
            warnings.push(FreeListWarning::InListButNotFree(edict_num));
        }
        in_list[edict_num as usize] = 1;
        current_index = (current_index + 1) % MAX_EDICTS;
    }

    for i in 0..num_edicts {
        let free = arena.free(EdictId(i as u32));
        if free {
            if in_list[i as usize] != 1 {
                warnings.push(FreeListWarning::FreeButNotInList(i));
            }
        } else if in_list[i as usize] != 0 {
            warnings.push(FreeListWarning::NotFreeButInList(i));
        }
    }

    warnings
}

/// `ED_freetime_compare_func`.
///
/// COMPAT: C returns `(int)copysign (1.0, a - b)`, which is **never 0** — so
/// equal freetimes compare as "greater" (`copysign(1.0, +0.0) == +1.0`) and
/// the comparator is inconsistent. `qsort` with an inconsistent comparator has
/// implementation-defined ordering, which is why [`rebuild_free_list`] takes
/// the sort as a parameter instead of implementing it: the engine must keep
/// calling the same platform `qsort` to keep tie ordering identical.
#[must_use]
pub fn freetime_compare(a: f32, b: f32) -> c_int {
    // float subtraction, as in C (usual arithmetic conversions keep it f32
    // before copysign promotes the result to double)
    let d = a - b;
    if d.is_sign_negative() {
        -1
    } else {
        1
    }
}

/// `ED_RebuildFreeList`.
///
/// `sort` must order the edict numbers by [`freetime_compare`] using the same
/// `qsort` the C build uses — see that function's COMPAT note. It is not
/// called at all when `force_free_reuse` is set, matching C.
pub fn rebuild_free_list(
    free_list: &mut FreeList,
    arena: &mut EdictArena,
    num_edicts: c_int,
    force_free_reuse: bool,
    sort: &mut dyn FnMut(&mut [c_int]),
) {
    let mut free_edicts = Vec::with_capacity(num_edicts.max(0) as usize);
    for i in 0..num_edicts {
        let id = EdictId(i as u32);
        if arena.free(id) {
            if force_free_reuse {
                arena.set_freetime(id, 0.0);
            }
            free_edicts.push(i);
        }
    }

    if !force_free_reuse {
        sort(&mut free_edicts);
    }

    // memset (&qcvm->free_list, 0, sizeof (freelist_t)) -- the whole circular
    // buffer, not just the size and head
    free_list.size = 0;
    free_list.head_index = 0;
    free_list.circular_buffer.fill(0);

    for num in free_edicts {
        add_to_free_list(free_list, EdictId(num as u32));
    }
}
