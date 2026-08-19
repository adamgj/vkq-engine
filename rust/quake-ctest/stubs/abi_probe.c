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
