#!/usr/bin/env python3
"""Structural diff of two -netcapture files (Rust migration Phase 5, ADR-019
gate 4).

Live two-process sessions are timing-nondeterministic (resend cadence, server
frame pacing), so raw byte-diffing two independently captured sessions is not
a meaningful gate. What IS deterministic at the NET_* funnel level, for the
same map and command script:

  * the concatenated per-direction *reliable* message stream: reliable
    delivery is ordered and duplicate-free, and the signon conversation
    (serverinfo, model/sound lists, baselines, lightstyles) depends only on
    the map content -- so the streams from two builds must agree byte-for-byte
    over the signon prefix, diverging (if at all) only in the live-game tail
    where entity timing enters;
  * per-(direction, kind) record counts, up to run-length differences.

Checks (all must pass):
  1. Both captures parse cleanly (no truncated record).
  2. Per direction, the concatenated reliable payload streams share a common
     prefix of at least --min-reliable-prefix bytes (or the full shorter
     stream, if it is shorter than that); any mismatch inside that window is
     reported with its offset.
  3. Per (direction, kind), record counts agree within --count-tolerance
     relative difference.

Exit 0 on pass, 1 on mismatch, 2 on usage/parse errors.

Usage:
  capture_diff.py A.cap B.cap [--min-reliable-prefix 16384]
                              [--count-tolerance 0.5] [-v]
Self-check mode (parser sanity, used by the CI smoke step):
  capture_diff.py A.cap A.cap
"""

import argparse
import os
import struct
import sys

DIR_NAMES = {0: "recv", 1: "send"}
KIND_NAMES = {0: "unknown", 1: "reliable", 2: "unreliable"}


def parse_capture(path):
    """Returns a list of (direction, driver, kind, payload) records."""
    records = []
    size = os.path.getsize(path)
    with open(path, "rb") as f:
        pos = 0
        while pos < size:
            hdr = f.read(7)
            if len(hdr) < 7:
                sys.exit(f"error: {path}: truncated record header at {pos}")
            direction, driver, kind = hdr[0], hdr[1], hdr[2]
            (length,) = struct.unpack("<I", hdr[3:7])
            if pos + 7 + length > size:
                sys.exit(f"error: {path}: truncated payload at {pos} (len {length})")
            payload = f.read(length)
            records.append((direction, driver, kind, payload))
            pos += 7 + length
    return records


def reliable_stream(records, direction):
    return b"".join(p for d, _drv, k, p in records if d == direction and k == 1)


def counts(records):
    c = {}
    for d, _drv, k, _p in records:
        c[(d, k)] = c.get((d, k), 0) + 1
    return c


def main():
    p = argparse.ArgumentParser()
    p.add_argument("cap_a")
    p.add_argument("cap_b")
    p.add_argument("--min-reliable-prefix", type=int, default=16384,
                   help="bytes of per-direction reliable stream that must "
                        "match exactly (clamped to the shorter stream)")
    p.add_argument("--min-window", type=int, default=1024,
                   help="fail if the calibrated gate window drops below this "
                        "many bytes: a degenerate early reference divergence "
                        "must not silently reduce the gate to a no-op")
    p.add_argument("--window-from", metavar="CAP",
                   help="calibrate the gate window per direction to the "
                        "common reliable prefix of cap_a and this capture "
                        "(a second run of the SAME build): live sessions "
                        "carry a time-bearing message near the signon tail, "
                        "so the honest gate is 'the compared build matches "
                        "at least as far as the reference build matches "
                        "itself'")
    p.add_argument("--count-tolerance", type=float, default=0.5,
                   help="max relative difference in per-(direction,kind) "
                        "record counts")
    p.add_argument("-v", "--verbose", action="store_true")
    args = p.parse_args()

    rec_a = parse_capture(args.cap_a)
    rec_b = parse_capture(args.cap_b)
    rec_w = parse_capture(args.window_from) if args.window_from else None
    ok = True

    for direction in (0, 1):
        sa = reliable_stream(rec_a, direction)
        sb = reliable_stream(rec_b, direction)
        window = min(len(sa), len(sb), args.min_reliable_prefix)
        if rec_w is not None:
            sw = reliable_stream(rec_w, direction)
            noise_floor = next(
                (i for i in range(min(len(sa), len(sw))) if sa[i] != sw[i]),
                min(len(sa), len(sw)))
            if noise_floor < window:
                print(f"note: {DIR_NAMES[direction]} window calibrated to "
                      f"{noise_floor} bytes (reference build's own "
                      f"run-to-run divergence point)")
                window = noise_floor
            # only gate the floor where a stream exists at all: the send
            # direction of a short session can be legitimately tiny
            if window < args.min_window and min(len(sa), len(sb)) >= args.min_window:
                print(f"FAIL: {DIR_NAMES[direction]} gate window degenerate "
                      f"({window} < --min-window {args.min_window}): the "
                      f"reference captures diverge too early to gate anything")
                ok = False
                continue
        if (len(sa) == 0) != (len(sb) == 0):
            print(f"FAIL: {DIR_NAMES[direction]} reliable stream present in "
                  f"only one capture ({len(sa)} vs {len(sb)} bytes)")
            ok = False
            continue
        if sa[:window] != sb[:window]:
            mismatch = next(i for i in range(window) if sa[i] != sb[i])
            print(f"FAIL: {DIR_NAMES[direction]} reliable streams diverge at "
                  f"byte {mismatch} (within the {window}-byte gate window): "
                  f"{sa[mismatch]:#04x} vs {sb[mismatch]:#04x}")
            ok = False
        elif args.verbose:
            print(f"ok: {DIR_NAMES[direction]} reliable prefix identical over "
                  f"{window} bytes (streams: {len(sa)} vs {len(sb)} bytes)")

    ca, cb = counts(rec_a), counts(rec_b)
    for key in sorted(set(ca) | set(cb)):
        na, nb = ca.get(key, 0), cb.get(key, 0)
        rel = abs(na - nb) / max(na, nb, 1)
        label = f"{DIR_NAMES.get(key[0], key[0])}/{KIND_NAMES.get(key[1], key[1])}"
        if rel > args.count_tolerance:
            print(f"FAIL: {label} record counts differ beyond tolerance: "
                  f"{na} vs {nb} (rel {rel:.2f} > {args.count_tolerance})")
            ok = False
        elif args.verbose:
            print(f"ok: {label} counts {na} vs {nb}")

    if ok:
        print(f"capture_diff: PASS ({len(rec_a)} vs {len(rec_b)} records)")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
