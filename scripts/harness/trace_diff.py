#!/usr/bin/env python3
"""Diff two progs-VM instruction traces (ADR-019 gate 3).

Phase 0 built the producer (Quake/pr_trace.c, -Dtrace=true, run_trace.py);
this is the consumer the Phase 6 port is verified against. Both builds run the
same headless scenario, and every record the VM emits -- statements, global and
entity-field writes, function enter/leave, builtin calls and their returns --
must match, in order, exactly.

Only headless traces are oracles: PR_ExecuteProgram can run on a task worker
for CSQC drawing, and the trace sink is an unlocked FILE * (pr_trace.h).

Usage:
  trace_diff.py --vkquake <build-a/vkqr-engine> --vkquake-b <build-b/vkqr-engine> \
                (--demo demo1 | --map e1m1) [--game-data <dir>] \
                [--exitafter N] [--min-records N] [--context N]

Both executables must come from a -Dtrace=true build; a build without
-DPR_TRACE silently writes nothing, which the minimum-record floor catches.
"""

import argparse
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import run_trace  # noqa: E402  (same directory; shares the staging logic)

# pr_exec.c pr_opnames[], for decoding S records in failure output. The six
# "INDIRECT" entries are the LOAD_* family, which the C table does not name
# individually.
OPNAMES = [
    "DONE",
    "MUL_F", "MUL_V", "MUL_FV", "MUL_VF",
    "DIV",
    "ADD_F", "ADD_V",
    "SUB_F", "SUB_V",
    "EQ_F", "EQ_V", "EQ_S", "EQ_E", "EQ_FNC",
    "NE_F", "NE_V", "NE_S", "NE_E", "NE_FNC",
    "LE", "GE", "LT", "GT",
    "INDIRECT", "INDIRECT", "INDIRECT", "INDIRECT", "INDIRECT", "INDIRECT",
    "ADDRESS",
    "STORE_F", "STORE_V", "STORE_S", "STORE_ENT", "STORE_FLD", "STORE_FNC",
    "STOREP_F", "STOREP_V", "STOREP_S", "STOREP_ENT", "STOREP_FLD", "STOREP_FNC",
    "RETURN",
    "NOT_F", "NOT_V", "NOT_S", "NOT_ENT", "NOT_FNC",
    "IF", "IFNOT",
    "CALL0", "CALL1", "CALL2", "CALL3", "CALL4", "CALL5", "CALL6", "CALL7", "CALL8",
    "STATE",
    "GOTO",
    "AND", "OR",
    "BITAND", "BITOR",
]

EXPECTED_HEADER = b"PRTRACE 1"


def annotate(line):
    """Decode a record enough to make a diff readable."""
    parts = line.split()
    if not parts:
        return ""
    kind = parts[0]
    if kind == b"S" and len(parts) >= 3:
        try:
            op = int(parts[2])
        except ValueError:
            return ""
        name = OPNAMES[op] if 0 <= op < len(OPNAMES) else "?"
        return f"  (statement, op {op} = {name})"
    if kind == b"B" and len(parts) >= 2:
        return f"  (builtin #{parts[1].decode()})"
    if kind == b"E" and len(parts) >= 2:
        return f"  (enter function {parts[1].decode()})"
    if kind == b"L":
        return "  (leave function)"
    if kind == b"W":
        return "  (global write)"
    if kind == b"P":
        return "  (entity-field write)"
    if kind == b"R":
        return "  (builtin return)"
    return ""


def show(label, line):
    if line is None:
        return f"  {label}: <end of trace>"
    return f"  {label}: {line.decode(errors='replace')}{annotate(line)}"


def compare(path_a, path_b, min_records, context):
    with open(path_a, "rb") as fa, open(path_b, "rb") as fb:
        head_a = fa.readline().rstrip(b"\n")
        head_b = fb.readline().rstrip(b"\n")
        if head_a != EXPECTED_HEADER or head_b != EXPECTED_HEADER:
            sys.exit(
                f"error: trace header mismatch; expected {EXPECTED_HEADER!r}, "
                f"got a={head_a!r} b={head_b!r} (is this a -Dtrace=true build?)"
            )

        recent = []
        n = 0
        while True:
            la = fa.readline()
            lb = fb.readline()
            if not la and not lb:
                break
            la = la.rstrip(b"\n") if la else None
            lb = lb.rstrip(b"\n") if lb else None
            n += 1
            if la != lb:
                sys.stderr.write(f"trace divergence at record {n}\n")
                for i, line in enumerate(recent):
                    sys.stderr.write(
                        f"  ctx {n - len(recent) + i}: "
                        f"{line.decode(errors='replace')}{annotate(line)}\n"
                    )
                sys.stderr.write(show("A", la) + "\n")
                sys.stderr.write(show("B", lb) + "\n")
                sys.exit(1)
            recent.append(la)
            if len(recent) > context:
                recent.pop(0)

    # An engine that errored out early, or a build without -DPR_TRACE, produces
    # a short or empty trace that would otherwise compare equal and pass.
    if n < min_records:
        sys.exit(
            f"error: only {n} trace records (floor is {min_records}); the "
            "scenario did not execute enough progs code for this to be a gate"
        )
    return n


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--vkquake", required=True, help="build A (-Dtrace=true)")
    p.add_argument("--vkquake-b", required=True, help="build B (-Dtrace=true)")
    p.add_argument("--game-data", default=os.environ.get("QUAKE_GAME_DATA"))
    p.add_argument("--demo", default=None)
    p.add_argument("--map", dest="mapname", default=None)
    p.add_argument("--exitafter", type=int, default=1000)
    p.add_argument("--game", default=None,
                   help="mod/mission-pack dir, so the trace covers its progs.dat")
    p.add_argument("--min-records", type=int, default=10000)
    p.add_argument("--context", type=int, default=8)
    args = p.parse_args()

    if not args.game_data or not os.path.isdir(os.path.join(args.game_data, "id1")):
        sys.exit("error: game data not found; pass --game-data or set QUAKE_GAME_DATA")
    if bool(args.demo) == bool(args.mapname):
        sys.exit("error: pass exactly one of --demo or --map")

    scenario = args.demo or args.mapname
    a = run_trace.run_once(
        args.vkquake, args.game_data, args.demo, args.mapname, args.exitafter, args.game
    )
    try:
        b = run_trace.run_once(
            args.vkquake_b, args.game_data, args.demo, args.mapname, args.exitafter, args.game
        )
        try:
            n = compare(a, b, args.min_records, args.context)
        finally:
            os.unlink(b)
    finally:
        os.unlink(a)

    where = f"{args.game}/{scenario}" if args.game else scenario
    print(f"trace identical: {n} records over {where}")


if __name__ == "__main__":
    main()
