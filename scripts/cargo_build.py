#!/usr/bin/env python3
"""Meson <-> cargo bridge for the Rust migration.

Runs `cargo build` for the quake-capi staticlib, copies the artifact to the
path Meson expects, and rewrites cargo's dep-info file so ninja re-runs the
target when any Rust source changes.

Invoked from the quake_rs custom_target in meson.build.
"""

import argparse
import filecmp
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
    p.add_argument("--features", help="comma-separated cargo features")
    p.add_argument("--cbindgen")
    p.add_argument("--header-output")
    args = p.parse_args()

    cmd = [args.cargo, "build", "--locked", "-p", "quake-capi",
           "--manifest-path", args.manifest_path,
           "--target-dir", args.target_dir]
    if args.profile == "release":
        cmd.append("--release")
    if args.features:
        cmd += ["--features", args.features]

    # cwd must be the workspace root: rustup resolves rust-toolchain.toml by
    # walking up from the current directory, not from --manifest-path, and meson
    # runs this script with the build dir as cwd -- so the pin is silently
    # ignored unless we chdir into the workspace
    workspace = os.path.dirname(os.path.abspath(args.manifest_path))
    proc = subprocess.run(cmd, cwd=workspace)
    if proc.returncode != 0:
        sys.exit(proc.returncode)

    libname = "quake_rs.lib" if os.name == "nt" else "libquake_rs.a"
    artifact = os.path.join(args.target_dir, args.profile, libname)
    if not os.path.isfile(artifact):
        sys.exit(f"error: cargo produced no {artifact}")
    # only rewrite the output when the bytes actually changed: the custom_target
    # is build_always_stale, so an unconditional copy bumps the mtime and
    # relinks vkqr-engine on every ninja invocation even when cargo did nothing
    if not (os.path.isfile(args.output) and filecmp.cmp(artifact, args.output, shallow=False)):
        shutil.copyfile(artifact, args.output)

    # generate quake_rs.h (cbindgen), also copy-if-changed so including C files
    # only recompile when the exported API actually changed
    if args.cbindgen and args.header_output:
        crate_dir = os.path.join(workspace, "quake-capi")
        tmp_header = args.header_output + ".tmp"
        proc = subprocess.run([args.cbindgen,
                               "--config", os.path.join(crate_dir, "cbindgen.toml"),
                               "--output", tmp_header, crate_dir])
        if proc.returncode != 0:
            sys.exit(proc.returncode)
        if not (os.path.isfile(args.header_output)
                and filecmp.cmp(tmp_header, args.header_output, shallow=False)):
            shutil.copyfile(tmp_header, args.header_output)
        os.remove(tmp_header)

    # cargo's dep-info names its own artifact path; point it at meson's output
    cargo_dep = os.path.splitext(artifact)[0] + ".d"
    with open(args.depfile, "w") as out:
        if os.path.isfile(cargo_dep):
            with open(cargo_dep) as f:
                content = f.read()
            # each line is "<artifact>: <deps...>"; retarget to the copied lib.
            # partition on ": " rather than ":" -- a Windows dep-info line starts
            # with a drive letter ("C:\src\..."), which split(":", 1) would cut
            # in half and fold the old target into the dependency list
            for line in content.splitlines():
                _target, sep, deps = line.partition(": ")
                if sep:
                    out.write(f"{args.output}: {deps}\n")
        else:
            out.write(f"{args.output}:\n")


if __name__ == "__main__":
    main()
