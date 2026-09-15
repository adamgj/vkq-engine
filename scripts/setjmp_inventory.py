#!/usr/bin/env python3
"""Residual setjmp/longjmp inventory for the Rust migration (Phase 9 M7).

Generates docs/rust-migration/setjmp-inventory.md from the C tree: every
executable `setjmp` / `longjmp` / `jmp_buf` / `<setjmp.h>` site (comments
stripped), each mapped to the disposition Phase 9 D7 requires -- "deleted
with soak exit" or "converted at Phase 10" -- plus the per-TU `Host_Guard`
and `Host_Reraise` counts with the Rust crate files that call the guarded
thunks (ADR-009). A site with no disposition rule fails the run, so a new
setjmp cannot appear without being classified here.

    python3 scripts/setjmp_inventory.py          # rewrite the document
    python3 scripts/setjmp_inventory.py --check  # fail if it is stale
"""

import argparse
import glob
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DOC = os.path.join("docs", "rust-migration", "setjmp-inventory.md")
QUAKE = "Quake"
RUST_SRC = [os.path.join("rust", d, "src") for d in ("quake-c-sys", "quake-capi", "quake-platform", "quake-host")]

SITE_RE = re.compile(r"\bsetjmp\b|\blongjmp\b|\bjmp_buf\b|#\s*include\s*<setjmp\.h>")
GUARD_CALL_RE = re.compile(r"\bHost_Guard\s*\(")
GUARD_MACRO_RE = re.compile(r"^HOST_GUARD_(?:VOID|PTR|INT)\s*\(\s*([A-Za-z_][A-Za-z0-9_]*)")
RERAISE_CALL_RE = re.compile(r"\bHost_Reraise\s*\(")
DEF_RE = re.compile(r"^(?:int\s+Host_Guard|void\s+Host_Reraise)\s*\(")

SOAK = "deleted with soak exit"
P10 = "converted at Phase 10"

# (file, regex over "<enclosing function>: <line>", disposition, note) --
# first match wins; every site must match one.
RULES = [
    ("host.c", r".", SOAK, "C-oracle TU (`use_rust_host` swaps in `host_glue.c`; Phase 9 deletion list)"),
    ("gl_screen.c", r".", SOAK, "C-oracle TU (`use_rust_render` swaps in `gl_screen_glue.c`)"),
    ("host_glue.c", r"#\s*include\s*<setjmp\.h>", P10, "goes with the last `jmp_buf`"),
    ("host_glue.c", r": jmp_buf\s+(host_abortserver|screen_error)", P10, "the two raise targets; become `Result` propagation once `Host_Error`/`Host_EndGame` are Rust (ADR-009 end state)"),
    ("host_glue.c", r"^(Host_Guard|Host_Reraise): ", P10, "`Host_Guard`/`Host_Reraise` trampoline pair (ADR-009 rule 3); needed while any C caller can raise past a Rust frame"),
    ("host_glue.c", r"longjmp \(screen_error, 1\)", P10, "`Host_Error` while CSQC draws the HUD; becomes an error path of the Rust render frame (ADR-009)"),
    ("host_glue.c", r"longjmp \(host_abortserver, 1\)", P10, "`Host_Error`/`Host_EndGame` raise; becomes `Err(HostError)` when the raise moves to Rust"),
    ("host_glue.c", r"^Host_Glue_FrameInner: .*setjmp", SOAK, "the C frame's own `setjmp`, compiled only without `USE_RUST_PLATFORM` (with it the frame is a `Host_Guard` whose status `quake_rs_host_frame` hands back); the Rust loop (`quake-platform::main_sdl::recover`) owns frame-abort recovery, and the `#ifndef` goes with the `use_rust_platform` switch"),
    ("gl_screen_glue.c", r"#\s*include\s*<setjmp\.h>|extern jmp_buf screen_error", P10, "goes with `SCR_DrawGUI`'s `setjmp`"),
    ("gl_screen_glue.c", r"setjmp \(screen_error\)", P10, "`SCR_DrawGUI`'s CSQC recovery point; becomes an error path of the Rust render frame function (ADR-009 end state)"),
]


def strip_comments(text):
    """Blank out block and line comments, keeping line structure."""
    out = []
    i = 0
    n = len(text)
    while i < n:
        c = text[i]
        if text.startswith("/*", i):
            j = text.find("*/", i + 2)
            j = n if j < 0 else j + 2
            out.append("".join("\n" if ch == "\n" else " " for ch in text[i:j]))
            i = j
        elif text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            out.append(" " * (j - i))
            i = j
        elif c == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            out.append(text[i : j + 1])
            i = j + 1
        else:
            out.append(c)
            i += 1
    return "".join(out)


def c_files():
    # the engine's C remnant only: the quake-ctest stubs keep their own
    # jmp_bufs for the differential harness and are not shipped
    files = []
    for ext in ("*.c", "*.h", "*.m"):
        files += glob.glob(os.path.join(ROOT, QUAKE, ext))
    return sorted(files, key=lambda p: os.path.basename(p))


def read(path):
    with open(path, encoding="utf-8", errors="replace", newline="") as f:
        return f.read().replace("\r\n", "\n")


def disposition(name, func, line):
    for fname, pattern, disp, note in RULES:
        if fname == name and re.search(pattern, "%s: %s" % (func, line)):
            return disp, note
    return None


# a definition head at column 0: `int Host_Guard (void (*fn) (void *), void *arg)`
FN_HEAD_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_ \t\*]*?\b([A-Za-z_][A-Za-z0-9_]*)\s*\([^;]*$")


def scan():
    sites = []  # (file, lineno, function, text, disposition, note)
    guards = {}  # file -> [guard calls, reraise calls, {thunk names}]
    errors = []
    for path in c_files():
        name = os.path.basename(path)
        text = strip_comments(read(path))
        func = "-"
        in_directive = False
        for lineno, raw in enumerate(text.split("\n"), 1):
            line = " ".join(raw.split())
            directive = in_directive or line.startswith("#")
            in_directive = line.endswith("\\")
            if not line:
                continue
            m = FN_HEAD_RE.match(raw) if raw[0] not in " \t#" else None
            if m and not raw.startswith("HOST_GUARD_"):
                func = m.group(1)
            if SITE_RE.search(line):
                d = disposition(name, func, line)
                if d is None:
                    errors.append("%s:%d: unclassified setjmp site in %s: %s" % (name, lineno, func, line))
                    d = ("UNCLASSIFIED", "")
                sites.append((name, lineno, func, line, d[0], d[1]))
            if DEF_RE.search(line) or (directive and not line.startswith("#include")):
                # the HOST_GUARD_* macro bodies are counted at their
                # instantiations, not their definitions
                continue
            g = len(GUARD_CALL_RE.findall(line))
            macro = GUARD_MACRO_RE.match(line)
            r = len(RERAISE_CALL_RE.findall(line))
            if g or macro or r:
                counts = guards.setdefault(name, [0, 0, set()])
                counts[0] += g + (1 if macro else 0)
                counts[1] += r
                if macro:
                    counts[2].add("Host_Glue_" + macro.group(1))
                elif g:
                    counts[2].add(func)
    return sites, guards, errors


def rust_callers(thunks):
    """thunk name -> rust files (relative) with a call `name(`; the
    quake-c-sys declarations do not count."""
    index = {t: set() for t in thunks}
    pattern = re.compile(r"\b(" + "|".join(re.escape(t) for t in sorted(thunks)) + r")\s*\(")
    for d in RUST_SRC:
        if d.startswith(os.path.join("rust", "quake-c-sys")):
            continue
        for path in glob.glob(os.path.join(ROOT, d, "**", "*.rs"), recursive=True):
            rel = os.path.relpath(path, ROOT).replace(os.sep, "/")
            for m in pattern.finditer(read(path)):
                index[m.group(1)].add(rel)
    return index


def md_code(s):
    return "`" + s.replace("`", "'") + "`"


def render(sites, guards, callers):
    out = []
    w = out.append
    w("# Residual setjmp/longjmp inventory")
    w("")
    w("<!-- generated by scripts/setjmp_inventory.py; do not edit by hand -->")
    w("")
    w("Rust migration Phase 9 M7 (plan D7, [ADR-009](adr/ADR-009-error-handling.md)).")
    w("The ROADMAP's Phase 9 line \"last `setjmp`/`longjmp` removed\" is scoped by D7:")
    w("while the C oracle is built from the same tree and the glue TUs below raise")
    w("through `Host_Guard`, the trampoline pair stays. This file is the committed")
    w("record of what remains and when each site goes. Regenerate with")
    w("`python3 scripts/setjmp_inventory.py`; `--check` fails when it is stale or a")
    w("new site has no disposition rule.")
    w("")
    w("What is already true (Phase 9 M6/M7):")
    w("")
    w("- The Rust host loop (`quake-platform::main_sdl`) is the sole owner of")
    w("  frame-abort recovery under `USE_RUST_PLATFORM`: `Host_Glue_FrameInner`")
    w("  has no `setjmp` there (it is a `Host_Guard` whose status")
    w("  `quake_rs_host_frame` hands back), and `SysGlue_HostFrame`'s")
    w("  `Host_Guard` status is consumed as `quake_host::error::HostError` in")
    w("  `main_sdl::recover`.")
    w("- `Sys_Init`/`Host_Init` run under the same guard; a raise there is a")
    w("  `Sys_Error` (the C jumped to an un-`setjmp`ed buffer).")
    w("- No Rust frame is ever unwound by `longjmp`: every C call that can raise")
    w("  is made through a `Host_Guard` thunk, and every Rust function C calls")
    w("  returns a guard status that C passes to `Host_Reraise` (ADR-009 rule 3).")
    w("")
    w("## Executable sites")
    w("")
    w("Comments stripped; `#include <setjmp.h>`, `jmp_buf`, `setjmp`, `longjmp`")
    w("over `Quake/*.{c,h,m}` -- the shipped C remnant. `rust/quake-ctest/stubs`")
    w("keeps its own harness-only traps and is out of scope. Runs in CI (the")
    w("`core headers bindgen smoke` job of `rust.yml`).")
    w("")
    w("| File:line | In | Site | Disposition | Why |")
    w("|---|---|---|---|---|")
    for name, lineno, func, line, disp, note in sites:
        w("| `%s:%d` | %s | %s | %s | %s |" % (name, lineno, md_code(func) if func != "-" else "-", md_code(line), disp, note))
    w("")
    by_disp = {}
    for s in sites:
        by_disp[s[4]] = by_disp.get(s[4], 0) + 1
    w("Totals: " + ", ".join("%d %s" % (by_disp[k], k) for k in sorted(by_disp)) + ".")
    w("")
    w("## `Host_Guard` / `Host_Reraise` call sites per TU")
    w("")
    w("Every `Host_Guard` call outside its definition sits in a thunk a Rust")
    w("crate invokes (Rust -> C: the guard turns a raise into a status); every")
    w("`Host_Reraise` call re-issues the status a Rust export returned (C ->")
    w("Rust). `HOST_GUARD_VOID/PTR/INT` instantiations count as one call each.")
    w("The last column lists the Rust source files that call the TU's thunks")
    w("(`quake-c-sys` only declares them); a thunk no Rust file calls is named.")
    w("")
    w("| TU | `Host_Guard` | `Host_Reraise` | Rust callers of the thunks |")
    w("|---|---:|---:|---|")
    tg = tr = 0
    for name in sorted(guards):
        g, r, thunks = guards[name]
        tg += g
        tr += r
        files = set()
        uncalled = []
        for t in sorted(thunks):
            if callers.get(t):
                files |= callers[t]
            else:
                uncalled.append(t)
        cell = ", ".join("`%s`" % f for f in sorted(files)) or ("-" if not thunks else "(none)")
        if uncalled:
            cell += "; no Rust caller: " + ", ".join("`%s`" % t for t in uncalled)
        w("| `%s` | %d | %d | %s |" % (name, g, r, cell))
    w("| **total (%d TUs)** | **%d** | **%d** | |" % (len(guards), tg, tr))
    w("")
    w("All of these convert at Phase 10 with the trampoline pair: once the raise")
    w("itself is Rust (`Host_Error`/`Host_EndGame` return `Err(HostError)`), a")
    w("thunk becomes a plain call and a re-raise becomes `?`.")
    w("")
    return "\n".join(out)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--check", action="store_true", help="fail if the committed document is stale")
    ap.add_argument("--output", default=DOC, help="document path relative to the repo root")
    args = ap.parse_args()

    sites, guards, errors = scan()
    for e in errors:
        print(e, file=sys.stderr)
    if errors:
        return 2
    thunks = set()
    for counts in guards.values():
        thunks |= counts[2]
    text = render(sites, guards, rust_callers(thunks))
    path = os.path.join(ROOT, args.output)
    if args.check:
        current = read(path) if os.path.exists(path) else ""
        if current != text:
            print("%s is stale; run scripts/setjmp_inventory.py" % args.output, file=sys.stderr)
            return 1
        print("OK: %s is current (%d sites, %d guarded TUs)" % (args.output, len(sites), len(guards)))
        return 0
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    print("wrote %s (%d sites, %d guarded TUs)" % (args.output, len(sites), len(guards)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
