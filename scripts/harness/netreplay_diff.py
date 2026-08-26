#!/usr/bin/env python3
"""Captured-session replay byte gate (Phase 5 M8, ADR-019 gate 4).

Replays a -netcapture file deterministically on two builds (-netreplay +
-demohash force the fixed timestep) and byte-compares the per-frame
state-hash chains, plus a demo recorded during the replay. Unlike the live
capture_diff gate this is timing-noise-free: identical inputs, identical
frames, so the chains must match exactly.

Usage:
  netreplay_diff.py --capture <cap> --vkquake <exeA> [--vkquake-b <exeB>]
                    [--game-data <dir>] [--frames 1400]
Without --vkquake-b the same build runs twice (self-determinism check).
"""

import argparse
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
    staging = tempfile.mkdtemp(prefix="vkq-replay-")
    for entry in sorted(os.listdir(game_data)):
        src = os.path.join(game_data, entry)
        if not os.path.isdir(src):
            continue
        dst = os.path.join(staging, entry)
        os.makedirs(dst, exist_ok=True)
        for f in sorted(os.listdir(src)):
            _stage_entry(os.path.join(src, f), os.path.join(dst, f))
    return staging


def run_replay(exe, game_data, capture, frames, record_at):
    work = stage(game_data)
    try:
        with open(os.path.join(work, "harness.cmds"), "w") as f:
            f.write("0 connect replay\n")
            f.write(f"{record_at} record replaydemo\n")
            f.write(f"{frames - 50} stop\n")
        proc = subprocess.run(
            [os.path.abspath(exe), "-headless", "-basedir", ".",
             "-netreplay", os.path.abspath(capture),
             "-demohash", "harness.hash",
             "-exitafter", str(frames), "-harnesscmds", "harness.cmds"],
            cwd=work, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            text=True, timeout=600)
        if proc.returncode not in (0, 2):
            sys.stderr.write(proc.stdout[-3000:])
            sys.exit(f"error: replay client exited with {proc.returncode}")
        hashes = open(os.path.join(work, "harness.hash"), "rb").read()
        demo_path = os.path.join(work, "id1", "replaydemo.dem")
        demo = open(demo_path, "rb").read() if os.path.isfile(demo_path) else b""
        return hashes, demo, proc.stdout
    finally:
        shutil.rmtree(work, ignore_errors=True)


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--capture", required=True)
    p.add_argument("--vkquake", required=True)
    p.add_argument("--vkquake-b")
    p.add_argument("--game-data", default=os.environ.get("QUAKE_GAME_DATA"))
    p.add_argument("--frames", type=int, default=1400)
    p.add_argument("--record-at", type=int, default=60)
    args = p.parse_args()

    if not args.game_data or not os.path.isdir(os.path.join(args.game_data, "id1")):
        sys.exit("error: game data not found; pass --game-data or set QUAKE_GAME_DATA")

    ha, da, _ = run_replay(args.vkquake, args.game_data, args.capture,
                           args.frames, args.record_at)
    hb, db, outb = run_replay(args.vkquake_b or args.vkquake, args.game_data,
                              args.capture, args.frames, args.record_at)

    ok = True
    if not ha:
        print("FAIL: replay produced no state-hash chain")
        ok = False
    elif ha != hb:
        # find first divergent line for the report
        la, lb = ha.splitlines(), hb.splitlines()
        line = next((i for i, (x, y) in enumerate(zip(la, lb)) if x != y),
                    min(len(la), len(lb)))
        print(f"FAIL: state-hash chains diverge at line {line} "
              f"({len(la)} vs {len(lb)} lines)")
        ok = False
    else:
        print(f"ok: state-hash chains identical ({len(ha.splitlines())} frames)")

    if not da:
        print("FAIL: replay produced no recorded demo")
        sys.stderr.write(outb[-2000:])
        ok = False
    elif da != db:
        diff = next((i for i, (x, y) in enumerate(zip(da, db)) if x != y),
                    min(len(da), len(db)))
        print(f"FAIL: recorded demos diverge at byte {diff} "
              f"({len(da)} vs {len(db)} bytes)")
        ok = False
    else:
        print(f"ok: recorded demos identical ({len(da)} bytes)")

    print("netreplay_diff:", "PASS" if ok else "FAIL")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
