# Task plan: Rust migration Phase 10 — cleanup & idiomatic pass (pre-deletion tranche)

Status: approved (author-approved per `docs/ai/FABLE5_WORKFLOW.md`; single-session autonomous run)
Baseline: `8711ab4a` (`origin/master`, Phase 9 merged as PR #41)
Roadmap phase: **Phase 10 — Cleanup & idiomatic pass** (`docs/rust-migration/ROADMAP.md`)
Authority order: `docs/rust-migration/PLAN.md` > `docs/rust-migration/ROADMAP.md` > ADRs > this plan.

## 1. Objective

Start Phase 10 in roadmap order. Phase 9 is merged but still `[~]`: its
status block defines a **soak exit** (one tagged release built from the
Rust main on Windows, Linux and macOS, plus a green harness) and a
**separate deletion PR** (tag `c-reference/final`, delete the C-oracle
TUs and the `use_rust_*` switches, retire `quakedef.h`/the PCH, retire
the C-only CI legs). No release tag exists at the baseline (`git tag`
is empty), so the deletion PR has not happened.

Most of Phase 10's headline items *depend* on that deletion: the capi
shims and dual-view globals exist to serve the C glue TUs; `CbContext`
and the `repr(C)` structs have C viewers with `COMPILE_TIME_ASSERT`s in
`Quake/gl_rmain_glue.c`; the residual setjmp sites (ADR-009) live in
`host_glue.c`/`gl_screen_glue.c`; the allocator revisit (ADR-013) is
gated on "no ownership crosses the boundary". Doing any of that now would
delete or port C out of roadmap order.

This plan therefore delivers the **order-safe pre-deletion tranche** of
Phase 10 in full, and records the **post-deletion tranche** as explicitly
gated milestones so the phase is picked up without re-deriving it:

1. the C-remnant inventory and ADR-002 Phase-10 appendix (what native
   code survives, and why, keyed to the ADRs that decide each item);
2. the ADR-004 unsafe inventory review, made reproducible and gated;
3. the clippy pedantic tier across the workspace;
4. the roadmap/ADR bookkeeping that closes the ADR-013/ADR-014
   "revisit at Phase 10" questions that can be closed today.

## 2. Requirements and non-goals

Requirements (from ROADMAP Phase 10):

- R1 Native remnants decided: mimalloc (ADR-013), codecs (ADR-014),
  lodepng (ADR-012), any ObjC — each with a recorded keep/port decision.
- R2 Idiomatic pass started: clippy pedantic tier enabled workspace-wide
  with a documented allow-list; `-D warnings` stays green on every CI
  clippy arm.
- R3 Unsafe inventory reviewed and minimized; the inventory is
  reproducible (script + committed doc + CI check), not a one-off count.
- R4 ADR-002 appendix enumerating surviving native code with
  justification (exit criterion), produced now as the pre-deletion
  version and marked to be re-cut after the deletion PR.
- R5 Roadmap Phase 10 status block records evidence, deviations, and the
  gated remainder.

Non-goals (explicitly out of scope for this tranche — roadmap order):

- N1 Deleting any C file, `use_rust_*` switch, CI leg, `quakedef.h` or
  the PCH (Phase 9 deletion PR).
- N2 Porting `Host_Error`/`Host_EndGame` or converting the 19 setjmp
  sites (ADR-009 says Phase 10, but the sites are in glue TUs that the
  deletion PR removes; converting them requires the C callers gone).
- N3 Removing capi shims / dual-view globals, converting `repr(C)` types
  or `CbContext` (C viewers still compile against them).
- N4 Changing the allocator (ADR-013 gate not met), codec backends, or
  any crate dependency.
- N5 Any behavior change. The tranche is docs, scripts, CI steps, lint
  configuration, and lint-driven Rust edits that are semantically inert.
- N6 The fuzz workspace (`rust/fuzz/`) is its own workspace and is not
  moved onto the pedantic tier (it inherits nothing from the root).

## 3. Invariants

- I1 Harness parity is untouched: no C or Rust behavior changes. Any
  pedantic fix that would change a value, a cast, a float comparison, or
  evaluation order is *not* applied; the lint is allowed instead.
- I2 ADR-010 determinism: `as` casts that mirror C conversions stay `as`
  casts (hence the `cast_*` allows).
- I3 ADR-004: crate-level unsafe attributes are not weakened. Pure crates
  keep `#![forbid(unsafe_code)]`; `quake-net` and `quake-progs` keep
  `#![deny(unsafe_code)]` with their single allowed modules.
- I4 ADR-003: no new crates.
- I5 CI paths filter: workflow edits are limited to adding steps to the
  existing `bindgen-smoke` job in `rust.yml`.
- I6 Generated docs are reproducible from the scripts and gated with
  `--check` (pattern of `scripts/setjmp_inventory.py`).

## 4. Migration authority

- Phase: 10. Phase 9 remains `[~]` and is not touched other than the
  cross-reference from the Phase 10 block.
- ADRs applied: 002 (appendix), 003 (license gate closes ADR-014's
  Symphonia option), 004 (unsafe policy and inventory), 007 (dual-view
  end state, deferred), 009 (setjmp end state, deferred), 012 (lodepng
  keep), 013 (allocator, deferred with criterion), 014 (codecs stay C),
  019 (harness stays green: no behavior change).
- Ordering: post-deletion milestones are listed in §9 with their gate;
  none is started here.

## 5. Repository evidence

| Fact | Evidence |
| --- | --- |
| No soak-exit tag | `git tag` empty at `8711ab4a`; no `c-reference/final`. |
| Mixed build compiles 80 C TUs | Phase 9 worktree `build-rs/build.ninja` (configured 2026-09-16): 55 `Quake/*_glue.c` (21 199 lines) + 25 TUs shared with the C-only build. |
| C-only build compiles 166 TUs | `build-c/build.ninja`: 25 shared + 79 Quake C-oracle TUs (86 944 lines) + 62 meson-generated shader `.c` from `bintoc` (replaced by the xtask pipeline, Phase 8 M11). |
| Shared TUs (both builds) | `mkpak.c`, `bintoc.c` (native build tools, meson.build:113-114); `harness.c`, `harness_render.c` (harness); `mem.c` (mimalloc include), `image_stb.c` (stb), `image.c` (lodepng); `snd_{flac,mpg123,opus,vorbis}.c` (codec bridges); `cd_null.c`, `common.c`, `gl_model.c`, `model_parse.c`, `net_loop.c`, `net_main.c`, `net_win.c`, `palette.c`, `pr_cmds.c`, `pr_edict.c`, `pr_ext.c`, `pr_trace.c`, `q_thread_sdl.c`, `steam_api.c` (engine C that still compiles under the mixed build; several carry `USE_RUST_*` arms). |
| Vendored native | `Quake/mimalloc/` 24 743 lines; `lodepng.{h,c}` 8 758; `miniz.{h,c}` 9 432; `stb_image*.h` 11 331; `jsmn.h` 516. |
| `CbContext` C viewers | `rust/quake-types/src/render.rs:437-448` `#[repr(C)]`; `Quake/gl_rmain_glue.c:46-50` `COMPILE_TIME_ASSERT`s. |
| `repr(C)` counts | quake-types 160, quake-capi 34, quake-render 9, quake-platform 4, quake-net 1. |
| `#[no_mangle]` statics in capi | 99 (dual-view globals, ADR-007). |
| Crate unsafe attributes | forbid: cvar, formats, fs, image, math, snd, tasks, types, util; deny: net, progs; none: c-sys, capi, ctest, host, platform, render, xtask. |
| Unsafe token counts (ADR-004 method) | c-sys 284, capi 5153, ctest 207, net 21, platform 16, progs 141, render 324, snd 1, types 36, others 0 (before comment-line exclusion; script gives exact numbers). |
| Pedantic baseline | `cargo clippy --workspace --all-targets -W clippy::pedantic` at `8711ab4a`: 11 431 unique warnings over 57 lints; top 5: cast_possible_truncation 2194, cast_sign_loss 1586, doc_markdown 1269, cast_possible_wrap 1108, cast_lossless 984. With the §6 D3 allow-list, 524 remain (capi 123, ctest 292, render 62, progs 10, image 8, net 6, xtask 6, snd 4, fs 4, tasks 3, util 2, types 2, formats 1, cvar 1). |
| Current workspace lints | `rust/Cargo.toml`: `unsafe_op_in_unsafe_fn = "deny"`, `clippy.undocumented_unsafe_blocks = "deny"` only. |
| setjmp gate | `scripts/setjmp_inventory.py --check` in `rust.yml` `bindgen-smoke` (19 residual sites, all Phase 10 disposition). |
| ADR-014 vs ADR-003 | ADR-014 names Symphonia (MPL-2.0) as the Phase 10 candidate; ADR-003 rejects MPL-2.0 outright. |

## 6. Architecture decisions

- **D1 C-remnant inventory is rule-driven, like the setjmp inventory.**
  `scripts/c_remnant_inventory.py` classifies every `Quake/*.c|*.m`,
  `Quake/*/` vendored directory, `Shaders/bintoc.c` and
  `Misc/vq_pak/mkpak.c` by an explicit rule table into: `glue`
  (mixed-build only, deleted/ported post-deletion), `oracle` (C-only
  build, deleted by the deletion PR), `shared` (compiles in both builds
  today; post-deletion port list), `vendored` (ADR-002 remnant),
  `codec` (ADR-014 remnant), `tool` (native build tool), `harness`.
  An unclassified file fails `--check`, so new C files must be
  classified. Optional `--ninja <build.ninja>` cross-checks the rule
  table against a real configured build (used once here; recorded).
  Rationale: parsing `meson.build` is fragile and not needed; the rule
  table is the documented decision.
- **D2 Unsafe inventory script uses the ADR-004 counting method** (token
  `\bunsafe\b` per crate, comment lines excluded) and additionally checks
  crate-attribute conformance against ADR-004's crate lists. Output
  `docs/rust-migration/unsafe-inventory.md`; `--check` in CI.
- **D3 Pedantic tier = `pedantic = warn` at priority −1 plus a documented
  allow-list in `rust/Cargo.toml`.** Allowed (with the reason recorded
  inline): the five `cast_*` lints (ADR-010: casts mirror C),
  `cast_ptr_alignment`, `borrow_as_ptr`, `ptr_as_ptr` (FFI boundary
  style, retired with the shims), `float_cmp` (bit-exact C
  comparisons), `unreadable_literal` (C constants kept verbatim for
  side-by-side review), `doc_markdown`, `must_use_candidate`,
  `missing_errors_doc`, `missing_panics_doc` (doc-only churn in
  transitional crates), `too_many_lines`, `similar_names`,
  `many_single_char_names`, `items_after_statements`, `if_not_else`,
  `struct_excessive_bools` (1:1 C ports; renaming would break the
  C-to-Rust review trail). Everything else in pedantic is fixed
  (≈524 sites), so the tier is real, not decorative. Per-site
  `#[allow]`s are permitted only with a reason.
- **D4 ADR-014 revisit closes the Symphonia option.** ADR-003's
  license policy forbids MPL-2.0; the codec bridges stay C (a permanent
  ADR-002 remnant) unless a permissive decoder set is adopted by a new
  ADR. Recorded as an ADR-014 amendment, not a new ADR.
- **D5 ADR-013 revisit is deferred with an explicit criterion**:
  decide after the deletion PR, when the C-remnant inventory shows no
  `shared`/`glue` TU allocating engine memory; record the criterion in
  ADR-013 now.
- **D6 Post-deletion milestones are gated, not started** (§9).

## 7. Change boundary

Files added: `docs/ai/plans/rust-conversion-phase-10.md`,
`scripts/c_remnant_inventory.py`, `scripts/unsafe_inventory.py`,
`docs/rust-migration/c-remnant-inventory.md`,
`docs/rust-migration/unsafe-inventory.md`.

Files edited: `.github/workflows/rust.yml` (two steps in
`bindgen-smoke`), `rust/Cargo.toml` (workspace lints), Rust sources for
pedantic fixes (any crate, semantically inert edits only),
`docs/rust-migration/ROADMAP.md` (Phase 10 block),
`docs/rust-migration/adr/ADR-002-*.md` (appendix), `ADR-013`, `ADR-014`
(amendments), `docs/rust-migration/adr/README.md` only if an ADR's
status line changes.

Not touched: any `.c/.h/.m`, `meson.build`, `meson_options.txt`, crate
manifests other than `rust/Cargo.toml` lints, `Cargo.lock`.

## 8. Acceptance matrix

| ID | Criterion | Verification |
| --- | --- | --- |
| A1 | `python3 scripts/c_remnant_inventory.py --check` passes and the doc lists every TU of both builds with a disposition | script run; `--ninja` cross-check against the Phase 9 worktree ninja files |
| A2 | `python3 scripts/unsafe_inventory.py --check` passes; attribute conformance matches ADR-004 | script run |
| A3 | `cargo clippy --workspace --all-targets --locked -- -D warnings` green on default, `progs`, `platform,sdl3`, `platform,sdl2` arms | run from `rust/` |
| A4 | `cargo fmt --check`, `cargo test --workspace --locked` (release and debug), `cargo deny check` green | run from `rust/` |
| A5 | ADR-002 appendix present and consistent with A1's doc | review |
| A6 | ROADMAP Phase 10 `[~]` block with evidence, deviations, gated remainder | review |
| A7 | No C/Meson change in the diff | `git diff --stat` |

## 9. Milestones

Pre-deletion tranche (this PR):

- **M1 C-remnant inventory + CI gate** (D1). Deliverables: script, doc,
  `rust.yml` step.
- **M2 Unsafe inventory + review** (D2). Deliverables: script, doc,
  `rust.yml` step, review notes in the doc (attribute conformance,
  hotspots, what is expected to disappear post-deletion).
- **M3 Clippy pedantic tier** (D3). Deliverables: `rust/Cargo.toml`
  lints; fixes; all CI clippy arms green.
- **M4 Bookkeeping** (D4, D5, R4, R5). Deliverables: ADR-002 appendix,
  ADR-013/ADR-014 amendments, ROADMAP Phase 10 block.

Post-deletion tranche (gated on the Phase 9 soak exit: tag
`c-reference/final` exists and the deletion PR is merged; each becomes
its own approved milestone under this plan):

- **M5** Port `Host_Error`/`Host_EndGame` to `quake-host`; convert the
  19 setjmp sites (ADR-009 end state); `setjmp_inventory.py` reaches
  zero residual and is retired or kept as a zero-gate.
- **M6** Remove capi shims and the 99 dual-view globals TU by TU
  following the `glue` list in the C-remnant inventory (ADR-007 end
  state: `Host` struct owns the state).
- **M7** `repr(C)` → idiomatic types once no C viewer remains;
  lifetime-scope `CbContext`; drop the `COMPILE_TIME_ASSERT`s with the
  glue.
- **M8** Port the `shared` TUs that are engine code (`pr_ext.c`,
  `model_parse.c`, `gl_model.c`, `pr_cmds.c`, `common.c`, …) and delete
  `Quake/qs_bmp.h` from `quake-platform/build.rs` (Phase 9 amendment).
- **M9** ADR-013 allocator decision (D5 criterion); IPv6-on-MSVC flip
  (Phase 9 amendment); shrink `quake-c-sys`/`quake-capi` to the documented
  end-state surface.
- **M10** Re-cut the ADR-002 appendix and both inventories; retire the
  pedantic allow entries that only existed for FFI style; Phase 10 exit.

## 10. Final verification (pre-deletion tranche)

From `rust/`: `cargo fmt --check`; clippy on the four arms in A3; `cargo
test --workspace --locked --release` and debug; `cargo deny check`;
`cargo deny check licenses --manifest-path fuzz/Cargo.toml --config
deny.toml`. From the repo root: `scripts/harness/check_headers.sh`,
`python3 scripts/setjmp_inventory.py --check`, both new `--check`s.
No Meson build is required (no C change; A7).

## 11. Risks

| Risk | Mitigation |
| --- | --- |
| A pedantic "fix" changes semantics (e.g. `needless_pass_by_value` moving a copy, `manual_assert` changing panic text) | Only mechanical categories are fixed; anything touching numeric conversion, float compare, or panic messages consumed by the harness is allowed instead (I1). |
| Feature arms (`progs`, `platform`) surface pedantic warnings not in the baseline measurement | All four arms are run before commit (A3). |
| Rule table drifts from reality | `--check` fails on unclassified files; `--ninja` cross-check documented for re-runs. |
| Someone reads the pre-deletion appendix as the Phase 10 exit | Appendix header states it is the pre-deletion cut and lists M10 as the re-cut. |

## 12. Amendment log

- 2026-09-16 — Plan created at baseline `8711ab4a`.
- 2026-09-16 (M2) — ADR-004's "single `quake-progs` island" was already
  false at baseline: `quake_progs::image` (Phase 6) is a second
  `allow(unsafe_code)` module. Recorded in the ADR-004 amendment instead of
  removing the module; the inventory script's tier table lists both.
- 2026-09-16 (M3) — I1 exception, recorded: three release-mode
  `assert!(a == b)` in `quake-capi/src/gl_mesh.rs` (mesh-upload
  invariants) became `assert_eq!`. Condition unchanged; only the panic text
  differs, and no harness gate consumes it. Every other assert change is a
  `debug_assert!` → `debug_assert_eq!`.
- 2026-09-16 (M3) — Ten local `#[allow]` attributes (three lints, six files) instead of workspace entries:
  `wildcard_imports` on the `keys::*` tables (`quake-platform/src/input/
  mod.rs`, `scancode.rs`) and `enum_glob_use` on the three bindgen SDL2 enum
  tables (`sdl2.rs`), because the alternative is a 60-name import list that
  no longer reads like `keys.h`; `redundant_closure_for_method_calls` on
  two higher-ranked closures (`net.rs:372`, `net_dgrm_orch.rs:1020`) where
  clippy's suggested method path fails to type-check (lifetime-generic
  `with_net_message` callbacks); and `wildcard_imports` on the three
  `basedirs` glob imports in `quake-capi/src/fs.rs` (the `fs`+`sdl3`
  slice, which no CI arm compiles -- found by the PR-review Linux-target
  pass below).
- 2026-09-16 (M3) — Workspace `allow` list grew past the plan's D3 sketch
  by nine entries: `manual_midpoint` (`f32::midpoint` is not bit-identical
  to the C `(a + b) / 2`, ADR-010), `case_sensitive_file_extension_comparisons`
  (the C compares are case-sensitive), `ptr_cast_constness`, `ref_as_ptr`
  and `unnecessary_wraps` (the same FFI-shim style as D3's pointer lints,
  retired with the shims), and `decimal_bitwise_operands`,
  `struct_field_names`, `match_same_arms`, `large_stack_arrays` ("read
  like the C" cases, most surfaced only by the `progs,render` and
  `platform` arms). 29 entries total, each commented in `rust/Cargo.toml`.
- 2026-09-16 (M3) — One more text-only change beside the `gl_mesh.rs`
  asserts: `rust/xtask/src/shaders.rs:204` formats the failing shader
  compiler's program path with `Path::display()` instead of `{:?}`, so the
  build-tool error message loses its quoting. No engine, harness or
  generated-output surface reads it.
- 2026-09-16 (review) — `scripts/c_remnant_inventory.py --check` compares
  the doc with the `Lines` column of every table masked, so a C-only edit
  that moves a line count does not fail `rust.yml` until the doc is
  regenerated; classification (rows, categories, file counts, `USE_RUST_*`
  arms, notes) is still gated. (The first cut masked every integer-only
  cell, which also hid the arms column; narrowed on the PR #42 review.)
  The unsafe inventory keeps exact counts in its check on purpose: it
  exists to measure the Rust side, and a Rust change that moves a count
  should regenerate it.
- 2026-09-16 (M4) — `rust/fuzz/Cargo.lock` refresh (`windows-sys` under
  `quake-net`, stale since Phase 9 M2) committed as a side effect of the
  fuzz-workspace `cargo deny` run in §10; not a new dependency.
- 2026-09-17 (PR #42 review) — The two Linux clippy jobs failed on two
  pedantic sites in `quake-capi/src/net_udp.rs` (`range_plus_one`,
  `single_match_else`) that the Windows dev box never compiles
  (`cfg(unix)`); fixed to read like `net_wins.rs`, together with a
  `redundant_closure_for_method_calls` in `quake-platform/src/sys/unix.rs`.
  The local evidence now includes a `--target x86_64-unknown-linux-gnu`
  clippy pass over the CI arms and the full feature union (§13), which also
  surfaced the `fs.rs` wildcard imports above. Copilot's other two findings
  were checked and not applied: the `cl_parse.rs:1238` `continue` was in
  tail position (the `if`/`else if`/`else` chain is the last statement of
  the `loop` body, and the C's `continue` lands at the same place), and
  `bytes[1..=len]` with `len == 0` is the empty slice, not a panic
  (`1..=0` indexes as `1..1`), so `[]` still reaches the resolver as an
  empty host exactly as `1..1 + len` did and as the C does.
  Second round: the same two jobs then failed on the one file the
  Linux-target pass had skipped, `quake-ctest/tests/net_udp_differential.rs`
  (`#![cfg(unix)]`; the crate was excluded because its `build.rs` compiles
  C for the host) -- a `redundant_closure_for_method_calls` on the
  poison-tolerant lock and a `needless_for_each`. Fixed mechanically. It
  is the only non-Windows-gated file in `quake-ctest`, and it lints clean
  on the Windows host with the gate line temporarily removed (clippy does
  not link, so the `c_ref_*` externs are irrelevant; the gate was restored
  before committing) -- that is the local evidence for it (§13).
- 2026-09-17 (PR #42 review, second round) — ten findings, all taken:
  the two `nonminimal_bool` rewrites (`menu.rs` `m_in_scrollbar`,
  `r_part_fte.rs` `stain*` tokens) had inverted the C's shape and the
  `menu.rs` SAFETY comment had gone stale; both now read like the C, the
  dropped duplicate term is recorded as an I1 deviation (§13, ROADMAP).
  `scripts/unsafe_inventory.py` now counts `tests/**`, `benches/**` and
  `build.rs` beside `src/**` (the `quake-ctest` differential suites carry
  1 991 tokens the doc and the ADR-004 amendment did not mention; total
  8 782, ADR-004 re-cut) and matches every `allow`/`expect(unsafe_code)`
  attribute, `cfg_attr`-wrapped or not, refining with the `mod` name only
  when one follows directly (an item-level allow records as the bare path
  and fails the policy). `scripts/c_remnant_inventory.py`: the `--ninja`
  regex accepts `objc_COMPILER` so a future `.m` TU is cross-checked, and
  an `UNCLASSIFIED` TU renders (the run still exits 1). `rust.yml`
  re-includes the two generated inventories in its `paths` filter so a
  hand edit runs its own guard. The ADR-013/ADR-014 amendment headings say
  M4, the milestone the plan assigns them. ADR-002's appendix names the
  post-port host of the vendored code: Meson-compiled TUs of their own, no
  `build.rs` `cc` step for engine code.

## 13. Verification evidence / handoff

Local, windows-x86_64, `cargo +1.97.1`, 2026-09-16, from `rust/` unless
noted. CI runs the same commands on the PR; the results below are the
pre-PR evidence.

| Check | Kind | Result |
| --- | --- | --- |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | broad | clean |
| `cargo clippy -p quake-capi --all-targets --locked --features progs,render -- -D warnings` | broad | clean |
| `cargo clippy -p quake-capi --all-targets --locked --features platform,sdl3 -- -D warnings` | broad | clean |
| `cargo clippy -p quake-capi --all-targets --locked --features platform,sdl2 -- -D warnings` | broad | clean |
| The same four arms with `--target x86_64-unknown-linux-gnu` (2026-09-17, after the PR #42 review fixes; `quake-ctest` excluded from the workspace arm because its `build.rs` compiles C for the host) | broad | clean |
| `-p quake-capi --target x86_64-unknown-linux-gnu` with the `quake-ctest` feature union plus `progs,render,tasks`, alone and with each of `platform,sdl3` / `platform,sdl2` (no CI arm compiles these unions) | broad | clean |
| `cargo clippy -p quake-ctest --test net_udp_differential -- -D warnings` on the Windows host with the file's `#![cfg(unix)]` line temporarily removed (2026-09-17, second review round; the only unix-gated file in `quake-ctest`) | targeted | clean |
| `cargo fmt --all -- --check` | broad | clean |
| `cargo test --workspace --locked` (debug) | broad | 1 566 passed, 0 failed |
| `cargo test --workspace --locked --release` | broad | 1 566 passed, 0 failed |
| `cargo deny check` | broad | clean (pre-existing `Unicode-3.0` unmatched-allowance warning) |
| `cargo deny --manifest-path fuzz/Cargo.toml check licenses` | broad | clean |
| `python scripts/setjmp_inventory.py --check` (repo root) | targeted | OK, 40 sites / 39 guarded TUs |
| `python scripts/c_remnant_inventory.py --check` | targeted | OK |
| `python scripts/unsafe_inventory.py --check` | targeted | OK |
| `unsafe_inventory.py` attribute matcher probed against item-level, `expect`, `cfg_attr`-wrapped and comment-separated `allow(unsafe_code)` forms (2026-09-17, second review round; scratch script) | targeted | each form records, only a direct `mod` refines |
| `git diff --stat` shows no `Quake/`, `meson.build`, `Shaders/` change (A7) | targeted | 155 files, all under `rust/`, `docs/`, `scripts/`, `.github/` |
| I1 semantic review of the M3 diff | manual | 172 non-trivial hunks read (everything that was not a semicolon, `writeln!`, `T::from(bool)`, `&[..]` for `vec![..]`, or a `match`→`if let` on the same arms); two source-shape deviations, the `gl_mesh.rs` `assert_eq!` (§12) and the `menu.rs` `m_in_scrollbar` duplicate term dropped for `nonminimal_bool` (integers only, behaviour unchanged; the second review round under-reported this, see §12). The `r_part_fte.rs` `stain*` chain is the other `nonminimal_bool` site: the `args == 3` repeat is folded out and, after the second review round, the OR-of-matches shape of the C is kept. The five `needless_continue` sites are all in tail position of their loop bodies: `quake-capi/src/snd_dma.rs` `S_Update` (after `combine = Some(j)`), `quake-net/src/dgrm.rs` `get_message` (end of the `NETFLAG_DATA` arm), `quake-capi/src/cl_parse.rs:1238` (the `removeflag` arm that ends the `loop`), `quake-capi/src/progs_edict_dispatch.rs:296` and `:546` (`PRPARSE_OK => {}`, the `match` is the last statement of each labeled loop), and `quake-tasks/src/queue.rs:74`/`:84` (`Steal::Retry => {}` in a `loop { match .. }`). The `net_wins.rs` `open_socket_fail(&OpenError, ..)` by-reference change is inert (no `Drop` on `OpenError` or `SysSocket`). |
| Meson builds, harness corpus/render gates | not run | no C or build line changed (A7); the differential tests above are the Rust-side gate |

Handoff: M1-M4 are the pre-deletion tranche in full. The single next
action is the Phase 9 soak exit (tag `c-reference/final`, deletion PR);
M5 is the first post-deletion milestone and starts as its own approved
milestone under this plan.

## 14. Completion gate

The pre-deletion tranche is complete when A1–A7 hold with current tool
evidence, the commit carries the required trailer, and the ROADMAP
Phase 10 block names the soak-exit gate for M5–M10.
