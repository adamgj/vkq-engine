#!/usr/bin/env python3
"""Run the differential-verification corpus (Misc/harness/corpus.json).

Modes:
  --generate   write golden hash files to Misc/harness/goldens/<os>-<arch>/
  --check      run and byte-compare against the committed goldens
  --stability  run every entry twice and require identical output

Entries whose required data (checksummed files or mod dirs) is absent are
skipped with a warning, so the same corpus file serves the CI shareware tier
and full local installs.

Usage:
  run_corpus.py --vkquake <exe> (--generate | --check | --stability)
                [--game-data <dir>] [--tier shareware,registered,...]
"""

import argparse
import hashlib
import json
import os
import platform
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CORPUS = os.path.join(ROOT, "Misc", "harness", "corpus.json")
GOLDENS = os.path.join(ROOT, "Misc", "harness", "goldens")
RUN_DEMO = os.path.join(ROOT, "scripts", "harness", "run_demo.py")


def platform_key():
    system = platform.system().lower()
    machine = platform.machine().lower()
    machine = {"amd64": "x86_64", "aarch64": "arm64"}.get(machine, machine)
    return f"{system}-{machine}"


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def find_file_ci(base, relpath):
    """Case-insensitive lookup of relpath under base (pak filename case varies)."""
    cur = base
    for part in relpath.split("/"):
        if not os.path.isdir(cur):
            return None
        match = next((e for e in os.listdir(cur) if e.lower() == part.lower()), None)
        if match is None:
            return None
        cur = os.path.join(cur, match)
    return cur


def entry_available(entry, game_data):
    for req in entry.get("requires", []):
        path = find_file_ci(game_data, req["path"])
        if not path:
            return f"missing {req['path']}"
        if sha256(path) != req["sha256"]:
            return f"checksum mismatch for {req['path']} (different data version)"
    if "requires_dir" in entry:
        if not find_file_ci(game_data, entry["requires_dir"]):
            return f"missing mod dir {entry['requires_dir']}"
    return None


def run_entry(vkquake, entry, game_data, out):
    cmd = [sys.executable, RUN_DEMO, "--vkquake", vkquake,
           "--game-data", game_data, "--out", out]
    if entry.get("demo"):
        cmd += ["--demo", entry["demo"]]
    if entry.get("game"):
        cmd += [f"--game={entry['game']}"]
    if entry.get("exitafter"):
        cmd += ["--exitafter", str(entry["exitafter"])]
    if entry.get("cmds"):
        f = tempfile.NamedTemporaryFile("w", suffix=".cmds", delete=False)
        f.write("\n".join(entry["cmds"]) + "\n")
        f.close()
        cmd += ["--cmds", f.name]
    proc = subprocess.run(cmd)
    return proc.returncode == 0


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--vkquake", required=True)
    p.add_argument("--game-data", default=os.environ.get("QUAKE_GAME_DATA"))
    p.add_argument("--tier", default=None, help="comma-separated tier filter")
    mode = p.add_mutually_exclusive_group(required=True)
    mode.add_argument("--generate", action="store_true")
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--stability", action="store_true")
    args = p.parse_args()

    if not args.game_data:
        sys.exit("error: pass --game-data or set QUAKE_GAME_DATA")

    with open(CORPUS) as f:
        corpus = json.load(f)
    tiers = set(args.tier.split(",")) if args.tier else None

    plat = platform_key()
    golden_dir = os.path.join(GOLDENS, plat)
    exe = os.path.abspath(args.vkquake)

    ran, skipped, failed = [], [], []
    for entry in corpus["entries"]:
        name = entry["name"]
        if tiers and entry["tier"] not in tiers:
            continue
        data_root = args.game_data
        if entry.get("data_subdir"):
            data_root = os.path.join(args.game_data, entry["data_subdir"])
            if not os.path.isdir(data_root):
                skipped.append((name, f"missing data dir {entry['data_subdir']}"))
                continue
        reason = entry_available(entry, data_root)
        if reason:
            skipped.append((name, reason))
            continue

        golden = os.path.join(golden_dir, f"{name}.hash")
        if args.generate:
            os.makedirs(golden_dir, exist_ok=True)
            ok = run_entry(exe, entry, data_root, golden)
            print(f"{'generated' if ok else 'FAILED'}: {name}")
            (ran if ok else failed).append(name)
        elif args.check:
            if not os.path.isfile(golden):
                skipped.append((name, f"no golden for {plat}"))
                continue
            out = tempfile.NamedTemporaryFile(suffix=".hash", delete=False).name
            ok = run_entry(exe, entry, data_root, out)
            if ok and open(out, "rb").read() == open(golden, "rb").read():
                print(f"ok: {name}")
                ran.append(name)
            else:
                print(f"MISMATCH: {name} ({out} vs {golden})")
                failed.append(name)
        else:  # stability
            a = tempfile.NamedTemporaryFile(suffix=".hash", delete=False).name
            b = tempfile.NamedTemporaryFile(suffix=".hash", delete=False).name
            ok = run_entry(exe, entry, data_root, a) and run_entry(exe, entry, data_root, b)
            if ok and open(a, "rb").read() == open(b, "rb").read():
                print(f"stable: {name}")
                ran.append(name)
            else:
                print(f"UNSTABLE: {name}")
                failed.append(name)

    print(f"\n{len(ran)} ran, {len(skipped)} skipped, {len(failed)} failed")
    for name, reason in skipped:
        print(f"  skipped {name}: {reason}")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
