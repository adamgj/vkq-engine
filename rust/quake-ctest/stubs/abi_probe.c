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
	SZ ("hostcache_t", hostcache_t),
	OFF ("hostcache_t", hostcache_t, name),
	OFF ("hostcache_t", hostcache_t, map),
	OFF ("hostcache_t", hostcache_t, gamedir),
	OFF ("hostcache_t", hostcache_t, cname),
	OFF ("hostcache_t", hostcache_t, users),
	OFF ("hostcache_t", hostcache_t, maxusers),
	OFF ("hostcache_t", hostcache_t, driver),
	OFF ("hostcache_t", hostcache_t, ldriver),
	OFF ("hostcache_t", hostcache_t, addr),
	SZ ("PollProcedure", PollProcedure),
	OFF ("PollProcedure", PollProcedure, next),
	OFF ("PollProcedure", PollProcedure, nextTime),
	OFF ("PollProcedure", PollProcedure, procedure),
	OFF ("PollProcedure", PollProcedure, arg),
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
	/* observable, not an implementation detail: UDP4_GetAddrFromName
	   rejects any host:port whose host part is >= MAXHOSTNAMELEN, so a
	   drift between <sys/param.h> and the Rust constant would change which
	   hostnames are connectable (Phase 5 M10 review fix) */
	{"const.MAXHOSTNAMELEN", MAXHOSTNAMELEN},
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

/* ---------------------------------------------------------------------------
 * Phase 6 (progs VM) ABI probe. Under -Duse_rust_progs the Rust VM reads and
 * writes the C-owned qcvm_t embedded in `sv`/`cl`, strides the C-allocated
 * edict array by qcvm->edict_size, and calls C builtins out of
 * qcvm->builtins[], so a mirror drift is silent memory corruption. edict_t's
 * layout additionally forks on DEBUG/_DEBUG, which is why the table reports
 * `const.ENGINE_DEBUG`: tests/progs_abi.rs asserts the Rust `engine-debug`
 * feature agrees with how this TU was compiled.
 */
#include "progs.h"

static const ctest_abi_entry_t ctest_abi_progs_table[] = {
	SZ ("dstatement_t", dstatement_t),
	OFF ("dstatement_t", dstatement_t, op),
	OFF ("dstatement_t", dstatement_t, a),
	OFF ("dstatement_t", dstatement_t, b),
	OFF ("dstatement_t", dstatement_t, c),

	SZ ("ddef_t", ddef_t),
	OFF ("ddef_t", ddef_t, type),
	OFF ("ddef_t", ddef_t, ofs),
	OFF ("ddef_t", ddef_t, s_name),

	SZ ("dfunction_t", dfunction_t),
	OFF ("dfunction_t", dfunction_t, first_statement),
	OFF ("dfunction_t", dfunction_t, parm_start),
	OFF ("dfunction_t", dfunction_t, locals),
	OFF ("dfunction_t", dfunction_t, profile),
	OFF ("dfunction_t", dfunction_t, s_name),
	OFF ("dfunction_t", dfunction_t, s_file),
	OFF ("dfunction_t", dfunction_t, numparms),
	OFF ("dfunction_t", dfunction_t, parm_size),

	SZ ("dprograms_t", dprograms_t),
	OFF ("dprograms_t", dprograms_t, version),
	OFF ("dprograms_t", dprograms_t, crc),
	OFF ("dprograms_t", dprograms_t, ofs_statements),
	OFF ("dprograms_t", dprograms_t, numstatements),
	OFF ("dprograms_t", dprograms_t, ofs_globaldefs),
	OFF ("dprograms_t", dprograms_t, numglobaldefs),
	OFF ("dprograms_t", dprograms_t, ofs_fielddefs),
	OFF ("dprograms_t", dprograms_t, numfielddefs),
	OFF ("dprograms_t", dprograms_t, ofs_functions),
	OFF ("dprograms_t", dprograms_t, numfunctions),
	OFF ("dprograms_t", dprograms_t, ofs_strings),
	OFF ("dprograms_t", dprograms_t, numstrings),
	OFF ("dprograms_t", dprograms_t, ofs_globals),
	OFF ("dprograms_t", dprograms_t, numglobals),
	OFF ("dprograms_t", dprograms_t, entityfields),

	SZ ("globalvars_t", globalvars_t),
	OFF ("globalvars_t", globalvars_t, pad),
	OFF ("globalvars_t", globalvars_t, self),
	OFF ("globalvars_t", globalvars_t, other),
	OFF ("globalvars_t", globalvars_t, world),
	OFF ("globalvars_t", globalvars_t, time),
	OFF ("globalvars_t", globalvars_t, frametime),
	OFF ("globalvars_t", globalvars_t, force_retouch),
	OFF ("globalvars_t", globalvars_t, mapname),
	OFF ("globalvars_t", globalvars_t, deathmatch),
	OFF ("globalvars_t", globalvars_t, coop),
	OFF ("globalvars_t", globalvars_t, teamplay),
	OFF ("globalvars_t", globalvars_t, serverflags),
	OFF ("globalvars_t", globalvars_t, total_secrets),
	OFF ("globalvars_t", globalvars_t, total_monsters),
	OFF ("globalvars_t", globalvars_t, found_secrets),
	OFF ("globalvars_t", globalvars_t, killed_monsters),
	OFF ("globalvars_t", globalvars_t, parm1),
	OFF ("globalvars_t", globalvars_t, parm2),
	OFF ("globalvars_t", globalvars_t, parm3),
	OFF ("globalvars_t", globalvars_t, parm4),
	OFF ("globalvars_t", globalvars_t, parm5),
	OFF ("globalvars_t", globalvars_t, parm6),
	OFF ("globalvars_t", globalvars_t, parm7),
	OFF ("globalvars_t", globalvars_t, parm8),
	OFF ("globalvars_t", globalvars_t, parm9),
	OFF ("globalvars_t", globalvars_t, parm10),
	OFF ("globalvars_t", globalvars_t, parm11),
	OFF ("globalvars_t", globalvars_t, parm12),
	OFF ("globalvars_t", globalvars_t, parm13),
	OFF ("globalvars_t", globalvars_t, parm14),
	OFF ("globalvars_t", globalvars_t, parm15),
	OFF ("globalvars_t", globalvars_t, parm16),
	OFF ("globalvars_t", globalvars_t, v_forward),
	OFF ("globalvars_t", globalvars_t, v_up),
	OFF ("globalvars_t", globalvars_t, v_right),
	OFF ("globalvars_t", globalvars_t, trace_allsolid),
	OFF ("globalvars_t", globalvars_t, trace_startsolid),
	OFF ("globalvars_t", globalvars_t, trace_fraction),
	OFF ("globalvars_t", globalvars_t, trace_endpos),
	OFF ("globalvars_t", globalvars_t, trace_plane_normal),
	OFF ("globalvars_t", globalvars_t, trace_plane_dist),
	OFF ("globalvars_t", globalvars_t, trace_ent),
	OFF ("globalvars_t", globalvars_t, trace_inopen),
	OFF ("globalvars_t", globalvars_t, trace_inwater),
	OFF ("globalvars_t", globalvars_t, msg_entity),
	OFF ("globalvars_t", globalvars_t, main),
	OFF ("globalvars_t", globalvars_t, StartFrame),
	OFF ("globalvars_t", globalvars_t, PlayerPreThink),
	OFF ("globalvars_t", globalvars_t, PlayerPostThink),
	OFF ("globalvars_t", globalvars_t, ClientKill),
	OFF ("globalvars_t", globalvars_t, ClientConnect),
	OFF ("globalvars_t", globalvars_t, PutClientInServer),
	OFF ("globalvars_t", globalvars_t, ClientDisconnect),
	OFF ("globalvars_t", globalvars_t, SetNewParms),
	OFF ("globalvars_t", globalvars_t, SetChangeParms),

	SZ ("entvars_t", entvars_t),
	OFF ("entvars_t", entvars_t, modelindex),
	OFF ("entvars_t", entvars_t, absmin),
	OFF ("entvars_t", entvars_t, absmax),
	OFF ("entvars_t", entvars_t, ltime),
	OFF ("entvars_t", entvars_t, movetype),
	OFF ("entvars_t", entvars_t, solid),
	OFF ("entvars_t", entvars_t, origin),
	OFF ("entvars_t", entvars_t, oldorigin),
	OFF ("entvars_t", entvars_t, velocity),
	OFF ("entvars_t", entvars_t, angles),
	OFF ("entvars_t", entvars_t, avelocity),
	OFF ("entvars_t", entvars_t, punchangle),
	OFF ("entvars_t", entvars_t, classname),
	OFF ("entvars_t", entvars_t, model),
	OFF ("entvars_t", entvars_t, frame),
	OFF ("entvars_t", entvars_t, skin),
	OFF ("entvars_t", entvars_t, effects),
	OFF ("entvars_t", entvars_t, mins),
	OFF ("entvars_t", entvars_t, maxs),
	OFF ("entvars_t", entvars_t, size),
	OFF ("entvars_t", entvars_t, touch),
	OFF ("entvars_t", entvars_t, use),
	OFF ("entvars_t", entvars_t, think),
	OFF ("entvars_t", entvars_t, blocked),
	OFF ("entvars_t", entvars_t, nextthink),
	OFF ("entvars_t", entvars_t, groundentity),
	OFF ("entvars_t", entvars_t, health),
	OFF ("entvars_t", entvars_t, frags),
	OFF ("entvars_t", entvars_t, weapon),
	OFF ("entvars_t", entvars_t, weaponmodel),
	OFF ("entvars_t", entvars_t, weaponframe),
	OFF ("entvars_t", entvars_t, currentammo),
	OFF ("entvars_t", entvars_t, ammo_shells),
	OFF ("entvars_t", entvars_t, ammo_nails),
	OFF ("entvars_t", entvars_t, ammo_rockets),
	OFF ("entvars_t", entvars_t, ammo_cells),
	OFF ("entvars_t", entvars_t, items),
	OFF ("entvars_t", entvars_t, takedamage),
	OFF ("entvars_t", entvars_t, chain),
	OFF ("entvars_t", entvars_t, deadflag),
	OFF ("entvars_t", entvars_t, view_ofs),
	OFF ("entvars_t", entvars_t, button0),
	OFF ("entvars_t", entvars_t, button1),
	OFF ("entvars_t", entvars_t, button2),
	OFF ("entvars_t", entvars_t, impulse),
	OFF ("entvars_t", entvars_t, fixangle),
	OFF ("entvars_t", entvars_t, v_angle),
	OFF ("entvars_t", entvars_t, idealpitch),
	OFF ("entvars_t", entvars_t, netname),
	OFF ("entvars_t", entvars_t, enemy),
	OFF ("entvars_t", entvars_t, flags),
	OFF ("entvars_t", entvars_t, colormap),
	OFF ("entvars_t", entvars_t, team),
	OFF ("entvars_t", entvars_t, max_health),
	OFF ("entvars_t", entvars_t, teleport_time),
	OFF ("entvars_t", entvars_t, armortype),
	OFF ("entvars_t", entvars_t, armorvalue),
	OFF ("entvars_t", entvars_t, waterlevel),
	OFF ("entvars_t", entvars_t, watertype),
	OFF ("entvars_t", entvars_t, ideal_yaw),
	OFF ("entvars_t", entvars_t, yaw_speed),
	OFF ("entvars_t", entvars_t, aiment),
	OFF ("entvars_t", entvars_t, goalentity),
	OFF ("entvars_t", entvars_t, spawnflags),
	OFF ("entvars_t", entvars_t, target),
	OFF ("entvars_t", entvars_t, targetname),
	OFF ("entvars_t", entvars_t, dmg_take),
	OFF ("entvars_t", entvars_t, dmg_save),
	OFF ("entvars_t", entvars_t, dmg_inflictor),
	OFF ("entvars_t", entvars_t, owner),
	OFF ("entvars_t", entvars_t, movedir),
	OFF ("entvars_t", entvars_t, message),
	OFF ("entvars_t", entvars_t, sounds),
	OFF ("entvars_t", entvars_t, noise),
	OFF ("entvars_t", entvars_t, noise1),
	OFF ("entvars_t", entvars_t, noise2),
	OFF ("entvars_t", entvars_t, noise3),

	SZ ("entity_state_t", entity_state_t),
	OFF ("entity_state_t", entity_state_t, origin),
	OFF ("entity_state_t", entity_state_t, effects),
	OFF ("entity_state_t", entity_state_t, velocity),
	OFF ("entity_state_t", entity_state_t, colormod),
	OFF ("entity_state_t", entity_state_t, solidsize),

	SZ ("link_t", link_t),
	OFF ("link_t", link_t, prev),
	OFF ("link_t", link_t, next),

	SZ ("prstack_t", prstack_t),
	OFF ("prstack_t", prstack_t, s),
	OFF ("prstack_t", prstack_t, f),

	SZ ("freelist_t", freelist_t),
	OFF ("freelist_t", freelist_t, size),
	OFF ("freelist_t", freelist_t, head_index),
	OFF ("freelist_t", freelist_t, circular_buffer),

	SZ ("areanode_t", areanode_t),
	OFF ("areanode_t", areanode_t, axis),
	OFF ("areanode_t", areanode_t, dist),
	OFF ("areanode_t", areanode_t, children),
	OFF ("areanode_t", areanode_t, trigger_edicts),
	OFF ("areanode_t", areanode_t, solid_edicts),

	SZ ("edict_t", edict_t),
#if defined(DEBUG) || defined(_DEBUG)
	OFF ("edict_t", edict_t, edict_ptr),
	OFF ("edict_t", edict_t, qcvm_owner),
	OFF ("edict_t", edict_t, edict_num),
#endif
	OFF ("edict_t", edict_t, area),
	OFF ("edict_t", edict_t, num_leafs),
	OFF ("edict_t", edict_t, leafnums),
	OFF ("edict_t", edict_t, baseline),
	OFF ("edict_t", edict_t, alpha),
	OFF ("edict_t", edict_t, sendinterval),
	OFF ("edict_t", edict_t, sendinterval_default),
	OFF ("edict_t", edict_t, oldframe),
	OFF ("edict_t", edict_t, oldthinktime),
	OFF ("edict_t", edict_t, predthinkpos),
	OFF ("edict_t", edict_t, lastthink),
	OFF ("edict_t", edict_t, freetime),
	OFF ("edict_t", edict_t, free),
	OFF ("edict_t", edict_t, v),

	SZ ("pr_extglobals_s", struct pr_extglobals_s),
	OFF ("pr_extglobals_s", struct pr_extglobals_s, time),
	OFF ("pr_extglobals_s", struct pr_extglobals_s, physics_mode),
	OFF ("pr_extglobals_s", struct pr_extglobals_s, player_localentnum),

	SZ ("pr_extfuncs_s", struct pr_extfuncs_s),
	OFF ("pr_extfuncs_s", struct pr_extfuncs_s, GameCommand),
	OFF ("pr_extfuncs_s", struct pr_extfuncs_s, CSQC_DrawHud),
	OFF ("pr_extfuncs_s", struct pr_extfuncs_s, CSQC_Parse_Print),

	SZ ("pr_extfields_s", struct pr_extfields_s),
	OFF ("pr_extfields_s", struct pr_extfields_s, alpha),
	OFF ("pr_extfields_s", struct pr_extfields_s, customphysics),
	OFF ("pr_extfields_s", struct pr_extfields_s, SendFlags),

	SZ ("qcvm_t", qcvm_t),
	OFF ("qcvm_t", qcvm_t, progs),
	OFF ("qcvm_t", qcvm_t, functions),
	OFF ("qcvm_t", qcvm_t, function_map),
	OFF ("qcvm_t", qcvm_t, statements),
	OFF ("qcvm_t", qcvm_t, globals),
	OFF ("qcvm_t", qcvm_t, fielddefs),
	OFF ("qcvm_t", qcvm_t, fielddefs_map),
	OFF ("qcvm_t", qcvm_t, edict_size),
	OFF ("qcvm_t", qcvm_t, builtins),
	OFF ("qcvm_t", qcvm_t, numbuiltins),
	OFF ("qcvm_t", qcvm_t, argc),
	OFF ("qcvm_t", qcvm_t, trace),
	OFF ("qcvm_t", qcvm_t, xfunction),
	OFF ("qcvm_t", qcvm_t, xstatement),
	OFF ("qcvm_t", qcvm_t, progscrc),
	OFF ("qcvm_t", qcvm_t, progshash),
	OFF ("qcvm_t", qcvm_t, progssize),
	OFF ("qcvm_t", qcvm_t, extglobals),
	OFF ("qcvm_t", qcvm_t, extfuncs),
	OFF ("qcvm_t", qcvm_t, extfields),
	OFF ("qcvm_t", qcvm_t, strings),
	OFF ("qcvm_t", qcvm_t, stringssize),
	OFF ("qcvm_t", qcvm_t, knownstrings),
	OFF ("qcvm_t", qcvm_t, knownstringsowned),
	OFF ("qcvm_t", qcvm_t, maxknownstrings),
	OFF ("qcvm_t", qcvm_t, numknownstrings),
	OFF ("qcvm_t", qcvm_t, progsstrings),
	OFF ("qcvm_t", qcvm_t, freeknownstrings),
	OFF ("qcvm_t", qcvm_t, globaldefs),
	OFF ("qcvm_t", qcvm_t, globaldefs_map),
	OFF ("qcvm_t", qcvm_t, knownzone),
	OFF ("qcvm_t", qcvm_t, knownzonesize),
	OFF ("qcvm_t", qcvm_t, stack),
	OFF ("qcvm_t", qcvm_t, depth),
	OFF ("qcvm_t", qcvm_t, localstack),
	OFF ("qcvm_t", qcvm_t, localstack_used),
	OFF ("qcvm_t", qcvm_t, time),
	OFF ("qcvm_t", qcvm_t, num_edicts),
	OFF ("qcvm_t", qcvm_t, reserved_edicts),
	OFF ("qcvm_t", qcvm_t, max_edicts),
	OFF ("qcvm_t", qcvm_t, edicts),
	OFF ("qcvm_t", qcvm_t, free_list),
	OFF ("qcvm_t", qcvm_t, worldmodel),
	OFF ("qcvm_t", qcvm_t, GetModel),
	OFF ("qcvm_t", qcvm_t, areanodes),
	OFF ("qcvm_t", qcvm_t, numareanodes),

	{"sizeof.builtin_t", sizeof (builtin_t)},
	{"const.PROG_VERSION", PROG_VERSION},
	{"const.PROGHEADER_CRC", PROGHEADER_CRC},
	{"const.MAX_PARMS", MAX_PARMS},
	{"const.DEF_SAVEGLOBAL", DEF_SAVEGLOBAL},
	{"const.OFS_RETURN", OFS_RETURN},
	{"const.OFS_PARM0", OFS_PARM0},
	{"const.OFS_PARM7", OFS_PARM7},
	{"const.RESERVED_OFS", RESERVED_OFS},
	{"const.MAX_ENT_LEAFS", MAX_ENT_LEAFS},
	{"const.MAX_EDICTS", MAX_EDICTS},
	{"const.MAX_AREA_DEPTH", MAX_AREA_DEPTH},
	{"const.AREA_NODES", AREA_NODES},
	{"const.MAX_STACK_DEPTH", MAX_STACK_DEPTH},
	{"const.LOCALSTACK_SIZE", LOCALSTACK_SIZE},
	{"const.STRINGTEMP_BUFFERS", STRINGTEMP_BUFFERS},
	{"const.STRINGTEMP_LENGTH", STRINGTEMP_LENGTH},
	{"const.MIN_EDICTS", MIN_EDICTS},
	{"const.MIN_EDICT_AGE_FOR_REUSE", (size_t)MIN_EDICT_AGE_FOR_REUSE},
	{"const.MAX_EDICT_FREETIME_ALWAYS_REUSE", (size_t)MAX_EDICT_FREETIME_ALWAYS_REUSE},
#if defined(DEBUG) || defined(_DEBUG)
	{"const.ENGINE_DEBUG", 1},
#else
	{"const.ENGINE_DEBUG", 0},
#endif
};

size_t ctest_abi_progs_lookup (const char *key)
{
	size_t i;
	for (i = 0; i < sizeof (ctest_abi_progs_table) / sizeof (ctest_abi_progs_table[0]); i++)
		if (!strcmp (ctest_abi_progs_table[i].name, key))
			return ctest_abi_progs_table[i].value;
	return (size_t)-1;
}

/* ---------------------------------------------------------------------------
 * Phase 7 (host/server/client) ABI probe: `sv`/`svs` (server.h) and
 * `cl`/`cls` (client.h) are the process-global server and client state.
 * Under the Phase 7 host/server/client port Rust reads and writes these
 * C-owned instances directly (dual-view closes per `sv`/`svs` at M6 and
 * `cl`/`cls` at M7, docs/ai/plans/rust-conversion-phase-7.md), so a mirror
 * drift here is silent memory corruption, not a link error.
 *
 * Neither header is a bindgen-clean root: both pull qcvm_t (progs.h,
 * already included above), and client.h's client_state_t additionally
 * embeds entity_t (render.h) by value via `viewent`. render.h
 * unconditionally #includes "tasks.h" (pulls q_stdinc.h -> SDL.h, not
 * stubbed anywhere in c_ref_prelude.h) and "q_render_types.h" (the real
 * Vulkan SDK header; c_ref_prelude.h already stands that one in via
 * __Q_RENDER_TYPES_H's handle typedefs, for the Phase 3 gl_model.h tail).
 * Since quake_types::host::EntityOpaque treats entity_t entirely as an
 * opaque, size/align-verified blob -- Phase 8's renderer owns its real
 * fields, not Phase 7 -- only entity_t's sizeof/alignof are needed here,
 * not per-field offsets. The bounded, field-verbatim shadow of
 * entity_t/lightcache_t/entlerp_t/efrag_t transcribed from render.h used to
 * live right here; Phase 7 M3 moved it into c_ref_prelude.h, because
 * world.c's World_ClipToNetwork walks cl.entities as entity_t and needs the
 * same layout in its own TU. Two copies would have been free to drift, so
 * there is now one, force-included into every oracle TU including this one.
 * It reuses the Vulkan handle stand-ins above for entity_t's only
 * Vulkan-typed member (entity_blas_t, reached solely through a pointer, so
 * its own body is never needed).
 *
 * KNOWN GAP -- that shadow is the one place in this probe's input that is
 * NOT header-derived, and it is load-bearing: client.h has no #includes of its
 * own, so the client_state_t laid out in this TU takes every offset at or
 * after `viewent` from the transcription below rather than from render.h.
 * A field added to the real entity_t therefore still compiles, still passes
 * host_abi.rs, and silently invalidates those offsets. Nothing detects that
 * automatically; render.h carries the codebase's usual
 * "!!! if this is changed !!!" markers on entity_s/entlerp_s/lightcache_s
 * pointing here, which is a convention, not a gate. The constants above are
 * gated properly, by a _Static_assert in Quake/harness.c. Same class,
 * smaller blast radius: entity_blas_t is transcribed only as far as
 * "reached solely through a pointer", which holds today but is unenforced.
 *
 * What the transcription does buy is that the compiler computes the real
 * sizeof/alignof from it rather than from a hardcoded number -- and every
 * field this probe's Rust consumer (host_abi.rs) can
 * actually reach through a named quake_types::host field gets its own
 * offsetof entry below, exactly like every other phase in this file.
 *
 * server.h/client.h declare `extern server_t sv;`, `extern server_static_t
 * svs;`, `extern client_state_t cl;` and `extern client_static_t cls;`, but
 * c_ref_prelude.h already declared small stand-in stubs under those same
 * four names for the Phase 4/5/6 oracles (snd_mix.c, pr_exec.c,
 * net_loop.c). Redeclaring the same identifiers with the real, larger
 * types would be a C "conflicting types" error, so they are renamed out of
 * the way for just the two #includes below, using the same #define/#undef
 * idiom c_ref_prelude.h itself uses throughout (e.g. `#define X c_ref_X`).
 * This does not touch unrelated identifiers that merely share a prefix
 * (`cl_lightstyle`, `host_client`, ...): C macro substitution is
 * whole-token only.
 *
 * quakedef.h constants server.h/client.h expect from their normal
 * #include "quakedef.h" chain, which c_ref_prelude.h pre-empts before this
 * TU ever reaches them (MAX_SOUNDS, MAX_LIGHTSTYLES, MAX_DATAGRAM,
 * MAX_MSGLEN and MAX_EDICTS are already defined above by the earlier
 * phases' slices; SERVER_INFO_STRING_SIZE, CLIENT_USER_INFO_STRING_SIZE,
 * NUM_PING_TIMES, NUM_TOTAL_SPAWN_PARMS, NUM_CSHIFTS, MAX_MAPSTRING,
 * MAX_DEMOS and MAX_DEMONAME are `#define`d by server.h/client.h
 * themselves): */
#define MAX_MODELS		  8192
#define MAX_PARTICLETYPES 2048
#define MAX_STYLESTRING	  64
#define MAX_CL_STATS	  256
#define MAX_SCOREBOARDNAME 32
#define VID_CBITS		  6
#define VID_GRADES		  (1 << VID_CBITS)

/* entity_t / lightcache_t / entlerp_t / efrag_t come from c_ref_prelude.h
 * (see the note above); nothing is transcribed locally any more. */

/* c_ref_prelude.h already declares its own `server_state_t`/`cactive_t`
 * (with the same enumerator names) for the Phase 6 pr_exec.c oracle's
 * stub `sv`/`cl`. Including the real server.h/client.h below would
 * redefine those same enumerators, so they're renamed out of the way for
 * just these two #includes, same idiom as the sv/svs/cl/cls dodge. */
#define sv			  ctest_host_probe_unused_sv
#define svs			  ctest_host_probe_unused_svs
#define cl			  ctest_host_probe_unused_cl
#define cls			  ctest_host_probe_unused_cls
#define server_state_t	  ctest_host_real_server_state_t
#define ss_loading		  ctest_host_real_ss_loading
#define ss_active		  ctest_host_real_ss_active
#define cactive_t		  ctest_host_real_cactive_t
#define ca_dedicated	  ctest_host_real_ca_dedicated
#define ca_disconnected	  ctest_host_real_ca_disconnected
#define ca_connected	  ctest_host_real_ca_connected
/* Phase 7 M2: c_ref_prelude.h now also declares its own small `client_t`
 * (struct client_s), for cvar.c's CVAR_SERVERINFO replication block, under
 * this same real name. Same dodge as sv/svs/cl/cls above, but NOT undone
 * below like the others: the OFF()/SZ() table further down still spells the
 * type as bare `client_t` (only the "client_t" string label needs to stay
 * literal, which it does -- macros don't expand inside string literals), so
 * the rename has to stay live for the rest of this translation unit for
 * those to keep resolving to server.h's real, full struct rather than
 * falling back to the small mock once this block ends. */
#define client_t	  ctest_host_real_client_t
#define client_s	  ctest_host_real_client_s
/* server.h also declares `extern client_t *host_client;` under the real
 * type -- c_ref_prelude.h's own extern of the same name (mock type) is
 * already in scope from the force-included prelude, so this one needs the
 * same treatment as sv/svs/cl/cls: renamed away for the include, then
 * discarded (nothing below needs the variable, only the struct layout). */
#define host_client	  ctest_host_probe_unused_host_client
/* Phase 7 M3: c_ref_prelude.h supplies world.c's slice of server.h's
 * movetype/solid/flags enumerators as object-like macros (quakedef.h is
 * neutered, so world.c never sees the real enums). Here -- the one TU that
 * does include the real server.h -- those macros would rewrite the enum
 * bodies themselves (`SOLID_NOT = 0,` -> `0 = 0,`). Drop them for good:
 * nothing below this point needs the constants, only the struct layouts. */
#undef MOVETYPE_PUSH
#undef SOLID_NOT
#undef SOLID_TRIGGER
#undef SOLID_BSP
#undef FL_MONSTER
#undef FL_ITEM
#include "server.h"
#include "client.h"
#undef sv
#undef svs
#undef cl
#undef cls
#undef host_client
#undef server_state_t
#undef ss_loading
#undef ss_active
#undef cactive_t
#undef ca_dedicated
#undef ca_disconnected
#undef ca_connected

static const ctest_abi_entry_t ctest_abi_host_table[] = {
	SZ ("server_static_t", server_static_t),
	OFF ("server_static_t", server_static_t, maxclients),
	OFF ("server_static_t", server_static_t, maxclientslimit),
	OFF ("server_static_t", server_static_t, clients),
	OFF ("server_static_t", server_static_t, serverflags),
	OFF ("server_static_t", server_static_t, changelevel_issued),
	OFF ("server_static_t", server_static_t, serverinfo),

	{"const.ss_loading", ctest_host_real_ss_loading},
	{"const.ss_active", ctest_host_real_ss_active},

	SZ ("ambientsound_t", struct ambientsound_s),
	OFF ("ambientsound_t", struct ambientsound_s, origin),
	OFF ("ambientsound_t", struct ambientsound_s, soundindex),
	OFF ("ambientsound_t", struct ambientsound_s, volume),
	OFF ("ambientsound_t", struct ambientsound_s, attenuation),

	SZ ("svcustomstat_t", struct svcustomstat_s),
	OFF ("svcustomstat_t", struct svcustomstat_s, idx),
	OFF ("svcustomstat_t", struct svcustomstat_s, type),
	OFF ("svcustomstat_t", struct svcustomstat_s, fld),
	OFF ("svcustomstat_t", struct svcustomstat_s, ptr),

	SZ ("server_t", server_t),
	OFF ("server_t", server_t, active),
	OFF ("server_t", server_t, paused),
	OFF ("server_t", server_t, loadgame),
	OFF ("server_t", server_t, nomonsters),
	OFF ("server_t", server_t, lastsave),
	OFF ("server_t", server_t, lastcheck),
	OFF ("server_t", server_t, lastchecktime),
	OFF ("server_t", server_t, qcvm),
	OFF ("server_t", server_t, name),
	OFF ("server_t", server_t, modelname),
	OFF ("server_t", server_t, model_precache),
	OFF ("server_t", server_t, models),
	OFF ("server_t", server_t, sound_precache),
	OFF ("server_t", server_t, lightstyles),
	OFF ("server_t", server_t, state),
	OFF ("server_t", server_t, datagram),
	OFF ("server_t", server_t, datagram_buf),
	OFF ("server_t", server_t, reliable_datagram),
	OFF ("server_t", server_t, reliable_datagram_buf),
	OFF ("server_t", server_t, signon),
	OFF ("server_t", server_t, signon_buf),
	OFF ("server_t", server_t, protocol),
	OFF ("server_t", server_t, protocolflags),
	OFF ("server_t", server_t, multicast),
	OFF ("server_t", server_t, multicast_buf),
	OFF ("server_t", server_t, particle_precache),
	OFF ("server_t", server_t, static_entities),
	OFF ("server_t", server_t, num_statics),
	OFF ("server_t", server_t, max_statics),
	OFF ("server_t", server_t, ambientsounds),
	OFF ("server_t", server_t, num_ambients),
	OFF ("server_t", server_t, max_ambients),
	OFF ("server_t", server_t, customstats),
	OFF ("server_t", server_t, numcustomstats),
	OFF ("server_t", server_t, effectsmask),

	SZ ("usercmd_t", usercmd_t),
	OFF ("usercmd_t", usercmd_t, servertime),
	OFF ("usercmd_t", usercmd_t, seconds),
	OFF ("usercmd_t", usercmd_t, viewangles),
	OFF ("usercmd_t", usercmd_t, forwardmove),
	OFF ("usercmd_t", usercmd_t, sidemove),
	OFF ("usercmd_t", usercmd_t, upmove),
	OFF ("usercmd_t", usercmd_t, forwardmove_accumulator),
	OFF ("usercmd_t", usercmd_t, sidemove_accumulator),
	OFF ("usercmd_t", usercmd_t, upmove_accumulator),
	OFF ("usercmd_t", usercmd_t, buttons),
	OFF ("usercmd_t", usercmd_t, impulse),
	OFF ("usercmd_t", usercmd_t, sequence),
	OFF ("usercmd_t", usercmd_t, weapon),

	{"const.PRESPAWN_DONE", PRESPAWN_DONE},
	{"const.PRESPAWN_FLUSH", PRESPAWN_FLUSH},
	{"const.PRESPAWN_MODELS", PRESPAWN_MODELS},
	{"const.PRESPAWN_SOUNDS", PRESPAWN_SOUNDS},
	{"const.PRESPAWN_PARTICLES", PRESPAWN_PARTICLES},
	{"const.PRESPAWN_BASELINES", PRESPAWN_BASELINES},
	{"const.PRESPAWN_STATICS", PRESPAWN_STATICS},
	{"const.PRESPAWN_AMBIENTS", PRESPAWN_AMBIENTS},
	{"const.PRESPAWN_SIGNONMSG", PRESPAWN_SIGNONMSG},

	SZ ("entity_num_state_t", struct entity_num_state_s),
	OFF ("entity_num_state_t", struct entity_num_state_s, num),
	OFF ("entity_num_state_t", struct entity_num_state_s, state),

	/* `struct deltaframe_s.ents` points at a truly anonymous C struct (no
	 * tag), so it has no name offsetof/sizeof can take -- this is the
	 * standard sizeof-of-unevaluated-expression idiom (sizeof never
	 * evaluates its operand) to still get a real, header-derived size for
	 * quake_types::host::DeltaFrameEnt. */
	{"sizeof.deltaframe_ents_t", sizeof (*(((client_t *)0)->frames->ents))},

	SZ ("deltaframe_t", struct deltaframe_s),
	OFF ("deltaframe_t", struct deltaframe_s, sequence),
	OFF ("deltaframe_t", struct deltaframe_s, timestamp),
	OFF ("deltaframe_t", struct deltaframe_s, resendstatsnum),
	OFF ("deltaframe_t", struct deltaframe_s, resendstatsstr),
	OFF ("deltaframe_t", struct deltaframe_s, ents),
	OFF ("deltaframe_t", struct deltaframe_s, numents),
	OFF ("deltaframe_t", struct deltaframe_s, maxents),

	SZ ("client_t", client_t),
	OFF ("client_t", client_t, active),
	OFF ("client_t", client_t, spawned),
	OFF ("client_t", client_t, dropasap),
	OFF ("client_t", client_t, sendsignon),
	OFF ("client_t", client_t, signonidx),
	OFF ("client_t", client_t, signon_sounds),
	OFF ("client_t", client_t, signon_models),
	OFF ("client_t", client_t, last_message),
	OFF ("client_t", client_t, netconnection),
	OFF ("client_t", client_t, cmd),
	OFF ("client_t", client_t, wishdir),
	OFF ("client_t", client_t, message),
	OFF ("client_t", client_t, msgbuf),
	OFF ("client_t", client_t, edict),
	OFF ("client_t", client_t, name),
	OFF ("client_t", client_t, colors),
	OFF ("client_t", client_t, ping_times),
	OFF ("client_t", client_t, num_pings),
	OFF ("client_t", client_t, spawn_parms),
	OFF ("client_t", client_t, old_frags),
	OFF ("client_t", client_t, datagram),
	OFF ("client_t", client_t, datagram_buf),
	OFF ("client_t", client_t, limit_entities),
	OFF ("client_t", client_t, limit_unreliable),
	OFF ("client_t", client_t, limit_reliable),
	OFF ("client_t", client_t, limit_models),
	OFF ("client_t", client_t, limit_sounds),
	OFF ("client_t", client_t, pextknown),
	OFF ("client_t", client_t, protocol_pext1),
	OFF ("client_t", client_t, protocol_pext2),
	OFF ("client_t", client_t, resendstatsnum),
	OFF ("client_t", client_t, resendstatsstr),
	OFF ("client_t", client_t, oldstats_i),
	OFF ("client_t", client_t, oldstats_f),
	OFF ("client_t", client_t, oldstats_s),
	OFF ("client_t", client_t, previousentities),
	OFF ("client_t", client_t, numpreviousentities),
	OFF ("client_t", client_t, maxpreviousentities),
	OFF ("client_t", client_t, snapshotresume),
	OFF ("client_t", client_t, pendingentities_bits),
	OFF ("client_t", client_t, numpendingentities),
	OFF ("client_t", client_t, frames),
	OFF ("client_t", client_t, numframes),
	OFF ("client_t", client_t, lastacksequence),
	OFF ("client_t", client_t, lastmovemessage),
	OFF ("client_t", client_t, lastmovetime),
	OFF ("client_t", client_t, knowntoqc),
	OFF ("client_t", client_t, userinfo),

	{"const.CA_DEDICATED", ctest_host_real_ca_dedicated},
	{"const.CA_DISCONNECTED", ctest_host_real_ca_disconnected},
	{"const.CA_CONNECTED", ctest_host_real_ca_connected},

	SZ ("cshift_t", cshift_t),
	OFF ("cshift_t", cshift_t, destcolor),
	OFF ("cshift_t", cshift_t, percent),

	SZ ("scoreboard_t", scoreboard_t),
	OFF ("scoreboard_t", scoreboard_t, name),
	OFF ("scoreboard_t", scoreboard_t, entertime),
	OFF ("scoreboard_t", scoreboard_t, frags),
	OFF ("scoreboard_t", scoreboard_t, colors),
	OFF ("scoreboard_t", scoreboard_t, ping),
	OFF ("scoreboard_t", scoreboard_t, translations),
	OFF ("scoreboard_t", scoreboard_t, userinfo),

	SZ ("client_static_t", client_static_t),
	OFF ("client_static_t", client_static_t, state),
	OFF ("client_static_t", client_static_t, spawnparms),
	OFF ("client_static_t", client_static_t, demonum),
	OFF ("client_static_t", client_static_t, demos),
	OFF ("client_static_t", client_static_t, demorecording),
	OFF ("client_static_t", client_static_t, demoplayback),
	OFF ("client_static_t", client_static_t, demopaused),
	OFF ("client_static_t", client_static_t, demoseeking),
	OFF ("client_static_t", client_static_t, seektime),
	OFF ("client_static_t", client_static_t, demospeed),
	OFF ("client_static_t", client_static_t, demo_prespawn_end),
	OFF ("client_static_t", client_static_t, timedemo),
	OFF ("client_static_t", client_static_t, forcetrack),
	OFF ("client_static_t", client_static_t, demofile),
	OFF ("client_static_t", client_static_t, td_lastframe),
	OFF ("client_static_t", client_static_t, td_startframe),
	OFF ("client_static_t", client_static_t, td_starttime),
	OFF ("client_static_t", client_static_t, signon),
	OFF ("client_static_t", client_static_t, netcon),
	OFF ("client_static_t", client_static_t, message),
	OFF ("client_static_t", client_static_t, userinfo),

	{"sizeof.entity_t", sizeof (entity_t)},
	{"alignof.entity_t", _Alignof (entity_t)},

	SZ ("efrag_t", struct efrag_s),
	OFF ("efrag_t", struct efrag_s, leafnext),
	OFF ("efrag_t", struct efrag_s, entity),

	/* client_state_t::particle_precache/local_particle_precache elements
	 * are also a truly anonymous C struct -- same idiom as
	 * deltaframe_ents_t above. */
	{"sizeof.particle_precache_entry_t", sizeof (((client_state_t *)0)->particle_precache[0])},

	SZ ("client_state_t", client_state_t),
	OFF ("client_state_t", client_state_t, movemessages),
	OFF ("client_state_t", client_state_t, ackedmovemessages),
	OFF ("client_state_t", client_state_t, movecmds),
	OFF ("client_state_t", client_state_t, pendingcmd),
	OFF ("client_state_t", client_state_t, stats),
	OFF ("client_state_t", client_state_t, statsf),
	OFF ("client_state_t", client_state_t, statss),
	OFF ("client_state_t", client_state_t, items),
	OFF ("client_state_t", client_state_t, item_gettime),
	OFF ("client_state_t", client_state_t, faceanimtime),
	OFF ("client_state_t", client_state_t, v_dmg_time),
	OFF ("client_state_t", client_state_t, v_dmg_roll),
	OFF ("client_state_t", client_state_t, v_dmg_pitch),
	OFF ("client_state_t", client_state_t, cshift_empty),
	OFF ("client_state_t", client_state_t, cshifts),
	OFF ("client_state_t", client_state_t, prev_cshifts),
	OFF ("client_state_t", client_state_t, mviewangles),
	OFF ("client_state_t", client_state_t, viewangles),
	OFF ("client_state_t", client_state_t, mvelocity),
	OFF ("client_state_t", client_state_t, velocity),
	OFF ("client_state_t", client_state_t, punchangle),
	OFF ("client_state_t", client_state_t, idealpitch),
	OFF ("client_state_t", client_state_t, pitchvel),
	OFF ("client_state_t", client_state_t, nodrift),
	OFF ("client_state_t", client_state_t, driftmove),
	OFF ("client_state_t", client_state_t, laststop),
	OFF ("client_state_t", client_state_t, viewheight),
	OFF ("client_state_t", client_state_t, crouch),
	OFF ("client_state_t", client_state_t, paused),
	OFF ("client_state_t", client_state_t, onground),
	OFF ("client_state_t", client_state_t, inwater),
	OFF ("client_state_t", client_state_t, fixangle_time),
	OFF ("client_state_t", client_state_t, intermission),
	OFF ("client_state_t", client_state_t, completed_time),
	OFF ("client_state_t", client_state_t, mtime),
	OFF ("client_state_t", client_state_t, time),
	OFF ("client_state_t", client_state_t, oldtime),
	OFF ("client_state_t", client_state_t, last_received_message),
	OFF ("client_state_t", client_state_t, model_precache),
	OFF ("client_state_t", client_state_t, sound_precache),
	OFF ("client_state_t", client_state_t, mapname),
	OFF ("client_state_t", client_state_t, levelname),
	OFF ("client_state_t", client_state_t, viewentity),
	OFF ("client_state_t", client_state_t, maxclients),
	OFF ("client_state_t", client_state_t, gametype),
	OFF ("client_state_t", client_state_t, worldmodel),
	OFF ("client_state_t", client_state_t, free_efrags),
	OFF ("client_state_t", client_state_t, num_efrags),
	OFF ("client_state_t", client_state_t, efrag_allocs),
	OFF ("client_state_t", client_state_t, num_efragallocs),
	OFF ("client_state_t", client_state_t, viewent),
	OFF ("client_state_t", client_state_t, entities),
	OFF ("client_state_t", client_state_t, max_edicts),
	OFF ("client_state_t", client_state_t, num_entities),
	OFF ("client_state_t", client_state_t, static_entities),
	OFF ("client_state_t", client_state_t, max_static_entities),
	OFF ("client_state_t", client_state_t, num_statics),
	OFF ("client_state_t", client_state_t, cdtrack),
	OFF ("client_state_t", client_state_t, looptrack),
	OFF ("client_state_t", client_state_t, scores),
	OFF ("client_state_t", client_state_t, protocol),
	OFF ("client_state_t", client_state_t, protocolflags),
	OFF ("client_state_t", client_state_t, protocol_pext1),
	OFF ("client_state_t", client_state_t, protocol_pext2),
	OFF ("client_state_t", client_state_t, protocol_particles),
	OFF ("client_state_t", client_state_t, particle_precache),
	OFF ("client_state_t", client_state_t, local_particle_precache),
	OFF ("client_state_t", client_state_t, ackframes),
	OFF ("client_state_t", client_state_t, ackframes_count),
	OFF ("client_state_t", client_state_t, requestresend),
	OFF ("client_state_t", client_state_t, sendprespawn),
	OFF ("client_state_t", client_state_t, qcvm),
	OFF ("client_state_t", client_state_t, zoom),
	OFF ("client_state_t", client_state_t, zoomdir),
	OFF ("client_state_t", client_state_t, serverinfo),
};

size_t ctest_abi_host_lookup (const char *key)
{
	size_t i;
	for (i = 0; i < sizeof (ctest_abi_host_table) / sizeof (ctest_abi_host_table[0]); i++)
		if (!strcmp (ctest_abi_host_table[i].name, key))
			return ctest_abi_host_table[i].value;
	return (size_t)-1;
}
