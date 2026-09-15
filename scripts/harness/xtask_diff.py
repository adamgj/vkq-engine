#!/usr/bin/env python3
"""xtask shader/pak byte-identity gate (Phase 8 M11, task plan D8, AC10).

Runs `cargo xtask shaders --c` and `cargo xtask pak` and byte-compares every
output with what Meson's glslangValidator/spirv-opt/bintoc/mkpak custom
targets wrote into a C-only build directory (the same tools, so the .spv
files must match; bintoc/mkpak are reimplemented, so the .c files and
vkquake.pak must match too).

The .c files are compared after CRLF normalisation: bintoc opens its output
in text mode, so a Windows C build writes CRLF while xtask always writes LF.

Debug buildtypes are out of scope: glslang -g embeds the source path, which
Meson passes relative to the build directory and xtask relative to the
repository root, so those blobs differ by design (see the plan amendment);
the build dir's meson-info is checked up front. Both sides must also run
the same glslangValidator/spirv-opt binaries (two spirv-opt versions do not
agree on -Os output): Meson records its find_program results in
meson-logs/meson-log.txt, point QUAKE_GLSLANG/QUAKE_SPIRV_OPT at them when
PATH resolves something else. A default --out temp dir is removed on
success and kept (and named) on failure.

Usage:
  xtask_diff.py --build-dir <meson C build dir> [--out <dir>] [--cargo cargo]
"""

import argparse
import filecmp
import json
import os
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def run_xtask(cargo, args):
    cmd = [cargo, "run", "--quiet", "--locked", "--package", "xtask", "--"] + args
    subprocess.check_call(cmd, cwd=os.path.join(ROOT, "rust"))


def read_text_lf(path):
    with open(path, "rb") as f:
        return f.read().replace(b"\r\n", b"\n")


def meson_buildtype(build_dir):
    with open(os.path.join(build_dir, "meson-info", "intro-buildoptions.json"), "rb") as f:
        return next(o["value"] for o in json.load(f) if o["name"] == "buildtype")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--build-dir", required=True, help="Meson build dir configured with -Duse_rust=disabled")
    ap.add_argument("--out", help="directory for the xtask outputs (default: a temp dir)")
    ap.add_argument("--cargo", default="cargo")
    a = ap.parse_args()

    # xtask runs with cwd=rust, so both paths must be absolute before Cargo
    # sees them; --out must not be the reference dir or the compare is a no-op.
    build_dir = os.path.abspath(a.build_dir)
    try:
        buildtype = meson_buildtype(build_dir)
    except (OSError, StopIteration, ValueError) as e:
        print(f"FAIL: {build_dir} is not a Meson build directory ({e})")
        return 2
    if buildtype.startswith("debug"):
        print(f"FAIL: {build_dir} is a {buildtype} buildtype; the identity claim is release-only (see the docstring)")
        return 2
    out = os.path.abspath(a.out) if a.out else tempfile.mkdtemp(prefix="vkq-xtask-")
    if os.path.normcase(out) == os.path.normcase(build_dir):
        print("FAIL: --out must not be the C build directory")
        return 2
    os.makedirs(out, exist_ok=True)
    run_xtask(a.cargo, ["shaders", "--out", out, "--c"])
    run_xtask(a.cargo, ["pak", "--out", out])

    def spv_names(d):
        return sorted(n for n in os.listdir(d) if n.endswith(".spv") and not n.endswith(".raw.spv"))

    def kept():
        if not a.out:
            print(f"xtask outputs kept in {out}")

    # Both directions: a job in meson.build's tables but not in the xtask
    # table would otherwise pass with one shader fewer.
    names = spv_names(out)
    if len(names) != 61:
        print(f"FAIL: expected 61 shader outputs, xtask wrote {len(names)}")
        kept()
        return 1
    meson_names = spv_names(build_dir)
    if meson_names != names:
        for n in sorted(set(meson_names) - set(names)):
            print(f"FAIL: {n}: built by Meson but not by xtask (rust/xtask/src/shaders.rs job table out of date)")
        for n in sorted(set(names) - set(meson_names)):
            print(f"FAIL: {n}: built by xtask but not by Meson")
        kept()
        return 1

    failures = []
    checked = 0
    for spv in names:
        stem = spv[: -len(".spv")]
        for name, text in ((spv, False), (stem + ".c", True)):
            ours = os.path.join(out, name)
            theirs = os.path.join(build_dir, name)
            if not os.path.exists(theirs):
                failures.append(f"{name}: missing from {build_dir}")
                continue
            same = read_text_lf(ours) == read_text_lf(theirs) if text else filecmp.cmp(ours, theirs, shallow=False)
            checked += 1
            if not same:
                failures.append(f"{name}: differs")
    pak_identical = False
    for name, text in (("vkquake.pak", False), ("embedded_pak.c", True)):
        ours = os.path.join(out, name)
        theirs = os.path.join(build_dir, name)
        if not os.path.exists(theirs):
            failures.append(f"{name}: missing from {build_dir}")
            continue
        same = read_text_lf(ours) == read_text_lf(theirs) if text else filecmp.cmp(ours, theirs, shallow=False)
        checked += 1
        if same and name == "vkquake.pak":
            pak_identical = True
        if not same:
            failures.append(f"{name}: differs")
            if name == "embedded_pak.c" and pak_identical:
                failures.append("  (vkquake.pak is identical, so this is the deflate: a final block of <= 32 coded bytes is the"
                                " known miniz_oxide/tdefl divergence, see deflate_raw in rust/xtask/src/bintoc.rs -- both inflate"
                                " to the same bytes, accept it rather than changing xtask)")

    for f in failures:
        print("FAIL:", f)
    if failures:
        kept()
        return 1
    print(f"OK: {checked} xtask outputs byte-identical to {build_dir} ({len(names)} shaders + .c, vkquake.pak, embedded_pak.c)")
    if not a.out:
        shutil.rmtree(out, ignore_errors=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
