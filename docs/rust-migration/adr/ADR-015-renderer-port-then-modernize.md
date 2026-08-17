# ADR-015: Renderer — port-then-modernize; ash over vulkano/wgpu

**Status:** Accepted
**Date:** 2026-08-16
**Tags:** —

## Context

The renderer is ~44k LOC of hand-tuned Vulkan: classic render passes with subpasses and input attachments (MBOIT composite), a custom GPU-memory suballocator (`gl_heap.c`, no VMA), ~hundreds of pipelines created up front, a manual dispatch table, ray-query ray tracing (BLAS/TLAS, no RT pipelines), 6-way parallel secondary command-buffer recording driven by the task graph, and MoltenVK direct-linking on macOS. Two temptations exist: modernize while porting (dynamic rendering, VMA, bindless), and use a safety-layer binding (vulkano) or portability layer (wgpu).

## Decision

- **Binding: `ash`.** The port must reproduce an existing Vulkan architecture exactly; ash is a thin, de-facto-standard binding whose handle types are `#[repr(transparent)]` (layout-compatible with the C structs that embed Vulkan handles during the split-renderer window). vulkano's safety model would force architectural rewrites mid-port; wgpu abstracts away required features (subpass input attachments, ray query control, exact memory management). Vulkan via ash is inherently `unsafe` — accepted and confined per ADR-004.
- **Port-then-modernize.** During the port: keep render passes/subpasses (no dynamic rendering), keep `gl_heap` (no VMA — it ports as pure logic, property-tested against C on randomized allocation traces), keep the descriptor layout scheme (no bindless), keep pipeline-creation structure, keep the frame graph's task/dependency shape and the 6-way secondary command-buffer scheme. Modernization proposals are welcome **after** Phase 8 closes, as separate ADRs.
- **Loading path preserved:** Vulkan entry points via `SDL_Vulkan_GetVkGetInstanceProcAddr`; MoltenVK direct link on macOS (ash initialized from the same loader function — mirrors `meson.build` today).
- **Verification & tolerance policy:** pixel output is *not* required to be bit-identical (FP/driver variance makes that unachievable). Gates: screenshot corpus compared by SSIM ≥ threshold per scene (threshold set from C-build run-to-run variance), Vulkan validation layers clean, lavapipe/SwiftShader CI smoke, `timedemo` throughput within regression thresholds on reference hardware. Cull *decisions* (SIMD paths) and draw-call *structure* are compared exactly on captured frames — geometry submitted must match even if shaded pixels vary in low bits.

## Consequences

- The port is a mechanical-as-possible translation of a known-good architecture, minimizing the biggest phase's risk.
- Rust renderer inherits today's design debts (subpass architecture, manual dispatch) temporarily — by design; each modernization gets its own ADR later with the Rust code as a clean base.
- SSIM-not-bitexact is the one compat surface with an explicit tolerance; the policy and thresholds live in the harness configuration and this ADR.
