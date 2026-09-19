#!/usr/bin/env python3
"""Which Quake/*_glue.c TUs can be deleted next? (Phase 10 M6, plan D1)

Reads the defined external symbols of every compiled glue object under a
configured Meson build directory (llvm-nm, falling back to nm), then greps
the remaining Quake/*.c for each symbol outside its defining TU. A glue TU
with no C reader for any of its symbols is deletable once its Rust-side
readers are re-homed (the count of references under rust/*/src is printed
next to each symbol so the re-homing work is visible too).

The check is textual (word match on the symbol name): a reader hidden behind
a macro or token paste is missed, which the link step catches immediately
because the strata are small. Symbols that a header declares are counted
against the TU that includes the header only if the TU also spells the name.

Usage:
  python3 scripts/glue_deps.py <build-dir>
  python3 scripts/glue_deps.py <build-dir> --assume-deleted a_glue.c,b_glue.c
      # simulate a stratum: treat those TUs as gone when looking for readers
  python3 scripts/glue_deps.py <build-dir> --mem
      # plan D9: list Mem_Alloc/Mem_Free sites in the remnant (non-glue) TUs
  python3 scripts/glue_deps.py <build-dir> --verbose
      # also print the symbols that still have C readers, with the reader TUs
"""

import argparse
import glob
import os
import re
import shutil
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OBJ_GLOB = "**/Quake_*_glue.c.o*"
IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")


def read(path):
    with open(path, encoding="utf-8", errors="replace", newline="") as f:
        return f.read().replace("\r\n", "\n")


def strip_comments(text):
    text = re.sub(r"/\*.*?\*/", " ", text, flags=re.S)
    return re.sub(r"//[^\n]*", " ", text)


def nm_tool():
    for name in ("llvm-nm", "nm"):
        path = shutil.which(name)
        if path:
            return path
    for cand in ("C:/Program Files/LLVM/bin/llvm-nm.exe",):
        if os.path.exists(cand):
            return cand
    sys.exit("error: neither llvm-nm nor nm on PATH")


def defined_symbols(nm, obj):
    out = subprocess.run([nm, "--defined-only", "--extern-only", obj], capture_output=True, text=True, check=True).stdout
    syms = set()
    for line in out.splitlines():
        parts = line.split()
        if len(parts) < 2:
            continue
        name = parts[-1]
        # MSVC/COFF decorates cdecl symbols on x86 only; x64 is undecorated.
        # Drop MSVC C++-style and compiler-internal names either way.
        if name.startswith(("?", ".", "$", "__real@", "__xmm@")):
            continue
        syms.add(name)
    return syms


def glue_objects(build_dir):
    objs = {}
    for obj in glob.glob(os.path.join(build_dir, OBJ_GLOB), recursive=True):
        base = os.path.basename(obj)
        m = re.match(r"Quake_(.+_glue)\.c\.o", base)
        if m:
            objs[m.group(1) + ".c"] = obj
    return objs


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("build_dir")
    ap.add_argument("--assume-deleted", default="", metavar="A_glue.c,B_glue.c")
    ap.add_argument("--mem", action="store_true")
    ap.add_argument("--verbose", action="store_true")
    args = ap.parse_args()

    assumed = {s.strip() for s in args.assume_deleted.split(",") if s.strip()}
    c_files = sorted(glob.glob(os.path.join(ROOT, "Quake", "*.c")))
    c_text = {}
    for path in c_files:
        base = os.path.basename(path)
        if base in assumed:
            continue
        c_text[base] = strip_comments(read(path))
    c_idents = {base: set(IDENT.findall(text)) for base, text in c_text.items()}

    rust_files = glob.glob(os.path.join(ROOT, "rust", "*", "src", "**", "*.rs"), recursive=True)
    rust_idents = {}
    for path in rust_files:
        for ident in set(IDENT.findall(read(path))):
            rust_idents[ident] = rust_idents.get(ident, 0) + 1

    objs = glue_objects(args.build_dir)
    if not objs:
        sys.exit("error: no %s under %s (configure and build first)" % (OBJ_GLOB, args.build_dir))
    nm = nm_tool()

    deletable = []
    for tu in sorted(objs):
        if tu in assumed:
            continue
        syms = defined_symbols(nm, objs[tu])
        held = {}
        for sym in sorted(syms):
            readers = sorted(b for b, ids in c_idents.items() if b != tu and sym in ids)
            if readers:
                held[sym] = readers
        rust_refs = sum(rust_idents.get(sym, 0) for sym in syms)
        if not held:
            deletable.append(tu)
            print("%-28s deletable  (%d symbols, %d rust refs)" % (tu, len(syms), rust_refs))
        else:
            print("%-28s held by C  (%d/%d symbols read, %d rust refs)" % (tu, len(held), len(syms), rust_refs))
            if args.verbose:
                for sym, readers in held.items():
                    print("    %-40s %s" % (sym, " ".join(readers)))
    print()
    print("deletable now (%d): %s" % (len(deletable), " ".join(deletable) or "-"))
    if assumed:
        print("assumed deleted (%d): %s" % (len(assumed), " ".join(sorted(assumed))))

    if args.mem:
        print()
        print("Mem_Alloc/Mem_Free sites in non-glue TUs (plan D9):")
        mem_re = re.compile(r"\bMem_(?:Alloc|Free|Realloc)\b")
        for base in sorted(c_text):
            if base.endswith("_glue.c"):
                continue
            n = len(mem_re.findall(c_text[base]))
            if n:
                print("    %-24s %d" % (base, n))
    return 0


if __name__ == "__main__":
    sys.exit(main())
