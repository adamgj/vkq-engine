/* ABI probe: reports what the real engine headers say on THIS platform, so
 * tests/fs_abi.rs can compare against the Rust mirrors in quake-types::fs.
 *
 * MAX_OSPATH is PATH_MAX, which q_types.h derives from a four-way
 * MAXPATHLEN/_MAX_PATH/PATH_MAX/fallback chain. The Rust side cannot read
 * that at build time -- the bindgen output is generated on one host and
 * committed -- so it hardcodes a per-target cfg ladder, i.e. a guess about
 * the C toolchain. Under -Duse_rust_fs the C walks Rust-allocated
 * searchpath_t and pack_t nodes directly, so a wrong guess is silent memory
 * corruption rather than a link error. Hence this probe, compiled from the
 * engine's own headers on every platform CI runs the test suite on.
 */

#include "common.h"
#include "pakfile.h"
#include "cvar.h"
#include "q_thread.h"
#include "q_sound.h"
/* Phase 5 net: net_sys.h needs the PLATFORM_* detection from arch_def.h and
 * must precede net_defs.h; NET_MAXMESSAGE/qhostaddr_t come from net.h */
#include "arch_def.h"
#include "net_sys.h"
#include "net.h"
#include "net_defs.h"

size_t ctest_abi_max_ospath (void)
{
	return (size_t)MAX_OSPATH;
}

size_t ctest_abi_max_qpath (void)
{
	return (size_t)MAX_QPATH;
}

/* Indices must match the ABI_SIZEOF_* constants in tests/fs_abi.rs */
size_t ctest_abi_sizeof (int which)
{
	switch (which)
	{
	case 0:
		return sizeof (searchpath_t);
	case 1:
		return sizeof (pack_t);
	case 2:
		return sizeof (packfile_t);
	case 3:
		return sizeof (dpackfile_t);
	case 4:
		return sizeof (dpackheader_t);
	default:
		return (size_t)-1;
	}
}

/* Indices must match the ABI_OFFSET_* constants in tests/fs_abi.rs */
size_t ctest_abi_offsetof (int which)
{
	switch (which)
	{
	case 0:
		return offsetof (searchpath_t, path_id);
	case 1:
		return offsetof (searchpath_t, filename);
	case 2:
		return offsetof (searchpath_t, pack);
	case 3:
		return offsetof (searchpath_t, dir);
	case 4:
		return offsetof (searchpath_t, next);
	case 5:
		return offsetof (pack_t, filename);
	case 6:
		return offsetof (pack_t, handle);
	case 7:
		return offsetof (pack_t, numfiles);
	case 8:
		return offsetof (pack_t, files);
	case 9:
		return offsetof (packfile_t, name);
	case 10:
		return offsetof (packfile_t, filepos);
	case 11:
		return offsetof (packfile_t, filelen);
	default:
		return (size_t)-1;
	}
}

/* ---------------------------------------------------------------------------
 * Phase 3 (BSP/model) ABI probe: name-keyed so the Rust consumer
 * (tests/bsp_abi.rs) and this table can't drift by index. gl_model.h /
 * bspfile.h arrive via the force-included c_ref_prelude.h (with the Vk
 * handle stand-ins -- the probed fields never include the Vk tail's
 * *contents*, only its offsets, which the stand-ins reproduce on 64-bit).
 */

#include <string.h>

typedef struct
{
	const char *name;
	size_t		value;
} ctest_abi_entry_t;

#define SZ(tag, type)          {"sizeof." tag, sizeof (type)}
#define OFF(tag, type, member) {tag "." #member, offsetof (type, member)}

static const ctest_abi_entry_t ctest_abi_bsp_table[] = {
	SZ ("qmodel_t", qmodel_t),
	SZ ("texture_t", texture_t),
	SZ ("msurface_t", msurface_t),
	SZ ("mnode_t", mnode_t),
	SZ ("mleaf_t", mleaf_t),
	SZ ("mclipnode_t", mclipnode_t),
	SZ ("hull_t", hull_t),
	SZ ("mtexinfo_t", mtexinfo_t),
	SZ ("medge_t", medge_t),
	SZ ("mvertex_t", mvertex_t),
	SZ ("mplane_t", mplane_t),
	SZ ("dmodel_t", dmodel_t),
	SZ ("dheader_t", dheader_t),
	SZ ("lump_t", lump_t),
	SZ ("dmiptexlump_t", dmiptexlump_t),
	SZ ("miptex_t", miptex_t),
	SZ ("miptex64_t", miptex64_t),
	SZ ("dvertex_t", dvertex_t),
	SZ ("dplane_t", dplane_t),
	SZ ("dsnode_t", dsnode_t),
	SZ ("dl1node_t", dl1node_t),
	SZ ("dl2node_t", dl2node_t),
	SZ ("dsclipnode_t", dsclipnode_t),
	SZ ("dlclipnode_t", dlclipnode_t),
	SZ ("texinfo_t", texinfo_t),
	SZ ("dsedge_t", dsedge_t),
	SZ ("dledge_t", dledge_t),
	SZ ("dsface_t", dsface_t),
	SZ ("dlface_t", dlface_t),
	SZ ("dsleaf_t", dsleaf_t),
	SZ ("dl1leaf_t", dl1leaf_t),
	SZ ("dl2leaf_t", dl2leaf_t),

	OFF ("qmodel_t", qmodel_t, name),
	OFF ("qmodel_t", qmodel_t, needload),
	OFF ("qmodel_t", qmodel_t, type),
	OFF ("qmodel_t", qmodel_t, flags),
	OFF ("qmodel_t", qmodel_t, mins),
	OFF ("qmodel_t", qmodel_t, maxs),
	OFF ("qmodel_t", qmodel_t, ymins),
	OFF ("qmodel_t", qmodel_t, rmins),
	OFF ("qmodel_t", qmodel_t, clipbox),
	OFF ("qmodel_t", qmodel_t, clipmins),
	OFF ("qmodel_t", qmodel_t, firstmodelsurface),
	OFF ("qmodel_t", qmodel_t, nummodelsurfaces),
	OFF ("qmodel_t", qmodel_t, numsubmodels),
	OFF ("qmodel_t", qmodel_t, submodels),
	OFF ("qmodel_t", qmodel_t, numplanes),
	OFF ("qmodel_t", qmodel_t, planes),
	OFF ("qmodel_t", qmodel_t, numleafs),
	OFF ("qmodel_t", qmodel_t, leafs),
	OFF ("qmodel_t", qmodel_t, numvertexes),
	OFF ("qmodel_t", qmodel_t, vertexes),
	OFF ("qmodel_t", qmodel_t, numedges),
	OFF ("qmodel_t", qmodel_t, edges),
	OFF ("qmodel_t", qmodel_t, numnodes),
	OFF ("qmodel_t", qmodel_t, nodes),
	OFF ("qmodel_t", qmodel_t, numtexinfo),
	OFF ("qmodel_t", qmodel_t, texinfo),
	OFF ("qmodel_t", qmodel_t, numsurfaces),
	OFF ("qmodel_t", qmodel_t, surfaces),
	OFF ("qmodel_t", qmodel_t, numsurfedges),
	OFF ("qmodel_t", qmodel_t, surfedges),
	OFF ("qmodel_t", qmodel_t, numclipnodes),
	OFF ("qmodel_t", qmodel_t, clipnodes),
	OFF ("qmodel_t", qmodel_t, nummarksurfaces),
	OFF ("qmodel_t", qmodel_t, marksurfaces),
	OFF ("qmodel_t", qmodel_t, soa_leafbounds),
	OFF ("qmodel_t", qmodel_t, hulls),
	OFF ("qmodel_t", qmodel_t, numtextures),
	OFF ("qmodel_t", qmodel_t, textures),
	OFF ("qmodel_t", qmodel_t, texofs),
	OFF ("qmodel_t", qmodel_t, usedtextures),
	OFF ("qmodel_t", qmodel_t, visdata),
	OFF ("qmodel_t", qmodel_t, lightdata),
	OFF ("qmodel_t", qmodel_t, entities),
	OFF ("qmodel_t", qmodel_t, viswarn),
	OFF ("qmodel_t", qmodel_t, bogus_tree),
	OFF ("qmodel_t", qmodel_t, bspversion),
	OFF ("qmodel_t", qmodel_t, contentstransparent),
	OFF ("qmodel_t", qmodel_t, used_specials),
	OFF ("qmodel_t", qmodel_t, extradata),
	OFF ("qmodel_t", qmodel_t, blas),

	OFF ("texture_t", texture_t, name),
	OFF ("texture_t", texture_t, width),
	OFF ("texture_t", texture_t, height),
	OFF ("texture_t", texture_t, shift),
	OFF ("texture_t", texture_t, type),
	OFF ("texture_t", texture_t, source_file),
	OFF ("texture_t", texture_t, source_offset),
	OFF ("texture_t", texture_t, update_warp),
	OFF ("texture_t", texture_t, texturechains),
	OFF ("texture_t", texture_t, chain_size),
	OFF ("texture_t", texture_t, anim_total),
	OFF ("texture_t", texture_t, anim_min),
	OFF ("texture_t", texture_t, anim_max),
	OFF ("texture_t", texture_t, anim_next),
	OFF ("texture_t", texture_t, alternate_anims),
	OFF ("texture_t", texture_t, offsets),
	OFF ("texture_t", texture_t, palette),

	OFF ("msurface_t", msurface_t, visframe),
	OFF ("msurface_t", msurface_t, plane),
	OFF ("msurface_t", msurface_t, flags),
	OFF ("msurface_t", msurface_t, firstedge),
	OFF ("msurface_t", msurface_t, numedges),
	OFF ("msurface_t", msurface_t, texturemins),
	OFF ("msurface_t", msurface_t, extents),
	OFF ("msurface_t", msurface_t, polys),
	OFF ("msurface_t", msurface_t, texinfo),
	OFF ("msurface_t", msurface_t, dlightbits),
	OFF ("msurface_t", msurface_t, styles),
	OFF ("msurface_t", msurface_t, styles_bitmap),
	OFF ("msurface_t", msurface_t, cached_light),
	OFF ("msurface_t", msurface_t, cached_dlight),
	OFF ("msurface_t", msurface_t, samples),

	OFF ("mnode_t", mnode_t, contents),
	OFF ("mnode_t", mnode_t, minmaxs),
	OFF ("mnode_t", mnode_t, firstsurface),
	OFF ("mnode_t", mnode_t, numsurfaces),
	OFF ("mnode_t", mnode_t, plane),
	OFF ("mnode_t", mnode_t, children),

	OFF ("mleaf_t", mleaf_t, contents),
	OFF ("mleaf_t", mleaf_t, minmaxs),
	OFF ("mleaf_t", mleaf_t, nummarksurfaces),
	OFF ("mleaf_t", mleaf_t, combined_deps),
	OFF ("mleaf_t", mleaf_t, ambient_sound_level),
	OFF ("mleaf_t", mleaf_t, compressed_vis),
	OFF ("mleaf_t", mleaf_t, firstmarksurface),
	OFF ("mleaf_t", mleaf_t, efrags),

	OFF ("mclipnode_t", mclipnode_t, planenum),
	OFF ("mclipnode_t", mclipnode_t, children),

	OFF ("hull_t", hull_t, clipnodes),
	OFF ("hull_t", hull_t, planes),
	OFF ("hull_t", hull_t, firstclipnode),
	OFF ("hull_t", hull_t, lastclipnode),
	OFF ("hull_t", hull_t, clip_mins),
	OFF ("hull_t", hull_t, clip_maxs),

	OFF ("mtexinfo_t", mtexinfo_t, vecs),
	OFF ("mtexinfo_t", mtexinfo_t, texture),
	OFF ("mtexinfo_t", mtexinfo_t, flags),
	OFF ("mtexinfo_t", mtexinfo_t, tex_idx),

	OFF ("medge_t", medge_t, v),
	OFF ("medge_t", medge_t, cachededgeoffset),
};

/* Phase 3 M4: the MDL/SPR half of the same seam, in its own table (and its
 * own lookup) so the two milestones' entries stay independently reviewable. */
static const ctest_abi_entry_t ctest_abi_alias_table[] = {
	SZ ("mdl_t", mdl_t),
	SZ ("stvert_t", stvert_t),
	SZ ("dtriangle_t", dtriangle_t),
	SZ ("trivertx_t", trivertx_t),
	SZ ("daliasframe_t", daliasframe_t),
	SZ ("daliasgroup_t", daliasgroup_t),
	SZ ("daliasskingroup_t", daliasskingroup_t),
	SZ ("daliasinterval_t", daliasinterval_t),
	SZ ("daliasskininterval_t", daliasskininterval_t),
	SZ ("daliasframetype_t", daliasframetype_t),
	SZ ("daliasskintype_t", daliasskintype_t),
	SZ ("dsprite_t", dsprite_t),
	SZ ("dspriteframe_t", dspriteframe_t),
	SZ ("dspritegroup_t", dspritegroup_t),
	SZ ("dspriteinterval_t", dspriteinterval_t),
	SZ ("dspriteframetype_t", dspriteframetype_t),
	SZ ("aliashdr_t", aliashdr_t),
	SZ ("maliasframedesc_t", maliasframedesc_t),
	SZ ("mtriangle_t", mtriangle_t),
	SZ ("md5vert_t", md5vert_t),
	SZ ("md5vert8_t", md5vert8_t),
	SZ ("md3XyzNormal_t", md3XyzNormal_t),
	SZ ("msprite_t", msprite_t),
	SZ ("mspriteframe_t", mspriteframe_t),
	SZ ("mspritegroup_t", mspritegroup_t),
	SZ ("mspriteframedesc_t", mspriteframedesc_t),

	OFF ("mdl_t", mdl_t, ident),
	OFF ("mdl_t", mdl_t, version),
	OFF ("mdl_t", mdl_t, scale),
	OFF ("mdl_t", mdl_t, scale_origin),
	OFF ("mdl_t", mdl_t, boundingradius),
	OFF ("mdl_t", mdl_t, eyeposition),
	OFF ("mdl_t", mdl_t, numskins),
	OFF ("mdl_t", mdl_t, skinwidth),
	OFF ("mdl_t", mdl_t, skinheight),
	OFF ("mdl_t", mdl_t, numverts),
	OFF ("mdl_t", mdl_t, numtris),
	OFF ("mdl_t", mdl_t, numframes),
	OFF ("mdl_t", mdl_t, synctype),
	OFF ("mdl_t", mdl_t, flags),
	OFF ("mdl_t", mdl_t, size),

	OFF ("stvert_t", stvert_t, onseam),
	OFF ("stvert_t", stvert_t, s),
	OFF ("stvert_t", stvert_t, t),

	OFF ("dtriangle_t", dtriangle_t, facesfront),
	OFF ("dtriangle_t", dtriangle_t, vertindex),

	OFF ("trivertx_t", trivertx_t, v),
	OFF ("trivertx_t", trivertx_t, lightnormalindex),

	OFF ("daliasframe_t", daliasframe_t, bboxmin),
	OFF ("daliasframe_t", daliasframe_t, bboxmax),
	OFF ("daliasframe_t", daliasframe_t, name),

	OFF ("daliasgroup_t", daliasgroup_t, numframes),
	OFF ("daliasgroup_t", daliasgroup_t, bboxmin),
	OFF ("daliasgroup_t", daliasgroup_t, bboxmax),

	OFF ("dsprite_t", dsprite_t, ident),
	OFF ("dsprite_t", dsprite_t, version),
	OFF ("dsprite_t", dsprite_t, type),
	OFF ("dsprite_t", dsprite_t, boundingradius),
	OFF ("dsprite_t", dsprite_t, width),
	OFF ("dsprite_t", dsprite_t, height),
	OFF ("dsprite_t", dsprite_t, numframes),
	OFF ("dsprite_t", dsprite_t, beamlength),
	OFF ("dsprite_t", dsprite_t, synctype),

	OFF ("dspriteframe_t", dspriteframe_t, origin),
	OFF ("dspriteframe_t", dspriteframe_t, width),
	OFF ("dspriteframe_t", dspriteframe_t, height),

	OFF ("dspritegroup_t", dspritegroup_t, numframes),
	OFF ("dspriteinterval_t", dspriteinterval_t, interval),

	OFF ("aliashdr_t", aliashdr_t, ident),
	OFF ("aliashdr_t", aliashdr_t, version),
	OFF ("aliashdr_t", aliashdr_t, scale),
	OFF ("aliashdr_t", aliashdr_t, scale_origin),
	OFF ("aliashdr_t", aliashdr_t, boundingradius),
	OFF ("aliashdr_t", aliashdr_t, eyeposition),
	OFF ("aliashdr_t", aliashdr_t, numskins),
	OFF ("aliashdr_t", aliashdr_t, skinwidth),
	OFF ("aliashdr_t", aliashdr_t, skinheight),
	OFF ("aliashdr_t", aliashdr_t, numverts),
	OFF ("aliashdr_t", aliashdr_t, numtris),
	OFF ("aliashdr_t", aliashdr_t, numframes),
	OFF ("aliashdr_t", aliashdr_t, synctype),
	OFF ("aliashdr_t", aliashdr_t, flags),
	OFF ("aliashdr_t", aliashdr_t, size),
	OFF ("aliashdr_t", aliashdr_t, numindexes),
	OFF ("aliashdr_t", aliashdr_t, numverts_vbo),
	OFF ("aliashdr_t", aliashdr_t, numposes),
	OFF ("aliashdr_t", aliashdr_t, nextsurface),
	OFF ("aliashdr_t", aliashdr_t, numjoints),
	OFF ("aliashdr_t", aliashdr_t, poseverttype),
	OFF ("aliashdr_t", aliashdr_t, gltextures),
	OFF ("aliashdr_t", aliashdr_t, fbtextures),
	OFF ("aliashdr_t", aliashdr_t, texels),
	OFF ("aliashdr_t", aliashdr_t, vertex_buffer),
	OFF ("aliashdr_t", aliashdr_t, vbostofs),
	OFF ("aliashdr_t", aliashdr_t, joints_set),
	OFF ("aliashdr_t", aliashdr_t, frames),

	OFF ("maliasframedesc_t", maliasframedesc_t, firstpose),
	OFF ("maliasframedesc_t", maliasframedesc_t, numposes),
	OFF ("maliasframedesc_t", maliasframedesc_t, interval),
	OFF ("maliasframedesc_t", maliasframedesc_t, bboxmin),
	OFF ("maliasframedesc_t", maliasframedesc_t, bboxmax),
	OFF ("maliasframedesc_t", maliasframedesc_t, frame),
	OFF ("maliasframedesc_t", maliasframedesc_t, name),

	OFF ("mtriangle_t", mtriangle_t, facesfront),
	OFF ("mtriangle_t", mtriangle_t, vertindex),

	OFF ("md5vert_t", md5vert_t, xyz),
	OFF ("md5vert8_t", md5vert8_t, xyz),
	OFF ("md3XyzNormal_t", md3XyzNormal_t, xyz),
	OFF ("md3XyzNormal_t", md3XyzNormal_t, latlong),

	OFF ("msprite_t", msprite_t, type),
	OFF ("msprite_t", msprite_t, maxwidth),
	OFF ("msprite_t", msprite_t, maxheight),
	OFF ("msprite_t", msprite_t, numframes),
	OFF ("msprite_t", msprite_t, frames),

	OFF ("mspriteframe_t", mspriteframe_t, width),
	OFF ("mspriteframe_t", mspriteframe_t, height),
	OFF ("mspriteframe_t", mspriteframe_t, up),
	OFF ("mspriteframe_t", mspriteframe_t, down),
	OFF ("mspriteframe_t", mspriteframe_t, left),
	OFF ("mspriteframe_t", mspriteframe_t, right),
	OFF ("mspriteframe_t", mspriteframe_t, smax),
	OFF ("mspriteframe_t", mspriteframe_t, tmax),
	OFF ("mspriteframe_t", mspriteframe_t, gltexture),

	OFF ("mspritegroup_t", mspritegroup_t, numframes),
	OFF ("mspritegroup_t", mspritegroup_t, intervals),
	OFF ("mspritegroup_t", mspritegroup_t, frames),

	OFF ("mspriteframedesc_t", mspriteframedesc_t, type),
	OFF ("mspriteframedesc_t", mspriteframedesc_t, frameptr),
};


/* Phase 3 M5: the MD3/MD5 half of the same seam -- the on-disk MD3 records,
 * the MD5 in-memory vertex layouts the shim writes through, jointpose_t /
 * aliasmesh_t (handed to GLMesh_UploadBuffers) and the skin-definition block
 * the MD3 loader Mem_Allocs for the still-C .skin plumbing. */
static const ctest_abi_entry_t ctest_abi_mdx_table[] = {
	SZ ("md3Header_t", md3Header_t),
	SZ ("md3Frame_t", md3Frame_t),
	SZ ("md3Surface_t", md3Surface_t),
	SZ ("md3Triangle_t", md3Triangle_t),
	SZ ("md3St_t", md3St_t),
	SZ ("md3Shader_t", md3Shader_t),
	SZ ("jointpose_t", jointpose_t),
	SZ ("aliasmesh_t", aliasmesh_t),
	SZ ("qpath_str_t", qpath_str_t),
	SZ ("skin_def_t", skin_def_t),
	SZ ("surface_def_t", surface_def_t),
	SZ ("all_surfaces_def_t", all_surfaces_def_t),

	OFF ("md3Header_t", md3Header_t, ident),
	OFF ("md3Header_t", md3Header_t, version),
	OFF ("md3Header_t", md3Header_t, name),
	OFF ("md3Header_t", md3Header_t, flags),
	OFF ("md3Header_t", md3Header_t, numFrames),
	OFF ("md3Header_t", md3Header_t, numTags),
	OFF ("md3Header_t", md3Header_t, numSurfaces),
	OFF ("md3Header_t", md3Header_t, numSkins),
	OFF ("md3Header_t", md3Header_t, ofsFrames),
	OFF ("md3Header_t", md3Header_t, ofsTags),
	OFF ("md3Header_t", md3Header_t, ofsSurfaces),
	OFF ("md3Header_t", md3Header_t, ofsEnd),

	OFF ("md3Frame_t", md3Frame_t, bounds),
	OFF ("md3Frame_t", md3Frame_t, localOrigin),
	OFF ("md3Frame_t", md3Frame_t, radius),
	OFF ("md3Frame_t", md3Frame_t, name),

	OFF ("md3Surface_t", md3Surface_t, ident),
	OFF ("md3Surface_t", md3Surface_t, name),
	OFF ("md3Surface_t", md3Surface_t, flags),
	OFF ("md3Surface_t", md3Surface_t, numFrames),
	OFF ("md3Surface_t", md3Surface_t, numShaders),
	OFF ("md3Surface_t", md3Surface_t, numVerts),
	OFF ("md3Surface_t", md3Surface_t, numTriangles),
	OFF ("md3Surface_t", md3Surface_t, ofsTriangles),
	OFF ("md3Surface_t", md3Surface_t, ofsShaders),
	OFF ("md3Surface_t", md3Surface_t, ofsSt),
	OFF ("md3Surface_t", md3Surface_t, ofsXyzNormals),
	OFF ("md3Surface_t", md3Surface_t, ofsEnd),

	OFF ("md3Triangle_t", md3Triangle_t, indexes),
	OFF ("md3St_t", md3St_t, s),
	OFF ("md3St_t", md3St_t, t),
	OFF ("md3Shader_t", md3Shader_t, name),
	OFF ("md3Shader_t", md3Shader_t, shaderIndex),

	OFF ("jointpose_t", jointpose_t, mat),
	OFF ("aliasmesh_t", aliasmesh_t, st),
	OFF ("aliasmesh_t", aliasmesh_t, vertindex),

	OFF ("md5vert_t", md5vert_t, xyz),
	OFF ("md5vert_t", md5vert_t, norm),
	OFF ("md5vert_t", md5vert_t, st),
	OFF ("md5vert_t", md5vert_t, joint_weights),
	OFF ("md5vert_t", md5vert_t, joint_indices),
	OFF ("md5vert_t", md5vert_t, joint_position_x),
	OFF ("md5vert_t", md5vert_t, joint_position_y),
	OFF ("md5vert_t", md5vert_t, joint_position_z),

	OFF ("md5vert8_t", md5vert8_t, xyz),
	OFF ("md5vert8_t", md5vert8_t, norm),
	OFF ("md5vert8_t", md5vert8_t, st),
	OFF ("md5vert8_t", md5vert8_t, joint_weights),
	OFF ("md5vert8_t", md5vert8_t, joint_indices),
	OFF ("md5vert8_t", md5vert8_t, joint_position_x),
	OFF ("md5vert8_t", md5vert8_t, joint_position_y),
	OFF ("md5vert8_t", md5vert8_t, joint_position_z),

	OFF ("qpath_str_t", qpath_str_t, c_str),
	OFF ("skin_def_t", skin_def_t, framegroups),
	OFF ("skin_def_t", skin_def_t, numframegroups),
	OFF ("surface_def_t", surface_def_t, surfname),
	OFF ("surface_def_t", surface_def_t, skins),
	OFF ("surface_def_t", surface_def_t, numskins),
	OFF ("all_surfaces_def_t", all_surfaces_def_t, surfaces),
	OFF ("all_surfaces_def_t", all_surfaces_def_t, numsurfaces),
};

#undef SZ
#undef OFF

/* Returns the size/offset for a "sizeof.<type>" / "<type>.<member>" key, or
 * (size_t)-1 for an unknown key (the Rust test fails on that). */
size_t ctest_abi_bsp_lookup (const char *key)
{
	size_t i;
	for (i = 0; i < sizeof (ctest_abi_bsp_table) / sizeof (ctest_abi_bsp_table[0]); i++)
		if (!strcmp (ctest_abi_bsp_table[i].name, key))
			return ctest_abi_bsp_table[i].value;
	return (size_t)-1;
}

size_t ctest_abi_alias_lookup (const char *key)
{
	size_t i;
	for (i = 0; i < sizeof (ctest_abi_alias_table) / sizeof (ctest_abi_alias_table[0]); i++)
		if (!strcmp (ctest_abi_alias_table[i].name, key))
			return ctest_abi_alias_table[i].value;
	return (size_t)-1;
}

size_t ctest_abi_mdx_lookup (const char *key)
{
	size_t i;
	for (i = 0; i < sizeof (ctest_abi_mdx_table) / sizeof (ctest_abi_mdx_table[0]); i++)
		if (!strcmp (ctest_abi_mdx_table[i].name, key))
			return ctest_abi_mdx_table[i].value;
	return (size_t)-1;
}

/* Phase 4 sound ABI (q_sound.h): the Rust mixer walks channel_t arrays and
 * sfxcache_t blocks C also touches; see tests/snd_abi.rs. */
#define SZ(tag, type)          {"sizeof." tag, sizeof (type)}
#define OFF(tag, type, member) {tag "." #member, offsetof (type, member)}

static const ctest_abi_entry_t ctest_abi_snd_table[] = {
	SZ ("portable_samplepair_t", portable_samplepair_t),
	OFF ("portable_samplepair_t", portable_samplepair_t, left),
	OFF ("portable_samplepair_t", portable_samplepair_t, right),
	SZ ("sfxcache_t", sfxcache_t),
	OFF ("sfxcache_t", sfxcache_t, length),
	OFF ("sfxcache_t", sfxcache_t, loopstart),
	OFF ("sfxcache_t", sfxcache_t, speed),
	OFF ("sfxcache_t", sfxcache_t, width),
	OFF ("sfxcache_t", sfxcache_t, stereo),
	OFF ("sfxcache_t", sfxcache_t, data),
	SZ ("sfx_t", sfx_t),
	OFF ("sfx_t", sfx_t, name),
	OFF ("sfx_t", sfx_t, cache),
	SZ ("dma_t", dma_t),
	OFF ("dma_t", dma_t, channels),
	OFF ("dma_t", dma_t, samples),
	OFF ("dma_t", dma_t, submission_chunk),
	OFF ("dma_t", dma_t, samplepos),
	OFF ("dma_t", dma_t, samplebits),
	OFF ("dma_t", dma_t, signed8),
	OFF ("dma_t", dma_t, speed),
	OFF ("dma_t", dma_t, buffer),
	SZ ("channel_t", channel_t),
	OFF ("channel_t", channel_t, sfx),
	OFF ("channel_t", channel_t, leftvol),
	OFF ("channel_t", channel_t, rightvol),
	OFF ("channel_t", channel_t, end),
	OFF ("channel_t", channel_t, pos),
	OFF ("channel_t", channel_t, looping),
	OFF ("channel_t", channel_t, entnum),
	OFF ("channel_t", channel_t, entchannel),
	OFF ("channel_t", channel_t, origin),
	OFF ("channel_t", channel_t, dist_mult),
	OFF ("channel_t", channel_t, master_vol),
	SZ ("wavinfo_t", wavinfo_t),
	OFF ("wavinfo_t", wavinfo_t, rate),
	OFF ("wavinfo_t", wavinfo_t, width),
	OFF ("wavinfo_t", wavinfo_t, channels),
	OFF ("wavinfo_t", wavinfo_t, loopstart),
	OFF ("wavinfo_t", wavinfo_t, samples),
	OFF ("wavinfo_t", wavinfo_t, dataofs),
	{"const.MAX_CHANNELS", MAX_CHANNELS},
	{"const.MAX_DYNAMIC_CHANNELS", MAX_DYNAMIC_CHANNELS},
	{"const.MAX_RAW_SAMPLES", MAX_RAW_SAMPLES},
	{"const.MAX_QPATH", MAX_QPATH},
	{"const.NUM_AMBIENTS", NUM_AMBIENTS},
	{"const.MAX_SOUNDS", MAX_SOUNDS},
};

size_t ctest_abi_snd_lookup (const char *key)
{
	size_t i;
	for (i = 0; i < sizeof (ctest_abi_snd_table) / sizeof (ctest_abi_snd_table[0]); i++)
		if (!strcmp (ctest_abi_snd_table[i].name, key))
			return ctest_abi_snd_table[i].value;
	return (size_t)-1;
}

/* Phase 5 net ABI (common.h sizebuf, net.h, net_defs.h): the Rust wire layer
 * shares net_message/qsocket_t storage with C and its driver functions are
 * installed into the C vtables; see tests/net_abi.rs. The net headers are not
 * bindgen-clean, so the Rust side is hand-mirrored (ADR-011). */
static const ctest_abi_entry_t ctest_abi_net_table[] = {
	SZ ("sizebuf_t", sizebuf_t),
	OFF ("sizebuf_t", sizebuf_t, allowoverflow),
	OFF ("sizebuf_t", sizebuf_t, overflowed),
	OFF ("sizebuf_t", sizebuf_t, data),
	OFF ("sizebuf_t", sizebuf_t, maxsize),
	OFF ("sizebuf_t", sizebuf_t, cursize),
	SZ ("qsockaddr", struct qsockaddr),
	OFF ("qsockaddr", struct qsockaddr, qsa_family),
	OFF ("qsockaddr", struct qsockaddr, qsa_data),
	SZ ("qsocket_t", qsocket_t),
	OFF ("qsocket_t", qsocket_t, next),
	OFF ("qsocket_t", qsocket_t, connecttime),
	OFF ("qsocket_t", qsocket_t, lastMessageTime),
	OFF ("qsocket_t", qsocket_t, lastSendTime),
	OFF ("qsocket_t", qsocket_t, isvirtual),
	OFF ("qsocket_t", qsocket_t, disconnected),
	OFF ("qsocket_t", qsocket_t, canSend),
	OFF ("qsocket_t", qsocket_t, sendNext),
	OFF ("qsocket_t", qsocket_t, driver),
	OFF ("qsocket_t", qsocket_t, landriver),
	OFF ("qsocket_t", qsocket_t, socket),
	OFF ("qsocket_t", qsocket_t, driverdata),
	OFF ("qsocket_t", qsocket_t, ackSequence),
	OFF ("qsocket_t", qsocket_t, sendSequence),
	OFF ("qsocket_t", qsocket_t, unreliableSendSequence),
	OFF ("qsocket_t", qsocket_t, sendMessageLength),
	OFF ("qsocket_t", qsocket_t, sendMessage),
	OFF ("qsocket_t", qsocket_t, receiveSequence),
	OFF ("qsocket_t", qsocket_t, unreliableReceiveSequence),
	OFF ("qsocket_t", qsocket_t, receiveMessageLength),
	OFF ("qsocket_t", qsocket_t, receiveMessage),
	OFF ("qsocket_t", qsocket_t, addr),
	OFF ("qsocket_t", qsocket_t, trueaddress),
	OFF ("qsocket_t", qsocket_t, maskedaddress),
	OFF ("qsocket_t", qsocket_t, proquake_angle_hack),
	OFF ("qsocket_t", qsocket_t, max_datagram),
	OFF ("qsocket_t", qsocket_t, pending_max_datagram),
	SZ ("net_landriver_t", net_landriver_t),
	OFF ("net_landriver_t", net_landriver_t, name),
	OFF ("net_landriver_t", net_landriver_t, initialized),
	OFF ("net_landriver_t", net_landriver_t, controlSock),
	OFF ("net_landriver_t", net_landriver_t, Init),
	OFF ("net_landriver_t", net_landriver_t, Shutdown),
	OFF ("net_landriver_t", net_landriver_t, Listen),
	OFF ("net_landriver_t", net_landriver_t, QueryAddresses),
	OFF ("net_landriver_t", net_landriver_t, Open_Socket),
	OFF ("net_landriver_t", net_landriver_t, Close_Socket),
	OFF ("net_landriver_t", net_landriver_t, Connect),
	OFF ("net_landriver_t", net_landriver_t, CheckNewConnections),
	OFF ("net_landriver_t", net_landriver_t, Read),
	OFF ("net_landriver_t", net_landriver_t, Write),
	OFF ("net_landriver_t", net_landriver_t, Broadcast),
	OFF ("net_landriver_t", net_landriver_t, AddrToString),
	OFF ("net_landriver_t", net_landriver_t, StringToAddr),
	OFF ("net_landriver_t", net_landriver_t, GetSocketAddr),
	OFF ("net_landriver_t", net_landriver_t, GetNameFromAddr),
	OFF ("net_landriver_t", net_landriver_t, GetAddrFromName),
	OFF ("net_landriver_t", net_landriver_t, AddrCompare),
	OFF ("net_landriver_t", net_landriver_t, GetSocketPort),
	OFF ("net_landriver_t", net_landriver_t, SetSocketPort),
	OFF ("net_landriver_t", net_landriver_t, listeningSock),
	SZ ("net_driver_t", net_driver_t),
	OFF ("net_driver_t", net_driver_t, name),
	OFF ("net_driver_t", net_driver_t, initialized),
	OFF ("net_driver_t", net_driver_t, Init),
	OFF ("net_driver_t", net_driver_t, Listen),
	OFF ("net_driver_t", net_driver_t, QueryAddresses),
	OFF ("net_driver_t", net_driver_t, SearchForHosts),
	OFF ("net_driver_t", net_driver_t, Connect),
	OFF ("net_driver_t", net_driver_t, CheckNewConnections),
	OFF ("net_driver_t", net_driver_t, QGetAnyMessage),
	OFF ("net_driver_t", net_driver_t, QGetMessage),
	OFF ("net_driver_t", net_driver_t, QSendMessage),
	OFF ("net_driver_t", net_driver_t, SendUnreliableMessage),
	OFF ("net_driver_t", net_driver_t, CanSendMessage),
	OFF ("net_driver_t", net_driver_t, CanSendUnreliableMessage),
	OFF ("net_driver_t", net_driver_t, Close),
	OFF ("net_driver_t", net_driver_t, Shutdown),
	{"const.NET_NAMELEN", NET_NAMELEN},
	{"const.NET_MAXMESSAGE", NET_MAXMESSAGE},
	{"const.MAX_MSGLEN", MAX_MSGLEN},
	{"const.MAX_DATAGRAM", MAX_DATAGRAM},
	{"const.DATAGRAM_MTU", DATAGRAM_MTU},
	{"const.NET_HEADERSIZE", NET_HEADERSIZE},
	{"const.NET_DATAGRAMSIZE", NET_DATAGRAMSIZE},
	{"const.NETFLAG_LENGTH_MASK", NETFLAG_LENGTH_MASK},
	{"const.NETFLAG_DATA", NETFLAG_DATA},
	{"const.NETFLAG_ACK", NETFLAG_ACK},
	{"const.NETFLAG_NAK", NETFLAG_NAK},
	{"const.NETFLAG_EOM", NETFLAG_EOM},
	{"const.NETFLAG_UNRELIABLE", NETFLAG_UNRELIABLE},
	{"const.NETFLAG_CTL", NETFLAG_CTL},
	{"const.NET_LOOPBACKBUFFERS", NET_LOOPBACKBUFFERS},
	{"const.NET_LOOPBACKHEADERSIZE", NET_LOOPBACKHEADERSIZE},
	{"const.NET_PROTOCOL_VERSION", NET_PROTOCOL_VERSION},
	{"const.CCREQ_CONNECT", CCREQ_CONNECT},
	{"const.CCREQ_SERVER_INFO", CCREQ_SERVER_INFO},
	{"const.CCREQ_PLAYER_INFO", CCREQ_PLAYER_INFO},
	{"const.CCREQ_RULE_INFO", CCREQ_RULE_INFO},
	{"const.CCREQ_RCON", CCREQ_RCON},
	{"const.CCREP_ACCEPT", CCREP_ACCEPT},
	{"const.CCREP_REJECT", CCREP_REJECT},
	{"const.CCREP_SERVER_INFO", CCREP_SERVER_INFO},
	{"const.CCREP_PLAYER_INFO", CCREP_PLAYER_INFO},
	{"const.CCREP_RULE_INFO", CCREP_RULE_INFO},
	{"const.CCREP_RCON", CCREP_RCON},
	{"const.HOSTCACHESIZE", HOSTCACHESIZE},
	{"const.SA_FAM_OFFSET", SA_FAM_OFFSET},
	{"sizeof.sys_socket_t", sizeof (sys_socket_t)},
	{"sizeof.qhostaddr_t", sizeof (qhostaddr_t)},
};

size_t ctest_abi_net_lookup (const char *key)
{
	size_t i;
	for (i = 0; i < sizeof (ctest_abi_net_table) / sizeof (ctest_abi_net_table[0]); i++)
		if (!strcmp (ctest_abi_net_table[i].name, key))
			return ctest_abi_net_table[i].value;
	return (size_t)-1;
}
