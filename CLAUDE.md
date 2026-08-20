# CLAUDE.md

@AGENTS.md

## Claude Code controls

- Use `high` effort for normal complex implementation. Use `xhigh` only for the bounded `/feature-plan` pass or a specifically identified capability-sensitive decision.
- Do not use `max`, `ultracode`, `/loop`, a Stop hook, or an agent team unless the user explicitly authorizes it for a documented reason.
- Use `/feature-plan` once and `/feature-implement` one approved milestone at a time. For Rust work, the migration plan, roadmap, and ADRs remain authoritative.
- Use `repo-researcher` for noisy read-only discovery and `verification-diagnostician` for large build/test/harness failures; do not delegate routine targeted reads.
- After repeated failure, use `/loop-breaker` for a read-only diagnostic turn instead of continuing edits.
- Use `/integration-review` from a fresh context before completing a high-risk, cross-platform, compatibility-sensitive, or Rust-migration feature.
- Keep progress claims grounded in current tool evidence. Compact only at a coherent boundary using `docs/ai/FABLE5_WORKFLOW.md`.
- When the selected milestone or requested feature is complete and verified, stop.
