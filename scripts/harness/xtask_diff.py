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
repository root, so those blobs differ by design (see the plan amendment).

Usage:
  xtask_diff.py --build-dir <meson C build dir> [--out <dir>] [--cargo cargo]
"""

import argparse
import filecmp
import os
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def run_xtask(cargo, args):
    cmd = [cargo, "run", "--quiet", "--package", "xtask", "--"] + args
    subprocess.check_call(cmd, cwd=os.path.join(ROOT, "rust"))


def read_text_lf(path):
    with open(path, "rb") as f:
        return f.read().replace(b"\r\n", b"\n")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--build-dir", required=True, help="Meson build dir configured with -Duse_rust=disabled")
    ap.add_argument("--out", help="directory for the xtask outputs (default: a temp dir)")
    ap.add_argument("--cargo", default="cargo")
    a = ap.parse_args()

    out = a.out or tempfile.mkdtemp(prefix="vkq-xtask-")
    os.makedirs(out, exist_ok=True)
    run_xtask(a.cargo, ["shaders", "--out", out, "--c"])
    run_xtask(a.cargo, ["pak", "--out", out])

    names = sorted(n for n in os.listdir(out) if n.endswith(".spv") and not n.endswith(".raw.spv"))
    if len(names) != 61:
        print(f"FAIL: expected 61 shader outputs, xtask wrote {len(names)}")
        return 1

    failures = []
    checked = 0
    for spv in names:
        stem = spv[: -len(".spv")]
        for name, text in ((spv, False), (stem + ".c", True)):
            ours = os.path.join(out, name)
            theirs = os.path.join(a.build_dir, name)
            if not os.path.exists(theirs):
                failures.append(f"{name}: missing from {a.build_dir}")
                continue
            same = read_text_lf(ours) == read_text_lf(theirs) if text else filecmp.cmp(ours, theirs, shallow=False)
            checked += 1
            if not same:
                failures.append(f"{name}: differs")
    for name, text in (("vkquake.pak", False), ("embedded_pak.c", True)):
        ours = os.path.join(out, name)
        theirs = os.path.join(a.build_dir, name)
        if not os.path.exists(theirs):
            failures.append(f"{name}: missing from {a.build_dir}")
            continue
        same = read_text_lf(ours) == read_text_lf(theirs) if text else filecmp.cmp(ours, theirs, shallow=False)
        checked += 1
        if not same:
            failures.append(f"{name}: differs")

    for f in failures:
        print("FAIL:", f)
    if failures:
        return 1
    print(f"OK: {checked} xtask outputs byte-identical to {a.build_dir} ({len(names)} shaders + .c, vkquake.pak, embedded_pak.c)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
