#!/usr/bin/env python3
"""config.cfg byte-diff gate (Rust migration Phase 7; completes at M10b).

`Host_WriteConfiguration` (Quake/host.c:486) is the engine's only config
writer, and it is a byte-diff subject in the plan's M2, M8 and M10 gate lines.
It emits, in order, `Key_WriteBindings` (keys.c:782), `Cvar_WriteVariables`
(cvar.c) and two literal trailing commands. Those three producers crossed into
Rust in different milestones -- the cvar registry at M2, the writer body itself
at M8 (absorbed into the host.c Pattern A swap), and the binding writer at
M10b -- so only from M10b on does this compare a fully Rust pipeline against
the C oracle.

What makes the artifact deterministic and hermetic:

  * `-headless` sets harness_active, and `COM_FOpenPrefFile`
    (common_fs.c) then redirects per-user state into `com_gamedir` instead of
    the real pref path, so the file lands in the disposable staging tree;
  * the script issues `unbindall` before its own binds, so the written table
    is built by this script alone and not by whatever config the install
    carries;
  * the run ends with an explicit `quit`, which reaches `Host_Shutdown`
    (host.c:1435) and therefore the writer. `-exitafter` is set well beyond
    that only as a deadlock backstop.

Any pre-existing `vkQuake.cfg` staged from the game data is removed before the
run: the staging tree links its files back to the read-only corpus data, and
opening one of those links "w" would write through into the source tree.

Covered: `Key_WriteBindings` key-name spelling and iteration order, including
bindings that carry embedded spaces and semicolons; `Cvar_WriteVariables`
selection (CVAR_ARCHIVE only) and its float formatting through the ADR-005
formatter; the trailing `vid_restart` / `+mlook` lines.

Not covered (recorded, not silently dropped): the non-harness pref-path
branch of `COM_FOpenPrefFile`, and the `Con_Printf ("Couldn't write ...")`
failure arm -- neither is reachable from a scripted headless run.

Usage:
  config_diff.py --vkquake <exeA> [--vkquake-b <exeB>] [--game-data <dir>]
                 [--keep-on-fail]

Exit 0 when the two files are byte-identical, 1 when they differ, 2 on setup
errors.
"""

import argparse
import difflib
import os
import shutil
import subprocess
import sys
import tempfile

CONFIG_NAME = "vkQuake.cfg"  # quakedef.h:33

# (frame, command). Frames are spaced so each command lands in its own frame:
# Harness_Frame executes every command whose frame has arrived, in file order.
SCRIPT = [
    (100, "unbindall"),
    (101, 'bind a "+forward"'),
    (102, 'bind b "+back"'),
    (103, 'bind CTRL "+attack"'),
    (104, 'bind MOUSE1 "+jump"'),
    (105, 'bind UPARROW "impulse 10"'),
    (106, 'bind SEMICOLON "impulse 12"'),
    (107, 'bind F5 "echo one two   three"'),
    (108, 'bind KP_ENTER "impulse 1; impulse 2"'),
    (109, "unbind b"),
    # archived cvars: exercise Cvar_WriteVariables' selection and the ADR-005
    # float formatter on integral, fractional and negative values
    (110, "sensitivity 3"),
    (111, "cl_bob 0.0200001"),
    (112, "m_pitch 0.75"),
    (113, "cl_forwardspeed 400"),
    (114, "cl_maxpitch 110.5"),
    (115, "m_side 0.0625"),
    (116, "cl_minpitch -2.5"),
    (117, '_cl_name "harness player"'),
]
QUIT_FRAME = 130


def _stage_entry(src, dst):
    try:
        os.symlink(src, dst)
    except OSError:
        shutil.copyfile(src, dst)  # Windows without symlink privilege


def run_scenario(exe, game_data):
    staging = tempfile.mkdtemp(prefix="vkq-cfg-")
    for entry in sorted(os.listdir(game_data)):
        src = os.path.join(game_data, entry)
        if not os.path.isdir(src):
            continue
        dst = os.path.join(staging, entry)
        os.makedirs(dst, exist_ok=True)
        for f in sorted(os.listdir(src)):
            _stage_entry(os.path.join(src, f), os.path.join(dst, f))

    # never let the writer follow a link back into the read-only source tree
    staged_cfg = os.path.join(staging, "id1", CONFIG_NAME)
    if os.path.lexists(staged_cfg):
        os.unlink(staged_cfg)

    cmds = os.path.join(staging, "harness.cmds")
    with open(cmds, "w") as f:
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

    if not os.path.isfile(staged_cfg):
        sys.stderr.write(proc.stdout[-4000:])
        shutil.rmtree(staging, ignore_errors=True)
        sys.exit(f"error: expected {staged_cfg} was not written")
    keep = tempfile.NamedTemporaryFile(delete=False, suffix=".cfg").name
    shutil.copyfile(staged_cfg, keep)
    shutil.rmtree(staging, ignore_errors=True)
    return keep


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--vkquake", required=True)
    p.add_argument("--vkquake-b", default=None,
                   help="second build to compare against (default: rerun the first)")
    p.add_argument("--game-data", default=os.environ.get("QUAKE_GAME_DATA"))
    p.add_argument("--keep-on-fail", action="store_true",
                   help="keep both configs on disk when they differ")
    args = p.parse_args()

    if not args.game_data or not os.path.isdir(os.path.join(args.game_data, "id1")):
        sys.exit("error: game data not found; pass --game-data or set QUAKE_GAME_DATA")

    a = run_scenario(args.vkquake, args.game_data)
    b = run_scenario(args.vkquake_b or args.vkquake, args.game_data)

    with open(a, "rb") as f:
        da = f.read()
    with open(b, "rb") as f:
        db = f.read()

    if da == db:
        binds = sum(1 for ln in da.split(b"\n") if ln.startswith(b"bind "))
        if binds == 0:
            sys.exit("error: the config carries no bind lines -- Key_WriteBindings "
                     "wrote nothing, so this comparison proves nothing")
        print(f"config.cfg: identical ({len(da)} bytes, "
              f"{da.count(chr(10).encode())} lines, {binds} bind lines)")
        os.unlink(a)
        os.unlink(b)
        return 0

    print(f"config.cfg: DIFFERS ({len(da)} vs {len(db)} bytes)")
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
