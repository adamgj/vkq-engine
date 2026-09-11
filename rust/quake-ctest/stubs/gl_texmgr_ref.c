/* Phase 8 M4 oracle TU for Quake/gl_texmgr.c -- the texture manager
 * (ADR-015; tests/gl_texmgr_differential.rs).
 *
 * gl_texmgr.c is composed here rather than listed in build.rs's C_SOURCES
 * because its seams -- the Vulkan entry points, the gl_rmisc.c staging and
 * descriptor helpers, GL_SetObjectName, COM_FOpenFile/fread/Sys_fseek,
 * Image_LoadImage, vulkan_globals -- are exactly what the differential
 * replaces with deterministic fakes. Every fake appends one line to a text
 * trace and hands out sequential handles; the Rust FakeBackend in the test
 * does the same, so the two traces compare line-for-line. The GL_Heap* calls
 * go to the M3 oracle (stubs/gl_heap_ref.c, c_ref_GL_Heap*), whose fake
 * memory handles are sequential too.
 *
 * The prelude (include/c_ref_prelude.h) is force-included before this file
 * and #defines QUAKEDEFS_H, so gl_texmgr.c's `#include "quakedef.h"` is a
 * no-op and every type it needs comes from the prelude or from here. The
 * VK_* values below are those of vulkan_core.h (the mixed engine build
 * checks the same numbers against the SDK header through ash).
 *
 * _DEBUG is undefined for this TU (nothing in gl_texmgr.c depends on it, but
 * the prelude's stub helpers are the release ones). */
#undef _DEBUG
#include <stdio.h>
#include <stdarg.h>
#include "quakedef.h"

/* ---- Vulkan spellings gl_texmgr.c uses that the prelude lacks ---------- */
#if defined(__LP64__) || defined(_WIN64) || (defined(__x86_64__) && !defined(__ILP32__)) || defined(_M_X64) || defined(__ia64) || defined(_M_IA64) || \
	defined(__aarch64__) || defined(__powerpc64__) || (defined(__riscv) && __riscv_xlen == 64)
typedef struct VkSampler_T *VkSampler;
#else
typedef uint64_t VkSampler;
#endif
typedef int		VkFormat;
typedef int		VkImageLayout;
typedef int		VkImageType;
typedef int		VkImageTiling;
typedef int		VkImageViewType;
typedef int		VkSharingMode;
typedef int		VkDescriptorType;
typedef int		VkComponentSwizzle;
typedef int		VkSampleCountFlagBits;
typedef VkFlags VkImageCreateFlags;
typedef VkFlags VkImageUsageFlags;
typedef VkFlags VkImageViewCreateFlags;
typedef VkFlags VkFramebufferCreateFlags;
typedef VkFlags VkImageAspectFlags;
typedef VkFlags VkAccessFlags;
typedef VkFlags VkPipelineStageFlags;
typedef VkFlags VkDependencyFlags;

typedef struct
{
	uint32_t width, height, depth;
} VkExtent3D;
typedef struct
{
	int32_t x, y, z;
} VkOffset3D;
typedef struct
{
	VkStructureType		  sType;
	const void			 *pNext;
	VkImageCreateFlags	  flags;
	VkImageType			  imageType;
	VkFormat			  format;
	VkExtent3D			  extent;
	uint32_t			  mipLevels;
	uint32_t			  arrayLayers;
	VkSampleCountFlagBits samples;
	VkImageTiling		  tiling;
	VkImageUsageFlags	  usage;
	VkSharingMode		  sharingMode;
	uint32_t			  queueFamilyIndexCount;
	const uint32_t		 *pQueueFamilyIndices;
	VkImageLayout		  initialLayout;
} VkImageCreateInfo;
typedef struct
{
	VkComponentSwizzle r, g, b, a;
} VkComponentMapping;
typedef struct
{
	VkImageAspectFlags aspectMask;
	uint32_t		   baseMipLevel;
	uint32_t		   levelCount;
	uint32_t		   baseArrayLayer;
	uint32_t		   layerCount;
} VkImageSubresourceRange;
typedef struct
{
	VkImageAspectFlags aspectMask;
	uint32_t		   mipLevel;
	uint32_t		   baseArrayLayer;
	uint32_t		   layerCount;
} VkImageSubresourceLayers;
typedef struct
{
	VkStructureType			sType;
	const void			   *pNext;
	VkImageViewCreateFlags	flags;
	VkImage					image;
	VkImageViewType			viewType;
	VkFormat				format;
	VkComponentMapping		components;
	VkImageSubresourceRange subresourceRange;
} VkImageViewCreateInfo;
typedef struct
{
	VkStructureType			 sType;
	const void				*pNext;
	VkFramebufferCreateFlags flags;
	VkRenderPass			 renderPass;
	uint32_t				 attachmentCount;
	const VkImageView		*pAttachments;
	uint32_t				 width;
	uint32_t				 height;
	uint32_t				 layers;
} VkFramebufferCreateInfo;
typedef struct
{
	VkStructureType			sType;
	const void			   *pNext;
	VkAccessFlags			srcAccessMask;
	VkAccessFlags			dstAccessMask;
	VkImageLayout			oldLayout;
	VkImageLayout			newLayout;
	uint32_t				srcQueueFamilyIndex;
	uint32_t				dstQueueFamilyIndex;
	VkImage					image;
	VkImageSubresourceRange subresourceRange;
} VkImageMemoryBarrier;
typedef struct
{
	VkDeviceSize			 bufferOffset;
	uint32_t				 bufferRowLength;
	uint32_t				 bufferImageHeight;
	VkImageSubresourceLayers imageSubresource;
	VkOffset3D				 imageOffset;
	VkExtent3D				 imageExtent;
} VkBufferImageCopy;
typedef struct
{
	VkSampler	  sampler;
	VkImageView	  imageView;
	VkImageLayout imageLayout;
} VkDescriptorImageInfo;
typedef struct
{
	VkStructureType				 sType;
	const void					*pNext;
	VkDescriptorSet				 dstSet;
	uint32_t					 dstBinding;
	uint32_t					 dstArrayElement;
	uint32_t					 descriptorCount;
	VkDescriptorType			 descriptorType;
	const VkDescriptorImageInfo *pImageInfo;
	const void					*pBufferInfo;
	const void					*pTexelBufferView;
} VkWriteDescriptorSet;

#define VK_ACCESS_SHADER_READ_BIT				  0x00000020
#define VK_ACCESS_TRANSFER_WRITE_BIT			  0x00001000
#define VK_COMPONENT_SWIZZLE_R					  3
#define VK_COMPONENT_SWIZZLE_G					  4
#define VK_COMPONENT_SWIZZLE_B					  5
#define VK_COMPONENT_SWIZZLE_A					  6
#define VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER 1
#define VK_DESCRIPTOR_TYPE_STORAGE_IMAGE		  3
#define VK_FORMAT_A2B10G10R10_UNORM_PACK32		  64
#define VK_FORMAT_R32_UINT						  98
#define VK_FORMAT_R8G8B8A8_UNORM				  37
#define VK_IMAGE_ASPECT_COLOR_BIT				  0x00000001
#define VK_IMAGE_CREATE_CUBE_COMPATIBLE_BIT		  0x00000010
#define VK_IMAGE_LAYOUT_UNDEFINED				  0
#define VK_IMAGE_LAYOUT_GENERAL					  1
#define VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL  5
#define VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL	  7
#define VK_IMAGE_TILING_OPTIMAL					  0
#define VK_IMAGE_TYPE_2D						  1
#define VK_IMAGE_USAGE_TRANSFER_SRC_BIT			  0x00000001
#define VK_IMAGE_USAGE_TRANSFER_DST_BIT			  0x00000002
#define VK_IMAGE_USAGE_SAMPLED_BIT				  0x00000004
#define VK_IMAGE_USAGE_STORAGE_BIT				  0x00000008
#define VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT		  0x00000010
#define VK_IMAGE_VIEW_TYPE_2D					  1
#define VK_IMAGE_VIEW_TYPE_CUBE					  3
#define VK_OBJECT_TYPE_IMAGE					  10
#define VK_OBJECT_TYPE_IMAGE_VIEW				  14
#define VK_OBJECT_TYPE_DESCRIPTOR_SET			  23
#define VK_OBJECT_TYPE_FRAMEBUFFER				  24
#define VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT		  0x00000001
#define VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT	  0x00000080
#define VK_PIPELINE_STAGE_TRANSFER_BIT			  0x00001000
#define VK_QUEUE_FAMILY_IGNORED					  (~0U)
#define VK_SAMPLE_COUNT_1_BIT					  1
#define VK_SHARING_MODE_EXCLUSIVE				  0
#define VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO		  14
#define VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO  15
#define VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET	  35
#define VK_STRUCTURE_TYPE_FRAMEBUFFER_CREATE_INFO 37
#define VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER	  45

/* ---- glquake.h / gl_texmgr.h / render.h spellings ----------------------- */
#ifndef MAX_GLTEXTURES
#define MAX_GLTEXTURES (16 * 4096) /* glquake.h:58 */
#endif
#ifndef LIGHTMAP_BYTES
#define LIGHTMAP_BYTES 4 /* gl_texmgr.h */
#endif
#ifndef WARPIMAGEMIPS
#define WARPIMAGEMIPS 5 /* gl_texmgr.h */
#endif
#ifndef TOP_RANGE
#define TOP_RANGE	 16 /* render.h:34 */
#define BOTTOM_RANGE 96
#endif

/* The vulkan_globals members gl_texmgr.c reads (glquake.h). The prelude's
 * compile-only vulkanglobals_t has none of them, so this TU carries its own
 * and renames the global. vulkan_desc_set_layout_t is the prelude's real
 * copy since M5; num_storage_images tells the two layouts apart in the
 * trace, as the real single_texture_cs_write layout has one. */
typedef struct
{
	VkDevice device;
	struct
	{
		struct
		{
			uint32_t maxImageDimension2D;
			uint32_t maxImageDimensionCube;
		} limits;
	} device_properties;
	VkFormat				 color_format;
	VkSampler				 point_sampler_lod_bias;
	VkSampler				 linear_sampler_lod_bias;
	VkSampler				 point_aniso_sampler_lod_bias;
	VkSampler				 linear_aniso_sampler_lod_bias;
	VkRenderPass			 warp_render_pass;
	vulkan_desc_set_layout_t single_texture_set_layout;
	vulkan_desc_set_layout_t single_texture_cs_write_set_layout;
} c_ref_texmgr_globals_t;
c_ref_texmgr_globals_t c_ref_texmgr_globals = {(VkDevice)0x10, {{4096, 4096}}, VK_FORMAT_R8G8B8A8_UNORM, 0, 0, 0, 0, 0, {0}, {.num_storage_images = 1}};
#define vulkan_globals c_ref_texmgr_globals

/* ---- the process globals gl_texmgr.c touches ---------------------------- */
qboolean		 c_ref_texmgr_no_rendering;
qboolean		 c_ref_texmgr_in_update_screen;
atomic_uint32_t	 c_ref_texmgr_num_vulkan_tex_allocations;
cvar_t			 c_ref_texmgr_gl_fullbrights = {"gl_fullbrights", "1", CVAR_NONE};
cvar_t			 c_ref_texmgr_vid_filter = {"vid_filter", "1", CVAR_NONE};
cvar_t			 c_ref_texmgr_vid_anisotropic = {"vid_anisotropic", "0", CVAR_NONE};
static texture_t c_ref_texmgr_notex[2];
texture_t		*c_ref_texmgr_r_notexture_mip = &c_ref_texmgr_notex[0];
texture_t		*c_ref_texmgr_r_notexture_mip2 = &c_ref_texmgr_notex[1];
#define no_rendering			   c_ref_texmgr_no_rendering
#define in_update_screen		   c_ref_texmgr_in_update_screen
#define num_vulkan_tex_allocations c_ref_texmgr_num_vulkan_tex_allocations
#define gl_fullbrights			   c_ref_texmgr_gl_fullbrights
#define vid_filter				   c_ref_texmgr_vid_filter
#define vid_anisotropic			   c_ref_texmgr_vid_anisotropic
#define r_notexture_mip			   c_ref_texmgr_r_notexture_mip
#define r_notexture_mip2		   c_ref_texmgr_r_notexture_mip2

/* ---- trace ------------------------------------------------------------- */
static char	 *trace;
static size_t trace_len, trace_cap;

static void trace_vappend (const char *fmt, va_list ap)
{
	va_list ap2;
	va_copy (ap2, ap);
	int n = vsnprintf (NULL, 0, fmt, ap2);
	va_end (ap2);
	if (n < 0)
		return;
	if (trace_len + (size_t)n + 2 > trace_cap)
	{
		trace_cap = (trace_cap ? trace_cap * 2 : 65536);
		while (trace_len + (size_t)n + 2 > trace_cap)
			trace_cap *= 2;
		trace = (char *)realloc (trace, trace_cap);
	}
	vsnprintf (trace + trace_len, (size_t)n + 1, fmt, ap);
	trace_len += (size_t)n;
	trace[trace_len++] = '\n';
	trace[trace_len] = 0;
}

static void trace_append (const char *fmt, ...)
{
	va_list ap;
	va_start (ap, fmt);
	trace_vappend (fmt, ap);
	va_end (ap);
}

const char *c_ref_texmgr_trace (void)
{
	return trace ? trace : "";
}

void c_ref_texmgr_trace_reset (void)
{
	trace_len = 0;
	if (trace)
		trace[0] = 0;
}

/* ---- handles + fake images ---------------------------------------------- */
static uint64_t next_handle;

uint64_t c_ref_texmgr_next_handle (void)
{
	return next_handle;
}

#define H64(x) ((unsigned long long)(uintptr_t)(x))

typedef struct
{
	uint64_t handle;
	uint64_t size;
} fake_image_t;
#define MAX_FAKE_IMAGES 4096
static fake_image_t fake_images[MAX_FAKE_IMAGES];
static int			num_fake_images;

static uint64_t fake_image_size (uint64_t handle)
{
	for (int i = 0; i < num_fake_images; i++)
		if (fake_images[i].handle == handle)
			return fake_images[i].size;
	return 0;
}

/* ---- the Vulkan fakes --------------------------------------------------- */
static VkResult c_ref_texmgr_vkCreateImage (VkDevice device, const VkImageCreateInfo *info, const void *allocator, VkImage *image)
{
	(void)device;
	(void)allocator;
	uint64_t handle = ++next_handle;
	uint64_t bytes = (uint64_t)info->extent.width * info->extent.height * info->extent.depth * 4 * info->arrayLayers;
	if (info->mipLevels > 1)
		bytes += bytes / 2;
	bytes = (bytes + 255) & ~(uint64_t)255;
	if (num_fake_images < MAX_FAKE_IMAGES)
	{
		fake_images[num_fake_images].handle = handle;
		fake_images[num_fake_images].size = bytes;
		num_fake_images++;
	}
	trace_append (
		"CreateImage flags=%u type=%d fmt=%d %ux%ux%u mips=%u layers=%u samples=%d tiling=%d usage=%u sharing=%d qfi=%u layout=%d -> %llu",
		(unsigned)info->flags, info->imageType, info->format, info->extent.width, info->extent.height, info->extent.depth, info->mipLevels, info->arrayLayers,
		info->samples, info->tiling, (unsigned)info->usage, info->sharingMode, info->queueFamilyIndexCount, info->initialLayout, (unsigned long long)handle);
	*image = (VkImage)(uintptr_t)handle;
	return VK_SUCCESS;
}

static void c_ref_texmgr_vkDestroyImage (VkDevice device, VkImage image, const void *allocator)
{
	(void)device;
	(void)allocator;
	trace_append ("DestroyImage %llu", H64 (image));
	for (int i = 0; i < num_fake_images; i++)
		if (fake_images[i].handle == (uint64_t)(uintptr_t)image)
		{
			fake_images[i] = fake_images[--num_fake_images];
			break;
		}
}

static void c_ref_texmgr_vkGetImageMemoryRequirements (VkDevice device, VkImage image, VkMemoryRequirements *reqs)
{
	(void)device;
	reqs->size = fake_image_size ((uint64_t)(uintptr_t)image);
	reqs->alignment = 256;
	reqs->memoryTypeBits = 0xff;
	trace_append (
		"ImageMemoryRequirements %llu -> size=%llu align=%llu bits=%u", H64 (image), (unsigned long long)reqs->size, (unsigned long long)reqs->alignment,
		reqs->memoryTypeBits);
}

static VkResult c_ref_texmgr_vkBindImageMemory (VkDevice device, VkImage image, VkDeviceMemory memory, VkDeviceSize offset)
{
	(void)device;
	trace_append ("BindImageMemory %llu mem=%llu off=%llu", H64 (image), H64 (memory), (unsigned long long)offset);
	return VK_SUCCESS;
}

static VkResult c_ref_texmgr_vkCreateImageView (VkDevice device, const VkImageViewCreateInfo *info, const void *allocator, VkImageView *view)
{
	(void)device;
	(void)allocator;
	uint64_t handle = ++next_handle;
	trace_append (
		"CreateImageView flags=%u image=%llu type=%d fmt=%d swz=%d,%d,%d,%d aspect=%u mip=%u/%u layer=%u/%u -> %llu", (unsigned)info->flags, H64 (info->image),
		info->viewType, info->format, info->components.r, info->components.g, info->components.b, info->components.a,
		(unsigned)info->subresourceRange.aspectMask, info->subresourceRange.baseMipLevel, info->subresourceRange.levelCount,
		info->subresourceRange.baseArrayLayer, info->subresourceRange.layerCount, (unsigned long long)handle);
	*view = (VkImageView)(uintptr_t)handle;
	return VK_SUCCESS;
}

static void c_ref_texmgr_vkDestroyImageView (VkDevice device, VkImageView view, const void *allocator)
{
	(void)device;
	(void)allocator;
	trace_append ("DestroyImageView %llu", H64 (view));
}

static VkResult c_ref_texmgr_vkCreateFramebuffer (VkDevice device, const VkFramebufferCreateInfo *info, const void *allocator, VkFramebuffer *fb)
{
	(void)device;
	(void)allocator;
	uint64_t handle = ++next_handle;
	trace_append (
		"CreateFramebuffer flags=%u rp=%llu att=%u[%llu] %ux%u layers=%u -> %llu", (unsigned)info->flags, H64 (info->renderPass), info->attachmentCount,
		info->attachmentCount ? H64 (info->pAttachments[0]) : 0ULL, info->width, info->height, info->layers, (unsigned long long)handle);
	*fb = (VkFramebuffer)(uintptr_t)handle;
	return VK_SUCCESS;
}

static void c_ref_texmgr_vkDestroyFramebuffer (VkDevice device, VkFramebuffer fb, const void *allocator)
{
	(void)device;
	(void)allocator;
	trace_append ("DestroyFramebuffer %llu", H64 (fb));
}

static void
c_ref_texmgr_vkUpdateDescriptorSets (VkDevice device, uint32_t write_count, const VkWriteDescriptorSet *writes, uint32_t copy_count, const void *copies)
{
	(void)device;
	(void)copies;
	for (uint32_t i = 0; i < write_count; i++)
	{
		const VkWriteDescriptorSet *w = &writes[i];
		trace_append (
			"UpdateDescriptorSet set=%llu binding=%u elem=%u count=%u type=%d sampler=%llu view=%llu layout=%d copies=%u", H64 (w->dstSet), w->dstBinding,
			w->dstArrayElement, w->descriptorCount, w->descriptorType, H64 (w->pImageInfo->sampler), H64 (w->pImageInfo->imageView), w->pImageInfo->imageLayout,
			copy_count);
	}
}

static void c_ref_texmgr_vkCmdPipelineBarrier (
	VkCommandBuffer cb, VkPipelineStageFlags src, VkPipelineStageFlags dst, VkDependencyFlags dep, uint32_t mem_count, const void *mem, uint32_t buf_count,
	const void *buf, uint32_t img_count, const VkImageMemoryBarrier *img)
{
	(void)mem;
	(void)buf;
	trace_append (
		"Barrier cb=%llu src=%u dst=%u dep=%u counts=%u/%u/%u", H64 (cb), (unsigned)src, (unsigned)dst, (unsigned)dep, mem_count, buf_count, img_count);
	for (uint32_t i = 0; i < img_count; i++)
		trace_append (
			"  image access=%u->%u layout=%d->%d qf=%u->%u image=%llu aspect=%u mip=%u/%u layer=%u/%u", (unsigned)img[i].srcAccessMask,
			(unsigned)img[i].dstAccessMask, img[i].oldLayout, img[i].newLayout, img[i].srcQueueFamilyIndex, img[i].dstQueueFamilyIndex, H64 (img[i].image),
			(unsigned)img[i].subresourceRange.aspectMask, img[i].subresourceRange.baseMipLevel, img[i].subresourceRange.levelCount,
			img[i].subresourceRange.baseArrayLayer, img[i].subresourceRange.layerCount);
}

static void c_ref_texmgr_vkCmdCopyBufferToImage (
	VkCommandBuffer cb, VkBuffer buffer, VkImage image, VkImageLayout layout, uint32_t region_count, const VkBufferImageCopy *regions)
{
	trace_append ("CopyBufferToImage cb=%llu buf=%llu image=%llu layout=%d regions=%u", H64 (cb), H64 (buffer), H64 (image), layout, region_count);
	for (uint32_t i = 0; i < region_count; i++)
		trace_append (
			"  region off=%llu rowlen=%u imgh=%u aspect=%u mip=%u layer=%u/%u offset=%d,%d,%d extent=%ux%ux%u", (unsigned long long)regions[i].bufferOffset,
			regions[i].bufferRowLength, regions[i].bufferImageHeight, (unsigned)regions[i].imageSubresource.aspectMask, regions[i].imageSubresource.mipLevel,
			regions[i].imageSubresource.baseArrayLayer, regions[i].imageSubresource.layerCount, regions[i].imageOffset.x, regions[i].imageOffset.y,
			regions[i].imageOffset.z, regions[i].imageExtent.width, regions[i].imageExtent.height, regions[i].imageExtent.depth);
}

#define vkCreateImage				 c_ref_texmgr_vkCreateImage
#define vkDestroyImage				 c_ref_texmgr_vkDestroyImage
#define vkGetImageMemoryRequirements c_ref_texmgr_vkGetImageMemoryRequirements
#define vkBindImageMemory			 c_ref_texmgr_vkBindImageMemory
#define vkCreateImageView			 c_ref_texmgr_vkCreateImageView
#define vkDestroyImageView			 c_ref_texmgr_vkDestroyImageView
#define vkCreateFramebuffer			 c_ref_texmgr_vkCreateFramebuffer
#define vkDestroyFramebuffer		 c_ref_texmgr_vkDestroyFramebuffer
#define vkUpdateDescriptorSets		 c_ref_texmgr_vkUpdateDescriptorSets
#define vkCmdPipelineBarrier		 c_ref_texmgr_vkCmdPipelineBarrier
#define vkCmdCopyBufferToImage		 c_ref_texmgr_vkCmdCopyBufferToImage

/* ---- the gl_rmisc.c / gl_vidsdl.c seams --------------------------------- */
static void c_ref_texmgr_GL_SetObjectName (uint64_t object, VkObjectType type, const char *name)
{
	trace_append ("SetObjectName %llu type=%d '%s'", (unsigned long long)object, type, name);
}

static int c_ref_texmgr_GL_MemoryTypeFromProperties (uint32_t type_bits, VkFlags required, VkFlags preferred)
{
	trace_append ("MemoryType bits=%u req=%u pref=%u -> 3", type_bits, (unsigned)required, (unsigned)preferred);
	return 3;
}

static void c_ref_texmgr_GL_WaitForDeviceIdle (void)
{
	trace_append ("WaitForDeviceIdle");
}

static VkDescriptorSet c_ref_texmgr_R_AllocateDescriptorSet (vulkan_desc_set_layout_t *layout)
{
	uint64_t handle = ++next_handle;
	trace_append ("AllocateDescriptorSet %s -> %llu", layout->num_storage_images ? "cs_write" : "single", (unsigned long long)handle);
	return (VkDescriptorSet)(uintptr_t)handle;
}

static void c_ref_texmgr_R_FreeDescriptorSet (VkDescriptorSet set, vulkan_desc_set_layout_t *layout)
{
	trace_append ("FreeDescriptorSet %llu %s", H64 (set), layout->num_storage_images ? "cs_write" : "single");
}

/* A bump arena stands in for the staging ring; the copy the C makes into it
 * between R_StagingBeginCopy and R_StagingEndCopy is hashed at EndCopy, so
 * the pixel pipeline (palette, alpha fix, premultiply, mip chain, cube face
 * order, 10-bit packing) is compared byte for byte without a device. */
#define STAGING_ARENA_SIZE (8 * 1024 * 1024)
static byte staging_arena[STAGING_ARENA_SIZE];
static int	arena_offset;
static int	staging_last_offset, staging_last_size;

static byte *c_ref_texmgr_R_StagingAllocate (int size, int alignment, VkCommandBuffer *cb, VkBuffer *buffer, int *buffer_offset)
{
	int off = (arena_offset + alignment - 1) / alignment * alignment;
	if (off + size > STAGING_ARENA_SIZE)
		off = 0;
	arena_offset = off + size;
	staging_last_offset = off;
	staging_last_size = size;
	*cb = (VkCommandBuffer)0x1000;
	*buffer = (VkBuffer)0x2000;
	*buffer_offset = off;
	trace_append ("StagingAllocate size=%d align=%d -> off=%d", size, alignment, off);
	return staging_arena + off;
}

static void c_ref_texmgr_R_StagingBeginCopy (void)
{
	trace_append ("StagingBeginCopy");
}

static uint64_t fnv1a (const byte *p, size_t n)
{
	uint64_t h = 0xcbf29ce484222325ULL;
	for (size_t i = 0; i < n; i++)
		h = (h ^ p[i]) * 0x100000001b3ULL;
	return h;
}

static void c_ref_texmgr_R_StagingEndCopy (void)
{
	trace_append ("StagingEndCopy hash=%016llx", (unsigned long long)fnv1a (staging_arena + staging_last_offset, (size_t)staging_last_size));
}

#define GL_SetObjectName			c_ref_texmgr_GL_SetObjectName
#define GL_MemoryTypeFromProperties c_ref_texmgr_GL_MemoryTypeFromProperties
#define GL_WaitForDeviceIdle		c_ref_texmgr_GL_WaitForDeviceIdle
#define R_AllocateDescriptorSet		c_ref_texmgr_R_AllocateDescriptorSet
#define R_FreeDescriptorSet			c_ref_texmgr_R_FreeDescriptorSet
#define R_StagingAllocate			c_ref_texmgr_R_StagingAllocate
#define R_StagingBeginCopy			c_ref_texmgr_R_StagingBeginCopy
#define R_StagingEndCopy			c_ref_texmgr_R_StagingEndCopy

/* ---- console / error ---------------------------------------------------- */
static void c_ref_texmgr_Con_Printf (const char *fmt, ...)
{
	char	buf[1024];
	va_list ap;
	va_start (ap, fmt);
	vsnprintf (buf, sizeof (buf), fmt, ap);
	va_end (ap);
	trace_append ("Con: %s", buf);
}

static void c_ref_texmgr_Sys_Error (const char *fmt, ...)
{
	va_list ap;
	va_start (ap, fmt);
	fprintf (stderr, "gl_texmgr_ref Sys_Error: ");
	vfprintf (stderr, fmt, ap);
	fprintf (stderr, "\n");
	va_end (ap);
	abort ();
}

#define Con_Printf	   c_ref_texmgr_Con_Printf
#define Con_SafePrintf c_ref_texmgr_Con_Printf
#define Sys_Error	   c_ref_texmgr_Sys_Error

/* ---- the fake file system ------------------------------------------------
 * COM_FOpenFile / Sys_fseek / fread / fclose become reads of an in-memory
 * table (the prelude already renames COM_FOpenFile to the real
 * c_ref_COM_FOpenFile, undone here); Image_LoadImage answers from a second
 * table with a Mem_Alloc'ed copy, as the engine's does. */
typedef struct
{
	char   name[MAX_QPATH];
	byte  *data;
	size_t size;
} fake_file_t;
#define MAX_FAKE_FILES 32
static fake_file_t fake_files[MAX_FAKE_FILES];
static int		   num_fake_files;

typedef struct
{
	char   name[MAX_QPATH];
	int	   width, height;
	int	   format;
	byte  *data;
	size_t size;
} fake_image_file_t;
static fake_image_file_t fake_image_files[MAX_FAKE_FILES];
static int				 num_fake_image_files;

static struct
{
	const fake_file_t *file;
	size_t			   pos;
	qboolean		   open;
} open_file;

void c_ref_texmgr_files_reset (void)
{
	for (int i = 0; i < num_fake_files; i++)
		free (fake_files[i].data);
	for (int i = 0; i < num_fake_image_files; i++)
		free (fake_image_files[i].data);
	num_fake_files = 0;
	num_fake_image_files = 0;
}

void c_ref_texmgr_add_file (const char *name, const byte *data, size_t size)
{
	assert (num_fake_files < MAX_FAKE_FILES);
	fake_file_t *f = &fake_files[num_fake_files++];
	q_strlcpy (f->name, name, sizeof (f->name));
	f->data = (byte *)malloc (size ? size : 1);
	memcpy (f->data, data, size);
	f->size = size;
}

void c_ref_texmgr_add_image (const char *name, int width, int height, int format, const byte *data, size_t size)
{
	assert (num_fake_image_files < MAX_FAKE_FILES);
	fake_image_file_t *f = &fake_image_files[num_fake_image_files++];
	q_strlcpy (f->name, name, sizeof (f->name));
	f->width = width;
	f->height = height;
	f->format = format;
	f->data = (byte *)malloc (size ? size : 1);
	memcpy (f->data, data, size);
	f->size = size;
}

static qfilesize_t c_ref_texmgr_COM_FOpenFile (const char *filename, FILE **file, unsigned int *path_id)
{
	(void)path_id;
	for (int i = 0; i < num_fake_files; i++)
		if (!strcmp (fake_files[i].name, filename))
		{
			open_file.file = &fake_files[i];
			open_file.pos = 0;
			open_file.open = true;
			*file = (FILE *)&open_file;
			trace_append ("FOpen '%s' -> found", filename);
			return (qfilesize_t)fake_files[i].size;
		}
	*file = NULL;
	trace_append ("FOpen '%s' -> none", filename);
	return -1;
}

static int c_ref_texmgr_Sys_fseek (FILE *file, qfileofs_t ofs, int origin)
{
	(void)file;
	assert (origin == SEEK_CUR);
	open_file.pos += (size_t)ofs;
	trace_append ("Seek %llu", (unsigned long long)ofs);
	return 0;
}

static size_t c_ref_texmgr_fread (void *dst, size_t size, size_t count, FILE *file)
{
	(void)file;
	size_t want = size * count;
	size_t avail = open_file.pos < open_file.file->size ? open_file.file->size - open_file.pos : 0;
	size_t got = want < avail ? want : avail;
	memcpy (dst, open_file.file->data + open_file.pos, got);
	open_file.pos += got;
	trace_append ("Read %llu -> %llu", (unsigned long long)want, (unsigned long long)got);
	return got / size;
}

static int c_ref_texmgr_fclose (FILE *file)
{
	(void)file;
	open_file.open = false;
	trace_append ("Close");
	return 0;
}

static byte *c_ref_texmgr_Image_LoadImage (const char *name, int *width, int *height, enum srcformat *fmt, unsigned int min_path_id)
{
	for (int i = 0; i < num_fake_image_files; i++)
		if (!strcmp (fake_image_files[i].name, name))
		{
			const fake_image_file_t *f = &fake_image_files[i];
			byte					*out = (byte *)Mem_Alloc (f->size);
			memcpy (out, f->data, f->size);
			*width = f->width;
			*height = f->height;
			*fmt = (enum srcformat)f->format;
			trace_append ("ImageLoad '%s' path=%u -> %dx%d fmt=%d", name, min_path_id, f->width, f->height, f->format);
			return out;
		}
	trace_append ("ImageLoad '%s' path=%u -> none", name, min_path_id);
	return NULL;
}

#undef COM_FOpenFile
#define COM_FOpenFile	c_ref_texmgr_COM_FOpenFile
#define Sys_fseek		c_ref_texmgr_Sys_fseek
#define fread			c_ref_texmgr_fread
#define fclose			c_ref_texmgr_fclose
#define Image_LoadImage c_ref_texmgr_Image_LoadImage

/* TexMgr_Init's registrations are not the differential's subject; the cvar
 * values come through c_ref_texmgr_set_cvars and imagelist is excluded. */
#undef Cvar_RegisterVariable
#define Cvar_RegisterVariable(v) ((void)(v))
#undef Cmd_AddCommand
#define Cmd_AddCommand(name, func) ((void)(func), (cmd_function_t *)NULL)

/* The blue-noise RGBA tile is built in a TEMP_ALLOC buffer whose alpha bytes
 * the C never writes; on the alloca path that is uninitialised stack. Zero
 * it here so the C oracle is deterministic (the Rust port zeroes it too). */
#undef alloca
#if defined(_MSC_VER)
#define alloca(n) memset (_alloca (n), 0, (n))
#else
#define alloca(n) memset (__builtin_alloca (n), 0, (n))
#endif

/* The M3 oracle's renames, so the texture heap is the C gl_heap.c with its
 * sequential fake memory handles (stubs/gl_heap_ref.c). */
#define GL_HeapCreate			   c_ref_GL_HeapCreate
#define GL_HeapDestroy			   c_ref_GL_HeapDestroy
#define GL_HeapAllocate			   c_ref_GL_HeapAllocate
#define GL_HeapFree				   c_ref_GL_HeapFree
#define GL_HeapGetAllocationMemory c_ref_GL_HeapGetAllocationMemory
#define GL_HeapGetAllocationOffset c_ref_GL_HeapGetAllocationOffset
#define GL_HeapGetStats			   c_ref_GL_HeapGetStats

/* The public entry points, renamed so the real gl_texmgr.c's symbols (and
 * the stub doubles in stubs.c / r_part_fte_ref.c / host_ref.c) do not clash. */
#define TexMgr_UpdateTextureDescriptorSets c_ref_TexMgr_UpdateTextureDescriptorSets
#define TexMgr_FindTexture				   c_ref_TexMgr_FindTexture
#define TexMgr_NewTexture				   c_ref_TexMgr_NewTexture
#define TexMgr_FreeTexture				   c_ref_TexMgr_FreeTexture
#define TexMgr_FreeTextures				   c_ref_TexMgr_FreeTextures
#define TexMgr_FreeTexturesForOwner		   c_ref_TexMgr_FreeTexturesForOwner
#define TexMgr_DeleteTextureObjects		   c_ref_TexMgr_DeleteTextureObjects
#define TexMgr_InitHeap					   c_ref_TexMgr_InitHeap
#define TexMgr_LoadPalette				   c_ref_TexMgr_LoadPalette
#define TexMgr_NewGame					   c_ref_TexMgr_NewGame
#define TexMgr_Init						   c_ref_TexMgr_Init
#define TexMgr_LoadImage				   c_ref_TexMgr_LoadImage
#define TexMgr_ReloadImage				   c_ref_TexMgr_ReloadImage
#define TexMgr_ReloadNobrightImages		   c_ref_TexMgr_ReloadNobrightImages
#define TexMgr_CollectGarbage			   c_ref_TexMgr_CollectGarbage
#define notexture						   c_ref_texmgr_notexture
#define nulltexture						   c_ref_texmgr_nulltexture
#define whitetexture					   c_ref_texmgr_whitetexture
#define greytexture						   c_ref_texmgr_greytexture
#define greylightmap					   c_ref_texmgr_greylightmap
#define bluenoisetexture				   c_ref_texmgr_bluenoisetexture
#define d_8to24table					   c_ref_texmgr_d_8to24table
#define d_8to24table_fbright			   c_ref_texmgr_d_8to24table_fbright
#define d_8to24table_fbright_fence		   c_ref_texmgr_d_8to24table_fbright_fence
#define d_8to24table_nobright			   c_ref_texmgr_d_8to24table_nobright
#define d_8to24table_nobright_fence		   c_ref_texmgr_d_8to24table_nobright_fence
#define d_8to24table_conchars			   c_ref_texmgr_d_8to24table_conchars

#include "gl_heap.h"

/* gl_texmgr.h's prototypes for the entry points gl_texmgr.c calls before
 * defining them (the prelude declares only TexMgr_LoadImage, under its
 * pre-rename spelling). */
gltexture_t *TexMgr_LoadImage (
	qmodel_t *owner, const char *name, int width, int height, enum srcformat format, byte *data, const char *source_file, src_offset_t source_offset,
	unsigned flags);
void		 TexMgr_ReloadImage (gltexture_t *glt, int shirt, int pants);
void		 TexMgr_FreeTexture (gltexture_t *kill);
gltexture_t *TexMgr_FindTexture (qmodel_t *owner, const char *name);
gltexture_t *TexMgr_NewTexture (void);
void		 TexMgr_CollectGarbage (void);

#include "gl_texmgr.c"

/* ---- test-facing helpers ------------------------------------------------ */
void c_ref_texmgr_set_cvars (float fullbrights, float filter, float anisotropic, float max_size, float picmip)
{
	c_ref_texmgr_gl_fullbrights.value = fullbrights;
	c_ref_texmgr_vid_filter.value = filter;
	c_ref_texmgr_vid_anisotropic.value = anisotropic;
	gl_max_size.value = max_size;
	gl_picmip.value = picmip;
}

void c_ref_texmgr_set_env (
	uint32_t max_2d, uint32_t max_cube, int color_format, uint64_t point, uint64_t linear, uint64_t point_aniso, uint64_t linear_aniso,
	uint64_t warp_render_pass)
{
	c_ref_texmgr_globals.device_properties.limits.maxImageDimension2D = max_2d;
	c_ref_texmgr_globals.device_properties.limits.maxImageDimensionCube = max_cube;
	c_ref_texmgr_globals.color_format = color_format;
	c_ref_texmgr_globals.point_sampler_lod_bias = (VkSampler)(uintptr_t)point;
	c_ref_texmgr_globals.linear_sampler_lod_bias = (VkSampler)(uintptr_t)linear;
	c_ref_texmgr_globals.point_aniso_sampler_lod_bias = (VkSampler)(uintptr_t)point_aniso;
	c_ref_texmgr_globals.linear_aniso_sampler_lod_bias = (VkSampler)(uintptr_t)linear_aniso;
	c_ref_texmgr_globals.warp_render_pass = (VkRenderPass)(uintptr_t)warp_render_pass;
}

void c_ref_texmgr_set_in_update_screen (qboolean value)
{
	c_ref_texmgr_in_update_screen = value;
}

int c_ref_texmgr_num_textures (void)
{
	return numgltextures;
}

gltexture_t *c_ref_texmgr_active_head (void)
{
	return active_gltextures;
}

uint32_t c_ref_texmgr_num_vulkan_allocations (void)
{
	return Atomic_LoadUInt32 (&c_ref_texmgr_num_vulkan_tex_allocations);
}

/* The static TexMgr_Downsample (stb_image_resize), which the Rust port
 * calls back into as its `downsample` seam: stb stays C on both sides. */
void ctest_texmgr_downsample (unsigned *data, int in_width, int in_height, int out_width, int out_height)
{
	TexMgr_Downsample (data, in_width, in_height, out_width, out_height);
}
