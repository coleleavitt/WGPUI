```markdown
# WGPUI Development Patterns

> Auto-generated skill from repository analysis

## Overview

This skill teaches effective contribution to the WGPUI codebase, a Rust-based UI framework. It covers the project's coding conventions, commit patterns, and the main workflows for developing features, iterating on demos, overhauling subsystems, updating event infrastructure, and merging branches. By following these patterns, you can write code that fits seamlessly into the repository and collaborate efficiently with other contributors.

## Coding Conventions

- **File Naming:**  
  Use `snake_case` for all file and module names.  
  _Example:_  
  ```
  src/text_system/line_layout.rs
  src/elements/text.rs
  examples/learn/blur_showcase.rs
  ```

- **Import Style:**  
  Use relative imports within the crate.  
  _Example:_  
  ```rust
  use crate::text_system::line_layout::LineLayout;
  use super::renderer::Renderer;
  ```

- **Export Style:**  
  Use named exports for modules and functions.  
  _Example:_  
  ```rust
  pub struct TextSystem { /* ... */ }
  pub fn render_text(...) { /* ... */ }
  ```

- **Commit Messages:**  
  - Freeform, no strict prefix required.
  - Average length: ~40 characters.
  - Example:  
    ```
    Add blur style property to renderer
    Update inspector.rs for new layout
    Fix event dispatch in window.rs
    ```

## Workflows

### Feature Development with Examples and Core
**Trigger:** When adding a new rendering or style feature and demonstrating it  
**Command:** `/new-feature-with-demo`

1. Update or add core logic in `src/` (e.g., `src/platform/cross/renderer.rs`, `src/style.rs`, `src/text_system.rs`).
2. Update or add traits or API surfaces (e.g., `src/styled.rs`, `src/elements/*.rs`).
3. Add or update an example/demo in `examples/` (e.g., `examples/learn/blur_showcase.rs`, `examples/learn/text.rs`).
4. Update `Cargo.toml` and/or `Cargo.lock` if dependencies or features are added.
5. Commit your changes with descriptive messages.

_Example:_
```rust
// src/style.rs
pub struct Style {
    pub blur: Option<f32>,
    // ...
}

// examples/learn/blur_showcase.rs
fn main() {
    // Demonstrate blur usage
}
```

---

### Demo Iteration in Examples
**Trigger:** When refining or debugging a UI demo or example  
**Command:** `/iterate-demo`

1. Edit the relevant example file in `examples/` (e.g., `examples/learn/blur_showcase.rs`).
2. Commit changes, often repeatedly in a short time span.
3. Test the demo to ensure desired behavior.

---

### Inspector or Large Feature Overhaul
**Trigger:** When overhauling a feature (e.g., inspector) with incremental changes  
**Command:** `/overhaul-feature`

1. Edit core files for the subsystem (e.g., `src/inspector.rs`, `src/window.rs`, `src/element.rs`).
2. Commit multiple times, often with messages like "Update X.rs".
3. After the overhaul is complete, merge the feature branch into `main`.

---

### Event or Action Dispatch Infrastructure Update
**Trigger:** When changing how events or actions are dispatched  
**Command:** `/update-dispatch`

1. Edit `src/window.rs` and related dispatch files (`src/key_dispatch.rs`, `src/action.rs`, `src/app/context.rs`).
2. Commit sequential changes to refine the dispatch logic.
3. Test event/action flow to ensure correctness.

---

### Merge Feature Branch into Main
**Trigger:** When a feature branch is ready to be integrated into `main`  
**Command:** `/merge-feature`

1. Merge the feature branch (e.g., `blur-overhaul`, `inspector-overhaul`) into `main`.
2. Resolve any conflicts and ensure all related file changes are included.
3. Test the main branch to confirm stability.

---

## Testing Patterns

- **Framework:** Unknown (not explicitly detected).
- **File Pattern:** Test files use the `*.test.*` naming convention.
  - _Example:_ `src/text_system/line_layout.test.rs`
- **Typical Structure:**  
  Use Rust's built-in test framework.
  ```rust
  #[cfg(test)]
  mod tests {
      #[test]
      fn test_line_layout() {
          // test logic here
      }
  }
  ```

## Commands

| Command                | Purpose                                                        |
|------------------------|----------------------------------------------------------------|
| /new-feature-with-demo  | Add a new core feature and demonstrate it with an example      |
| /iterate-demo          | Rapidly iterate on a demo or example file                      |
| /overhaul-feature      | Overhaul or refactor a large subsystem (e.g., inspector)       |
| /update-dispatch       | Update event or action dispatch infrastructure                 |
| /merge-feature         | Merge a feature branch into the main branch                    |
```