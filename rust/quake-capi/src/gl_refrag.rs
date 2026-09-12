//! `gl_refrag.c` -- static-entity fragment lists (Rust migration Phase 8 M7).
//!
//! `R_AddEfrags` and `R_RebuildAllEfrags` are exported directly; their only
//! callees are `Mem_*`, `Con_DWarning` and the BSP walk. `R_StoreEfrags` runs
//! `PScript_RunParticleEffectState` and `R_AllocateEntityBLAS`, which can
//! reach `Host_Error`, so `Quake/gl_refrag_glue.c` keeps the C entry point:
//! it calls [`RRefrag_StoreEfrags`], which routes those two calls through
//! `Host_Guard` wrappers and returns the guard code for the C side to
//! `Host_Reraise` (ADR-009 -- no longjmp through Rust frames).

use core::ffi::{c_float, c_int, c_void};
use core::ptr;

use quake_c_sys as c;
use quake_math::mathlib::{angle_vectors, vector_add, vector_ma, vector_scale, Vec3};
use quake_types::host::{ClientState, ClientStatic, Efrag, Entity};
use quake_types::model_mem::{MLeaf, MNode, QModel, MOD_ALIAS};
use quake_types::plane::MPlane;

extern "C" {
    /// `client_state_t cl`, `client_static_t cls` (ADR-007 rows closed in
    /// Phase 7).
    static mut cl: ClientState;
    static mut cls: ClientStatic;
}

/// `Host_Guard` result (`quakedef.h:475`): 0 is `HOST_GUARD_OK`.
type Raise = c_int;

macro_rules! raise {
    ($e:expr) => {{
        let r: Raise = $e;
        if r != 0 {
            return r;
        }
    }};
}

/// `bspfile.h:163`.
const CONTENTS_SOLID: i32 = -2;
/// `gl_model.h:610-611`.
const MOD_EMITREPLACE: i32 = 2048;
const MOD_EMITFORWARDS: i32 = 4096;
/// `gl_refrag.c:53`.
const EXTRA_EFRAGS: usize = 128;

/// `ENTSCALE_DECODE (es)` (`protocol.h`): `(es) / 16.0f`.
fn entscale_decode(es: u8) -> f32 {
    f32::from(es) / 16.0
}

static mut R_PEFRAGTOPNODE: *mut MNode = ptr::null_mut();
static mut R_EMINS: Vec3 = [0.0; 3];
static mut R_EMAXS: Vec3 = [0.0; 3];
static mut R_ADDENT: *mut Entity = ptr::null_mut();

/// `R_GetEfrag` (`gl_refrag.c:56`): pops the free list, refilling it with a
/// block of `EXTRA_EFRAGS` when empty (the recursion in C is one level deep).
///
/// # Safety
/// `cl` is the live client state on the main thread.
unsafe fn get_efrag() -> *mut Efrag {
    // SAFETY: the caller's contract; the free list is a well-formed singly
    // linked list of `Mem_Alloc`ed blocks tracked in `cl.efrag_allocs`.
    unsafe {
        let clp = ptr::addr_of_mut!(cl);
        loop {
            if !(*clp).free_efrags.is_null() {
                let ef = (*clp).free_efrags;
                (*clp).free_efrags = (*ef).leafnext;
                (*ef).leafnext = ptr::null_mut();

                (*clp).num_efrags += 1;

                return ef;
            }

            let block = c::Mem_Alloc(EXTRA_EFRAGS * core::mem::size_of::<Efrag>()).cast::<Efrag>();
            (*clp).free_efrags = block;

            // Track allocations so we don't leak
            (*clp).efrag_allocs = c::Mem_Realloc(
                (*clp).efrag_allocs.cast::<c_void>(),
                core::mem::size_of::<*mut *mut Efrag>() * ((*clp).num_efragallocs as usize + 1),
            )
            .cast::<*mut Efrag>();
            *(*clp).efrag_allocs.add((*clp).num_efragallocs as usize) = block;
            (*clp).num_efragallocs += 1;

            for i in 0..EXTRA_EFRAGS - 1 {
                (*block.add(i)).leafnext = block.add(i + 1);
            }
            (*block.add(EXTRA_EFRAGS - 1)).leafnext = ptr::null_mut();
        }
    }
}

/// `BOX_ON_PLANE_SIDE (emins, emaxs, p)` (`mathlib.h:160`): the axial fast
/// path, else `BoxOnPlaneSide`.
///
/// # Safety
/// `p` is a valid BSP split plane.
unsafe fn box_on_plane_side(emins: &Vec3, emaxs: &Vec3, p: *const MPlane) -> c_int {
    // SAFETY: the caller's contract.
    unsafe {
        let t = (*p).type_;
        if t < 3 {
            let i = t as usize;
            if (*p).dist <= emins[i] {
                1
            } else if (*p).dist >= emaxs[i] {
                2
            } else {
                3
            }
        } else {
            c::render::BoxOnPlaneSide(emins.as_ptr(), emaxs.as_ptr(), p.cast::<c_void>())
        }
    }
}

/// `R_SplitEntityOnNode`.
///
/// # Safety
/// `node` is a node of `cl.worldmodel`; `R_ADDENT`/`R_EMINS`/`R_EMAXS` are
/// set by `R_AddEfrags`.
unsafe fn split_entity_on_node(node: *mut MNode) {
    // SAFETY: the caller's contract; leaves are `mleaf_t`s sharing the
    // `mnode_t` prefix (`contents < 0`).
    unsafe {
        if (*node).contents == CONTENTS_SOLID {
            return;
        }

        // add an efrag if the node is a leaf

        if (*node).contents < 0 {
            if R_PEFRAGTOPNODE.is_null() {
                R_PEFRAGTOPNODE = node;
            }

            let leaf = node.cast::<MLeaf>();

            // grab an efrag off the free list
            let ef = get_efrag();
            (*ef).entity = R_ADDENT.cast();

            // set the leaf links
            (*ef).leafnext = (*leaf).efrags.cast::<Efrag>();
            (*leaf).efrags = ef.cast::<c_void>();

            return;
        }

        // NODE_MIXED

        let splitplane = (*node).plane;
        let sides = box_on_plane_side(
            &*ptr::addr_of!(R_EMINS),
            &*ptr::addr_of!(R_EMAXS),
            splitplane,
        );

        if sides == 3 {
            // split on this plane
            // if this is the first splitter of this bmodel, remember it
            if R_PEFRAGTOPNODE.is_null() {
                R_PEFRAGTOPNODE = node;
            }
        }

        // recurse down the contacted sides
        if sides & 1 != 0 {
            split_entity_on_node((*node).children[0]);
        }

        if sides & 2 != 0 {
            split_entity_on_node((*node).children[1]);
        }
    }
}

/// `R_CheckEfrags` -- johnfitz -- check for excessive efrag count
/// (`cl_parse.c` calls it after parsing static entities too).
///
/// # Safety
/// Main thread; `cl`/`cls`/`dev_stats` are the live globals.
#[no_mangle]
pub unsafe extern "C" fn R_CheckEfrags() {
    // SAFETY: the caller's contract.
    unsafe {
        if (*ptr::addr_of!(cls)).signon < 2 {
            return; // don't spam when still parsing signon packet full of static ents
        }

        let num_efrags = (*ptr::addr_of!(cl)).num_efrags;
        if num_efrags > 640 && (*ptr::addr_of!(c::cl_parse::dev_peakstats)).efrags <= 640 {
            c::Con_DWarning(
                c"%i efrags exceeds standard limit of 640.\n".as_ptr(),
                num_efrags,
            );
        }

        (*ptr::addr_of_mut!(c::cl_parse::dev_stats)).efrags = num_efrags;
        let peak = ptr::addr_of_mut!((*ptr::addr_of_mut!(c::cl_parse::dev_peakstats)).efrags);
        *peak = num_efrags.max(*peak);
    }
}

/// `R_AddEfrags`.
///
/// # Safety
/// `ent` is a live `entity_t` on the main thread with `cl.worldmodel` loaded.
#[no_mangle]
pub unsafe extern "C" fn R_AddEfrags(ent: *mut Entity) {
    // SAFETY: the caller's contract.
    unsafe {
        if (*ent).model.is_null() {
            return;
        }

        R_ADDENT = ent;

        R_PEFRAGTOPNODE = ptr::null_mut();

        let entmodel: *mut QModel = (*ent).model;

        let scalefactor = entscale_decode((*ent).netstate.scale);
        let emins = &mut *ptr::addr_of_mut!(R_EMINS);
        let emaxs = &mut *ptr::addr_of_mut!(R_EMAXS);
        if scalefactor != 1.0 {
            vector_ma(&(*ent).origin, scalefactor, &(*entmodel).mins, emins);
            vector_ma(&(*ent).origin, scalefactor, &(*entmodel).maxs, emaxs);
        } else {
            vector_add(&(*ent).origin, &(*entmodel).mins, emins);
            vector_add(&(*ent).origin, &(*entmodel).maxs, emaxs);
        }

        split_entity_on_node((*(*ptr::addr_of!(cl)).worldmodel).nodes);

        (*ent).topnode = R_PEFRAGTOPNODE.cast::<c_void>();

        R_CheckEfrags(); // johnfitz
    }
}

/// `R_RebuildAllEfrags`.
///
/// # Safety
/// Main thread with `cl` live.
#[no_mangle]
pub unsafe extern "C" fn R_RebuildAllEfrags() {
    // SAFETY: the caller's contract; `cl.static_entities` holds
    // `cl.num_statics` entity pointers.
    unsafe {
        let clp = ptr::addr_of_mut!(cl);
        if (*clp).worldmodel.is_null() || (*clp).static_entities.is_null() {
            return;
        }

        let world = (*clp).worldmodel;
        for i in 0..(*world).numleafs as usize {
            (*(*world).leafs.add(i)).efrags = ptr::null_mut();
        }

        for i in 0..(*clp).num_efragallocs as usize {
            c::Mem_Free((*(*clp).efrag_allocs.add(i)).cast::<c_void>());
        }
        c::Mem_Free((*clp).efrag_allocs.cast::<c_void>());
        (*clp).efrag_allocs = ptr::null_mut();
        (*clp).num_efragallocs = 0;
        (*clp).free_efrags = ptr::null_mut();
        (*clp).num_efrags = 0;

        for i in 0..(*clp).num_statics as usize {
            let ent = (*(*clp).static_entities.add(i)).cast::<Entity>();
            (*ent).visframe = -1;
            if !(*ent).model.is_null() {
                R_AddEfrags(ent);
            }
        }
    }
}

/// `R_StoreEfrags` -- johnfitz -- pointless switch statement removed.
///
/// Returns the first non-OK `Host_Guard` code (see the module doc), else 0.
///
/// # Safety
/// `ppefrag` points at a leaf's efrag list head; main thread inside the frame.
#[no_mangle]
pub unsafe extern "C" fn RRefrag_StoreEfrags(ppefrag: *mut *mut Efrag) -> Raise {
    // SAFETY: the caller's contract; every efrag's entity is a live static
    // entity with a model.
    unsafe {
        let mut ppefrag = ppefrag;
        let clp = ptr::addr_of!(cl);
        let r_framecount = *ptr::addr_of!(c::render::r_framecount);
        loop {
            let pefrag = *ppefrag;
            if pefrag.is_null() {
                break;
            }
            let pent = (*pefrag).entity.cast::<Entity>();
            if (*pent).visframe != r_framecount
                && *ptr::addr_of!(c::cl_main::cl_numvisedicts)
                    < *ptr::addr_of!(c::cl_main::cl_maxvisedicts)
            {
                // PSET_SCRIPT is defined unconditionally (quakedef.h:38).
                let model = (*pent).model;
                if (*pent).netstate.emiteffectnum > 0 {
                    let mut t: c_float = ((*clp).time - (*clp).oldtime) as c_float;
                    let mut axis = [[0.0f32; 3]; 3];
                    if t < 0.0 {
                        t = 0.0;
                    } else if f64::from(t) > 0.1 {
                        t = 0.1;
                    }
                    let (a0, rest) = axis.split_at_mut(1);
                    let (a1, a2) = rest.split_at_mut(1);
                    angle_vectors(&(*pent).angles, &mut a0[0], &mut a1[0], &mut a2[0]);
                    if (*model).type_ == MOD_ALIAS {
                        axis[0][2] *= -1.0; // stupid vanilla bug
                    }
                    let typenum =
                        (*clp).particle_precache[(*pent).netstate.emiteffectnum as usize].index;
                    raise!(c::render::Refrag_Glue_RunParticleEffectState(
                        (*pent).origin.as_ptr(),
                        axis[0].as_ptr(),
                        t,
                        typenum,
                        ptr::addr_of_mut!((*pent).emitstate),
                    ));
                } else if (*model).emiteffect >= 0 {
                    let mut t: c_float = ((*clp).time - (*clp).oldtime) as c_float;
                    let mut axis = [[0.0f32; 3]; 3];
                    if t < 0.0 {
                        t = 0.0;
                    } else if f64::from(t) > 0.1 {
                        t = 0.1;
                    }
                    let (a0, rest) = axis.split_at_mut(1);
                    let (a1, a2) = rest.split_at_mut(1);
                    angle_vectors(&(*pent).angles, &mut a0[0], &mut a1[0], &mut a2[0]);
                    if (*model).flags & MOD_EMITFORWARDS != 0 {
                        if (*model).type_ == MOD_ALIAS {
                            axis[0][2] *= -1.0; // stupid vanilla bug
                        }
                    } else {
                        let up = axis[2];
                        vector_scale(&up, -1.0, &mut axis[0]);
                    }
                    raise!(c::render::Refrag_Glue_RunParticleEffectState(
                        (*pent).origin.as_ptr(),
                        axis[0].as_ptr(),
                        t,
                        (*model).emiteffect,
                        ptr::addr_of_mut!((*pent).emitstate),
                    ));
                    if (*model).flags & MOD_EMITREPLACE != 0 {
                        ppefrag = ptr::addr_of_mut!((*pefrag).leafnext);
                        continue;
                    }
                }
                raise!(c::render::Refrag_Glue_AllocateEntityBLAS(
                    pent.cast::<c_void>()
                ));
                let n = *ptr::addr_of!(c::cl_main::cl_numvisedicts);
                *(*ptr::addr_of!(c::cl_main::cl_visedicts)).add(n as usize) = pent.cast::<c_void>();
                *ptr::addr_of_mut!(c::cl_main::cl_numvisedicts) = n + 1;
                (*pent).visframe = r_framecount;
            }
            ppefrag = ptr::addr_of_mut!((*pefrag).leafnext);
        }
        0
    }
}
