#!/usr/bin/env python3
"""Collect a progs VM instruction trace (ADR-019 gate 3).

Needs a -Dtrace=true build. Traces are enormous, so runs are bounded by
--exitafter (default 1000 frames) and the output is gzipped. The Phase 6
differ consumes these; in Phase 0 the deliverable is a stable producer:
--stability runs the scenario twice and byte-compares the traces.

Usage:
  run_trace.py --vkquake <trace-build-exe> [--game-data <dir>] --out <trace.gz> \
               (--demo demo1 | --map e1m1) [--exitafter N] [--stability]
"""

import argparse
import gzip
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


def run_once(exe, game_data, demo, mapname, exitafter):
    staging = tempfile.mkdtemp(prefix="vkq-t-")
    for entry in sorted(os.listdir(game_data)):
        src = os.path.join(game_data, entry)
        if not os.path.isdir(src):
            continue
        dst = os.path.join(staging, entry)
        os.makedirs(dst, exist_ok=True)
        for f in sorted(os.listdir(src)):
            _stage_entry(os.path.join(src, f), os.path.join(dst, f))

    with open(os.path.join(staging, "harness.cmds"), "w") as f:
        f.write(f"0 {'playdemo ' + demo if demo else 'map ' + mapname}\n")

    cmd = [os.path.abspath(exe), "-headless", "-basedir", ".",
           "-demohash", "harness.hash", "-tracefile", "harness.trace",
           "-harnesscmds", "harness.cmds", "-exitafter", str(exitafter)]
    proc = subprocess.run(cmd, cwd=staging, stdout=subprocess.PIPE,
                          stderr=subprocess.STDOUT, text=True)
    trace = os.path.join(staging, "harness.trace")
    if proc.returncode not in (0, 2) or not os.path.isfile(trace):
        sys.stderr.write(proc.stdout[-4000:])
        shutil.rmtree(staging, ignore_errors=True)
        sys.exit(f"error: trace run failed (exit {proc.returncode})")
    keep = tempfile.NamedTemporaryFile(delete=False, suffix=".trace").name
    shutil.move(trace, keep)
    shutil.rmtree(staging, ignore_errors=True)
    return keep


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--vkquake", required=True)
    p.add_argument("--game-data", default=os.environ.get("QUAKE_GAME_DATA"))
    p.add_argument("--out", required=True)
    p.add_argument("--demo", default=None)
    p.add_argument("--map", dest="mapname", default=None)
    p.add_argument("--exitafter", type=int, default=1000)
    p.add_argument("--stability", action="store_true")
    args = p.parse_args()

    if not args.game_data or not os.path.isdir(os.path.join(args.game_data, "id1")):
        sys.exit("error: game data not found; pass --game-data or set QUAKE_GAME_DATA")
    if bool(args.demo) == bool(args.mapname):
        sys.exit("error: pass exactly one of --demo or --map")

    a = run_once(args.vkquake, args.game_data, args.demo, args.mapname, args.exitafter)
    if args.stability:
        b = run_once(args.vkquake, args.game_data, args.demo, args.mapname, args.exitafter)
        same = open(a, "rb").read() == open(b, "rb").read()
        os.unlink(b)
        if not same:
            sys.exit("error: trace differs between runs")
        print("trace stable across two runs")

    with open(a, "rb") as fin, gzip.open(args.out, "wb") as fout:
        shutil.copyfileobj(fin, fout)
    lines = sum(1 for _ in open(a, "rb"))
    os.unlink(a)
    print(f"trace: {lines} records -> {args.out}")


if __name__ == "__main__":
    main()
