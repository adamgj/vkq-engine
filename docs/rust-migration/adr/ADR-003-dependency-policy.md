# ADR-003: Third-party crate policy

**Status:** Accepted
**Date:** 2026-08-16
**Tags:** —

## Context

The project allows Cargo dependencies but requires them to be widely used, actively supported, and free of open high/critical CVEs. vkqr-engine is GPLv2+, constraining acceptable licenses. Supply-chain risk must be managed for a codebase that parses untrusted network and file input.

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

## Amended (Phase 2, 2026-08-18)

Runtime dependency added: `miniz_oxide` ("MIT OR Zlib OR Apache-2.0" → MIT;
transitively `adler2`, "0BSD OR MIT OR Apache-2.0") for `.kpf` zip inflation
and the embedded pak, replacing the vendored miniz on the Rust side
(ADR-012 gate: accept/reject parity with the in-tree miniz, differential-
and fuzz-tested). The `zip` crate was rejected for its dependency tree and
because parity with miniz — not generic zip support — is the requirement.

`NCSA` was added to the deny.toml allowlist for exactly one dev-only case:
`libfuzzer-sys` (cargo-fuzz, its own workspace under `rust/fuzz/`, never
linked into the engine) is "(MIT OR Apache-2.0) AND NCSA", the NCSA term
covering the vendored LLVM libFuzzer runtime. NCSA is a permissive
MIT/BSD-3 hybrid, OSI-approved and GPL-compatible; it is not a copyleft or
paid license, but as it extends the enforced allowlist this amendment
records it for owner review.

## Amended (Phase 3 M8, 2026-08-24)

The **first third-party runtime dependencies shipped inside
`libquake_rs.a`** land with the ADR-012 image-decode swap; everything
previously in the graph was dev- or build-only. Introduction-bar review:

- **`png` 0.18.1** ("MIT OR Apache-2.0" → MIT) — PNG decode behind the
  `Image_DecodeSTB` seam, gated pixel-exact vs stb over the fixture matrix,
  the in-repo `Misc/vq_pak` PNGs and the re-release depot corpus. The
  image-rs org crate: actively maintained, releases within weeks of this
  amendment, tens of millions of downloads, fuzzed upstream and again
  in-tree (`fuzz_image_stb`). Transitives, all "MIT OR Apache-2.0"-class:
  `bitflags`, `crc32fast`, `fdeflate`, `flate2`, `miniz_oxide` (already in
  the graph via quake-fs), `simd-adler32` (MIT), `adler2`, `cfg-if`.
  Alternatives considered: the `image` facade (rejected in the Phase 3 plan
  D8 for tree size), lodepng-rs (unmaintained), a full hand-port (the
  structural/acceptance half *is* hand-ported; only
  inflate/defilter/expand run in the crate).
- **`zune-jpeg` 0.5.15** ("MIT OR Apache-2.0 OR Zlib" → MIT) — JPEG decode
  under the owner-relaxed gate (see the ADR-012 M8 amendment). Actively
  maintained, used by the `image` facade as its default JPEG decoder,
  fuzzed upstream and in-tree. Sole transitive: `zune-core` 0.5.3 (same
  license). Alternatives: jpeg-decoder (slower, less maintained),
  keeping stb (remains the revert lever — `Image_DecodeSTBMem` stays
  compiled in every configuration).

## Amended (Phase 5 M1, 2026-08-25) — planned network dependencies

Phase 5's M7 UDP landriver plans three new direct dependencies, recorded
here at phase start so the introduction-bar review happens before any code
depends on them (adoption and the full `cargo deny check` land with M7):

- **`socket2`** ("MIT OR Apache-2.0" → MIT) — one cross-platform socket
  layer replacing the net_udp.c/net_wins.c near-duplicate pair; exposes
  v6only, multicast join, and broadcast directly, and its raw-fd interop
  maps onto the `sys_socket_t` slots in the C vtable ABI. Maintained by the
  rust-lang org.
- **`libc`** (already on the expected list above) — unix `getifaddrs`
  address enumeration, which socket2 does not provide.
- **`windows-sys`** ("MIT OR Apache-2.0" → MIT, Microsoft) —
  `GetAdaptersAddresses` and WSA specifics on Windows targets; its
  `windows-targets` transitives are the same dual license.

All three satisfy the allowlist via MIT (deny.toml excludes Apache-only
licenses; none of these trees are Apache-only). The M7 PR must still run
`cargo deny check licenses` over the full resolved tree and verify the
`*-pc-windows-gnu` build before merging.

## Amended (Phase 8 M1, 2026-09-05) — planned task-system and renderer dependencies

Phase 8 plans four new direct dependencies, recorded here at phase start so
the introduction-bar review happens before any code depends on them (each
adoption and its `cargo deny check` land with the milestone named):

- **`crossbeam-deque`** 0.8 ("MIT OR Apache-2.0" → MIT; crossbeam-rs org) —
  the injector + per-worker deque + stealer trio the ADR-016 scheduler is
  designed around (`quake-tasks`, M2). Its transitives `crossbeam-epoch` and
  `crossbeam-utils` carry the same dual license. Alternatives: `rayon` (a
  different scheduler model, not the ADR-016 design), an in-tree Chase-Lev
  deque (unsafe we would own; rejected while a maintained one exists).
- **`crossbeam-utils`** 0.8 (same license, already a transitive) —
  `Parker`/`Backoff`/`CachePadded` for worker idling (M2).
- **`loom`** 0.7 (MIT; tokio-rs org) — dev-only, declared under
  `[target.'cfg(loom)'.dev-dependencies]` so it is never resolved into the
  staticlib; runs the scheduler's state-machine models in the `rust.yml`
  `loom` job (M2).
- **`ash`** 0.38 ("MIT OR Apache-2.0" → MIT; ash-rs org) — the ADR-015 Vulkan
  binding. `default-features = false, features = ["std", "debug"]` and
  explicitly **not** `loaded`: the engine keeps loading through
  `SDL_Vulkan_GetVkGetInstanceProcAddr`, so `libloading` stays out of the
  tree and the crate has no runtime transitives. Introduced at M3 for the
  `vulkan_memory_t` handle mirror (ADR-011); the wider API surface follows
  from M6. Alternatives: `vulkano`/`wgpu` (ADR-015 rejects an abstraction
  layer), unmaintained raw `vk-sys`-style bindings.

All four satisfy the allowlist via MIT. Each adopting milestone must run
`cargo deny check licenses` over the full resolved tree and record the result
in the task plan's evidence table.

### Adopted at Phase 8 M2 (2026-09-05)

- `crossbeam-deque` 0.8.8 with its transitives `crossbeam-epoch` 0.9.21 and
  `crossbeam-utils` 0.8.23 (all "MIT OR Apache-2.0", taken as MIT) —
  `quake-tasks` runtime dependencies, reaching the staticlib only under the
  `tasks` feature. `crossbeam-utils` stays a transitive: the scheduler idles
  on a per-slot `Mutex`/`Condvar`, so `Parker`/`Backoff` were not adopted.
- `loom` 0.7.2 (MIT), dev-only under `[target.'cfg(loom)'.dev-dependencies]`
  of `quake-tasks`. Its transitives are dev-only too: `generator` 0.8.9,
  `scoped-tls` 1.0.1, `lazy_static` 1.5.0, `log` 0.4.34, `pin-project-lite`
  0.2.17, `regex-automata` 0.4.18, `rustversion` 1.0.23, `smallvec` 1.16.0,
  `thread_local` 1.1.10, `windows-result` 0.4.1 ("MIT OR Apache-2.0");
  `tracing` 0.1.44, `tracing-core` 0.1.36, `tracing-log` 0.2.0,
  `tracing-subscriber` 0.3.23, `matchers` 0.2.0, `nu-ansi-term` 0.50.3,
  `sharded-slab` 0.1.7, `valuable` 0.1.1 (MIT); `aho-corasick` 1.1.5 and
  `memchr` 2.8.3 ("Unlicense OR MIT", taken as MIT; both already in the
  tree through `regex`).
- `cargo deny check` (licenses, bans, advisories, sources) is clean in both
  workspaces (`rust/`, `rust/fuzz`) with `deny.toml` unchanged.
- `ash` is not yet adopted (M3).

### Adopted at Phase 8 M3 (2026-09-06)

- `ash` 0.38.0+1.3.281 ("MIT OR Apache-2.0", taken as MIT) with
  `default-features = false, features = ["std", "debug"]` — no `loaded`
  feature, so no `libloading`; the crate brings **zero** transitive
  dependencies into either workspace (`cargo tree -p ash` is a single
  line). It reaches the staticlib only under `quake-capi`'s `render`
  feature (via `quake-render`, which uses it for the `vk::DeviceMemory`
  handle in the `vulkan_memory_t` mirror and for the
  `VkMemoryAllocateInfo`/`VkMemoryAllocateFlagsInfo` structs the heap
  backend hands `R_AllocateVulkanMemory`); `quake-ctest` takes the same
  feature set as a dev-only dependency for the differential.
- The ADR-011 size probe (`quake-ctest/tests/render_abi.rs`) confirms
  `size_of::<ash::vk::DeviceMemory>() == sizeof (VkDeviceMemory)` on the
  Windows x86_64 leg; the D2 fallback (a `u64` newtype) was not needed.
- `cargo deny check` is clean in both workspaces with `deny.toml`
  unchanged.
