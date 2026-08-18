# vkQuake C → Rust Migration Roadmap

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

**Status (2026-08-17):** implemented on `claude/rust-conversion-phase-1-7c0612`; every exit criterion is met except the C deletions, which are deliberately deferred (user decision) pending the [PLAN.md §3](PLAN.md#3-platform--build-matrix) MinGW/GNU-Makefile decision — deleting now would unlink `-Duse_rust=disabled` (the harness comparison oracle) and the C-only MinGW/clangarm64 legs.
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

## Phase 2 — Infrastructure: memory + filesystem (M) `[ ]`

**Scope**
- `mem.c` shims: Rust `#[global_allocator]` = `libmimalloc-sys` bound to the **same** vendored mimalloc build; `Mem_Alloc`/`Mem_Free` remain the mandatory cross-boundary ownership API ([ADR-013](adr/ADR-013-allocator.md)). `TEMP_ALLOC` stack-alloc pattern gets a Rust equivalent (SmallVec-style or explicit scratch arenas).
- `quake-fs`: the COM_* file layer from `common.c` — searchpaths, `path_id` semantics, PAK loading (+CRC checks), gamedir/mission-pack/flavor logic (original vs remastered, Nightdive addon dir, shareware gating), `.kpf` zip reading + the localization escape parser (golden-tested against `QuakeEX.kpf`), Steam/GOG/EGS discovery (`steam.c`, path logic from `sys_sdl_win.c`/`sys_sdl_unix.c`).
- **Deliberately deferred:** `tasks.c` stays C until Phase 8 ([ADR-016](adr/ADR-016-task-system.md)) — it is the concurrency spine under the frame graph; porting it early is risk without compatibility payoff.

**Exit criteria**
- Engine boots and runs the harness with the Rust filesystem under both `-Duse_rust_fs` settings; searchpath/gamedir behavior identical on the mod bench (incl. flavor switching and localization).
- Fuzzers live for pak/wad/kpf inputs.

**Deletes:** filesystem portion of `common.c` (file splits into remaining-C `common_msg.c` etc. as needed), `steam.c`.

---

## Phase 3 — Formats & assets (M/L) `[ ]`

Crates: `quake-formats`, `quake-image`. Pure functions over byte slices — callable from C worker threads during parallel model loading (loaders must be `Send`).

**Scope**
- BSP family: 29, 30 (Valve, incl. palette/sky/lighting differences), BSP2, 2PSB, Quake64; `.lit` (strict size check), external `.vis`, external `.ent` (incl. `@crc` naming); PVS decompression, hull setup, submodels.
- MDL v6 — producing **byte-exact `aliashdr_t`** and mesh data, since C (`gl_mesh.c`, renderer) remains the consumer this phase; MD3; MD5 (`.md5mesh`/`.md5anim` + JSON metadata via the Phase-1 parser); SPR; LMP/QPIC; PCX; TGA.
- `quake-image`: PNG/JPG/TGA decode via `png`/`image` crates behind the `Image_Load*` seam, gated on a pixel-exact decode corpus vs stb/lodepng; PNG **encode** (screenshots) stays on lodepng until parity is accepted ([ADR-012](adr/ADR-012-vendored-libs.md)).
- Palette/colormap loading (`palette.c` data stays; loading logic ports).

**Exit criteria**
- Full asset corpus (id1 + mission packs + re-release + mod bench) parses **bit-identically** (hash of parsed structs vs C).
- Accept/reject parity on the fuzz corpora (differential fuzzing C-via-FFI vs Rust).
- Harness parity unchanged; parallel model loading works with Rust loaders on C task workers.

**Deletes:** parsing portions of `gl_model.c` (rendering-side mesh upload stays C until Phase 8), `image.c` decode paths, `modelgen.h`/`spritegn.h` C usage migrates to `quake-types`.

---

## Phase 4 — Sound (M) `[ ]`

Crate: `quake-snd`.

**Scope**
- `snd_mem.c` (sfx cache/resample), `snd_mix.c` (software mixer — PCM-hash parity on fixed inputs), `snd_dma.c` (channels, spatialization), `bgmusic.c`, `snd_mp3tag.c` (ID3/APE tag skip), UMX container port.
- Codec framework: Rust trait mirroring the `snd_codec_t` vtable; **C codec libraries stay** (libmad or mpg123, vorbis, opus, flac, + libogg) behind it ([ADR-014](adr/ADR-014-audio-codecs.md)); per-codec Symphonia/lewton swaps are a later, optional, non-compat change.
- SDL audio backends (`snd_sdl.c`/`snd_sdl3.c`) move to `quake-platform` glue (SDL2 + SDL3 feature-gated).

**Exit criteria**
- PCM-hash parity on a demo-soundtrack corpus (mixer) and per-codec decode comparisons.
- Harness parity unchanged.

**Deletes:** `snd_mem.c`, `snd_mix.c`, `snd_dma.c`, `bgmusic.c`, `snd_codec.c`, `snd_wave.c`, `snd_umx.c`, `snd_mp3tag.c`, `cd_sdl.c` (dead) + `cd_null.c`, codec wrapper files as each is rewrapped.

---

## Phase 5 — Networking wire layer (L) `[ ]`

Crate: `quake-net`. Protocol *logic* in `cl_parse.c`/`sv_main.c` stays C this phase — the wire layer beneath it becomes Rust.

**Scope**
- `MSG_Read*/Write*` + `SZ_*` from `common.c`: coord 16/24/32f by `protocolflags`, angle variants, varint u64, `MSG_WriteEntity` pext2-aware encoding, `ENTALPHA`/`ENTSCALE` 4.4 fixed-point rounding — golden-tested byte-for-byte.
- Drivers, ported one at a time into the existing `net_drivers[]`/`net_landrivers[]` vtables: loopback first, then the dgrm reliable layer (`NETFLAG_*`, `NET_HEADERSIZE`, sequencing), CCREQ/CCREP connectionless protocol including rcon, then UDP IPv4/IPv6 (v6only, `ff03::1` multicast discovery, Linux address enumeration) via `socket2`/thin `libc`.
- Demo file IO (`cl_demo.c` read/write path): forcetrack line + [length + 3 viewangle floats + payload] records, `fflush` cadence, resume-record `-17` seek, seek/prespawn bookkeeping.
- Host cache / `NET_Poll` plumbing (`net_main.c`).

**Exit criteria**
- 4-way interop matrix green (C/Rust client × C/Rust server, localhost) across 15/666/999 × PRFL × PEXT combinations.
- Captured-session replay and demo recording byte-identical vs C.
- Net message reader fuzzers live (per protocol).

**Deletes:** `net_loop.c`, `net_dgrm.c`, `net_udp.c`, `net_wins.c`, `net_main.c`, `net_bsd.c`, `net_win.c`, MSG portion of `common.c`.

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
- Host: `host.c` (fixed-step accumulator, `Host_FilterTime`, frame orchestration — setjmp remains C-side until Phase 9), `host_cmd.c` (savegame writer/reader — the byte-diff gate's subject — plus all game commands), `cmd.c`/`cvar.c` (registry with C-callable callbacks), `keys.c`, `console.c`, `menu.c`, `sbar.c`, `r_part.c`/`r_part_fte.c` particle *simulation* (their Vulkan buffer code stays C until Phase 8).
- `config.cfg` writer (exact order) + boot sequence (`quake.rc`, `autoexec.cfg`).

**Exit criteria**
- Long-demo state-hash parity per platform across the corpus; physics determinism suite green (incl. pusher modes 0–3 and both hullcheck impls).
- Netplay soak: mixed C/Rust client-server sessions across protocol matrix, hours-long, no desync.
- Savegame/config byte-diff clean end-to-end (now generated by Rust code).

**Deletes:** `sv_*.c`, `world.c`, `cl_*.c`, `view.c`, `chase.c`, `host.c`*, `host_cmd.c`, `cmd.c`, `cvar.c`, `keys.c`, `console.c`, `menu.c`, `sbar.c` (*host.c's setjmp shell survives minimally until Phase 9).

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
- `main()` moves to Rust: `main_sdl.c` loop (client + dedicated), `Sys_*` layers (`sys_sdl.c`, `sys_sdl_unix.c`, `sys_sdl_win.c` — incl. crash handler policy), `pl_win.c`/`pl_linux.c`/`pl_osx.m` equivalents (window icon, clipboard, error dialogs; the ObjC file's functionality via `objc2` or a tiny kept-C/ObjC stub), input (`in_sdl*.c` with the SDL2/SDL3 feature gate and the scancode table).
- Error-path completion: last `setjmp`/`longjmp` removed; `HostError` Results end-to-end ([ADR-009](adr/ADR-009-error-handling.md)).
- `quakedef.h` retired; remaining C compiles as `libquake_c.a` against explicit headers.
- Build: Meson orchestration either retained (thin) or replaced by cargo + `cc` for the C remnant — decide by remnant size.

**Exit criteria**
- Full harness green with Rust `main()` on all three OSes; packaging (AppImage, Windows installer) reproduced.
- One release-cycle soak with the C-only build still available in CI, then C-only CI retired and the final C-capable commit tagged (`c-reference/final`).

**Deletes:** `main_sdl.c`, `sys_sdl*.c`, `pl_*.{c,m}`, `in_sdl*.c`, `quakedef.h` and the PCH machinery.

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
