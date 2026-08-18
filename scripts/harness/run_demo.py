#!/usr/bin/env python3
"""Run vkqr-engine headless demo playback and collect the per-frame state hash.

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


def _stage_entry(src, dst):
    try:
        os.symlink(src, dst)
    except OSError:
        shutil.copyfile(src, dst)  # Windows without symlink privilege


def stage_basedir(game_data, staging):
    """Symlink each game dir (id1, hipnotic, ...) into a writable basedir."""
    for entry in sorted(os.listdir(game_data)):
        src = os.path.join(game_data, entry)
        if not os.path.isdir(src):
            continue
        dst = os.path.join(staging, entry)
        os.makedirs(dst, exist_ok=True)
        for f in sorted(os.listdir(src)):
            _stage_entry(os.path.join(src, f), os.path.join(dst, f))


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

        # the engine's cmdline cvar is CMDLINE_LENGTH (256) bytes and is
        # deliberately empty in shareware installs (stuffcmds does nothing),
        # so demos start via -harnesscmds rather than +playdemo, and we run
        # from the staging dir with short relative paths
        hashname = "harness.hash"
        cmd = [
            os.path.abspath(args.vkquake),
            "-headless",
            "-basedir", ".",
            "-demohash", hashname,
            "-exitafter", str(args.exitafter),
        ]
        cmdlines = []
        if args.cmds:
            with open(args.cmds) as f:
                cmdlines.extend(f.read().splitlines())
        if args.demo:
            cmdlines.append(f"0 playdemo {args.demo}")
        if cmdlines:
            with open(os.path.join(staging, "harness.cmds"), "w") as f:
                f.write("\n".join(cmdlines) + "\n")
            cmd += ["-harnesscmds", "harness.cmds"]
        if args.game:
            if args.game.startswith("-"):
                cmd += [args.game]
            else:
                cmd += ["-game", args.game]
        if args.extra_args:
            cmd += args.extra_args.split()

        # com_cmdline is truncated at CMDLINE_LENGTH, but the only consumer is the
        # informational `cmdline` cvar that stuffcmds reads -- and harness runs
        # drive everything through -harnesscmds, never +commands. Truncation is
        # therefore cosmetic, and failing on it would make the corpus pass or
        # fail depending on how long the path to the binary happens to be.
        cmdline_len = len(" ".join(cmd))
        if cmdline_len > 255:
            print(f"note: engine command line is {cmdline_len} chars, so the cmdline cvar "
                  f"will be truncated (CMDLINE_LENGTH is 256); harmless for harness runs",
                  file=sys.stderr)

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
