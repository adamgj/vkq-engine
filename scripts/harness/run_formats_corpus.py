#!/usr/bin/env python3
"""Phase 3 formats-corpus gate (D9): drive every model/image asset of one or
more game directories through both the c_ref C loaders and the Rust ports
in-process (quake-ctest's `formats_corpus` binary) and emit a per-asset hash
manifest. Parity is asserted inside the binary; a divergence aborts with a
line-level diff. Per ADR-019 only hashes are written — no game data leaves
the machine.

Usage:
  run_formats_corpus.py [--game-data <dir>] [--gamedir id1 --gamedir hipnotic ...]
                        [--out <manifest>] [--profile release]

--game-data defaults to $QUAKE_GAME_DATA. Each --gamedir is mounted the way
the engine would (loose files plus pak0.pak, pak1.pak, ... in priority
order); the asset list is discovered here (loose files and pak directory
entries) and resolution happens inside the binary through each side's own
filesystem. A gamedir may contain a slash (e.g. rerelease/id1), in which
case the *game-data root* is mounted as the base dir, exactly like the
env-gated md5 differential does.
"""

import argparse
import os
import struct
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
EXTS = (".bsp", ".mdl", ".spr", ".md3", ".md5mesh", ".pcx", ".lmp")


def pak_entries(path):
    """Names in a PACK file's directory."""
    with open(path, "rb") as f:
        header = f.read(12)
        if len(header) < 12 or header[:4] != b"PACK":
            return
        dirofs, dirlen = struct.unpack("<ii", header[4:12])
        f.seek(dirofs)
        directory = f.read(dirlen)
    for i in range(len(directory) // 64):
        raw = directory[i * 64 : i * 64 + 56]
        name = raw.split(b"\0", 1)[0].decode("latin-1")
        if name:
            yield name


def discover(base, gamedir):
    """Asset names (fs-relative) for one mounted gamedir."""
    names = set()
    gdpath = os.path.join(base, *gamedir.split("/"))
    if not os.path.isdir(gdpath):
        print(f"warning: {gdpath} does not exist, skipping", file=sys.stderr)
        return names
    for dirpath, _, files in os.walk(gdpath):
        rel = os.path.relpath(dirpath, gdpath)
        for fn in files:
            low = fn.lower()
            if low.endswith(".pak"):
                for entry in pak_entries(os.path.join(dirpath, fn)):
                    if entry.lower().endswith(EXTS):
                        names.add(entry)
            elif low.endswith(EXTS):
                name = fn if rel == "." else f"{rel}/{fn}".replace(os.sep, "/")
                names.add(name)
    return names


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--game-data", default=os.environ.get("QUAKE_GAME_DATA"))
    ap.add_argument("--gamedir", action="append", default=None)
    ap.add_argument("--out", default=None, help="manifest path (default stdout)")
    ap.add_argument("--profile", default="release")
    args = ap.parse_args()

    if not args.game_data:
        sys.exit("--game-data or QUAKE_GAME_DATA required")
    gamedirs = args.gamedir or ["id1"]

    out = open(args.out, "w") if args.out else sys.stdout
    failed = False
    for gd in gamedirs:
        names = sorted(discover(args.game_data, gd))
        print(f"# gamedir {gd}: {len(names)} assets", file=out)
        if not names:
            # a gate that silently verified nothing must not go green: an
            # empty discovery is the signature of a broken layout/fetch
            print(f"ERROR: no assets discovered under {gd}", file=sys.stderr)
            failed = True
            continue
        cmd = [
            "cargo", "run", "-p", "quake-ctest", "--locked", "--bin", "formats_corpus",
        ]
        if args.profile == "release":
            cmd.insert(2, "--release")
        # COM_ResetGameDirectories filters the literal "id1" (the engine
        # mounts that one from COM_InitFilesystem, which the ctest harness
        # does not run), so mount from one directory up with the game-data
        # root's basename folded into the gamedir — the same trick the
        # env-gated md5 differential uses for rerelease/id1.
        root = os.path.abspath(args.game_data)
        base, mount_gd = os.path.dirname(root), f"{os.path.basename(root)}/{gd}"
        cmd += ["--", "--base", base, "--gamedir", mount_gd]
        proc = subprocess.run(
            cmd,
            cwd=os.path.join(ROOT, "rust"),
            input="\n".join(names),
            capture_output=True,
            text=True,
        )
        out.write(proc.stdout)
        if proc.returncode != 0:
            failed = True
            print(f"FAILED gamedir {gd} (exit {proc.returncode})", file=sys.stderr)
            sys.stderr.write(proc.stderr[-8000:])
    if out is not sys.stdout:
        out.close()
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
