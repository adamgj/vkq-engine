# Rust Migration Phase 4 — Sound

## Context

Phases 0–3 of the Rust migration are complete (deletions deferred). Phase 4 (ROADMAP.md:102–115, crate `quake-snd`, currently a Phase-0 stub) ports the sound subsystem: sfx cache/resample (`snd_mem.c`), software mixer (`snd_mix.c`), channels/spatialization (`snd_dma.c`), background music (`bgmusic.c`), codec framework + pure-Rust WAV/UMX/mp3tag parsers, and SDL audio backends. Exit criteria: PCM-hash parity on a demo-soundtrack corpus, per-codec decode comparisons, full harness parity unchanged (ADR-019).

Governing ADRs: ADR-014 (codec libraries stay C behind a Rust trait; registration order is lookup-preference order; mixer PCM tests use fixed decoder inputs), ADR-017 (SDL2+SDL3 both kept; sdl2/sdl3 cargo features), ADR-019 (verification), ADR-010 (per-platform bit parity; libm via quake-c-sys), ADR-011 (FFI tooling), ADR-013 (engine mimalloc across boundary), ADR-009 (no unwind into C), ADR-007 (needs a new sound-globals row), ADR-003 (permissive licenses only).

**User decisions (recorded):**
1. SDL backends: **adopt the `sdl2`/`sdl3` binding crates now** in `quake-platform`, feature-gated per ADR-017. Verify licenses (`sdl2` MIT, `sdl3` Zlib) and full dep trees with `cargo deny check licenses` before adding; if a transitive dep violates ADR-003, stop and report.
2. PCM-hash gate: **approved** — add `-sndhash` harness flag + deterministic DMA clock + guarded mixer hook to both builds (C first).
3. Codec decode-comparison tests: **env-gated** (`QUAKE_CTEST_CODECS=1`) Linux CI leg with pkg-config system libs.

**Deletions:** deferred, same as Phases 1–3 (PLAN §3 MinGW decision). C sound files stay compiled as the differential oracle behind `-Duse_rust_snd=disabled`.

## Verified ground truth (landmines)

- `S_LoadSound` allocates via **`Mem_Alloc`** (snd_mem.c:149) — ADR-013 satisfied via `quake_c_sys::Mem_Alloc`; no cache/hunk system involved.
- Mixer runs on the **main thread** (`S_Update_`, `S_ExtraUpdate`); SDL callback only memcpys `shm->buffer` under `SNDDMA_LockBuffer`. `snd_mutex` is a **recursive** SDL mutex (q_thread_sdl.c) and recursion is load-bearing (`S_Update_` → `S_PaintChannels` → `S_LoadSound` re-locks). Rust must use `QMutex_*` via c-sys, never a Rust Mutex.
- `S_FlushOldestSounds` memmove leaves dangling `channel_t::sfx` pointers — existing behavior, reproduce exactly.
- `GetWavinfo` has OOB-read potential (cue/LIST path, snd_mem.c:273–321) — expect ASan trips in differential fuzzing; land behavior-neutral C bounds clamps first (Phase 3 playbook), verified by `--compare`.
- Mixer arithmetic is exact: C truncating division (`/256`, not `>>8`), scaletable float→int truncation, `S_ApplyFilter` 4-lane partial float sums (preserve lane order), `ResampleSfx` int64 samplefrac. `-ffp-contract=off` already pinned; transcendentals (`sin`/`cos`/`exp`/`log`/`sqrt`) via `quake_c_sys::libm` with `// COMPAT: ADR-010`.
- `S_StartSound` draws `COM_Rand()` — RNG draw order feeds the demo state-hash chain; parity required.
- `-headless` currently skips `S_Init` (host.c:1273) — the `-sndhash` instrument adds a deterministic harness DMA clock so headless runs can init sound.
- Compat surface pinned by external C readers: `menu.c` reads `sfxvolume`/`bgmvolume` cvar storage; `cl_demo.c` iterates `snd_channels[]`/`total_channels`; these stay C-visible symbols (in new `snd_glue.c`).
- Header is `q_sound.h`. `cd_null.c` stays compiled; `cd_sdl.c` already dead.
- UMX rewrites `stream->fh.start/length` then forwards to WAV/MP3(/absent MOD) — codec framework must support forwarding.
- `fshandle_t`/`FS_fread`/`FS_fseek` used by umx/mp3tag are already Rust (Phase 2 `fs_stdio.rs`).

## Milestones (implement one per session, tree green after each)

### M1 — Scaffolding: types, bindings, ctest oracle, `-sndhash` instrument
- `rust/quake-types/src/sound.rs`: `#[repr(C)]` mirrors — `portable_samplepair_t`, `sfxcache_t` (flexible array → header + data-offset const), `sfx_t`, `dma_t`, `channel_t`, `wavinfo_t`, `snd_info_t`, `snd_stream_t`, `snd_codec_t` (7-fn vtable). Const layout asserts + extend `rust/quake-ctest/stubs/abi_probe.c` with `snd_abi` probe + `tests/snd_abi.rs`.
- `quake-c-sys` allowlist additions + `scripts/gen_c_bindings.sh` regen: `QMutex_*`, `COM_Rand`, `Cmd_*`, `Cvar_*`, `SNDDMA_*`, `Mod_PointInLeaf`, `CDAudio_Play`, data globals (`shm`, `snd_channels`, `total_channels`, `paintedtime`, `soundtime`, `s_rawend`, `s_rawsamples`, listener vectors, sound cvars) as an audited globals module (SAFETY: main thread under snd_mutex).
- `rust/quake-ctest/build.rs`: add the 8 portable snd C files as `c_ref_*`; extend `stubs/stubs.c`.
- `-sndhash <file>` in `harness.c/h` + guarded `Harness_SndHash` hook in `snd_mix.c` + deterministic harness DMA clock (fixed 44100/16/stereo, samplepos advanced per frame). C build first; goldens generated only from C (ADR-019).
- meson: `use_rust_snd` option (`use_rust_fs` pattern) + cargo feature `snd` + empty-for-now `phase4_c_srcs`; plumb `codec-mp3/flac/vorbis/opus` cargo features from the existing `USE_CODEC_*` meson logic.
- Gates: all harness configs build; `run_corpus.py --check` unchanged; `-sndhash` run twice on build-c → identical output; bindgen regen-diff, `check_headers.sh` clean.

### M2 — Pure WAV-info parser + resampler (no engine flip)
- `rust/quake-snd/src/wav.rs` (`get_wavinfo`, exact chunk walk incl. cue/LIST quirks, `BadLoopLength` variant) and `src/resample.rs` (`resample_sfx`, fast+general paths, `loadas8bit`). `#![forbid(unsafe_code)]`.
- ctest `wavinfo_differential` + `resample_differential` over shareware WAVs (speed 11025/44100 × width 1/2 × loadas8bit 0/1) + synthetic edge fixtures. Fuzz target `wavinfo_diff`; C bounds clamps if ASan trips (separate behavior-neutral commit, `--compare`-verified).

### M3 — `snd_mem.c` engine flip (first `use_rust_snd` slice)
- `quake-capi/src/snd_mem.rs`: export `S_LoadSound`, `GetWavinfo` with exact C signatures; lock C-owned `snd_mutex` via c-sys; `Mem_Alloc(len + sizeof(sfxcache_t))`; same `Con_Printf` messages; `Sys_Error` for bad loop length (diverging, ADR-009).
- meson: `use_rust_snd` excludes `Quake/snd_mem.c`. Extend `check_capi_signatures.sh` TU with `q_sound.h`.
- Gates: `run_corpus.py --compare` build-rs-snd vs build-c; `-sndhash --compare`; ctest/fuzz unchanged.

### M4 — Pure mixer core + fixed-input PCM ctest gate
- `quake-snd/src/mix.rs`: `MixerState` (paintbuffer[2048], scaletable[32][256], filters, underwater accum) + exact ports: `S_PaintChannels`, `Snd_WriteLinearBlastStereo16`, `S_TransferStereo16`/`S_TransferPaintBuffer` (16/U8/S8), `S_MakeBlackmanWindowKernel` (libm), `S_UpdateFilter`/`S_ApplyFilter` (lane-exact), `S_LowpassFilter`, `S_UnderwaterFilter`, `SND_PaintChannelFrom8/16`, `SND_InitScaletable`.
- ctest `snd_mix_differential`: c_ref vs Rust over scripted channel schedules on shareware sfx (loaded by `c_ref_S_LoadSound` — identical fixed inputs both sides, ADR-014) + injected raw samples; matrix over shm formats × `snd_filterquality` 1–5 × underwater. Byte-compare buffers; committed golden manifest from c_ref.

### M5 — Mixer engine flip
- `quake-capi/src/snd_mix.rs`: export `S_PaintChannels`, `SND_InitScaletable`, `S_SetUnderwaterIntensity`; `MixerState` as Rust static (documented ADR-007 sound row: storage C, mixer state Rust, main-thread + snd_mutex). `-sndhash` hook reimplemented identically in Rust. meson gates `snd_mix.c`.
- Gates: **`-sndhash` corpus `--compare` build-rs-snd vs build-c = the engine-level mixer PCM parity proof**; `--check` demo hashes unchanged; `save_diff.py`.

### M6a/M6b — `snd_dma.c` (two sessions)
- **M6a (pure + ctest):** `quake-snd/src/dma.rs`: `SND_PickChannel`, `SND_Spatialize` (quake-math, COMPAT), `S_RawSamples` scheduling, ambient ramps, `GetSoundtime` wrap; ctest `snd_dma_differential` sweeps.
- **M6b (flip):** new **`Quake/snd_glue.c`** (compiled only under `use_rust_snd`; `model_parse_glue.c` precedent) owning compat storage: 15 cvars, `snd_mutex`, `shm`/`sn`, `snd_channels[]`/`total_channels`, timing globals, listener vectors, command/cvar-callback trampolines. `quake-capi/src/snd_dma.rs` exports the full 22-function `S_*`/`SND_*` API; `known_sfx` storage moves to Rust (flush/memmove semantics byte-preserved); `COM_Rand` draw-order parity. meson gates `snd_dma.c`.
- Gates: full `--check` + `--compare` all configs (RNG draws now Rust-side — demo state hash is a real gate), `-sndhash --compare`, `save_diff.py`, capi signature TU.

### M7 — Codec framework + UMX + mp3tag + WAV codec
- `quake-snd/src/codec/`: `Codec` trait mirroring `snd_codec_t`; registry with exact registration order (UMX, WAV, FLAC, MP3, VORBIS, OPUS — snd_codec.c:63–80) + stream forwarding. Native impls: `wav_stream.rs`, `umx.rs` (upkg parser incl. FCompactIndex + version quirks), `mp3tag.rs` (ID3v1/v2, APE, Lyrics3, MusicMatch).
- C decoders (`snd_mp3.c`/`snd_mpg123.c`/`snd_vorbis.c`/`snd_opus.c`/`snd_flac.c`) stay compiled, wrapped by a `CVtableCodec` adapter over their `snd_codec_t` statics, gated by `codec-*` features. capi exports for C wrappers: `S_CodecUtilOpen/Close`, `S_CodecForwardStream`, `mp3_skiptags`, plus `S_Codec*` public API for bgmusic (still C until M8).
- meson gates `snd_codec.c`, `snd_wave.c`, `snd_umx.c`, `snd_mp3tag.c` (move `snd_mp3tag.c` out of the mp3 conditional).
- Tests: ctest `umx_differential`/`mp3tag_differential` over **self-generated committed fixtures** (generator script under `scripts/`, à la `gen_jpeg_fixtures.py`); env-gated `codec_stream_differential` (`QUAKE_CTEST_CODECS=1`, pkg-config libs, Linux CI job) comparing C vs Rust framework streaming byte-for-byte over the same C decoders — the per-codec exit gate. Fuzz: `umx_diff`, `mp3tag_diff`.

### M8 — bgmusic port
- `quake-snd/src/bgmusic.rs` + capi exports `BGM_*` (8 fns); handler table exact order incl. dead MOD entries; `music*` command trampolines + `bgm_extmusic` cvar in `snd_glue.c`; 16 KiB `BGM_UpdateStream` staging loop byte-matched. meson gates `bgmusic.c`.
- Gates: `-sndhash` corpus extended with a music `-harnesscmds` script over a committed synthetic WAV (deterministic goldens); lossy codecs exercised in `--compare`/env-gated legs only.

### M9 — SDL audio backends → quake-platform (sdl2/sdl3 crates)
- Add `sdl2` and `sdl3` binding crates as mutually exclusive cargo features in `quake-platform` (ADR-017). **First:** `cargo deny check licenses` on both full dep trees (ADR-003; MIT/Zlib expected — stop and report if any transitive violation). Record the review note in the PR description.
- `snd_sdl2.rs`/`snd_sdl3.rs` porting `snd_sdl.c`/`snd_sdl3.c`; `paint_audio` callback panic-free/allocation-free, volatile `shm` access under SDL lock. capi exports the six `SNDDMA_*`; meson gates `snd_sdl.c`/`snd_sdl3.c` and wires the sdl2/sdl3 cargo feature from the existing SDL major-version selection.
- Gates: CI builds both SDL legs (Linux/macOS SDL2+SDL3, Windows SDL3); harness unaffected (deterministic clock); manual audible smoke both backends.

### M10 — Phase exit
- Full ADR-019 suite re-run on all configs incl. `build-rs-csnd` (all-Rust-except-sound oracle config); `-sndhash` goldens committed (darwin-arm64; linux/windows ride the existing registered-tier deferral); fuzz soak; `/integration-review` from fresh context (CLAUDE.md requirement for high-risk migration features).
- Docs: ROADMAP Phase-4 checkboxes + deletion-deferral note, ADR-007 sound row, ADR-017 note (adopted in Phase 4 for audio), CI workflow updates (`rust.yml` matrix, fuzz list, codec-libs leg, SDL2/SDL3 legs).

## Meson design

Single `use_rust_snd` feature (auto-follows `use_rust`; `enabled` without `use_rust` errors — verbatim `use_rust_fs` pattern) → cargo feature `snd`. Gated file list grows per milestone; final:

```
phase4_c_srcs = [snd_mem, snd_mix, snd_dma, snd_codec, snd_wave, snd_umx, snd_mp3tag, bgmusic, snd_sdl, snd_sdl3]
phase4_glue_c_srcs = [snd_glue.c]  # M6b+
```

Codec wrapper conditionals unchanged except `snd_mp3tag.c` relocation; each `USE_CODEC_*` decision appends the matching `codec-*` cargo feature. Harness gains config `build-rs-csnd`.

## Verification summary

Per-milestone: targeted ctest differentials + affected harness gates. Milestone boundaries: `run_corpus.py --check` (state-hash goldens) + `--compare`, `save_diff.py`, `-sndhash` stability/compare, `check_headers.sh`, `check_capi_signatures.sh`, bindgen regen-diff, `cargo deny`/`cargo audit` (M9), clippy/fmt per rust.yml. Phase exit: full suite on all configs + fuzz soak + integration review. `QUAKE_GAME_DATA` points at local full id1 for registered-tier local runs (see memory notes); CI uses shareware + synthetic fixtures.

## Key reusable code

- `quake_c_sys::Mem_Alloc`/`mi_*` (ADR-013), `quake_c_sys::libm` (ADR-010), Phase-2 `fs_stdio.rs`/`quake-fs` for stream IO, `quake-math` for spatialization vec ops, `quake-ctest` `c_ref_*` build pattern + `stubs/`, `scripts/gen_jpeg_fixtures.py` as fixture-generator precedent, `Quake/model_parse_glue.c` as glue-file precedent, `harness.c/h` Phase-0 instrument pattern.

## Process notes

- One milestone per session via `/feature-implement`; this plan (once approved) is committed under `docs/ai/plans/rust-conversion-phase-4.md` per AGENTS.md.
- No C deletions; no roadmap-order violations; `tasks.c` untouched (ADR-016).
- After each milestone: commit with phase/ADR references, run format.sh for C changes.

## Amendment log

- **2026-08-25 (M1/M2 merge):** M1 and M2 were implemented and gated together; the ctest c_ref wiring originally listed under M1 landed with the milestone that first tested it. The `-sndhash` chain covers both the painted paintbuffer region and the full DMA buffer per paint block; goldens live in their own `<name>.snd` / `<name>.snd-demo.hash` namespace because the sound engine's `COM_Rand` draws shift the demo state hash.
- **2026-08-25 (M2/M3):** three behavior-neutral bounds clamps landed in the C oracle beyond the planned ones — an invalid-sample-rate reject (`rate <= 0` made `(int)(samples/stepscale)` UB and platform-divergent) and a *precise* resampler read-bounds clamp (the fuzz differential proved float rounding can push the last source index past the data chunk). The M3 flip found `com_filesize` is THREAD_LOCAL: shims must use `COM_ThreadFileSize()`; the raw binding was removed.
- **2026-08-25 (M6a):** the c_ref oracle build now renames the shared sound globals/cvars to `c_ref_*` (snd_dma.c defines them), keeping the c_ref sound subsystem self-consistent; the Rust-side stand-ins live in `quake-ctest/src/snd_stubs.rs`. The spatialize sweep cannot distinguish the C's double-promotion from single-rounded f32 on random inputs (sub-ULP); the port keeps the C-exact double form by construction.
- **2026-08-25 (M7):** the codec framework is implemented directly over the `snd_codec_t` vtable rather than an additional Rust trait layer — the vtable *is* the ADR-014 mirror, the C decoder wrappers plug in unchanged, and registration order is trivially preserved. UMX ports behind `codec-umx` (mirroring `USE_CODEC_UMX`, enabled by no Meson config). The per-codec decode comparison runs engine-level (same C decoder libs both builds) via the music harness corpus instead of the planned env-gated ctest lib leg; lossy-format fixtures cannot be committed (no encoders in CI), so CI covers WAV and local registered-tier runs covered vorbis + flac.
- **2026-08-25 (M9, scope deviation):** the SDL2 audio backend was NOT ported. SDL2 dev libraries are absent from the dev machine and every CI leg, so the `sdl2`-crate backend could not be built anywhere; shipping unverifiable code was rejected. `snd_sdl.c` stays C (meson compiles it under use_rust_snd when SDL2 is selected); the port follows with a use_rust+SDL2 CI leg (ADR-017 amendment).
- **2026-08-25 (M10):** committed-fixture staging added to the harness (`run_demo.py --fixture-dir`, corpus `fixture_dir` key) with two synthetic WAVs under `Misc/harness/fixtures/sndfix/`; new corpus entry `music-wav`; darwin-arm64 `-sndhash` goldens committed for all 11 entries.
- **2026-08-25 (post-review fixes):** the fresh-context integration review
  (report: phase4-review.md in the session scratchpad; verdict "ready with
  stated residual risk, conditional on finding 1") produced: (1) FIXED — the
  snd_mix shim redeclared `SND_Glue_PauseLoops` as returning `c_int` where the
  C returns `qboolean`/_Bool (upper register bits unspecified per ABI); it now
  uses the correctly-typed bindgen declaration. (2) FIXED — the `channels()`
  helper handed out overlapping `&'static mut` borrows of `snd_channels`
  (Stacked-Borrows UB); snd_dma.rs now works from a raw base pointer with
  tightly-scoped exclusive references, and `SfxSource::load` is keyed by the
  channel's `sfx` pointer so the mixer's loader no longer aliases the paint
  loop's borrow. (3) FIXED — samples/stepscale outside int range was UB and
  platform-divergent; a float-arithmetic-exact clamp landed in C and is
  mirrored in the shim and test/fuzz gates. (5) FIXED — the Rust shims now
  call SNDDMA_Lock/Submit/Block/Unblock unconditionally like the C (the
  backends no-op with no device under -sndhash). (6) FIXED — music_jump now
  parses with C atoi semantics. (7) FIXED — play/playvol ".wav" append now
  matches q_strlcat truncation. (4) RECORDED — the `cue` chunk-length clamp
  is not strictly behavior-neutral: a well-formed short cue chunk (< 28 bytes
  of data) previously read the following chunk's bytes as loopstart and now
  yields loopstart=-1; accepted as part of the UB-removal, both sides agree.
  (8) RECORDED — the mixer reads 0 past a stale channel's cache where the C
  read out-of-bounds heap (UB); deliberate divergence-on-UB, the
  S_FlushOldestSounds path (needs >4096 distinct sounds) has no test and any
  real divergence would surface as a -sndhash compare failure, not silence.
