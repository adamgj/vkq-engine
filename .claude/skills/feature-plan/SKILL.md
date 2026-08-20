---
name: feature-plan
description: Produce one bounded, repository-grounded architecture and execution plan for a large vkqr-engine feature before implementation.
argument-hint: [feature-brief-or-task-plan-path]
disable-model-invocation: true
model: fable
effort: xhigh
disallowed-tools:
  - Write
  - Edit
  - NotebookEdit
---

Create the authoritative task plan for `$ARGUMENTS`. Do not implement code or modify repository files in this turn.

Read `AGENTS.md`, the feature brief, relevant implementation/contracts, tests or harness coverage, and affected CI workflows. If the task touches `rust/`, Rust integration, C deletion, migration verification, or a deferred subsystem, also read `docs/rust-migration/PLAN.md`, `ROADMAP.md`, the ADR index, and every applicable ADR. Identify the current roadmap phase. The task plan is subordinate to those documents and may not change phase ordering or compatibility policy implicitly.

Investigate only enough to establish requirements, non-goals, invariants, existing patterns, C/Rust ownership boundaries, affected platforms and contracts, migration/data/rollback concerns, risks, and verification. Use `repo-researcher` only for a broad or noisy concrete question.

Do not re-read unchanged material, enumerate alternatives without a material trade-off, redesign unrelated systems, or add speculative capabilities. When evidence is sufficient, choose an architecture.

Return a plan using `docs/ai/FEATURE_PLAN_TEMPLATE.md`. It must include a decision-complete architecture, affected files/modules, existing components to reuse, roadmap/ADR constraints, ordered coherent milestones, acceptance criteria, exact targeted and final verification, non-goals, risks/assumptions, and a repository evidence ledger.

End after presenting the plan for approval. Do not begin implementation or continue searching for optional improvements.
