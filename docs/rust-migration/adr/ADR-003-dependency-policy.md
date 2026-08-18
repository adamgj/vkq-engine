# ADR-003: Third-party crate policy

**Status:** Accepted
**Date:** 2026-08-16
**Tags:** —

## Context

The project allows Cargo dependencies but requires them to be widely used, actively supported, and free of open high/critical CVEs. vkQuake is GPLv2+, constraining acceptable licenses. Supply-chain risk must be managed for a codebase that parses untrusted network and file input.

GPLv2-compatibility is necessary but not sufficient. LGPL and other copyleft licenses are GPLv2-compatible yet impose relinking, source-disclosure, and per-file obligations on downstream packagers and on anyone reusing engine code — a real cost for a project distributed through many third-party package repositories. Crates requiring a paid or commercial license (dual-licensed "free for non-commercial use", BUSL/SSPL/Elastic-style source-available terms, or a purchased seat/redistribution key) are incompatible with a GPLv2+ codebase anyone can build and redistribute for free. Permissive licensing keeps distribution obligations at "keep the notice file accurate".

## Decision

- **CI gates:** `cargo audit` (RustSec advisory DB) and `cargo deny check` run on every PR. Any advisory of high/critical severity on the dependency graph fails the build; lower severities require a tracked issue with a remediation plan.
- **Licenses — permissive only, MIT preferred:** `cargo deny` enforces an allowlist of permissive, GPLv2-compatible licenses: `MIT`, `MIT-0`, `Apache-2.0`, `Apache-2.0 WITH LLVM-exception`, `BSD-2-Clause`, `BSD-3-Clause`, `Zlib`, `ISC`, `0BSD`, `Unicode-DFS-2016`, `Unicode-3.0`. When several crates solve the same problem, pick the MIT (or MIT/Apache-2.0 dual-licensed) one.
- **No copyleft, no paid licenses:** any crate under a copyleft license — `LGPL-*`, `GPL-*`, `AGPL-*`, `MPL-2.0`, `CDDL-*`, `EPL-*`, `EUPL-*`, `CC-BY-SA-*` — or under source-available/commercial terms (`BUSL-1.1`, `SSPL-*`, `Elastic-2.0`, "free for non-commercial use", or anything needing a purchased seat or redistribution key) is **rejected**. This applies to the whole dependency graph, not just direct dependencies: a permissive crate that pulls in a copyleft transitive dependency is itself rejected. `cargo deny check licenses` runs with `unlicensed = "deny"` and `allow-osi-fsf-free = "neither"` so unknown, missing, or non-listed licenses fail the build rather than defaulting through. Exceptions require an ADR amendment approved by the project owner, not a per-PR waiver; the one pre-approved carve-out is a `Zlib`/`MIT`-licensed binding crate wrapping a system library the user already has (e.g. SDL), where nothing copyleft is vendored or statically linked.
- **Notices:** `cargo about` (or equivalent) generates a third-party notice file from the lockfile as part of packaging, so attribution stays in sync with the actual dependency graph.
- **Lockfile:** `Cargo.lock` is committed. Dependency updates are deliberate PRs, not implicit.
- **MSRV:** pinned in `rust-toolchain.toml` and workspace metadata; bumps are explicit PRs.
- **Introduction bar:** each new *direct* dependency requires a short review note in its PR: maintenance status (recent releases/commits, bus factor), download/user base, transitive dependency footprint, and alternatives considered. Prefer std or small hand-written code over a dependency for trivial functionality.
- **Expected direct dependencies** (reviewed at introduction, not grandfathered): `ash`, `sdl2`, `sdl3`, `crossbeam-deque`/`crossbeam-utils`, `zip` or `flate2`+`miniz_oxide`, `png`/`image`, `libmimalloc-sys`, `bitflags`, `libc`; build/dev-only: `bindgen`, `cbindgen`, `libfuzzer-sys`, `criterion`, `proptest`, `loom`.

## Consequences

- CVE exposure is monitored continuously rather than at release time; the "no open high/critical CVE" requirement is enforced mechanically.
- Some conveniences are rejected for footprint reasons; small utility code is written in-tree instead.
- License compliance is automatic; GPLv2+ distribution remains clean, and downstream packagers inherit no relinking or source-disclosure obligations beyond the engine's own GPLv2+.
- Occasionally the best-in-class crate for a job is copyleft and must be passed over for a weaker permissive alternative or in-tree code. That cost is accepted deliberately; `cargo deny` makes it visible at PR time rather than at release time.
- The allowlist needs maintenance: crates do relicense, and SPDX expressions for dual-licensed crates (`MIT OR Apache-2.0`) must be satisfiable from the allowlist alone. Revisit if a genuinely unavoidable dependency (a Vulkan or platform binding with no permissive equivalent) turns up.

## Amended (Phase 1, 2026-08-17)

`rust/deny.toml` is the enforced allowlist and deliberately diverges from the
table above in one direction: **Apache-2.0 is excluded** (it is
GPLv3-only-compatible; accepting it would force the combined GPLv2-or-later
binary to GPLv3+ — see deny.toml's header comment). Dual-licensed
"MIT OR Apache-2.0" crates still pass via MIT, which covers the ecosystem.
The unused `MPL-2.0`/bare `GPL-2.0` allowances from Phase 0 were removed.
First dependencies added (dev-only, never linked into the staticlib):
`cc` and `proptest`, both MIT OR Apache-2.0 → MIT. External CLI tools
(`cbindgen`, MPL-2.0; `bindgen-cli`, BSD-3-Clause) do not enter the
dependency graph.
