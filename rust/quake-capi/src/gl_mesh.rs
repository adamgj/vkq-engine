//! `gl_mesh.c` (Phase 8 M8, ray tracing M10): the alias-model mesh heap,
//! the GPU buffer upload for MDL/MD3/MD5 headers, the deferred buffer/BLAS
//! garbage lists, `GL_MakeAliasModelDisplayLists`, and the per-entity
//! bottom-level acceleration structures (`R_AllocateEntityBLAS`,
//! `R_UpdateAnimatedBLASes`) that `r_brush.rs` instances into the TLAS.
//! [`AsProcs`] wraps the `VK_KHR_acceleration_structure` entry points both
//! modules share.
//!
//! Headless mode (`no_rendering`) has no `VkDevice`: `GLMesh_UploadBuffers`
//! computes `vbostofs` and returns before touching the device, and the
//! delete paths return on the null vertex buffer, so nothing here reaches
//! `device()` headless.

#![allow(non_snake_case, non_upper_case_globals)]

use core::ffi::{c_int, c_void};
use core::ptr;
use std::collections::HashMap;

use ash::vk::{self, Handle};
use quake_c_sys as c;
use quake_render::cb::{self, CmdProcs};
use quake_render::heap::Allocation;
use quake_render::rmisc::{allocate_descriptor_set, free_descriptor_set, q_align, Engine};
use quake_types::host::{ClientState, Entity};
use quake_types::model_mem::{
    AliasHdr, AliasMesh, JointPose, MTriangle, Md3XyzNormal, Md5Vert, Md5Vert8, QModel,
    MAXALIASFRAMES, MAXALIASVERTS, MAX_SKINS, MOD_ALIAS, PV_MD5, PV_MD5_8, PV_QUAKE1, PV_QUAKE3,
    PV_SIZE,
};
use quake_types::modelgen::{StVert, TriVertX};
use quake_types::render::{
    CbContext, GlHeapStats, LerpData, MeshInterpolatePushConstants, MeshSt, MeshXyz,
    SkinningPushConstants, VulkanDescSetLayout,
};

use crate::gl_heap::{GL_HeapAllocate, GL_HeapCreate, GL_HeapFree, GL_HeapGetStats, GlHeap};
use crate::gl_rmisc::{device, num_vulkan_mesh_allocations, vg, vulkan_globals, with_ctx, STAGING};
use crate::r_alias::R_SetupAliasFrame;
use crate::r_brush::{
    as_scratch_buffer, as_scratch_buffer_size, entalpha_decode, R_EnsureASScratchBufferSize,
    ENTALPHA_DEFAULT,
};

extern "C" {
    /// `client_state_t cl` (ADR-007 row closed in Phase 7).
    static mut cl: ClientState;
    /// The alias loader's scratch arrays (`model_parse.c`, declared in
    /// `gl_model.h`); `GL_MakeAliasModelDisplayLists` reads them right after
    /// `Mod_LoadAliasModel` filled them.
    static mut stverts: [StVert; MAXALIASVERTS as usize];
    static mut triangles: *mut MTriangle;
    static mut poseverts: [*mut TriVertX; MAXALIASFRAMES as usize];
}

const MESH_HEAP_SIZE_MB: u64 = 16;
const MESH_HEAP_PAGE_SIZE: u32 = 4096;
const MESH_HEAP_NAME: &core::ffi::CStr = c"Mesh heap";
/// `VULKAN_MEMORY_TYPE_DEVICE`.
const VULKAN_MEMORY_TYPE_DEVICE: c_int = 1;

/// `static glheap_t *mesh_buffer_heap`.
static mut mesh_buffer_heap: *mut GlHeap = ptr::null_mut();

#[derive(Clone, Copy)]
struct BufferGarbage {
    buffer: vk::Buffer,
    allocation: *mut Allocation,
    desc_set: vk::DescriptorSet,
    desc_set_layout: *const VulkanDescSetLayout,
}

#[derive(Clone, Copy)]
struct BlasGarbage {
    blas: vk::AccelerationStructureKHR,
    buffer: vk::Buffer,
    allocation: *mut Allocation,
}

static mut CURRENT_GARBAGE_INDEX: usize = 0;
static mut BUFFER_GARBAGE: [Vec<BufferGarbage>; 2] = [Vec::new(), Vec::new()];
static mut BLAS_GARBAGE: [Vec<BlasGarbage>; 2] = [Vec::new(), Vec::new()];

fn mesh_counter() -> *mut c_void {
    ptr::addr_of!(num_vulkan_mesh_allocations)
        .cast_mut()
        .cast::<c_void>()
}

/// `AddBufferGarbage` (the unused `descriptor_set` parameter dropped).
///
/// # Safety
/// Main thread only (the garbage lists are plain statics, as in C).
unsafe fn add_buffer_garbage(
    buffer: vk::Buffer,
    allocation: *mut Allocation,
    desc_set: vk::DescriptorSet,
    desc_set_layout: *const VulkanDescSetLayout,
) {
    // SAFETY: the caller's contract.
    unsafe {
        let idx = CURRENT_GARBAGE_INDEX;
        (*ptr::addr_of_mut!(BUFFER_GARBAGE))[idx].push(BufferGarbage {
            buffer,
            allocation,
            desc_set,
            desc_set_layout,
        });
    }
}

/// `AddBLASGarbage`.
///
/// # Safety
/// Main thread only.
unsafe fn add_blas_garbage(
    blas: vk::AccelerationStructureKHR,
    buffer: vk::Buffer,
    allocation: *mut Allocation,
) {
    // SAFETY: the caller's contract.
    unsafe {
        let idx = CURRENT_GARBAGE_INDEX;
        (*ptr::addr_of_mut!(BLAS_GARBAGE))[idx].push(BlasGarbage {
            blas,
            buffer,
            allocation,
        });
    }
}

/// `R_InitMeshHeap`: a throwaway 16-byte buffer picks the memory type the
/// mesh buffers will need, then the 16 MiB heap is created on it.
#[no_mangle]
pub extern "C" fn R_InitMeshHeap() {
    with_ctx(|ctx| {
        let ray_query = vg!(ctx, ray_query);
        let mut usage = vk::BufferUsageFlags::VERTEX_BUFFER
            | vk::BufferUsageFlags::INDEX_BUFFER
            | vk::BufferUsageFlags::STORAGE_BUFFER
            | vk::BufferUsageFlags::TRANSFER_DST;
        if ray_query {
            usage |= vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR;
        }
        let info = vk::BufferCreateInfo::default().size(16).usage(usage);
        // SAFETY: `info` is complete; the device is live (M6 created it).
        let dummy = match unsafe { device().create_buffer(&info, None) } {
            Ok(b) => b,
            Err(err) => ctx.vk_fail("vkCreateBuffer", err),
        };
        // SAFETY: `dummy` is a live buffer.
        let reqs = unsafe { device().get_buffer_memory_requirements(dummy) };
        let memory_type_index = ctx.memory_type_from_properties(
            reqs.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
            vk::MemoryPropertyFlags::empty(),
        );
        let heap_size = MESH_HEAP_SIZE_MB * 1024 * 1024;
        let heap = GL_HeapCreate(
            heap_size,
            MESH_HEAP_PAGE_SIZE,
            memory_type_index,
            VULKAN_MEMORY_TYPE_DEVICE,
            ray_query,
            MESH_HEAP_NAME.as_ptr(),
        );
        // SAFETY: main-thread renderer init; `dummy` is unused elsewhere.
        unsafe {
            mesh_buffer_heap = heap;
            device().destroy_buffer(dummy, None);
        }
    });
}

/// `R_GetMeshHeapStats`.
#[no_mangle]
pub extern "C" fn R_GetMeshHeapStats() -> *mut GlHeapStats {
    // SAFETY: `mesh_buffer_heap` was created by `R_InitMeshHeap`; the stats
    // pointer is read on the main thread only.
    unsafe { GL_HeapGetStats(mesh_buffer_heap) }
}

/// `R_CollectMeshBufferGarbage`: flips the two-slot ring and destroys the
/// buffers, descriptor sets and BLASes queued two frames ago.
#[no_mangle]
pub extern "C" fn R_CollectMeshBufferGarbage() {
    let destroy_blas = with_ctx(|ctx| vg!(ctx, vk_destroy_acceleration_structure));
    // SAFETY: main thread, between frames (the C call site); every queued
    // handle was live when queued and is destroyed exactly once here.
    unsafe {
        CURRENT_GARBAGE_INDEX = (CURRENT_GARBAGE_INDEX + 1) % 2;
        let idx = CURRENT_GARBAGE_INDEX;
        let buffers = core::mem::take(&mut (*ptr::addr_of_mut!(BUFFER_GARBAGE))[idx]);
        if !buffers.is_empty() {
            for g in &buffers {
                device().destroy_buffer(g.buffer, None);
                GL_HeapFree(mesh_buffer_heap, g.allocation, mesh_counter());
                if g.desc_set != vk::DescriptorSet::null() {
                    with_ctx(|ctx| free_descriptor_set(ctx, g.desc_set, &*g.desc_set_layout));
                }
            }
        }
        let blases = core::mem::take(&mut (*ptr::addr_of_mut!(BLAS_GARBAGE))[idx]);
        if !blases.is_empty() {
            let vk_device = (*ptr::addr_of!(vulkan_globals)).device;
            for g in &blases {
                if let Some(destroy) = destroy_blas {
                    destroy(vk_device, g.blas, ptr::null());
                }
                device().destroy_buffer(g.buffer, None);
                GL_HeapFree(mesh_buffer_heap, g.allocation, mesh_counter());
            }
        }
    }
}

/// `GL_MakeAliasModelDisplayLists` (MH / RMQEngine): dedupes the
/// (vertex, s, t) triples into the VBO vertex list and index list, then
/// uploads. The hash map keys on the same three fields `AliasMeshHash`
/// hashes (the `st` floats compared bitwise, as `HashMap_Lookup`'s memcmp
/// does).
///
/// # Safety
/// `m` and `paliashdr` are the model and header `Mod_LoadAliasModel` just
/// filled, with `stverts`/`triangles`/`poseverts` still describing it.
#[no_mangle]
pub unsafe extern "C" fn GL_MakeAliasModelDisplayLists(m: *mut QModel, paliashdr: *mut AliasHdr) {
    // SAFETY: the caller's contract.
    unsafe {
        let hdr = &mut *paliashdr;
        assert_eq!(hdr.poseverttype, PV_QUAKE1);
        c::Con_DPrintf2(c"meshing %s...\n".as_ptr(), (*m).name.as_ptr());

        let numposes = hdr.numposes.max(0) as usize;
        let numverts = hdr.numverts.max(0) as usize;
        let mut verts = vec![
            TriVertX {
                v: [0; 3],
                lightnormalindex: 0
            };
            numposes * numverts
        ];
        for i in 0..numposes {
            let pose = *ptr::addr_of!(poseverts).cast::<*mut TriVertX>().add(i);
            for j in 0..numverts {
                verts[i * numverts + j] = *pose.add(j);
            }
        }

        let maxverts_vbo = (hdr.numtris.max(0) as usize) * 3;
        let mut desc = vec![AliasMesh::default(); maxverts_vbo];
        let mut indexes = vec![0u16; maxverts_vbo];
        let mut vertex_to_index: HashMap<(u16, u32, u32), u16> =
            HashMap::with_capacity(maxverts_vbo);

        let tris = *ptr::addr_of!(triangles);
        let stv = ptr::addr_of!(stverts).cast::<StVert>();
        for i in 0..hdr.numtris.max(0) as usize {
            let tri = &*tris.add(i);
            for j in 0..3 {
                let vertindex = tri.vertindex[j] as u16;
                let st = &*stv.add(vertindex as usize);
                let mut s = st.s;
                let t = st.t;
                if tri.facesfront == 0 && st.onseam != 0 {
                    s += hdr.skinwidth / 2;
                }
                let key = (vertindex, (s as f32).to_bits(), (t as f32).to_bits());
                let index = if let Some(&found) = vertex_to_index.get(&key) {
                    found
                } else {
                    let index = hdr.numverts_vbo as u16;
                    vertex_to_index.insert(key, index);
                    let d = &mut desc[hdr.numverts_vbo as usize];
                    d.vertindex = vertindex;
                    d.st[0] = s as f32;
                    d.st[1] = t as f32;
                    hdr.numverts_vbo += 1;
                    index
                };
                indexes[hdr.numindexes as usize] = index;
                hdr.numindexes += 1;
            }
        }

        hdr.poseverttype = PV_QUAKE1;
        GLMesh_UploadBuffers(
            m,
            paliashdr,
            indexes.as_mut_ptr(),
            verts.as_mut_ptr().cast::<u8>(),
            desc.as_mut_ptr(),
            ptr::null_mut(),
        );
    }
}

/// `GLMesh_DeleteMeshBuffers`: every surface of the chain; deferred through
/// the garbage ring while a frame is being recorded, otherwise after an
/// idle wait.
///
/// # Safety
/// `mainhdr` is NULL or a live alias header chain.
#[no_mangle]
pub unsafe extern "C" fn GLMesh_DeleteMeshBuffers(mainhdr: *mut AliasHdr) {
    // SAFETY: the caller's contract.
    unsafe {
        let mut hdr = mainhdr;
        while !hdr.is_null() {
            let h = &mut *hdr;
            if h.vertex_buffer.is_null() {
                return;
            }
            let vertex_buffer = vk::Buffer::from_raw(h.vertex_buffer as u64);
            let index_buffer = vk::Buffer::from_raw(h.index_buffer as u64);
            let joints_buffer = vk::Buffer::from_raw(h.joints_buffer as u64);
            let joints_set = vk::DescriptorSet::from_raw(h.joints_set as u64);
            let joints_layout =
                ptr::addr_of!((*ptr::addr_of!(vulkan_globals)).joints_buffer_set_layout);
            if *ptr::addr_of!(c::render::in_update_screen) {
                add_buffer_garbage(
                    vertex_buffer,
                    h.vertex_allocation.cast::<Allocation>(),
                    vk::DescriptorSet::null(),
                    ptr::null(),
                );
                add_buffer_garbage(
                    index_buffer,
                    h.index_allocation.cast::<Allocation>(),
                    vk::DescriptorSet::null(),
                    ptr::null(),
                );
                if !h.joints_buffer.is_null() {
                    add_buffer_garbage(
                        joints_buffer,
                        h.joints_allocation.cast::<Allocation>(),
                        joints_set,
                        joints_layout,
                    );
                }
            } else {
                crate::gl_vidsdl::GL_WaitForDeviceIdle();
                device().destroy_buffer(vertex_buffer, None);
                GL_HeapFree(
                    mesh_buffer_heap,
                    h.vertex_allocation.cast::<Allocation>(),
                    mesh_counter(),
                );
                device().destroy_buffer(index_buffer, None);
                GL_HeapFree(
                    mesh_buffer_heap,
                    h.index_allocation.cast::<Allocation>(),
                    mesh_counter(),
                );
                if !h.joints_buffer.is_null() {
                    device().destroy_buffer(joints_buffer, None);
                    GL_HeapFree(
                        mesh_buffer_heap,
                        h.joints_allocation.cast::<Allocation>(),
                        mesh_counter(),
                    );
                    with_ctx(|ctx| free_descriptor_set(ctx, joints_set, &*joints_layout));
                }
            }
            h.vertex_buffer = ptr::null_mut();
            h.vertex_allocation = ptr::null_mut();
            h.index_buffer = ptr::null_mut();
            h.index_allocation = ptr::null_mut();
            h.joints_buffer = ptr::null_mut();
            h.joints_allocation = ptr::null_mut();
            h.joints_set = ptr::null_mut();
            for i in 0..MAX_SKINS {
                if !h.texels[i].is_null() {
                    c::Mem_Free(h.texels[i].cast::<c_void>());
                    h.texels[i] = ptr::null_mut();
                }
            }
            hdr = h.nextsurface;
        }
    }
}

/// Creates a mesh-heap buffer of `size` bytes with `usage`, names it after
/// the model, uploads `data` and returns the buffer, its heap allocation and
/// (when ray query is on) its device address.
///
/// # Safety
/// Main thread with a live device; `name` is NUL-terminated.
unsafe fn create_mesh_buffer(
    name: *const core::ffi::c_char,
    usage: vk::BufferUsageFlags,
    data: &[u8],
) -> (vk::Buffer, *mut Allocation, u64) {
    with_ctx(|ctx| {
        let info = vk::BufferCreateInfo::default()
            .size(data.len() as u64)
            .usage(usage);
        // SAFETY: `info` is complete; the device is live.
        let buffer = match unsafe { device().create_buffer(&info, None) } {
            Ok(b) => b,
            Err(err) => ctx.vk_fail("vkCreateBuffer", err),
        };
        // SAFETY: the caller's contract on `name`.
        let name = unsafe { core::ffi::CStr::from_ptr(name) };
        ctx.name_object(buffer, name);
        // SAFETY: `buffer` is live.
        let reqs = unsafe { device().get_buffer_memory_requirements(buffer) };
        // SAFETY: `mesh_buffer_heap` is the live mesh heap; the counter is
        // the exported atomic.
        let allocation =
            unsafe { GL_HeapAllocate(mesh_buffer_heap, reqs.size, reqs.alignment, mesh_counter()) };
        // SAFETY: `allocation` is a fresh, live heap allocation.
        let (memory, offset) = unsafe { ((*allocation).memory(), (*allocation).offset()) };
        // SAFETY: the buffer is unbound and the memory range is ours.
        if let Err(err) = unsafe { device().bind_buffer_memory(buffer, memory, offset) } {
            ctx.vk_fail("vkBindBufferMemory", err);
        }
        STAGING.upload_buffer(ctx, buffer, data);
        let address = if vg!(ctx, ray_query) {
            ctx.buffer_device_address(buffer)
        } else {
            0
        };
        (buffer, allocation, address)
    })
}

/// `GLMesh_UploadBuffers`: upload data for a single `aliashdr_t` (not its
/// `nextsurface`s). Sets `vbostofs` for MDL/MD3 before the headless /
/// empty-mesh early outs, as C does.
///
/// # Safety
/// `hdr` is NULL or a live header; `indexes` holds `hdr->numindexes`
/// entries, `vertexes` the per-pose vertex data of `hdr->poseverttype`,
/// `desc` `hdr->numverts_vbo` records (MDL/MD3), and `joints` NULL or
/// `numframes * numjoints` poses.
#[no_mangle]
pub unsafe extern "C" fn GLMesh_UploadBuffers(
    mod_: *mut QModel,
    hdr: *mut AliasHdr,
    indexes: *mut u16,
    vertexes: *mut u8,
    desc: *mut AliasMesh,
    joints: *mut JointPose,
) {
    if hdr.is_null() {
        return;
    }
    // SAFETY: the caller's contract.
    unsafe {
        let hdr = &mut *hdr;
        let numindexes;
        let numverts;
        let mut totalvbosize: i32 = 0;
        match hdr.poseverttype {
            PV_QUAKE1 => {
                numverts = hdr.numverts_vbo;
                totalvbosize += numverts * hdr.numposes * core::mem::size_of::<MeshXyz>() as i32;
                numindexes = hdr.numindexes;
            }
            PV_QUAKE3 => {
                numverts = hdr.numverts_vbo;
                totalvbosize += numverts * hdr.numframes * core::mem::size_of::<MeshXyz>() as i32;
                numindexes = hdr.numindexes;
            }
            PV_MD5 => {
                assert_eq!(hdr.numposes, 1);
                totalvbosize += hdr.numverts_vbo * core::mem::size_of::<Md5Vert>() as i32;
                numverts = hdr.numverts_vbo;
                numindexes = hdr.numindexes;
            }
            PV_MD5_8 => {
                assert_eq!(hdr.numposes, 1);
                totalvbosize += hdr.numverts_vbo * core::mem::size_of::<Md5Vert8>() as i32;
                numverts = hdr.numverts_vbo;
                numindexes = hdr.numindexes;
            }
            _ => unreachable!("GLMesh_UploadBuffers: bad poseverttype"),
        }

        let totaljointssize =
            hdr.numframes as usize * hdr.numjoints as usize * core::mem::size_of::<JointPose>();

        if hdr.poseverttype == PV_QUAKE1 || hdr.poseverttype == PV_QUAKE3 {
            hdr.vbostofs = totalvbosize;
            totalvbosize += numverts * core::mem::size_of::<MeshSt>() as i32;
        }

        if *ptr::addr_of!(c::host::no_rendering) {
            return;
        }
        if numindexes == 0 || totalvbosize == 0 {
            return;
        }

        let ray_query = (*ptr::addr_of!(vulkan_globals)).ray_query;
        let name = (*mod_).name.as_ptr();

        {
            let index_bytes =
                core::slice::from_raw_parts(indexes.cast::<u8>(), numindexes as usize * 2);
            let mut usage = vk::BufferUsageFlags::INDEX_BUFFER | vk::BufferUsageFlags::TRANSFER_DST;
            if ray_query {
                usage |= vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
                    | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR;
            }
            let (buffer, allocation, address) = create_mesh_buffer(name, usage, index_bytes);
            hdr.index_buffer = buffer.as_raw() as *mut c_void;
            hdr.index_allocation = allocation.cast::<c_void>();
            if ray_query {
                hdr.index_buffer_address = address;
            }
        }

        let mut vbodata = vec![0u8; totalvbosize as usize];
        let mut vertofs: usize = 0;
        match hdr.poseverttype {
            PV_QUAKE1 => {
                for f in 0..hdr.numposes.max(0) as usize {
                    let xyz = vbodata.as_mut_ptr().cast::<MeshXyz>().add(vertofs);
                    let tv = vertexes.cast::<TriVertX>().add(hdr.numverts as usize * f);
                    vertofs += hdr.numverts_vbo as usize;
                    for v in 0..hdr.numverts_vbo.max(0) as usize {
                        let trivert = *tv.add((*desc.add(v)).vertindex as usize);
                        let out = &mut *xyz.add(v);
                        // MDL is [0-255] => remapped on unsigned 16bit [0; 65535] seen as [0,1]
                        // coords in the vertex shader to be compatible with the MD3 range
                        out.xyz[0] = (trivert.v[0] as i32 * 257) as u16;
                        out.xyz[1] = (trivert.v[1] as i32 * 257) as u16;
                        out.xyz[2] = (trivert.v[2] as i32 * 257) as u16;
                        out.xyz[3] = 1;
                        // normals in [-1..1] mapped to [-127..127] (error < 0.004)
                        let n =
                            &crate::r_alias::r_avertexnormals[trivert.lightnormalindex as usize];
                        out.normal[0] = (127.0f32 * n[0]) as i8;
                        out.normal[1] = (127.0f32 * n[1]) as i8;
                        out.normal[2] = (127.0f32 * n[2]) as i8;
                        out.normal[3] = 0;
                    }
                }
            }
            PV_QUAKE3 => {
                for f in 0..hdr.numframes.max(0) as usize {
                    let xyz = vbodata.as_mut_ptr().cast::<MeshXyz>().add(vertofs);
                    let mut tv = vertexes
                        .cast::<Md3XyzNormal>()
                        .add(hdr.numverts as usize * f);
                    vertofs += hdr.numverts_vbo as usize;
                    for v in 0..hdr.numverts_vbo.max(0) as usize {
                        let src = &*tv;
                        let out = &mut *xyz.add(v);
                        // MD3 is SIGNED 16bit => remapped on unsigned 16bit seen as [0,1] coords
                        out.xyz[0] = (src.xyz[0] as i32 + 32768) as u16;
                        out.xyz[1] = (src.xyz[1] as i32 + 32768) as u16;
                        out.xyz[2] = (src.xyz[2] as i32 + 32768) as u16;
                        out.xyz[3] = 1;
                        let lat = (src.latlong[0] as f32 as f64
                            * (2.0 * core::f64::consts::PI)
                            * (1.0 / 255.0)) as f32;
                        let lng = (src.latlong[1] as f32 as f64
                            * (2.0 * core::f64::consts::PI)
                            * (1.0 / 255.0)) as f32;
                        out.normal[0] = (127.0 * (lng as f64).cos() * (lat as f64).sin()) as i8;
                        out.normal[1] = (127.0 * (lng as f64).sin() * (lat as f64).sin()) as i8;
                        out.normal[2] = (127.0 * (lat as f64).cos()) as i8;
                        out.normal[3] = 0;
                        tv = tv.add(1);
                    }
                }
            }
            PV_MD5 | PV_MD5_8 => {
                // vertexes is already the concat of the hdr surface vertices, triangles, ST,
                // and normals already baked in.
                ptr::copy_nonoverlapping(vertexes, vbodata.as_mut_ptr(), totalvbosize as usize);
            }
            _ => unreachable!("GLMesh_UploadBuffers: bad poseverttype"),
        }

        // fill in the ST coords at the end of the buffer for MDL and MD3:
        if hdr.poseverttype == PV_QUAKE1 {
            assert!(hdr.nextsurface.is_null());
            let st = vbodata
                .as_mut_ptr()
                .add(hdr.vbostofs as usize)
                .cast::<MeshSt>();
            for f in 0..hdr.numverts_vbo.max(0) as usize {
                let d = &*desc.add(f);
                let out = &mut *st.add(f);
                out.st[0] = (d.st[0] + 0.5) / hdr.skinwidth as f32;
                out.st[1] = (d.st[1] + 0.5) / hdr.skinheight as f32;
            }
        } else if hdr.poseverttype == PV_QUAKE3 {
            let st = vbodata
                .as_mut_ptr()
                .add(hdr.vbostofs as usize)
                .cast::<MeshSt>();
            for f in 0..hdr.numverts_vbo.max(0) as usize {
                let d = &*desc.add(f);
                let out = &mut *st.add(f);
                // md3 has floating-point skin coords. use the values directly.
                out.st[0] = d.st[0];
                out.st[1] = d.st[1];
            }
        }

        {
            let mut usage = vk::BufferUsageFlags::VERTEX_BUFFER
                | vk::BufferUsageFlags::TRANSFER_DST
                | vk::BufferUsageFlags::STORAGE_BUFFER;
            if ray_query {
                usage |= vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS;
            }
            let (buffer, allocation, address) = create_mesh_buffer(name, usage, &vbodata);
            hdr.vertex_buffer = buffer.as_raw() as *mut c_void;
            hdr.vertex_allocation = allocation.cast::<c_void>();
            if ray_query {
                hdr.vertex_buffer_address = address;
            }
        }

        if !joints.is_null() {
            let joint_bytes = core::slice::from_raw_parts(joints.cast::<u8>(), totaljointssize);
            let mut usage =
                vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_DST;
            if ray_query {
                usage |= vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS;
            }
            let (buffer, allocation, address) = create_mesh_buffer(name, usage, joint_bytes);
            hdr.joints_buffer = buffer.as_raw() as *mut c_void;
            hdr.joints_allocation = allocation.cast::<c_void>();
            if ray_query {
                hdr.joints_buffer_address = address;
            }

            let set = with_ctx(|ctx| {
                let layout = ptr::addr_of!((*ctx.vg.as_ptr()).joints_buffer_set_layout);
                // `layout` points into the live `vulkan_globals` (outer SAFETY).
                allocate_descriptor_set(ctx, &*layout)
            });
            hdr.joints_set = set.as_raw() as *mut c_void;

            let buffer_info = [vk::DescriptorBufferInfo::default()
                .buffer(buffer)
                .offset(0)
                .range(vk::WHOLE_SIZE)];
            let write = vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(0)
                .dst_array_element(0)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&buffer_info);
            device().update_descriptor_sets(&[write], &[]);
        }
    }
}

/// `GLMesh_DeleteAllMeshBuffers`: every precached alias model, every
/// extradata slot.
#[no_mangle]
pub extern "C" fn GLMesh_DeleteAllMeshBuffers() {
    // SAFETY: main thread; `cl.model_precache` is the live precache list and
    // each entry's extradata slots are NULL or live headers.
    unsafe {
        let clp = ptr::addr_of!(cl);
        for j in 1..(*clp).model_precache.len() {
            let m = (*clp).model_precache[j];
            if m.is_null() {
                break;
            }
            if (*m).type_ != MOD_ALIAS {
                continue;
            }
            for i in 0..PV_SIZE {
                GLMesh_DeleteMeshBuffers((*m).extradata[i].cast::<AliasHdr>());
            }
        }
    }
}

// ---- entity BLASes (Phase 8 M10) -----------------------------------------------

/// `EF_ROCKET` (`gl_model.h`).
const EF_ROCKET: c_int = 1;
/// `MAX_PENDING_BLAS_BUILDS` (`gl_mesh.c`).
const MAX_PENDING_BLAS_BUILDS: usize = 256;

/// `entity_blas_t` (`render.h`), member for member. Only this module reads
/// or writes one -- `entity_t::blas_data` is opaque everywhere else (the
/// `abi_probe.c` note: reached solely through a pointer) -- so it is boxed
/// here instead of `Mem_Alloc`ed.
#[repr(C)]
pub(crate) struct EntityBlas {
    pub(crate) blas: vk::AccelerationStructureKHR,
    buffer: vk::Buffer,
    allocation: *mut Allocation,
    pub(crate) address: vk::DeviceAddress,
    build_scratch_size: vk::DeviceSize,
    update_scratch_size: vk::DeviceSize,
    pub(crate) model: *mut QModel,
    pub(crate) needs_initial_build: bool,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(core::mem::size_of::<EntityBlas>() == 64);

/// The `VK_KHR_acceleration_structure` entry points `gl_vidsdl.c` loaded
/// into `vulkan_globals` (the engine's, not ash's, so both builds resolve the
/// same functions), taken once per call site. Every caller is behind
/// `ray_query`, which is only set when all of them loaded.
pub(crate) struct AsProcs {
    pub(crate) device: vk::Device,
    get_build_sizes: vk::PFN_vkGetAccelerationStructureBuildSizesKHR,
    create: vk::PFN_vkCreateAccelerationStructureKHR,
    destroy: vk::PFN_vkDestroyAccelerationStructureKHR,
    cmd_build: vk::PFN_vkCmdBuildAccelerationStructuresKHR,
    get_device_address: vk::PFN_vkGetAccelerationStructureDeviceAddressKHR,
}

impl AsProcs {
    pub(crate) fn load() -> Self {
        with_ctx(|ctx| {
            let missing = |name: &str| -> ! {
                ctx.engine
                    .sys_error(&format!("{name} is not loaded (no ray query support)"))
            };
            let Some(get_build_sizes) = vg!(ctx, vk_get_acceleration_structure_build_sizes) else {
                missing("vkGetAccelerationStructureBuildSizesKHR")
            };
            let Some(create) = vg!(ctx, vk_create_acceleration_structure) else {
                missing("vkCreateAccelerationStructureKHR")
            };
            let Some(destroy) = vg!(ctx, vk_destroy_acceleration_structure) else {
                missing("vkDestroyAccelerationStructureKHR")
            };
            let Some(cmd_build) = vg!(ctx, vk_cmd_build_acceleration_structures) else {
                missing("vkCmdBuildAccelerationStructuresKHR")
            };
            let Some(get_device_address) = vg!(ctx, vk_get_acceleration_structure_device_address)
            else {
                missing("vkGetAccelerationStructureDeviceAddressKHR")
            };
            AsProcs {
                device: vg!(ctx, device),
                get_build_sizes,
                create,
                destroy,
                cmd_build,
                get_device_address,
            }
        })
    }

    /// `vulkan_globals.vk_get_acceleration_structure_build_sizes (device,
    /// DEVICE, info, counts, &sizes)`.
    pub(crate) fn build_sizes(
        &self,
        info: &vk::AccelerationStructureBuildGeometryInfoKHR<'_>,
        max_primitive_counts: &[u32],
    ) -> vk::AccelerationStructureBuildSizesInfoKHR<'static> {
        debug_assert_eq!(max_primitive_counts.len(), info.geometry_count as usize);
        let mut sizes = vk::AccelerationStructureBuildSizesInfoKHR::default();
        // SAFETY: the loaded entry point over the live device; `info` and
        // `max_primitive_counts` are complete and outlive the call.
        unsafe {
            (self.get_build_sizes)(
                self.device,
                vk::AccelerationStructureBuildTypeKHR::DEVICE,
                info,
                max_primitive_counts.as_ptr(),
                &mut sizes,
            );
        }
        sizes
    }

    /// `vulkan_globals.vk_create_acceleration_structure` over a zeroed
    /// create info with `buffer`, `size` and `type` set.
    pub(crate) fn create(
        &self,
        buffer: vk::Buffer,
        size: vk::DeviceSize,
        ty: vk::AccelerationStructureTypeKHR,
    ) -> Result<vk::AccelerationStructureKHR, vk::Result> {
        let info = vk::AccelerationStructureCreateInfoKHR::default()
            .buffer(buffer)
            .size(size)
            .ty(ty);
        let mut handle = vk::AccelerationStructureKHR::null();
        // SAFETY: as in `build_sizes`; `buffer` is a live buffer bound to
        // memory.
        let err = unsafe { (self.create)(self.device, &info, ptr::null(), &mut handle) };
        if err == vk::Result::SUCCESS {
            Ok(handle)
        } else {
            Err(err)
        }
    }

    /// `vulkan_globals.vk_destroy_acceleration_structure (device, as, NULL)`.
    ///
    /// # Safety
    /// `handle` is live and no longer referenced by any pending work.
    pub(crate) unsafe fn destroy(&self, handle: vk::AccelerationStructureKHR) {
        // SAFETY: the caller's contract.
        unsafe { (self.destroy)(self.device, handle, ptr::null()) }
    }

    /// `vulkan_globals.vk_get_acceleration_structure_device_address`.
    pub(crate) fn device_address(&self, handle: vk::AccelerationStructureKHR) -> vk::DeviceAddress {
        let info =
            vk::AccelerationStructureDeviceAddressInfoKHR::default().acceleration_structure(handle);
        // SAFETY: as in `build_sizes`; `handle` is live.
        unsafe { (self.get_device_address)(self.device, &info) }
    }

    /// `vulkan_globals.vk_cmd_build_acceleration_structures (cb, n, infos,
    /// range_ptrs)`.
    ///
    /// # Safety
    /// `cb` is recording; every `infos[i]` names live structures and
    /// `range_ptrs[i]` points at `infos[i].geometry_count` range infos.
    pub(crate) unsafe fn cmd_build(
        &self,
        cb: vk::CommandBuffer,
        infos: &[vk::AccelerationStructureBuildGeometryInfoKHR<'_>],
        range_ptrs: &[*const vk::AccelerationStructureBuildRangeInfoKHR],
    ) {
        debug_assert_eq!(infos.len(), range_ptrs.len());
        // SAFETY: the caller's contract.
        unsafe { (self.cmd_build)(cb, infos.len() as u32, infos.as_ptr(), range_ptrs.as_ptr()) }
    }
}

/// `vkCmdPipelineBarrier` with one global memory barrier and no buffer or
/// image barriers (the shape every AS build barrier takes).
///
/// # Safety
/// `cb` is a command buffer in the recording state on a live device.
pub(crate) unsafe fn cmd_memory_barrier(
    cb: vk::CommandBuffer,
    src_stage: vk::PipelineStageFlags,
    dst_stage: vk::PipelineStageFlags,
    src_access: vk::AccessFlags,
    dst_access: vk::AccessFlags,
) {
    let barrier = vk::MemoryBarrier::default()
        .src_access_mask(src_access)
        .dst_access_mask(dst_access);
    // SAFETY: `cb` is recording (the caller's contract).
    unsafe {
        device().cmd_pipeline_barrier(
            cb,
            src_stage,
            dst_stage,
            vk::DependencyFlags::empty(),
            &[barrier],
            &[],
            &[],
        );
    }
}

/// `cl.entities[i]` / `cl.static_entities[i - cl.num_entities]` for
/// `i < cl.num_entities + cl.num_statics`.
///
/// # Safety
/// `clp` is the live `cl` and `i` is in range of its two entity lists.
pub(crate) unsafe fn entity_at(clp: *const ClientState, i: c_int) -> *mut Entity {
    // SAFETY: the caller's contract (`i` is in range of the live lists).
    unsafe {
        let num_entities = (*clp).num_entities;
        if i < num_entities {
            (*clp).entities.cast::<Entity>().add(i as usize)
        } else {
            (*(*clp).static_entities.add((i - num_entities) as usize)).cast::<Entity>()
        }
    }
}

/// `R_AllocateEntityBLAS`: the per-entity bottom-level AS of an animated
/// alias model (MDL, MD3, MD5), sized for its VBO triangle list and backed
/// by the mesh heap; [`R_UpdateAnimatedBLASes`] streams the posed vertices
/// through the AS scratch buffer every frame.
///
/// # Safety
/// `e` is a live entity. Reached from the `R_StoreLeafEFrags` task as well
/// as the main thread, exactly as the C -- including the C's unlocked
/// `GL_HeapAllocate` on the shared mesh heap and garbage ring from that
/// worker (COMPAT: a C data race kept as is; Phase 9 owns its removal).
#[no_mangle]
pub unsafe extern "C" fn R_AllocateEntityBLAS(e: *mut Entity) {
    // Reached from `CL_RelinkEntities` headless too, where no `VkDevice`
    // exists and `with_ctx` would `Sys_Error` loading it: the guard reads
    // the field raw, as the C does, and only the build below needs `Ctx`.
    // SAFETY: the caller's contract; `vulkan_globals` and the cvar static
    // live for the program and only the one field is read.
    unsafe {
        let ray_query = (*ptr::addr_of!(vulkan_globals)).ray_query;
        if !ray_query || (*ptr::addr_of!(c::menu::r_rtshadows)).value <= 0.0 {
            return;
        }
        let model = (*e).model;
        if model.is_null() || (*model).type_ != MOD_ALIAS {
            return;
        }
        if (*model).flags & EF_ROCKET != 0 {
            return;
        }
        // COMPAT (ADR-009): `Mod_Extradata` -> `Mod_LoadModel (mod, true)`
        // can only `Host_Error` (or parse on a task worker) when `needload`
        // is set -- a model whose `Mod_EnhancedModels_f` reload failed. The
        // guard landed here at M10 and `gl_mesh.c` mirrors it since the PR
        // #39 review; the two sibling walks below already skip such
        // entities, so the early return only withholds a BLAS from a model
        // nothing could draw (plan amendment log, M10).
        if (*model).needload {
            return;
        }
        let hdr = c::render::Mod_Extradata(model.cast::<c_void>()).cast::<AliasHdr>();
        if hdr.is_null() {
            return;
        }

        // TODO: handle multi-surface models (nextsurface chain)
        let num_triangles = (*hdr).numtris as u32;
        if num_triangles == 0 {
            return;
        }

        // Check if the entity switched models; enhanced model reloads free all entity BLASes explicitly.
        let blas_data = (*e).blas_data.cast::<EntityBlas>();
        if !blas_data.is_null() && (*blas_data).model != model {
            R_FreeEntityBLAS(e);
        }
        if !(*e).blas_data.is_null() {
            return;
        }

        // Allocate BLAS data struct (after validation to avoid alloc/free cycles)
        let mut data = Box::new(EntityBlas {
            blas: vk::AccelerationStructureKHR::null(),
            buffer: vk::Buffer::null(),
            allocation: ptr::null_mut(),
            address: 0,
            build_scratch_size: 0,
            update_scratch_size: 0,
            model: ptr::null_mut(),
            needs_initial_build: true,
        });

        // Vertex positions will be computed into scratch memory as vec3 floats
        let tri_data = vk::AccelerationStructureGeometryTrianglesDataKHR::default()
            .vertex_format(vk::Format::R32G32B32_SFLOAT)
            .vertex_stride(core::mem::size_of::<f32>() as u64 * 3)
            .max_vertex((*hdr).numverts_vbo as u32)
            .index_type(vk::IndexType::UINT16);
        let geometries = [vk::AccelerationStructureGeometryKHR::default()
            .geometry_type(vk::GeometryTypeKHR::TRIANGLES)
            .geometry(vk::AccelerationStructureGeometryDataKHR {
                triangles: tri_data,
            })];
        let build_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
            .ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
            .flags(
                vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_BUILD
                    | vk::BuildAccelerationStructureFlagsKHR::ALLOW_UPDATE,
            )
            .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
            .geometries(&geometries);

        let procs = AsProcs::load();
        let sizes = procs.build_sizes(&build_info, &[num_triangles]);

        let buffer_info = vk::BufferCreateInfo::default()
            .size(sizes.acceleration_structure_size)
            .usage(
                vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR
                    | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
            );
        data.buffer = match device().create_buffer(&buffer_info, None) {
            Ok(b) => b,
            Err(err) => with_ctx(|ctx| {
                ctx.engine.sys_error(&format!(
                    "vkCreateBuffer failed for entity BLAS with code {}",
                    err.as_raw()
                ))
            }),
        };

        let reqs = device().get_buffer_memory_requirements(data.buffer);
        data.allocation =
            GL_HeapAllocate(mesh_buffer_heap, reqs.size, reqs.alignment, mesh_counter());
        let memory = (*data.allocation).memory();
        let offset = (*data.allocation).offset();
        if let Err(err) = device().bind_buffer_memory(data.buffer, memory, offset) {
            with_ctx(|ctx| {
                ctx.engine.sys_error(&format!(
                    "vkBindBufferMemory failed for entity BLAS with code {}",
                    err.as_raw()
                ))
            });
        }

        data.blas = match procs.create(
            data.buffer,
            sizes.acceleration_structure_size,
            vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL,
        ) {
            Ok(blas) => blas,
            Err(err) => with_ctx(|ctx| {
                ctx.engine.sys_error(&format!(
                    "vkCreateAccelerationStructure failed for entity BLAS with code {}",
                    err.as_raw()
                ))
            }),
        };
        data.address = procs.device_address(data.blas);

        // Store scratch sizes for per-frame rebuilds/updates
        data.build_scratch_size = sizes.build_scratch_size;
        data.update_scratch_size = sizes.update_scratch_size;

        // Track which model this BLAS was allocated for
        data.model = model;
        (*e).blas_data = Box::into_raw(data).cast::<c_void>();
    }
}

/// `R_FreeEntityBLAS`: queues the BLAS for collection two frames on and
/// drops the entity's record.
///
/// # Safety
/// `e` is null or a live entity whose `blas_data` this module wrote.
#[no_mangle]
pub unsafe extern "C" fn R_FreeEntityBLAS(e: *mut Entity) {
    // SAFETY: the caller's contract.
    unsafe {
        if e.is_null() || (*e).blas_data.is_null() {
            return;
        }
        let data = Box::from_raw((*e).blas_data.cast::<EntityBlas>());
        // Add to garbage collection - resources will be freed after GPU is done with them
        if data.blas != vk::AccelerationStructureKHR::null() {
            add_blas_garbage(data.blas, data.buffer, data.allocation);
        }
        drop(data);
        (*e).blas_data = ptr::null_mut();
    }
}

/// `R_FreeAllEntityBLASes`: every client and static entity. Called when RT
/// shadows are disabled and on enhanced-model reloads.
#[no_mangle]
pub extern "C" fn R_FreeAllEntityBLASes() {
    // SAFETY: main thread; `cl.entities`/`cl.static_entities` are the live
    // lists (or `entities` is null before the first server connect).
    unsafe {
        let clp = ptr::addr_of!(cl);
        if (*clp).entities.is_null() {
            return;
        }
        for i in 0..(*clp).num_entities {
            R_FreeEntityBLAS((*clp).entities.cast::<Entity>().add(i as usize));
        }
        for i in 0..(*clp).num_statics {
            R_FreeEntityBLAS((*(*clp).static_entities.add(i as usize)).cast::<Entity>());
        }
    }
}

/// One entry of the pending batch `R_UpdateAnimatedBLASes` collects between
/// compute dispatches and the batched build call.
struct PendingBlasBuild {
    blas: vk::AccelerationStructureKHR,
    update: bool,
    vertex_address: vk::DeviceAddress,
    index_address: vk::DeviceAddress,
    max_vertex: u32,
    num_tris: u32,
    scratch_address: vk::DeviceAddress,
}

/// `R_FlushPendingBLASBuilds`: compute->AS barrier, one batched build call,
/// then the barrier for the next phase.
///
/// # Safety
/// `cb` is recording and every pending BLAS is live.
unsafe fn flush_pending_blas_builds(
    procs: &AsProcs,
    cb: vk::CommandBuffer,
    pending: &[PendingBlasBuild],
    more_entities: bool,
) {
    if pending.is_empty() {
        return;
    }
    // SAFETY: the caller's contract.
    unsafe {
        // Barrier: compute writes -> AS reads
        cmd_memory_barrier(
            cb,
            vk::PipelineStageFlags::COMPUTE_SHADER,
            vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
            vk::AccessFlags::SHADER_WRITE,
            vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR,
        );

        let geometries: Vec<vk::AccelerationStructureGeometryKHR<'_>> = pending
            .iter()
            .map(|p| {
                let tri_data = vk::AccelerationStructureGeometryTrianglesDataKHR::default()
                    .vertex_format(vk::Format::R32G32B32_SFLOAT)
                    .vertex_data(vk::DeviceOrHostAddressConstKHR {
                        device_address: p.vertex_address,
                    })
                    .vertex_stride(core::mem::size_of::<f32>() as u64 * 3)
                    .max_vertex(p.max_vertex)
                    .index_type(vk::IndexType::UINT16)
                    .index_data(vk::DeviceOrHostAddressConstKHR {
                        device_address: p.index_address,
                    });
                vk::AccelerationStructureGeometryKHR::default()
                    .geometry_type(vk::GeometryTypeKHR::TRIANGLES)
                    .geometry(vk::AccelerationStructureGeometryDataKHR {
                        triangles: tri_data,
                    })
            })
            .collect();
        let build_infos: Vec<vk::AccelerationStructureBuildGeometryInfoKHR<'_>> = pending
            .iter()
            .zip(&geometries)
            .map(|(p, geometry)| {
                let mut build = vk::AccelerationStructureBuildGeometryInfoKHR::default()
                    .ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
                    .flags(
                        vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_BUILD
                            | vk::BuildAccelerationStructureFlagsKHR::ALLOW_UPDATE,
                    )
                    .dst_acceleration_structure(p.blas)
                    .geometries(core::slice::from_ref(geometry))
                    .scratch_data(vk::DeviceOrHostAddressKHR {
                        device_address: p.scratch_address,
                    });
                if p.update {
                    build = build
                        .mode(vk::BuildAccelerationStructureModeKHR::UPDATE)
                        .src_acceleration_structure(p.blas); // Required for UPDATE mode
                } else {
                    build = build.mode(vk::BuildAccelerationStructureModeKHR::BUILD);
                }
                build
            })
            .collect();
        let ranges: Vec<vk::AccelerationStructureBuildRangeInfoKHR> = pending
            .iter()
            .map(|p| {
                vk::AccelerationStructureBuildRangeInfoKHR::default().primitive_count(p.num_tris)
            })
            .collect();
        let range_ptrs: Vec<*const vk::AccelerationStructureBuildRangeInfoKHR> =
            ranges.iter().map(|r| r as *const _).collect();

        // Single batched AS build call
        procs.cmd_build(cb, &build_infos, &range_ptrs);

        // Barrier for next phase
        if more_entities {
            // More batches coming: need AS_READ for TLAS + SHADER_WRITE for next compute batch
            cmd_memory_barrier(
                cb,
                vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
                vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR
                    | vk::PipelineStageFlags::COMPUTE_SHADER,
                vk::AccessFlags::ACCELERATION_STRUCTURE_WRITE_KHR,
                vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR | vk::AccessFlags::SHADER_WRITE,
            );
        } else {
            // Final batch: only need AS_READ for TLAS build
            cmd_memory_barrier(
                cb,
                vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
                vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
                vk::AccessFlags::ACCELERATION_STRUCTURE_WRITE_KHR,
                vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR,
            );
        }
    }
}

/// `skinning_push_constants_t` as the 48 bytes `R_PushConstants` sends
/// (the C pushes `sizeof (pc)`, tail padding included).
fn skinning_push_constant_bytes(pc: &SkinningPushConstants) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(core::mem::size_of::<SkinningPushConstants>());
    bytes.extend_from_slice(&pc.input_address.to_ne_bytes());
    bytes.extend_from_slice(&pc.joints_address.to_ne_bytes());
    bytes.extend_from_slice(&pc.output_address.to_ne_bytes());
    bytes.extend_from_slice(&pc.joints_offset0.to_ne_bytes());
    bytes.extend_from_slice(&pc.joints_offset1.to_ne_bytes());
    bytes.extend_from_slice(&pc.output_offset.to_ne_bytes());
    bytes.extend_from_slice(&pc.num_verts.to_ne_bytes());
    bytes.extend_from_slice(&pc.blend_factor.to_ne_bytes());
    bytes.resize(core::mem::size_of::<SkinningPushConstants>(), 0);
    bytes
}

/// `mesh_interpolate_push_constants_t` as its 40 bytes.
fn mesh_interpolate_push_constant_bytes(pc: &MeshInterpolatePushConstants) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(core::mem::size_of::<MeshInterpolatePushConstants>());
    bytes.extend_from_slice(&pc.input_address.to_ne_bytes());
    bytes.extend_from_slice(&pc.output_address.to_ne_bytes());
    bytes.extend_from_slice(&pc.pose1_offset.to_ne_bytes());
    bytes.extend_from_slice(&pc.pose2_offset.to_ne_bytes());
    bytes.extend_from_slice(&pc.output_offset.to_ne_bytes());
    bytes.extend_from_slice(&pc.num_verts.to_ne_bytes());
    bytes.extend_from_slice(&pc.blend_factor.to_ne_bytes());
    bytes.extend_from_slice(&pc.flags.to_ne_bytes());
    debug_assert_eq!(
        bytes.len(),
        core::mem::size_of::<MeshInterpolatePushConstants>()
    );
    bytes
}

/// `R_UpdateAnimatedBLASes`: dispatches the interpolate/skinning compute
/// shaders for every visible animated entity into the AS scratch buffer,
/// then builds (first frame) or refits its BLAS, in batches of
/// [`MAX_PENDING_BLAS_BUILDS`] or whatever the scratch buffer holds.
///
/// # Safety
/// Main thread (or the AS-build task, which runs alone) with `cbx`
/// recording; every entity's `blas_data` was written by this module.
#[no_mangle]
pub unsafe extern "C" fn R_UpdateAnimatedBLASes(cbx: *mut CbContext) {
    // SAFETY: `vulkan_globals` is live for the process; a plain field read
    // ahead of `with_ctx`, so the C's early return never loads the device.
    if !unsafe { (*ptr::addr_of!(vulkan_globals)).ray_query } {
        return;
    }
    let (scratch_alignment, buffer_alignment) = with_ctx(|ctx| {
        (
            vg!(
                ctx,
                physical_device_acceleration_structure_properties
                    .min_acceleration_structure_scratch_offset_alignment
            ) as vk::DeviceSize,
            // 16 bytes because of device address default buffer_reference_align
            vg!(
                ctx,
                device_properties.limits.min_storage_buffer_offset_alignment
            )
            .max(16),
        )
    });
    let (mesh_interpolate_pipeline, skinning_pipeline, skinning_8_pipeline) = with_ctx(|ctx| {
        (
            vg!(ctx, mesh_interpolate_pipeline),
            vg!(ctx, skinning_pipeline),
            vg!(ctx, skinning_8_pipeline),
        )
    });
    // SAFETY: the caller's contract.
    unsafe {
        if (*ptr::addr_of!(as_scratch_buffer)).buffer == vk::Buffer::null() {
            return;
        }

        let clp = ptr::addr_of!(cl);
        let total_entities = (*clp).num_entities + (*clp).num_statics;

        // Pre-pass: find max scratch size needed across all entities and resize if necessary
        {
            let mut max_scratch_needed: vk::DeviceSize = 0;
            for i in 0..total_entities {
                let e = entity_at(clp, i);
                let model = (*e).model;
                let blas_data = (*e).blas_data.cast::<EntityBlas>();
                if model.is_null()
                    || (*model).needload
                    || (*model).type_ != MOD_ALIAS
                    || blas_data.is_null()
                    || (*blas_data).blas == vk::AccelerationStructureKHR::null()
                {
                    continue;
                }
                if (*e).alpha != ENTALPHA_DEFAULT && entalpha_decode((*e).alpha) < 1.0 {
                    continue;
                }
                let hdr = c::render::Mod_Extradata(model.cast::<c_void>()).cast::<AliasHdr>();
                if hdr.is_null() || (*hdr).numverts_vbo == 0 {
                    continue;
                }
                if (*blas_data).model != model {
                    continue;
                }

                let vertex_size = (*hdr).numverts_vbo as vk::DeviceSize * 12;
                let as_scratch_size = if (*blas_data).needs_initial_build {
                    (*blas_data).build_scratch_size
                } else {
                    (*blas_data).update_scratch_size
                };
                let total_needed = q_align(vertex_size, scratch_alignment) + as_scratch_size;
                max_scratch_needed = max_scratch_needed.max(total_needed);
            }

            R_EnsureASScratchBufferSize(max_scratch_needed as u32);
        }

        let scratch_buffer_size = *ptr::addr_of!(as_scratch_buffer_size) as vk::DeviceSize;
        let scratch_device_address = (*ptr::addr_of!(as_scratch_buffer)).device_address;
        let mut scratch_offset: vk::DeviceSize = 0;
        let mut pending: Vec<PendingBlasBuild> = Vec::with_capacity(MAX_PENDING_BLAS_BUILDS);
        let mut entity_index: c_int = 0;

        let procs = AsProcs::load();
        let cmd_procs = with_ctx(|ctx| CmdProcs::new(ctx.vg));
        let cb = (*cbx).cb;

        cb::begin_debug_utils_label(&cmd_procs, &*cbx, c"Update Animated BLAS");

        while entity_index < total_entities {
            // Phase 1: Compute - dispatch shaders and collect build info
            while entity_index < total_entities && pending.len() < MAX_PENDING_BLAS_BUILDS {
                let e = entity_at(clp, entity_index);
                entity_index += 1;

                let model = (*e).model;
                let blas_data = (*e).blas_data.cast::<EntityBlas>();
                if model.is_null()
                    || (*model).needload
                    || (*model).type_ != MOD_ALIAS
                    || blas_data.is_null()
                    || (*blas_data).blas == vk::AccelerationStructureKHR::null()
                {
                    continue;
                }

                // Skip transparent entities (same as TLAS)
                if (*e).alpha != ENTALPHA_DEFAULT && entalpha_decode((*e).alpha) < 1.0 {
                    continue;
                }

                let hdr = c::render::Mod_Extradata(model.cast::<c_void>()).cast::<AliasHdr>();
                if hdr.is_null() || (*hdr).numverts_vbo == 0 {
                    continue;
                }

                // Skip if BLAS was allocated for a different model/geometry (model changed but entity not visible yet)
                if (*blas_data).model != model {
                    continue;
                }

                // Get lerp data for vertex interpolation
                let mut lerpdata = LerpData::default();
                R_SetupAliasFrame(e, hdr, &mut lerpdata);
                let pose1 = lerpdata.pose1 as i32;
                let pose2 = lerpdata.pose2 as i32;
                let blend = lerpdata.blend;

                // Always use refit after first build. We trace few rays and full updates are expensive.
                let use_update = !(*blas_data).needs_initial_build;

                let numverts_vbo = (*hdr).numverts_vbo;
                let vertex_size = numverts_vbo as vk::DeviceSize * 12;
                let as_scratch_size = if use_update {
                    (*blas_data).update_scratch_size
                } else {
                    (*blas_data).build_scratch_size
                };

                // Check if we have space; if not, flush current batch and reset
                let vertex_offset = q_align(scratch_offset, buffer_alignment);
                let as_scratch_offset = q_align(vertex_offset + vertex_size, scratch_alignment);
                let total_needed = as_scratch_offset - scratch_offset + as_scratch_size;
                if scratch_offset + total_needed > scratch_buffer_size {
                    // Need to flush - back up entity_index to retry this entity after flush
                    entity_index -= 1;
                    break;
                }

                (*blas_data).needs_initial_build = false;

                let vertex_output_address = scratch_device_address + vertex_offset;
                let scratch_address = scratch_device_address + as_scratch_offset;

                // Dispatch compute shader with push constants containing buffer addresses
                if (*hdr).poseverttype == PV_MD5 || (*hdr).poseverttype == PV_MD5_8 {
                    // MD5 skinning
                    let pc = SkinningPushConstants {
                        input_address: (*hdr).vertex_buffer_address,
                        joints_address: (*hdr).joints_buffer_address,
                        output_address: vertex_output_address,
                        joints_offset0: (pose1 * (*hdr).numjoints) as u32,
                        joints_offset1: (pose2 * (*hdr).numjoints) as u32,
                        output_offset: 0, // output starts at output_address
                        num_verts: numverts_vbo as u32,
                        blend_factor: blend,
                    };
                    cb::bind_pipeline(
                        &cmd_procs,
                        &mut *cbx,
                        vk::PipelineBindPoint::COMPUTE,
                        if (*hdr).poseverttype == PV_MD5_8 {
                            skinning_8_pipeline
                        } else {
                            skinning_pipeline
                        },
                    );
                    cb::push_constants(
                        &cmd_procs,
                        &*cbx,
                        vk::ShaderStageFlags::COMPUTE,
                        0,
                        &skinning_push_constant_bytes(&pc),
                    );
                } else {
                    // MDL/MD3 interpolation
                    let pc = MeshInterpolatePushConstants {
                        input_address: (*hdr).vertex_buffer_address,
                        output_address: vertex_output_address,
                        pose1_offset: (pose1 * numverts_vbo) as u32,
                        pose2_offset: (pose2 * numverts_vbo) as u32,
                        output_offset: 0, // output starts at output_address
                        num_verts: numverts_vbo as u32,
                        blend_factor: blend,
                        flags: if (*hdr).poseverttype == PV_QUAKE3 {
                            0x4
                        } else {
                            0
                        },
                    };
                    cb::bind_pipeline(
                        &cmd_procs,
                        &mut *cbx,
                        vk::PipelineBindPoint::COMPUTE,
                        mesh_interpolate_pipeline,
                    );
                    cb::push_constants(
                        &cmd_procs,
                        &*cbx,
                        vk::ShaderStageFlags::COMPUTE,
                        0,
                        &mesh_interpolate_push_constant_bytes(&pc),
                    );
                }

                let num_groups = (numverts_vbo as u32).div_ceil(64);
                device().cmd_dispatch(cb, num_groups, 1, 1);

                // Store build info for later
                pending.push(PendingBlasBuild {
                    blas: (*blas_data).blas,
                    update: use_update,
                    vertex_address: vertex_output_address,
                    index_address: (*hdr).index_buffer_address,
                    max_vertex: numverts_vbo as u32,
                    num_tris: (*hdr).numtris as u32,
                    scratch_address,
                });

                scratch_offset += total_needed;
            }

            // Phase 2: Build - flush pending builds
            if !pending.is_empty() {
                let more_entities = entity_index < total_entities;
                flush_pending_blas_builds(&procs, cb, &pending, more_entities);
                pending.clear();
                scratch_offset = 0;
            }
        }

        cb::end_debug_utils_label(&cmd_procs, &*cbx);
    }
}
