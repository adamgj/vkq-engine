#!/usr/bin/env python3
"""Run the differential-verification corpus (Misc/harness/corpus.json).

Modes:
  --generate   write golden hash files to Misc/harness/goldens/<os>-<arch>/
  --check      run and byte-compare against the committed goldens
  --stability  run every entry twice and require identical output
  --compare B  run every entry on this build and on B, require identical output
               (the mixed-vs-C-only gate; needs no goldens, so it works on
               platforms that have none yet)

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


def run_entry(vkquake, entry, game_data, out, extra_args="", sndhash=False):
    cmd = [sys.executable, RUN_DEMO, "--vkquake", vkquake,
           "--game-data", game_data, "--out", out]
    if sndhash:
        # the mixer PCM-hash chain lands next to the demo hash (Phase 4 gate);
        # note the demo hash itself differs from the non-sndhash goldens (the
        # sound engine draws from the shared RNG), so sndhash goldens live in
        # their own <name>.snd / <name>.snd-demo.hash namespace
        extra_args = (extra_args + " " if extra_args else "") + "-sndhash " + out + ".snd"
    if extra_args:
        cmd += ["--extra-args", extra_args]
    if entry.get("demo"):
        cmd += ["--demo", entry["demo"]]
    if entry.get("game"):
        cmd += [f"--game={entry['game']}"]
    if entry.get("fixture_dir"):
        cmd += ["--fixture-dir", os.path.join(ROOT, entry["fixture_dir"])]
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
    mode.add_argument("--compare", metavar="OTHER_VKQUAKE", default=None)
    # engine argv appended to every run. --compare-extra-args differs only for
    # the second binary, so the same build can be compared against itself under
    # different flags -- e.g. the Phase 3 M6 worker sweep,
    #   --compare <same exe> --extra-args "-pinnedworkers 0" \
    #                        --compare-extra-args "-pinnedworkers 0,1,2,3,4,5,6,7"
    # which is what gates the parallel model loaders on order independence.
    p.add_argument("--sndhash", action="store_true",
                   help="also run the software mixer on the deterministic "
                        "harness DMA clock and hash its output (Phase 4); "
                        "goldens are <name>.snd + <name>.snd-demo.hash")
    p.add_argument("--extra-args", default="")
    p.add_argument("--compare-extra-args", default=None,
                   help="engine argv for the --compare/--stability second run "
                        "(defaults to --extra-args)")
    args = p.parse_args()
    args_b = args.extra_args if args.compare_extra_args is None else args.compare_extra_args

    if not args.game_data:
        sys.exit("error: pass --game-data or set QUAKE_GAME_DATA")
    # goldens must describe a canonical run, not one under ad-hoc engine flags
    if args.generate and (args.extra_args or args.compare_extra_args):
        sys.exit("error: --generate does not accept --extra-args/--compare-extra-args")

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

        def outputs(base):
            # the artifacts one run produces: the demo state-hash chain plus,
            # under --sndhash, the mixer PCM-hash chain
            return [base] + ([base + ".snd"] if args.sndhash else [])

        def same(a, b):
            return all(open(x, "rb").read() == open(y, "rb").read()
                       for x, y in zip(outputs(a), outputs(b)))

        if args.sndhash:
            golden = os.path.join(golden_dir, f"{name}.snd-demo.hash")
            golden_alias = {golden: golden, golden + ".snd":
                            os.path.join(golden_dir, f"{name}.snd")}
        else:
            golden = os.path.join(golden_dir, f"{name}.hash")
            golden_alias = {golden: golden}
        if args.generate:
            os.makedirs(golden_dir, exist_ok=True)
            ok = run_entry(exe, entry, data_root, golden, args.extra_args,
                           args.sndhash)
            if ok and args.sndhash:
                os.replace(golden + ".snd", golden_alias[golden + ".snd"])
            print(f"{'generated' if ok else 'FAILED'}: {name}")
            (ran if ok else failed).append(name)
        elif args.check:
            if not all(os.path.isfile(g) for g in golden_alias.values()):
                skipped.append((name, f"no golden for {plat}"))
                continue
            out = tempfile.NamedTemporaryFile(suffix=".hash", delete=False).name
            ok = run_entry(exe, entry, data_root, out, args.extra_args,
                           args.sndhash)
            if not ok:
                print(f"RUN FAILED: {name} (no hash produced)")
                failed.append(name)
            elif all(open(o, "rb").read() == open(golden_alias[g], "rb").read()
                     for o, g in zip(outputs(out), outputs(golden))):
                print(f"ok: {name}")
                ran.append(name)
            else:
                print(f"MISMATCH: {name} ({out} vs {golden})")
                failed.append(name)
        else:  # stability / compare
            a = tempfile.NamedTemporaryFile(suffix=".hash", delete=False).name
            b = tempfile.NamedTemporaryFile(suffix=".hash", delete=False).name
            other = os.path.abspath(args.compare) if args.compare else exe
            ok = (run_entry(exe, entry, data_root, a, args.extra_args, args.sndhash)
                  and run_entry(other, entry, data_root, b, args_b, args.sndhash))
            if not ok:
                print(f"RUN FAILED: {name} (no hash produced)")
                failed.append(name)
            elif same(a, b):
                print(f"{'identical' if args.compare else 'stable'}: {name}")
                ran.append(name)
            else:
                print(f"{'DIFFERS' if args.compare else 'UNSTABLE'}: {name} ({a} vs {b})")
                failed.append(name)

    print(f"\n{len(ran)} ran, {len(skipped)} skipped, {len(failed)} failed")
    for name, reason in skipped:
        print(f"  skipped {name}: {reason}")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
