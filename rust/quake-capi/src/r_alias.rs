//! `r_alias.c` -- alias model rendering (Rust migration Phase 8 M8).
//!
//! `R_DrawAliasModel` and `R_DrawAliasModel_ShowTris` stay as thin wrappers
//! in `Quake/r_alias_glue.c`: they call `Mod_Extradata_CheckSkin`, which can
//! reach `Host_Error` through `Mod_LoadModel`, in C (ADR-009 -- no longjmp
//! through Rust frames) and hand the resolved `aliashdr_t *` to
//! [`RAlias_DrawAliasModel`] / [`RAlias_DrawAliasModel_ShowTris`]. Everything
//! else is transcribed statement for statement.
//!
//! COMPAT: ADR-010. The lerp and lighting maths mix `float` and `double`
//! exactly where the C does: `cl.time` is `double`, the frame intervals and
//! blends are `float`, `CLAMP` evaluates in `double` and narrows on
//! assignment, and `tan`/`cos`/`sin` are the CRT's `double` functions.

use core::ffi::{c_int, c_void};
use core::ptr;

use ash::vk::{self, Handle};
use quake_c_sys as c;
use quake_math::mathlib::{
    dot_product, identity_matrix, matrix_multiply, scale_matrix, translation_matrix, vector_length,
    vector_normalize, Vec3,
};
use quake_render::cb::{self, CmdProcs};
use quake_render::rmisc::Ctx;
use quake_render::vid::{
    RENDER_PASS_INDEX_MBOIT_COMPOSITE, RENDER_PASS_INDEX_MBOIT_MOMENTS, RENDER_PASS_INDEX_WBOIT,
};
use quake_types::host::{ClientState, Entity};
use quake_types::model_mem::{AliasHdr, QModel, PV_MD5, PV_MD5_8, PV_QUAKE1, PV_QUAKE3};
use quake_types::refdef::RefDef;
use quake_types::render::{
    AliasUbo, CbContext, GlTexture, LerpData, Md5Ubo, VulkanPipeline,
    MODEL_PIPELINE_ALPHA_BLEND_BIT, MODEL_PIPELINE_ALPHA_TEST_BIT, MODEL_PIPELINE_SHOWTRIS,
    MODEL_PIPELINE_SHOWTRIS_DEPTH_TEST, TEXPREF_ALPHAPIXELS,
};

use crate::gl_rlight::R_LightPoint;
use crate::gl_rmisc::{device, vg, with_ctx, CEngine, DYN};

extern "C" {
    /// `client_state_t cl` (ADR-007 row closed in Phase 7).
    static mut cl: ClientState;
    /// `gl_rmain.c:58` -- `refdef_t r_refdef`.
    static mut r_refdef: RefDef;
}

/// `MAX_DLIGHTS` (`quakedef.h`).
const MAX_DLIGHTS: usize = 64;
/// `MAX_SCOREBOARD` (`quakedef.h`).
const MAX_SCOREBOARD: usize = 16;
/// `SHADEDOT_QUANT` (`r_alias.c:42`).
const SHADEDOT_QUANT: c_int = 16;
/// `EF_ROTATE` (`gl_model.h:598`).
const EF_ROTATE: c_int = 8;
/// `MF_HOLEY` (`gl_model.h:603`).
const MF_HOLEY: c_int = 1 << 14;
/// `MOD_NOLERP` (`gl_model.h:606`).
const MOD_NOLERP: c_int = 256;
/// `M_PI` (`mathlib.h`).
const M_PI: f64 = core::f64::consts::PI;

/// `gltexture_t *playertextures[MAX_SCOREBOARD]` (`r_alias.c:44`): up to 16
/// color-translated skin textures (`gl_texmgr.c` / `gl_rmain.c` fill it).
#[no_mangle]
pub static mut playertextures: [*mut c_void; MAX_SCOREBOARD] = [ptr::null_mut(); MAX_SCOREBOARD];

/// `ENTALPHA_DECODE (a)` (`protocol.h:223`).
fn entalpha_decode(a: u8) -> f32 {
    if a == 0 {
        1.0
    } else {
        (f32::from(a) - 1.0) / 254.0
    }
}

/// `CLAMP (0, x, 1)` over a `double` expression, narrowed to `float` on
/// assignment as in the C.
fn clamp01(x: f64) -> f32 {
    x.clamp(0.0, 1.0) as f32
}

/// `GLARB_GetXYZOffset`: the offset of the first vertex's `meshxyz_t.xyz`
/// in the vbo for the given pose (`offsetof (meshxyz_t, xyz)` is 0).
fn xyz_offset(hdr: &AliasHdr, pose: i16) -> vk::DeviceSize {
    (hdr.numverts_vbo * i32::from(pose) * 12) as vk::DeviceSize
}

/// The `R_PipelineForRenderPass` selection over one pipeline family.
#[allow(clippy::type_complexity)]
fn family_pipeline(
    ctx: &mut Ctx<'_, CEngine>,
    cbx: &CbContext,
    poseverttype: i32,
    pipeline_index: usize,
) -> VulkanPipeline {
    let idx = cbx.render_pass_index;
    let variant = cb::main_pass_pipeline_variant(idx);
    let (main, wboit, moment, composite) = match poseverttype {
        PV_MD5_8 => (
            vg!(ctx, md5_8_pipelines[variant][pipeline_index]),
            vg!(ctx, md5_8_wboit_pipelines[pipeline_index]),
            vg!(ctx, md5_8_mboit_moment_pipelines[pipeline_index]),
            vg!(ctx, md5_8_mboit_composite_pipelines[pipeline_index]),
        ),
        PV_MD5 => (
            vg!(ctx, md5_pipelines[variant][pipeline_index]),
            vg!(ctx, md5_wboit_pipelines[pipeline_index]),
            vg!(ctx, md5_mboit_moment_pipelines[pipeline_index]),
            vg!(ctx, md5_mboit_composite_pipelines[pipeline_index]),
        ),
        _ => (
            vg!(ctx, alias_pipelines[variant][pipeline_index]),
            vg!(ctx, alias_wboit_pipelines[pipeline_index]),
            vg!(ctx, alias_mboit_moment_pipelines[pipeline_index]),
            vg!(ctx, alias_mboit_composite_pipelines[pipeline_index]),
        ),
    };
    cb::pipeline_for_render_pass(idx, main, wboit, moment, composite)
}

/// `GL_DrawAliasFrame` -- based on code by MH from RMQEngine.
///
/// # Safety
/// `cbx` is recording inside a render pass; `paliashdr` has uploaded mesh
/// buffers; `tx` (and `fb` when non-null) are live `gltexture_t`s.
#[allow(clippy::too_many_arguments)]
unsafe fn gl_draw_alias_frame(
    cbx: *mut CbContext,
    paliashdr: *mut AliasHdr,
    lerpdata: LerpData,
    tx: *mut GlTexture,
    fb: *mut GlTexture,
    model_matrix: &[f32; 16],
    entity_alpha: f32,
    alphatest: bool,
    shadevector: Vec3,
    lightcolor: Vec3,
    showtris: c_int,
) {
    // SAFETY: the caller's contract.
    unsafe {
        let hdr = &*paliashdr;
        let render_pass_index = (*cbx).render_pass_index;

        // only enable alpha management if entity have alpha or the surface texture has effective
        // non-opaque pixels:
        let has_alpha = (entity_alpha < 1.0) || ((*tx).flags & TEXPREF_ALPHAPIXELS) != 0;

        let pipeline_index = if showtris == 0 {
            (if has_alpha {
                MODEL_PIPELINE_ALPHA_BLEND_BIT
            } else {
                0
            }) | (if alphatest {
                MODEL_PIPELINE_ALPHA_TEST_BIT
            } else {
                0
            })
        } else if showtris >= 2 {
            MODEL_PIPELINE_SHOWTRIS_DEPTH_TEST
        } else {
            MODEL_PIPELINE_SHOWTRIS
        };

        let oit_pass = render_pass_index == RENDER_PASS_INDEX_WBOIT
            || render_pass_index == RENDER_PASS_INDEX_MBOIT_MOMENTS
            || render_pass_index == RENDER_PASS_INDEX_MBOIT_COMPOSITE;
        if oit_pass && (showtris != 0 || !has_alpha) {
            return;
        }

        let blend = if lerpdata.pose1 != lerpdata.pose2 {
            lerpdata.blend
        } else {
            // poses the same means either 1. the entity has paused its animation, or 2. r_lerpmodels is disabled
            0.0
        };

        let mut flags: u32 = if fb.is_null() { 0 } else { 1 };
        if *ptr::addr_of!(c::render::r_fullbright_cheatsafe)
            || (*ptr::addr_of!(c::render::r_lightmap_cheatsafe)
                && (*ptr::addr_of!(c::render::r_fullbright)).value != 0.0)
        {
            flags |= 2;
        }

        let tx_set = (*tx).descriptor_set;
        let fb_set = if fb.is_null() {
            tx_set
        } else {
            (*fb).descriptor_set
        };
        let vertex_buffer = vk::Buffer::from_raw(hdr.vertex_buffer as u64);
        let index_buffer = vk::Buffer::from_raw(hdr.index_buffer as u64);

        with_ctx(|ctx| {
            let procs = CmdProcs::new(ctx.vg);
            let pipeline = family_pipeline(ctx, &*cbx, hdr.poseverttype, pipeline_index);
            cb::bind_pipeline(&procs, &mut *cbx, vk::PipelineBindPoint::GRAPHICS, pipeline);
            let device = device();
            let cmd = (*cbx).cb;

            match hdr.poseverttype {
                PV_QUAKE1 | PV_QUAKE3 => {
                    let a = DYN.uniform_allocate(ctx, core::mem::size_of::<AliasUbo>() as u32);
                    let ubo = a.data.cast::<AliasUbo>();
                    (*ubo).model_matrix = *model_matrix;
                    (*ubo).shade_vector = shadevector;
                    (*ubo).blend_factor = blend;
                    (*ubo).light_color = lightcolor;
                    (*ubo).flags = flags;
                    if hdr.poseverttype == PV_QUAKE3 {
                        (*ubo).flags |= 4;
                    }
                    (*ubo).entalpha = entity_alpha;

                    device.cmd_bind_descriptor_sets(
                        cmd,
                        vk::PipelineBindPoint::GRAPHICS,
                        pipeline.layout.handle,
                        0,
                        &[tx_set, fb_set, a.descriptor_set],
                        &[a.buffer_offset as u32],
                    );
                    device.cmd_bind_vertex_buffers(
                        cmd,
                        0,
                        &[vertex_buffer, vertex_buffer, vertex_buffer],
                        &[
                            hdr.vbostofs as u32 as vk::DeviceSize,
                            xyz_offset(hdr, lerpdata.pose1),
                            xyz_offset(hdr, lerpdata.pose2),
                        ],
                    );
                    device.cmd_bind_index_buffer(cmd, index_buffer, 0, vk::IndexType::UINT16);
                    cb::draw_indexed(&procs, cmd, hdr.numindexes as u32, 1, 0, 0, 0);
                }
                PV_MD5 | PV_MD5_8 => {
                    let a = DYN.uniform_allocate(ctx, core::mem::size_of::<Md5Ubo>() as u32);
                    let ubo = a.data.cast::<Md5Ubo>();
                    (*ubo).model_matrix = *model_matrix;
                    (*ubo).shade_vector = shadevector;
                    (*ubo).blend_factor = blend;
                    (*ubo).light_color = lightcolor;
                    (*ubo).flags = flags;
                    (*ubo).entalpha = entity_alpha;
                    (*ubo).joints_offsets[0] = (i32::from(lerpdata.pose1) * hdr.numjoints) as u32;
                    (*ubo).joints_offsets[1] = (i32::from(lerpdata.pose2) * hdr.numjoints) as u32;

                    let joints_set = vk::DescriptorSet::from_raw(hdr.joints_set as u64);
                    device.cmd_bind_descriptor_sets(
                        cmd,
                        vk::PipelineBindPoint::GRAPHICS,
                        pipeline.layout.handle,
                        0,
                        &[tx_set, fb_set, a.descriptor_set, joints_set],
                        &[a.buffer_offset as u32],
                    );
                    device.cmd_bind_vertex_buffers(cmd, 0, &[vertex_buffer], &[0]);
                    device.cmd_bind_index_buffer(cmd, index_buffer, 0, vk::IndexType::UINT16);
                    cb::draw_indexed(&procs, cmd, hdr.numindexes as u32, 1, 0, 0, 0);
                }
                _ => debug_assert!(false),
            }
        });
    }
}

/// `R_EntityPoseAt`: the pose of `frame` at `time`.
///
/// # Safety
/// `paliashdr` must be a live alias header.
unsafe fn entity_pose_at(paliashdr: *const AliasHdr, mut frame: c_int, time: f64) -> c_int {
    // SAFETY: the caller's contract; `frame` is clamped before indexing.
    unsafe {
        let hdr = &*paliashdr;
        if frame >= hdr.numframes || frame < 0 {
            frame = 0;
        }
        let frames = hdr.frames.as_ptr();
        let desc = &*frames.add(frame as usize);
        let mut posenum = desc.firstpose;
        let numposes = desc.numposes;
        if numposes > 1 {
            posenum += ((time / f64::from(desc.interval)) as c_int) % numposes;
        }
        posenum
    }
}

/// `R_SetupAliasFrame` -- johnfitz -- rewritten to support lerping.
///
/// # Safety
/// `e` is a live alias entity, `paliashdr` its extradata and `lerpdata` a
/// writable out-parameter.
#[no_mangle]
pub unsafe extern "C" fn R_SetupAliasFrame(
    e: *mut Entity,
    paliashdr: *mut AliasHdr,
    lerpdata: *mut LerpData,
) {
    // SAFETY: the caller's contract; frame/pose indices are clamped before
    // use exactly as in the C.
    unsafe {
        let hdr = &*paliashdr;
        let model = &*(*e).model;
        let mut frame = (*e).frame;
        let cl_time = (*ptr::addr_of!(cl)).time;
        let out = &mut *lerpdata;

        if frame >= hdr.numframes || frame < 0 {
            c::Con_DPrintf(
                c"R_AliasSetupFrame: no such frame %d for '%s'\n".as_ptr(),
                frame,
                model.name.as_ptr(),
            );
            frame = 0;
        }

        let frames = hdr.frames.as_ptr();
        let desc = &*frames.add(frame as usize);
        let posenum = desc.firstpose;
        let numposes = desc.numposes;
        let lerpmodels = (*ptr::addr_of!(c::cl_main::r_lerpmodels)).value;

        if lerpmodels != 0.0 && !((model.flags & MOD_NOLERP) != 0 && lerpmodels != 2.0) {
            if numposes > 1 {
                // framegroup
                let interval = f64::from(desc.interval);
                let idx = (cl_time / interval) as c_int;
                let boundary = f64::from(idx) * interval;
                out.pose2 = (posenum + idx % numposes) as i16;
                let change_time = (*e).lerp.frame_change_time;
                if change_time > boundary {
                    // the framegroup started after the last boundary: lerp from the previous frame
                    out.pose1 = entity_pose_at(paliashdr, (*e).lerp.prev_frame, change_time) as i16;
                    out.blend = clamp01((cl_time - change_time) / interval);
                } else {
                    out.pose1 = (posenum + (idx + numposes - 1) % numposes) as i16;
                    out.blend = clamp01((cl_time - boundary) / interval);
                }
            } else if (*e).lerp.frame_change_time > 0.0 {
                let change_time = (*e).lerp.frame_change_time;
                let duration = if (*e).lerp.frame_duration > 0.0 {
                    (*e).lerp.frame_duration
                } else {
                    0.1
                };
                out.pose2 = posenum as i16;
                out.pose1 = entity_pose_at(paliashdr, (*e).lerp.prev_frame, change_time) as i16;
                out.blend = clamp01((cl_time - change_time) / duration);
            } else {
                out.pose2 = posenum as i16;
                out.pose1 = out.pose2;
                out.blend = 1.0;
            }

            if hdr.poseverttype == PV_QUAKE1 {
                if i32::from(out.pose2) < 0 || i32::from(out.pose2) >= hdr.numposes {
                    c::Con_DPrintf(
                        c"R_AliasSetupFrame: invalid current pose %d (%d total) for '%s'\n"
                            .as_ptr(),
                        c_int::from(out.pose2),
                        hdr.numposes,
                        model.name.as_ptr(),
                    );
                    out.pose2 = 0;
                }
                if i32::from(out.pose1) < 0 || i32::from(out.pose1) >= hdr.numposes {
                    c::Con_DPrintf(
                        c"R_AliasSetupFrame: invalid prev pose %d (%d total) for '%s'\n".as_ptr(),
                        c_int::from(out.pose1),
                        hdr.numposes,
                        model.name.as_ptr(),
                    );
                    out.pose1 = out.pose2;
                }
            } else if hdr.poseverttype == PV_MD5 || hdr.poseverttype == PV_MD5_8 {
                if i32::from(out.pose2) < 0 || i32::from(out.pose2) >= hdr.numframes {
                    c::Con_DPrintf(
                        c"R_AliasSetupFrame: invalid current pose %d (%d total) for '%s'\n"
                            .as_ptr(),
                        c_int::from(out.pose2),
                        hdr.numframes,
                        model.name.as_ptr(),
                    );
                    out.pose2 = 0;
                }
                if i32::from(out.pose1) < 0 || i32::from(out.pose1) >= hdr.numframes {
                    c::Con_DPrintf(
                        c"R_AliasSetupFrame: invalid prev pose %d (%d total) for '%s'\n".as_ptr(),
                        c_int::from(out.pose1),
                        hdr.numframes,
                        model.name.as_ptr(),
                    );
                    out.pose1 = out.pose2;
                }
            }
        } else {
            // don't lerp
            out.blend = 1.0;
            out.pose1 = entity_pose_at(paliashdr, frame, cl_time) as i16;
            out.pose2 = out.pose1;
        }
    }
}

/// `R_GetEntityLerpedTransform` -- johnfitz -- moved from `R_SetupAliasFrame`
/// so it can serve brush and sprite models too.
///
/// # Safety
/// `e` is a live entity with a model; the out-pointers address three floats
/// each.
#[no_mangle]
#[allow(clippy::needless_range_loop)]
pub unsafe extern "C" fn R_GetEntityLerpedTransform(
    e: *mut Entity,
    out_origin: *mut f32,
    out_angles: *mut f32,
) {
    // SAFETY: the caller's contract.
    unsafe {
        let ent = &*e;
        let viewent = ptr::addr_of!((*ptr::addr_of!(cl)).viewent).cast::<Entity>();
        let cl_time = (*ptr::addr_of!(cl)).time;
        let mut origin = [0.0f32; 3];
        let mut angles = [0.0f32; 3];

        if (*ptr::addr_of!(c::menu::r_lerpmove)).value != 0.0
            && e.cast_const() != viewent
            && ent.lerp.movestep
            && ent.netstate.tagentity == 0
            && ent.lerp.move_change_time > 0.0
        {
            let duration = if ent.lerp.move_duration > 0.0 {
                ent.lerp.move_duration
            } else {
                0.1
            };
            let blend = clamp01((cl_time - ent.lerp.move_change_time) / duration);

            for i in 0..3 {
                origin[i] = ent.lerp.prev_origin[i]
                    + (ent.msg_origins[0][i] - ent.lerp.prev_origin[i]) * blend;
            }

            if (*ptr::addr_of!(c::cl_main::r_lerpturn)).value != 0.0
                && ((*ent.model).flags & EF_ROTATE) == 0
            {
                for i in 0..3 {
                    let mut d = ent.msg_angles[0][i] - ent.lerp.prev_angles[i];
                    if d > 180.0 {
                        d -= 360.0;
                    }
                    if d < -180.0 {
                        d += 360.0;
                    }
                    angles[i] = ent.lerp.prev_angles[i] + d * blend;
                }
            } else {
                angles = ent.angles;
            }
        } else {
            origin = ent.origin;
            angles = ent.angles;
        }

        ptr::copy_nonoverlapping(origin.as_ptr(), out_origin, 3);
        ptr::copy_nonoverlapping(angles.as_ptr(), out_angles, 3);
    }
}

/// `R_SetupAliasLighting` -- johnfitz -- broken out from `R_DrawAliasModel`
/// and rewritten.
///
/// # Safety
/// `e` is a live alias entity; `cl_dlights` and `cl.worldmodel` are live.
#[allow(clippy::needless_range_loop, clippy::approx_constant)] // C's literal 3.14159
unsafe fn setup_alias_lighting(e: *mut Entity, shadevector: &mut Vec3, lightcolor: &mut Vec3) {
    // SAFETY: the caller's contract.
    unsafe {
        let ent = &mut *e;
        let cl_time = (*ptr::addr_of!(cl)).time;

        if R_LightPoint(
            ent.origin.as_mut_ptr(),
            0.0,
            &mut ent.lightcache,
            lightcolor,
        ) == 0
        {
            R_LightPoint(
                ent.origin.as_mut_ptr(),
                (*ent.model).maxs[2] * 0.5,
                &mut ent.lightcache,
                lightcolor,
            );
        }

        // add dlights
        let dlights = ptr::addr_of!(c::cl_main::cl_dlights).cast::<c::cl_tent::dlight_t>();
        for i in 0..MAX_DLIGHTS {
            let dl = &*dlights.add(i);
            if f64::from(dl.die) >= cl_time {
                let mut dist = [0.0f32; 3];
                for k in 0..3 {
                    dist[k] = ent.origin[k] - dl.origin[k];
                }
                let mut add = dl.radius - vector_length(&dist);
                if add > 0.0 {
                    if dl.cone_cos > -1.0 {
                        let mut dir = dist;
                        vector_normalize(&mut dir);
                        let cone_dot = dot_product(&dir, &dl.cone_dir);
                        let cone_scale = if dl.kex_intensity > 0.0 {
                            if cone_dot < dl.cone_cos {
                                continue;
                            }
                            1.0 - (1.0 - cone_dot) / (1.0 - dl.cone_cos)
                        } else {
                            let cone_soft = dl.cone_cos + (1.0 - dl.cone_cos) * 0.25;
                            let denom = if cone_soft - dl.cone_cos > 0.0001 {
                                cone_soft - dl.cone_cos
                            } else {
                                0.0001
                            };
                            clamp01(f64::from((cone_dot - dl.cone_cos) / denom))
                        };
                        add *= cone_scale;
                        if add <= 0.0 {
                            continue;
                        }
                    }
                    if dl.kex_intensity > 0.0 {
                        add *= dl.kex_intensity * 0.5 * (256.0 / dl.radius);
                    }
                    for k in 0..3 {
                        lightcolor[k] += add * dl.color[k];
                    }
                }
            }
        }

        // minimum light value on gun (24)
        let viewent = ptr::addr_of!((*ptr::addr_of!(cl)).viewent).cast::<Entity>();
        if e.cast_const() == viewent {
            let add = 72.0 - (lightcolor[0] + lightcolor[1] + lightcolor[2]);
            if add > 0.0 {
                lightcolor[0] += add / 3.0;
                lightcolor[1] += add / 3.0;
                lightcolor[2] += add / 3.0;
            }
        }

        // minimum light value on players (8)
        let entities = (*ptr::addr_of!(cl)).entities.cast::<Entity>();
        let maxclients = (*ptr::addr_of!(cl)).maxclients as usize;
        if e.cast_const() > entities.cast_const()
            && e.cast_const() <= entities.add(maxclients).cast_const()
        {
            let add = 24.0 - (lightcolor[0] + lightcolor[1] + lightcolor[2]);
            if add > 0.0 {
                lightcolor[0] += add / 3.0;
                lightcolor[1] += add / 3.0;
                lightcolor[2] += add / 3.0;
            }
        }

        // clamp lighting so it doesn't overbright as much (96)
        let add = 288.0 / (lightcolor[0] + lightcolor[1] + lightcolor[2]);
        if add < 1.0 {
            lightcolor[0] *= add;
            lightcolor[1] *= add;
            lightcolor[2] *= add;
        }

        let quantizedangle = ((f64::from(ent.angles[1]) * (f64::from(SHADEDOT_QUANT) / 360.0))
            as c_int)
            & (SHADEDOT_QUANT - 1);

        // ericw -- shadevector is passed to the shader to compute shadedots inside the
        // shader, see GLAlias_CreateShaders()
        let radiansangle = ((f64::from(quantizedangle) / 16.0) * 2.0 * 3.14159) as f32;
        shadevector[0] = (-f64::from(radiansangle)).cos() as f32;
        shadevector[1] = (-f64::from(radiansangle)).sin() as f32;
        shadevector[2] = 1.0;
        vector_normalize(shadevector);
        // ericw --

        lightcolor[0] *= 1.0 / 200.0;
        lightcolor[1] *= 1.0 / 200.0;
        lightcolor[2] *= 1.0 / 200.0;
    }
}

/// The shared front half of both draw paths: lerp setup, transform, cull
/// and the model matrix. Returns `None` when the entity is culled.
///
/// # Safety
/// As for [`RAlias_DrawAliasModel`].
unsafe fn setup_alias_model(
    e: *mut Entity,
    paliashdr: *mut AliasHdr,
    showtris: bool,
) -> Option<(LerpData, [f32; 16])> {
    // SAFETY: the caller's contract.
    unsafe {
        let hdr = &*paliashdr;
        let mut lerpdata = LerpData::default();
        let mut model_matrix = [0.0f32; 16];

        //
        // setup pose/lerp data -- do it first so we don't miss updates due to culling
        //
        R_SetupAliasFrame(e, paliashdr, &mut lerpdata);
        R_GetEntityLerpedTransform(
            e,
            lerpdata.origin.as_mut_ptr(),
            lerpdata.angles.as_mut_ptr(),
        );

        //
        // cull
        //
        if c::render::R_CullModelForEntity(e.cast::<c_void>()) {
            return None;
        }

        //
        // transform it
        //
        identity_matrix(&mut model_matrix);
        c::render::R_RotateForEntity(
            model_matrix.as_mut_ptr(),
            lerpdata.origin.as_mut_ptr(),
            lerpdata.angles.as_mut_ptr(),
            (*e).netstate.scale,
        );

        let viewent = ptr::addr_of!((*ptr::addr_of!(cl)).viewent).cast::<Entity>();
        let basefov = (*ptr::addr_of!(r_refdef)).basefov;
        let gun_fovscale = (*ptr::addr_of!(c::render::cl_gun_fovscale)).value;
        let mut fovscale = 1.0f32;
        // The `_ShowTris` variant omits the `cl_gun_fovscale.value` test.
        if e.cast_const() == viewent && basefov > 90.0 && (showtris || gun_fovscale != 0.0) {
            fovscale = (f64::from(basefov) * (0.5 * M_PI / 180.0)).tan() as f32;
            fovscale = 1.0 + (fovscale - 1.0) * gun_fovscale;
        }

        let mut translation = [0.0f32; 16];
        translation_matrix(
            &mut translation,
            hdr.scale_origin[0],
            hdr.scale_origin[1] * fovscale,
            hdr.scale_origin[2] * fovscale,
        );
        matrix_multiply(&mut model_matrix, &translation);
        let mut scale = [0.0f32; 16];
        scale_matrix(
            &mut scale,
            hdr.scale[0],
            hdr.scale[1] * fovscale,
            hdr.scale[2] * fovscale,
        );
        matrix_multiply(&mut model_matrix, &scale);

        Some((lerpdata, model_matrix))
    }
}

/// `R_DrawAliasModel` minus the `Mod_Extradata_CheckSkin` call (done by the
/// C wrapper in `r_alias_glue.c`).
///
/// # Safety
/// `cbx` is a live command-buffer context inside a render pass; `e` is an
/// alias entity, `paliashdr` its resolved extradata and `aliaspolys` a live
/// counter.
#[no_mangle]
pub unsafe extern "C" fn RAlias_DrawAliasModel(
    cbx: *mut CbContext,
    e: *mut Entity,
    paliashdr: *mut AliasHdr,
    aliaspolys: *mut c_int,
) {
    // SAFETY: the caller's contract; skin indices are range-checked before
    // indexing the texture tables.
    unsafe {
        let model: *mut QModel = (*e).model;
        let alphatest = ((*model).flags & MF_HOLEY) != 0;
        let mut lightcolor = [0.0f32; 3];
        let mut shadevector = [0.0f32; 3];

        let Some((lerpdata, model_matrix)) = setup_alias_model(e, paliashdr, false) else {
            return;
        };

        //
        // random stuff
        //
        let entalpha = if *ptr::addr_of!(c::render::r_lightmap_cheatsafe) {
            1.0
        } else {
            entalpha_decode((*e).alpha)
        };
        if entalpha == 0.0 {
            return;
        }

        //
        // set up lighting
        //
        setup_alias_lighting(e, &mut shadevector, &mut lightcolor);

        let cl_time = (*ptr::addr_of!(cl)).time;
        let entities = (*ptr::addr_of!(cl)).entities.cast::<Entity>();
        let maxclients = (*ptr::addr_of!(cl)).maxclients as usize;
        let vid_colormap = (*ptr::addr_of!(c::cl_parse::vid)).colormap;
        let nocolors = (*ptr::addr_of!(c::render::gl_nocolors)).value;
        let fullbrights = (*ptr::addr_of!(c::render::gl_fullbrights)).value;
        let fullbright_cheatsafe = *ptr::addr_of!(c::render::r_fullbright_cheatsafe);
        let lightmap_cheatsafe = *ptr::addr_of!(c::render::r_lightmap_cheatsafe);
        let r_fullbright = (*ptr::addr_of!(c::render::r_fullbright)).value;
        let greytexture = (*ptr::addr_of!(c::render::greytexture)).cast::<GlTexture>();

        let mut hdr = paliashdr;
        while !hdr.is_null() {
            //
            // set up textures
            //
            let anim = ((cl_time * 10.0) as c_int) & 3;
            let mut skinnum = (*e).skinnum;
            if skinnum >= (*hdr).numskins || skinnum < 0 {
                c::Con_DPrintf(
                    c"R_DrawAliasModel: no such skin # %d for '%s'\n".as_ptr(),
                    skinnum,
                    (*model).name.as_ptr(),
                );
                // ericw -- display skin 0 for winquake compatibility
                skinnum = 0;
            }
            let mut tx = (*hdr).gltextures[skinnum as usize][anim as usize].cast::<GlTexture>();
            let mut fb = (*hdr).fbtextures[skinnum as usize][anim as usize].cast::<GlTexture>();
            if (*e).colormap != vid_colormap
                && nocolors == 0.0
                && e.cast_const() >= entities.add(1).cast_const()
                && e.cast_const() <= entities.add(maxclients).cast_const()
            {
                let idx = e.offset_from(entities) as usize - 1;
                let pt = (*ptr::addr_of!(playertextures))[idx].cast::<GlTexture>();
                if !pt.is_null() {
                    tx = pt;
                }
            }
            if tx.is_null() {
                tx = greytexture;
                fb = ptr::null_mut();
            }
            if fullbrights == 0.0 {
                fb = ptr::null_mut();
            }

            //
            // draw it
            //
            if fullbright_cheatsafe {
                lightcolor = [0.5, 0.5, 0.5];
            }
            if lightmap_cheatsafe {
                tx = greytexture;
                fb = ptr::null_mut();
                if r_fullbright != 0.0 {
                    lightcolor = [1.0, 1.0, 1.0];
                }
            }

            gl_draw_alias_frame(
                cbx,
                hdr,
                lerpdata,
                tx,
                fb,
                &model_matrix,
                entalpha,
                alphatest,
                shadevector,
                lightcolor,
                0,
            );

            *aliaspolys += (*hdr).numtris;
            hdr = (*hdr).nextsurface.cast::<AliasHdr>();
        }
    }
}

/// `R_DrawAliasModel_ShowTris` minus the `Mod_Extradata_CheckSkin` call
/// (done by the C wrapper in `r_alias_glue.c`).
///
/// # Safety
/// As for [`RAlias_DrawAliasModel`].
#[no_mangle]
pub unsafe extern "C" fn RAlias_DrawAliasModel_ShowTris(
    cbx: *mut CbContext,
    e: *mut Entity,
    paliashdr: *mut AliasHdr,
) {
    // SAFETY: the caller's contract.
    unsafe {
        let Some((lerpdata, model_matrix)) = setup_alias_model(e, paliashdr, true) else {
            return;
        };
        let nulltexture = (*ptr::addr_of!(c::render::nulltexture)).cast::<GlTexture>();
        let showtris = (*ptr::addr_of!(c::render::r_showtris)).value as c_int;

        let mut hdr = paliashdr;
        while !hdr.is_null() {
            gl_draw_alias_frame(
                cbx,
                hdr,
                lerpdata,
                nulltexture,
                nulltexture,
                &model_matrix,
                0.0,
                false,
                [0.0; 3],
                [0.0; 3],
                showtris,
            );
            hdr = (*hdr).nextsurface.cast::<AliasHdr>();
        }
    }
}

/// `float r_avertexnormals[NUMVERTEXNORMALS][3]` (`r_alias.c:37` / `anorms.h`):
/// also read by `gl_mesh.rs` and the C `r_part.c`.
#[no_mangle]
pub static r_avertexnormals: [[f32; 3]; 162] = [
    [-0.525731, 0.000000, 0.850651],
    [-0.442863, 0.238856, 0.864188],
    [-0.295242, 0.000000, 0.955423],
    [-0.309017, 0.500000, 0.809017],
    [-0.162460, 0.262866, 0.951056],
    [0.000000, 0.000000, 1.000000],
    [0.000000, 0.850651, 0.525731],
    [-0.147621, 0.716567, 0.681718],
    [0.147621, 0.716567, 0.681718],
    [0.000000, 0.525731, 0.850651],
    [0.309017, 0.500000, 0.809017],
    [0.525731, 0.000000, 0.850651],
    [0.295242, 0.000000, 0.955423],
    [0.442863, 0.238856, 0.864188],
    [0.162460, 0.262866, 0.951056],
    [-0.681718, 0.147621, 0.716567],
    [-0.809017, 0.309017, 0.500000],
    [-0.587785, 0.425325, 0.688191],
    [-0.850651, 0.525731, 0.000000],
    [-0.864188, 0.442863, 0.238856],
    [-0.716567, 0.681718, 0.147621],
    [-0.688191, 0.587785, 0.425325],
    [-0.500000, 0.809017, 0.309017],
    [-0.238856, 0.864188, 0.442863],
    [-0.425325, 0.688191, 0.587785],
    [-0.716567, 0.681718, -0.147621],
    [-0.500000, 0.809017, -0.309017],
    [-0.525731, 0.850651, 0.000000],
    [0.000000, 0.850651, -0.525731],
    [-0.238856, 0.864188, -0.442863],
    [0.000000, 0.955423, -0.295242],
    [-0.262866, 0.951056, -0.162460],
    [0.000000, 1.000000, 0.000000],
    [0.000000, 0.955423, 0.295242],
    [-0.262866, 0.951056, 0.162460],
    [0.238856, 0.864188, 0.442863],
    [0.262866, 0.951056, 0.162460],
    [0.500000, 0.809017, 0.309017],
    [0.238856, 0.864188, -0.442863],
    [0.262866, 0.951056, -0.162460],
    [0.500000, 0.809017, -0.309017],
    [0.850651, 0.525731, 0.000000],
    [0.716567, 0.681718, 0.147621],
    [0.716567, 0.681718, -0.147621],
    [0.525731, 0.850651, 0.000000],
    [0.425325, 0.688191, 0.587785],
    [0.864188, 0.442863, 0.238856],
    [0.688191, 0.587785, 0.425325],
    [0.809017, 0.309017, 0.500000],
    [0.681718, 0.147621, 0.716567],
    [0.587785, 0.425325, 0.688191],
    [0.955423, 0.295242, 0.000000],
    [1.000000, 0.000000, 0.000000],
    [0.951056, 0.162460, 0.262866],
    [0.850651, -0.525731, 0.000000],
    [0.955423, -0.295242, 0.000000],
    [0.864188, -0.442863, 0.238856],
    [0.951056, -0.162460, 0.262866],
    [0.809017, -0.309017, 0.500000],
    [0.681718, -0.147621, 0.716567],
    [0.850651, 0.000000, 0.525731],
    [0.864188, 0.442863, -0.238856],
    [0.809017, 0.309017, -0.500000],
    [0.951056, 0.162460, -0.262866],
    [0.525731, 0.000000, -0.850651],
    [0.681718, 0.147621, -0.716567],
    [0.681718, -0.147621, -0.716567],
    [0.850651, 0.000000, -0.525731],
    [0.809017, -0.309017, -0.500000],
    [0.864188, -0.442863, -0.238856],
    [0.951056, -0.162460, -0.262866],
    [0.147621, 0.716567, -0.681718],
    [0.309017, 0.500000, -0.809017],
    [0.425325, 0.688191, -0.587785],
    [0.442863, 0.238856, -0.864188],
    [0.587785, 0.425325, -0.688191],
    [0.688191, 0.587785, -0.425325],
    [-0.147621, 0.716567, -0.681718],
    [-0.309017, 0.500000, -0.809017],
    [0.000000, 0.525731, -0.850651],
    [-0.525731, 0.000000, -0.850651],
    [-0.442863, 0.238856, -0.864188],
    [-0.295242, 0.000000, -0.955423],
    [-0.162460, 0.262866, -0.951056],
    [0.000000, 0.000000, -1.000000],
    [0.295242, 0.000000, -0.955423],
    [0.162460, 0.262866, -0.951056],
    [-0.442863, -0.238856, -0.864188],
    [-0.309017, -0.500000, -0.809017],
    [-0.162460, -0.262866, -0.951056],
    [0.000000, -0.850651, -0.525731],
    [-0.147621, -0.716567, -0.681718],
    [0.147621, -0.716567, -0.681718],
    [0.000000, -0.525731, -0.850651],
    [0.309017, -0.500000, -0.809017],
    [0.442863, -0.238856, -0.864188],
    [0.162460, -0.262866, -0.951056],
    [0.238856, -0.864188, -0.442863],
    [0.500000, -0.809017, -0.309017],
    [0.425325, -0.688191, -0.587785],
    [0.716567, -0.681718, -0.147621],
    [0.688191, -0.587785, -0.425325],
    [0.587785, -0.425325, -0.688191],
    [0.000000, -0.955423, -0.295242],
    [0.000000, -1.000000, 0.000000],
    [0.262866, -0.951056, -0.162460],
    [0.000000, -0.850651, 0.525731],
    [0.000000, -0.955423, 0.295242],
    [0.238856, -0.864188, 0.442863],
    [0.262866, -0.951056, 0.162460],
    [0.500000, -0.809017, 0.309017],
    [0.716567, -0.681718, 0.147621],
    [0.525731, -0.850651, 0.000000],
    [-0.238856, -0.864188, -0.442863],
    [-0.500000, -0.809017, -0.309017],
    [-0.262866, -0.951056, -0.162460],
    [-0.850651, -0.525731, 0.000000],
    [-0.716567, -0.681718, -0.147621],
    [-0.716567, -0.681718, 0.147621],
    [-0.525731, -0.850651, 0.000000],
    [-0.500000, -0.809017, 0.309017],
    [-0.238856, -0.864188, 0.442863],
    [-0.262866, -0.951056, 0.162460],
    [-0.864188, -0.442863, 0.238856],
    [-0.809017, -0.309017, 0.500000],
    [-0.688191, -0.587785, 0.425325],
    [-0.681718, -0.147621, 0.716567],
    [-0.442863, -0.238856, 0.864188],
    [-0.587785, -0.425325, 0.688191],
    [-0.309017, -0.500000, 0.809017],
    [-0.147621, -0.716567, 0.681718],
    [-0.425325, -0.688191, 0.587785],
    [-0.162460, -0.262866, 0.951056],
    [0.442863, -0.238856, 0.864188],
    [0.162460, -0.262866, 0.951056],
    [0.309017, -0.500000, 0.809017],
    [0.147621, -0.716567, 0.681718],
    [0.000000, -0.525731, 0.850651],
    [0.425325, -0.688191, 0.587785],
    [0.587785, -0.425325, 0.688191],
    [0.688191, -0.587785, 0.425325],
    [-0.955423, 0.295242, 0.000000],
    [-0.951056, 0.162460, 0.262866],
    [-1.000000, 0.000000, 0.000000],
    [-0.850651, 0.000000, 0.525731],
    [-0.955423, -0.295242, 0.000000],
    [-0.951056, -0.162460, 0.262866],
    [-0.864188, 0.442863, -0.238856],
    [-0.951056, 0.162460, -0.262866],
    [-0.809017, 0.309017, -0.500000],
    [-0.864188, -0.442863, -0.238856],
    [-0.951056, -0.162460, -0.262866],
    [-0.809017, -0.309017, -0.500000],
    [-0.681718, 0.147621, -0.716567],
    [-0.681718, -0.147621, -0.716567],
    [-0.850651, 0.000000, -0.525731],
    [-0.688191, 0.587785, -0.425325],
    [-0.587785, 0.425325, -0.688191],
    [-0.425325, 0.688191, -0.587785],
    [-0.425325, -0.688191, -0.587785],
    [-0.587785, -0.425325, -0.688191],
    [-0.688191, -0.587785, -0.425325],
];
