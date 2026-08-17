#!/usr/bin/env python3
"""Meson <-> cargo bridge for the Rust migration.

Runs `cargo build` for the quake-capi staticlib, copies the artifact to the
path Meson expects, and rewrites cargo's dep-info file so ninja re-runs the
target when any Rust source changes.

Invoked from the quake_rs custom_target in meson.build.
"""

import argparse
import os
import shutil
import subprocess
import sys


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--cargo", required=True)
    p.add_argument("--manifest-path", required=True)
    p.add_argument("--target-dir", required=True)
    p.add_argument("--profile", required=True, choices=["debug", "release"])
    p.add_argument("--output", required=True)
    p.add_argument("--depfile", required=True)
    args = p.parse_args()

    cmd = [args.cargo, "build", "--locked", "-p", "quake-capi",
           "--manifest-path", args.manifest_path,
           "--target-dir", args.target_dir]
    if args.profile == "release":
        cmd.append("--release")

    proc = subprocess.run(cmd)
    if proc.returncode != 0:
        sys.exit(proc.returncode)

    libname = "quake_rs.lib" if os.name == "nt" else "libquake_rs.a"
    artifact = os.path.join(args.target_dir, args.profile, libname)
    if not os.path.isfile(artifact):
        sys.exit(f"error: cargo produced no {artifact}")
    shutil.copyfile(artifact, args.output)

    # cargo's dep-info names its own artifact path; point it at meson's output
    cargo_dep = os.path.splitext(artifact)[0] + ".d"
    with open(args.depfile, "w") as out:
        if os.path.isfile(cargo_dep):
            with open(cargo_dep) as f:
                content = f.read()
            # each line is "<artifact>: <deps...>"; retarget to the copied lib
            for line in content.splitlines():
                if ":" in line:
                    deps = line.split(":", 1)[1]
                    out.write(f"{args.output}:{deps}\n")
        else:
            out.write(f"{args.output}:\n")


if __name__ == "__main__":
    main()
