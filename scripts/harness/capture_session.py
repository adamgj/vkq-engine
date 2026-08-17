#!/usr/bin/env python3
"""Protocol capture: run a dedicated server + headless client on localhost and
record the client's wire traffic via -netcapture (ADR-019 gate 4).

The capture is a stream of framed records:
  [u8 direction 0=recv,1=send][u8 driver][u8 kind 1=reliable,2=unreliable][u32le len][payload]

Usage:
  capture_session.py --vkquake <exe> [--game-data <dir>] --out <capture> \
                     [--map start] [--frames 2000] [--port 26011]
"""

import argparse
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import time


def stage(game_data):
    staging = tempfile.mkdtemp(prefix="vkq-c-")
    for entry in sorted(os.listdir(game_data)):
        src = os.path.join(game_data, entry)
        if not os.path.isdir(src):
            continue
        dst = os.path.join(staging, entry)
        os.makedirs(dst, exist_ok=True)
        for f in sorted(os.listdir(src)):
            os.symlink(os.path.join(src, f), os.path.join(dst, f))
    return staging


def summarize(path):
    counts = {}
    total = 0
    with open(path, "rb") as f:
        while True:
            hdr = f.read(7)
            if len(hdr) < 7:
                break
            direction, driver, kind = hdr[0], hdr[1], hdr[2]
            (length,) = struct.unpack("<I", hdr[3:7])
            f.seek(length, os.SEEK_CUR)
            counts[(direction, kind)] = counts.get((direction, kind), 0) + 1
            total += 1
    return total, counts


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--vkquake", required=True)
    p.add_argument("--game-data", default=os.environ.get("QUAKE_GAME_DATA"))
    p.add_argument("--out", required=True)
    p.add_argument("--map", default="start")
    p.add_argument("--frames", type=int, default=2000)
    p.add_argument("--port", type=int, default=26011)
    args = p.parse_args()

    if not args.game_data or not os.path.isdir(os.path.join(args.game_data, "id1")):
        sys.exit("error: game data not found; pass --game-data or set QUAKE_GAME_DATA")

    exe = os.path.abspath(args.vkquake)
    sv_dir = stage(args.game_data)
    cl_dir = stage(args.game_data)

    server = subprocess.Popen(
        [exe, "-dedicated", "-basedir", ".", "-port", str(args.port), "+map", args.map],
        cwd=sv_dir, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    try:
        time.sleep(3)  # let the server come up
        if server.poll() is not None:
            sys.exit(f"error: server exited early with {server.returncode}:\n" + server.stdout.read()[-2000:])

        client = subprocess.run(
            [exe, "-headless", "-basedir", ".", "-netcapture", "harness.cap",
             "-exitafter", str(args.frames), "+connect", f"127.0.0.1:{args.port}"],
            cwd=cl_dir, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
            timeout=600)
        # -exitafter exits with code 2 by design
        if client.returncode not in (0, 2):
            sys.stderr.write(client.stdout[-3000:])
            sys.exit(f"error: client exited with {client.returncode}")
    finally:
        server.terminate()
        try:
            server.wait(timeout=10)
        except subprocess.TimeoutExpired:
            server.kill()

    cap = os.path.join(cl_dir, "harness.cap")
    if not os.path.isfile(cap) or os.path.getsize(cap) == 0:
        sys.stderr.write(client.stdout[-3000:])
        sys.exit("error: no capture produced")
    shutil.move(cap, args.out)
    shutil.rmtree(sv_dir, ignore_errors=True)
    shutil.rmtree(cl_dir, ignore_errors=True)

    total, counts = summarize(args.out)
    print(f"capture: {total} records -> {args.out}")
    for (direction, kind), n in sorted(counts.items()):
        print(f"  {'send' if direction else 'recv'} {'reliable' if kind == 1 else 'unreliable'}: {n}")


if __name__ == "__main__":
    main()
