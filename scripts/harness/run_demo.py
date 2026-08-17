#!/usr/bin/env python3
"""Run vkQuake headless demo playback and collect the per-frame state hash.

Stages a temporary writable basedir (game dirs symlinked from read-only game
data) so the engine can write configs/saves without touching the source data.

Usage:
  run_demo.py --vkquake <exe> [--game-data <dir>] [--game <mod>] \
              --demo <name> --out <hashfile> [--exitafter N] [--cmds <file>] \
              [--extra-args "..."]

Game data resolves from --game-data or $QUAKE_GAME_DATA and must contain id1/.
"""

import argparse
import os
import shutil
import subprocess
import sys
import tempfile

DEFAULT_EXITAFTER = 200000


def stage_basedir(game_data, staging):
    """Symlink each game dir (id1, hipnotic, ...) into a writable basedir."""
    for entry in sorted(os.listdir(game_data)):
        src = os.path.join(game_data, entry)
        if not os.path.isdir(src):
            continue
        dst = os.path.join(staging, entry)
        os.makedirs(dst, exist_ok=True)
        for f in sorted(os.listdir(src)):
            os.symlink(os.path.join(src, f), os.path.join(dst, f))


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--vkquake", required=True)
    p.add_argument("--game-data", default=os.environ.get("QUAKE_GAME_DATA"))
    p.add_argument("--game", help="mod/mission pack dir or -hipnotic/-rogue flavor", default=None)
    p.add_argument("--demo", help="demo name for +playdemo", default=None)
    p.add_argument("--out", required=True)
    p.add_argument("--exitafter", type=int, default=DEFAULT_EXITAFTER)
    p.add_argument("--cmds", help="-harnesscmds file", default=None)
    p.add_argument("--extra-args", default="")
    p.add_argument("--keep-basedir", action="store_true")
    args = p.parse_args()

    if not args.game_data or not os.path.isdir(os.path.join(args.game_data, "id1")):
        sys.exit("error: game data not found; pass --game-data or set QUAKE_GAME_DATA (must contain id1/)")

    staging = tempfile.mkdtemp(prefix="vkq-h-")
    try:
        stage_basedir(args.game_data, staging)

        # the engine's cmdline cvar is CMDLINE_LENGTH (256) bytes: run from the
        # staging dir with short relative paths so +commands never truncate away
        hashname = "harness.hash"
        cmd = [
            os.path.abspath(args.vkquake),
            "-headless",
            "-basedir", ".",
            "-demohash", hashname,
            "-exitafter", str(args.exitafter),
        ]
        if args.cmds:
            shutil.copyfile(args.cmds, os.path.join(staging, "harness.cmds"))
            cmd += ["-harnesscmds", "harness.cmds"]
        if args.game:
            if args.game.startswith("-"):
                cmd += [args.game]
            else:
                cmd += ["-game", args.game]
        if args.extra_args:
            cmd += args.extra_args.split()
        if args.demo:
            cmd += ["+playdemo", args.demo]

        cmdline_len = len(" ".join(cmd))
        if cmdline_len > 255:
            sys.exit(f"error: engine command line would be {cmdline_len} chars (CMDLINE_LENGTH is 256)")

        proc = subprocess.run(cmd, cwd=staging, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        if proc.returncode != 0:
            sys.stderr.write(proc.stdout[-4000:])
            sys.exit(f"error: vkquake exited with {proc.returncode}")
        staged_hash = os.path.join(staging, hashname)
        if not os.path.isfile(staged_hash) or os.path.getsize(staged_hash) == 0:
            sys.stderr.write(proc.stdout[-4000:])
            sys.exit("error: no hash output produced")
        shutil.move(staged_hash, args.out)
    finally:
        if args.keep_basedir:
            print(f"basedir kept at {staging}")
        else:
            shutil.rmtree(staging, ignore_errors=True)


if __name__ == "__main__":
    main()
