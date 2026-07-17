---
name: feature-development-with-examples-and-core
description: Workflow command scaffold for feature-development-with-examples-and-core in WGPUI.
allowed_tools: ["Bash", "Read", "Write", "Grep", "Glob"]
---

# /feature-development-with-examples-and-core

Use this workflow when working on **feature-development-with-examples-and-core** in `WGPUI`.

## Goal

Implements a new core feature or capability, including core logic, style integration, and example/demo usage.

## Common Files

- `src/platform/cross/renderer.rs`
- `src/style.rs`
- `src/text_system.rs`
- `src/text_system/line_layout.rs`
- `src/text_system/line_wrapper.rs`
- `src/styled.rs`

## Suggested Sequence

1. Understand the current state and failure mode before editing.
2. Make the smallest coherent change that satisfies the workflow goal.
3. Run the most relevant verification for touched files.
4. Summarize what changed and what still needs review.

## Typical Commit Signals

- Update or add core logic in src/ (e.g., src/platform/cross/renderer.rs, src/style.rs, src/text_system.rs)
- Update or add trait or API surface (e.g., src/styled.rs, src/elements/*.rs)
- Update or add example/demo in examples/ (e.g., examples/learn/blur_showcase.rs, examples/learn/text.rs, examples/prelude.rs)
- Update Cargo.toml and/or Cargo.lock if dependencies or features are added

## Notes

- Treat this as a scaffold, not a hard-coded script.
- Update the command if the workflow evolves materially.