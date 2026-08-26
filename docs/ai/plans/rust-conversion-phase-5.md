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
