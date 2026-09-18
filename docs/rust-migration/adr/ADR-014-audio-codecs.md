# ADR-014: Audio codecs remain C behind the codec vtable

**Status:** Accepted
**Date:** 2026-08-16
**Tags:** —

## Context

Music/sound streaming uses a pluggable codec registry (`snd_codec.c`, `snd_codec_t` vtable: init/shutdown/open/read/rewind/jump/close) with codecs backed by external C libraries: libmad **or** libmpg123 (MP3), libvorbis/tremor (Vorbis), libopus+opusfile (Opus), libFLAC, libogg underneath — system packages on Linux/macOS, bundled DLLs on Windows. WAV and UMX are engine-native parsers. Rust alternatives exist (Symphonia — MP3/FLAC/Vorbis/etc., `lewton` — Vorbis), but decoded output is not guaranteed bit-identical to the C decoders, and music decode fidelity is user-audible rather than sim-observable.

Licensing note: libmad is GPL (compatible with the GPLv2+ engine); Symphonia is MPL-2.0 (also compatible).

## Decision

- The codec **framework** ports to Rust in Phase 4: a `Codec` trait mirroring the `snd_codec_t` vtable, registration order preserved (registration order is lookup-preference order), stream state in safe Rust.
- The codec **decoders stay C**, wrapped as trait implementations over thin FFI to the same external libraries currently used. They are ADR-002 native remnants: isolated, behind one trait, individually swappable.
- WAV, UMX (container), and MP3 tag skipping (`snd_mp3tag.c`) port to pure Rust in Phase 4 (they are parsers of untrusted input — highest value for memory safety).
- **Optional later migration:** any codec may move to Symphonia/lewton as a per-codec PR *after* Phase 4, gated on listening-test acceptance + decode-comparison metrics (not bit-identity — decoded PCM is not a compat surface as long as sound plays correctly; the mixer above it *is* PCM-hash tested with fixed decoder inputs).
- Makefile-only codecs (mikmod/xmp/modplug) are removed per ADR-018.

## Consequences

- Phase 4 delivers a Rust mixer and framework without betting compatibility on decoder swaps.
- External codec libraries remain runtime dependencies exactly as today (no packaging changes).
- The per-codec swap path is open-ended future work with a clear gate, not a migration blocker.

## Amendment (Phase 10 M4, 2026-09-16): the Symphonia option is closed by ADR-003

The "optional later migration" above named Symphonia (MPL-2.0) and lewton.
[ADR-003](ADR-003-dependency-policy.md) forbids MPL-2.0 anywhere in the
dependency tree, so Symphonia is not an option under the project's crate
policy; the licensing note above is superseded. `lewton` (MIT/Apache-2.0)
remains admissible for Vorbis only. The codec bridges are therefore a
**standing** ADR-002 remnant (listed in that ADR's Phase-10 appendix), not
a pending swap: any future per-codec replacement needs a permissive decoder
and its own ADR with the listening-test gate above.
