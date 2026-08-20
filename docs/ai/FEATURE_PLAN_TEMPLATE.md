# Feature task plan: [feature name]

Status: draft | approved | implementing | complete
Owner: [name/team]
Baseline: [branch/commit]
Last materially updated: [date/commit]

For Rust-migration work, this task plan is subordinate to `docs/rust-migration/PLAN.md`, `ROADMAP.md`, and applicable ADRs. It cannot change phase ordering or compatibility policy without an explicit approved documentation/ADR change.

## Objective

[One precise paragraph describing the user or engine outcome.]

## Requirements and non-goals

- R1: [testable required behavior]
- NG1: [explicitly excluded adjacent work]

## Invariants and compatibility surfaces

- I1: [C99/platform/Vulkan/SDL/security/data invariant]
- Preserved behavior: [savegame/demo/network/protocol/mod/config/render/audio behavior as applicable]
- Public, FFI, data-layout, ownership, or error contracts: [paths/symbols]
- Supported platforms/configurations: [matrix]

## Migration authority (when applicable)

- Roadmap phase: [phase and current status]
- Applicable ADRs: [IDs and constraints]
- C reference/oracle: [implementation/build]
- Deferred systems or deletion restrictions: [items]

## Repository evidence

| Fact/dependency | Evidence (file/symbol/workflow/doc) | Confidence/uncertainty |
|---|---|---|
| [fact] | `[path:symbol]` | confirmed |

Record durable conclusions, not full file contents or raw output.

## Architecture and decisions

### D1: [decision]

- Choice: [concrete design]
- Why: [requirement/evidence/ADR]
- Material alternative, if any: [decisive trade-off]
- Consequences: [platform/build/FFI/compatibility/verification]

## Change boundary

### Expected to change

- `[path/module]`: [reason]

### Must not change without plan amendment

- `[path/system/contract]`: [reason]

## Acceptance matrix

| ID | Acceptance criterion | Verification/gate | Evidence/status |
|---|---|---|---|
| AC1 | [observable new behavior] | [specific check] | pending |
| AC2 | [preserved behavior/invariant] | [regression/harness check] | pending |

## Milestones

### M1: [coherent milestone]

- Scope: [exact behavior/components]
- Expected files: [paths/modules]
- Acceptance criteria: [AC IDs]
- Targeted verification: [exact commands/manual check]
- Completion evidence: pending

Repeat for each milestone. Every milestone must leave the repository buildable/coherent and must not silently advance the Rust roadmap.

## Final verification

- Formatting: [applicable command]
- Targeted build/tests: [commands]
- Affected CI workflows: [names]
- Differential harness/FFI/bindgen/license/advisory gates: [commands or not applicable]
- Cross-platform or manual checks that cannot run locally: [explicit list]

## Risks, assumptions, and open questions

| ID | Type | Item | Mitigation/decision | Status |
|---|---|---|---|---|
| RA1 | risk | [material risk] | [mitigation] | open |

Only questions that materially alter architecture, scope, compatibility, or acceptance should block approval.

## Plan amendment log

| Date | Repository contradiction/evidence | Smallest amendment | Acceptance impact | Approval |
|---|---|---|---|---|

Do not rewrite history or re-plan because another design merely exists.

## Verification evidence and handoff

| Milestone | Changed files/behavior | Check and result | Acceptance IDs | Remaining risk/next action |
|---|---|---|---|---|

## Completion gate

- [ ] Requirements map to acceptance criteria.
- [ ] Required compatibility and ADR constraints are satisfied.
- [ ] Current evidence exists for every completed acceptance criterion.
- [ ] Required CI/harness/manual verification is complete or explicitly not run.
- [ ] Independent integration review is complete for high-risk work.
- [ ] Remaining risk is explicit and accepted.
- [ ] The diff contains no unrelated work.
