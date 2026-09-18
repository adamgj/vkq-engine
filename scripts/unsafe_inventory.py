#!/usr/bin/env python3
"""Unsafe inventory (Rust migration Phase 10 M2, ADR-004).

Reproduces the ADR-004 "grep-based count per crate" and writes
docs/rust-migration/unsafe-inventory.md: per workspace crate, the number of
`unsafe` tokens in `src/**`, `tests/**`, `benches/**` and `build.rs` with
comment lines excluded (the ROADMAP's Phase 8 block recorded `src/**` only;
the differential suites in `quake-ctest/tests` are part of the ADR-019
harness and count too), a breakdown by token kind, the crate-level
`unsafe_code` attribute, and the per-file hotspots of the FFI crates. It also
checks each crate's attribute against the ADR-004 expectation table below, so
a pure crate cannot silently drop `forbid(unsafe_code)` and a bounded crate
cannot grow a second `allow(unsafe_code)` module.

Usage:
  python3 scripts/unsafe_inventory.py            # regenerate the doc
  python3 scripts/unsafe_inventory.py --check    # CI: stale doc or policy
                                                 # violation -> exit 1
"""
import argparse
import glob
import os
import re
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), os.pardir))
DOC = "docs/rust-migration/unsafe-inventory.md"
WORKSPACE = os.path.join("rust", "Cargo.toml")

# ADR-004 expectation per crate: "forbid" (pure crate), "deny" (crate-wide
# deny with the listed allow(unsafe_code) modules), "open" (a concentrated
# location: no crate-level attribute; unsafe_op_in_unsafe_fn and
# undocumented_unsafe_blocks come from the workspace lints).
POLICY = {
    "quake-types": ("forbid", ()),
    "quake-math": ("forbid", ()),
    "quake-util": ("forbid", ()),
    "quake-cvar": ("forbid", ()),
    "quake-fs": ("forbid", ()),
    "quake-formats": ("forbid", ()),
    "quake-image": ("forbid", ()),
    "quake-snd": ("forbid", ()),
    "quake-tasks": ("forbid", ()),
    "quake-net": ("deny", ("src/udp.rs (mod sys)",)),
    "quake-progs": ("deny", ("src/arena.rs", "src/image.rs")),  # image.rs: Phase 10 M2 amendment
    "quake-c-sys": ("open", ()),
    "quake-capi": ("open", ()),
    "quake-render": ("open", ()),
    "quake-platform": ("open", ()),
    "quake-host": ("open", ()),
    "quake-ctest": ("open", ()),
    "xtask": ("open", ()),
}

HOTSPOT_CRATES = ("quake-capi", "quake-render", "quake-c-sys", "quake-ctest", "quake-progs")
HOTSPOT_ROWS = 12

TOKEN_RE = re.compile(r"\bunsafe\b")
KIND_RES = [
    ("unsafe fn", re.compile(r"\bunsafe\s+(?:extern\s+\"C\"\s+)?fn\b")),
    ("unsafe extern block", re.compile(r"\bunsafe\s+extern\s+\"C\"\s*\{")),
    ("unsafe impl", re.compile(r"\bunsafe\s+impl\b")),
    ("unsafe block", re.compile(r"\bunsafe\s*\{")),
]
FORBID_RE = re.compile(r"^\s*#!\[forbid\(unsafe_code\)\]", re.M)
DENY_RE = re.compile(r"^\s*#!\[deny\(unsafe_code\)\]", re.M)
# Every `allow`/`expect(unsafe_code)` counts, whatever it is attached to (an
# item-level one, or one wrapped in `cfg_attr`, records as the bare file
# path, which no policy entry names, so `check_policy` fails as intended);
# an outer attribute directly on a `mod` is refined with the module name.
ALLOW_RE = re.compile(
    r"^\s*#(!?)\[(?:cfg_attr\([^,]*,\s*)?(?:allow|expect)\(unsafe_code(?:,[^)]*)?\)\)?\]"
    r"\s*(?:(?:pub(?:\([a-z]+\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*))?",
    re.M,
)
SCAN_GLOBS = ("src/**/*.rs", "tests/**/*.rs", "benches/**/*.rs", "build.rs")
MEMBER_RE = re.compile(r'^\s*"([A-Za-z0-9_-]+)",', re.M)


def read(path):
    with open(path, encoding="utf-8", errors="replace", newline="") as f:
        return f.read().replace("\r\n", "\n")


def members():
    text = read(os.path.join(ROOT, WORKSPACE))
    block = text.split("members = [", 1)[1].split("]", 1)[0]
    return MEMBER_RE.findall(block)


def code_lines(text):
    """Lines that are not comment lines (the ADR-004 counting method: token
    grep minus lines that are comments), with trailing line comments and
    block comments removed so a `/* unsafe */` paragraph does not count."""
    out = []
    in_block = False
    for line in text.split("\n"):
        s = line.strip()
        if in_block:
            if "*/" in s:
                in_block = False
                s = s.split("*/", 1)[1].strip()
            else:
                continue
        if s.startswith("//"):
            continue
        if s.startswith("/*"):
            if "*/" not in s:
                in_block = True
            continue
        # a trailing `// ...` comment on a code line does not count either
        # (`#![forbid(unsafe_code)] // unsafe lives in quake-capi`)
        if "//" in s and '"' not in s.split("//", 1)[0]:
            s = s.split("//", 1)[0]
        out.append(s)
    return out


def scan_crate(name):
    crate_dir = os.path.join(ROOT, "rust", name)
    files = sorted(p for g in SCAN_GLOBS for p in glob.glob(os.path.join(crate_dir, g), recursive=True))
    total = 0
    kinds = {k: 0 for k, _ in KIND_RES}
    per_file = []
    attr = "none"
    allow_modules = []
    lints_workspace = False
    manifest = os.path.join(ROOT, "rust", name, "Cargo.toml")
    if os.path.exists(manifest):
        lints_workspace = re.search(r"^\[lints\]\s*\nworkspace\s*=\s*true", read(manifest), re.M) is not None
    for path in files:
        rel = os.path.relpath(path, os.path.join(ROOT, "rust", name)).replace(os.sep, "/")
        text = read(path)
        if rel in ("src/lib.rs", "src/main.rs"):
            if FORBID_RE.search(text):
                attr = "forbid"
            elif DENY_RE.search(text):
                attr = "deny"
        for m in ALLOW_RE.finditer(text):
            allow_modules.append("%s (mod %s)" % (rel, m.group(2)) if m.group(1) != "!" and m.group(2) else rel)
        code = "\n".join(code_lines(text))
        n = len(TOKEN_RE.findall(code))
        if n:
            per_file.append((rel, n))
            total += n
            for k, r in KIND_RES:
                kinds[k] += len(r.findall(code))
    per_file.sort(key=lambda fr: (-fr[1], fr[0]))
    return {
        "name": name,
        "total": total,
        "kinds": kinds,
        "files": per_file,
        "attr": attr,
        "allow_modules": sorted(set(allow_modules)),
        "lints_workspace": lints_workspace,
        "src_files": len(files),
    }


def check_policy(crates):
    errors = []
    for c in crates:
        pol = POLICY.get(c["name"])
        if pol is None:
            errors.append("%s: workspace member without an ADR-004 policy entry" % c["name"])
            continue
        want, mods = pol
        if want == "forbid" and c["attr"] != "forbid":
            errors.append("%s: ADR-004 pure crate must carry #![forbid(unsafe_code)] (found %s)" % (c["name"], c["attr"]))
        if want == "deny":
            if c["attr"] != "deny":
                errors.append("%s: ADR-004 bounded crate must carry #![deny(unsafe_code)] (found %s)" % (c["name"], c["attr"]))
            got = tuple(sorted(c["allow_modules"]))
            if got != tuple(sorted(mods)):
                errors.append("%s: allow(unsafe_code) modules %s, ADR-004 permits %s" % (c["name"], list(got), list(mods)))
        if want == "open" and c["attr"] != "none":
            # tightening is fine, but the table should say so
            errors.append("%s: carries %s(unsafe_code); move it to the forbid/deny tier in POLICY" % (c["name"], c["attr"]))
        if want != "deny" and c["allow_modules"]:
            errors.append("%s: unexpected allow(unsafe_code) in %s" % (c["name"], c["allow_modules"]))
        if not c["lints_workspace"]:
            errors.append("%s: Cargo.toml lacks [lints] workspace = true (unsafe_op_in_unsafe_fn / undocumented_unsafe_blocks)" % c["name"])
    for name in POLICY:
        if name not in {c["name"] for c in crates}:
            errors.append("%s: in POLICY but not a workspace member" % name)
    return errors


def md_code(s):
    return "`" + s.replace("`", "'") + "`"


def render(crates):
    out = []
    w = out.append
    w("# Unsafe inventory")
    w("")
    w("<!-- generated by scripts/unsafe_inventory.py; do not edit by hand -->")
    w("")
    w("Rust migration Phase 10 M2 ([ADR-004](adr/ADR-004-unsafe-policy.md)).")
    w("`unsafe` tokens per workspace crate over `src/**`, `tests/**`, `benches/**`")
    w("and `build.rs`, comment lines excluded (the ROADMAP's Phase 8 block first")
    w("recorded `src/**` alone by hand; the `quake-ctest` differential suites are")
    w("the ADR-019 harness and count here),")
    w("with the crate-level `unsafe_code` attribute checked against the ADR-004")
    w("tier table in the script. Regenerate with `python3 scripts/unsafe_inventory.py`;")
    w("`--check` fails when the doc is stale or a crate's attribute or")
    w("`allow(unsafe_code)` module set departs from the table.")
    w("")
    w("Tiers: **forbid** = pure crate, `#![forbid(unsafe_code)]`; **deny** =")
    w("crate-wide `#![deny(unsafe_code)]` with the listed `allow` module(s);")
    w("**open** = concentrated location (FFI, Vulkan, harness), governed by the")
    w("workspace `unsafe_op_in_unsafe_fn = deny` and")
    w("`clippy::undocumented_unsafe_blocks = deny` lints.")
    w("")
    w("## Per crate")
    w("")
    w("| Crate | Tier | Attribute | `allow(unsafe_code)` modules | Tokens | `unsafe fn` | `unsafe extern` | `unsafe impl` | `unsafe {` | Files with unsafe |")
    w("| --- | --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |")
    total = 0
    for c in sorted(crates, key=lambda c: (-c["total"], c["name"])):
        total += c["total"]
        tier = POLICY.get(c["name"], ("?", ()))[0]
        mods = ", ".join(md_code(m) for m in c["allow_modules"]) or "—"
        k = c["kinds"]
        w("| %s | %s | %s | %s | %d | %d | %d | %d | %d | %d/%d |" % (
            md_code(c["name"]), tier, c["attr"], mods, c["total"],
            k["unsafe fn"], k["unsafe extern block"], k["unsafe impl"], k["unsafe block"],
            len(c["files"]), c["src_files"]))
    w("| **total** | | | | **%d** | | | | | |" % total)
    w("")
    w("Token kinds overlap by construction (`unsafe extern \"C\" fn` counts as")
    w("`unsafe fn`; a token can be none of the four when it is an `unsafe`")
    w("function-pointer *type*, which is what the `forbid` crates' non-zero rows")
    w("are).")
    w("")
    w("## Hotspots")
    w("")
    for c in crates:
        if c["name"] not in HOTSPOT_CRATES or not c["files"]:
            continue
        w("### %s" % md_code(c["name"]))
        w("")
        w("| File | Tokens |")
        w("| --- | ---: |")
        for rel, n in c["files"][:HOTSPOT_ROWS]:
            w("| %s | %d |" % (md_code(rel), n))
        rest = c["files"][HOTSPOT_ROWS:]
        if rest:
            w("| *%d more files* | %d |" % (len(rest), sum(n for _, n in rest)))
        w("")
    return "\n".join(out)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args()
    crates = [scan_crate(m) for m in members()]
    errors = check_policy(crates)
    for e in errors:
        print("error: " + e, file=sys.stderr)
    doc = render(crates) + "\n"
    path = os.path.join(ROOT, DOC)
    if args.check:
        current = read(path) if os.path.exists(path) else ""
        if current != doc:
            print("error: %s is stale; run scripts/unsafe_inventory.py" % DOC, file=sys.stderr)
            return 1
        return 1 if errors else 0
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(doc)
    print("wrote %s (%d crates, %d tokens)" % (DOC, len(crates), sum(c["total"] for c in crates)))
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
