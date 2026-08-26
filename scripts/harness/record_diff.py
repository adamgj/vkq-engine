#!/usr/bin/env python3
"""Demo-recording byte gate (Rust migration Phase 5 M4, ADR-019 gate 4
"demos recorded by both engines byte-diffed").

Runs a deterministic single-process loopback session (`-demohash` forces the
fixed timestep and fixed RNG seed; all traffic is the loopback driver) that
records a demo from frame 0, on two builds, and byte-diffs the resulting
.dem files. With one deterministic simulation on both sides, the recorded
bytes must be identical -- any divergence is a wire-layer or demo-format
regression.

Usage:
  record_diff.py --vkquake A [--vkquake-b B] [--game-data DIR]
                 [--map e1m1] [--frames 600]

With only --vkquake, runs the same build twice (self-determinism check).
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
        shutil.copyfile(src, dst)


def stage(game_data):
    staging = tempfile.mkdtemp(prefix="vkq-rd-")
    for entry in sorted(os.listdir(game_data)):
        src = os.path.join(game_data, entry)
        if not os.path.isdir(src):
            continue
        dst = os.path.join(staging, entry)
        os.makedirs(dst, exist_ok=True)
        for f in sorted(os.listdir(src)):
            _stage_entry(os.path.join(src, f), os.path.join(dst, f))
    return staging


def run_record(exe, game_data, map_name, frames):
    staging = stage(game_data)
    with open(os.path.join(staging, "harness.cmds"), "w") as f:
        f.write(f"0 record rdiff {map_name}\n")
        f.write(f"{frames - 60} stop\n")
    proc = subprocess.run(
        [os.path.abspath(exe), "-headless", "-basedir", ".",
         "-demohash", "rdiff.hash", "-exitafter", str(frames),
         "-harnesscmds", "harness.cmds"],
        cwd=staging, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        text=True, timeout=600)
    if proc.returncode not in (0, 2):
        sys.stderr.write(proc.stdout[-3000:])
        sys.exit(f"error: {exe} exited with {proc.returncode}")
    demo = os.path.join(staging, "id1", "rdiff.dem")
    if not os.path.isfile(demo) or os.path.getsize(demo) == 0:
        sys.stderr.write(proc.stdout[-3000:])
        sys.exit(f"error: {exe} produced no demo")
    return staging, demo


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--vkquake", required=True)
    p.add_argument("--vkquake-b")
    p.add_argument("--game-data", default=os.environ.get("QUAKE_GAME_DATA"))
    p.add_argument("--map", default="e1m1")
    p.add_argument("--frames", type=int, default=600)
    args = p.parse_args()

    if not args.game_data or not os.path.isdir(os.path.join(args.game_data, "id1")):
        sys.exit("error: game data not found; pass --game-data or set QUAKE_GAME_DATA")

    exe_b = args.vkquake_b or args.vkquake
    stage_a, demo_a = run_record(args.vkquake, args.game_data, args.map, args.frames)
    stage_b, demo_b = run_record(exe_b, args.game_data, args.map, args.frames)

    size_a, size_b = os.path.getsize(demo_a), os.path.getsize(demo_b)
    identical = size_a == size_b and filecmp.cmp(demo_a, demo_b, shallow=False)
    if identical:
        print(f"record_diff: identical ({size_a} bytes, map {args.map}, "
              f"{args.frames} frames)")
    else:
        with open(demo_a, "rb") as fa, open(demo_b, "rb") as fb:
            a, b = fa.read(), fb.read()
        off = next((i for i in range(min(len(a), len(b))) if a[i] != b[i]),
                   min(len(a), len(b)))
        print(f"record_diff: DIFFER (sizes {size_a} vs {size_b}, first "
              f"divergence at byte {off})")
    shutil.rmtree(stage_a, ignore_errors=True)
    shutil.rmtree(stage_b, ignore_errors=True)
    sys.exit(0 if identical else 1)


if __name__ == "__main__":
    main()
