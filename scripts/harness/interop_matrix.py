#!/usr/bin/env python3
"""4-way interop matrix (Phase 5 M8, ADR-019 gate 4 / phase-5 exit criterion).

Runs C/Rust client x C/Rust server localhost sessions across the protocol
cells this server can negotiate:

  Base-15   protocol 15, no FTE extensions      FTE+15   15  + PEXT2
  Base-666  protocol 666, no FTE extensions     FTE+666  666 + PEXT2
  Base-999  999 (PRFL_INT32COORD|SHORTANGLE)    FTE+999  999 + PEXT2
                                                         (PRFL_FLOATCOORD|SHORTANGLE)

Per cell x build-combo the gate checks: the client negotiated the expected
protocol string, the session produced healthy traffic, and -- across the
four build combos of a cell -- the reliable record counts match exactly and
the unreliable counts sit within the live-session noise floor. The
msgbadread counters are compared C-vs-Rust per the M1 amendment (benign
events occur by design on the dgrm connect path).

Live sessions carry run-to-run timing noise, so byte-exactness is NOT
asserted here -- that is netreplay_diff.py's job. This matrix proves the
cross-implementation handshake+session health that a replay cannot.

Usage:
  interop_matrix.py --vkquake-c <exe> --vkquake-rs <exe>
                    [--game-data <dir>] [--frames 600] [--cells ...]
                    [--ipv6] [--base-port 26100]
"""

import argparse
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import time

CELLS = ["Base-15", "FTE+15", "Base-666", "FTE+666", "Base-999", "FTE+999"]
EXPECT_PROTO = {
    "Base-15": "15",
    "FTE+15": "fte15",
    "Base-666": "666",
    "FTE+666": "fte666",
    "Base-999": "999",
    "FTE+999": "fte999",
}


def _stage_entry(src, dst):
    try:
        os.symlink(src, dst)
    except OSError:
        shutil.copyfile(src, dst)


def stage(game_data):
    staging = tempfile.mkdtemp(prefix="vkq-interop-")
    for entry in sorted(os.listdir(game_data)):
        src = os.path.join(game_data, entry)
        if not os.path.isdir(src):
            continue
        dst = os.path.join(staging, entry)
        os.makedirs(dst, exist_ok=True)
        for f in sorted(os.listdir(src)):
            _stage_entry(os.path.join(src, f), os.path.join(dst, f))
    return staging


def summarize(path):
    counts = {}
    with open(path, "rb") as f:
        while True:
            hdr = f.read(7)
            if len(hdr) < 7:
                break
            (length,) = struct.unpack("<I", hdr[3:7])
            f.seek(length, os.SEEK_CUR)
            key = (hdr[0], hdr[2])
            counts[key] = counts.get(key, 0) + 1
    return counts


def run_cell(server_exe, client_exe, game_data, cell, host, port, frames, map_name):
    sv_dir = stage(game_data)
    cl_dir = stage(game_data)
    try:
        with open(os.path.join(sv_dir, "harness.cmds"), "w") as f:
            f.write(f"0 sv_protocol {cell}\n")
            f.write(f"0 map {map_name}\n")
        with open(os.path.join(cl_dir, "harness.cmds"), "w") as f:
            f.write(f"0 connect {host}:{port}\n")

        server = subprocess.Popen(
            [os.path.abspath(server_exe), "-dedicated", "-basedir", ".",
             "-port", str(port), "-harnesscmds", "harness.cmds"],
            cwd=sv_dir, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        try:
            time.sleep(3)
            if server.poll() is not None:
                return None, f"server exited early ({server.returncode})"
            client = subprocess.run(
                [os.path.abspath(client_exe), "-headless", "-basedir", ".",
                 "-netcapture", "harness.cap",
                 "-exitafter", str(frames), "-harnesscmds", "harness.cmds"],
                cwd=cl_dir, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                text=True, timeout=600)
            if client.returncode not in (0, 2):
                return None, f"client exited with {client.returncode}:\n" + client.stdout[-1500:]
        finally:
            server.terminate()
            try:
                server.wait(timeout=10)
            except subprocess.TimeoutExpired:
                server.kill()

        out = client.stdout
        m = re.search(r"Using protocol (\S+)", out)
        proto = m.group(1) if m else None
        m = re.search(r"Harness: msgbadread=(\d+)", out)
        if not m:
            return None, "no msgbadread counter in client output:\n" + out[-1500:]
        badread = int(m.group(1))
        cap = os.path.join(cl_dir, "harness.cap")
        if not os.path.isfile(cap) or os.path.getsize(cap) == 0:
            return None, "no capture produced:\n" + out[-1500:]
        counts = summarize(cap)
        return {
            "proto": proto,
            "badread": badread,
            "recv_rel": counts.get((0, 1), 0),
            "recv_unrel": counts.get((0, 2), 0),
            "send_rel": counts.get((1, 1), 0),
            "send_unrel": counts.get((1, 2), 0),
        }, None
    finally:
        shutil.rmtree(sv_dir, ignore_errors=True)
        shutil.rmtree(cl_dir, ignore_errors=True)


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--vkquake-c", required=True)
    p.add_argument("--vkquake-rs", required=True)
    p.add_argument("--game-data", default=os.environ.get("QUAKE_GAME_DATA"))
    p.add_argument("--frames", type=int, default=600)
    p.add_argument("--map", default="start")
    p.add_argument("--cells", nargs="*", default=CELLS)
    p.add_argument("--ipv6", action="store_true",
                   help="also run the FTE+999 cell over [::1] (local-only leg)")
    p.add_argument("--base-port", type=int, default=26100)
    args = p.parse_args()

    if not args.game_data or not os.path.isdir(os.path.join(args.game_data, "id1")):
        sys.exit("error: game data not found; pass --game-data or set QUAKE_GAME_DATA")

    combos = [
        ("C/C", args.vkquake_c, args.vkquake_c),
        ("Csv/Rcl", args.vkquake_c, args.vkquake_rs),
        ("Rsv/Ccl", args.vkquake_rs, args.vkquake_c),
        ("R/R", args.vkquake_rs, args.vkquake_rs),
    ]

    jobs = [(cell, "127.0.0.1") for cell in args.cells]
    if args.ipv6:
        jobs.append(("FTE+999", "[::1]"))

    ok = True
    port = args.base_port
    for cell, host in jobs:
        results = {}
        label = f"{cell}@{host}"
        for name, sv, cl in combos:
            r, err = run_cell(sv, cl, args.game_data, cell, host, port,
                              args.frames, args.map)
            port += 1
            if err:
                print(f"FAIL {label} {name}: {err}")
                ok = False
                continue
            results[name] = r
            expect = EXPECT_PROTO[cell]
            if r["proto"] != expect:
                print(f"FAIL {label} {name}: negotiated {r['proto']!r}, expected {expect!r}")
                ok = False
            # absolute floor: signon reliables must have flowed; unreliable
            # volume is protocol-dependent (a Base-15 idle session emits
            # almost none) so its health is judged against the C/C baseline
            # below, not an absolute number
            if r["recv_rel"] < 2:
                print(f"FAIL {label} {name}: unhealthy traffic {r}")
                ok = False
            print(f"  {label:16s} {name:8s} proto={r['proto']:7s} "
                  f"rel {r['recv_rel']}/{r['send_rel']} "
                  f"unrel {r['recv_unrel']}/{r['send_unrel']} "
                  f"badread={r['badread']}")

        if len(results) == 4:
            base = results["C/C"]
            for name, r in results.items():
                if r["recv_rel"] != base["recv_rel"] or r["send_rel"] != base["send_rel"]:
                    print(f"FAIL {label}: reliable counts differ: {name} "
                          f"{r['recv_rel']}/{r['send_rel']} vs C/C "
                          f"{base['recv_rel']}/{base['send_rel']}")
                    ok = False
                for k in ("recv_unrel", "send_unrel"):
                    lo = base[k] - max(6, base[k] // 10)
                    hi = base[k] + max(6, base[k] // 10)
                    if not lo <= r[k] <= hi:
                        print(f"FAIL {label}: {k} outside noise floor: "
                              f"{name} {r[k]} vs C/C {base[k]}")
                        ok = False
                # msgbadread scales with the number of received messages
                # (the dgrm/parse paths probe optional fields per message),
                # so live +-1-message timing noise shifts the raw counter;
                # the per-session invariant is its offset from the received
                # message count
                delta = r["badread"] - r["recv_rel"] - r["recv_unrel"]
                bdelta = base["badread"] - base["recv_rel"] - base["recv_unrel"]
                if delta != bdelta:
                    print(f"FAIL {label}: msgbadread profile differs: {name} "
                          f"{r['badread']}-{r['recv_rel'] + r['recv_unrel']} "
                          f"vs C/C {base['badread']}-"
                          f"{base['recv_rel'] + base['recv_unrel']}")
                    ok = False

    print("interop_matrix:", "PASS" if ok else "FAIL")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
