---
name: feature-implement
description: Implement one approved vkqr-engine feature milestone with Fable 5 while preserving compatibility, context, scope, and verification quality.
argument-hint: [approved-task-plan-path] [milestone-id]
disable-model-invocation: true
model: fable
effort: high
---

Implement only the milestone identified by `$ARGUMENTS` from the approved task plan.

Before editing, read the milestone, applicable `AGENTS.md` guidance, relevant implementation/tests/contracts, and affected CI workflow. For migration work, reread the applicable roadmap phase and ADRs; the task plan cannot override them. Confirm requirements, non-goals, invariants, authorized systems, compatibility surfaces, and acceptance criteria in a concise working summary.

Treat approved decisions as authoritative. Re-plan only if repository evidence proves a decision impossible, unsafe, materially incorrect, or inconsistent with a requirement/ADR. State the contradiction and smallest plan amendment before changing direction.

Implement the smallest complete solution. Preserve established C99, Meson, Vulkan/SDL, Rust/FFI, and compatibility patterns as applicable. Do not perform unrelated cleanup, renaming, modernization, dependency replacement, speculative abstraction, phase advancement, or C deletion outside the approved roadmap milestone.

Do not repeat unchanged searches, file reads, commands, or failing edits. One retry is allowed only for evidence of a transient failure. After two failed attempts at the same problem, stop editing and diagnose. After two distinct approaches fail, state what each disproved before another attempt. Use `/loop-breaker` on the next turn when a clean diagnostic checkpoint is needed.

Keep noisy discovery and build/harness output out of this context: use `repo-researcher` or `verification-diagnostician` only for a concrete bounded question. Do not create an agent team unless the approved plan explicitly justifies independent complex workstreams.

Verify incrementally with the cheapest relevant check first. Use the affected CI workflow and `docs/ai/FABLE5_WORKFLOW.md` to select milestone/final gates. Never regenerate compatibility goldens except under the documented c-reference/platform rules.

At the end of this turn:

1. Stop if the selected milestone is complete; do not start the next milestone.
2. Report changed files and behavior concisely.
3. Map verification results to acceptance criteria and distinguish targeted, broad, manual, and not run.
4. Report unresolved risk, blockers, or unverified items explicitly.
5. Provide the exact evidence-ledger and handoff updates for the task plan.
