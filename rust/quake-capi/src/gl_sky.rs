//! `gl_sky.c` -- sky textures, skyboxes and the sky pass (Rust migration
//! Phase 8 M7).
//!
//! Every function of `gl_sky.c` lives here except the console commands, the
//! `%g` formatting (`Sky_GetSkyCommand`, ADR-005 gap), the cvar definitions
//! and `Sky_Init`, which stay in `Quake/gl_sky_glue.c`. The skybox record is
//! Rust-owned and exported as the C symbol `skybox` so that the glue TU's
//! commands keep writing the same object (ADR-007 dual view).

use core::ffi::{c_char, c_float, c_int, c_void};
use core::ptr;
use core::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use ash::vk;
use quake_c_sys as c;
use quake_math::mathlib::{angle_vectors, dot_product, vector_add, vector_subtract, Vec3};
use quake_render::cb::{self, CmdProcs};
use quake_types::host::{ClientState, Entity};
use quake_types::model_mem::{
    MSurface, QModel, Texture, MOD_BRUSH, SURF_DRAWSKY, SURF_PLANEBACK, TEXTYPE_SKY,
};
use quake_types::render::{BasicVertex, CbContext, GlTexture, VulkanPipeline};

use crate::gl_fog::{Fog_GetColor, Fog_GetDensity};
use crate::gl_rmisc::{device, vg, with_ctx, DYN};
use crate::gl_texmgr::{TexMgr_FreeTexture, TexMgr_LoadImage};

/// `gl_texmgr.h:54` -- `enum srcformat`.
const SRC_INDEXED: c_int = 0;
const SRC_RGBA: c_int = 2;
const SRC_RGBA_CUBEMAP: c_int = 4;
/// `gl_texmgr.h` -- `TEXPREF_NONE`, `TEXPREF_ALPHA`.
const TEXPREF_NONE: u32 = 0;
const TEXPREF_ALPHA: u32 = 0x0008;
/// `gl_model.h:75` -- `chain_world`.
const CHAIN_WORLD: usize = 0;
/// `gl_model.h` -- `VERTEXSIZE`.
const VERTEXSIZE: usize = 7;
/// `quakedef.h:64`, `glquake.h:56`, `protocol.h:219`.
const ON_EPSILON: f64 = 0.1;
const BACKFACE_EPSILON: f32 = 0.01;
const ENTALPHA_ZERO: u8 = 1;
/// `quakedef.h` -- `SIDE_FRONT`, `SIDE_BACK`, `SIDE_ON`.
const SIDE_FRONT: i32 = 0;
const SIDE_BACK: i32 = 1;
const SIDE_ON: i32 = 2;
/// `common.h` -- `CPE_ALLOWTRUNC`.
const CPE_ALLOWTRUNC: c_int = 1;
/// `mathlib.h:35` -- `M_PI / 180.0`.
const M_PI_DIV_180: f64 = core::f64::consts::PI / 180.0;
const MAX_OSPATH: usize = 1024;

extern "C" {
    static mut cl: ClientState;
}

/// `gl_model.h` -- `glpoly_t`; variable-sized, `verts` runs on past the
/// declared four entries. Only ever handled through raw pointers.
#[repr(C)]
struct GlPoly {
    next: *mut GlPoly,
    numverts: c_int,
    verts: [[f32; VERTEXSIZE]; 4],
}

/// `gl_sky.c:72` -- `skybox_t`, exported as the C symbol `skybox` for the
/// glue TU's `skywind*`/`sky` commands and `Sky_GetSkyCommand`.
#[repr(C)]
pub struct SkyBox {
    pub name: [c_char; 1024],
    pub name_worldspawn: [c_char; 1024],
    pub textures: [*mut GlTexture; 6],
    pub cubemap: *mut GlTexture,
    pub wind_dist: f32,
    pub wind_yaw: f32,
    pub wind_pitch: f32,
    pub wind_period: f32,
}

/// `gl_sky.c:84` -- `static skybox_t skybox` (non-static here: ADR-007 view).
#[no_mangle]
pub static mut skybox: SkyBox = SkyBox {
    name: [0; 1024],
    name_worldspawn: [0; 1024],
    textures: [ptr::null_mut(); 6],
    cubemap: ptr::null_mut(),
    wind_dist: 0.0,
    wind_yaw: 0.0,
    wind_pitch: 0.0,
    wind_period: 0.0,
};

/// `gl_sky.c:55` -- `static float skyfog` (exported for `Sky_GetSkyCommand`
/// and `R_SetSkyfog_f` in the glue TU).
#[no_mangle]
pub static mut skyfog: f32 = 0.0;

static mut SKYFLATCOLOR: [f32; 3] = [0.0; 3];
static mut SKYMINS: [[f32; 6]; 2] = [[0.0; 6]; 2];
static mut SKYMAXS: [[f32; 6]; 2] = [[0.0; 6]; 2];
static mut SOLIDSKYTEXTURE: *mut GlTexture = ptr::null_mut();
static mut ALPHASKYTEXTURE: *mut GlTexture = ptr::null_mut();
/// `gl_sky.c:59-60` -- `load_skytexture_mutex` guards `max_skytexture_index`,
/// `alphaskytexture` and `skyflatcolor` between the model-load workers.
static LOAD_SKYTEXTURE_MUTEX: Mutex<()> = Mutex::new(());
static mut MAX_SKYTEXTURE_INDEX: c_int = -1;
/// `gl_sky.c:62` -- `qboolean need_bounds` (no user outside this file).
static mut NEED_BOUNDS: bool = false;

/// `gl_sky.c:44` -- for skybox.
const SKYTEXORDER: [usize; 6] = [0, 2, 1, 3, 4, 5];
const SKYCLIP: [Vec3; 6] = [
    [1.0, 1.0, 0.0],
    [1.0, -1.0, 0.0],
    [0.0, -1.0, 1.0],
    [0.0, 1.0, 1.0],
    [1.0, 0.0, 1.0],
    [-1.0, 0.0, 1.0],
];
const ST_TO_VEC: [[i32; 3]; 6] = [
    [3, -1, 2],
    [-3, 1, 2],
    [1, 3, 2],
    [-1, -3, 2],
    [-2, -1, 3], // straight up
    [2, -1, -3], // straight down
];
const VEC_TO_ST: [[i32; 3]; 6] = [
    [-2, 3, 1],
    [2, 3, -1],
    [1, 3, 2],
    [-1, 3, -2],
    [-2, -1, 3],
    [-2, 1, -3],
];
/// `gl_sky.c:452`.
const SUF: [&core::ffi::CStr; 6] = [c"rt", c"bk", c"lf", c"ft", c"up", c"dn"];

/// `q_minmax.h` -- `clamp_f`.
fn clamp_f(minval: f32, val: f32, maxval: f32) -> f32 {
    if val < minval {
        minval
    } else if val > maxval {
        maxval
    } else {
        val
    }
}

fn skyflatcolor_from(r: u32, g: u32, b: u32, count: u32) -> [f32; 3] {
    let n = count.wrapping_mul(255) as f32;
    [r as f32 / n, g as f32 / n, b as f32 / n]
}

//==============================================================================
//
//  INIT
//
//==============================================================================

/// `Sky_LoadTexture` -- a sky texture is 256*128, with the left side being a
/// masked overlay.
///
/// # Safety
/// `mod_` is a live model and `mt` one of its miptex textures with
/// `width * height` indexed bytes following the struct; callable from the
/// model-load task workers (the shared state is mutex-guarded as in C).
#[no_mangle]
pub unsafe extern "C" fn Sky_LoadTexture(mod_: *mut QModel, mt: *mut Texture, tex_index: c_int) {
    // SAFETY: per the contract.
    unsafe {
        let mt = &mut *mt;
        let mut texturename = [0 as c_char; 64];
        if mt.width != 256 || mt.height != 128 {
            c::Con_Warning(
                c"Sky texture %s is %d x %d, expected 256 x 128\n".as_ptr(),
                mt.name.as_ptr(),
                mt.width as c_int,
                mt.height as c_int,
            );
            if mt.width < 2 || mt.height < 1 {
                return;
            }
        }

        let halfwidth = mt.width / 2;
        let back_data = c::Mem_Alloc((halfwidth * mt.height * 2) as usize).cast::<u8>();
        let front_data = back_data.add((halfwidth * mt.height) as usize);
        let mut src = (mt as *mut Texture).add(1).cast::<u8>();

        // extract back layer and upload
        for y in 0..mt.height as usize {
            ptr::copy_nonoverlapping(
                src.add(halfwidth as usize + y * mt.width as usize),
                back_data.add(y * halfwidth as usize),
                halfwidth as usize,
            );
        }

        c::cl_main::q_snprintf(
            texturename.as_mut_ptr(),
            texturename.len(),
            c"%s:%s_back".as_ptr(),
            (*mod_).name.as_ptr(),
            mt.name.as_ptr(),
        );
        SOLIDSKYTEXTURE = TexMgr_LoadImage(
            mod_.cast(),
            texturename.as_ptr(),
            halfwidth as c_int,
            mt.height as c_int,
            SRC_INDEXED,
            back_data,
            c"".as_ptr(),
            back_data as usize,
            TEXPREF_NONE,
        );

        // extract front layer and upload
        let (mut r, mut g, mut b, mut count) = (0u32, 0u32, 0u32, 0u32);
        let mut front = front_data;
        for _ in 0..mt.height {
            for x in 0..halfwidth as usize {
                let mut p = u32::from(*src.add(x));
                if p == 0 {
                    p = 255;
                } else {
                    let rgba = (*ptr::addr_of!(c::render::d_8to24table))[p as usize].to_ne_bytes();
                    r = r.wrapping_add(u32::from(rgba[0]));
                    g = g.wrapping_add(u32::from(rgba[1]));
                    b = b.wrapping_add(u32::from(rgba[2]));
                    count = count.wrapping_add(1);
                }
                *front.add(x) = p as u8;
            }
            src = src.add(mt.width as usize);
            front = front.add(halfwidth as usize);
        }

        c::cl_main::q_snprintf(
            texturename.as_mut_ptr(),
            texturename.len(),
            c"%s:%s_front".as_ptr(),
            (*mod_).name.as_ptr(),
            mt.name.as_ptr(),
        );

        // This is horrible but it matches the non-threaded behavior. Does this even make sense?
        {
            let _guard = LOAD_SKYTEXTURE_MUTEX
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if tex_index > MAX_SKYTEXTURE_INDEX {
                MAX_SKYTEXTURE_INDEX = tex_index;
                ALPHASKYTEXTURE = TexMgr_LoadImage(
                    mod_.cast(),
                    texturename.as_ptr(),
                    halfwidth as c_int,
                    mt.height as c_int,
                    SRC_INDEXED,
                    front_data,
                    c"".as_ptr(),
                    front_data as usize,
                    TEXPREF_ALPHA,
                );

                // calculate r_fastsky color based on average of all opaque foreground colors
                SKYFLATCOLOR = skyflatcolor_from(r, g, b, count);
            }
        }

        c::Mem_Free(back_data.cast());
    }
}

/// `Sky_LoadTextureQ64` -- Quake64 sky textures are 32*64.
///
/// # Safety
/// As [`Sky_LoadTexture`].
#[no_mangle]
pub unsafe extern "C" fn Sky_LoadTextureQ64(mod_: *mut QModel, mt: *mut Texture, tex_index: c_int) {
    // SAFETY: per the contract.
    unsafe {
        let mt = &mut *mt;
        let mut texturename = [0 as c_char; 64];
        if mt.width != 32 || mt.height != 64 {
            c::Con_DWarning(
                c"Q64 sky texture %s is %d x %d, expected 32 x 64\n".as_ptr(),
                mt.name.as_ptr(),
                mt.width as c_int,
                mt.height as c_int,
            );
            if mt.width < 1 || mt.height < 2 {
                return;
            }
        }

        // pointers to both layer textures
        let halfheight = mt.height / 2;
        let mut front = (mt as *mut Texture).add(1).cast::<u8>();
        let back = front.add((mt.width * halfheight) as usize);
        let front_rgba = c::Mem_Alloc((4 * mt.width * halfheight) as usize).cast::<u8>();

        // Normal indexed texture for the back layer
        c::cl_main::q_snprintf(
            texturename.as_mut_ptr(),
            texturename.len(),
            c"%s:%s_back".as_ptr(),
            (*mod_).name.as_ptr(),
            mt.name.as_ptr(),
        );
        SOLIDSKYTEXTURE = TexMgr_LoadImage(
            mod_.cast(),
            texturename.as_ptr(),
            mt.width as c_int,
            halfheight as c_int,
            SRC_INDEXED,
            back,
            c"".as_ptr(),
            back as usize,
            TEXPREF_NONE,
        );

        // front layer, convert to RGBA and upload
        let (mut p, mut r, mut g, mut b, mut count) = (0usize, 0u32, 0u32, 0u32, 0u32);
        for _ in 0..mt.width * halfheight {
            let rgba = (*ptr::addr_of!(c::render::d_8to24table))[usize::from(*front)].to_ne_bytes();
            front = front.add(1);

            // RGB
            *front_rgba.add(p) = rgba[0];
            *front_rgba.add(p + 1) = rgba[1];
            *front_rgba.add(p + 2) = rgba[2];
            // Alpha
            *front_rgba.add(p + 3) = 128; // this look ok to me!
            p += 4;

            // Fast sky
            r = r.wrapping_add(u32::from(rgba[0]));
            g = g.wrapping_add(u32::from(rgba[1]));
            b = b.wrapping_add(u32::from(rgba[2]));
            count = count.wrapping_add(1);
        }

        c::cl_main::q_snprintf(
            texturename.as_mut_ptr(),
            texturename.len(),
            c"%s:%s_front".as_ptr(),
            (*mod_).name.as_ptr(),
            mt.name.as_ptr(),
        );
        // This is horrible but it matches the non-threaded behavior. Does this even make sense?
        {
            let _guard = LOAD_SKYTEXTURE_MUTEX
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if tex_index > MAX_SKYTEXTURE_INDEX {
                MAX_SKYTEXTURE_INDEX = tex_index;
                if !ALPHASKYTEXTURE.is_null() {
                    TexMgr_FreeTexture(ALPHASKYTEXTURE);
                }

                ALPHASKYTEXTURE = TexMgr_LoadImage(
                    mod_.cast(),
                    texturename.as_ptr(),
                    mt.width as c_int,
                    halfheight as c_int,
                    SRC_RGBA,
                    front_rgba,
                    c"".as_ptr(),
                    front_rgba as usize,
                    TEXPREF_ALPHA,
                );
                // calculate r_fastsky color based on average of all opaque foreground colors
                SKYFLATCOLOR = skyflatcolor_from(r, g, b, count);
            }
        }

        c::Mem_Free(front_rgba.cast());
    }
}

/// `gl_sky.c:236` -- `Skywind_is_enabled` (the glue TU carries its own copy
/// for `Sky_SkyCommand_f`).
///
/// # Safety
/// Main thread: reads `skybox` and `r_skywind`.
unsafe fn skywind_is_enabled() -> bool {
    // SAFETY: per the contract.
    unsafe {
        let sb = &*ptr::addr_of!(skybox);
        sb.name[0] != 0
            && (*ptr::addr_of!(c::render::r_skywind)).value > 0.0
            && (sb.wind_dist != 0.0
                || sb.wind_period != 0.0
                || sb.wind_pitch != 0.0
                || sb.wind_yaw != 0.0)
    }
}

/// `gl_sky.c:247` -- `Skywind_Clear`.
///
/// # Safety
/// Main thread: writes `skybox`.
unsafe fn skywind_clear() {
    // SAFETY: per the contract.
    unsafe {
        let sb = &mut *ptr::addr_of_mut!(skybox);
        sb.wind_dist = 0.0;
        sb.wind_period = 0.0;
        sb.wind_pitch = 0.0;
        sb.wind_yaw = 0.0;
    }
}

/// `Sky_LoadSkyBox`.
///
/// # Safety
/// `name` is NUL-terminated; main thread with `cl.worldmodel` loaded (the
/// textures are owned by it).
#[no_mangle]
pub unsafe extern "C" fn Sky_LoadSkyBox(name: *const c_char) {
    // SAFETY: per the contract.
    unsafe {
        if *ptr::addr_of!(c::host::no_rendering) {
            return;
        }

        let sb = &mut *ptr::addr_of_mut!(skybox);
        if c::cl_main::strcmp(sb.name.as_ptr(), name) == 0 {
            return; // no change
        }

        let notexture = (*ptr::addr_of!(c::render::notexture)).cast::<GlTexture>();

        // purge old textures
        for tex in sb.textures.iter_mut() {
            if !tex.is_null() && *tex != notexture {
                TexMgr_FreeTexture(*tex);
            }
            *tex = ptr::null_mut();
        }
        if !sb.cubemap.is_null() {
            TexMgr_FreeTexture(sb.cubemap);
        }
        sb.cubemap = ptr::null_mut();

        // turn off skybox if sky is set to ""
        if *name == 0 {
            sb.name[0] = 0;
            return;
        }

        // load textures
        let mut width = [0 as c_int; 6];
        let mut height = [0 as c_int; 6];
        let mut fmt = [0 as c_int; 6];
        let mut filename = [[0 as c_char; MAX_OSPATH]; 6];
        let mut data = [ptr::null_mut::<u8>(); 6];
        let mut nonefound = true;
        let mut cubemap = true;
        for i in 0..6 {
            c::cl_main::q_snprintf(
                filename[i].as_mut_ptr(),
                MAX_OSPATH,
                c"gfx/env/%s%s".as_ptr(),
                name,
                SUF[i].as_ptr(),
            );
            data[i] = c::render::Image_LoadImage(
                filename[i].as_ptr(),
                &mut width[i],
                &mut height[i],
                &mut fmt[i],
                0,
            );
            if !data[i].is_null() {
                nonefound = false;
            }
            if data[i].is_null()
                || width[i] != height[i]
                || width[i] != width[0]
                || fmt[i] != SRC_RGBA
            {
                cubemap = false;
            }
        }

        let worldmodel = (*ptr::addr_of!(cl)).worldmodel;
        if cubemap {
            c::cl_main::q_snprintf(
                filename[0].as_mut_ptr(),
                MAX_OSPATH,
                c"gfx/env/%scube".as_ptr(),
                name,
            );
            sb.cubemap = TexMgr_LoadImage(
                worldmodel.cast(),
                filename[0].as_ptr(),
                width[0],
                height[0],
                SRC_RGBA_CUBEMAP,
                data.as_mut_ptr().cast::<u8>(),
                filename[0].as_ptr(),
                0,
                TEXPREF_NONE,
            );
        } else {
            for i in 0..6 {
                if !data[i].is_null() {
                    sb.textures[i] = TexMgr_LoadImage(
                        worldmodel.cast(),
                        filename[i].as_ptr(),
                        width[i],
                        height[i],
                        fmt[i],
                        data[i],
                        filename[i].as_ptr(),
                        0,
                        TEXPREF_NONE,
                    );
                } else {
                    c::Con_Printf(c"Couldn't load %s\n".as_ptr(), filename[i].as_ptr());
                    sb.textures[i] = notexture;
                }
            }
        }

        for d in data {
            if !d.is_null() {
                c::Mem_Free(d.cast());
            }
        }

        if nonefound {
            // go back to scrolling sky if skybox is totally missing
            for tex in sb.textures.iter_mut() {
                if !tex.is_null() && *tex != notexture {
                    TexMgr_FreeTexture(*tex);
                }
                *tex = ptr::null_mut();
            }
            sb.name[0] = 0;
            return;
        }

        c::cl_main::q_strlcpy(sb.name.as_mut_ptr(), name, sb.name.len());

        c::render::Skywind_Load_f();
    }
}

/// `Sky_ClearAll` -- called on map unload/game change to avoid keeping
/// pointers to freed data.
///
/// # Safety
/// Main thread, with no model-load task in flight.
#[no_mangle]
pub unsafe extern "C" fn Sky_ClearAll() {
    // SAFETY: per the contract.
    unsafe {
        let sb = &mut *ptr::addr_of_mut!(skybox);
        sb.name[0] = 0;
        sb.textures = [ptr::null_mut(); 6];
        sb.cubemap = ptr::null_mut();

        skywind_clear();

        SOLIDSKYTEXTURE = ptr::null_mut();
        ALPHASKYTEXTURE = ptr::null_mut();
        MAX_SKYTEXTURE_INDEX = -1;
        let r_skyfog = ptr::addr_of_mut!(c::render::r_skyfog);
        c::Cvar_SetQuick(r_skyfog, (*r_skyfog).default_string);
    }
}

/// `Sky_NewMap`.
///
/// # Safety
/// Main thread with `cl.worldmodel` loaded.
#[no_mangle]
pub unsafe extern "C" fn Sky_NewMap() {
    let mut key = [0 as c_char; 128];
    let mut value = [0 as c_char; 4096];

    // SAFETY: per the contract.
    unsafe {
        skyfog = (*ptr::addr_of!(c::render::r_skyfog)).value;

        //
        // read worldspawn (this is so ugly, and shouldn't it be done on the server?)
        //
        let mut data = (*(*ptr::addr_of!(cl)).worldmodel).entities.cast_const();
        if data.is_null() {
            return; // FIXME: how could this possibly ever happen? -- if there's no
        }
        // worldspawn then the sever wouldn't send the loadmap message to the client

        data = c::COM_Parse(data);
        if data.is_null() {
            return; // error
        }
        if *c::COM_ThreadToken() != b'{' as c_char {
            return; // error
        }
        loop {
            data = c::COM_Parse(data);
            if data.is_null() {
                return; // error
            }
            let token = c::COM_ThreadToken();
            if *token == b'}' as c_char {
                break; // end of worldspawn
            }
            if *token == b'_' as c_char {
                c::cl_main::q_strlcpy(key.as_mut_ptr(), token.add(1), key.len());
            } else {
                c::cl_main::q_strlcpy(key.as_mut_ptr(), token, key.len());
            }
            // remove trailing spaces
            let mut len = c::menu::strlen(key.as_ptr());
            while len > 0 && key[len - 1] == b' ' as c_char {
                key[len - 1] = 0;
                len -= 1;
            }
            data = c::progs_edict_dispatch::COM_ParseEx(data, CPE_ALLOWTRUNC);
            if data.is_null() {
                return; // error
            }
            c::cl_main::q_strlcpy(value.as_mut_ptr(), c::COM_ThreadToken(), value.len());

            if c::cl_main::strcmp(c"sky".as_ptr(), key.as_ptr()) == 0 {
                Sky_LoadSkyBox(value.as_ptr());
            }

            if c::cl_main::strcmp(c"skyfog".as_ptr(), key.as_ptr()) == 0 {
                skyfog = c::host_cmd::atof(value.as_ptr()) as f32;
            }
            // also accept non-standard keys
            else if c::cl_main::strcmp(c"skyname".as_ptr(), key.as_ptr()) == 0 {
                // half-life
                Sky_LoadSkyBox(value.as_ptr());
            } else if c::cl_main::strcmp(c"qlsky".as_ptr(), key.as_ptr()) == 0 {
                // quake lives
                Sky_LoadSkyBox(value.as_ptr());
            }
        }

        let sb = &mut *ptr::addr_of_mut!(skybox);
        c::cl_main::q_strlcpy(
            sb.name_worldspawn.as_mut_ptr(),
            sb.name.as_ptr(),
            sb.name_worldspawn.len(),
        );
    }
}

/// `Sky_SetSkyfog`.
///
/// # Safety
/// Main thread.
#[no_mangle]
pub unsafe extern "C" fn Sky_SetSkyfog(value: c_float) {
    // SAFETY: per the contract.
    unsafe {
        skyfog = value;
    }
}

//==============================================================================
//
//  PROCESS SKY SURFS
//
//==============================================================================

/// `Sky_ProjectPoly` -- update sky bounds.
///
/// # Safety
/// `vecs` holds `nump` `vec3_t`s; main thread (writes the sky bounds).
#[no_mangle]
pub unsafe extern "C" fn Sky_ProjectPoly(nump: c_int, vecs: *mut c_float) {
    // SAFETY: per the contract.
    unsafe {
        let verts = core::slice::from_raw_parts(vecs, nump.max(0) as usize * 3);
        project_poly(verts);
    }
}

/// `Sky_ProjectPoly` over a `vec3_t` run.
///
/// # Safety
/// Main thread (writes the sky bounds).
unsafe fn project_poly(vecs: &[f32]) {
    // decide which face it maps to
    let mut v: Vec3 = [0.0; 3];
    for vp in vecs.chunks_exact(3) {
        let vp: Vec3 = [vp[0], vp[1], vp[2]];
        let t = v;
        vector_add(&vp, &t, &mut v);
    }
    let av = [v[0].abs(), v[1].abs(), v[2].abs()];
    let axis = if av[0] > av[1] && av[0] > av[2] {
        if v[0] < 0.0 {
            1
        } else {
            0
        }
    } else if av[1] > av[2] && av[1] > av[0] {
        if v[1] < 0.0 {
            3
        } else {
            2
        }
    } else if v[2] < 0.0 {
        5
    } else {
        4
    };

    // project new texture coords
    for vecs in vecs.chunks_exact(3) {
        let mut j = VEC_TO_ST[axis][2];
        let dv = if j > 0 {
            vecs[(j - 1) as usize]
        } else {
            -vecs[(-j - 1) as usize]
        };

        j = VEC_TO_ST[axis][0];
        let s = if j < 0 {
            -vecs[(-j - 1) as usize] / dv
        } else {
            vecs[(j - 1) as usize] / dv
        };
        j = VEC_TO_ST[axis][1];
        let t = if j < 0 {
            -vecs[(-j - 1) as usize] / dv
        } else {
            vecs[(j - 1) as usize] / dv
        };

        // SAFETY: per the contract.
        unsafe {
            let mins = &mut *ptr::addr_of_mut!(SKYMINS);
            let maxs = &mut *ptr::addr_of_mut!(SKYMAXS);
            if s < mins[0][axis] {
                mins[0][axis] = s;
            }
            if t < mins[1][axis] {
                mins[1][axis] = t;
            }
            if s > maxs[0][axis] {
                maxs[0][axis] = s;
            }
            if t > maxs[1][axis] {
                maxs[1][axis] = t;
            }
        }
    }
}

/// `gl_sky.c:800` -- `Sky_ClipPoly`. `vecs` holds `nump` `vec3_t`s plus one
/// spare (C's `MAX_CLIP_VERTS = nump + 2` scratch).
///
/// # Safety
/// Main thread (writes the sky bounds).
unsafe fn clip_poly(nump: usize, vecs: &mut Vec<f32>, stage: usize) {
    if stage == 6 {
        // fully clipped
        // SAFETY: per the contract.
        unsafe { project_poly(&vecs[..nump * 3]) };
        return;
    }

    let mut front = false;
    let mut back = false;
    let mut sides = vec![0i32; nump + 2];
    let mut dists = vec![0f32; nump + 2];

    for i in 0..nump {
        let v: Vec3 = [vecs[i * 3], vecs[i * 3 + 1], vecs[i * 3 + 2]];
        let d = dot_product(&v, &SKYCLIP[stage]);

        if f64::from(d) > ON_EPSILON {
            front = true;
            sides[i] = SIDE_FRONT;
        } else if f64::from(d) < ON_EPSILON {
            back = true;
            sides[i] = SIDE_BACK;
        } else {
            sides[i] = SIDE_ON;
        }
        dists[i] = d;
    }

    if !front || !back {
        // not clipped
        // SAFETY: per the contract.
        unsafe { clip_poly(nump, vecs, stage + 1) };
        return;
    }

    // clip it
    sides[nump] = sides[0];
    dists[nump] = dists[0];
    vecs.resize((nump + 2) * 3, 0.0);
    let (v0, v1, v2) = (vecs[0], vecs[1], vecs[2]);
    vecs[nump * 3] = v0;
    vecs[nump * 3 + 1] = v1;
    vecs[nump * 3 + 2] = v2;
    let mut newc = [0usize; 2];

    let mut newv_0 = vec![0f32; (nump + 2) * 3];
    let mut newv_1 = vec![0f32; (nump + 2) * 3];

    for i in 0..nump {
        let v = &vecs[i * 3..i * 3 + 6];
        match sides[i] {
            SIDE_FRONT => {
                newv_0[newc[0] * 3..newc[0] * 3 + 3].copy_from_slice(&v[..3]);
                newc[0] += 1;
            }
            SIDE_BACK => {
                newv_1[newc[1] * 3..newc[1] * 3 + 3].copy_from_slice(&v[..3]);
                newc[1] += 1;
            }
            _ => {
                newv_0[newc[0] * 3..newc[0] * 3 + 3].copy_from_slice(&v[..3]);
                newc[0] += 1;
                newv_1[newc[1] * 3..newc[1] * 3 + 3].copy_from_slice(&v[..3]);
                newc[1] += 1;
            }
        }

        if sides[i] == SIDE_ON || sides[i + 1] == SIDE_ON || sides[i + 1] == sides[i] {
            continue;
        }

        let d = dists[i] / (dists[i] - dists[i + 1]);
        for j in 0..3 {
            let e = v[j] + d * (v[j + 3] - v[j]);
            newv_0[newc[0] * 3 + j] = e;
            newv_1[newc[1] * 3 + j] = e;
        }
        newc[0] += 1;
        newc[1] += 1;
    }

    // continue
    // SAFETY: per the contract.
    unsafe {
        clip_poly(newc[0], &mut newv_0, stage + 1);
        clip_poly(newc[1], &mut newv_1, stage + 1);
    }
}

/// `Sky_ProcessPoly`.
///
/// # Safety
/// `cbx` is recording; `p` is a live `glpoly_t`; `color` holds three floats;
/// main thread.
#[no_mangle]
pub unsafe extern "C" fn Sky_ProcessPoly(cbx: *mut CbContext, p: *mut c_void, color: *mut c_float) {
    // SAFETY: per the contract.
    unsafe {
        // draw it
        c::render::DrawGLPoly(cbx.cast(), p, color, 1.0);

        // update sky bounds
        if NEED_BOUNDS {
            let p = p.cast::<GlPoly>();
            let num_verts = (*p).numverts.max(0) as usize;
            let mut verts = vec![0f32; (num_verts + 2) * 3];
            let origin = &*ptr::addr_of!(c::host::r_origin);
            let base = ptr::addr_of!((*p).verts).cast::<f32>();
            for i in 0..num_verts {
                let pv = base.add(i * VERTEXSIZE);
                let pv: Vec3 = [*pv, *pv.add(1), *pv.add(2)];
                let mut out: Vec3 = [0.0; 3];
                vector_subtract(&pv, origin, &mut out);
                verts[i * 3..i * 3 + 3].copy_from_slice(&out);
            }
            clip_poly(num_verts, &mut verts, 0);
        }
    }
}

/// `Sky_ProcessTextureChains` -- handles sky polys in world model.
///
/// # Safety
/// `cbx` is recording; `color` holds three floats; `skypolys` is writable;
/// main thread with the world texture chains built.
#[no_mangle]
pub unsafe extern "C" fn Sky_ProcessTextureChains(
    cbx: *mut CbContext,
    color: *mut c_float,
    skypolys: *mut c_int,
) {
    // SAFETY: per the contract.
    unsafe {
        if !*ptr::addr_of!(c::render::r_drawworld_cheatsafe) {
            return;
        }

        let world = &*(*ptr::addr_of!(cl)).worldmodel;
        let sky = TEXTYPE_SKY as usize;
        for i in world.texofs[sky]..world.texofs[sky + 1] {
            let t = *world
                .textures
                .add(*world.usedtextures.add(i as usize) as usize);

            if t.is_null() || (*t).texturechains[CHAIN_WORLD].is_null() {
                continue;
            }

            let mut s = (*t).texturechains[CHAIN_WORLD];
            while !s.is_null() {
                Sky_ProcessPoly(cbx, (*s).polys, color);
                *skypolys += 1;
                s = (*s).texturechains[CHAIN_WORLD];
            }
        }
    }
}

/// `gl_sky.c:970` -- `Sky_DrawSkySurface`: copy the polygon and translate
/// manually, since `Sky_ProcessPoly` needs it to be in world space.
///
/// # Safety
/// As [`Sky_ProcessEntities`]; `s` is one of `e`'s surfaces with `polys`.
#[allow(clippy::too_many_arguments)]
unsafe fn draw_sky_surface(
    cbx: *mut CbContext,
    color: *mut c_float,
    e: &Entity,
    s: &MSurface,
    rotated: bool,
    forward: &Vec3,
    right: &Vec3,
    up: &Vec3,
) {
    // SAFETY: per the contract.
    unsafe {
        let sp = s.polys.cast::<GlPoly>();
        let numverts = (*sp).numverts.max(0) as usize;
        let bytes = core::mem::offset_of!(GlPoly, verts) + numverts * VERTEXSIZE * 4;
        let mut storage = vec![0u64; bytes.div_ceil(8)];
        let p = storage.as_mut_ptr().cast::<GlPoly>();
        (*p).next = ptr::null_mut();
        (*p).numverts = numverts as c_int;
        let sv = ptr::addr_of!((*sp).verts).cast::<f32>();
        let pv = ptr::addr_of_mut!((*p).verts).cast::<f32>();
        for k in 0..numverts {
            let sk = sv.add(k * VERTEXSIZE);
            let pk = pv.add(k * VERTEXSIZE);
            if rotated {
                for j in 0..3 {
                    *pk.add(j) =
                        e.origin[j] + *sk * forward[j] - *sk.add(1) * right[j] + *sk.add(2) * up[j];
                }
            } else {
                let s_poly_vert: Vec3 = [*sk, *sk.add(1), *sk.add(2)];
                let mut out: Vec3 = [0.0; 3];
                vector_add(&s_poly_vert, &e.origin, &mut out);
                *pk = out[0];
                *pk.add(1) = out[1];
                *pk.add(2) = out[2];
            }
        }
        Sky_ProcessPoly(cbx, p.cast(), color);
    }
}

/// `Sky_ProcessEntities` -- handles sky polys on brush models.
///
/// # Safety
/// `cbx` is recording; `color` holds three floats; main thread after
/// `cl_visedicts` has been built for the frame.
#[no_mangle]
pub unsafe extern "C" fn Sky_ProcessEntities(cbx: *mut CbContext, color: *mut c_float) {
    // SAFETY: per the contract.
    unsafe {
        if (*ptr::addr_of!(c::render::r_drawentities)).value == 0.0 {
            return;
        }

        let mut vieworg: Vec3 = [0.0; 3];
        c::render::VID_Glue_ViewOrg(vieworg.as_mut_ptr());

        let num = *ptr::addr_of!(c::cl_main::cl_numvisedicts);
        let visedicts = *ptr::addr_of!(c::cl_main::cl_visedicts);
        for i in 0..num.max(0) as usize {
            let ep = (*visedicts.add(i)).cast::<Entity>();
            let e = &*ep;
            let model = &*e.model;

            if model.type_ != MOD_BRUSH {
                continue;
            }

            if model.used_specials & SURF_DRAWSKY == 0 {
                continue;
            }

            if c::render::R_IndirectBrush(ep.cast()) {
                continue;
            }

            if c::render::R_CullModelForEntity(ep.cast()) {
                continue;
            }

            if e.alpha == ENTALPHA_ZERO {
                continue;
            }

            let mut modelorg: Vec3 = [0.0; 3];
            let mut forward: Vec3 = [0.0; 3];
            let mut right: Vec3 = [0.0; 3];
            let mut up: Vec3 = [0.0; 3];
            vector_subtract(&vieworg, &e.origin, &mut modelorg);
            let rotated = if e.angles[0] != 0.0 || e.angles[1] != 0.0 || e.angles[2] != 0.0 {
                angle_vectors(&e.angles, &mut forward, &mut right, &mut up);
                let temp = modelorg;
                modelorg[0] = dot_product(&temp, &forward);
                modelorg[1] = -dot_product(&temp, &right);
                modelorg[2] = dot_product(&temp, &up);
                true
            } else {
                false
            };

            let mut s = model.surfaces.add(model.firstmodelsurface.max(0) as usize);
            for _ in 0..model.nummodelsurfaces.max(0) {
                let surf = &*s;
                if surf.flags & SURF_DRAWSKY != 0 {
                    let plane = &*surf.plane;
                    let dot = dot_product(&modelorg, &plane.normal) - plane.dist;
                    if ((surf.flags & SURF_PLANEBACK != 0) && dot < -BACKFACE_EPSILON)
                        || ((surf.flags & SURF_PLANEBACK == 0) && dot > BACKFACE_EPSILON)
                    {
                        draw_sky_surface(cbx, color, e, surf, rotated, &forward, &right, &up);
                    }
                }
                s = s.add(1);
            }
        }
    }
}

//==============================================================================
//
//  RENDER SKYBOX
//
//==============================================================================

/// `Sky_EmitSkyBoxVertex`.
///
/// # Safety
/// `vertex` is writable; `axis` in `0..6` with `skybox.textures` loaded;
/// main thread.
#[no_mangle]
pub unsafe extern "C" fn Sky_EmitSkyBoxVertex(
    vertex: *mut BasicVertex,
    s: c_float,
    t: c_float,
    axis: c_int,
) {
    // SAFETY: per the contract.
    unsafe {
        let farclip = (*ptr::addr_of!(c::render::gl_farclip)).value;
        let sqrt3 = 3.0f64.sqrt();
        let b: Vec3 = [
            (f64::from(s * farclip) / sqrt3) as f32,
            (f64::from(t * farclip) / sqrt3) as f32,
            (f64::from(farclip) / sqrt3) as f32,
        ];

        let axis = axis as usize;
        let origin = &*ptr::addr_of!(c::host::r_origin);
        let mut v: Vec3 = [0.0; 3];
        for j in 0..3 {
            let k = ST_TO_VEC[axis][j];
            v[j] = if k < 0 {
                -b[(-k - 1) as usize]
            } else {
                b[(k - 1) as usize]
            };
            v[j] += origin[j];
        }

        // convert from range [-1,1] to [0,1]
        let mut s = (f64::from(s + 1.0) * 0.5) as f32;
        let mut t = (f64::from(t + 1.0) * 0.5) as f32;

        // avoid bilerp seam
        let tex = &*(*ptr::addr_of!(skybox)).textures[SKYTEXORDER[axis]];
        let w = tex.width as f32;
        let h = tex.height as f32;
        s = (f64::from(s * (w - 1.0) / w) + 0.5 / f64::from(w)) as f32;
        t = (f64::from(t * (h - 1.0) / h) + 0.5 / f64::from(h)) as f32;

        t = (1.0 - f64::from(t)) as f32;

        let vertex = &mut *vertex;
        vertex.position = v;
        vertex.texcoord = [s, t];
        vertex.color = [255; 4];
    }
}

/// `Sky_DrawSkyBox` -- FIXME: eliminate cracks by adding an extra vert on
/// tjuncs.
///
/// # Safety
/// `cbx` is recording with the sky box pipeline bound; `skypolys` is
/// writable; main thread.
#[no_mangle]
pub unsafe extern "C" fn Sky_DrawSkyBox(cbx: *mut CbContext, skypolys: *mut c_int) {
    // SAFETY: per the contract.
    let (indirect, cb, render_pass_index) = unsafe {
        (
            *ptr::addr_of!(c::render::indirect),
            (*cbx).cb,
            (*cbx).render_pass_index,
        )
    };
    let variant = cb::main_pass_pipeline_variant(render_pass_index);
    let layout = with_ctx(|ctx| {
        vg!(
            ctx,
            sky_stencil_pipeline[variant][indirect as usize]
                .layout
                .handle
        )
    });

    for i in 0..6 {
        // SAFETY: per the contract.
        unsafe {
            let mins = &mut *ptr::addr_of_mut!(SKYMINS);
            let maxs = &mut *ptr::addr_of_mut!(SKYMAXS);
            if indirect {
                // Don't have bounds for the world polys. This type of sky (malformed cube) is very rare, so just draw the entire box (will
                // be stencil-culled). Also note tjunctions avoidance below: this only culled entire cube faces in the first place.
                mins[0][i] = -1.0;
                mins[1][i] = -1.0;
                maxs[0][i] = 1.0;
                maxs[1][i] = 1.0;
            }

            if mins[0][i] >= maxs[0][i] || mins[1][i] >= maxs[1][i] {
                continue;
            }

            let set = (*(*ptr::addr_of!(skybox)).textures[SKYTEXORDER[i]]).descriptor_set;
            device().cmd_bind_descriptor_sets(
                cb,
                vk::PipelineBindPoint::GRAPHICS,
                layout,
                0,
                &[set],
                &[],
            );

            let a = with_ctx(|ctx| {
                DYN.vertex_allocate(ctx, 4 * core::mem::size_of::<BasicVertex>() as u32)
            });
            let vertices = a.data.cast::<BasicVertex>();

            // FIXME: this is to avoid tjunctions until i can do it the right way
            mins[0][i] = -1.0;
            mins[1][i] = -1.0;
            maxs[0][i] = 1.0;
            maxs[1][i] = 1.0;
            Sky_EmitSkyBoxVertex(vertices, mins[0][i], mins[1][i], i as c_int);
            Sky_EmitSkyBoxVertex(vertices.add(1), mins[0][i], maxs[1][i], i as c_int);
            Sky_EmitSkyBoxVertex(vertices.add(2), maxs[0][i], maxs[1][i], i as c_int);
            Sky_EmitSkyBoxVertex(vertices.add(3), maxs[0][i], mins[1][i], i as c_int);

            device().cmd_bind_vertex_buffers(cb, 0, &[a.buffer], &[a.buffer_offset]);
            with_ctx(|ctx| cb::draw_indexed(&CmdProcs::new(ctx.vg), cb, 6, 1, 0, 0, 0));

            *skypolys += 1;
        }
    }
}

//==============================================================================
//
//  RENDER CLOUDS
//
//==============================================================================

/// `gl_sky.c:1175` -- `Skywind_UpdateParams`.
///
/// # Safety
/// Main thread: reads `skybox`, `r_skywind` and `cl.time`.
unsafe fn skywind_update_params(wind_phase: &mut f32, wind_dir: &mut [f32]) {
    // SAFETY: per the contract.
    unsafe {
        if skywind_is_enabled() {
            let sb = &*ptr::addr_of!(skybox);
            let yaw = (f64::from(sb.wind_yaw) * M_PI_DIV_180) as f32;
            let pitch = (f64::from(sb.wind_pitch) * M_PI_DIV_180) as f32;
            let sy = c::libm::sinf(yaw);
            let sp = c::libm::sinf(pitch);
            let cy = c::libm::cosf(yaw);
            let cp = c::libm::cosf(pitch);
            let dist = clamp_f(-2.0, sb.wind_dist, 2.0);
            let r_skywind = (*ptr::addr_of!(c::render::r_skywind)).value;
            let period: f32 = if r_skywind != 0.0 {
                sb.wind_period / r_skywind
            } else {
                0.0
            };
            let mut phase: f64 = if period != 0.0 {
                (*ptr::addr_of!(cl)).time * 0.5 / f64::from(period)
            } else {
                0.5
            };

            phase -= c::libm::floor(phase) + 0.5; // [-0.5, 0.5)

            wind_dir[0] = dist * cp * sy;
            wind_dir[1] = dist * sp;
            wind_dir[2] = -dist * cp * cy;
            *wind_phase = phase as f32;
        } else {
            // reset
            wind_dir[0] = 0.0;
            wind_dir[1] = 0.0;
            wind_dir[2] = 0.0;
            *wind_phase = 0.0;
        }
    }
}

fn floats_to_bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_ne_bytes()).collect()
}

/// `Sky_DrawSky` -- called once per frame after opaques before transparents,
/// handles world + entities.
///
/// # Safety
/// `cbx` is a live recording context, unaliased for the call; main thread
/// with the frame's texture chains and `cl_visedicts` built.
#[no_mangle]
pub unsafe extern "C" fn Sky_DrawSky(cbx: *mut CbContext) {
    // SAFETY: per the contract.
    let (render_pass_index, indirect, lightmap) = unsafe {
        (
            (*cbx).render_pass_index,
            *ptr::addr_of!(c::render::indirect),
            *ptr::addr_of!(c::render::r_lightmap_cheatsafe),
        )
    };
    let variant = cb::main_pass_pipeline_variant(render_pass_index);
    let ind = indirect as usize;

    if lightmap {
        return;
    }

    with_ctx(|ctx| {
        let procs = CmdProcs::new(ctx.vg);
        // SAFETY: per the contract.
        unsafe { cb::begin_debug_utils_label(&procs, &*cbx, c"Sky") };
    });

    // SAFETY: per the contract.
    let (flat_color, fog_density, mut color) = unsafe {
        let density = Fog_GetDensity();
        let flat_color =
            (*ptr::addr_of!(c::render::r_fastsky)).value != 0.0 || (density > 0.0 && skyfog >= 1.0);
        NEED_BOUNDS = Sky_NeedStencil();

        //
        // reset sky bounds
        //
        let mins = &mut *ptr::addr_of_mut!(SKYMINS);
        let maxs = &mut *ptr::addr_of_mut!(SKYMAXS);
        for i in 0..6 {
            mins[0][i] = f32::MAX;
            mins[1][i] = f32::MAX;
            maxs[0][i] = -f32::MAX;
            maxs[1][i] = -f32::MAX;
        }

        let fog_density = if density > 0.0 { skyfog } else { 0.0 };

        let mut color = [0f32; 4];
        if density > 0.0 {
            Fog_GetColor(color.as_mut_ptr()); // color[3] is not used
        } else {
            color[..3].copy_from_slice(&*ptr::addr_of!(SKYFLATCOLOR));
        }
        (flat_color, fog_density, color)
    };

    let mut constant_values = [0f32; 27];
    with_ctx(|ctx| {
        constant_values[..16].copy_from_slice(&vg!(ctx, view_projection_matrix));
    });
    constant_values[16] = clamp_f(0.0, color[0], 1.0);
    constant_values[17] = clamp_f(0.0, color[1], 1.0);
    constant_values[18] = clamp_f(0.0, color[2], 1.0);
    constant_values[19] = fog_density;

    // SAFETY: per the contract.
    let (cubemap, has_name, solid, alpha) = unsafe {
        let sb = &*ptr::addr_of!(skybox);
        (
            sb.cubemap,
            sb.name[0] != 0,
            SOLIDSKYTEXTURE,
            ALPHASKYTEXTURE,
        )
    };

    // With slow sky we first write stencil for the part of the screen that is covered by sky geometry and passes the depth test
    // Sky_DrawSkyBox then only fills the parts that had stencil written
    let bound = with_ctx(|ctx| {
        let procs = CmdProcs::new(ctx.vg);
        let color_pipeline: VulkanPipeline = vg!(ctx, sky_color_pipeline[variant][ind]);
        let cube_pipeline: VulkanPipeline = vg!(ctx, sky_cube_pipeline[variant][ind]);
        let layer_pipeline: VulkanPipeline = vg!(ctx, sky_layer_pipeline[variant][ind]);
        let stencil_pipeline: VulkanPipeline = vg!(ctx, sky_stencil_pipeline[variant][ind]);
        let fan_index_buffer: vk::Buffer = vg!(ctx, fan_index_buffer);
        // SAFETY: per the contract.
        unsafe {
            let cbx = &mut *cbx;
            if flat_color {
                if indirect {
                    constant_values[19] = 1.0;
                }
                let pipeline = color_pipeline;
                cb::bind_pipeline(&procs, cbx, vk::PipelineBindPoint::GRAPHICS, pipeline);
                cb::push_constants(
                    &procs,
                    cbx,
                    vk::ShaderStageFlags::ALL_GRAPHICS,
                    0,
                    &floats_to_bytes(&constant_values[..20]),
                );
            } else if !cubemap.is_null() {
                let pipeline = cube_pipeline;
                cb::bind_pipeline(&procs, cbx, vk::PipelineBindPoint::GRAPHICS, pipeline);
                ctx.device.cmd_bind_descriptor_sets(
                    cbx.cb,
                    vk::PipelineBindPoint::GRAPHICS,
                    pipeline.layout.handle,
                    0,
                    &[(*cubemap).descriptor_set],
                    &[],
                );
                c::render::VID_Glue_ViewOrg(constant_values[20..23].as_mut_ptr());

                let (phase, dir) = constant_values[23..27].split_at_mut(1);
                skywind_update_params(&mut phase[0], dir);

                cb::push_constants(
                    &procs,
                    cbx,
                    vk::ShaderStageFlags::ALL_GRAPHICS,
                    0,
                    &floats_to_bytes(&constant_values[..27]),
                );
            } else if !has_name {
                if solid.is_null() || alpha.is_null() {
                    cb::end_debug_utils_label(&procs, cbx);
                    return false;
                }
                let pipeline = layer_pipeline;
                cb::bind_pipeline(&procs, cbx, vk::PipelineBindPoint::GRAPHICS, pipeline);
                let descriptor_sets = [(*solid).descriptor_set, (*alpha).descriptor_set];
                ctx.device.cmd_bind_descriptor_sets(
                    cbx.cb,
                    vk::PipelineBindPoint::GRAPHICS,
                    pipeline.layout.handle,
                    0,
                    &descriptor_sets,
                    &[],
                );
                c::render::VID_Glue_ViewOrg(constant_values[20..23].as_mut_ptr());
                let cl_time = (*ptr::addr_of!(cl)).time;
                constant_values[23] = (cl_time - f64::from((cl_time as c_int) / 16 * 16)) as f32;
                constant_values[24] = (*ptr::addr_of!(c::render::r_skyalpha)).value;
                cb::push_constants(
                    &procs,
                    cbx,
                    vk::ShaderStageFlags::ALL_GRAPHICS,
                    0,
                    &floats_to_bytes(&constant_values[..25]),
                );
            } else {
                let pipeline = stencil_pipeline;
                cb::bind_pipeline(&procs, cbx, vk::PipelineBindPoint::GRAPHICS, pipeline);
                cb::push_constants(
                    &procs,
                    cbx,
                    vk::ShaderStageFlags::ALL_GRAPHICS,
                    0,
                    &floats_to_bytes(&constant_values[..20]),
                );
            }
            ctx.device
                .cmd_bind_index_buffer(cbx.cb, fan_index_buffer, 0, vk::IndexType::UINT16);
            true
        }
    });
    if !bound {
        return;
    }

    //
    // process world and bmodels: draw flat-shaded sky surfs, and update skybounds
    //
    let mut skypolys: c_int = 0;
    if indirect {
        // SAFETY: per the contract.
        unsafe { c::render::R_DrawIndirectBrushes(cbx.cast(), false, false, true, -1) };

        // Entities cannot use the indirect pipelines
        with_ctx(|ctx| {
            let procs = CmdProcs::new(ctx.vg);
            let fan_index_buffer: vk::Buffer = vg!(ctx, fan_index_buffer);
            let pipeline: VulkanPipeline = if flat_color {
                vg!(ctx, sky_color_pipeline[variant][0])
            } else if !cubemap.is_null() {
                vg!(ctx, sky_cube_pipeline[variant][0])
            } else if !has_name {
                vg!(ctx, sky_layer_pipeline[variant][0])
            } else {
                vg!(ctx, sky_stencil_pipeline[variant][0])
            };
            // SAFETY: per the contract.
            unsafe {
                let cbx = &mut *cbx;
                ctx.device.cmd_bind_index_buffer(
                    cbx.cb,
                    fan_index_buffer,
                    0,
                    vk::IndexType::UINT16,
                );
                cb::bind_pipeline(&procs, cbx, vk::PipelineBindPoint::GRAPHICS, pipeline);
            }
        });
    } else {
        // SAFETY: per the contract.
        unsafe { Sky_ProcessTextureChains(cbx, color.as_mut_ptr(), &mut skypolys) };
    }

    // SAFETY: per the contract.
    unsafe { Sky_ProcessEntities(cbx, color.as_mut_ptr()) };

    //
    // render slow sky: non-cubemap skybox
    //
    if !flat_color && cubemap.is_null() && has_name {
        with_ctx(|ctx| {
            let procs = CmdProcs::new(ctx.vg);
            let pipeline: VulkanPipeline = vg!(ctx, sky_box_pipeline[variant]);
            // SAFETY: per the contract.
            unsafe {
                cb::bind_pipeline(&procs, &mut *cbx, vk::PipelineBindPoint::GRAPHICS, pipeline)
            };
        });
        // SAFETY: per the contract.
        unsafe { Sky_DrawSkyBox(cbx, &mut skypolys) };
    }

    // SAFETY: `rs_skypolys` is the C `atomic_uint32_t` (4 bytes, aligned).
    unsafe {
        AtomicU32::from_ptr(ptr::addr_of_mut!(c::render::rs_skypolys))
            .fetch_add(skypolys as u32, Ordering::SeqCst);
    }

    with_ctx(|ctx| {
        let procs = CmdProcs::new(ctx.vg);
        // SAFETY: per the contract.
        unsafe { cb::end_debug_utils_label(&procs, &*cbx) };
    });
}

/// `Sky_NeedStencil`.
///
/// # Safety
/// Main thread: reads `r_fastsky`, the fog state and `skybox`.
#[no_mangle]
pub unsafe extern "C" fn Sky_NeedStencil() -> bool {
    // SAFETY: per the contract.
    unsafe {
        let sb = &*ptr::addr_of!(skybox);
        let flat_color = (*ptr::addr_of!(c::render::r_fastsky)).value != 0.0
            || (Fog_GetDensity() > 0.0 && skyfog >= 1.0);
        !flat_color && sb.cubemap.is_null() && sb.name[0] != 0
    }
}
