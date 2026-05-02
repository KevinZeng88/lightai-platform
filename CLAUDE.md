# Agent Instructions

## Work Style

Understand the current code before changing it.

Keep changes small, focused, and easy to review.

Do not expand scope unless explicitly asked.

Prefer simple MVP changes over premature architecture.

State important assumptions. Do not pretend something was verified if it was not.

## Code Rules

Preserve existing behavior unless the task requires changing it.

Avoid unnecessary rewrites and new dependencies.

Make errors visible and actionable.

Keep user-facing text clear and consistent.

## Safety Rules

Do not execute arbitrary user-provided commands.

Do not build shell command strings from untrusted input.

Use argv-style execution for controlled programs.

Do not perform destructive file operations unless explicitly requested and safely bounded.

Validate paths before file operations.

Do not log tokens, passwords, API keys, authorization headers, or other secrets.

## Checks

After code changes, run applicable checks:

\`\`\`bash
cargo fmt --all --check
cargo test --workspace
cargo build --workspace
cd web && npm run build
\`\`\`

If a check cannot be run, try to resolve the issue and retry. If it still cannot be run after reasonable attempts, explain why and what is needed to run it.

## Final Summary

After changes, summarize:

- What changed
- Main files modified
- Validation results
- Known limitations
