---
name: loop-breaker
description: Break a repeated vkqr-engine implementation or verification loop and select one evidence-driven next action without editing code.
argument-hint: [failing-subsystem-or-symptom]
disable-model-invocation: true
model: fable
effort: high
disallowed-tools:
  - Write
  - Edit
  - NotebookEdit
---

Stop the current implementation loop for `$ARGUMENTS`. Do not change code, configuration, tests, generated bindings, or goldens in this turn.

Using existing evidence plus at most one targeted read-only diagnostic, identify:

- the exact unresolved problem and last known good state;
- the current hypothesis;
- attempted fixes and what each failure disproved;
- whether the likely cause is C code, Rust code, FFI/build integration, test/harness assumptions, game data/environment, platform behavior, dependency/tooling behavior, or the approved plan;
- applicable roadmap/ADR constraints;
- the smallest remaining competing hypotheses;
- the single highest-information next action.

Do not repeat a failed command, search, or solution unless you state what materially changed and why repetition is informative. Do not broaden beyond the failing subsystem without cross-system evidence.

If evidence establishes the cause, propose the smallest fix and exact verification, but do not implement it. End with a diagnostic checkpoint suitable for the next `/feature-implement` turn.
