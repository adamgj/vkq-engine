# Rust migration Phase 5 — Networking wire layer (`quake-net`) — M1–M5

## Context

Phases 0–4 are complete on this branch (master at `9951f9b3`, Phase 4 merged). Phase 5 per `docs/rust-migration/ROADMAP.md:119` ports the networking **wire layer**: MSG/SZ serialization from `common.c`, the loopback/datagram/UDP drivers, demo file IO, and `net_main.c`. Protocol *logic* (`cl_parse.c`, `sv_main.c`) stays C until Phase 7.

**User decisions (recorded):**
- **Execution scope this session: M1–M5 only** (scaffolding → MSG/SZ port+flip → demo IO → loopback driver). Stop for user review before M6–M10 (dgrm, UDP/socket2, interop matrix, net_main, phase exit). The full 10-milestone phase design is recorded below so M6+ can resume from it.
- **Local gates**: run everything runnable on this Mac (incl. registered-data legs via the known `QUAKE_GAME_DATA`); document unreachable legs (Windows interop, IPv6 multicast in CI) as deferred in the exit checklist — Phase 3/4 precedent.
- Deletions deferred (phases 1–4 precedent; C stays the `-Duse_rust_net=disabled` oracle).
- Suspected bugs preserved bug-for-bug with `// COMPAT:` comments: `MSG_ReadUInt64` int-shift truncation (common.c:1267), UDP-vs-WINS `masked` AddrToString divergence (M7 concern). Logged as post-parity fix candidates.
- ENTALPHA/ENTSCALE macros stay in `protocol.h` (consumers are renderer/game code).
- Demo-record scope line: **Rust owns the demo file format; C keeps playback policy and CL_Record_* synthesis** (they call `MSG_WriteStaticOrBaseLine`, defined in sv_main.c:975 — Phase 7). No shim needed.

Governing: ADR-003 (deps), ADR-004 (unsafe), ADR-007 (needs new networking row), ADR-009 (no longjmp through Rust), ADR-010, ADR-011 (hand-written mirrors — net headers are NOT bindgen-clean), ADR-018, ADR-019 (gate 4 = phase exit; gate 5 = net fuzzers). Template: `docs/ai/plans/rust-conversion-phase-4.md` (one milestone per commit, tree green after each, amendment log). The approved task plan is committed as `docs/ai/plans/rust-conversion-phase-5.md` in M1.

## Verified ground truth (landmines)

- MSG/SZ wire section: common.c:977-1465. Readers use ambient `msg_readcount`/`msg_badread` (common.c:1178) and read the global `net_message` sizebuf directly; writers only touch their `sizebuf_t*` arg. Rust core is a slice-based reader/writer; the capi shim loads/stores C globals via quake-c-sys.
- DEBUG-only range checks in MSG_WriteChar/Byte/Short — release silently truncates; mirror with debug-only checks.
- Angle byte path asymmetric: write `& 255` unsigned, read as signed char (`MSG_ReadChar * 360/256`). Coord 16/24/32f + 13.3 fixed selected by PRFL flags; the 24-bit path has an odd `%255` (common.c:1121). Varint u64 UTF-8-like prefix; int64 zigzag. `MSG_WriteEntity` pext2 encoding (>0x7fff → `0x8000|hi` short + lo byte).
- `SZ_GetSpace` can `Host_Error` (longjmp) → ADR-009: Rust export returns status; a thin C glue fn re-raises. `SZ_*` has a non-net consumer (cmd.c Cbuf, 12 uses) — symbol names/signatures must not change at the flip.
- `MSG_ReadData` declared (common.h:190) but never defined — dead; do not port.
- cl_demo.c: wire IO = `CL_WriteDemoMessage` :77 (fflush every message), raw-read half of `CL_GetDemoMessage` :136-162, forcetrack `"%i\n"` write :659 / parse :730-737 (explicit `fgetc()=='\n'` check), resume-record `Sys_fseek(-17, SEEK_END)` :684 (4 len + 12 angles + 1 svc_disconnect). `CL_Record_Signons` swaps `net_message.data` to a scratch buffer :542-556 — one reason net_message storage stays C-owned.
- net_loop.c (266 lines): 3 statics; frames `[type u8][len u16 LE][pad u8]` (+4-byte LE sequence for unreliable), `IntAlign`; `Loop_GetMessage` sets the *peer's* `canSend` on reliable receipt; `Loop_Init` returns -1 for dedicated (net_main.c:872-877 depends on it); loopback must stay driver slot 0 (`IS_LOOP_DRIVER`).
- Driver vtables: `net_driver_t` (16 fields, net_defs.h:217-235), `net_landriver_t` (21 fields, :185-211); arrays defined per-platform in net_bsd.c:31-89 AND net_win.c:31-92 (meson platform split at meson.build:329-346) — driver-slot #ifdefs must be applied to both in lockstep.
- `net_driverlevel`/`net_landriverlevel` are ambient globals read back by drivers; no readers outside net_* files — read via quake-c-sys accessor until M9.
- Harness `-netcapture` instrument already exists in C (harness.c:158-163,336; hooks at net_main.c:595/629/680/703; framing `[u8 dir][u8 driver][u8 kind][u32le len][payload]`; `scripts/harness/capture_session.py` stages server + headless client) but has **no CI gate** — M1 wires it.
- quake-net is a 3-line stub; quake-capi is the sole `#[no_mangle]` point (feature-gated per phase — add `net` feature); bindgen generated.rs is committed with CI regen-diff (`scripts/gen_c_bindings.sh`, pinned bindgen-cli 0.72.1); cbindgen header generated at build.
- ctest: c_ref_* renames via force-included prelude (`rust/quake-ctest/include/c_ref_prelude.h`), `-ffp-contract=off`, `check_ctest_symbols.sh` gate; naming `<module>_differential.rs` / `<module>_abi.rs`.
- Fuzz: `rust/fuzz` separate workspace, targets fuzz **pure Rust** (not C-via-FFI), seeds committed, target list hardcoded at `.github/workflows/rust.yml:114-116`.
- CI per-module oracle configs are workflow-side: add `build-rs-cnet` (`-Duse_rust_net=disabled`) to build-linux.yml:130-142 + `check_capi_signatures.sh` line + `--compare`/`save_diff` legs.
- Live two-process captures are timing-nondeterministic — byte gates only via deterministic replay / single-process loopback sessions under `host_framerate`; live sessions get structural diffs (M8 design).

## Flip-mechanism map

| Scope | Mechanism |
|---|---|
| MSG_*/SZ_* in common.c | In-file `#if !defined(USE_RUST_NET)` around common.c:977-1465 (Phase-3 idiom) + new `Quake/net_msg_glue.c` compiled only under use_rust_net: C-signature wrappers calling capi exports; SZ_GetSpace overflow status re-raised as Host_Error in the C frame (ADR-009). cmd.c keeps linking the same SZ_* symbols. |
| cl_demo.c wire IO | In-file #ifdef swapping the IO primitives to capi calls. Playback policy + CL_Record_* synthesis untouched. |
| Drivers | **Per-driver vtable registration** (PLAN §4.3): under `USE_RUST_NET`, static initializers in net_bsd.c AND net_win.c point individual `net_drivers[]`/`net_landrivers[]` slots at `rust_net_*` extern "C" fns. Slot order preserved. C driver files stay compiled in both configs (oracle). |
| net_main.c (M9, future) | Whole-file meson exclusion + `Quake/net_glue.c` for C-owned storage (`net_message`, msg globals) and NET_* wrappers. |

Single meson option `use_rust_net` ('auto', follows use_rust, enabled-without-use_rust errors — verbatim `use_rust_snd` pattern meson.build:357-419) → cargo feature `net`. Per-driver granularity via #ifdef contents, not extra options.

## Milestones — THIS SESSION (M1–M5)

One commit per milestone; tree green (all configs build, gates below) after each.

### M1 — Scaffolding: mirrors, bindings, meson option, capture gate wiring
- `rust/quake-types/src/net.rs`: hand-written `#[repr(C)]` mirrors (ADR-011): `sizebuf_t`, `qsockaddr` (net_defs.h:36-45), `qsocket_t` (:144-179), `net_landriver_t`, `net_driver_t`, `qhostaddr_t`, NETFLAG_*/NET_HEADERSIZE/CCREQ/CCREP consts. Const layout asserts + `abi_probe.c` net probe (sizes/offsets incl. fn-pointer slots and qsocket buffer offsets) + `tests/net_abi.rs`.
- quake-c-sys allowlist + regen: `net_message`, `msg_readcount`, `msg_badread`, `net_driverlevel`, `net_landriverlevel`, `net_time`, `NET_NewQSocket`, `NET_FreeQSocket`, `SZ_*` (temporary until M3), `Con_Printf`/`Con_DPrintf`, `Harness_NetCapture`.
- meson: `use_rust_net` option + `net` cargo feature + feature-gated `net` module stub in quake-capi; `build-rs-cnet` added to build-linux.yml + check_capi_signatures + `--compare`/`save_diff` legs.
- Harness: `scripts/harness/capture_diff.py` (structural capture differ: handshake bytes exact, reassembled reliable stream per direction, per-kind counts) + msg_badread exit counter + CI capture smoke step (capture_session.py on build-c, self-compare).
- Docs: commit this plan as `docs/ai/plans/rust-conversion-phase-5.md`; ADR-007 networking row; ADR-004 amendment note (quake-net `#![deny(unsafe_code)]` with a future `sys` unsafe island in M7); ADR-003 socket2/libc/windows-sys decision note (adoption happens in M7).
- Gates: all configs build; run_corpus --check unchanged; bindgen regen-diff clean; check_headers.sh clean.

### M2 — MSG readers/writers + varint + entity encoding (ctest-only, no engine flip)
- `rust/quake-net/src/{sizebuf,msg}.rs`: slice-based SizeBuf/MsgReader/MsgWriter; every MSG_Write*/Read* incl. coord/angle variants by PRFL flags, angle asymmetry, varints, WriteEntity pext2. COMPAT: ReadUInt64 bug preserved; debug-only range checks; badread semantics as reader-state flag.
- ctest: c_ref_ the common.c wire section (prelude renames); `net_msg_differential.rs` sweeping every fn × protocols × flags × boundary offsets (incl. the ReadUInt64 bug domain, overflow/allowoverflow, badread sequencing).
- `fuzz_net_msg` target + seeds; add to rust.yml target list.
- Gates: ctest green; engine untouched; corpus unchanged.

### M3 — SZ_* port + engine flip of the MSG/SZ layer
- capi `net` exports (exact C signatures; SZ_GetSpace → status); `Quake/net_msg_glue.c` (Host_Error re-raise); `#if !defined(USE_RUST_NET)` around common.c:977-1465.
- Verify cmd.c/Cbuf path unchanged; MSG_ReadData stays dead.
- Gates: run_corpus --check + --compare (build-c vs build-rs-cnet; readers exercised via demo playback — writers rely on the ctest sweep until M5), save_diff, check_capi_signatures on build-rs-cnet, capture smoke unchanged.

### M4 — Demo file IO
- `rust/quake-net/src/demo.rs`: forcetrack write/parse, record framing read/write, fflush-per-message cadence, resume-record -17 seek. File-level differential tests (write both, byte-diff; parse corpus demos both).
- cl_demo.c in-file #ifdef flips the four wire-IO sites; policy + CL_Record_* stay C.
- New `record_diff` harness step: deterministic loopback session (`host_framerate` + harness cmds) with `record` on build-c vs build-rs-cnet → byte-diff demo files; playback of C-recorded demos on rs-cnet via `--compare`.
- `fuzz_net_demo` target + seeds.

### M5 — Loopback driver (first vtable flip)
- `rust/quake-net/src/loopback.rs`: port net_loop.c; the 3 statics become Rust module state (never C-visible, ADR-007). Calls NET_NewQSocket/FreeQSocket + reads net_driverlevel via c-sys. `Loop_Init` -1-for-dedicated behavior preserved.
- extern "C" `rust_loop_*` set registered via #ifdef in net_bsd.c AND net_win.c slot 0.
- ctest: c_ref_ net_loop.c + `net_loop_differential.rs` (frame packing/alignment sweep).
- Gates: full corpus --check/--compare (all single-player traffic is loopback) + record_diff + listen-server loopback smoke; **then STOP for user review**.

## Future milestones (M6–M10, next session; designed, not executed now)

- **M6** dgrm reliable + CCREQ/CCREP, ctest-first over a mock `NetSys` trait; BOTH RX paths transliterated verbatim (ProcessPacket:301 and GetMessage:514 inline); `fuzz_net_dgrm`, `fuzz_net_ccreq`.
- **M7** UDP landriver via socket2 (+libc getifaddrs, windows-sys) — ADR-003 review note + cargo deny (deny.toml excludes Apache-only; all three are MIT-dual); ADR-004 `sys` unsafe island; flip dgrm+udp slots.
- **M8** `-netreplay` C pseudo-driver instrument (byte gates: replayed cap → identical state-hash chains + recorded demos both builds) + `interop_matrix.py` 4-way × {15,666,999} × PRFL × PEXT; CI subset {666, 999, 999+PRFL variants, 666+PEXT2, 15} on localhost IPv4 shareware; IPv6/multicast/rcon/master/full matrix local-only.
- **M9** net_main.c port + Rust-owned driver table; meson-exclude net_main/net_bsd/net_win; `Quake/net_glue.c` for C-visible storage; ADR-009 audit of every Host_Error-capable path reachable from Rust frames.
- **M10** phase exit: full gate-4 checklist, fuzz soak, `/integration-review` from fresh context, deferred-deletes recorded, ROADMAP checkbox.

## Risks (top)

| Risk | Mitigation |
|---|---|
| Live-session timing nondeterminism defeats byte-diff gates | Byte gates only via netreplay (M8) + host_framerate loopback (M4+); structural capture_diff for live; stated honestly vs ADR-019 wording |
| Host_Error longjmp unwinding Rust frames (UB) | ADR-009 status shims + C-frame re-raise from M3 onward |
| Preserved ReadUInt64 bug "fixed" accidentally | COMPAT comment + differential sweep covering the bug domain |
| SZ/cmd.c Cbuf consumer breaks at M3 flip | Identical symbols via glue; check_capi_signatures; Cbuf-heavy harness cmds |
| Hand-written net mirrors drift from C headers | abi_probe const asserts on sizes+offsets; net_abi.rs in CI |
| MSG writer coverage thin at M3 flip (demo playback exercises readers) | ctest exhaustive sweep + fuzz roundtrip until M5 loopback sessions cover writers in-engine |

## Non-goals

cl_parse/sv_main protocol logic (P7); MSG_WriteStaticOrBaseLine + CL_Record_* synthesis (stays C); ENTALPHA/ENTSCALE relocation; fixing the two preserved bugs; unifying dgrm RX paths; any C deletion; M6–M10 in this session.

## Verification summary (this session)

Per milestone: cargo test -p quake-ctest (targeted differentials), cargo clippy/fmt, affected builds (build-c, build-rs, build-rs-cnet equivalents locally via meson configs).
Milestone boundaries: `run_corpus.py --check` (darwin-arm64 goldens) + `--compare` C-vs-mixed, `save_diff.py`, `check_headers.sh`, `check_capi_signatures.sh`, `check_ctest_symbols.sh`, bindgen regen-diff, `cargo deny check`.
M5 exit (session end): full corpus incl. registered-tier local entries, record_diff byte-identity, fuzz smoke on the two new targets, then stop and report for user review before M6.

## Amendment log

- **2026-08-25 M1**: `msg_badread` instrument is a comparative *counter*
  (`harness_badread_count`, printed as `Harness: msgbadread=<n>` at
  Harness_Shutdown), not a zero-gate: the dgrm connect path deliberately
  probes optional ProQuake fields until badread reports end-of-message, so
  benign events occur by design. Interop cells compare the counter C-vs-Rust
  instead of requiring zero.
- **2026-08-25 M1**: `capture_diff.py` gates the concatenated per-direction
  *reliable* stream prefix + per-kind record counts; CCREQ/CCREP handshakes
  are NOT visible in -netcapture (the hooks sit at the NET_* message funnels,
  above the drivers' control-packet IO), so handshake parity is deferred to
  the M8 interop matrix / netreplay instruments rather than capture diffs.
- **2026-08-25 M2**: the MSG/SZ wire section was **split out of common.c into
  `Quake/net_msg.c`** (verbatim move, corpus-verified behavior-neutral)
  instead of the planned in-file #ifdef: it lets quake-ctest compile the
  section directly as the c_ref oracle without dragging all of common.c in,
  and turns the M3 flip into the established whole-file Meson idiom.
- **2026-08-25 M2**: upstream quirk found and mirrored: `MSG_ReadDouble` is
  declared `float` in C -- it assembles the 8-byte double, then narrows to
  float at the return. The Rust `read_double` returns f32 accordingly.
- **2026-08-25 M3**: live localhost sessions proved to carry ~1 byte of
  run-to-run nondeterminism near the signon tail of the recv reliable stream
  (C-vs-C diverges at the same offset as C-vs-Rust). `capture_diff.py` gained
  `--window-from <second-C-run>`: the gate window is calibrated to the
  reference build's own divergence point, so the criterion is "Rust matches
  C at least as far as C matches itself." The CI step captures C twice +
  Rust once accordingly.
- **2026-08-25 M4**: demo seam refined: Rust owns the byte *format* (record
  header assembly/parse, forcetrack line format + fscanf-%i-equivalent
  parse, resume seek constant); the raw stdio on cls.demofile (which may
  sit inside a pak) stays in cl_demo.c under #ifdef. The forcetrack parse
  reads a bounded chunk and seeks to the consumed offset. record_diff.py
  is the byte gate: -demohash-deterministic loopback session recorded by
  both builds -> .dem byte-identical (verified: 20239 bytes, C self and
  C-vs-Rust).
- **2026-08-25 M5**: `Loop_SearchForHosts` stays C until M9 (hostcache/slist
  plumbing over net_main.c state, not wire logic); its vtable slot keeps the
  C symbol under USE_RUST_NET. `Loop_Init`'s `cls.state == ca_dedicated`
  test is mirrored via the bound `isDedicated` global (host.c derives the
  state from it before NET_Init; cls lives in a non-bindgen-clean header) --
  COMPAT-commented. Loopback driver logic lives in quake-capi::net_loop
  (FFI-heavy qsocket plumbing; precedent: snd_dma.rs), the frame format in
  quake-net::loopback (pure). The ctest prelude now also neuters q_stdinc.h
  (SDL.h) for direct includers like net_loop.c.
- **2026-08-26 review fixes** (fresh-context compatibility review of M1–M5):
  1. `Con_Printf` is not a leaf (it can reach `SCR_UpdateScreen`), so the
     Rust `with_sizebuf` frame no longer calls it while holding sizebuf
     borrows: allowed-overflow diagnostics accumulate Rust-side and
     `net_msg_glue.c`'s `NetMsg_Raise` drains them via
     `quake_rs_sz_take_overflow_events()` and prints from its C frame,
     before any Host_Error (same output order as C).
  2. C's float→int conversion is UB out-of-domain and the two architectures
     resolve it differently (x86-64 cvtt* → INT_MIN incl. NaN; arm64 fcvtzs
     saturates, NaN→0, = Rust `as`): `c_cast_i32` in msg.rs now reproduces
     each platform's C behavior (Q_rint casts + coord24's two casts), so
     per-platform C-vs-Rust parity holds on the x86-64 CI legs too.
  3. `uint64_bug_domain_goldens` pins the exact encode bytes and (buggy)
     decode values of the ReadUInt64/WriteUInt64 masked-shift domain on both
     the Rust port and the c_ref oracle, so an optimization- or
     platform-dependent codegen change breaks the gate loudly. Note: the
     c_ref oracle is compiled at the cargo profile's opt level (-O0 for
     tests), the engine at -O2; the goldens plus the engine-level corpus
     gates bracket that gap.
  4. Explicit `break;` after each noreturn raise in `NetMsg_Raise`.
  - Accepted divergence (recorded): the glue's `Sys_Error ("SZ_GetSpace: %i
    is > full buffer size")` prints a fixed per-writer length placeholder
    rather than the exact internal request on multi-write ops — fatal-path
    message text only, unreachable with SZ_Alloc-sized buffers.
  - Verified during review: `Sys_Error` exits (no longjmp) on both
    sys_sdl_unix.c and sys_sdl_win.c, so Sys_Error-from-Rust is ADR-009
    safe; `CL_Record_Signons`' scratch buffer is `static byte
    [NET_MAXMESSAGE]`, exactly net_message.maxsize, so the shim slice
    construction stays in-bounds during the data swap.
- **2026-08-26 second review pass** (M4/M5/ABI/CI surfaces; verdict "ready
  with stated residual risk", no correctness/ABI/build defects found):
  1. `capture_diff.py` gained `--min-window` (default 1024): a degenerate
     early C-vs-C reference divergence now FAILS the gate instead of
     silently shrinking the byte-compare window to nothing.
  2. New `net_demo_differential.rs` + `ctest_demo_forcetrack_oracle` stub:
     `parse_forcetrack` is differentially tested against the platform
     libc's actual `fscanf("%i")`+`fgetc` (20k randomized inputs + corner
     grammar), closing the M4 "file-level differential" gap.
  3. COMPAT notes added in cl_demo.c for the two accepted divergences: the
     64-byte forcetrack chunk bound (C's fscanf run was unbounded) and the
     atomic 16-byte header read (no partial cl.mviewangles updates on a
     truncated record; both builds stop playback).
  4. `parse_forcetrack` now accumulates in the sign's direction so overflow
     saturates to LONG_MIN like strtol (was -LONG_MAX).
  5. The USE_RUST_NET vtable entries in net_bsd.c/net_win.c use designated
     initializers: the three same-signature slot pairs can no longer swap
     silently.
  6. debug_asserts record the loopback aliasing invariant (peer != sock).
  - Verified during review (recorded): pak-embedded forcetrack seek
    arithmetic exact; Loop_Init isDedicated substitution sound (host.c
    derives cls.state from the same parm before NET_Init); net_abi.rs runs
    on windows-latest via rust.yml's 3-OS cargo test, gating SysSocket and
    the qsa_family ladder on MSVC; build-windows.yml/build-mac.yml's
    -Duse_rust=enabled legs auto-enable use_rust_net, so the flipped
    configuration compiles on all three OSes in CI; the workflows' shared
    paths filter admits all changed paths.
  - Deferred to M8/M10 (recorded): a cross-build record->playback
    round-trip leg (today: byte-identical .dem files + the generic corpus
    playback compose to cover it); an explicit multi-client listen-server
    smoke.

- **2026-08-26 CI fix (M2 gate domain)**: `uint64_bug_domain_goldens` pinned
  the ten-byte lead form (`c >= 2^63`, `b == 9`), where `MSG_WriteUInt64`
  evaluates `c >> 64`. That is UB with no stable answer: gcc (all -O) and
  MSVC emit the register-masked shift, but clang folds it to 0 at -O2/-O3 --
  the level the engine and the `cargo test --release` binaries build at --
  so the macOS leg produced `0` where the golden (captured from a debug
  build) had `c & 0xff`. The goldens now stop at `2^63 - 1`, the writer
  sweeps filter on `wire_value_is_defined`, and the new
  `uint64_ub_domain_variants` states the real contract for the UB domain:
  identical length, lead byte and every other continuation byte, the port
  pinned to the masked form, the c_ref byte allowed to be either observed
  form, and both readers agreeing over the same bytes. `write_uint64` gained
  the matching COMPAT note (accepted divergence; reachable only from the QC
  `WriteUInt64`/`WriteInt64` builtins, and the ten-byte form never
  round-trips through `MSG_ReadUInt64` anyway).
- **2026-08-26 CI fix (M4 gate domain)**: `forcetrack_parse_matches_libc`
  compared the port against the *host* libc, but `%i` on a `0x`/`0X` prefix
  with no hex digit after it is a genuine libc disagreement: C99 7.21.6.2p9
  allows one character of pushback (macOS libc and MSVC honour it) while
  glibc swallows the whole `0x` and converts 0, so on Linux a header line of
  exactly `"0x\n"` is accepted as track 0 and elsewhere it is not. The C
  engine already behaves differently per platform there. `parse_forcetrack`
  implements the one-character-pushback reading everywhere (COMPAT-noted as
  an accepted divergence, reachable only from a hand-authored header) and
  the differential skips that input class, pinning it against the port's own
  contract instead. Verified against glibc in a container: 0 mismatches over
  the fixed vectors plus the 20k randomized sweep.
- **2026-08-26 PR review fixes** (adversarial review on PR #21; all seven
  findings accepted and addressed):
  1. `capture_diff.py`: the `--min-window` floor was gated on
     `min(len(sa), len(sb))`, which disabled it in exactly the case it
     existed to catch — a *compared* build emitting a truncated reliable
     stream shrinks `window` itself and would then match trivially over its
     own short prefix. The floor is now judged against the reference stream
     only, and applies on the uncalibrated path too (the same hole existed
     there). Verified empirically: a capture whose recv reliable stream was
     truncated 5274 -> 300 bytes PASSED the old gate and FAILS the new one,
     while the healthy capture still passes.
  2. `sizebuf.rs::print`: the trailing-NUL branch could index one past the
     slice (allowoverflow + trailing NUL + `s.len() == maxsize`, where
     `get_space` resets cursize to 0), turning C's out-of-allocation
     scribble into a Rust panic/abort. Writes are now clamped to the
     allocation and both COMPAT deviations are documented; a regression test
     pins it (verified to fail against the pre-fix code).
  3. `quake_rs_demo_forcetrack_line` silently clamped to `outsize` while
     returning the length the caller writes — a corrupt demo header with no
     diagnostic if the invariant ever rotted. Now a fatal `Sys_Error`.
  4. `net_msg_glue.c`: documented that the `length` argument is a
     PLACEHOLDER for the writers whose reservation is computed inside Rust,
     and unreachable (SZ_Alloc floors maxsize at 256).
  5. `net_loop.rs`: the `Sys_Error`-for-`Host_Error` substitution is marked
     DO-NOT-CARRY for the M6/M7 dgrm receive path, which assembles wire
     fragments and has no bounded-sizebuf guarantee — there the M3
     status-plus-C-frame shape is required.
  6. `cl_demo.c`: the `goto`-into-`if (0)` seam is replaced by an `invalid`
     flag with one unconditional error block; the record-header COMPAT note
     now also records that the `mviewangles[0] -> [1]` copy is skipped and
     that it runs before the MAX_MSGLEN check (C ran it after).
  7. `quake-types/src/net.rs`: the pointer-width-dependent layout asserts
     are behind `#[cfg(target_pointer_width = "64")]` so they cannot
     hard-fail a 32-bit build. Verified with an actual
     `i686-unknown-linux-gnu` check: zero errors now originate in net.rs.
     NOTE: the crate still does not compile 32-bit — 8 pre-existing asserts
     in `json.rs` (Phase 1) and `sound.rs` (Phase 4) fail there. Out of
     Phase 5 scope; worth a separate issue if 32-bit is wanted.

- **2026-08-26 M6 (dgrm reliable layer, ctest-first)**: net_dgrm.c was split
  like net_msg.c at M2: the reliable/unreliable wire layer (SendMessage /
  SendMessageNext / ReSendMessage / CanSend* / SendUnreliableMessage /
  ProcessPacket / GetMessage, the `packetBuffer` scratch and the six stat
  counters) moved **verbatim** to `Quake/net_dgrm_rel.c`, with the shared
  statics de-static'd through the new internal header `net_dgrm_int.h`
  (net_dgrm.c's GetAnyMessage and NET_Stats_f keep using them). The
  orchestration (connect handshake, `_Datagram_ServerControlPacket`,
  hostcache/slist, heartbeats, rcon, Test/Test2) stays in net_dgrm.c -- it
  is engine-entangled (svs/menus/cvars/SV_ConnectClient) and its
  byte-serialization already flows through the Rust MSG/SZ layer under
  `-Duse_rust_net`; its Rust port is M9 territory.
  Port decisions (quake-net::dgrm, pure, `NetSys` trait for
  sfunc.Read/Write/AddrCompare/AddrToString + Con prints):
  - BOTH RX paths transliterated separately as planned; their asymmetries
    are load-bearing (ProcessPacket ACKs to sock->addr and pre-checks the
    unreliable maxsize; GetMessage ACKs to readaddr and has NO unreliable
    maxsize pre-check -- that C path Host_Errors inside SZ_GetSpace, so the
    port returns `GET_MESSAGE_NET_MESSAGE_OVERFLOW` for the M7 glue to
    re-raise (ADR-009 M3 shape), honoring the M5 DO-NOT-CARRY note).
  - The C `packetBuffer` static is shared TX/RX scratch and its stale bytes
    are observable (a wire header claiming more bytes than were received
    copies stale scratch into net_message); every port function takes the
    same persistent scratch slice, and the differential compares the full
    64008-byte scratch after every op. COMPAT divergences (documented in
    dgrm.rs): reads beyond the scratch end (C UB) zero-fill; the
    release-mode oversize-send memcpy overflow (C UB) is a hard error.
  - Counters stay C-owned globals in the engine (NET_Stats_f untouched);
    the port mutates a `DgrmCounters` view the M7 shim will marshal.
  - `net_dgrm_rel.c` compiles as the c_ref oracle (the prelude gained the
    dgrm renames; stubs gained BigLong=LongSwap and the ambient net
    globals + a 3-slot `net_landrivers` whose Read/Write/AddrCompare/
    AddrToString slots the test aims at Rust trampolines sharing one mock
    core per side). `net_dgrm_differential.rs`: 12 suites -- ack cycles,
    fragmentation, resend timing, dup/stale/gap sequencing, junk (CTL,
    unknown flags, short, stray addr, read error), all oversize paths
    incl. the Host_Error mapping, stale-scratch reproduction, and a
    60-round randomized op sweep; compares returns, all qsocket sequencing
    fields, buffers, net_message, scratch, counters, emitted wire packets
    (bytes+addr), and the con-log diagnostics.
  - `fuzz_net_dgrm` (op-stream over the two RX + send paths, invariant
    asserts) and `fuzz_net_ccreq` (the exact CCREQ/CCREP/slist/
    getserversResponse read sequences over the Rust MSG reader) live with
    seeds and are in the CI fuzz list. Engine flip of the rel layer is M7
    (whole-file exclusion of net_dgrm_rel.c + glue, alongside the UDP
    landriver).
- **2026-08-26 M7a (dgrm engine flip)**: `net_dgrm_rel.c` swaps whole-file
  for `net_dgrm_glue.c` under `-Duse_rust_net` (the Phase 4 idiom): the glue
  keeps the `Datagram_*`/`SendMessageNext`/`ReSendMessage`/
  `Datagram_ProcessPacket` names so the net_bsd.c/net_win.c vtables and
  net_dgrm.c's orchestration half are untouched, owns the shared statics
  (packetBuffer + counters, bindgen-bound for the shims), pre-validates the
  send paths in its C frames (DEBUG Sys_Errors + the release oversize guard
  where C memcpy'd blindly), and re-raises rust_dgrm_GetMessage's -2 status
  as the exact SZ_GetSpace Host_Error (ADR-009). quake-capi::net_dgrm
  marshals counters/net_message/scratch per call, reaches
  `net_landrivers[]` through the ADR-011 mirror, and defers Con prints
  until every C-memory borrow ends (M3 lesson). Verified on darwin-arm64:
  3 configs build; calibrated live-capture gate PASS with the Rust dgrm
  layer on both ends (recv reliable prefix identical over the 4708-byte
  calibrated window); record_diff .dem byte-identical (20239 bytes);
  save_diff identical; corpus --compare green; capi signature gate,
  check_headers, clippy -D warnings, fmt, cargo deny licenses clean.
- **2026-08-26 M7b (UDP landriver via socket2)**: `quake-net` adopts
  `socket2` + `libc` per the ADR-003 M1 amendment (`cargo deny check
  licenses` green over the resolved tree). The landriver splits three ways:
  `quake_net::udp` (pure address logic: AddrToString, StringToAddr,
  AddrCompare, PartialIPAddress, port helpers -- sockaddr byte offsets are
  identical across supported unixes), `quake_net::udp::sys` (the ADR-004
  unsafe island: socket2 for creation/options/v6only/multicast/broadcast,
  raw libc for recvfrom/sendto/ioctl-FIONREAD and the *same* legacy
  resolver calls the C driver made -- gethostbyname/gethostbyaddr/
  getaddrinfo -- so their behavior is inherited, not reimplemented), and
  `quake-capi::net_udp` (engine globals net_hostport/my_ipv*_address/
  ipv*Available via bindgen, the file statics as Rust module state per
  ADR-007, Con_SafePrintf at the exact C print points).
  **Scope decision (plan amendment)**: the flip lands in net_bsd.c only
  (both UDP/UDP6 rows, designated initializers). net_wins.c keeps its C
  driver: Windows CI has no UDP runtime leg (its harness legs are
  loopback-only), and flipping a runtime-unverified network personality is
  exactly what the ADR-017 SDL2-audio precedent defers. Recorded for M8/M10:
  either add a Windows capture/interop leg and flip WINS, or record the
  deferral in the phase-exit checklist.
  COMPAT notes (documented in the sources): recvfrom's out-address is
  zero-filled past the kernel-written length (C left stale stack bytes;
  only _Datagram_AddPossibleHost's whole-struct memcmp could ever observe
  it); sscanf/atoi out-of-domain behavior (UB in C) is pinned; the linux
  iflist 1s cache is keyed per family.
  Verified on darwin-arm64: 3 configs build; the calibrated live-capture
  gate PASS with Rust sockets + Rust dgrm on both ends; record_diff
  byte-identical; save_diff identical; corpus --compare green;
  net_udp_differential (5 suites: AddrToString v4/v6/scope sweep 500+
  randomized, StringToAddr full-match domain, AddrCompare/port matrix,
  PartialIPAddress corner grammar, the MAXHOSTNAMELEN guard) green vs the
  c_ref oracle; capi signature gate, check_headers, clippy -D warnings,
  fmt, cargo deny clean; ctest 41 suites green.
- **2026-08-26 M8 (netreplay + interop matrix)**: two instruments close the
  phase's remaining gate-4 surface.
  1. `-netreplay <capture>` (harness.c + guarded funnel hooks in
     net_main.c, identical C in every configuration): NET_Connect hands out
     a pseudo-socket, one captured recv record is delivered into
     net_message per host frame, sends are absorbed; forces the fixed
     timestep. `netreplay_diff.py` replays one capture on two builds and
     byte-compares the -demohash state-hash chains plus a demo recorded
     mid-replay -- the timing-noise-free "captured-session replay"
     criterion (live sessions keep the calibrated structural gate).
  2. `interop_matrix.py`: 4-way C/Rust client x server localhost sessions
     across every protocol cell this server can negotiate (Base-/FTE+
     x 15/666/999; FTE+999 = PRFL_FLOATCOORD|SHORTANGLE, Base-999 =
     PRFL_INT32COORD|SHORTANGLE -- PRFL_24BITCOORD is not reachable from
     this engine's server, covered at the MSG layer by ctest instead).
     Checks per cell: expected negotiated protocol string, signon health,
     exact reliable-count match across the four combos, unreliable counts
     within the live noise floor, and the msgbadread profile normalized as
     (badread - messages received) -- the raw counter scales with message
     count, so +-1-message timing noise shifts it (observed 117 vs 116
     tracking 117 vs 116 messages exactly).
  Calibration findings recorded: a Base-15 idle session legitimately emits
  almost no unreliables (C/C baseline: 2 in 600 frames), so per-cell health
  is judged against the C/C baseline rather than an absolute floor.
  CI: build-linux.yml gains the replay byte gate (C self + C-vs-Rust) and
  the IPv4 matrix; the IPv6 [::1] leg is local-only as planned.
- **2026-08-26 M9 (net_main.c core port; audit-driven scope amendment)**:
  the ADR-009 audit reshaped M9 away from the planned whole-file exclusion
  + net_glue.c sketch. Finding: the dispatch funnels -- NET_Connect,
  NET_GetMessage, NET_GetServerMessage, NET_SendMessage/Unreliable,
  NET_CanSendMessage, NET_SendToAll, NET_Poll/SchedulePollProcedure, and
  NET_Init/Shutdown -- all have `Host_Error`-capable code beneath them
  (the dgrm glue's SZ_GetSpace re-raise, `_Datagram_ServerControlPacket`
  -> SV_ConnectClient, the MSG-writer glue under SearchForHosts/connect
  handshakes), and a longjmp must never unwind a Rust frame. Those
  functions ARE the C boundary frames ADR-009 requires until Phase 7
  statusizes the strata beneath, so they stay verbatim C in net_main.c;
  whole-file exclusion would only have shuffled them into a glue file.
  Ported instead (quake-capi::net_main, Phase-3 in-file #ifdef idiom with
  trampolines/#defines in net_main.c): SetNetTime, the qsocket pool
  (NET_NewQSocket/FreeQSocket over the C-owned pool + new NetMain_*
  svs/sv accessor funnels), all eight NET_QSocket* accessors, the
  listen/maxplayers/port command handlers, the slist UI (sort +
  byte-exact %-W.Ws print helpers + the two static-buffer print
  functions; `slistLastShown` moves to Rust), and the leaf driver loops
  NET_Close / NET_CheckNewConnections / NET_ListAddresses (only
  Sys_Error-exit paths beneath them). hostcache_t and PollProcedure
  gained ADR-011 mirrors pinned by abi_probe/net_abi.
  Deletion note for M10: net_main.c is now part-ported; its funnel
  remainder (and net_dgrm.c's orchestration half) transfer to the Phase 7
  deletion list.
  Verified on darwin-arm64: 3 configs build; corpus --check (both) and
  --compare (mixed + cnet oracle) green; save/record byte-identical;
  calibrated capture PASS; netreplay_diff PASS; interop subset PASS; a
  listen-server slist/maxplayers/port/listen console smoke is
  byte-identical C-vs-Rust (exercises the Rust pool via loopback connect
  too); capi signature, check_headers, net_abi, clippy -D warnings, fmt
  clean; ctest 40 suites green.
- **2026-08-26 M10 (phase exit)**: exit evidence and the fresh-context
  review round.
  - Fuzz soak: 150s per net target, no findings -- fuzz_net_msg 12.2M,
    fuzz_net_demo 84.5M, fuzz_net_dgrm 4.9M, fuzz_net_ccreq 92.5M execs.
  - Fresh-context compatibility review (M6-M9, e6e683ae..M9): verdict
    "findings require fixes" -- two should-fix + notes, all addressed:
    (5) rust_udp_GetNameFromAddr's AddrToString fallback now clamps to
    NET_NAMELEN-1 (was safe only by arithmetic: worst-case formatted
    address is 59 bytes); (9) the netreplay gate gained an inert-replay
    guard -- the engine reports "Harness: netreplay=<n>" delivered records
    and netreplay_diff.py fails below a floor (default 50) or on a count
    mismatch (the same hole class capture_diff's --min-window closed);
    (2) COMPAT notes at all four dgrm oversize pre-checks recording that
    C's 32-bit `receiveMessageLength + length` sum WRAPS for hostile
    sub-header wire lengths (C then memcpy's a negative length, UB/crash)
    while the port's usize sum takes the oversize path -- the differential
    deliberately avoids the C-crash domain; (3) split_host_port now
    mirrors strtoul's whitespace/sign/ULONG_MAX semantics (port ":-1" ->
    65535 like C); (4) getaddrinfo_pick6 returns Err/Ok(None)/Ok(Some) so
    the UDP6 no-port retry fires only on resolver errors like C, not on
    successful IPv4-only lookups; (6)/(8) COMPAT notes for the
    uninitialized-fromlen absorb and the zero-filled out-structs;
    (10) interop_matrix fails loudly when the msgbadread counter line is
    missing. Review confirmations recorded: no ADR-009 violation in the
    newly-Rust frames (all raise-capable callees verified beneath C
    funnels), transliteration fidelity of both dgrm RX paths, ABI mirrors
    pinned on all three OSes, ADR-003 licensing clean.
  - The ROADMAP Phase 5 exit-criteria checklist is filled in with the
    per-criterion evidence, the netreplay gate's precise coverage stratum
    (it bypasses the drivers; the flipped drivers are gated by ctest +
    record_diff + calibrated capture + interop), and the carried-to-later
    -phases record (net_wins.c per ADR-017; the Host_Error funnels and
    dgrm orchestration per the M9 audit). Deletions deferred as in
    Phases 1-4.
  - Known unexercised surfaces, recorded honestly: rust_udp6_* runtime
    coverage is local-only (the [::1] interop leg); Windows compiles
    net_dgrm_rel.c as a ctest oracle for the first time on CI;
    the Windows UDP runtime leg remains the condition for the net_wins.c
    flip.
- **2026-08-26 M10 peer review (PR #22)**: adversarial review of M6-M10;
  all 12 findings assessed, all accepted and fixed.
  1. **Reachable divergence**: the slist printers used `{:2}` where C uses
     `%2u` on an `int`. `hostcache.users/maxusers` take `MSG_ReadByte()`
     (-1 on a truncated CCREP_SERVER_INFO) and `atoi()` of a dpmaster
     `clients` key, so `users == -1` printed `4294967295` in C and `-1`
     here. Both printers now cast through `u32`.
  2. **Provenance**: `net_drivers`/`net_landrivers` were declared as a
     single element and indexed with `.add(idx)` -- arithmetic outside the
     declared object. Fixed *not* by fabricating an array size (the
     reviewer suggested `MAX_NET_DRIVERS`, which is a dead constant no C
     code enforces, and both arrays are incomplete types sized by their
     initializers) but by adding `NetMain_Drivers`/`NetMain_LanDrivers` in
     net_main.c: the base pointer comes from C, so the offset has
     provenance over the real object. `hostcache` IS a complete array type,
     so it gets a truthful `[HostCache; HOSTCACHESIZE]` extern.
  3. `sys::host_by_name` now checks `h_addr_list`/`h_addr_list[0]` for NULL
     and `h_length >= 4` before the copy -- the ADR-004 island makes the
     boundary sound rather than inheriting C's unchecked deref.
  4. `interop_matrix.py` ran the dedicated server on an unread `PIPE`,
     which wedges once its console output fills the 64 KB buffer. Server
     stdout now goes to a file (kept for the early-exit diagnostic).
     Readiness: the reviewer suggested polling the server log, but probing
     showed the engine's stdout is block-buffered when not a tty, so no
     marker appears until exit -- polling cannot work. Replaced the blind
     3s sleep with a **bind probe** instead (UDP4/6_OpenSocket never set
     SO_REUSEADDR, so our bind of the same port fails EADDRINUSE exactly
     once the server is listening): buffering-independent, and a cell now
     costs ~5s instead of 3s of guesswork.
  5. Deferred Con_Printf drain reorders rel-layer diagnostics against the
     landriver's. Eager draining would reinstate the M3 re-entrancy, so the
     divergence is accepted and documented (console text only), including a
     note at the differential's console assertion recording that its mock
     NetSys never prints and so cannot observe the interleaving.
  6. `c_atoi` saturated where C truncates. NB the review's example is
     miscomputed -- `atoi("99999999999")` is 1215752191 on LP64, not -1;
     LONG_MAX saturation needs >9.2e18 -- but the finding is real at other
     inputs (`"4294967300"` is 4 in C, `INT_MAX` when saturating, turning
     `maxplayers 4294967300` from 4 players into the server maximum). Now
     one shared `quake_net::cnum::c_atoi` implementing `(int) strtol` with
     the accumulator in `c_long` (so the saturation point moves with the
     platform: LP64 vs LLP64), used by both net_main and udp, and pinned by
     a new `net_cnum_differential` against the real libc `atoi` on every CI
     OS (fixed vectors + 20k randomized).
  7. `MAXHOSTNAMELEN` moved to `quake_net::udp` and pinned in abi_probe /
     net_abi (it is observable: it decides which hostnames are
     connectable); socket2's implicit `SOCK_CLOEXEC` documented; the
     `UDP6_GetAddrFromName` failure path now mirrors C's `sa_family = 0`
     clobber on the resolved-but-no-AF_INET6 case, with the tail-fill
     divergence documented.
  8. `PollProcedure` mirror documented as a deliberate pre-pin for Phase 7.
  9. ROADMAP: Phase 5 returned to `[~]` to match Phases 1-4 (same
     deferred-deletion posture; the marker means "not closed out", not "not
     done"), and every carve-out is now listed in the receiving phase's own
     Scope and Deletes -- the funnels + dgrm orchestration in Phase 7,
     net_wins.c + net_bsd.c/net_win.c in Phase 9.
  10. `netreplay_diff.py` dumped build B's log when build A produced no
      demo; fixed, and a partial (nonzero but short) record header in
      `Harness_NetReplayGetMessage` is now fatal instead of masquerading as
      EOF, as is a failed seek.
  11. `fuzz_net_dgrm` now derives `max_datagram` from the input across the
      whole legal range -- it is the one value that can drive
      `send_fragment`'s slice and the ACK `copy_within` window out of
      bounds, and it is held in range by another translation unit. The
      invariant is stated on `send_fragment`. 3.3M runs clean.
  Also removed: two ctest Rust array stand-ins made dead by fix 2.
