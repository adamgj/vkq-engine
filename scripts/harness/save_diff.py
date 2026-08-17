#!/usr/bin/env python3
"""Savegame byte-diff gate (ADR-019 gate 2).

Runs a deterministic scripted scenario (map load + saves at fixed frames via
-harnesscmds) on one or two vkQuake builds and byte-compares the resulting
.sav files. With one build, two runs prove run-to-run stability; with two
builds (e.g. C-only vs mixed, later C vs Rust), the saves must be identical
across builds.

Usage:
  save_diff.py --vkquake <exeA> [--vkquake-b <exeB>] [--game-data <dir>] \
               [--map e1m1] [--save-frames 300,600]
"""

import argparse
import filecmp
import os
import shutil
import subprocess
import sys
import tempfile


def _stage_entry(src, dst):
    try:
        os.symlink(src, dst)
    except OSError:
        shutil.copyfile(src, dst)  # Windows without symlink privilege


def run_scenario(exe, game_data, mapname, frames):
    staging = tempfile.mkdtemp(prefix="vkq-s-")
    for entry in sorted(os.listdir(game_data)):
        src = os.path.join(game_data, entry)
        if not os.path.isdir(src):
            continue
        dst = os.path.join(staging, entry)
        os.makedirs(dst, exist_ok=True)
        for f in sorted(os.listdir(src)):
            _stage_entry(os.path.join(src, f), os.path.join(dst, f))

    # the map is started from the cmds file, not +map: the cmdline cvar is
    # deliberately empty in shareware installs, so +commands never run there
    cmds = os.path.join(staging, "harness.cmds")
    with open(cmds, "w") as f:
        f.write(f"0 map {mapname}\n")
        for i, frame in enumerate(frames):
            f.write(f"{frame} save harness_{i}\n")
        f.write(f"{frames[-1] + 10} quit\n")

    cmd = [os.path.abspath(exe), "-headless", "-basedir", ".",
           "-harnesscmds", "harness.cmds", "-demohash", "harness.hash",
           "-exitafter", str(frames[-1] + 1000)]
    proc = subprocess.run(cmd, cwd=staging, stdout=subprocess.PIPE,
                          stderr=subprocess.STDOUT, text=True)
    if proc.returncode != 0:
        sys.stderr.write(proc.stdout[-4000:])
        shutil.rmtree(staging, ignore_errors=True)
        sys.exit(f"error: vkquake exited with {proc.returncode}")

    saves = {}
    savedir = os.path.join(staging, "id1")
    for i in range(len(frames)):
        path = os.path.join(savedir, f"harness_{i}.sav")
        if not os.path.isfile(path):
            sys.stderr.write(proc.stdout[-4000:])
            shutil.rmtree(staging, ignore_errors=True)
            sys.exit(f"error: expected save {path} was not written")
        keep = tempfile.NamedTemporaryFile(delete=False, suffix=".sav").name
        shutil.copyfile(path, keep)
        saves[i] = keep
    shutil.rmtree(staging, ignore_errors=True)
    return saves


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--vkquake", required=True)
    p.add_argument("--vkquake-b", default=None,
                   help="second build to compare against (default: rerun the first)")
    p.add_argument("--game-data", default=os.environ.get("QUAKE_GAME_DATA"))
    p.add_argument("--map", default="e1m1")
    p.add_argument("--save-frames", default="300,600")
    args = p.parse_args()

    if not args.game_data or not os.path.isdir(os.path.join(args.game_data, "id1")):
        sys.exit("error: game data not found; pass --game-data or set QUAKE_GAME_DATA")

    frames = [int(x) for x in args.save_frames.split(",")]
    a = run_scenario(args.vkquake, args.game_data, args.map, frames)
    b = run_scenario(args.vkquake_b or args.vkquake, args.game_data, args.map, frames)

    failed = False
    for i in sorted(a):
        if filecmp.cmp(a[i], b[i], shallow=False):
            print(f"save {i}: identical ({os.path.getsize(a[i])} bytes)")
            os.unlink(a[i])  # keep only the files that differ, for inspection
            os.unlink(b[i])
        else:
            print(f"save {i}: DIFFERS ({a[i]} vs {b[i]})")
            failed = True
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
