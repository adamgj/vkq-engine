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
