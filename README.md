# wgpui

A GPU-accelerated UI framework for Rust, forked from [Zed's GPUI](https://github.com/zed-industries/zed) and stripped to the standalone framework with rendering performance optimizations.

## What is this?

wgpui is GPUI extracted from the Zed editor into a standalone workspace (23 crates) with a focus on rendering pipeline performance. It includes the cross-platform wgpu renderer from [Far-Beyond-Pulsar](https://github.com/nicholasgasior/pulsar) and a suite of rendering optimizations:

- **Incremental scene updates** — per-view chunk tracking, only dirty views rebuild their primitives
- **Damage region rendering** — scissor rects clip GPU work to changed screen areas
- **Persistent GPU buffers** — per-type STORAGE buffers with diff-based uploads (zero upload on static frames)
- **Subtree skipping** — clean subtrees skip prepaint and paint entirely
- **Layout caching** — taffy results cached per view, reused when constraints unchanged
- **Batch merging** — same-order primitives grouped by type to reduce draw calls

## Quick Start

```rust
use gpui::*;
use gpui_platform::application;

struct Hello;

impl Render for Hello {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .size_full()
            .justify_center()
            .items_center()
            .bg(rgb(0x1e1e2e))
            .text_color(rgb(0xcdd6f4))
            .child("Hello from wgpui!")
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(WindowOptions::default(), |_, cx| cx.new(|_| Hello));
    });
}
```

## Running the Examples

```bash
# Hello world
cargo run -p gpui --example hello_world

# Rendering benchmark (1000 divs / 10k list items / animated)
cargo run -p gpui --example bench_render --release

# With performance overlay
cargo run -p gpui --example bench_render --release --features gpui/perf-overlay
```

## Workspace Structure

| Crate | Description |
|---|---|
| `gpui` | Core framework — elements, views, entities, styling, input |
| `gpui_platform` | Platform abstraction layer |
| `gpui_linux` | Linux backend (Wayland + X11) |
| `gpui_macos` | macOS backend (Metal) |
| `gpui_windows` | Windows backend |
| `gpui_wgpu` | Cross-platform wgpu renderer (Vulkan/Metal/DX12) |
| `gpui_macros` | Derive macros for actions, elements, etc. |
| `perf` | Frame metrics and performance instrumentation |
| `sum_tree` | Persistent B-tree for efficient list rendering |

## Building

Requires Rust stable (latest) on Linux, macOS, or Windows.

```bash
# Check everything compiles
cargo check --workspace

# Run clippy
./script/clippy

# Build in release mode
cargo build --workspace --release
```

### Linux Dependencies

On Linux you'll need development headers for your display server:

```bash
# Wayland
sudo apt install libwayland-dev libxkbcommon-dev

# X11
sudo apt install libx11-dev libxcb1-dev libxkbcommon-x11-dev
```

## Architecture

GPUI is a hybrid immediate/retained mode framework:

- **Entities** (`Entity<T>`) — owned application state, accessed through smart pointers
- **Views** — entities that implement `Render`, producing element trees each frame
- **Elements** — the building blocks: `div()`, `text()`, `uniform_list()`, etc.
- **Styling** — Tailwind CSS-inspired builder API (`.flex()`, `.bg()`, `.p_4()`, etc.)
- **Actions** — keyboard-driven commands dispatched through the focus tree
- **Async** — integrated executor with `cx.spawn()` and `cx.background_spawn()`

Rendering flows through: layout (taffy) → prepaint → paint → scene sort → GPU upload → draw.

## Acknowledgments

- [Zed Industries](https://zed.dev) — GPUI was created by the Zed team
- [Far-Beyond-Pulsar](https://github.com/nicholasgasior/pulsar) — cross-platform wgpu renderer

## License

See individual crate licenses. GPUI is licensed under Apache-2.0/MIT.
