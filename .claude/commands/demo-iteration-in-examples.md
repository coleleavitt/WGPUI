---
name: demo-iteration-in-examples
description: Workflow command scaffold for demo-iteration-in-examples in WGPUI.
allowed_tools: ["Bash", "Read", "Write", "Grep", "Glob"]
---

# /demo-iteration-in-examples

Use this workflow when working on **demo-iteration-in-examples** in `WGPUI`.

## Goal

Rapidly iterates on a demo or example file, often with a series of small commits to a single file.

## Common Files

- `examples/learn/blur_showcase.rs`
- `examples/learn/text.rs`
- `examples/legacy/input.rs`

## Suggested Sequence

1. Understand the current state and failure mode before editing.
2. Make the smallest coherent change that satisfies the workflow goal.
3. Run the most relevant verification for touched files.
4. Summarize what changed and what still needs review.

## Typical Commit Signals

- Edit the relevant example file in examples/ (e.g., examples/learn/blur_showcase.rs)
- Commit changes, often repeatedly in a short time span

## Notes

- Treat this as a scaffold, not a hard-coded script.
- Update the command if the workflow evolves materially.