# vkqr-engine C → Rust Migration Roadmap

Companion to [PLAN.md](PLAN.md). Eleven phases, each independently shippable: at every phase exit the engine builds and passes the full differential-verification suite ([PLAN.md §7](PLAN.md#7-verification-strategy-adr-019)) on Windows, Linux, and macOS. C files are deleted at phase exit (after a soak window where noted); transitional `-Duse_rust_<module>` Meson switches are removed with them.

Sizes are relative T-shirt estimates. The renderer (Phase 8) is roughly one-third of total effort — it is ~44k of ~124k engine LOC.

**Status legend:** `[ ]` not started · `[~]` in progress · `[x]` complete

---

## Phase 0 — Scaffolding, C prep, verification harness (M) `[~]`

The only phase that ports nothing. Everything later depends on it.

**Status (2026-08-17):** implemented on `claude/rust-conversion-phase-one-e696b9`; CI green on Windows, Linux and macOS (including the mixed-build hash/savegame identity gates on all three); awaiting merge.
- Done: Cargo workspace + Meson mixed build (verified byte-identical to C-only on the macOS corpus); header surgery (core headers bindgen-clean, checked in CI); verification harness (headless demo hash chain, savegame byte-diff, `-Dtrace` progs trace, protocol capture) with committed `darwin-arm64` goldens; approved deletions (`net_wipx.c`, `snd_mikmod/xmp/modplug.c`, `Windows/VisualStudio/`); CI matrix + rust lint/audit jobs; Windows CI rewritten from MSBuild to Meson+clang-cl (contrary to PLAN §3's claim, no Windows Meson CI existed before this phase — it was created here).
- Deferred: first c2rust oracle translations (`tools/c2rust-oracle/` is scaffolded; no working c2rust toolchain on the dev host — first consumers are Phases 1/6); registered-tier goldens for `linux-x86_64`/`windows-x86_64` (need a machine with game data; CI enforces run-twice stability and mixed-vs-C identity on the shareware tier meanwhile); tag `c-reference/phase0` at merge.

**Scope**
- Cargo workspace at `rust/` (crates per [PLAN.md §5](PLAN.md#5-workspace-design), initially near-empty); Meson `custom_target` building and linking an empty `libquake_rs.a` into `vkquake` on all three OSes (Windows: clang-cl + `x86_64-pc-windows-msvc`).
- CI: extend `.github/workflows/` with the mixed-build matrix, `cargo audit`, `cargo deny`, clippy, rustfmt. `deny.toml` lands in this phase with the permissive-only license allowlist from [ADR-003](adr/ADR-003-dependency-policy.md) (MIT preferred; no copyleft, no paid licenses; unknown licenses denied) plus the advisory gate — before the first crate is added, so no dependency is ever grandfathered in unchecked.
- **Header surgery** (pure C, behavior-neutral — [PLAN.md §4.1](PLAN.md#41-phase-0-header-surgery-pure-c-before-any-rust)): `q_types.h`, self-contained format headers, `q_thread.h` SDL-threading wrapper, `Sys_File*` funnel for SDL file IO, `q_render_types.h`.
- **Verification harness built first**: headless demo-playback mode + per-frame state-hash chain; scripted savegame byte-diff; `-Dtrace=true` progs trace hooks compiled into the **C** build; protocol capture tooling; per-platform golden generation from the C build; demo/mod corpus assembled and stored (id1, Hipnotic, Rogue, re-release, Arcane Dimensions, Copper, Alkaline, Quoth, CSQC mods).
- Approved removals, done in C ([ADR-018](adr/ADR-018-dropped-features.md)): delete `net_wipx.c` (+ IPX driver registration in `net_win.c`), delete `snd_mikmod.c`/`snd_xmp.c`/`snd_modplug.c` (+ Makefile hooks), retire `Windows/VisualStudio/` (Meson becomes the sole Windows build; keep `bintoc`/`mkpak` building under Meson until Phase 8).
- Set up `tools/c2rust-oracle/` and produce first oracle translations (`pr_exec.c`, `mathlib.c`, `world.c`) for reference.

**Exit criteria**
- C-only build passes the new harness with stable per-platform goldens (run-to-run identical).
- Mixed build (empty staticlib) links and runs identically to C-only on all three OSes.
- Header split merged; bindgen processes core headers standalone; PCH still works or is retired without build-time regression.

**Deletes:** `net_wipx.c`, `snd_mikmod.c`, `snd_xmp.c`, `snd_modplug.c`, `Windows/VisualStudio/` (sln/vcxproj).

---

## Phase 1 — Pure leaf utilities (S) `[~]`

Small, dependency-free code with outsized compatibility leverage. Crates: `quake-util`, `quake-math`, parts of `quake-types` (plus `quake-fs` for the wad logic per [PLAN.md §5](PLAN.md#5-workspace-design)).

**Status (2026-08-17):** implemented on `claude/rust-conversion-phase-1-7c0612`; every exit criterion is met except the C deletions, which are deliberately deferred (user decision) pending the [PLAN.md §3](PLAN.md#3-build-integration) MinGW/GNU-Makefile decision — deleting now would unlink `-Duse_rust=disabled` (the harness comparison oracle) and the C-only MinGW/clangarm64 legs.
- Done: all ten modules ported and wired behind the global `use_rust` switch (no per-module switches — Phase 1 names no soak window, and the leaves are non-interacting; per-module switches begin at Phase 2); differential suites in `rust/quake-ctest` (the C originals compiled as `c_ref_*` into the test binaries via `cc`, property tests + fixed corpora, bit-exact comparisons); ADR-005 formatter with stratified per-PR conformance CI on all three OSes plus a scheduled exhaustive sweep (all 2³² f32 patterns verified green on darwin-arm64, 52 min wall); first real FFI plumbing per ADR-011 (cbindgen-generated `quake_rs.h` in the build dir, committed script-generated bindgen imports with a CI regen-diff, `check_capi_signatures.sh` declaration-conflict gate); `-ffp-contract=off` pinned per ADR-010 (darwin-arm64 goldens regenerated — see the ADR-010 Phase 1 amendment); new behavior-neutral C seams: `COM_ThreadFileSize`/`COM_ThreadFileFromPak` accessors (bindgen cannot reach thread-locals).
- Deferred: the deletion wave + `c-reference/phase1` tag (blocked on the PLAN §3 decision: either GNU-ABI Rust comes in scope — `cargo --target` in `cargo_build.py`, a cargo step in the Makefiles — or those legs are retired; at deletion time the C files move to `rust/quake-ctest/csrc/` as frozen differential references); `%g`/`%e` formatter conversions (no engine user in the ported set; unreachable today since no C call site is wired to the Rust formatter, but they `panic!` rather than failing to compile — **Phase 6 must check the specifier set of every writer it moves onto `quake_util::printf`**, starting with `csprogsvers/%x.dat`); linux/windows registered-tier goldens (unchanged from Phase 0); c2rust oracle translation of `mathlib.c` (no toolchain on the dev host; the exhaustive differential FFI tests substitute); Miri CI job (run locally on the pure crates instead).

**Scope**
- `crc.c` (CRC16), `mdfour.c` (folded MD4 — the quirky fold is the `csprogsvers/%x.dat` key; preserve exactly).
- `mathlib.c`: vector ops, `AngleVectors`, `anglemod`, `VectorNormalize`, `BoxOnPlaneSide` **bit-exact** including the sign-bit fast path; `anorms.h` table.
- `hash_map.c` semantics port — progs symbol lookup relies on reverse-insertion first-match behavior; do **not** substitute `std::collections::HashMap` where match order is observable.
- `json.c`/`jsmn.h` hand-port (tolerant parser; MD5-metadata acceptance must not change — no serde_json, [ADR-012](adr/ADR-012-vendored-libs.md)).
- `cfgfile.c` (exact early-parse line format), `wad.c` (WAD2), `strlcpy/strlcat`, `q_ctype`.
- **C-printf-compatible float formatter** ([ADR-005](adr/ADR-005-printf-float-formatter.md)): `%f`/`%i`/`%g`-exact formatting used by `PR_UglyValueString`, savegame writing, cvar/config writing. Golden-tested against platform `snprintf` on all CI OSes (stratified + scheduled-exhaustive f32 sweep).

**Exit criteria**
- Each function differential-tested against its C original (property tests + golden vectors); formatter conformance suite green on all OSes.
- Harness parity unchanged. C files deleted.

**Deletes:** `crc.c`, `mdfour.c`, `mathlib.c`, `hash_map.c`, `json.c`, `jsmn.h`, `cfgfile.c`, `wad.c`, `strlcat.c`, `strlcpy.c`.

---

## Phase 2 — Infrastructure: memory + filesystem (M) `[~]`

**Status (2026-08-18):** implemented on `feature/rust-conversion-phase-2-165c44`.
- Done: behavior-neutral C split (`common_fs.c` holds the filesystem/localization/kpf half of `common.c`; the Steam API runtime moved to a new `steam_api.c`, `ChooseQuakeFlavor` to `sys_sdl.c`; `COM_Game_f`/`COM_CheckRegistered` stay in `common.c` as seams); Rust `#[global_allocator]` bound to the in-tree mimalloc via hand-declared `mi_*` (libmimalloc-sys rejected — see the ADR-013 amendment) behind the `engine-alloc` cargo feature, plus the `ScratchBuf` TEMP_ALLOC counterpart; the full fs port (pure logic in `quake-fs`: pak/searchpath/flavor/vdf/egs/loc + a zip reader pinned to the vendored miniz's accept/reject behavior; shims in `quake-capi` behind the `fs` feature) with the first per-module switch `-Duse_rust_fs` and a third CI harness configuration; a mid-run `game`-switch corpus entry (needed a headless-guard fix in `COM_Game_f`); ASan mixed-build smoke job; differential ctest suites and pak/wad/kpf/loc/vdf fuzz targets.
- Deferred: C deletion wave + `c-reference/phase2` tag (same PLAN §3 MinGW/GNU-Makefile blocker as Phase 1; `common_fs.c`/`steam.c` stay compiled under `-Duse_rust_fs=disabled` as the differential oracle); the Steam API runtime port (`steam_api.c`, ~340 lines of SDL-loaded achievements/rich-presence glue — natural Phase 9 platform material; amendment to the delete list below); `DO_USERDIRS` support in the Rust fs (a `userdirs` cargo feature is plumbed and the combination is a loud `compile_error!`); linux/windows registered-tier goldens (unchanged from Phase 0).

**Scope**
- `mem.c` shims: Rust `#[global_allocator]` bound to the **same** vendored mimalloc build; `Mem_Alloc`/`Mem_Free` remain the mandatory cross-boundary ownership API ([ADR-013](adr/ADR-013-allocator.md)). `TEMP_ALLOC` stack-alloc pattern gets a Rust equivalent (SmallVec-style or explicit scratch arenas).
- `quake-fs`: the COM_* file layer from `common.c` — searchpaths, `path_id` semantics, PAK loading (+CRC checks), gamedir/mission-pack/flavor logic (original vs remastered, Nightdive addon dir, shareware gating), `.kpf` zip reading + the localization escape parser (golden-tested against `QuakeEX.kpf`), Steam/GOG/EGS discovery (`steam.c`, path logic from `sys_sdl_win.c`/`sys_sdl_unix.c`).
- **Deliberately deferred:** `tasks.c` stays C until Phase 8 ([ADR-016](adr/ADR-016-task-system.md)) — it is the concurrency spine under the frame graph; porting it early is risk without compatibility payoff.

**Exit criteria**
- Engine boots and runs the harness with the Rust filesystem under both `-Duse_rust_fs` settings; searchpath/gamedir behavior identical on the mod bench (incl. flavor switching and localization).
- Fuzzers live for pak/wad/kpf inputs.

**Deletes:** filesystem portion of `common.c` (file splits into remaining-C `common_msg.c` etc. as needed), `steam.c` *(amended in Phase 2: the achievements/rich-presence runtime half of `steam.c` moved to `steam_api.c`, which stays C until Phase 9 — only the discovery half was ported and is deleted here)*.

---

## Phase 3 — Formats & assets (M/L) `[~]`

Crates: `quake-formats`, `quake-image`. Pure functions over byte slices — callable from C worker threads during parallel model loading (loaders must be `Send`).

**Status (2026-08-24):** implemented (M1–M8 of `docs/ai/plans/rust-conversion-phase-3.md`); every exit criterion is met except the C deletions, deferred exactly like Phases 1–2 (PLAN §3 MinGW blocker) — `model_parse.c`, `image_decode.c` and `image_stb.c` stay compiled as the differential oracle behind `-Duse_rust_formats=disabled` / `-Duse_rust_image=disabled`.
- Done: behavior-neutral C splits; PCX/LMP, all five BSP dialects (+`.lit`/`.vis`/`.ent`, PVS, hulls, submodels), MDL, SPR, MD3, MD5 ported with bit-exact differential suites; threaded-loading proof; differential fuzzing (14 targets) + the `formats_corpus` real-asset gate (bit-for-bit over the full local depot; shareware id1 + the in-repo vq_pak PNGs as the standing CI legs); PNG/TGA/JPG decode via `quake-image` (TGA hand-port and a hand-ported stb acceptance walk over the `png` crate, both bit-exact; JPEG via `zune-jpeg` under the owner-relaxed delta-bounded gate — see the ADR-012 M8 amendment; PNG *encode* stays lodepng). First third-party runtime crates per the ADR-003 M8 amendment.
- Deferred: the C deletion wave + `c-reference/phase3` tag (PLAN §3); MD3 real-asset coverage (no `.md3` exists anywhere in the depot — synthetic-only, revisit with the wider mod bench); registered-tier goldens for linux/windows (unchanged from Phase 0).

**Scope**
- BSP family: 29, 30 (Valve, incl. palette/sky/lighting differences), BSP2, 2PSB, Quake64; `.lit` (strict size check), external `.vis`, external `.ent` (incl. `@crc` naming); PVS decompression, hull setup, submodels.
- MDL v6 — producing **byte-exact `aliashdr_t`** and mesh data, since C (`gl_mesh.c`, renderer) remains the consumer this phase; MD3; MD5 (`.md5mesh`/`.md5anim`; the MD5 loader in this tree is a hand-rolled text parser with no JSON metadata — the Phase-1 `json.c` port is not involved); SPR; LMP/QPIC; PCX; TGA.
- `quake-image`: PNG/JPG/TGA decode via `png`/`image` crates behind the `Image_Load*` seam, gated on a pixel-exact decode corpus vs stb/lodepng; PNG **encode** (screenshots) stays on lodepng until parity is accepted ([ADR-012](adr/ADR-012-vendored-libs.md)).
- Palette/colormap loading (`palette.c` data stays; loading logic ports).

**Exit criteria**
- Full asset corpus (id1 + mission packs + re-release + mod bench) parses **bit-identically** (hash of parsed structs vs C).
- Accept/reject parity on the fuzz corpora (differential fuzzing C-via-FFI vs Rust).
- Harness parity unchanged; parallel model loading works with Rust loaders on C task workers.

**Deletes:** parsing portions of `gl_model.c` (rendering-side mesh upload stays C until Phase 8), `image.c` decode paths, `modelgen.h`/`spritegn.h` C usage migrates to `quake-types`.

---

## Phase 4 — Sound (M) `[~]`

Crate: `quake-snd` (+ the first `quake-platform` module). Task plan: `docs/ai/plans/rust-conversion-phase-4.md`.

**Scope** *(all ported behind `-Duse_rust_snd`, auto-follows `use_rust`)*
- [x] `snd_mem.c` (sfx cache/resample), `snd_mix.c` (software mixer — PCM-hash parity on fixed inputs via the `-sndhash` harness instrument and the `snd_mix_differential` ctest), `snd_dma.c` (channels, spatialization; compat storage stays C in `snd_glue.c` for the menu.c/cl_demo.c readers), `bgmusic.c`, `snd_mp3tag.c` (ID3/APE/Lyrics3/MusicMatch tag skip), UMX container port (behind the `codec-umx` cargo feature, mirroring `USE_CODEC_UMX`, which no Meson config enables).
- [x] Codec framework: the Rust registry operates on the `snd_codec_t` vtable directly (the ADR-014 mirror); **C codec libraries stay** (mpg123/mad, vorbis, opus, flac, + libogg) behind it ([ADR-014](adr/ADR-014-audio-codecs.md)); WAV is a Rust-native codec; per-codec Symphonia/lewton swaps remain a later, optional, non-compat change.
- [~] SDL audio backends: `snd_sdl3.c` ported to `quake-platform` over the `sdl3` crate ([ADR-017](adr/ADR-017-sdl2-sdl3.md)); the SDL2 backend `snd_sdl.c` **stays C until a use_rust+SDL2 CI leg exists to verify its Rust port** (SDL2 dev libraries are absent from the current dev/CI environments).

**Exit criteria** *(met on darwin-arm64; linux/windows goldens ride the Phase 0 registered-tier deferral)*
- [x] PCM-hash parity on the demo corpus: `run_corpus.py --sndhash` goldens (darwin-arm64), golden-checked by the macOS harness job and cross-checked C↔Rust by the Linux `--compare` legs — both **shareware tier**. The committed synthetic-WAV music entry (`music-wav`) is **`registered` tier and therefore local-only**: `-game <anything>` sets `com_modified` (→ `COM_CheckRegistered` `Sys_Error`), and independently `COM_FindFile` refuses any loose path containing `/` while `registered == 0`, so external music is unreachable on shareware data — a shareware-tier music entry would go green while testing nothing. Per-codec decode comparisons ran engine-level against the same C decoder libraries (WAV natively; vorbis + flac locally over registered-tier data — lossy fixtures need encoders CI lacks).
- [x] Harness parity unchanged (demo state hash, savegame byte-diff, all per-module oracle configs incl. the new `build-rs-csnd`).

**Deletes** *(recorded, deferred like Phases 1–3 on the PLAN §3 MinGW decision; the C stays the `-Duse_rust_snd=disabled` oracle)*: `snd_mem.c`, `snd_mix.c`, `snd_dma.c`, `bgmusic.c`, `snd_codec.c`, `snd_wave.c`, `snd_umx.c`, `snd_mp3tag.c`, `cd_sdl.c` (dead) + `cd_null.c`, codec wrapper files as each is rewrapped. `snd_sdl.c` additionally stays live as the SDL2 backend.

---

## Phase 5 — Networking wire layer (L) `[~]`

Crate: `quake-net`. Protocol *logic* in `cl_parse.c`/`sv_main.c` stays C this phase — the wire layer beneath it becomes Rust. Task plan: `docs/ai/plans/rust-conversion-phase-5.md`.

**Status (2026-08-25):** M1–M5 of the task plan implemented (scaffolding + ADR-011 net mirrors; MSG/SZ port with the section split into `net_msg.c` and flipped whole-file under `-Duse_rust_net`; demo file format; loopback driver flipped in vtable slot 0). Gates green on darwin-arm64: corpus `--check`/`--compare` (C vs mixed and vs the `build-rs-cnet` oracle), savegame byte-diff, `record_diff.py` .dem byte-identity, calibrated live-capture diff, `net_msg`/`net_loop` ctest differentials, fuzzers (`fuzz_net_msg`, `fuzz_net_demo`). M6 (dgrm reliable-layer port, ctest-first: net_dgrm.c split, quake-net::dgrm over a NetSys trait, differential + fuzz gates) done 2026-08-26. M7 done 2026-08-26 (M7a dgrm engine flip via net_dgrm_glue.c; M7b socket2 UDP landriver flipped in net_bsd.c -- net_wins.c stays C pending a Windows UDP runtime CI leg, ADR-017 precedent). M8 (netreplay byte gate + 4-way interop matrix, CI-wired) and M9 (net_main.c ADR-009-safe core ported; dispatch funnels stay C frames until Phase 7 -- see the task plan audit note) done 2026-08-26. M10 phase exit done 2026-08-26 (fuzz soak clean; fresh-context M6-M9 compatibility review, all findings addressed -- see the task plan).

**Scope**
- `MSG_Read*/Write*` + `SZ_*` from `common.c`: coord 16/24/32f by `protocolflags`, angle variants, varint u64, `MSG_WriteEntity` pext2-aware encoding, `ENTALPHA`/`ENTSCALE` 4.4 fixed-point rounding — golden-tested byte-for-byte.
- Drivers, ported one at a time into the existing `net_drivers[]`/`net_landrivers[]` vtables: loopback first, then the dgrm reliable layer (`NETFLAG_*`, `NET_HEADERSIZE`, sequencing), CCREQ/CCREP connectionless protocol including rcon, then UDP IPv4/IPv6 (v6only, `ff03::1` multicast discovery, Linux address enumeration) via `socket2`/thin `libc`.
- Demo file IO (`cl_demo.c` read/write path): forcetrack line + [length + 3 viewangle floats + payload] records, `fflush` cadence, resume-record `-17` seek, seek/prespawn bookkeeping.
- Host cache / `NET_Poll` plumbing (`net_main.c`).

**Exit criteria** *(met 2026-08-26 on darwin-arm64 + the Linux CI legs; see the caveats)* — the phase stays `[~]` like Phases 1–4: its exit criteria are met and its milestones are complete, but the C originals stay compiled as the `-Duse_rust_net=disabled` oracle and the deletion wave is deferred, so the phase is not *closed out*.
- [x] 4-way interop matrix green (C/Rust client × C/Rust server, localhost) across 15/666/999 × PRFL × PEXT combinations: `interop_matrix.py`, 6 protocol cells (`Base-`/`FTE+` × 15/666/999 — FTE+999 exercises PRFL_FLOATCOORD|SHORTANGLE, Base-999 PRFL_INT32COORD|SHORTANGLE, the FTE+ cells PEXT2) × 4 build combos, all green incl. a local-only `[::1]` IPv6 leg over the Rust UDP6 landriver. PRFL_24BITCOORD is not producible by this engine's server; it is covered at the MSG layer by `net_msg_differential`. The CI leg runs the IPv4 matrix on shareware data.
- [x] Captured-session replay and demo recording byte-identical vs C: `netreplay_diff.py` (the `-netreplay` instrument replays a `-netcapture` recv stream deterministically; state-hash chains and a demo recorded mid-replay byte-identical C-vs-Rust, with a delivered-record floor so an inert replay cannot pass) plus `record_diff.py` loopback record byte-identity. Stated precisely: the replay hook returns above the driver vtable, so its byte gate exercises the MSG/SZ readers, cl_parse and the demo writer; the flipped dgrm/UDP drivers themselves are gated by the ctest differentials, the loopback `record_diff` byte gate, the calibrated structural `capture_diff.py` live gate (its ~1-byte run-to-run timing noise is measured C-vs-C, per the task-plan M3 amendment), and the interop matrix counts.
- [x] Net message reader fuzzers live (per protocol): `fuzz_net_msg` (protocol 15/666/999 × PRFL × PEXT2 flag sets), `fuzz_net_dgrm` (both reliable-layer RX paths), `fuzz_net_ccreq` (the CCREQ/CCREP/slist read sequences), `fuzz_net_demo` — all in the CI fuzz job; exit soak clean.

**Carried to later phases** *(recorded at M10; each item is also listed in the receiving phase's Scope and Deletes)*: `net_wins.c` keeps the C WINS drivers until a Windows UDP runtime CI leg exists (ADR-017 precedent); the `Host_Error`-capable dispatch funnels in `net_main.c` (NET_Connect/GetMessage/GetServerMessage/Send\*/SendToAll/Poll, NET_Init/Shutdown) and `net_dgrm.c`'s orchestration half (connect handshake, `_Datagram_ServerControlPacket`, hostcache/heartbeats/rcon) stay C frames until Phase 7 statusizes the layers beneath them (task-plan M9 audit).

**Deletes** *(recorded, deferred like Phases 1–4; the C stays the `-Duse_rust_net=disabled` oracle)*: `net_loop.c`, `net_msg.c` (the MSG portion already split out of `common.c`), `net_dgrm_rel.c`, `net_udp.c`, and the ported (`#ifndef USE_RUST_NET`) sections of `net_main.c`. `net_dgrm.c`'s orchestration half, the `net_main.c` funnels, `net_wins.c`, `net_bsd.c`/`net_win.c` (vtable data) move to the Phase 7/9 deletion lists per the carve-outs above.

---

## Phase 6 — Progs VM (L — highest compatibility risk) `[ ]`

Crate: `quake-progs`. Done **before** client/server so Phase 7 sits on the Rust VM. Near-transliteration first; idiomatize only after trace parity.

**Scope**
- Loader (`pr_edict.c`): `PROG_VERSION 6`, `PROGHEADER_CRC 5927` (+ known-foreign-CRC diagnostics), byteswap rules, CRC16 + folded-MD4 recording, **`PR_MergeEngineFieldDefs`** (exact append order; synthesized `colormod_x/_y/_z` defs), reverse-built symbol hash maps, string table with negative engine strings.
- **Edict arena** ([ADR-006](adr/ADR-006-edict-arena.md)): runtime-ABI layout, per-profile header differences, `ED_Alloc` FIFO free-list with `MAX_EDICT_FREETIME_ALWAYS_REUSE` semantics (entity numbering is observable).
- Interpreter (`pr_exec.c`, 590 lines — small but every opcode's semantics matters): raw `OP_DIV_F`, float→int truncation, localstack raw copies, runaway counter, trace hooks matching the C `-Dtrace` format.
- `ED_Write`/`ED_WriteGlobals`/`ED_ParseEdict`/`ED_ParseGlobals` with the Phase-1 formatter and all skip rules; `ED_ParseEdict` quirks (bare-token vector coercion, `"entity "` prefix handling).
- Builtins in batches behind per-builtin dispatch (C and Rust coexist mid-phase): `pr_cmds.c` core table (ordinals are ABI; `PF_random`, `PF_aim`, `PF_changeyaw` exact), then the `pr_ext.c` long tail (extension table with name-vs-number resolution and don't-clobber rule; extension advertisement predicates; `pr_dumpplatform`). Note: `pr_ext.c`'s QC polygon-drawing builtins call the renderer — those shim through `quake-capi` to C rendering until Phase 8.
- Re-release: `PR_PatchRereleaseBuiltins` renumbering, `PR_FindSupportedEffects`/`EF_QEX_*` masking, `ex_*` stubs.
- Both QCVMs (sv + cl/CSQC) with the ambient-switch boundary shim ([ADR-008](adr/ADR-008-ambient-qcvm.md)); CSQC load chain (`csprogsvers/%x.dat` → `csprogs.dat` → `progs.dat`, `CSQC_DrawHud` requirement).

**Exit criteria**
- **Instruction-level trace parity** with the C build across the full corpus (all mission packs, re-release, mod bench incl. CSQC users).
- Builtin-table dump diff clean for every progs.dat in the corpus.
- Savegame byte-diff clean (saves flow through `ED_Write` + formatter); C-era save load-compat both directions.
- Demo-hash parity unchanged.

**Deletes:** `pr_edict.c`, `pr_exec.c`, `pr_cmds.c`, `pr_ext.c` (progressively), `progs.h` C usage migrates to `quake-types`/`quake-progs`.

---

## Phase 7 — Client & server simulation (L) `[ ]`

Crates: `quake-host` (lib for now), remaining parts of `quake-net`, `quake-cvar`.

**Scope**
- Server: `sv_phys.c` (incl. pusher-support-frame subsystem modes 0–3, `sv_smoothplatformlerps`, gravity half-step), `sv_user.c` (libm trig call-through per [ADR-010](adr/ADR-010-determinism-policy.md)), `world.c` (**both** hull-check implementations behind `sv_fte_recursivehullckeck`; `SV_LinkEdict`/`SV_TouchLinks` area-node ordering), `sv_move.c`, `sv_main.c` (protocol negotiation, FTE delta writer, `MSG_WriteStaticOrBaseLine`), `sv_user.c` client-move reads.
- Client: `cl_main.c` (lerp, relink), `cl_parse.c` (svc dispatch incl. collision disambiguation), `cl_input.c`, `cl_tent.c`, `cl_demo.c` playback logic, `view.c`, `chase.c`.
- **Carried over from Phase 5** (see its "Carried to later phases" note): the `Host_Error`-capable networking dispatch funnels left as C frames by the Phase 5 M9 ADR-009 audit — `NET_Connect`, `NET_GetMessage`, `NET_GetServerMessage`, `NET_SendMessage`/`NET_SendUnreliableMessage`, `NET_CanSendMessage`, `NET_SendToAll`, `NET_Poll`/`SchedulePollProcedure`, `NET_Init`/`NET_Shutdown` (`net_main.c`) — plus `net_dgrm.c`'s orchestration half (connect handshake, `_Datagram_ServerControlPacket` and its `SV_ConnectClient` path, hostcache/slist, heartbeats, rcon). They become portable once this phase statusizes the strata beneath them; `PollProcedure` is already ABI-mirrored.
- Host: `host.c` (fixed-step accumulator, `Host_FilterTime`, frame orchestration — setjmp remains C-side until Phase 9), `host_cmd.c` (savegame writer/reader — the byte-diff gate's subject — plus all game commands), `cmd.c`/`cvar.c` (registry with C-callable callbacks), `keys.c`, `console.c`, `menu.c`, `sbar.c`, `r_part.c`/`r_part_fte.c` particle *simulation* (their Vulkan buffer code stays C until Phase 8).
- `config.cfg` writer (exact order) + boot sequence (`quake.rc`, `autoexec.cfg`).

**Exit criteria**
- Long-demo state-hash parity per platform across the corpus; physics determinism suite green (incl. pusher modes 0–3 and both hullcheck impls).
- Netplay soak: mixed C/Rust client-server sessions across protocol matrix, hours-long, no desync.
- Savegame/config byte-diff clean end-to-end (now generated by Rust code).

**Deletes:** `sv_*.c`, `world.c`, `cl_*.c`, `view.c`, `chase.c`, `host.c`*, `host_cmd.c`, `cmd.c`, `cvar.c`, `keys.c`, `console.c`, `menu.c`, `sbar.c` (*host.c's setjmp shell survives minimally until Phase 9), plus the Phase 5 carry-overs: the remaining C in `net_main.c` (the dispatch funnels + `NET_Init`/`NET_Shutdown`) and `net_dgrm.c`'s orchestration half.

---

## Phase 8 — Renderer + task system (XL) `[ ]`

Crates: `quake-tasks`, `quake-render`. The largest phase; sub-slices land behind individual switches.

**Scope — in order**
1. **`quake-tasks`** ([ADR-016](adr/ADR-016-task-system.md)): work-stealing scheduler on `crossbeam-deque`, API-compatible `Task_*` C shims, task-graph dependency semantics, indexed tasks (parallel-for), 32-worker cap + `-pinnedworkers` affinity, `Task_Join` timeout. Validated with loom models + TSan, then swapped under the C renderer.
2. **`gl_heap.c`** GPU suballocator: pure logic; property-tested against the C implementation on randomized allocation traces (it has a self-test to port too).
3. **`gl_texmgr.c`** texture manager.
4. **`gl_rmisc.c`**: descriptor layouts, samplers, pipeline creation (all ~16 `R_Create*Pipelines` families incl. specialization constants), staging + dynamic ring buffers, `vulkanglobals_t` becomes Rust-owned with a C-layout view during the phase.
5. **`gl_vidsdl.c`**: instance/device/swapchain/present via `ash` — same extension set (ray-query bundle, present-wait2, full-screen-exclusive on Windows), same loading path (`SDL_Vulkan_GetVkGetInstanceProcAddr`), MoltenVK direct link on macOS; video modes + video menu glue.
6. Draw layer: `gl_draw.c`, `gl_sky.c`, `gl_warp.c`, `gl_fog.c`, `gl_rlight.c`, `gl_refrag.c`, `r_sprite.c`, `gl_screen.c` 2D orchestration.
7. World/mesh: `r_world.c` (SIMD culling via `std::arch` SSE/NEON — cull decisions must match C on captured frames), `r_brush.c` (lightmaps, indirect draws), `gl_mesh.c`, `r_alias.c`, particle rendering for `r_part.c`/`r_part_fte.c`, QC polygon builtin rendering (retiring the Phase-6 shim).
8. Frame graph (`gl_rmain.c`/`gl_screen.c`): ~20 tasks/frame, 6-way parallel secondary command-buffer recording, identical dependency structure.
9. Ray tracing: BLAS build (`gl_mesh.c`), TLAS + scratch (`r_brush.c`), ray-query lightmap shadows, `r_rtshadows` paths.
10. Shader pipeline + embedded pak move to `xtask` (`include_bytes!`), retiring `bintoc`, `mkpak`, and 61 generated TUs.

**Policy:** port-then-modernize ([ADR-015](adr/ADR-015-renderer-port-then-modernize.md)) — keep render passes/subpasses, keep `gl_heap` (no VMA), no dynamic rendering, no bindless. Modernization is future work after the port.

**Exit criteria**
- Screenshot SSIM corpus within tolerance on all three OSes; Vulkan validation layers clean; lavapipe/SwiftShader CI smoke green.
- `timedemo` benchmarks within regression thresholds vs C on reference hardware.
- Task system passes loom/TSan suites; frame-graph parallelism preserved (worker utilization comparable).
- Mixed-build switches removed; renderer fully Rust.

**Deletes:** all `gl_*.c`, `r_*.c`, `tasks.c`, `atomics.h` C usage, `Shaders/bintoc.c`, `Misc/vq_pak/mkpak.c`.

---

## Phase 9 — Host inversion + platform (M) `[ ]`

Crates: `quake-host` (becomes the bin), `quake-platform`.

**Scope**
- **Carried over from Phase 5**: `net_wins.c` (the Windows WINS landrivers, deferred at M7b because Windows CI has no UDP runtime leg — the ADR-017 precedent) and the per-platform driver-table files `net_bsd.c`/`net_win.c`, which are platform glue and so belong with the rest of the platform layer.
- `main()` moves to Rust: `main_sdl.c` loop (client + dedicated), `Sys_*` layers (`sys_sdl.c`, `sys_sdl_unix.c`, `sys_sdl_win.c` — incl. crash handler policy), `pl_win.c`/`pl_linux.c`/`pl_osx.m` equivalents (window icon, clipboard, error dialogs; the ObjC file's functionality via `objc2` or a tiny kept-C/ObjC stub), input (`in_sdl*.c` with the SDL2/SDL3 feature gate and the scancode table).
- Error-path completion: last `setjmp`/`longjmp` removed; `HostError` Results end-to-end ([ADR-009](adr/ADR-009-error-handling.md)).
- `quakedef.h` retired; remaining C compiles as `libquake_c.a` against explicit headers.
- Build: Meson orchestration either retained (thin) or replaced by cargo + `cc` for the C remnant — decide by remnant size.

**Exit criteria**
- Full harness green with Rust `main()` on all three OSes; packaging (AppImage, Windows installer) reproduced.
- One release-cycle soak with the C-only build still available in CI, then C-only CI retired and the final C-capable commit tagged (`c-reference/final`).

**Deletes:** `main_sdl.c`, `sys_sdl*.c`, `pl_*.{c,m}`, `in_sdl*.c`, `quakedef.h` and the PCH machinery, plus the Phase 5 carry-overs `net_wins.c`, `net_bsd.c` and `net_win.c`.

---

## Phase 10 — Remnant minimization + idiomatic pass (M, ongoing) `[ ]`

**Scope**
- Remaining native modules, each isolated behind an explicit interface ([ADR-002](adr/ADR-002-c-not-cpp-fallback.md)): vendored **mimalloc** (revisit allocator choice — [ADR-013](adr/ADR-013-allocator.md)), **audio codec libraries** (optional per-codec Symphonia migration — [ADR-014](adr/ADR-014-audio-codecs.md)), **lodepng encoder** if pixel-parity for PNG writing was never accepted.
- Idiomatic debt pass: remove dual-view globals and remaining `quake-capi` shims; convert `#[repr(C)]` structs with no external viewers to idiomatic types; lifetime-scope `CbContext`; clippy pedantic tier; revisit ADR exceptions to see which can now be closed.
- `quake-c-sys`/`quake-capi` shrink toward empty; document what remains and why.

**Exit criteria**
- Enumerated, justified list of surviving native code in an updated ADR-002 appendix.
- All temporary shims gone; unsafe inventory reviewed and minimized; harness green.

---

## Phase-ordering rationale (summary)

Bottom-up along the dependency graph, compatibility-risk-first for the sim: utilities before infrastructure before formats; **VM (P6) before sim (P7)** so client/server porting sits on a trace-verified Rust VM; **renderer (P8) after sim** because it is the largest, least compatibility-critical (pixels, not bytes) chunk and its data structures (`aliashdr_t`, `vulkanglobals_t`) stay stable while everything beneath them moves; **tasks with the renderer** because the frame graph is its main client; **host/platform inversion last (P9)** because `main()` ownership is most valuable once nearly everything it drives is already Rust.
