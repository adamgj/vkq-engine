---
name: integration-review
description: Run an independent fresh-context compatibility and integration review against an approved vkqr-engine task plan.
argument-hint: [approved-task-plan-path]
disable-model-invocation: true
context: fork
agent: compatibility-reviewer
background: false
---

Review the completed feature against `$ARGUMENTS`.

Inspect the approved task plan, current diff, affected C/Rust/public/FFI contracts, implementation, relevant tests and harness gates, recorded verification, affected CI workflows, and applicable migration phase/ADRs.

Determine whether every acceptance criterion is implemented and demonstrated, cross-platform and cross-language behavior remains coherent, and material compatibility, security, ownership, migration, failure-mode, or regression risks remain.

Stay within the feature boundary. Return evidence-backed findings and readiness status to the parent context. Do not modify files.
