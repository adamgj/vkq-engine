#!/usr/bin/env python3
"""Physics-determinism cvar-matrix driver (Phase 7 M1 T1.2, ADR-019).

Sweeps the physics-behavior cvar combinations that gate pusher/hullcheck
codepaths across two builds (or one build against itself) and compares the
per-frame server state hash, so the roadmap exit criterion "physics
determinism suite: pusher modes 0-3, both hullcheck impls" has a runnable
gate. This is a thin orchestrator on top of run_corpus.py's per-entry
plumbing (entry_available/run_entry, imported directly -- see below) plus
run_demo.py's --extra-args passthrough; it adds nothing to the engine or to
those scripts.

Matrix axes (verified against the engine source, not assumed):
  sv_fte_recursivehullckeck  0/1   Quake/world.c:33 (default "1"); gates the
                                    fast point-trace path in
                                    SV_RecursiveHullCheck, Quake/world.c:889
                                    ("both hullcheck impls" from the roadmap).
  sv_gameplayfix_elevators   0-3   Quake/sv_phys.c:704-705: "0=off; 1=legacy
                                    DIST_EPSILON nudge, clients only; 2=legacy
                                    nudge, all entities; 3=robust pusher
                                    contact (default)" -- the roadmap's
                                    "pusher modes 0-3".
  sv_smoothplatformlerps     0/1   Quake/sv_main.c:38 (default "1"); gates
                                    SV_UsePredThinkPos, Quake/sv_main.c:44-56,
                                    which only applies to MOVETYPE_STEP
                                    entities that are FL_ONGROUND -- i.e. it
                                    is about *walking* monsters, not pushers.
                                    Its predthinkpos/lastthink output is part
                                    of the harness edict hash (harness.c:207).

Cell count: the naive full factorial is 2x4x2 = 16. The Phase 7 plan
(docs/ai/plans/rust-conversion-phase-7.md:55) calls for "12 cells; document
CI vs local split". Code evidence argues AGAINST assuming
sv_smoothplatformlerps is redundant with sv_gameplayfix_elevators: the two
gate disjoint movetypes (MOVETYPE_STEP walkers vs MOVETYPE_PUSH pushers), so
collapsing the matrix on the premise "lerps only matters with certain
elevator modes" is not supported -- FULL_CELLS therefore keeps the complete
16-cell grid, always available locally / at milestone boundaries via
`--cells all`. DEFAULT_CELLS (12 cells, the CI leg) is instead a coverage
trim, not a redundancy claim: the full hullcheck x elevator grid (8 cells)
at the engine's default sv_smoothplatformlerps=1, plus both hullcheck impls
at the two sv_smoothplatformlerps=0 extremes of the orthogonal elevator axis
(off=0, robust-default=3; 4 cells) -- 12 cells that still exercise every
pusher mode, both hullcheck impls, and both lerp settings at least once.

Usage:
  physics_matrix.py --vkquake-a <exe> [--vkquake-b <exe>]
                    [--game-data <dir>] [--entries e1,e2,...]
                    [--cells hc0-el0-lp0 ... | all] [--list]

Omitting --vkquake-b runs each cell as a same-build self-compare (build A
against itself under identical cvars) -- a determinism/plumbing check, not
a C-vs-Rust gate. Passing --vkquake-b makes each cell a genuine differential
cell: both builds run under the *same* cvar combo, so a mismatch means the
two builds disagree about that combo's physics, not that the combos differ
from each other.

Cvar delivery (fixed after a T1.8 functional-validation finding): cells do
NOT use "+cvar value" via run_demo.py's --extra-args/stuffcmds. That path
round-trips through com_cmdline, a fixed CMDLINE_LENGTH=256-byte buffer
(Quake/common.c COM_InitArgv) built by joining the FULL argv *including
argv[0]* (the absolute exe path) -- Cmd_StuffCmds_f (Quake/cmd.c:237) then
reads only that truncated buffer. Proven empirically: on this checkout's
path length, a cell run this way silently dropped its trailing +cvar
tokens, so sv_gameplayfix_elevators=0 and =3 produced byte-identical
hashes on a pusher-heavy entry -- every cell had secretly been running
under engine defaults regardless of the requested combo, a "detector that
has never fired" (ADR-019) masked as 72/72 green. Cells instead prepend
"0 <cvar> <value>" lines to the entry's own frame-scripted cmds (the same
Harness_Frame/Cbuf_AddText console-injection mechanism interop_matrix.py's
--inject-desync-at already uses), ahead of the entry's own frame-0 "map"
line, so cvars land before SV_SpawnServer runs regardless of exe path
length -- Cbuf's dynamic buffer has no CMDLINE_LENGTH-sized cap.

KNOWN GAP (found under the same T1.8 functional-validation pass; PARTIALLY
fixed -- see the Phase 7 plan's amendment log): with the delivery bug above
fixed, a direct differential test (same binary, elevators=0 vs elevators=3,
lerps=0 vs lerps=1, hullcheck held at its default) originally produced
byte-identical hashes on every server-exercising entry that existed at the
time (e1m5-trains, e3m6-trains, save-e1m1, map-e1m2, e1m1-long, save-e2m1),
because none of them actually triggered a pusher BLOCKED contact. Root
cause: per Quake/sv_phys.c:704-705 (gated further at sv_phys.c:1540-1546),
sv_gameplayfix_elevators only changes behavior when an entity riding a
pusher (FL_ONGROUND with groundentity==pusher) is still embedded in it
after the pusher's move -- not for an entity merely shoved sideways out of
a pusher's way, which is unaffected regardless of contact -- and
sv_smoothplatformlerps (sv_main.c:44-56) only applies to MOVETYPE_STEP
entities that are FL_ONGROUND. The elevators axis is now proven non-
vacuous: the e1m1-plat-crush entry (Misc/harness/corpus.json) parks the
player inside e1m1's func_plat trigger field so the plat repeatedly rides
over and crushes it, and a direct differential run (elevators=0 vs 3,
cvar_cmds_for-style prepended cmds) diverges starting at frame 56, while a
same-cvar rerun reproduces byte-identical output -- see that entry's `note`
for the exact method. The lerps axis (a MOVETYPE_STEP monster riding a
lift while FL_ONGROUND) is STILL vacuous: no corpus entry scripts a
monster onto a moving pusher (there is no setpos-equivalent for
repositioning non-player edicts, and e1m1's monster_dog never patrols into
the plat's trigger footprint within tested frame budgets), so lerps=0 vs
lerps=1 still cannot fail on any current entry and must not be trusted as
a gate for M4 (sv_move.c/sv_phys.c port) until such an entry is authored.
Flagged as a follow-up content-authoring task, since a correct scripted
intercept requires interactive, map-specific interaction (monster
positioning against a moving pusher's timing) that cannot be derived from
static analysis alone.
"""

import argparse
import json
import os
import re
import shutil
import sys
import tempfile

HARNESS_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HARNESS_DIR)
import run_corpus  # noqa: E402  -- reuses entry_available()/run_entry(), not reimplemented

HULLCHECK_VALUES = (0, 1)
ELEVATOR_VALUES = (0, 1, 2, 3)
LERPS_VALUES = (0, 1)
_MAP_CMD_RE = re.compile(r"^\d+\s+map\s")


def cell_name(hullcheck, elevators, lerps):
    return f"hc{hullcheck}-el{elevators}-lp{lerps}"


def build_full_cells():
    cells = {}
    for h in HULLCHECK_VALUES:
        for e in ELEVATOR_VALUES:
            for l in LERPS_VALUES:
                name = cell_name(h, e, l)
                cells[name] = (h, e, l)
    return cells


FULL_CELLS = build_full_cells()


def build_default_cells():
    names = []
    for h in HULLCHECK_VALUES:
        for e in ELEVATOR_VALUES:
            names.append(cell_name(h, e, 1))
    for h in HULLCHECK_VALUES:
        for e in (0, 3):
            names.append(cell_name(h, e, 0))
    return names


DEFAULT_CELLS = build_default_cells()  # the 12-cell CI leg


def cvar_cmds_for(hullcheck, elevators, lerps):
    """Frame-0 console commands that set this cell's cvar combo, for
    prepending to an entry's own cmds list (see module docstring: this
    replaces a prior +cvar/--extra-args approach that silently truncated)."""
    return [
        f"0 sv_fte_recursivehullckeck {hullcheck}",
        f"0 sv_gameplayfix_elevators {elevators}",
        f"0 sv_smoothplatformlerps {lerps}",
    ]


def server_exercising_entries(corpus):
    """Entries whose harness cmds spawn a local server (a frame-0 'map' line).

    Demo-playback entries (id1-demo1, hipnotic-demo1, ...) replay a network
    capture client-side and never call SV_SpawnServer, so they cannot
    exercise SV_Physics/pusher/hullcheck code and are excluded. This is
    detected from corpus.json rather than hardcoded, so new server-exercising
    entries (Phase 7 T1.1) are picked up automatically. An entry that spawns a
    server but is not a physics scenario opts out with "physics": false in
    corpus.json (gamedir-switch does: it is a filesystem test, and it drags a
    hipnotic dependency into an otherwise shareware+PAK1 matrix).
    """
    return [e["name"] for e in corpus["entries"]
            if e.get("physics", True)
            and any(_MAP_CMD_RE.match(c) for c in e.get("cmds", []))]


def resolve_data_root(entry, game_data):
    data_root = game_data
    if entry.get("data_subdir"):
        data_root = os.path.join(game_data, entry["data_subdir"])
        if not os.path.isdir(data_root):
            return None, f"missing data dir {entry['data_subdir']}"
    reason = run_corpus.entry_available(entry, data_root)
    if reason:
        return None, reason
    return data_root, None


def first_diverging_line(path_a, path_b):
    """1-based line number of the first differing hash record, or None."""
    with open(path_a, encoding="utf-8", errors="replace") as fa,             open(path_b, encoding="utf-8", errors="replace") as fb:
        for n, (la, lb) in enumerate(zip(fa, fb), 1):
            if la != lb:
                return n, la.rstrip(), lb.rstrip()
    return None


def run_cell(exe_a, exe_b, entry, game_data, hullcheck, elevators, lerps, results_dir=None):
    data_root, reason = resolve_data_root(entry, game_data)
    if reason:
        return "SKIP", reason

    cell_entry = dict(entry)
    cell_entry["cmds"] = cvar_cmds_for(hullcheck, elevators, lerps) + list(entry.get("cmds", []))

    a = tempfile.NamedTemporaryFile(suffix=".hash", delete=False).name
    b = tempfile.NamedTemporaryFile(suffix=".hash", delete=False).name
    # a failing cell must leave its hash chains behind: deleting them on the
    # DIFFERS path reduces a red CI cell to a cell name with nothing to bisect,
    # which is the opposite of what interop_matrix.py's dump_soak_failure does
    keep = False
    try:
        ok = (run_corpus.run_entry(exe_a, cell_entry, data_root, a)
              and run_corpus.run_entry(exe_b, cell_entry, data_root, b))
        if not ok:
            return "FAIL", "run failed (no hash produced)"
        if open(a, "rb").read() == open(b, "rb").read():
            return "OK", None
        keep = True
        cell = f"{entry['name']}-{cell_name(hullcheck, elevators, lerps)}"
        if results_dir:
            os.makedirs(results_dir, exist_ok=True)
            for src, side in ((a, "a"), (b, "b")):
                dst = os.path.join(results_dir, f"{cell}-{side}.hash")
                shutil.move(src, dst)
                if side == "a":
                    a = dst
                else:
                    b = dst
        div = first_diverging_line(a, b)
        where = f", first diverging record line {div[0]}: {div[1]!r} vs {div[2]!r}" if div else ""
        return "FAIL", f"DIFFERS ({a} vs {b}){where}"
    finally:
        if not keep:
            for p in (a, b):
                try:
                    os.remove(p)
                except OSError:
                    pass


def main():
    p = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--vkquake-a", required=True, help="build under test")
    p.add_argument("--vkquake-b", default=None,
                    help="second build for a real C-vs-Rust differential; "
                         "omit for a same-build self-compare (determinism/plumbing check)")
    p.add_argument("--game-data", default=os.environ.get("QUAKE_GAME_DATA"))
    p.add_argument("--entries", default=None,
                    help="comma-separated corpus entry names "
                         "(default: entries whose harness cmds spawn a local server)")
    p.add_argument("--cells", nargs="*", default=None,
                    help=f"cell names to run, or 'all' for the full {len(FULL_CELLS)}-cell "
                         f"grid (default: the {len(DEFAULT_CELLS)}-cell CI leg)")
    p.add_argument("--results-dir", default=None,
                   help="directory to retain a failing cell's two hash chains in "
                        "(default: the system temp dir, which CI will not collect)")
    p.add_argument("--list", action="store_true",
                    help="print the selected cells and entries, then exit")
    args = p.parse_args()

    with open(run_corpus.CORPUS) as f:
        corpus = json.load(f)
    by_name = {e["name"]: e for e in corpus["entries"]}

    if args.entries:
        entry_names = args.entries.split(",")
        unknown = [n for n in entry_names if n not in by_name]
        if unknown:
            sys.exit(f"error: unknown corpus entries: {', '.join(unknown)}")
    else:
        entry_names = server_exercising_entries(corpus)

    if args.cells is None:
        cell_names = DEFAULT_CELLS
    elif "all" in args.cells:
        cell_names = list(FULL_CELLS)
    else:
        unknown = [n for n in args.cells if n not in FULL_CELLS]
        if unknown:
            sys.exit(f"error: unknown cells: {', '.join(unknown)} "
                     f"(known: {', '.join(sorted(FULL_CELLS))})")
        cell_names = args.cells

    if args.list:
        print(f"cells ({len(cell_names)} selected / {len(FULL_CELLS)} total, "
              f"CI leg has {len(DEFAULT_CELLS)}):")
        for name in cell_names:
            h, e, l = FULL_CELLS[name]
            tag = "ci+local" if name in DEFAULT_CELLS else "local-only"
            print(f"  {name:12s} hullcheck={h} elevators={e} lerps={l}  [{tag}]")
        print(f"entries ({len(entry_names)} selected):")
        for name in entry_names:
            print(f"  {name}")
        return

    if not args.game_data:
        sys.exit("error: pass --game-data or set QUAKE_GAME_DATA")

    exe_a = os.path.abspath(args.vkquake_a)
    exe_b = os.path.abspath(args.vkquake_b) if args.vkquake_b else exe_a
    mode = "compare" if args.vkquake_b else "self-compare"

    grid = {}  # (cell, entry) -> (status, detail)
    ok = True
    for cell_n in cell_names:
        h, e, l = FULL_CELLS[cell_n]
        for entry_n in entry_names:
            entry = by_name[entry_n]
            status, detail = run_cell(exe_a, exe_b, entry, args.game_data, h, e, l,
                                      results_dir=args.results_dir)
            grid[(cell_n, entry_n)] = (status, detail)
            if status == "OK":
                print(f"ok: {cell_n} {entry_n}")
            elif status == "SKIP":
                print(f"skipped: {cell_n} {entry_n}: {detail}")
            else:
                print(f"FAIL: {cell_n} {entry_n}: {detail}")
                ok = False

    col_w = max(10, max((len(n) for n in entry_names), default=10))
    header = "cell".ljust(12) + "".join(n.ljust(col_w + 1) for n in entry_names)
    print(f"\n{mode} matrix ({len(cell_names)} cells x {len(entry_names)} entries):")
    print(header)
    symbol = {"OK": "ok", "SKIP": "skip", "FAIL": "FAIL"}
    for cell_n in cell_names:
        row = cell_n.ljust(12)
        for entry_n in entry_names:
            status, _ = grid[(cell_n, entry_n)]
            row += symbol[status].ljust(col_w + 1)
        print(row)

    n_ok = sum(1 for s, _ in grid.values() if s == "OK")
    n_skip = sum(1 for s, _ in grid.values() if s == "SKIP")
    n_fail = sum(1 for s, _ in grid.values() if s == "FAIL")
    print(f"\n{n_ok} ok, {n_skip} skipped, {n_fail} failed")
    sys.exit(1 if not ok else 0)


if __name__ == "__main__":
    main()
