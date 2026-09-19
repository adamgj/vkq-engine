# Feature task plan: Rust migration — soak-exit deletion PR + Phase 10 post-deletion tranche (M5–M10)

Status: approved 2026-09-19
Baseline: `origin/master` fb372b7d (PR #44; C tree identical to 7eda85da/PR #42).
Roadmap phase: Phase 9 close (deletion PR) + Phase 10 M5–M10.
Authority: `docs/rust-migration/PLAN.md` > `ROADMAP.md` > ADRs > `docs/ai/plans/rust-conversion-phase-10.md` > this plan.

## Context

Every engine module is already ported behind `use_rust_*` switches; the C build survives only as the differential oracle. The roadmap gates the rest on a "soak exit" (a tagged release on three OSes). No tag or release exists. The user has asked, after that gate was raised twice, to implement the remaining conversion work anyway. Four decisions were put to the user and declined without answer; this plan proceeds on the **recommended defaults**, recorded below as D-A..D-D, and they are the first things to override at review.

What remains, from `docs/rust-migration/c-remnant-inventory.md`: 81 oracle TUs (88,510 lines) to delete, 55 glue TUs (21,199) to remove TU by TU, 19 shared engine TUs (23,421) to port, the ADR-009 error-path end state (Host_Error in Rust, 19 setjmp sites), ~91 dual-view `#[no_mangle] pub static`s in `quake-capi`, `repr(C)`/`COMPILE_TIME_ASSERT` retirement, `quakedef.h`/PCH retirement, the ADR-013 allocator decision, SDL2 audio, and the M10 re-cut. Permanent remnants by ADR: codec bridges (ADR-014), lodepng + stb hosts (ADR-012), harness TUs (ADR-019), possibly mimalloc.

## User decisions taken as defaults (roadmap/ADR amendments, written in M0)

- **D-A Soak gate waived.** Tag `c-reference/final` = `origin/master` head at M0 start (fb372b7d). Evidence standing in for the release soak: green PR #41/#42 CI matrices (three-OS harness, `--compare` 8/8 identical on every oracle leg). Edits: ROADMAP Phase 9 D8 paragraph gets an "Amended (2026-09-19)" note; ADR-019 records criterion (b) waived and the rebuild recipe `git checkout c-reference/final && meson setup build-c -Duse_rust=disabled ...`.
- **D-B GNU Makefile builds retired.** Delete `Quake/Makefile`, `Makefile.w64`, `Makefile.w64a`, `.github/workflows/build-mingw.yml`, `build-msys2-clangarm64.yml`. They are pure C with no Rust path (PLAN §3 offered retirement; ADR-018 item 3 is the MSVC precedent). ADR-018 gains item 4; PLAN §3 gets a one-line pointer.
- **D-C Linux identity gate = goldens.** Linux CI has no goldens and used `--compare build-c`. Add a `workflow_dispatch` job that builds the C-only tag and runs `run_corpus.py --generate --tier shareware`; commit `Misc/harness/goldens/linux-x86_64/` (MANIFEST pins runner image/gcc/SDL2/commit). Linux then runs `--check` like Windows/macOS.
- **D-D SDL2 audio ported.** `snd_sdl.c` → `quake-platform::snd_sdl2` over `sdl2::sys` (crate 0.38, MIT, already in `rust/Cargo.lock:628`, `default-features=false`). The ADR-017 Phase 4 amendment's blocker ("no SDL2 in CI") is already false: `build-linux.yml:83` installs `libsdl2-dev` and the Linux harness builds are `-Duse_sdl3=disabled`. ADR-017 gets a closing amendment.

## Requirements and non-goals

- R1 After M0: one Meson configuration per {OS, SDL, buildtype, trace}; `git grep 'USE_RUST\|use_rust'` outside `docs/` and Rust comments is empty.
- R2 After M0: `run_corpus.py --check --tier shareware` green on all three OSes.
- R3 `quake-ctest` (1,566 tests) compiles against frozen C references in `rust/quake-ctest/csrc/` and stays green through M10.
- R4 After M5: `HostError` carries the message; the Host_Error body runs once, in Rust. C-visible `Host_Error`/`Host_EndGame` survive as thin variadic wrappers until the last C caller goes (M8c), then `Host_Guard`/`Host_Reraise`/both `jmp_buf`s go and `setjmp_inventory.py` reports zero.
- R5 After M6d: zero glue TUs; zero `#[no_mangle] pub static` in `quake-capi`.
- R6 After M9: `Quake/*.c` = codec bridges, lodepng/stb host TUs, harness TUs, `mem.c` successor if mimalloc kept; `quakedef.h` and `c_pch` gone.
- R7 Every milestone: cargo gates, inventories `--check`, harness `--check` green (bit-identical goldens).
- NG1 No behaviour change, no renderer modernization (ADR-015), no codec port (ADR-014 closed).
- NG2 Meson stays the build driver (Phase 9 deviation 1). NG3 No golden edits beyond `linux-x86_64` generation and the three KNOWN STALE Windows registered entries regenerated from the tag. NG4 `quake-cvar` not redesigned.

## Invariants

- I1 ADR-010: transliteration only; `as` casts, float compare order, `-ffp-contract=off`, xoroshiro `COM_Rand`, NaN-sign rules untouched.
- I2 Goldens, savegame/demo/config/condump/netreplay bytes unchanged (ADR-005 formatter).
- I3 ADR-009: no `longjmp` unwinds a Rust frame; no panic crosses into C.
- I4 ADR-006/008: arena stride/free-list and ambient qcvm resolution unchanged; `PR_SwitchQCVM(NULL)` order inside the Host_Error body unchanged.
- I5 ADR-013 until M9: cross-boundary buffers are `Mem_Alloc`/`Mem_Free`.
- I6 ADR-003/004: new crates permissive only (none planned beyond what is locked); pure crates keep `forbid(unsafe_code)`; `undocumented_unsafe_blocks = deny`.
- I7 ADR-019: `harness.c`/`harness_render.c` keep reading canonical state; every struct they hash stays `repr(C)`.

## Repository evidence (key facts)

- Switches: `meson_options.txt:11-22` (`use_rust` + 11 sub-options); resolution `meson.build:105-110, 332-396`; `-DUSE_RUST_*` at 405-457, 485-501, 527, 592, 617, 636, 1087; source swaps fs 281-290/485, formats/image 263-280/490-501, net 525-535, platform 543-580, progs 588-600, cvar 615-620, host 634-940, tasks 955-959, render 970-1085 + 135-140 + 207-211; PCH `c_pch` at 1296; staticlib `custom_target` 1249-1270 via `scripts/cargo_build.py`.
- C arms: `USE_RUST_NET` 40 (`net_main.c` 27), `HOST` 29, `FORMATS` 5, `RENDER` 3, `PROGS` 3, `PLATFORM` 3, bare 2, `IMAGE` 1, `CVAR` 1; none in headers. Rust code never tests `USE_RUST_*` → the defines can go.
- `#include "quakedef.h"`: 159 in `Quake/*.c`, 7 in `rust/quake-ctest/stubs/*_ref.c`. `quakedef.h:469-480` holds `Host_Error`/`HOST_GUARD_*`/`Host_Guard`/`Host_Reraise` decls.
- Error path: `Quake/host_glue.c` `Host_EndGame:204`, `Host_Error:237` (longjmp 283/298), `Host_Guard:321`, `Host_Reraise:358`, jmp_bufs 114-115, frame setjmp 929 (`#ifndef USE_RUST_PLATFORM`); `gl_screen_glue.c:410` `setjmp(screen_error)` in `SCR_DrawGUI`. Rust: `rust/quake-host/src/error.rs` (`HostError{AbortServer,ScreenError,Unknown}`), `rust/quake-platform/src/main_sdl.rs:140-161` (`host_frame`/`recover`), `rust/quake-capi/src/host.rs:1812`. Variadic-wrapper precedent: `Quake/sys_glue.c:143-151` (`Sys_Error` → `Host_Reraise(quake_rs_sys_error(text))`).
- Callers of `Host_Error`: 237 in 41 glue TUs; also in shared `common.c`, `pr_edict.c`, `pr_ext.c`, `pr_cmds.c`, `model_parse.c`, `gl_model.c`, `net_main.c`; 140 mentions in `quake-capi`; `quake-progs` `BuiltinSys` rule: ported builtins may not call a seam that can raise; `pr_cmds_glue.c:347` `RUST_PF` + `PRBI_Raise` pattern.
- Dual-view statics (~91): `gl_rmisc.rs` 16 (`vulkan_globals`, pinned by 152 `COMPILE_TIME_ASSERT`s in `gl_rmisc_glue.c`), `r_brush.rs` 12, `fs.rs` 11, `gl_fog.rs` 10, `gl_screen.rs` 7, `gl_draw.rs` 6, `r_part_render.rs` 5, others ≤3. Glue cvars are C storage registered from Rust (`quake-c-sys/src/host.rs:73`, `quake-capi/src/host.rs:659`). Glue asserts: `gl_rmisc_glue.c` 152, `gl_rmain_glue.c` 28, `gl_texmgr_glue.c` 17, `gl_heap_glue.c` 11.
- `CbContext`: `rust/quake-types/src/render.rs:439`, value type; layout consts 655-661 mirror `gl_rmain_glue.c:46-53`; `*mut CbContext` entry points in `gl_draw.rs`, `gl_fog.rs`, `gl_mesh.rs`, `gl_rmain.rs`, `gl_screen.rs`. `repr(C)`: quake-types 160, workspace 267.
- ctest: `rust/quake-ctest/build.rs:1-4,104-127` compiles `Quake/*.c` with `-include c_ref_prelude.h`; `csrc/` repoint pending; composing stubs pattern `stubs/pr_ext_ref.c:1-40`; test template `tests/pr_ext_strext_differential.rs:1-35`.
- Harness scripts run single-build when `--vkquake-b` is omitted (`save_diff.py:79`, `config_diff.py:135`, `condump_diff.py:127`, `record_diff.py:73`, `netreplay_diff.py:76`); `interop_matrix.py --combos` (line 697) filters cells.
- Shared TU status: see the per-TU table in D8 below (source: exploration of each file; `gl_model.c` has nothing ported; `pr_ext.c` has 122 of 273 slots flipped; `net_loop.c` is fully duplicated in Rust; `mem.c` pairs with `quake-capi/src/alloc.rs:50` `#[global_allocator]` over the same `mi_*`).
- ADR-013 M9 criterion (ADR text lines 49-64): keep mimalloc if any remnant hands `Mem_Alloc` ownership across the boundary or the system allocator regresses a harness-measured path beyond the Phase 8 noise floor; else drop `engine-alloc`, `quake-c-sys::mi`, `Quake/mimalloc/`.

## Architecture decisions

- **D1 Interleave glue removal with shared ports by stratum.** Order: M0 → M5 → M6a → M8a → M6b → M8b → M6c → M8c → M6d → M7 → M9 → M10. A glue TU is deleted only when no remaining C TU references a symbol it defines. New read-only helper `scripts/glue_deps.py` (symbols of each glue object from `llvm-nm`/`dumpbin` vs grep of remaining `Quake/*.c`) prints the deletable set per stratum so the mixed build links after every commit.
- **D2 `quakedef.h` retirement moves to M9** (after the remnant TU set is final, ~12 TUs) instead of rewriting 160 includes in TUs that M6/M8 delete. M0 only drops decls of deleted oracle symbols. This is a recipe (c) re-scope recorded in the D-A amendment.
- **D3 M0 removes the switches entirely.** Delete `use_rust` and the 11 sub-options; collapse every `if use_rust_X … else …` to the Rust arm; drop all `-DUSE_RUST*`; `rust_feature_list` becomes fixed (keyed only on `use_sdl3`, `trace`, debug, codecs); collapse `#if*def USE_RUST_*` arms in the 9 C files to the Rust body. Cargo features stay until M9 (clippy arms depend on them). Remove `-Duse_rust*` from every workflow/packaging script.
- **D4 Freeze ctest C references at M0** into `rust/quake-ctest/csrc/` (flat copies of every `C_SOURCES` entry, all `Quake/*.h`, and stb-style vendored includes `miniz.c/.h`, `jsmn.h`, `stb_image*.h`); `build.rs` compiles/includes only `csrc/` + `include/`; `check_ctest_symbols.sh` repointed. `csrc/` is edited only to freeze a shared TU's final text when M8 deletes it from `Quake/`.
- **D5 Host_Error in Rust; jump kept as a C tail until M8c.** `quake-host::error`: `enum HostError { Error(String), EndGame(String), ScreenError(String), Unknown(c_int) }` + `raise(err) -> c_int` running the body verbatim from `host_glue.c:204-300` once (recursion guard → `AtomicBool`, `Sys_DebugBreak`, `PR_SwitchQCVM(NULL)`, `SCR_EndLoadingPlaque`, `Con_Printf`, `screen_error` branch, `Host_ShutdownServer`, dedicated `Sys_Error`, `CL_Disconnect`, `demonum=-1`, `intermission=0`), storing the error in a `Mutex<Option<HostError>>` and returning the guard status; `HostError::take(status)` replaces `from_guard_status`. `quake-capi` exports `quake_rs_host_raise_error(msg)`/`_endgame(msg)`. C `Host_Error`/`Host_EndGame` become 8-line variadic wrappers (`q_vsnprintf` → `Host_Reraise(quake_rs_host_raise_error(s))`) following the `Sys_Error` precedent, in `host_glue.c` then `Quake/host_raise.c` (M6a) deleted at M8c. Rust raise sites return `Err(HostError)`. `SCR_DrawGUI` CSQC recovery moves into `gl_screen.rs` (status-returning guard; on `ScreenError` call `PR_ClearProgs(&cl.qcvm)`), deleting the `gl_screen_glue.c` setjmp. `main_sdl::recover` stays a no-op (body already printed; no double print). The 344/137 `Host_Guard`/`Host_Reraise` sites are not rewritten; they die with their glue TUs. Verify at M5 start that Rust 1.97.1 has no stable C-variadic definitions (design does not depend on it).
- **D6 Dual-view statics → private per-subsystem state (M6), `Host` struct (M7).** At M6 each `#[no_mangle] pub static` loses `no_mangle`/`pub` once `glue_deps.py` shows no C reader; glue-owned `cvar_t` objects move into the owning `quake-capi` module as `static mut cvar_t` (the `cl_main.rs` `cls`/`cl` pattern), registered via `quake-cvar` as today. At M7 statics group into `ClientState`, `ServerState`, `RenderState`, `NetState`, `FsState`, `ModelCache` owned by one `Host` in `quake-host`, created in `quake_main` and passed `&mut` to frame entry points only. A `static mut` may remain only where a C remnant reads it, tagged `// HOST-REMNANT:` and verified by `unsafe_inventory.py`.
- **D7 `repr(C)` rule.** Keep `repr(C)` iff (a) file/wire format read via bytemuck/transmute or written to disk/net, (b) passed to SDL/Vulkan/codec FFI, or (c) viewed by a surviving C remnant (harness hash inputs, `snd_codec`, `dma_t`, `Mem_*`). Everything else converts at M7, tagged `// REPR-C:` where kept. `COMPILE_TIME_ASSERT`s die with their glue TU; Rust layout consts mirroring them are deleted. `CbContext` params become `&mut CbContext`; `secondary_cb_contexts` becomes per-worker `Vec<Box<[CbContext]>>` scoped by frame-graph stage.
- **D8 M8 per-TU targets** ("diff" = ctest differential vs frozen `csrc/` copy; "goldens" = `--check` + self-diffs):
  - `cd_null.c` → delete; `CDAudio_*` no-op stubs in `quake-capi/src/cd.rs`.
  - `pr_trace.c` → `quake-progs::trace` writer (binary mode); gate `trace_diff.py` stability + byte diff of one trace vs `csrc/pr_trace.c`.
  - `q_thread_sdl.c` → `quake-platform::thread` (`QMutex/QCond/QSem/QThread` over `sdl2::sys`/`sdl3::sys`); exports kept while codec bridges/console use them; tsan job.
  - `steam_api.c` → `quake-fs::steam` + `quake-capi/src/steam.rs` (dlopen via `libc`/`windows-sys` as `pl` does); diff on string parse; manual Steam smoke.
  - `palette.c` → `quake-image::palette_octree` (table `const`; transliterated builders); diff on LUT bytes.
  - `common.c` → pure half to `quake-util` (`Vec_*`, `q_str*`, UTF-8, `COM_Parse*`, `COM_WordWrap`, `va`, `Info_*`, byteswap, xoroshiro `COM_Rand`) with per-function diffs; cvar/cmd/argv/`COM_Init`/`COM_Game_f`/thread-file accessors → `quake-capi::common`; goldens + `config_diff`.
  - `image.c`/`image_stb.c` → writer/decode paths to `quake-image` (lodepng/stb via FFI); TUs shrink to `Quake/lodepng_host.c`, `Quake/stb_host.c` (ADR-012 keep); diff on writer bytes.
  - `net_loop.c` → delete; `net_bsd.c`/`net_win.c` tables → `quake-capi::net_drivers`; `Loop_SearchForHosts/QueryAddresses` → `quake-net::loopback`; `net_main.c` residue (`NET_Listen_f`, `MaxPlayers_f`, `NET_Port_f`, slist printers, qsocket accessors, `NetMain_Glue_*`) → `quake-capi::net_main`; gate interop `--combos R/R`, netreplay/condump self-diffs.
  - `model_parse.c` residue (scratch globals, `nameInList`, `Mod_SetExtraFlags`) → `quake-formats::alias`; existing formats diffs (frozen).
  - `pr_edict.c` → `quake-progs::edict_print` (+ `PR_Init`), `ED_LoadFromFile` and `EDICT_NUM`/`NUM_FOR_EDICT`/`PROG_TO_EDICT` into the arena module (ADR-006 verbatim); diffs on printed strings; goldens.
  - `pr_cmds.c` → remaining `PF_*` to `quake-progs::builtins` via `BuiltinSys`; builtin table moves to `quake-progs`; `RUST_PF` retired at M6c; goldens + committed `builtin_diff.py` dump.
  - `pr_ext.c` → M8b-2..6; `gl_model.c` → M8c-1..4 (below). `snd_sdl.c`, `mem.c` → M9.
- **D9 Allocator procedure (M9).** `glue_deps.py --mem` lists `Mem_Alloc`/`Mem_Free` in remnant TUs and whether pointers cross to Rust. If none cross: build with `engine-alloc` off, compare `harness_render` timings + `--stability` wall-clock (3 runs) against the Phase 8 noise floor; within noise → drop `engine-alloc`, `quake-c-sys::mi`, `Quake/mimalloc/`; `Mem_*` become `quake-capi::mem` exports over `libc` (same CRT heap as Rust `System`). Otherwise keep and record the measurement in ADR-013.
- **D10 CI shapes after M0.** harness-linux: `build-rs` (release SDL2) + `build-rs-trace`; `check_capi_signatures`; `--check`; `--stability` (plain, `-parthash`, `--sndhash`); `trace_diff` run-twice; `save/condump/config/record/netreplay_diff` self-mode; `interop_matrix.py --combos R/R`; worker-count perturbation; render corpus stability. Dropped: all `--compare`, `builtin_diff` C-vs-mixed, `capture_diff`, physics matrix, formats corpus (frozen into ctest), `xtask_diff`. harness-mac/windows: `build-rs` release+debug, `--check`, `--stability`, self-diffs, Windows interop R/R. rust.yml unchanged + `check_ctest_symbols.sh` against `csrc/`. Packaging jobs drop `-Duse_rust=enabled`.

## Change boundary

Expected to change (patterns, per milestone):
- M0: `meson.build`, `meson_options.txt`, `scripts/cargo_build.py`; delete 81 oracle TUs + `miniz.*`, `jsmn.h`, `cd_sdl.c`, `pl_osx.m`, `Shaders/bintoc.c`, `Misc/vq_pak/mkpak.c` (verify each is oracle-only before deletion), `Quake/Makefile*`, two workflows; arm collapse in `Quake/{net_main,net_bsd,net_win,net_loop,model_parse,pr_ext,pr_cmds,pr_edict,host_glue,*_glue}.c`; `quakedef.h` dead decls; new `rust/quake-ctest/csrc/**`, `build.rs`, `check_ctest_symbols.sh`; four workflows; AppImage/installer scripts; `Misc/harness/goldens/linux-x86_64/**` + 3 Windows regenerations; inventory scripts + docs; `Misc/harness/README.md`; ROADMAP/PLAN/ADR-017/018/019; new `scripts/glue_deps.py`.
- M5: `rust/quake-host/src/error.rs`, `quake-capi/src/{host,gl_screen,progs_*}.rs`, `Quake/{host_glue,gl_screen_glue,pr_cmds_glue}.c`, `quake-platform/src/main_sdl.rs`, ADR-009.
- M6a–d: delete `Quake/*_glue.c` by stratum; `quake-capi/src/*.rs` statics; `quake-c-sys` externs + `bindings_wrapper.h`; `cbindgen.toml`; `Quake/host_raise.c` (M6a → deleted M8c).
- M8: delete 17 shared TUs (2 become host TUs); `rust/quake-{util,image,formats,progs,net,fs,platform,capi}/src/**`; `quake-ctest/tests/*_differential.rs`; `csrc/` freezes.
- M7: `rust/quake-types/src/**`, `quake-render/src/**`, `quake-capi/src/gl_*.rs`, new `quake-host/src/host.rs`.
- M9: `quake-platform/src/snd_sdl2.rs`, `quake-capi/src/{snd_sdl,alloc,mem}.rs`, `Quake/mem.c`+`mimalloc/`, `quakedef.h` (deleted), `meson.build` (`c_pch`, IPv6 flip per the Phase 9 plan amendment log), `quake-c-sys` regenerated, `quake-capi/Cargo.toml` features collapsed, `quake-platform/build.rs` (`qs_bmp.h` embed).
- M10: `rust/Cargo.toml` lints, inventories, ADR-002 appendix, ROADMAP Phase 10 block.

Must not change without amendment: existing goldens (beyond NG3), `run_corpus.py` hashing, harness TU hashed layouts (I7), `quake-snd` codec trait/bridges, lodepng/stb keep, savegame/demo/protocol bytes, `quake-progs` arithmetic, `deny.toml` allow-list, `rust-toolchain.toml`.

## Acceptance matrix

| ID | Criterion | Gate |
|---|---|---|
| AC1 | `c-reference/final` tag exists with rebuild recipe | `git tag -n`, ADR-019 |
| AC2 | No `use_rust*`/`USE_RUST*` outside docs/comments; Makefiles + 2 workflows gone | `git grep`, `ls` |
| AC3 | Three harness jobs green with `--check` on the mixed build only | CI |
| AC4 | ctest green from `csrc/`; `check_ctest_symbols.sh` green | `cargo test -p quake-ctest --locked` |
| AC5 | Inventories `--check` green each milestone; setjmp sites 19 → M5 15 → M8c 0 | scripts |
| AC6 | `HostError` carries message; condump/config/save self-diffs + goldens unchanged after M5 | scripts |
| AC7 | Zero glue TUs; zero `#[no_mangle] pub static` in capi | inventory, grep |
| AC8 | `repr(C)` only under rule (a)/(b)/(c), tagged; `CbContext` by `&mut` | grep report in M7 evidence |
| AC9 | `Quake/*.c` = remnant set; `quakedef.h` + `c_pch` gone | `ls`, `meson.build` |
| AC10 | ADR-013 decision recorded with measurement; SDL2 audio Rust on Linux harness | ADR, CI |
| AC11 | Harness `--check` green on all three OSes at every milestone | CI |
| AC12 | Cargo gates green (fmt, 4 clippy arms, test debug+release, deny ×2) | `rust/` commands |

## Milestones (each via `/feature-implement <plan> <Mx>`; stop after each)

Common verification, Windows dev host (from `rust/`, then repo root; MSVC env per memory notes):
```
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy -p quake-capi --features progs,render --all-targets --locked -- -D warnings
cargo clippy -p quake-capi --features platform,sdl3 --all-targets --locked -- -D warnings
cargo clippy -p quake-capi --features platform,sdl2 --all-targets --locked -- -D warnings
cargo test --workspace --locked ; cargo test --workspace --locked --release
cargo deny check ; cargo deny check licenses --manifest-path fuzz/Cargo.toml --config deny.toml
python scripts/c_remnant_inventory.py --check ; python scripts/unsafe_inventory.py --check ; python scripts/setjmp_inventory.py --check
meson setup build-rs -Duse_sdl3=enabled --buildtype=release ; ninja -C build-rs
QUAKE_GAME_DATA=<id1 dir> python scripts/harness/run_corpus.py --vkquake build-rs/vkqr-engine.exe --check --tier shareware
python scripts/harness/save_diff.py / condump_diff.py / config_diff.py --vkquake build-rs/vkqr-engine.exe
```
CI-only: Linux/macOS harness, lavapipe render corpus, sanitizers, bindgen diff, SDL2 audio; `--target x86_64-unknown-linux-gnu` clippy locally as the `cfg(unix)` proxy.

- **M0 Deletion PR.** Rebase worktree onto master. Commits in order, each linking: (1) amendments D-A..D-D + `generate-goldens-linux` dispatch job; annotated tag `c-reference/final` on master head (push only with the PR); dispatch job, commit `linux-x86_64` goldens; locally regenerate the 3 KNOWN STALE Windows entries from a `c-reference/final` worktree. (2) Meson D3. (3) C deletions + arm collapse + `quakedef.h` dead decls (the `host_glue.c:929` frame setjmp goes with its arm). (4) D4 ctest freeze. (5) D10 CI reshape + README wording. (6) Regenerate inventories (prune oracle/unbuilt/tool/miniz/jsmn rules; add `glue_deps.py`); ROADMAP Phase 9 `[x]`. Exit: AC1–AC5, AC11, AC12; `git grep -l setjmp Quake/` = `host_glue.c gl_screen_glue.c`.
- **M5 Host_Error in Rust (D5).** Exit: AC6; setjmp 19→15; ADR-009 amendment. Manual: `changelevel nonexistent` condump identical apart from stack frames; CSQC HUD error recovers.
- **M6a Glue stratum A** (glue with no C reader per `glue_deps.py`; expected: `chase, cl_demo, cl_input, cl_parse, cl_tent, console, gl_draw, gl_fog, gl_heap, gl_refrag, gl_rlight, gl_sky, gl_vidsdl, gl_warp, host_cmd, in_sdl, keys, menu, r_alias, r_part*, r_sprite, sbar, snd, sv_*, sys, tasks, view, world`); cvars/statics to Rust; `host_glue.c` → `host_raise.c`; c-sys/cbindgen pruned.
- **M8a Leaf shared TUs** (D8: `cd_null, pr_trace, q_thread_sdl, steam_api, palette, common, image*, net_loop, net_bsd/win, net_main, model_parse` residue) with ctest diffs; freeze finals into `csrc/`.
- **M6b Glue stratum B** (`net_*_glue`, `cvar_cmd_glue`, `net_msg_glue`, `model_parse_glue`, `cl_main_glue` remainder).
- **M8b Progs, six sub-milestones:** M8b-1 `pr_edict.c` + `pr_cmds.c` residue; M8b-2 `pr_ext.c` 51-130 + `PF_sprintf` flip + `PR_Init/Shutdown/AutoCvarChanged/InitExtensions`; M8b-3 model/frame/surface 2083-2556; M8b-4 client/cvar/registercvar 2557-2752 + reflection 3885-4182; M8b-5 infokey/multicast (seam → `BuiltinSys` method returning `Result`) + uri/crc/digest + custom stats; M8b-6 CSQC 2D drawing, playerkey/serverkey/cl_read, touchtriggers/getrenderentity, `extensionbuiltins[]`/`qcextensions[]`, `PF_checkextension/builtinsupported/checkbuiltin`, `PR_EnableExtensions`; delete `pr_ext.c`. Gate: `pr_ext_ref.c`-pattern diffs per section, goldens, committed `builtin_diff` dump, trace stability.
- **M6c Glue stratum C** (`pr_*_glue.c` incl. `RUST_PF`/`PRBI_Raise`, `pr_exec_glue.c`).
- **M8c `gl_model.c`, four sub-milestones:** M8c-1 cache/registry + PVS → `quake-capi::model_cache`; M8c-2 brush model/faces/textures + texture tasks → `quake-render::model_tex` + `quake-tasks`; M8c-3 alias skins; M8c-4 MD3/MD5 skin machinery + `Mod_LoadModel` orchestration; delete `gl_model.c`, `host_raise.c`, `Host_Guard`/`Host_Reraise`, jmp_bufs, `quakedef.h` decls. Gate: diffs on `FloodFillSkin`, `DecompressVis`, `PointInLeaf`; render corpus stability; ASan map load. Exit: setjmp = 0 (script kept as zero-gate); ADR-009 closed.
- **M6d Glue stratum D** (`gl_texmgr_glue`, `gl_rmisc_glue`, `gl_rmain_glue`, `gl_screen_glue`, `pr_edict_load_glue` remainder) → AC7; ADR-007 amendment (all dual-view rows closed).
- **M7 `repr(C)` → idiomatic, `Host` struct, `CbContext` (D6/D7).** Verification adds render corpus stability (CI), Windows manual render smoke (timedemo, `r_showbboxes`, particles), `-parthash` stability. Exit: AC8.
- **M9 Allocator (D9), SDL2 audio (D-D), IPv6-on-MSVC flip, `quakedef.h`/`c_pch` retirement, c-sys regen from remnant headers, capi features collapsed to `{engine-debug, trace, sdl2|sdl3, codec-*}`, `qs_bmp.h` embed.** Verification adds Linux `--stability --sndhash` (CI) and manual SDL2 audio smoke. Exit: AC9, AC10.
- **M10 Re-cut and exit.** Regenerate inventories; ADR-002 appendix re-cut; retire FFI-style pedantic allows (`borrow_as_ptr`, `ptr_as_ptr`, `ptr_cast_constness`, `ref_as_ptr`, `cast_ptr_alignment`, `unnecessary_wraps`); ROADMAP Phase 10 `[x]`; `/integration-review` from fresh context.

## Risks and open items

| ID | Item | Handling |
|---|---|---|
| RA1 | No `--compare` oracle after M0; port bugs caught only by goldens/self-diffs/ctest | frozen `csrc/` diffs per ported function; goldens on 3 OSes; `--check` every milestone |
| RA2 | Host_Error body drift changes condump bytes | line-by-line transliteration; condump self-diff + manual check |
| RA3 | `sdl2` crate in maintenance mode | bindings only, MIT, locked |
| RA4 | C-variadic definitions not stable in 1.97.1 | C wrapper design works either way |
| RA5 | `glue_deps.py` misses a macro/inline reader | link failure is immediate; strata small |
| RA6 | Runner-image drift for Linux goldens | MANIFEST pins image; regeneration recipe from the tag |
| RA7 | Allocator perf unmeasurable within noise | keep mimalloc unless clearly within floor |
| RA8 | `Host` struct threading balloons | bounded to frame entry points (D6) |
| RA9 | User declined the four decision questions | defaults D-A..D-D used; override at plan review |

## First concrete action

M0 commit (1): write amendments D-A..D-D and the `generate-goldens-linux` dispatch job, create the annotated tag `c-reference/final` on fb372b7d locally, and regenerate the three KNOWN STALE Windows goldens from a `c-reference/final` worktree (`meson setup build-c -Duse_rust=disabled -Duse_sdl3=enabled --buildtype=release`).

## Verification evidence / handoff

### M0 (2026-09-19, commits eb097512 .. 3a24a44e on `feature/rust-conversion-phase-11-0e8f8b`)

Commits: (1) eb097512 amendments D-A..D-D, tag `c-reference/final` = fb372b7d (local, pushed with the PR), `generate-goldens-linux` dispatch job, three Windows registered-tier goldens regenerated from the tag; (2) ff07aff4 Meson switches removed (D3); (3) cd179810 81 oracle TUs, Makefiles and the MinGW/MSYS2 workflows deleted, `USE_RUST_*` arms collapsed; (4) 4ea4c3f8 ctest references frozen under `rust/quake-ctest/csrc/` (D4); (5) 29fdc743 harness CI reshaped (D10), `xtask_diff.py` retired; (6) 3a24a44e inventories regenerated, `scripts/glue_deps.py` added (D1), ROADMAP Phase 9 `[x]`.

| Check | Kind | Result |
| --- | --- | --- |
| `cargo fmt --all --check` | broad | clean |
| `cargo clippy` workspace, `progs,render`, `platform,sdl3`, `platform,sdl2` arms (`--locked -- -D warnings`) | broad | clean |
| `cargo test --workspace --locked` debug and `--release` | broad | 1 576 passed, 0 failed each |
| `cargo deny check`; `cargo deny --manifest-path fuzz/Cargo.toml check licenses` | broad | clean (pre-existing `Unicode-3.0` unmatched-allowance warning) |
| `scripts/harness/check_ctest_symbols.sh` (vcvars64) | targeted | OK, 43 frozen sources |
| `scripts/harness/check_capi_signatures.sh build-rs/quake_rs.h` (clang-cl) | targeted | OK |
| `c_remnant_inventory.py --check`, `--ninja build-rs/build.ninja` | targeted | OK; 78 TUs compiled, 0 problems (net_bsd.c, snd_mp3.c, snd_sdl.c not on this platform/config) |
| `setjmp_inventory.py --check` | targeted | OK, 19 sites (host_glue.c 16, gl_screen_glue.c 3), 39 guarded TUs |
| `unsafe_inventory.py --check` | targeted | OK |
| `glue_deps.py build-rs --mem` | targeted | runs; 9 glue TUs already have no C reader (gl_draw, gl_heap, gl_refrag, pr_cmds_cl, pr_cmds_sv, pr_cmds_sv_fx, pr_cmds_sv_msg, pr_edict_dispatch, tasks) |
| `ninja -C build-rs` (clang-cl, SDL3, release) after commit 6 | targeted | links |
| `run_corpus.py --check --tier shareware`, plain and `--sndhash`, full `id1` data | broad | 8 ran, 0 failed each |
| `save_diff.py`, `condump_diff.py`, `config_diff.py` self mode on build-rs | targeted | identical |
| `git grep -l setjmp Quake/` | targeted | executable sites only in `host_glue.c`, `gl_screen_glue.c`; the other TUs match ADR-009 comments |
| `git grep -nE '^\s*#\s*(if|ifdef|ifndef|elif).*USE_RUST' Quake/ rust/` | targeted | 19, all inside the frozen `rust/quake-ctest/csrc/` text (the tag's tree, by design) |
| Linux/macOS harness jobs, lavapipe render corpus, sanitizers, bindgen diff, the `generate-goldens-linux` dispatch | not run | CI only; no PR opened yet |
| YAML validity of the four edited workflows | not run | no PyYAML on the host; structural checks only (no tabs, consistent CRLF, trailing newline) |

Acceptance: AC1 met (tag local, ADR-019 recipe). AC2 met for executable references (no `get_option('use_rust`, no `-Duse_rust` outside the tag-building dispatch job, no `#if USE_RUST` outside `csrc/`); ~320 comment mentions of the retired switches remain in glue TU headers, `quake-c-sys` doc comments, `Quake/net_dgrm_int.h:29`, `Quake/pr_cmds.c:77`, the goldens MANIFESTs (provenance) -- R1's "empty" reading is not met literally and is left for the TUs' deletion at M6/M8. AC3, AC11 not run (CI). AC4 met locally. AC5 met (19 sites, matches the plan's M0 count). AC12 met locally.

Risks carried: RA1 (no `--compare` oracle); Linux `--check` is vacuous until the dispatch job's goldens are committed; the MANIFEST/README wording for the dispatch job is unverified against a real run; `glue_deps.py` is textual (RA5).

Handoff: open the PR (push the branch and the `c-reference/final` tag together), run the `generate-goldens-linux` dispatch and commit its output, then start M5 (`/feature-implement docs/ai/plans/rust-conversion-phase-10-post-deletion.md M5`). The single next action after the PR is green is M5.
