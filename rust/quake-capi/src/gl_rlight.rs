//! `gl_rlight.c` -- light-style animation, dynamic-light surface marking,
//! the rerelease "dynamiclight" entities and light-point sampling (Rust
//! migration Phase 8 M7).
//!
//! Nothing here can reach `Host_Error`, so every entry point exports its C
//! name directly; `Quake/gl_rlight_glue.c` only keeps the `r_entdlightscale`
//! cvar definition (cvars stay C-owned during Phase 8).

use core::ffi::{c_char, c_float, c_int};
use core::ptr;

use quake_c_sys as c;
use quake_math::mathlib::{dot_product, vector_length, vector_normalize, vector_subtract, Vec3};
use quake_types::host::{ClientState, LightCache, MAX_LIGHTSTYLES};
use quake_types::model_mem::{MNode, MSurface, SURF_DRAWTILED};

use crate::gl_rmisc::vulkan_globals;

extern "C" {
    /// `client_state_t cl` (ADR-007 row closed in Phase 7).
    static mut cl: ClientState;
}

/// `glquake.h` `MAX_DLIGHTS`.
const MAX_DLIGHTS: usize = 64;
/// `protocol.h:239`.
const GAME_COOP: c_int = 0;
/// `gl_rlight.c:210-211`.
const MAX_ENTITY_DLIGHTS: usize = 64;
const ENTITY_DLIGHT_KEY: c_int = 0x4000_0000;
/// `common.h` `CPE_ALLOWTRUNC`.
const CPE_ALLOWTRUNC: c_int = 1;

/// `mathlib.h:35` -- `M_PI / 180.0`.
const M_PI_DIV_180: f64 = core::f64::consts::PI / 180.0;

/// `DEG2RAD (a)` for a `float` operand: the product is `double`, and the
/// `cosf`/`sinf` argument conversion narrows it back.
fn deg2rad_f(a: f32) -> f32 {
    (f64::from(a) * M_PI_DIV_180) as f32
}

/// `gl_rlight.c:26` -- no user outside this file.
static mut R_DLIGHTFRAMECOUNT: c_int = 0;

/// `gl_rlight.c:30` -- dlight origins in the space of the model currently
/// having its lightmaps built (`r_brush.c` reads and rewrites it).
#[no_mangle]
pub static mut lightmap_dlight_origins: [Vec3; MAX_DLIGHTS] = [[0.0; 3]; MAX_DLIGHTS];

/// `DotProduct (v, vecs[i])` against a `texinfo` row (`float` arithmetic,
/// left to right, no contraction).
fn dot_vec4(v: &Vec3, row: &[f32; 4]) -> f32 {
    v[0] * row[0] + v[1] * row[1] + v[2] * row[2]
}

/// `DoublePrecisionDotProduct (x, y)` (`mathlib.h:62`).
fn double_precision_dot(x: &Vec3, y: &[f32; 4]) -> f64 {
    f64::from(x[0]) * f64::from(y[0])
        + f64::from(x[1]) * f64::from(y[1])
        + f64::from(x[2]) * f64::from(y[2])
}

/// `R_AnimateLight`.
///
/// # Safety
/// Main thread: `cl`, `cl_lightstyle` and `d_lightstylevalue` are
/// unsynchronised globals.
#[no_mangle]
pub unsafe extern "C" fn R_AnimateLight() {
    // SAFETY: the caller's contract.
    unsafe {
        //
        // light animations
        // 'm' is normal light, 'a' is no light, 'z' is double bright
        let f: f64 = (*ptr::addr_of!(cl)).time * 10.0;
        let i = f as c_int;
        let flat = (*ptr::addr_of!(c::render::r_flatlightstyles)).value;
        let dynamic = (*ptr::addr_of!(c::render::r_dynamic)).value;
        let gpu = (*ptr::addr_of!(c::render::r_gpulightmapupdate)).value;
        let lerp = (*ptr::addr_of!(c::render::r_lerplightstyles)).value;
        let styles = ptr::addr_of!(c::cl_parse::cl_lightstyle);
        let values = ptr::addr_of_mut!(c::render::d_lightstylevalue);
        for j in 0..MAX_LIGHTSTYLES {
            let ls = &(*styles)[j];
            if ls.length == 0 {
                (*values)[j] = 256; // should be 264 ?
                continue;
            }
            // johnfitz -- r_flatlightstyles
            let (k, mut n): (c_int, c_int);
            if flat == 2.0 {
                k = c_int::from(ls.peak) - c_int::from(b'a');
                n = k;
            } else if flat == 1.0 || dynamic == 0.0 {
                k = c_int::from(ls.average) - c_int::from(b'a');
                n = k;
            } else {
                k = c_int::from(ls.map[(i % ls.length) as usize]) - c_int::from(b'a');
                n = c_int::from(ls.map[((i + 1) % ls.length) as usize]) - c_int::from(b'a');
            }
            if gpu == 0.0
                || lerp == 0.0
                || (lerp < 2.0 && (n - k).abs() >= (c_int::from(b'm') - c_int::from(b'a')) / 2)
            {
                n = k;
            }
            (*values)[j] = ((f64::from(k) + f64::from(n - k) * (f - f64::from(i))) * 22.0) as c_int;
            // johnfitz
        }
    }
}

/// `R_MarkLights` -- johnfitz -- rewritten to use LordHavoc's lighting speedup.
///
/// # Safety
/// `light` is a live dlight, `node` a node of `cl.worldmodel`; main thread
/// (or the lightmap task that owns the surfaces it marks).
#[no_mangle]
pub unsafe extern "C" fn R_MarkLights(
    light: *mut c::cl_tent::dlight_t,
    num: c_int,
    node: *mut MNode,
) {
    // SAFETY: the caller's contract; every surface index is inside
    // `cl.worldmodel->surfaces`.
    unsafe {
        let mut node = node;
        let light = &*light;
        let dist: f32;
        loop {
            if (*node).contents < 0 {
                return;
            }

            let splitplane = &*(*node).plane;
            let d = if splitplane.type_ < 3 {
                light.origin[splitplane.type_ as usize] - splitplane.dist
            } else {
                dot_product(&light.origin, &splitplane.normal) - splitplane.dist
            };

            if d > light.radius {
                node = (*node).children[0];
                continue;
            }
            if d < -light.radius {
                node = (*node).children[1];
                continue;
            }
            dist = d;
            break;
        }

        let maxdist = light.radius * light.radius;
        // mark the polygons
        let world = (*ptr::addr_of!(cl)).worldmodel;
        let mut surf: *mut MSurface = (*world).surfaces.add((*node).firstsurface as usize);
        let dlightframecount = *ptr::addr_of!(R_DLIGHTFRAMECOUNT);
        for _ in 0..(*node).numsurfaces {
            let s_ref = &mut *surf;
            let plane = &*s_ref.plane;
            let texinfo = &*s_ref.texinfo;
            let mut impact: Vec3 = [0.0; 3];
            for (j, out) in impact.iter_mut().enumerate() {
                *out = light.origin[j] - plane.normal[j] * dist;
            }
            // clamp center of light to corner and check brightness
            let mut l = dot_vec4(&impact, &texinfo.vecs[0]) + texinfo.vecs[0][3]
                - f32::from(s_ref.texturemins[0]);
            let mut s = (f64::from(l) + 0.5) as c_int;
            if s < 0 {
                s = 0;
            } else if s > c_int::from(s_ref.extents[0]) {
                s = c_int::from(s_ref.extents[0]);
            }
            s = (l - s as f32) as c_int;
            l = dot_vec4(&impact, &texinfo.vecs[1]) + texinfo.vecs[1][3]
                - f32::from(s_ref.texturemins[1]);
            let mut t = (f64::from(l) + 0.5) as c_int;
            if t < 0 {
                t = 0;
            } else if t > c_int::from(s_ref.extents[1]) {
                t = c_int::from(s_ref.extents[1]);
            }
            t = (l - t as f32) as c_int;
            // compare to minimum light
            if ((s * s + t * t) as f32 + dist * dist) < maxdist {
                let word = (num >> 5) as usize;
                let bit = 1u32 << (num & 31);
                if s_ref.dlightframe != dlightframecount {
                    // not dynamic until now
                    s_ref.dlightbits[word] = bit;
                    s_ref.dlightframe = dlightframecount;
                } else {
                    // already dynamic
                    s_ref.dlightbits[word] |= bit;
                }
            }
            surf = surf.add(1);
        }

        if (*(*node).children[0]).contents >= 0 {
            R_MarkLights(ptr::from_ref(light).cast_mut(), num, (*node).children[0]);
        }
        if (*(*node).children[1]).contents >= 0 {
            R_MarkLights(ptr::from_ref(light).cast_mut(), num, (*node).children[1]);
        }
    }
}

/// `R_PushDlights`.
///
/// # Safety
/// Main thread with `cl.worldmodel` loaded.
#[no_mangle]
pub unsafe extern "C" fn R_PushDlights() {
    // SAFETY: the caller's contract.
    unsafe {
        R_DLIGHTFRAMECOUNT = *ptr::addr_of!(c::render::r_framecount);

        let cl_time = (*ptr::addr_of!(cl)).time;
        let nodes = (*(*ptr::addr_of!(cl)).worldmodel).nodes;
        let dlights = ptr::addr_of_mut!(c::cl_main::cl_dlights).cast::<c::cl_tent::dlight_t>();
        for i in 0..MAX_DLIGHTS {
            let l = dlights.add(i);
            if f64::from((*l).die) < cl_time || (*l).radius == 0.0 {
                continue;
            }
            (*ptr::addr_of_mut!(lightmap_dlight_origins))[i] = (*l).origin;
            R_MarkLights(l, i as c_int, nodes);
        }
    }
}

/*
=============================================================================

2021 RERELEASE DYNAMIC LIGHT ENTITIES

Dimension of the Machine places "dynamiclight" entities for shadow casting
dynamic lights; see gl_rlight.c:182-208 for the KEX background. They are
tied to the ray traced occlusion path here as well.

=============================================================================
*/

/// `gl_rlight.c:219` `entity_dlight_t`.
#[derive(Clone, Copy)]
struct EntityDlight {
    origin: Vec3,
    color: Vec3,
    radius: f32,
    intensity: f32,
    cone_dir: Vec3,
    /// <= -1: not a spotlight
    cone_cos: f32,
    start_fade_distance: f32,
    end_fade_distance: f32,
    style: c_int,
}

const ENTITY_DLIGHT_ZERO: EntityDlight = EntityDlight {
    origin: [0.0; 3],
    color: [0.0; 3],
    radius: 0.0,
    intensity: 0.0,
    cone_dir: [0.0; 3],
    cone_cos: 0.0,
    start_fade_distance: 0.0,
    end_fade_distance: 0.0,
    style: 0,
};

static mut ENTITY_DLIGHTS: [EntityDlight; MAX_ENTITY_DLIGHTS] =
    [ENTITY_DLIGHT_ZERO; MAX_ENTITY_DLIGHTS];
static mut NUM_ENTITY_DLIGHTS: c_int = 0;

/// `strcmp (key, lit) == 0` for a NUL-terminated C buffer.
///
/// # Safety
/// `key` is NUL-terminated.
unsafe fn key_is(key: *const c_char, lit: &core::ffi::CStr) -> bool {
    // SAFETY: the caller's contract.
    unsafe { c::cl_main::strcmp(key.cast(), lit.as_ptr()) == 0 }
}

/// Copies `com_token` into `key`, dropping trailing spaces
/// (`gl_rlight.c:285-287`).
///
/// # Safety
/// Main thread, right after a `COM_Parse`.
unsafe fn copy_key(key: &mut [c_char; 128]) {
    // SAFETY: the caller's contract; `key` is NUL-terminated by `q_strlcpy`.
    unsafe {
        c::cl_main::q_strlcpy(key.as_mut_ptr(), c::COM_ThreadToken(), key.len());
        let mut len = c::menu::strlen(key.as_ptr());
        while len > 0 && key[len - 1] == b' ' as c_char {
            key[len - 1] = 0;
            len -= 1;
        }
    }
}

/// `R_ParseEntityDlights` -- called at map load.
///
/// Parses "dynamiclight" entities out of the entity lump. The spot direction
/// comes from an "angle" key or, in a second pass, the origin of the targeted
/// entity.
///
/// # Safety
/// Main-thread map load with `cl.worldmodel` loaded.
#[no_mangle]
pub unsafe extern "C" fn R_ParseEntityDlights() {
    let mut key = [0 as c_char; 128];
    let mut value = [0 as c_char; 4096];
    let mut targets = [[0 as c_char; 64]; MAX_ENTITY_DLIGHTS];
    let mut cone_angles = [0.0f32; MAX_ENTITY_DLIGHTS];

    // SAFETY: the caller's contract; `entities` is the NUL-terminated entity
    // lump and `COM_ThreadToken` is this thread's `com_token`.
    unsafe {
        let clp = ptr::addr_of!(cl);
        NUM_ENTITY_DLIGHTS = 0;
        if (*clp).worldmodel.is_null() || (*(*clp).worldmodel).entities.is_null() {
            return;
        }

        let coop_game = (*clp).maxclients > 1 && (*clp).gametype == GAME_COOP;
        let mut any_targets = false;
        let lights = ptr::addr_of_mut!(ENTITY_DLIGHTS);

        let mut data: *const c_char = (*(*clp).worldmodel).entities;
        while NUM_ENTITY_DLIGHTS < MAX_ENTITY_DLIGHTS as c_int {
            data = c::COM_Parse(data);
            if data.is_null() || *c::COM_ThreadToken() != b'{' as c_char {
                break;
            }

            let idx = NUM_ENTITY_DLIGHTS as usize;
            let l = &mut (*lights)[idx];
            *l = ENTITY_DLIGHT_ZERO;
            l.color = [1.0, 1.0, 1.0];
            l.radius = 300.0;
            l.intensity = 10.0;
            l.cone_cos = -2.0;

            let mut is_dynamiclight = false;
            let mut has_angle = false;
            let mut angle = 0.0f32;
            let mut cone_angle = 0.0f32;
            let mut spawnflags: c_int = 0;
            targets[idx][0] = 0;

            loop {
                data = c::COM_Parse(data);
                if data.is_null() {
                    return;
                }
                if *c::COM_ThreadToken() == b'}' as c_char {
                    break;
                }
                copy_key(&mut key);
                data = c::progs_edict_dispatch::COM_ParseEx(data, CPE_ALLOWTRUNC);
                if data.is_null() {
                    return;
                }
                c::cl_main::q_strlcpy(value.as_mut_ptr(), c::COM_ThreadToken(), value.len());
                let k = key.as_ptr();
                let v = value.as_ptr();

                if key_is(k, c"classname") {
                    is_dynamiclight = c::cl_main::strcmp(v.cast(), c"dynamiclight".as_ptr()) == 0;
                } else if key_is(k, c"origin") {
                    c::cl_demo::sscanf(
                        v,
                        c"%f %f %f".as_ptr(),
                        ptr::addr_of_mut!(l.origin[0]),
                        ptr::addr_of_mut!(l.origin[1]),
                        ptr::addr_of_mut!(l.origin[2]),
                    );
                } else if key_is(k, c"_color") {
                    c::cl_demo::sscanf(
                        v,
                        c"%f %f %f".as_ptr(),
                        ptr::addr_of_mut!(l.color[0]),
                        ptr::addr_of_mut!(l.color[1]),
                        ptr::addr_of_mut!(l.color[2]),
                    );
                } else if key_is(k, c"_shadowlightradius") {
                    l.radius = c::host_cmd::atof(v) as f32;
                } else if key_is(k, c"_shadowlightintensity") {
                    l.intensity = c::host_cmd::atof(v) as f32;
                } else if key_is(k, c"_shadowlightconeangle") {
                    cone_angle = c::host_cmd::atof(v) as f32;
                } else if key_is(k, c"_shadowlightstartfadedistance") {
                    l.start_fade_distance = c::host_cmd::atof(v) as f32;
                } else if key_is(k, c"_shadowlightendfadedistance") {
                    l.end_fade_distance = c::host_cmd::atof(v) as f32;
                } else if key_is(k, c"_shadowlightstyle") {
                    l.style = c::cl_demo::atoi(v).clamp(0, MAX_LIGHTSTYLES as c_int - 1);
                } else if key_is(k, c"angle") {
                    angle = c::host_cmd::atof(v) as f32;
                    has_angle = true;
                } else if key_is(k, c"target") {
                    c::cl_main::q_strlcpy(targets[idx].as_mut_ptr(), v, targets[idx].len());
                } else if key_is(k, c"spawnflags") {
                    spawnflags = c::cl_demo::atoi(v);
                }
            }

            if !is_dynamiclight {
                continue;
            }
            if coop_game && (spawnflags & 1) != 0 {
                // DYNAMICLIGHT_NOT_IN_COOP
                continue;
            }

            if has_angle && cone_angle > 0.0 {
                if angle == -1.0 {
                    // up
                    l.cone_dir = [0.0, 0.0, 1.0];
                } else if angle == -2.0 {
                    // down
                    l.cone_dir = [0.0, 0.0, -1.0];
                } else {
                    l.cone_dir[0] = c::libm::cosf(deg2rad_f(angle));
                    l.cone_dir[1] = c::libm::sinf(deg2rad_f(angle));
                    l.cone_dir[2] = 0.0;
                }
                // _shadowlightconeangle is the full apex angle: KEX visibly lights surfaces
                // sideways from the axis (e.g. the fan walls), impossible with a half angle read
                l.cone_cos = c::libm::cosf(deg2rad_f(cone_angle));
            }
            cone_angles[idx] = cone_angle;
            any_targets = any_targets || targets[idx][0] != 0;

            NUM_ENTITY_DLIGHTS += 1;
        }

        if NUM_ENTITY_DLIGHTS > 0 {
            c::Con_DPrintf(c"%d entity dlights\n".as_ptr(), NUM_ENTITY_DLIGHTS);
        }

        if !any_targets {
            return;
        }

        // resolve spot directions from targeted entities, "target" wins over "angle"
        data = (*(*clp).worldmodel).entities;
        loop {
            data = c::COM_Parse(data);
            if data.is_null() || *c::COM_ThreadToken() != b'{' as c_char {
                break;
            }

            let mut targetname = [0 as c_char; 64];
            let mut origin: Vec3 = [0.0; 3];

            loop {
                data = c::COM_Parse(data);
                if data.is_null() {
                    return;
                }
                if *c::COM_ThreadToken() == b'}' as c_char {
                    break;
                }
                copy_key(&mut key);
                data = c::progs_edict_dispatch::COM_ParseEx(data, CPE_ALLOWTRUNC);
                if data.is_null() {
                    return;
                }
                c::cl_main::q_strlcpy(value.as_mut_ptr(), c::COM_ThreadToken(), value.len());

                if key_is(key.as_ptr(), c"targetname") {
                    c::cl_main::q_strlcpy(
                        targetname.as_mut_ptr(),
                        value.as_ptr(),
                        targetname.len(),
                    );
                } else if key_is(key.as_ptr(), c"origin") {
                    c::cl_demo::sscanf(
                        value.as_ptr(),
                        c"%f %f %f".as_ptr(),
                        ptr::addr_of_mut!(origin[0]),
                        ptr::addr_of_mut!(origin[1]),
                        ptr::addr_of_mut!(origin[2]),
                    );
                }
            }

            if targetname[0] == 0 {
                continue;
            }
            for i in 0..NUM_ENTITY_DLIGHTS as usize {
                if cone_angles[i] <= 0.0
                    || c::cl_main::strcmp(targets[i].as_ptr(), targetname.as_ptr()) != 0
                {
                    continue;
                }
                let l = &mut (*lights)[i];
                let mut dir: Vec3 = [0.0; 3];
                vector_subtract(&origin, &l.origin, &mut dir);
                if vector_length(&dir) > 0.0 {
                    vector_normalize(&mut dir);
                    l.cone_dir = dir;
                    l.cone_cos = c::libm::cosf(deg2rad_f(cone_angles[i]));
                }
            }
        }
    }
}

/// `R_UpdateEntityDlights` -- called every frame, keeps the parsed entity
/// dlights alive.
///
/// # Safety
/// Main thread inside the frame (`cl`, `r_refdef`, `cl_dlights` live).
#[no_mangle]
pub unsafe extern "C" fn R_UpdateEntityDlights() {
    // KEX ties these lights to its shadow system (r_staticshadows 0 removes them
    // entirely), so require the ray traced occlusion path here as well. They light
    // whole rooms that get retraced every frame, which is much more expensive than
    // the transient dlights, so r_rtshadows 1 (low) leaves them off
    // Plain field read of the exported static rather than `with_ctx`: this
    // runs every client frame, headless included, where no `VkDevice` exists
    // and `with_ctx` would `Sys_Error` loading it.
    // SAFETY: `vulkan_globals` is live for the whole process and only this
    // field is read (no reference to the struct is formed).
    let ray_query = unsafe { (*ptr::addr_of!(vulkan_globals)).ray_query };
    // SAFETY: the caller's contract.
    unsafe {
        if !ray_query
            || (*ptr::addr_of!(c::menu::r_rtshadows)).value < 2.0
            || (*ptr::addr_of!(c::render::r_gpulightmapupdate)).value == 0.0
        {
            return;
        }

        let mut vieworg: Vec3 = [0.0; 3];
        c::render::VID_Glue_ViewOrg(vieworg.as_mut_ptr());
        let scale = (*ptr::addr_of!(c::render::r_entdlightscale)).value;
        let cl_time = (*ptr::addr_of!(cl)).time;
        let values = ptr::addr_of!(c::render::d_lightstylevalue);
        let lights = ptr::addr_of!(ENTITY_DLIGHTS);
        for i in 0..NUM_ENTITY_DLIGHTS as usize {
            let l = &(*lights)[i];
            let mut intensity = ((*values)[l.style as usize] as f32 / 256.0) * l.intensity * scale;
            if l.end_fade_distance > 0.0 {
                let mut offset: Vec3 = [0.0; 3];
                vector_subtract(&l.origin, &vieworg, &mut offset);
                let view_distance = vector_length(&offset);
                if view_distance >= l.end_fade_distance {
                    continue;
                }
                if view_distance > l.start_fade_distance
                    && l.end_fade_distance > l.start_fade_distance
                {
                    intensity *= (l.end_fade_distance - view_distance)
                        / (l.end_fade_distance - l.start_fade_distance);
                }
            }
            if intensity <= 0.0 {
                continue;
            }

            let dl = &mut *c::cl_tent::CL_AllocDlight(ENTITY_DLIGHT_KEY + i as c_int);
            dl.origin = l.origin;
            dl.radius = l.radius;
            dl.die = (cl_time + f64::from(0.001f32)) as f32;
            dl.color = l.color;
            dl.cone_dir = l.cone_dir;
            dl.cone_cos = l.cone_cos;
            dl.kex_intensity = intensity;
        }
    }
}

/*
=============================================================================

LIGHT SAMPLING

=============================================================================
*/

/// `InterpolateLightmap`.
///
/// # Safety
/// `surf` has `samples` for every style it lists; `ds`/`dt` are inside its
/// extents.
unsafe fn interpolate_lightmap(color: &mut Vec3, surf: &MSurface, ds: c_int, dt: c_int) {
    let dsfrac = ds & 15;
    let dtfrac = dt & 15;
    let (mut r00, mut g00, mut b00, mut r01, mut g01, mut b01) =
        (0i32, 0i32, 0i32, 0i32, 0i32, 0i32);
    let (mut r10, mut g10, mut b10, mut r11, mut g11, mut b11) =
        (0i32, 0i32, 0i32, 0i32, 0i32, 0i32);
    let ext0 = c_int::from(surf.extents[0]);
    let ext1 = c_int::from(surf.extents[1]);
    let line3 = ((ext0 >> 4) + 1) * 3;

    // SAFETY: the caller's contract; `lightmap` stays inside `samples`.
    unsafe {
        let mut lightmap: *const u8 = surf
            .samples
            .offset((((dt >> 4) * ((ext0 >> 4) + 1) + (ds >> 4)) * 3) as isize); // LordHavoc: *3 for color
        let values = ptr::addr_of!(c::render::d_lightstylevalue);

        let mut maps = 0;
        while maps < surf.styles.len() && surf.styles[maps] != 255 {
            let scale = (*values)[surf.styles[maps] as usize];
            let px = |o: c_int| i32::from(*lightmap.offset(o as isize)) * scale;
            r00 += px(0);
            g00 += px(1);
            b00 += px(2);
            r01 += px(3);
            g01 += px(4);
            b01 += px(5);
            r10 += px(line3);
            g10 += px(line3 + 1);
            b10 += px(line3 + 2);
            r11 += px(line3 + 3);
            g11 += px(line3 + 4);
            b11 += px(line3 + 5);
            lightmap = lightmap.offset((((ext0 >> 4) + 1) * ((ext1 >> 4) + 1) * 3) as isize); // LordHavoc: *3 for colored lighting
            maps += 1;
        }
    }

    let lerp = |c11: i32, c10: i32, c01: i32, c00: i32| -> f32 {
        let a = (((c11 - c10) * dsfrac) >> 4) + c10;
        let b = (((c01 - c00) * dsfrac) >> 4) + c00;
        ((((a - b) * dtfrac) >> 4) + b) as f32 * (1.0f32 / 256.0f32)
    };
    color[0] = lerp(r11, r10, r01, r00);
    color[1] = lerp(g11, g10, g01, g00);
    color[2] = lerp(b11, b10, b01, b00);
}

/// `RecursiveLightPoint` -- johnfitz -- replaced entire function for lit
/// support via lordhavoc.
///
/// # Safety
/// `node` is a node of `cl.worldmodel`; `cache` is writable.
unsafe fn recursive_light_point(
    cache: &mut LightCache,
    node: *mut MNode,
    rayorg: &Vec3,
    start: &Vec3,
    end: &Vec3,
    maxdist: &mut f32,
) -> bool {
    // SAFETY: the caller's contract.
    unsafe {
        let mut node = node;
        let (front, back): (f32, f32);
        loop {
            if (*node).contents < 0 {
                return false; // didn't hit anything
            }

            // calculate mid point
            let plane = &*(*node).plane;
            let (f, b) = if plane.type_ < 3 {
                let t = plane.type_ as usize;
                (start[t] - plane.dist, end[t] - plane.dist)
            } else {
                (
                    dot_product(start, &plane.normal) - plane.dist,
                    dot_product(end, &plane.normal) - plane.dist,
                )
            };

            // LordHavoc: optimized recursion
            if (b < 0.0) == (f < 0.0) {
                node = (*node).children[usize::from(f < 0.0)];
                continue;
            }
            front = f;
            back = b;
            break;
        }

        let frac = front / (front - back);
        let mid: Vec3 = [
            start[0] + (end[0] - start[0]) * frac,
            start[1] + (end[1] - start[1]) * frac,
            start[2] + (end[2] - start[2]) * frac,
        ];

        // go down front side
        if recursive_light_point(
            cache,
            (*node).children[usize::from(front < 0.0)],
            rayorg,
            start,
            &mid,
            maxdist,
        ) {
            return true; // hit something
        }

        let world = (*ptr::addr_of!(cl)).worldmodel;
        let surfaces = (*world).surfaces;
        let mut surf = surfaces.add((*node).firstsurface as usize);
        for _ in 0..(*node).numsurfaces {
            let s = &*surf;
            surf = surf.add(1);

            if s.flags & SURF_DRAWTILED != 0 {
                continue; // no lightmaps
            }

            let texinfo = &*s.texinfo;
            // ericw -- added double casts to force 64-bit precision.
            // Without them the zombie at the start of jam3_ericw.bsp was
            // incorrectly being lit up in SSE builds.
            let mut ds = (double_precision_dot(&mid, &texinfo.vecs[0])
                + f64::from(texinfo.vecs[0][3])) as c_int;
            let mut dt = (double_precision_dot(&mid, &texinfo.vecs[1])
                + f64::from(texinfo.vecs[1][3])) as c_int;

            if ds < c_int::from(s.texturemins[0]) || dt < c_int::from(s.texturemins[1]) {
                continue;
            }

            ds -= c_int::from(s.texturemins[0]);
            dt -= c_int::from(s.texturemins[1]);

            if ds > c_int::from(s.extents[0]) || dt > c_int::from(s.extents[1]) {
                continue;
            }

            let plane = &*s.plane;
            let (sfront, sback) = if plane.type_ < 3 {
                let t = plane.type_ as usize;
                (rayorg[t] - plane.dist, end[t] - plane.dist)
            } else {
                (
                    dot_product(rayorg, &plane.normal) - plane.dist,
                    dot_product(end, &plane.normal) - plane.dist,
                )
            };
            let mut raydelta: Vec3 = [0.0; 3];
            vector_subtract(end, rayorg, &mut raydelta);
            let mut dist = sfront / (sfront - sback) * vector_length(&raydelta);

            if s.samples.is_null() {
                // We hit a surface that is flagged as lightmapped, but doesn't have actual lightmap info.
                // Instead of just returning black, we'll keep looking for nearby surfaces that do have valid samples.
                // This fixes occasional pitch-black models in otherwise well-lit areas in DOTM (e.g. mge1m1, mge4m1)
                // caused by overlapping surfaces with mixed lighting data.
                let nearby = 8.0f32;
                dist += nearby;
                if dist < *maxdist {
                    *maxdist = dist;
                }
                continue;
            }

            if dist < *maxdist {
                cache.surfidx = (ptr::from_ref(s).offset_from(surfaces) + 1) as c_int;
                cache.ds = ds as i16;
                cache.dt = dt as i16;
            } else {
                cache.surfidx = -1;
            }

            return true; // success
        }

        // go down back side
        recursive_light_point(
            cache,
            (*node).children[usize::from(front >= 0.0)],
            rayorg,
            &mid,
            end,
            maxdist,
        )
    }
}

/// `R_LightPoint` -- johnfitz -- replaced entire function for lit support via
/// lordhavoc.
///
/// # Safety
/// `p` is three floats, `lightcolor` three writable floats, `cache` a live
/// `lightcache_t`; `cl.worldmodel` is loaded.
#[no_mangle]
pub unsafe extern "C" fn R_LightPoint(
    p: *mut c_float,
    ofs: c_float,
    cache: *mut LightCache,
    lightcolor: *mut [c_float; 3],
) -> c_int {
    // SAFETY: the caller's contract.
    unsafe {
        let p: &Vec3 = &*p.cast::<Vec3>();
        let lightcolor = &mut *lightcolor;
        let world = (*ptr::addr_of!(cl)).worldmodel;
        let mut maxdist = 8192.0f32; // johnfitz -- was 2048

        if (*world).lightdata.is_null() {
            *lightcolor = [255.0; 3];
            return 255;
        }

        let start: Vec3 = [p[0], p[1], p[2] + ofs];
        let end: Vec3 = [start[0], start[1], start[2] - maxdist];

        *lightcolor = [0.0; 3];

        if (*cache).surfidx <= 0 // no cache or pitch black
            || (*cache).surfidx > (*world).numsurfaces
            || ((*cache).pos[0] - p[0]).abs() >= 1.0
            || ((*cache).pos[1] - p[1]).abs() >= 1.0
            || ((*cache).pos[2] - p[2]).abs() >= 1.0
        {
            (*cache).surfidx = 0;
            (*cache).pos = *p;
            recursive_light_point(
                &mut *cache,
                (*world).nodes,
                &start,
                &start,
                &end,
                &mut maxdist,
            );
        }

        if !cache.is_null() && (*cache).surfidx > 0 {
            let surf = &*(*world).surfaces.add(((*cache).surfidx - 1) as usize);
            interpolate_lightmap(
                lightcolor,
                surf,
                c_int::from((*cache).ds),
                c_int::from((*cache).dt),
            );
        }

        ((lightcolor[0] + lightcolor[1] + lightcolor[2]) * (1.0f32 / 3.0f32)) as c_int
    }
}
