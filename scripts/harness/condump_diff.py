#!/usr/bin/env python3
"""Console + key-binding byte-diff gate (Rust migration Phase 7 M10).

`console.c` and `keys.c` have no state-hash coverage: `Harness_HashClient`
(Quake/harness.c:264) hashes client simulation vars and `cl.qcvm`, never the
console ring or the binding table, and neither module produces network traffic
-- so `capture_diff.py`, `record_diff.py` and `netreplay_diff.py` cannot see
them either. This gate closes that hole with an artifact byte-diff, the same
shape as `save_diff.py`.

What makes the artifact deterministic:

  * under `-headless` the renderer never starts, so `Con_CheckResize`
    (its only caller is gl_screen.c:895) never runs and `con_linewidth`
    keeps the `Con_Init` value of 50 (console.c:1068) for the whole run --
    the ring geometry is therefore identical in every build;
  * the script issues `clear` before it emits anything, so the dumped ring
    holds only text the script itself produced -- none of the startup banner,
    which legitimately differs between builds (host.c:1322 prints __DATE__ /
    __TIME__, and a `-Duse_rust` build additionally prints the staticlib
    banner at host.c:1326);
  * `unbindall` precedes the binds, so `bindlist` dumps a table built only by
    this script and not by whatever config.cfg the install happens to carry.

The dumped text therefore exercises `Con_Printf` line wrapping and the
scrollback ring (`echo` lines longer than con_linewidth), `Con_Clear_f`,
`Con_Dump_f`'s empty-line skipping and trailing-space trimming, and keys.c's
`Key_Bind_f` / `Key_Unbind_f` / `Key_Unbindall_f` / `Key_Bindlist_f` including
the key-name table and its dump order.

Not covered (recorded, not silently dropped): high-bit/coloured console text
-- no console command emits it, so `Con_Dump_f`'s `&= 0x7f` masking is
untested here; and `menu.c` / `sbar.c`, which have no dumpable artifact
(`Sbar_Init` is skipped entirely under `-headless`, host.c:1354).

Usage:
  condump_diff.py --vkquake <exeA> [--vkquake-b <exeB>] [--game-data <dir>]
                  [--map <name>] [--keep-on-fail]

Exit 0 when the two dumps are byte-identical, 1 when they differ, 2 on setup
errors.
"""

import argparse
import difflib
import os
import shutil
import subprocess
import sys
import tempfile

DUMP_NAME = "harness_con"

# (frame, command). Frames are spaced so each command lands in its own frame:
# Harness_Frame executes every command whose frame has arrived, in file order.
SCRIPT = [
    (100, "clear"),
    (101, "unbindall"),
    (102, 'bind a "+forward"'),
    (103, 'bind b "+back"'),
    (104, 'bind CTRL "+attack"'),
    (105, 'bind MOUSE1 "+jump"'),
    (106, 'bind UPARROW "impulse 10"'),
    (107, "unbind b"),
    (108, "bindlist"),
    (109, "echo short line"),
    # longer than con_linewidth (50): forces Con_Printf's line wrap
    (110, "echo " + " ".join(f"w{i:02d}" for i in range(24))),
    (111, 'echo "one quoted argument with embedded   spaces"'),
    (112, "echo"),
    (113, "bindlist"),
    (120, f"condump {DUMP_NAME}"),
]
QUIT_FRAME = 130


def _stage_entry(src, dst):
    try:
        os.symlink(src, dst)
    except OSError:
        shutil.copyfile(src, dst)  # Windows without symlink privilege


def run_scenario(exe, game_data, mapname):
    staging = tempfile.mkdtemp(prefix="vkq-cd-")
    for entry in sorted(os.listdir(game_data)):
        src = os.path.join(game_data, entry)
        if not os.path.isdir(src):
            continue
        dst = os.path.join(staging, entry)
        os.makedirs(dst, exist_ok=True)
        for f in sorted(os.listdir(src)):
            _stage_entry(os.path.join(src, f), os.path.join(dst, f))

    cmds = os.path.join(staging, "harness.cmds")
    with open(cmds, "w") as f:
        if mapname:
            f.write(f"0 map {mapname}\n")
        for frame, command in SCRIPT:
            f.write(f"{frame} {command}\n")
        f.write(f"{QUIT_FRAME} quit\n")

    cmd = [os.path.abspath(exe), "-headless", "-basedir", ".",
           "-harnesscmds", "harness.cmds", "-demohash", "harness.hash",
           "-exitafter", str(QUIT_FRAME + 1000)]
    proc = subprocess.run(cmd, cwd=staging, stdout=subprocess.PIPE,
                          stderr=subprocess.STDOUT, text=True)
    if proc.returncode != 0:
        sys.stderr.write(proc.stdout[-4000:])
        shutil.rmtree(staging, ignore_errors=True)
        sys.exit(f"error: vkquake exited with {proc.returncode}")

    path = os.path.join(staging, "id1", DUMP_NAME + ".txt")
    if not os.path.isfile(path):
        sys.stderr.write(proc.stdout[-4000:])
        shutil.rmtree(staging, ignore_errors=True)
        sys.exit(f"error: expected dump {path} was not written")
    keep = tempfile.NamedTemporaryFile(delete=False, suffix=".condump").name
    shutil.copyfile(path, keep)
    shutil.rmtree(staging, ignore_errors=True)
    return keep


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--vkquake", required=True)
    p.add_argument("--vkquake-b", default=None,
                   help="second build to compare against (default: rerun the first)")
    p.add_argument("--game-data", default=os.environ.get("QUAKE_GAME_DATA"))
    p.add_argument("--map", default="",
                   help="optional map to load at frame 0 (default: none -- the "
                        "console and binding table need no server)")
    p.add_argument("--keep-on-fail", action="store_true",
                   help="keep both dumps on disk when they differ")
    args = p.parse_args()

    if not args.game_data or not os.path.isdir(os.path.join(args.game_data, "id1")):
        sys.exit("error: game data not found; pass --game-data or set QUAKE_GAME_DATA")

    a = run_scenario(args.vkquake, args.game_data, args.map)
    b = run_scenario(args.vkquake_b or args.vkquake, args.game_data, args.map)

    with open(a, "rb") as f:
        da = f.read()
    with open(b, "rb") as f:
        db = f.read()

    if da == db:
        if not da.strip():
            sys.exit("error: both dumps are empty -- the script produced no "
                     "console text, so this comparison proves nothing")
        print(f"condump: identical ({len(da)} bytes, "
              f"{da.count(chr(10).encode())} lines)")
        os.unlink(a)
        os.unlink(b)
        return 0

    print(f"condump: DIFFERS ({len(da)} vs {len(db)} bytes)")
    lines = list(difflib.unified_diff(
        da.decode("latin-1").splitlines(), db.decode("latin-1").splitlines(),
        fromfile="A", tofile="B", lineterm=""))
    for line in lines[:60]:
        print(line)
    if len(lines) > 60:
        print(f"... {len(lines) - 60} more diff lines")
    if args.keep_on_fail:
        print(f"kept: {a}\nkept: {b}")
    else:
        os.unlink(a)
        os.unlink(b)
    return 1


if __name__ == "__main__":
    sys.exit(main())
