#!/usr/bin/env python3
"""C remnant inventory (Rust migration Phase 10 M1, ADR-002).

Classifies every C translation unit and vendored native library in the
tree by an explicit rule table and writes
docs/rust-migration/c-remnant-inventory.md. The table is the decision; the
script only measures. A file with no rule fails ``--check`` so a new C file
has to be classified before it lands.

Since the Phase 9 deletion PR there is one engine build; the C originals of
the ported modules live at tag c-reference/final (ADR-019) and the ctest
oracle copies under rust/quake-ctest/csrc/, neither of which is inventoried
here.

Categories:

  glue      Quake/*_glue.c -- Host_Guard trampolines and C-owned globals for
            a ported module (ADR-007/ADR-009). Each goes when its module's
            last C caller goes (Phase 10 M6).
  shared    engine C not yet ported; the Phase 10 M8/M9 port list.
  codec     audio decoder bridges kept as C (ADR-014).
  vendored  third-party C reached through #include from a host TU (ADR-002).
  harness   differential-verification harness TUs (ADR-019).

Usage:
  python3 scripts/c_remnant_inventory.py            # regenerate the doc
  python3 scripts/c_remnant_inventory.py --check    # CI: stale classification or
                                                    # unclassified file -> 1
  python3 scripts/c_remnant_inventory.py --ninja build.ninja
      # cross-check the table against a configured Meson build: every TU it
      # compiles must be in the table, and every table entry the platform
      # builds must be compiled
"""
import argparse
import glob
import os
import re
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), os.pardir))
DOC = "docs/rust-migration/c-remnant-inventory.md"
MESON = "meson.build"

# --- rule table --------------------------------------------------------------
# basename -> note. Files not listed here and not matching GLUE_RE fail the
# run. Platform-conditional TUs are listed for every platform; the --ninja
# cross-check only sees one platform's build at a time.

SHARED = {
    "cd_null.c": "CD stub (no port planned; trivially portable)",
    "common.c": "non-filesystem half of common.c (COM_*, string/parse helpers, cmdline)",
    "gl_model.c": "model loading orchestration; format parsers ported in Phase 3",
    "image.c": "PNG writer host TU: includes lodepng.c (ADR-012 encoder keep)",
    "image_stb.c": "stb_image host TU: in-memory decode fallback/oracle (Phase 3 M8)",
    "mem.c": "mimalloc host TU: includes mimalloc/static.c (ADR-013)",
    "model_parse.c": "scratch globals and Mod_SetExtraFlags around the ported loaders",
    "net_bsd.c": "BSD socket driver vtable slots (unix)",
    "net_loop.c": "loopback driver",
    "net_main.c": "driver dispatch and console commands over the Rust drivers",
    "net_win.c": "Winsock driver vtable slots (Windows)",
    "palette.c": "palette/colormap tables",
    "pr_cmds.c": "builtin table; ported builtins dispatch to Rust",
    "pr_edict.c": "edict orchestration around the ported arena/parse/save pieces",
    "pr_ext.c": "QSS extension builtins (largest remaining engine TU)",
    "pr_trace.c": "progs VM trace oracle hooks (ADR-019)",
    "q_thread_sdl.c": "SDL thread primitives",
    "snd_sdl.c": "SDL2 audio backend (ADR-017); ported at Phase 10 M9",
    "steam_api.c": "Steam API stub",
}

CODEC = {
    "snd_flac.c": "libFLAC bridge",
    "snd_mp3.c": "libmad bridge (-Dmp3_lib=mad)",
    "snd_mpg123.c": "libmpg123 bridge (-Dmp3_lib=mpg123)",
    "snd_opus.c": "opusfile bridge",
    "snd_vorbis.c": "libvorbisfile / tremor bridge",
}

HARNESS = {
    "harness.c": "state-hash / savegame / trace harness (ADR-019)",
    "harness_render.c": "render harness (Phase 8)",
}

# vendored native libraries: path (relative to Quake/) -> (host TUs, note)
VENDORED = {
    "mimalloc/": ("mem.c", "allocator (ADR-013; decision at Phase 10 M9)"),
    "lodepng.c": ("image.c", "PNG encoder (ADR-012 keep)"),
    "lodepng.h": ("image.c", ""),
    "stb_image.h": ("image_stb.c", "decode fallback for formats quake-image does not decode"),
    "stb_image_resize.h": ("gl_texmgr_glue.c", "mipmap resize"),
    "stb_image_write.h": ("image.c", "TGA/JPEG writer"),
}

GLUE_RE = re.compile(r"_glue\.c$")
MESON_REF_RE = re.compile(r"'(Quake/[A-Za-z0-9_]+\.c)'")

# the categories the engine build compiles (vendored code enters through its
# host TU's #include)
COMPILED = {"glue", "shared", "codec", "harness"}

CATEGORY_ORDER = ["glue", "shared", "codec", "vendored", "harness"]


def read(path):
    with open(path, encoding="utf-8", errors="replace", newline="") as f:
        return f.read().replace("\r\n", "\n")


def count_lines(path):
    return read(path).count("\n")


def dir_lines(path):
    total = 0
    n = 0
    for p in glob.glob(os.path.join(path, "**", "*"), recursive=True):
        if os.path.isfile(p) and p.endswith((".c", ".h")):
            total += count_lines(p)
            n += 1
    return n, total


def classify(rel):
    """rel is 'Quake/x.c'."""
    base = os.path.basename(rel)
    if base in VENDORED:
        return "vendored", VENDORED[base][1]
    if GLUE_RE.search(base):
        return "glue", ""
    for cat, table in (("shared", SHARED), ("codec", CODEC), ("harness", HARNESS)):
        if base in table:
            return cat, table[base]
    return None, ""


def scan():
    rows = []  # (category, rel, lines, note)
    errors = []
    files = sorted(glob.glob(os.path.join(ROOT, "Quake", "*.c")))
    refs = set(MESON_REF_RE.findall(read(os.path.join(ROOT, MESON))))
    for path in files:
        rel = os.path.relpath(path, ROOT).replace(os.sep, "/")
        cat, note = classify(rel)
        if cat is None:
            errors.append("%s: no classification rule" % rel)
            cat = "UNCLASSIFIED"
        elif cat == "vendored":
            if rel in refs:
                errors.append("%s: classified %s but referenced by meson.build" % (rel, cat))
        elif rel not in refs:
            errors.append("%s: classified %s but not referenced by meson.build" % (rel, cat))
        rows.append((cat, rel, count_lines(path), note))
    for name in sorted(SHARED) + sorted(CODEC) + sorted(HARNESS):
        if not os.path.exists(os.path.join(ROOT, "Quake", name)):
            errors.append("Quake/%s: in the rule table but not in the tree" % name)
    for ref in sorted(refs):
        if not os.path.exists(os.path.join(ROOT, ref)):
            errors.append("%s: referenced by meson.build but not in the tree" % ref)
    vendored = []  # (rel, files, lines, host, note)
    for name, (host, note) in VENDORED.items():
        path = os.path.join(ROOT, "Quake", name)
        if name.endswith("/"):
            if not os.path.isdir(path):
                errors.append("Quake/%s: vendored directory missing" % name)
                continue
            n, lines = dir_lines(path)
        else:
            if not os.path.exists(path):
                errors.append("Quake/%s: vendored file missing" % name)
                continue
            n, lines = 1, count_lines(path)
        vendored.append(("Quake/" + name, n, lines, host, note))
    return rows, vendored, errors


def md_code(s):
    return "`" + s.replace("`", "'") + "`"


def mask_counts(text):
    """Blank the `Lines` column of every table; every other cell (file
    counts, notes) is compared verbatim."""
    out = []
    col = None
    for line in text.splitlines():
        if not line.startswith("| "):
            col = None
        elif col is None:
            cells = [c.strip() for c in line.split("|")]
            col = cells.index("Lines") if "Lines" in cells else -1
        elif col >= 0 and not line.startswith("| ---"):
            cells = line.split("|")
            cells[col] = " N "
            line = "|".join(cells)
        out.append(line)
    return "\n".join(out)


def render(rows, vendored):
    by_cat = {c: [] for c in CATEGORY_ORDER}
    for r in rows:
        if r[0] != "vendored":
            by_cat.setdefault(r[0], []).append(r)
    # an UNCLASSIFIED TU already fails the run; render it too so a local
    # regen shows which file it was
    cats = CATEGORY_ORDER + [c for c in by_cat if c not in CATEGORY_ORDER]
    out = []
    w = out.append
    w("# C remnant inventory")
    w("")
    w("<!-- generated by scripts/c_remnant_inventory.py; do not edit by hand -->")
    w("")
    w("Rust migration Phase 10 M1 ([ADR-002](adr/ADR-002-c-not-cpp-fallback.md)).")
    w("Every C/Objective-C translation unit and vendored native library in the")
    w("tree, classified by the rule table in `scripts/c_remnant_inventory.py`.")
    w("Regenerate with `python3 scripts/c_remnant_inventory.py`; `--check` fails")
    w("when the classification is stale, a file has no rule, or a rule disagrees")
    w("with `meson.build` (the line counts are informational and not compared,")
    w("so a C-only edit does not fail CI). `--ninja <build.ninja>` cross-checks")
    w("a configured build's TU list against the table.")
    w("")
    w("This is the **post-deletion** cut (Phase 9 deletion PR): the C originals")
    w("of the ported modules live at tag `c-reference/final` and, for the ctest")
    w("differentials, under `rust/quake-ctest/csrc/`; neither is inventoried")
    w("here. The ADR-002 Phase-10 appendix keys off the `shared`, `codec` and")
    w("`vendored` sections; `glue` is the Phase 10 M6 removal list and `shared`")
    w("the M8/M9 port list.")
    w("")
    w("## Summary")
    w("")
    w("| Category | Files | Lines | Compiled by | Disposition |")
    w("| --- | ---: | ---: | --- | --- |")
    disp = {
        "glue": ("engine build", "removed TU by TU as each module's C callers go (Phase 10 M6)"),
        "shared": ("engine build", "port at Phase 10 M8 (`mem.c`, `snd_sdl.c` at M9)"),
        "codec": ("engine build", "keep: ADR-014 remnant (Symphonia closed by ADR-003)"),
        "vendored": ("via host TU", "keep while the host TU exists (ADR-002)"),
        "harness": ("engine build", "harness stays until its Rust twin exists (ADR-019)"),
        "UNCLASSIFIED": ("?", "no classification rule: add one to `classify`"),
    }
    for cat in cats:
        if cat == "vendored":
            n = len(vendored)
            lines = sum(v[2] for v in vendored)
        else:
            n = len(by_cat[cat])
            lines = sum(r[2] for r in by_cat[cat])
        c, d = disp[cat]
        w("| %s | %d | %d | %s | %s |" % (cat, n, lines, c, d))
    w("")
    for cat in cats:
        w("## %s" % cat)
        w("")
        if cat == "vendored":
            w("| Path | Files | Lines | Host TU | Note |")
            w("| --- | ---: | ---: | --- | --- |")
            for rel, n, lines, host, note in vendored:
                w("| %s | %d | %d | %s | %s |" % (md_code(rel), n, lines, md_code(host), note))
            w("")
            continue
        w("| File | Lines | Note |")
        w("| --- | ---: | --- |")
        for _, rel, lines, note in sorted(by_cat[cat], key=lambda r: r[1]):
            w("| %s | %d | %s |" % (md_code(rel), lines, note))
        w("")
    return "\n".join(out)



def ninja_check(rows, path):
    text = read(path)
    tus = set()
    for m in re.finditer(r"^build [^:]*?\.(?:o|obj): c_COMPILER \.\./(Quake/[A-Za-z0-9_]+\.c)", text, re.M):
        tus.add(m.group(1))
    cat_of = {rel: cat for cat, rel, _, _ in rows}
    problems = []
    for tu in sorted(tus):
        cat = cat_of.get(tu)
        if cat is None:
            problems.append("%s: compiled but not in the table" % tu)
        elif cat not in COMPILED:
            problems.append("%s: compiled but classified %s" % (tu, cat))
    print("engine build: %d C TUs, %d problems" % (len(tus), len(problems)))
    for p in problems:
        print("  " + p)
    absent = sorted(rel for rel, cat in cat_of.items() if cat in COMPILED and rel not in tus)
    if absent:
        print("  not compiled on this platform/config (allowed): " + ", ".join(absent))
    return not problems


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--ninja", metavar="BUILD_NINJA")
    args = ap.parse_args()
    rows, vendored, errors = scan()
    for e in errors:
        print("error: " + e, file=sys.stderr)
    if args.ninja and not ninja_check(rows, args.ninja):
        return 1
    doc = render(rows, vendored) + "\n"
    path = os.path.join(ROOT, DOC)
    if args.check:
        current = read(path) if os.path.exists(path) else ""
        # the line counts are informational: a C-only edit must not turn CI
        # red until someone regenerates the doc. Classification (rows,
        # categories, file counts, arms, notes) is what --check guards.
        if mask_counts(current) != mask_counts(doc):
            print("error: %s is stale; run scripts/c_remnant_inventory.py" % DOC, file=sys.stderr)
            return 1
        return 1 if errors else 0
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(doc)
    print("wrote %s (%d TUs, %d vendored entries)" % (DOC, len(rows), len(vendored)))
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
