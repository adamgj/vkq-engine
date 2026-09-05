#!/usr/bin/env python3
"""Windowed renderer differential harness (Rust migration Phase 8, ADR-015).

The headless corpus never executes renderer code, so renderer parity gets its
own corpus (Misc/harness/render_corpus.json) driven through a real window:

  --stability      run every entry N times on one build: the -renderhash chain
                   (cull decisions + draw-call structure) must be identical run
                   to run; screenshot SSIM between runs is reported, and the
                   observed minimum is the basis for the per-entry thresholds
  --compare B      run every entry on this build and on B: identical -renderhash
                   chains, screenshot SSIM >= the entry threshold, and (with
                   --validation) no validation-layer messages on either side
  --timedemo       run the timedemo entries N times (no fixed timestep, no
                   -renderhash) and report the median fps; with --compare the
                   candidate must stay within --timedemo-tolerance of the
                   reference median on the same machine

Screenshots are written by the engine's `screenshot tga` console command and
decoded here (uncompressed 32-bit TGA); SSIM is the usual grayscale
formulation, computed globally and over 8x8 windows, with no third-party
dependencies (the harness stays stdlib-only).

Usage:
  render_corpus.py --vkquake <exe> --out <dir> (--stability | --compare B | --timedemo)
                   [--game-data <dir>] [--entry name] [--tier shareware,...]
                   [--validation] [--runs N] [--width W --height H]
"""

import argparse
import glob
import json
import os
import re
import shutil
import statistics
import struct
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from run_corpus import ROOT, entry_available, platform_key  # noqa: E402
from run_demo import stage_basedir  # noqa: E402

CORPUS = os.path.join(ROOT, "Misc", "harness", "render_corpus.json")
SCREENSHOT_PREFIX = "vkqr-engine"
FPS_RE = re.compile(r"(\d+) frames +([\d.]+) seconds +([\d.]+) fps")
VALIDATION_RE = re.compile(r"Validation (Error|Warning)|VUID-|vkDebug", re.IGNORECASE)


# --- TGA + SSIM ---------------------------------------------------------------

def read_tga_gray(path):
    """Decode an uncompressed true-colour TGA (what Image_WriteTGA emits) to a
    top-down list of luma values."""
    with open(path, "rb") as f:
        data = f.read()
    idlen, cmaptype, imgtype = data[0], data[1], data[2]
    width, height = struct.unpack_from("<HH", data, 12)
    bpp, desc = data[16], data[17]
    if imgtype != 2 or cmaptype != 0 or bpp not in (24, 32):
        raise ValueError(f"{path}: unsupported TGA (type {imgtype}, {bpp} bpp)")
    stride = bpp // 8
    off = 18 + idlen
    rows = []
    for y in range(height):
        row = data[off + y * width * stride: off + (y + 1) * width * stride]
        # stored as B, G, R(, A)
        gray = [(299 * row[i + 2] + 587 * row[i + 1] + 114 * row[i]) / 1000.0
                for i in range(0, width * stride, stride)]
        rows.append(gray)
    if not desc & 0x20:  # bottom-up origin
        rows.reverse()
    return width, height, rows


def _ssim_block(a, b):
    n = len(a)
    ma = sum(a) / n
    mb = sum(b) / n
    va = sum((x - ma) ** 2 for x in a) / max(n - 1, 1)
    vb = sum((x - mb) ** 2 for x in b) / max(n - 1, 1)
    cov = sum((x - ma) * (y - mb) for x, y in zip(a, b)) / max(n - 1, 1)
    c1 = (0.01 * 255) ** 2
    c2 = (0.03 * 255) ** 2
    return ((2 * ma * mb + c1) * (2 * cov + c2)) / ((ma * ma + mb * mb + c1) * (va + vb + c2))


def ssim(img_a, img_b, window=8):
    wa, ha, ra = img_a
    wb, hb, rb = img_b
    if (wa, ha) != (wb, hb):
        raise ValueError(f"screenshot size mismatch {wa}x{ha} vs {wb}x{hb}")
    flat_a = [v for row in ra for v in row]
    flat_b = [v for row in rb for v in row]
    global_ssim = _ssim_block(flat_a, flat_b)
    windows = []
    for y in range(0, ha - window + 1, window):
        for x in range(0, wa - window + 1, window):
            a = [ra[yy][xx] for yy in range(y, y + window) for xx in range(x, x + window)]
            b = [rb[yy][xx] for yy in range(y, y + window) for xx in range(x, x + window)]
            windows.append(_ssim_block(a, b))
    return {"global": global_ssim,
            "mean": sum(windows) / len(windows),
            "min": min(windows)}


# --- engine runs ----------------------------------------------------------------

def run_engine(exe, entry, args, label, out_dir, extra_args, renderhash=True):
    """One windowed run; returns a dict with rc, stdout tail, hash lines,
    screenshot paths (moved under out_dir) and the timedemo fps if printed."""
    staging = tempfile.mkdtemp(prefix="vkq-r-")
    try:
        stage_basedir(args.game_data, staging)
        # con_notifytime -1: the "Wrote <screenshot>" notify line is printed from
        # the asynchronous end-rendering task, so it lands on a
        # wall-clock-dependent frame (and would put a timestamped filename into
        # the next screenshot). 0 is not enough: con_times is a float and
        # realtime a double, so the stored time rounds above realtime about
        # half the time and the line is drawn for one frame; -1 closes the
        # window (Con_NotifyAlpha). Notify lines are therefore never hashed;
        # the console proper (Con_DrawConsole while it is down) still is.
        cmdlines = ["0 con_notifytime -1"]
        if entry.get("timedemo"):
            cmdlines.append(f"0 timedemo {entry['timedemo']}")
        elif entry.get("demo"):
            cmdlines.append(f"0 playdemo {entry['demo']}")
        cmdlines.extend(entry.get("cmds", []))
        with open(os.path.join(staging, "harness.cmds"), "w") as f:
            f.write("\n".join(cmdlines) + "\n")
        # -nomouse: the initial grab delta would rotate the view; -nosound: a
        # duplicate-sound start consumes COM_Rand only when the mixer has not
        # advanced the channel yet, i.e. depending on the real audio clock,
        # which shifts particle lifetimes (sound parity is the sndhash gate)
        cmd = [os.path.abspath(exe), "-basedir", ".", "-window", "-nomouse", "-nosound",
               "-width", str(args.width), "-height", str(args.height),
               "-exitafter", str(entry.get("exitafter", 20000)),
               "-harnesscmds", "harness.cmds", "-condebug"]
        if renderhash:
            cmd += ["-renderhash", "render.hash"]
        if args.validation:
            cmd += ["-validation"]
        if entry.get("game"):
            cmd += [entry["game"]] if entry["game"].startswith("-") else ["-game", entry["game"]]
        if extra_args:
            cmd += extra_args.split()
        try:
            proc = subprocess.run(cmd, cwd=staging, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                  text=True, errors="replace", timeout=args.timeout)
            rc, stdout = proc.returncode, proc.stdout
        except subprocess.TimeoutExpired as e:
            rc, stdout = -1, (e.stdout or b"").decode("utf-8", "replace") if isinstance(e.stdout, bytes) else (e.stdout or "")
            stdout += "\n[render_corpus: timeout]\n"
        # -condebug lands in the (redirected, hermetic) pref dir under the staging basedir
        logs = glob.glob(os.path.join(staging, "**", "qconsole.log"), recursive=True)
        for log in logs:
            with open(log, errors="replace") as f:
                stdout += "\n--- qconsole.log ---\n" + f.read()

        dest = os.path.join(out_dir, entry["name"], label)
        os.makedirs(dest, exist_ok=True)
        with open(os.path.join(dest, "stdout.txt"), "w", errors="replace") as f:
            f.write(stdout)
        hash_lines = None
        staged_hash = os.path.join(staging, "render.hash")
        if renderhash and os.path.isfile(staged_hash):
            with open(staged_hash) as f:
                hash_lines = f.read().splitlines()
            shutil.move(staged_hash, os.path.join(dest, "render.hash"))
        shots = []
        # the date-time in the name sorts chronologically; the index breaks ties
        for src in sorted(glob.glob(os.path.join(staging, "*", SCREENSHOT_PREFIX + "-*.tga"))):
            dst = os.path.join(dest, f"shot-{len(shots):02d}.tga")
            shutil.move(src, dst)
            shots.append(dst)
        fps = None
        m = FPS_RE.search(stdout)
        if m:
            fps = {"frames": int(m.group(1)), "seconds": float(m.group(2)), "fps": float(m.group(3))}
        validation = [ln for ln in stdout.splitlines() if VALIDATION_RE.search(ln)]
        return {"rc": rc, "stdout_tail": stdout[-3000:], "hash": hash_lines, "shots": shots,
                "fps": fps, "validation": validation}
    finally:
        if args.keep:
            print(f"basedir kept at {staging}")
        else:
            shutil.rmtree(staging, ignore_errors=True)


def run_engine_retry(exe, entry, args, label, out_dir, extra_args, renderhash=True, want_shots=0):
    """One retry when a run exits cleanly but yields no screenshots / no timedemo
    line. A screenshot request used to be dropped by a race with the previous
    frame's end-rendering task (fixed in gl_vidsdl.c alongside this script);
    the retry stays as a safety net and is reported. A second miss fails."""
    res = run_engine(exe, entry, args, label, out_dir, extra_args, renderhash)
    usable = res["rc"] == 0 and (res["fps"] is not None if entry.get("timedemo") else len(res["shots"]) >= want_shots)
    if not usable and res["rc"] == 0:
        print(f"{entry['name']}/{label}: no usable output, retrying once")
        dest = os.path.join(out_dir, entry["name"], label)
        try:
            os.replace(os.path.join(dest, "stdout.txt"), os.path.join(dest, "stdout-attempt1.txt"))
        except OSError:
            pass
        res = run_engine(exe, entry, args, label, out_dir, extra_args, renderhash)
        res["retried"] = True
    return res


def compare_hashes(a, b):
    """Return None when identical, else a description of the first divergence."""
    if a is None or b is None:
        return "missing -renderhash output"
    if a == b:
        return None
    for i, (la, lb) in enumerate(zip(a, b)):
        if la != lb:
            return f"line {i + 1}: {la!r} vs {lb!r}"
    return f"length {len(a)} vs {len(b)}"


def entry_threshold(entry, plat, default):
    t = entry.get("ssim", {})
    return t.get(plat, t.get("default", default))


def check_run(name, label, res, want_shots, args, failures):
    ok = True
    if res["rc"] != 0:
        failures.append(f"{name}/{label}: engine exited with {res['rc']}\n{res['stdout_tail']}")
        ok = False
    if args.validation and res["validation"]:
        failures.append(f"{name}/{label}: {len(res['validation'])} validation message(s), first: {res['validation'][0]}")
        ok = False
    if want_shots and len(res["shots"]) != want_shots:
        failures.append(f"{name}/{label}: expected {want_shots} screenshots, got {len(res['shots'])}")
        ok = False
    return ok


# --- modes ------------------------------------------------------------------------

def main():
    p = argparse.ArgumentParser()
    p.add_argument("--vkquake", required=True, help="reference build (the C build in a C-vs-mixed comparison)")
    p.add_argument("--game-data", default=os.environ.get("QUAKE_GAME_DATA"))
    p.add_argument("--out", required=True, help="directory for hashes, screenshots, logs and report.json")
    p.add_argument("--corpus", default=CORPUS)
    p.add_argument("--tier", default=None, help="comma-separated tier filter")
    p.add_argument("--entry", default=None, help="comma-separated entry-name filter")
    mode = p.add_mutually_exclusive_group(required=True)
    mode.add_argument("--stability", action="store_true")
    mode.add_argument("--compare", metavar="OTHER_VKQUAKE", default=None)
    mode.add_argument("--timedemo", action="store_true")
    p.add_argument("--timedemo-compare", metavar="OTHER_VKQUAKE", default=None,
                   help="with --timedemo: candidate build to hold within --timedemo-tolerance of --vkquake")
    p.add_argument("--timedemo-tolerance", type=float, default=0.10)
    p.add_argument("--runs", type=int, default=None, help="runs per entry (stability: 2, timedemo: 3)")
    p.add_argument("--ssim-threshold", type=float, default=0.95, help="fallback when an entry has no threshold for this platform")
    p.add_argument("--ssim-margin", type=float, default=0.01, help="--stability: suggested threshold = observed min - margin")
    p.add_argument("--validation", action="store_true", help="run with -validation and fail on any validation-layer message")
    p.add_argument("--width", type=int, default=640)
    p.add_argument("--height", type=int, default=480)
    p.add_argument("--timeout", type=int, default=600, help="seconds per engine run")
    p.add_argument("--extra-args", default="", help="engine argv appended to every run (use the = form for single -flags)")
    p.add_argument("--compare-extra-args", default=None, help="engine argv for the --compare build only")
    p.add_argument("--keep", action="store_true", help="keep staging basedirs")
    args = p.parse_args()

    if not args.game_data or not os.path.isdir(os.path.join(args.game_data, "id1")):
        sys.exit("error: game data not found; pass --game-data or set QUAKE_GAME_DATA (must contain id1/)")
    with open(args.corpus) as f:
        corpus = json.load(f)
    tiers = set(args.tier.split(",")) if args.tier else None
    names = set(args.entry.split(",")) if args.entry else None
    plat = platform_key()
    args_b = args.extra_args if args.compare_extra_args is None else args.compare_extra_args
    os.makedirs(args.out, exist_ok=True)

    entries = []
    for entry in corpus["entries"]:
        if tiers and entry.get("tier") not in tiers:
            continue
        if names and entry["name"] not in names:
            continue
        is_td = bool(entry.get("timedemo"))
        if is_td != args.timedemo:
            continue
        why = entry_available(entry, args.game_data)
        if why:
            print(f"skip {entry['name']}: {why}")
            continue
        entries.append(entry)
    if not entries:
        sys.exit("error: no runnable entries")

    report = {"platform": plat, "mode": "timedemo" if args.timedemo else ("compare" if args.compare else "stability"),
              "reference": os.path.abspath(args.vkquake), "entries": {}}
    failures = []

    if args.timedemo:
        runs = args.runs or 3
        for entry in entries:
            name = entry["name"]
            rec = {"reference": [], "candidate": []}
            for i in range(runs):
                res = run_engine_retry(args.vkquake, entry, args, f"ref-{i}", args.out, args.extra_args, renderhash=False)
                check_run(name, f"ref-{i}", res, 0, args, failures)
                if res["fps"]:
                    rec["reference"].append(res["fps"]["fps"])
                else:
                    failures.append(f"{name}/ref-{i}: no timedemo result line in output")
                if args.timedemo_compare:
                    res = run_engine_retry(args.timedemo_compare, entry, args, f"cand-{i}", args.out, args_b, renderhash=False)
                    check_run(name, f"cand-{i}", res, 0, args, failures)
                    if res["fps"]:
                        rec["candidate"].append(res["fps"]["fps"])
                    else:
                        failures.append(f"{name}/cand-{i}: no timedemo result line in output")
            if rec["reference"]:
                rec["reference_median"] = statistics.median(rec["reference"])
                print(f"{name}: reference median {rec['reference_median']:.1f} fps over {rec['reference']}")
            if rec["candidate"]:
                rec["candidate_median"] = statistics.median(rec["candidate"])
                floor = rec["reference_median"] * (1.0 - args.timedemo_tolerance)
                print(f"{name}: candidate median {rec['candidate_median']:.1f} fps over {rec['candidate']} (floor {floor:.1f})")
                if rec["candidate_median"] < floor:
                    failures.append(f"{name}: candidate {rec['candidate_median']:.1f} fps below {floor:.1f} "
                                    f"({args.timedemo_tolerance:.0%} under reference {rec['reference_median']:.1f})")
            report["entries"][name] = rec

    elif args.compare:
        report["candidate"] = os.path.abspath(args.compare)
        for entry in entries:
            name = entry["name"]
            want = sum(1 for c in entry.get("cmds", []) if c.split(None, 1)[1].startswith("screenshot"))
            a = run_engine_retry(args.vkquake, entry, args, "ref", args.out, args.extra_args, want_shots=want)
            b = run_engine_retry(args.compare, entry, args, "cand", args.out, args_b, want_shots=want)
            ok = check_run(name, "ref", a, want, args, failures) & check_run(name, "cand", b, want, args, failures)
            rec = {"hash_lines": len(a["hash"] or []), "ssim": []}
            if ok:
                diff = compare_hashes(a["hash"], b["hash"])
                if diff:
                    failures.append(f"{name}: -renderhash differs: {diff}")
                threshold = entry_threshold(entry, plat, args.ssim_threshold)
                for i, (sa, sb) in enumerate(zip(a["shots"], b["shots"])):
                    s = ssim(read_tga_gray(sa), read_tga_gray(sb))
                    rec["ssim"].append(s)
                    print(f"{name} shot {i}: ssim global {s['global']:.4f} mean {s['mean']:.4f} min {s['min']:.4f} (threshold {threshold})")
                    if s["mean"] < threshold:
                        failures.append(f"{name} shot {i}: mean window SSIM {s['mean']:.4f} < {threshold}")
                if not diff:
                    print(f"{name}: -renderhash identical over {rec['hash_lines']} frames")
            report["entries"][name] = rec

    else:  # stability
        runs = args.runs or 2
        for entry in entries:
            name = entry["name"]
            want = sum(1 for c in entry.get("cmds", []) if c.split(None, 1)[1].startswith("screenshot"))
            results = []
            for i in range(runs):
                res = run_engine_retry(args.vkquake, entry, args, f"run-{i}", args.out, args.extra_args, want_shots=want)
                check_run(name, f"run-{i}", res, want, args, failures)
                results.append(res)
            rec = {"hash_lines": len(results[0]["hash"] or []), "ssim": [], "suggested_threshold": None}
            if all(r["rc"] == 0 for r in results):
                for i in range(1, runs):
                    diff = compare_hashes(results[0]["hash"], results[i]["hash"])
                    if diff:
                        failures.append(f"{name}: -renderhash differs between run 0 and run {i}: {diff}")
                mins = []
                for i in range(1, runs):
                    for j, (sa, sb) in enumerate(zip(results[0]["shots"], results[i]["shots"])):
                        s = ssim(read_tga_gray(sa), read_tga_gray(sb))
                        rec["ssim"].append(s)
                        mins.append(s["mean"])
                        print(f"{name} shot {j} run 0 vs {i}: ssim global {s['global']:.4f} mean {s['mean']:.4f} min {s['min']:.4f}")
                if mins:
                    thr = entry_threshold(entry, plat, args.ssim_threshold)
                    if min(mins) < thr:
                        failures.append(f"{name}: run-to-run SSIM {min(mins):.4f} below the {plat} threshold {thr}")
                    rec["suggested_threshold"] = round(min(mins) - args.ssim_margin, 4)
                    print(f"{name}: suggested {plat} threshold {rec['suggested_threshold']} "
                          f"(observed min mean-window SSIM {min(mins):.4f}, entry has {entry_threshold(entry, plat, None)})")
                print(f"{name}: -renderhash stable over {rec['hash_lines']} frames x {runs} runs")
            report["entries"][name] = rec

    with open(os.path.join(args.out, "report.json"), "w") as f:
        json.dump(report, f, indent=2)
    if failures:
        print("\nFAILURES:")
        for msg in failures:
            print(" - " + msg)
        sys.exit(1)
    print("\nrender corpus OK")


if __name__ == "__main__":
    main()
