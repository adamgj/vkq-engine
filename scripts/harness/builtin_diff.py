#!/usr/bin/env python3
"""Builtin-table dump diff (ADR-019; ROADMAP Phase 6 exit criterion).

Runs `pr_dumpbuiltins` on two builds after loading the same progs.dat and
byte-compares the result. The dump is one line per extensionbuiltins[] entry --
`name declared-number bound-ordinal` -- plus the three re-release patch
results.

Why this gate exists at all, given the instruction-trace gate: builtin
*numbering* is decided by three mechanisms that never appear in a trace unless
a mod happens to call the affected builtin. PR_InitExtensions assigns
undocumented builtins downwards from 1024 in table order and mutates a static
table, so it depends on running exactly once; PR_EnableExtensions binds by
documented number but refuses to clobber an already-bound slot; and
PR_PatchRereleaseBuiltins rewrites first_statement afterwards and can undo an
extension binding. A divergence in any of them silently remaps what a mod's
`#0` functions resolve to.

Usage:
  builtin_diff.py --vkquake <build-a/vkqr-engine> --vkquake-b <build-b/...>
                  [--map start] [--game hipnotic] [--game-data <dir>]
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import run_trace  # noqa: E402  (same directory; shares the staging logic)

BEGIN = re.compile(r"^PRBUILTINS-BEGIN (\d+)$")


def dump(exe, game_data, mapname, game, exitafter):
    """Run one build and return the dump lines, or exit with a diagnostic."""
    staging = run_trace.stage(game_data)
    try:
        with open(os.path.join(staging, "harness.cmds"), "w") as f:
            f.write(f"0 map {mapname}\n")
            # several frames later, so the map is fully spawned and
            # PR_LoadProgs, PR_EnableExtensions and PR_PatchRereleaseBuiltins
            # have all run. The command selects sv.qcvm itself: console
            # commands run outside the server frame, where no VM is ambient.
            f.write("8 pr_dumpbuiltins\n")

        cmd = [os.path.abspath(exe), "-headless", "-basedir", ".",
               "-harnesscmds", "harness.cmds", "-exitafter", str(exitafter)]
        if game:
            cmd += ["-game", game]
        proc = subprocess.run(cmd, cwd=staging, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True)
        if proc.returncode not in (0, 2):
            sys.stderr.write(proc.stdout[-4000:])
            sys.exit(f"error: dump run failed (exit {proc.returncode})")
        return extract(proc.stdout)
    finally:
        shutil.rmtree(staging, ignore_errors=True)


def extract(stdout):
    """Pull the PRBUILTINS-BEGIN..END block out of the console output."""
    lines = []
    inside = False
    for line in stdout.splitlines():
        line = line.strip()
        if BEGIN.match(line):
            inside = True
            lines = [line]
            continue
        if not inside:
            continue
        if line == "PRBUILTINS-END":
            return lines
        if line.startswith(("PRBUILTIN ", "PRPATCH ")):
            lines.append(line)
    return []


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--vkquake", required=True)
    p.add_argument("--vkquake-b", required=True)
    p.add_argument("--game-data", default=os.environ.get("QUAKE_GAME_DATA"))
    p.add_argument("--map", dest="mapname", default="start")
    p.add_argument("--game", default=None,
                   help="mod/mission-pack dir, so the dump covers its progs.dat")
    p.add_argument("--exitafter", type=int, default=20)
    # the id1 table is ~500 entries; a build that printed nothing must fail
    # loudly rather than compare equal (the Phase 5 delivered-record lesson)
    p.add_argument("--min-entries", type=int, default=100)
    args = p.parse_args()

    if not args.game_data:
        sys.exit("error: --game-data or QUAKE_GAME_DATA is required")

    a = dump(args.vkquake, args.game_data, args.mapname, args.game, args.exitafter)
    b = dump(args.vkquake_b, args.game_data, args.mapname, args.game, args.exitafter)

    label = f"{args.game + '/' if args.game else ''}{args.mapname}"
    for name, got in (("A", a), ("B", b)):
        if len(got) < args.min_entries:
            sys.exit(f"error: build {name} produced {len(got)} dump lines over "
                     f"{label}, below the --min-entries floor of "
                     f"{args.min_entries}: the command did not run, or the "
                     f"progs did not load")

    if a == b:
        print(f"builtin table identical: {len(a) - 1} entries over {label}")
        return

    if a[0] != b[0]:
        sys.exit(f"error: the two builds loaded different progs "
                 f"({a[0]} vs {b[0]}) -- not a builtin-table divergence")

    shown = 0
    for i, (x, y) in enumerate(zip(a, b)):
        if x != y:
            print(f"  line {i}: A: {x}\n           B: {y}")
            shown += 1
            if shown >= 20:
                print("  ... (truncated)")
                break
    if len(a) != len(b):
        print(f"  and the dumps are different lengths: {len(a)} vs {len(b)}")
    sys.exit(f"error: builtin table differs over {label}")


if __name__ == "__main__":
    main()
