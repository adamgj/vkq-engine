//! `gl_mesh.c` (Phase 8 M8): the alias-model mesh heap, the GPU buffer
//! upload for MDL/MD3/MD5 headers, the deferred buffer/BLAS garbage lists
//! and `GL_MakeAliasModelDisplayLists`. The ray-tracing half of the file
//! (`R_AllocateEntityBLAS` and friends) stays C in `Quake/gl_mesh_glue.c`
//! until Phase 8 M10 (D5); it reaches the heap through the exported
//! `mesh_buffer_heap` and the garbage list through
//! [`GLMesh_Glue_AddBLASGarbage`].
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
use quake_render::heap::Allocation;
use quake_render::rmisc::{allocate_descriptor_set, free_descriptor_set};
use quake_types::host::ClientState;
use quake_types::model_mem::{
    AliasHdr, AliasMesh, JointPose, MTriangle, Md3XyzNormal, Md5Vert, Md5Vert8, QModel,
    MAXALIASFRAMES, MAXALIASVERTS, MAX_SKINS, MOD_ALIAS, PV_MD5, PV_MD5_8, PV_QUAKE1, PV_QUAKE3,
    PV_SIZE,
};
use quake_types::modelgen::{StVert, TriVertX};
use quake_types::render::{GlHeapStats, MeshSt, MeshXyz, VulkanDescSetLayout};

use crate::gl_heap::{GL_HeapAllocate, GL_HeapCreate, GL_HeapFree, GL_HeapGetStats, GlHeap};
use crate::gl_rmisc::{device, num_vulkan_mesh_allocations, vg, vulkan_globals, with_ctx, STAGING};

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

/// `static glheap_t *mesh_buffer_heap`, exported for `gl_mesh_glue.c`'s
/// BLAS allocations (M10 takes it back).
#[no_mangle]
pub static mut mesh_buffer_heap: *mut GlHeap = ptr::null_mut();

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

/// `AddBLASGarbage`, exported for `gl_mesh_glue.c`'s `R_FreeEntityBLAS`
/// (`VkAccelerationStructureKHR` and `VkBuffer` are 64-bit handles on every
/// target; `allocation` is the `glheapallocation_t *` from
/// `GL_HeapAllocate`).
///
/// # Safety
/// Main thread only.
#[no_mangle]
pub unsafe extern "C" fn GLMesh_Glue_AddBLASGarbage(
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
        assert!(hdr.poseverttype == PV_QUAKE1);
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
                let index = match vertex_to_index.get(&key) {
                    Some(&found) => found,
                    None => {
                        let index = hdr.numverts_vbo as u16;
                        vertex_to_index.insert(key, index);
                        let d = &mut desc[hdr.numverts_vbo as usize];
                        d.vertindex = vertindex;
                        d.st[0] = s as f32;
                        d.st[1] = t as f32;
                        hdr.numverts_vbo += 1;
                        index
                    }
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
                assert!(hdr.numposes == 1);
                totalvbosize += hdr.numverts_vbo * core::mem::size_of::<Md5Vert>() as i32;
                numverts = hdr.numverts_vbo;
                numindexes = hdr.numindexes;
            }
            PV_MD5_8 => {
                assert!(hdr.numposes == 1);
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
