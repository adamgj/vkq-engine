//! `Quake/r_part.c` -- the classic particle *rendering* half (Rust migration
//! Phase 8 M8; `r_part.c:54-221` and `r_part.c:951-1106`).
//!
//! The simulation half was ported in Phase 7 (`r_part.rs`) and the rendering
//! half stayed C in `Quake/r_part_glue.c` until the renderer's turn. This
//! module closes that split under `-Duse_rust_render`: it owns the particle
//! textures, `texturescalefactor` and the particle index buffer, and exports
//! `R_DrawParticles` / `R_DrawParticles_ShowTris` under their C names for
//! `gl_rmain.c`. The pool (`active_particles`, `r_numparticles`) and the two
//! cvars stay in the glue TU (ADR-007), read through `quake_c_sys::r_part`.
//!
//! Gated on both `host` and `render`: `build-rs-chost` compiles the whole of
//! `r_part.c` (both halves in C), so nothing here may export then.
//!
//! COMPAT: ADR-010. `scale` is a `float` in C but `1 + scale * 0.004` is
//! evaluated in `double` and narrowed on assignment, so the port evaluates
//! it in `f64` and casts once rather than rounding twice in `f32`.

use core::ffi::{c_int, CStr};
use core::ptr;
use core::sync::atomic::{AtomicU32, Ordering};

use ash::vk;
use quake_c_sys as c;
use quake_c_sys::cvar_t;
use quake_c_sys::r_part as g;
use quake_math::mathlib::{vector_ma, vector_scale, Vec3};
use quake_render::cb::{self, CmdProcs};
use quake_types::render::{
    BasicVertex, CbContext, GlTexture, SrcFormat, TEXPREF_ALPHA, TEXPREF_LINEAR, TEXPREF_NEAREST,
    TEXPREF_PERSIST,
};

use crate::gl_rmisc::{
    device, num_vulkan_dynbuf_allocations, total_device_vulkan_allocation_size, vg, with_ctx, DYN,
    STAGING,
};
use crate::gl_texmgr::TexMgr_LoadImage;

/// `gltexture_t *particletexture, *particletexture1, ... *particletexture4`
/// (`r_part.c:44`). `particletexture1` is read by `r_part_fte_glue.c`.
#[no_mangle]
pub static mut particletexture: *mut GlTexture = ptr::null_mut();
#[no_mangle]
pub static mut particletexture1: *mut GlTexture = ptr::null_mut();
#[no_mangle]
pub static mut particletexture2: *mut GlTexture = ptr::null_mut();
#[no_mangle]
pub static mut particletexture3: *mut GlTexture = ptr::null_mut();
#[no_mangle]
pub static mut particletexture4: *mut GlTexture = ptr::null_mut();

/// `static float texturescalefactor` (`r_part.c:45`) -- stored as bits so the
/// cvar callback and the draw path need no `static mut` access.
static TEXTURESCALEFACTOR: AtomicU32 = AtomicU32::new(0);

/// `static VkBuffer particle_index_buffer` (`r_part.c:52`).
static mut PARTICLE_INDEX_BUFFER: vk::Buffer = vk::Buffer::null();

/// The three `static byte particleN_data[]` arrays of `R_InitParticleTextures`
/// (`r_part.c:73-75`): `TexMgr_ReloadImage` re-reads an in-memory source
/// through `source_offset`, so the pixels must outlive the upload.
static mut PARTICLE1_DATA: [u8; 64 * 64 * 4] = [0; 64 * 64 * 4];
static mut PARTICLE2_DATA: [u8; 2 * 2 * 4] = [0; 2 * 2 * 4];
static mut PARTICLE3_DATA: [u8; 64 * 64 * 4] = [0; 64 * 64 * 4];

fn set_texturescalefactor(value: f32) {
    TEXTURESCALEFACTOR.store(value.to_bits(), Ordering::Relaxed);
}

fn texturescalefactor() -> f32 {
    f32::from_bits(TEXTURESCALEFACTOR.load(Ordering::Relaxed))
}

/// `R_ParticleTextureLookup` -- johnfitz -- generate nice antialiased 32x32
/// circle for particles.
#[no_mangle]
pub extern "C" fn R_ParticleTextureLookup(x: c_int, y: c_int, sharpness: c_int) -> c_int {
    let x = x - 16;
    let y = y - 16;
    let mut r = x * x + y * y; // distance from point x,y to circle origin, squared
    r = if r > 255 { 255 } else { r };
    let a = sharpness * (255 - r); // alpha value to return
    a.min(255)
}

/// Fills a `64 x 64` (or `2 x 2`) white RGBA image whose alpha comes from
/// `alpha(x, y)`, in the C's `x`-major order.
fn fill_particle_image(dst: &mut [u8], side: c_int, alpha: impl Fn(c_int, c_int) -> u8) {
    let mut i = 0;
    for x in 0..side {
        for y in 0..side {
            dst[i] = 255;
            dst[i + 1] = 255;
            dst[i + 2] = 255;
            dst[i + 3] = alpha(x, y);
            i += 4;
        }
    }
}

/// `R_InitParticleTextures` -- johnfitz -- rewritten.
///
/// # Safety
/// Main thread during renderer init (`R_InitParticles`), after the texture
/// manager is up.
#[no_mangle]
pub unsafe extern "C" fn R_InitParticleTextures() {
    // SAFETY: the caller's contract; the three data arrays are only touched
    // here and by the texture manager's reload, which reads them.
    unsafe {
        // particle texture 1 -- circle
        let data1 = &mut *ptr::addr_of_mut!(PARTICLE1_DATA);
        fill_particle_image(data1, 64, |x, y| R_ParticleTextureLookup(x, y, 8) as u8);
        particletexture1 = load_particle_texture(c"particle1", 64, data1, TEXPREF_LINEAR);

        // particle texture 2 -- square
        let data2 = &mut *ptr::addr_of_mut!(PARTICLE2_DATA);
        fill_particle_image(data2, 2, |x, y| if x != 0 || y != 0 { 0 } else { 255 });
        particletexture2 = load_particle_texture(c"particle2", 2, data2, TEXPREF_NEAREST);

        // particle texture 3 -- blob
        let data3 = &mut *ptr::addr_of_mut!(PARTICLE3_DATA);
        fill_particle_image(data3, 64, |x, y| R_ParticleTextureLookup(x, y, 2) as u8);
        particletexture3 = load_particle_texture(c"particle3", 64, data3, TEXPREF_LINEAR);

        // set default
        particletexture = particletexture1;
        set_texturescalefactor(1.27);
    }
}

/// One `TexMgr_LoadImage (NULL, name, side, side, SRC_RGBA, data, "",
/// (src_offset_t)data, TEXPREF_PERSIST | TEXPREF_ALPHA | filter)` call.
///
/// # Safety
/// `data` is `side * side * 4` bytes that outlive the texture (see the statics).
unsafe fn load_particle_texture(
    name: &CStr,
    side: c_int,
    data: &mut [u8],
    filter: u32,
) -> *mut GlTexture {
    let data = data.as_mut_ptr();
    // SAFETY: the caller's contract; `name` and `""` are NUL-terminated.
    unsafe {
        TexMgr_LoadImage(
            ptr::null_mut(),
            name.as_ptr(),
            side,
            side,
            SrcFormat::Rgba as c_int,
            data,
            c"".as_ptr(),
            data as usize,
            TEXPREF_PERSIST | TEXPREF_ALPHA | filter,
        )
    }
}

/// `R_SetParticleTexture_f` -- johnfitz. The `r_particles` cvar callback;
/// `r_part.rs` hands it to `Cvar_SetCallback` in place of the glue's
/// `RPart_Glue_SetParticleTexture_f`.
///
/// # Safety
/// `r_particles` is the glue-owned cvar; the textures were loaded by
/// [`R_InitParticleTextures`] (or are still null, which the C tolerates too).
#[no_mangle]
pub unsafe extern "C" fn R_SetParticleTexture_f(_var: *mut cvar_t) {
    // SAFETY: the caller's contract.
    unsafe {
        match (*ptr::addr_of!(g::r_particles)).value as c_int {
            1 => {
                particletexture = particletexture1;
                set_texturescalefactor(1.27);
            }
            2 => {
                particletexture = particletexture2;
                set_texturescalefactor(1.0);
            }
            //	case 3:
            //		particletexture = particletexture3;
            //		texturescalefactor = 1.5;
            //		break;
            _ => {}
        }
    }
}

/// `R_InitParticleIndexBuffer`: a device-local index buffer holding six
/// `uint16_t` indices per particle quad, filled through the staging ring.
///
/// # Safety
/// Main thread during renderer init, after the device and the staging
/// buffers are up; `r_numparticles` is set.
#[no_mangle]
pub unsafe extern "C" fn R_InitParticleIndexBuffer() {
    // SAFETY: the caller's contract.
    unsafe {
        let r_numparticles = *ptr::addr_of!(g::r_numparticles);
        // 6 indices per particle quad
        let particle_index_buffer_size =
            (r_numparticles as u32).wrapping_mul(size_of::<u16>() as u32 * 6);

        let device = device();
        let buffer_create_info = vk::BufferCreateInfo::default()
            .size(u64::from(particle_index_buffer_size))
            .usage(vk::BufferUsageFlags::INDEX_BUFFER | vk::BufferUsageFlags::TRANSFER_DST);
        let buffer = match device.create_buffer(&buffer_create_info, None) {
            Ok(b) => b,
            Err(err) => c::Sys_Error(c"vkCreateBuffer failed with code %i".as_ptr(), err.as_raw()),
        };
        PARTICLE_INDEX_BUFFER = buffer;
        with_ctx(|ctx| ctx.name_object(buffer, c"Particle index buffer"));

        let memory_requirements = device.get_buffer_memory_requirements(buffer);
        let aligned_size = memory_requirements
            .size
            .div_ceil(memory_requirements.alignment)
            * memory_requirements.alignment;

        let memory_type_index = with_ctx(|ctx| {
            ctx.memory_type_from_properties(
                memory_requirements.memory_type_bits,
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
                vk::MemoryPropertyFlags::empty(),
            )
        });
        let memory_allocate_info = vk::MemoryAllocateInfo::default()
            .allocation_size(aligned_size)
            .memory_type_index(memory_type_index);

        num_vulkan_dynbuf_allocations.fetch_add(1, Ordering::SeqCst);
        total_device_vulkan_allocation_size.fetch_add(memory_requirements.size, Ordering::SeqCst);
        let memory = match device.allocate_memory(&memory_allocate_info, None) {
            Ok(m) => m,
            Err(err) => c::Sys_Error(
                c"vkAllocateMemory failed with code %i".as_ptr(),
                err.as_raw(),
            ),
        };
        with_ctx(|ctx| ctx.name_object(memory, c"Particle index buffer"));

        if let Err(err) = device.bind_buffer_memory(buffer, memory, 0) {
            c::Sys_Error(
                c"vkBindBufferMemory failed with code %i".as_ptr(),
                err.as_raw(),
            );
        }

        let staging = with_ctx(|ctx| STAGING.allocate(ctx, particle_index_buffer_size as i32, 1));
        let region = vk::BufferCopy {
            src_offset: staging.buffer_offset as vk::DeviceSize,
            dst_offset: 0,
            size: u64::from(particle_index_buffer_size),
        };
        device.cmd_copy_buffer(staging.command_buffer, staging.buffer, buffer, &[region]);

        STAGING.begin_copy();
        let staging_indices = staging.data.cast::<u16>();
        for i in 0..r_numparticles.max(0) as usize {
            let base = (i * 4) as u16;
            let q = staging_indices.add(i * 6);
            *q = base;
            *q.add(1) = base.wrapping_add(1);
            *q.add(2) = base.wrapping_add(2);
            *q.add(3) = base;
            *q.add(4) = base.wrapping_add(2);
            *q.add(5) = base.wrapping_add(3);
        }
        STAGING.end_copy();
    }
}

/// `R_InitParticles`' rendering tail (`r_part.c:250-254`), called by
/// `r_part.rs` in place of the glue's `RPart_Glue_InitRender`.
///
/// # Safety
/// As for [`R_InitParticleTextures`] and [`R_InitParticleIndexBuffer`].
pub unsafe fn init_render() {
    // SAFETY: `no_rendering` is a process-wide C flag set before host init.
    unsafe {
        if !*ptr::addr_of!(c::host::no_rendering) {
            R_InitParticleTextures(); // johnfitz
            R_InitParticleIndexBuffer();
        }
    }
}

/// Writes one `basicvertex_t`.
///
/// # Safety
/// `v` is a live vertex slot.
#[inline]
unsafe fn put_vertex(v: *mut BasicVertex, pos: &Vec3, s: f32, t: f32, c: &[u8; 4]) {
    // SAFETY: the caller's contract.
    unsafe {
        (*v).position = *pos;
        (*v).texcoord = [s, t];
        (*v).color = [c[0], c[1], c[2], 255];
    }
}

/// `R_DrawParticlesFaces`.
///
/// # Safety
/// `cbx` is recording inside a render pass with a particle pipeline bound;
/// the particle pool is stable for the frame.
unsafe fn draw_particles_faces(cbx: *mut CbContext) {
    // SAFETY: the caller's contract; the view vectors and the palette are
    // frame-stable C globals.
    unsafe {
        if (*ptr::addr_of!(g::r_particles)).value == 0.0 {
            return;
        }

        let active_particles = *ptr::addr_of!(g::active_particles);
        if active_particles.is_null() {
            return;
        }

        let quad = (*ptr::addr_of!(g::r_quadparticles)).value != 0.0;
        let vup: Vec3 = *ptr::addr_of!(c::host::vup);
        let vright: Vec3 = *ptr::addr_of!(c::host::vright);
        let vpn: Vec3 = *ptr::addr_of!(c::cl_main::vpn);
        let r_origin: Vec3 = *ptr::addr_of!(c::host::r_origin);

        let mut up = Vec3::default();
        let mut right = Vec3::default();
        let texcoord_scale: f32 = if quad {
            vector_scale(&vup, 0.75, &mut up);
            vector_scale(&vright, 0.75, &mut right);
            0.5
        } else {
            vector_scale(&vup, 1.5, &mut up);
            vector_scale(&vright, 1.5, &mut right);
            1.0
        };

        let mut up_right = Vec3::default();
        for (o, (u, r)) in up_right.iter_mut().zip(up.iter().zip(right.iter())) {
            *o = u + r;
        }

        let mut num_particles: c_int = 0;
        let mut p = active_particles;
        while !p.is_null() {
            num_particles += 1;
            p = (*p).next;
        }
        AtomicU32::from_ptr(ptr::addr_of_mut!(c::render::rs_particles))
            .fetch_add(num_particles as u32, Ordering::SeqCst);

        let verts_per_particle: usize = if quad { 4 } else { 3 };
        let a = with_ctx(|ctx| {
            DYN.vertex_allocate(
                ctx,
                (num_particles as usize * verts_per_particle * size_of::<BasicVertex>()) as u32,
            )
        });
        let vertices = a.data.cast::<BasicVertex>();

        let texturescalefactor = texturescalefactor();
        let mut current_vertex = 0usize;
        let mut p = active_particles;
        while !p.is_null() {
            let org: Vec3 = (*p).org;
            // hack a scale up to keep particles from disapearing
            let mut scale = (org[0] - r_origin[0]) * vpn[0]
                + (org[1] - r_origin[1]) * vpn[1]
                + (org[2] - r_origin[2]) * vpn[2];
            if scale < 20.0 {
                scale = (1.0 + 0.08_f64) as f32; // johnfitz -- added .08 to be consistent
            } else {
                scale = (1.0 + f64::from(scale) * 0.004) as f32;
            }

            scale *= texturescalefactor; // johnfitz -- compensate for apparent size of different particle textures

            let color = (*ptr::addr_of!(c::render::d_8to24table))[(*p).color as c_int as usize];
            let c = color.to_ne_bytes();

            put_vertex(vertices.add(current_vertex), &org, 0.0, 0.0, &c);
            current_vertex += 1;

            let mut p_up = Vec3::default();
            vector_ma(&org, scale, &up, &mut p_up);
            put_vertex(vertices.add(current_vertex), &p_up, texcoord_scale, 0.0, &c);
            current_vertex += 1;

            if quad {
                let mut p_up_right = Vec3::default();
                vector_ma(&org, scale, &up_right, &mut p_up_right);
                put_vertex(
                    vertices.add(current_vertex),
                    &p_up_right,
                    texcoord_scale,
                    texcoord_scale,
                    &c,
                );
                current_vertex += 1;
            }

            let mut p_right = Vec3::default();
            vector_ma(&org, scale, &right, &mut p_right);
            put_vertex(
                vertices.add(current_vertex),
                &p_right,
                0.0,
                texcoord_scale,
                &c,
            );
            current_vertex += 1;

            p = (*p).next;
        }

        let device = device();
        let cb = (*cbx).cb;
        device.cmd_bind_vertex_buffers(cb, 0, &[a.buffer], &[a.buffer_offset]);
        with_ctx(|ctx| {
            let procs = CmdProcs::new(ctx.vg);
            if quad {
                device.cmd_bind_index_buffer(cb, PARTICLE_INDEX_BUFFER, 0, vk::IndexType::UINT16);
                cb::draw_indexed(&procs, cb, num_particles as u32 * 6, 1, 0, 0, 0);
            } else {
                cb::draw(&procs, cb, num_particles as u32 * 3, 1, 0, 0);
            }
        });
    }
}

/// `R_DrawParticles` -- johnfitz -- moved all non-drawing code to
/// `CL_RunParticles`.
///
/// # Safety
/// `cbx` is a live command-buffer context inside a render pass; the
/// particle textures were loaded by [`R_InitParticleTextures`].
#[no_mangle]
pub unsafe extern "C" fn R_DrawParticles(cbx: *mut CbContext) {
    with_ctx(|ctx| {
        let procs = CmdProcs::new(ctx.vg);
        // SAFETY: the caller's contract.
        let idx = unsafe { (*cbx).render_pass_index };
        let main = vg!(ctx, particle_pipeline);
        let oit = vg!(ctx, particle_oit_pipeline);
        let moment = vg!(ctx, particle_mboit_moment_pipeline);
        let composite = vg!(ctx, particle_mboit_composite_pipeline);
        let pipeline = cb::pipeline_for_render_pass(idx, main, oit, moment, composite);
        let layout = vg!(ctx, basic_pipeline_layout.handle);
        // SAFETY: the caller's contract; `particletexture` is live.
        unsafe {
            cb::begin_debug_utils_label(&procs, &*cbx, c"Particles");
            cb::bind_pipeline(&procs, &mut *cbx, vk::PipelineBindPoint::GRAPHICS, pipeline);
            let set = (*particletexture).descriptor_set;
            device().cmd_bind_descriptor_sets(
                (*cbx).cb,
                vk::PipelineBindPoint::GRAPHICS,
                layout,
                0,
                &[set],
                &[],
            );
        }
    });

    // SAFETY: the caller's contract.
    unsafe {
        draw_particles_faces(cbx);
        with_ctx(|ctx| cb::end_debug_utils_label(&CmdProcs::new(ctx.vg), &*cbx));
    }
}

/// `R_DrawParticles_ShowTris` -- johnfitz.
///
/// # Safety
/// As for [`R_DrawParticles`].
#[no_mangle]
pub unsafe extern "C" fn R_DrawParticles_ShowTris(cbx: *mut CbContext) {
    // SAFETY: the caller's contract; `r_showtris` is defined by gl_rmain.c
    // for the whole process.
    let (variant, showtris_value) = unsafe {
        (
            cb::main_pass_pipeline_variant((*cbx).render_pass_index),
            (*ptr::addr_of!(c::render::r_showtris)).value,
        )
    };

    with_ctx(|ctx| {
        let procs = CmdProcs::new(ctx.vg);
        let showtris = vg!(ctx, showtris_pipeline[variant]);
        let showtris_depth = vg!(ctx, showtris_depth_test_pipeline[variant]);
        let pipeline = if showtris_value == 1.0 {
            showtris
        } else {
            showtris_depth
        };
        // SAFETY: `cbx` is the caller's live context (contract above).
        unsafe {
            cb::bind_pipeline(&procs, &mut *cbx, vk::PipelineBindPoint::GRAPHICS, pipeline);
        }
    });

    // SAFETY: as above.
    unsafe { draw_particles_faces(cbx) }
}
