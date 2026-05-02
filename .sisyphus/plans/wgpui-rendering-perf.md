# WGPUI Rendering Performance Overhaul — Work Plan

## Status: DRAFT — Awaiting Decision Points Resolution

---

## 1. Context

WGPUI is a Zed GPUI fork stripped to 23 core framework crates. The rendering
engine has three fundamental performance problems that compound into unnecessary
CPU and GPU work every frame.

### 1.1 Current Rendering Pipeline

```
on_request_frame (platform vsync callback)
  → Window::draw(cx)
      → invalidate_entities()                // collect dirty views
      → draw_roots(cx)
          → DrawPhase::Prepaint
              → root_element.prepaint_as_root()
              → deferred_draws prepaint
          → DrawPhase::Paint
              → (same elements).paint()
              → Scene accumulates PaintOperations
      → next_frame.finish()                  // sort ALL primitives by draw order
      → swap(rendered_frame, next_frame)
      → needs_present.set(true)
  → Window::present()
      → platform_window.draw(&rendered_frame.scene)
          → WgpuRenderer::draw(&scene)
              → write_buffer (globals)
              → write_buffer (instances — ALL data every frame)
              → for each primitive type:
                  → draw_quads / draw_shadows / draw_underlines / ...
              → queue.submit()
              → surface.present()
```

### 1.2 Three Core Problems

**Problem 1: Full scene rebuild every frame.**
`Scene::clear()` wipes all primitive vectors. Even cached views (not dirty)
re-copy their primitives via `Scene::replay()`. Then `Scene::finish()` re-sorts
everything. For complex UIs: thousands of cloned structs + full sort per frame.

**Problem 2: Full GPU data reupload every frame.**
`WgpuRenderer::draw()` calls `queue.write_buffer()` with ALL instance data
every frame. The instance buffer is resized on demand (line 1702 of
`wgpu_renderer.rs`: `create_buffer` when capacity exceeded) but data is always
fully reuploaded. No concept of "this quad hasn't changed."

**Problem 3: No damage-region scoping.**
The render pass clears the entire framebuffer and redraws everything. Even if
only a cursor blink changed, the full screen gets a clear + full draw pass.

### 1.3 Existing Caching (Foundation to Build On)

GPUI already has view-level dirty tracking:
- `dirty_views: FxHashSet<EntityId>` tracks which views need re-rendering
- `mark_view_dirty()` walks ancestors, marking the path dirty
- `reuse_prepaint()` / `reuse_paint()` skip Rust layout/paint for clean views
- `Scene::replay(range, prev_scene)` copies previous frame primitives

This means the framework KNOWS which views changed. The gap is propagating that
knowledge through the scene and GPU layers to avoid redundant work.

### 1.4 Two Renderers Available

| Renderer | Location | Buffer Strategy | Status |
|---|---|---|---|
| `gpui_wgpu` (current) | `crates/gpui_wgpu/src/wgpu_renderer.rs` (1956 lines) | Resize-on-demand, full reupload | **Active, compiles** |
| Pulsar cross-platform | `crates/gpui/src/platform/cross/renderer.rs` (vendored) | Pre-allocated persistent typed buffers | **Vendored reference** |

Pulsar's `render_context.rs` pre-allocates typed buffers at init:
```rust
quads_buffer = device.create_buffer(...)       // persistent
shadows_buffer = device.create_buffer(...)     // persistent
underlines_buffer = device.create_buffer(...)  // persistent
mono_sprites_buffer = device.create_buffer(...) // persistent
poly_sprites_buffer = device.create_buffer(...) // persistent
```
This is the pattern the current renderer should adopt.

---

## 2. Decision Points (BLOCKING — Need Answers Before Phase 3+)

### DP-1: Renderer Base
**Question**: Retrofit `gpui_wgpu` renderer with persistent buffers, or port
Pulsar's renderer into the `gpui_wgpu` crate?

**Recommendation**: Retrofit `gpui_wgpu`. It's already wired into the module
tree, compiles, and is 1956 lines vs Pulsar's more complex multi-file setup.
Porting Pulsar wholesale risks introducing bugs from a different module
hierarchy. Use Pulsar as reference for the buffer allocation patterns.

### DP-2: Backward Compatibility
**Question**: Must existing apps using GPUI keep working unchanged through
this work, or is API breakage acceptable?

**Recommendation**: Keep the public API stable (Render, RenderOnce, Element
traits, div()/styled() builder). Internal APIs (Scene, Frame, PaintOperation)
can change since they're `pub(crate)`.

### DP-3: Benchmark Baseline Target
**Question**: What's the target device/scenario for benchmarks? A specific
app? An artificial stress test? What FPS is "good enough"?

**Recommendation**: Create a synthetic benchmark that renders a complex UI
(1000+ elements, scrollable list, animated elements) and measures:
- Frame time (ms) at p50/p95/p99
- CPU time in draw_roots vs present
- GPU buffer upload size per frame
- Draw call count

### DP-4: Platform Scope
**Question**: Optimize for Linux first (wgpu/Vulkan), all platforms, or
does macOS Metal path matter equally?

**Recommendation**: Linux/Vulkan first since that's the `gpui_wgpu` path.
macOS (`gpui_macos`) uses a different renderer entirely and would be a
separate effort.

---

## 3. Phases

### Phase 0: Instrumentation & Baselines (PREREQUISITE)
**Estimated effort**: 2-3 days
**Dependencies**: None
**Risk**: Low

Without measurements, we can't prove improvement or detect regressions.

#### 0.1 Frame Timing Infrastructure
- [x] Add frame-time measurement to `Window::draw()` and `Window::present()`
- [x] Measure time in each sub-phase:
  - `invalidate_entities()` duration
  - `draw_roots()` → prepaint duration
  - `draw_roots()` → paint duration
  - `Scene::finish()` (sort) duration
  - `WgpuRenderer::draw()` total duration
  - `queue.write_buffer()` cumulative bytes per frame
  - Draw call count per frame
- [x] Expose via the existing `perf` crate (replace stub with real impl)
- [x] Add opt-in frame-time overlay (conditional on feature flag)

**Files to modify**:
- `crates/perf/src/perf.rs` — real implementation
- `crates/gpui/src/window.rs` — timing hooks in `draw()` and `present()`
- `crates/gpui_wgpu/src/wgpu_renderer.rs` — GPU submission timing

#### 0.2 Benchmark Harness
- [x] Create `crates/gpui/examples/bench_render.rs` stress test
- [x] Scenario: window with 1000+ div elements, nested 5 levels deep
- [x] Scenario: scrollable `uniform_list` with 10,000 items
- [x] Scenario: animated element (forces redraw every frame)
- [ ] Record baseline numbers before any optimization

**Acceptance criteria**:
- Can measure frame time breakdown per phase
- Can reproduce measurements consistently (< 5% variance)
- Baseline numbers documented in `.sisyphus/evidence/baseline.md`

---

### Phase 1: Incremental Scene (Foundation)
**Estimated effort**: 5-7 days
**Dependencies**: Phase 0
**Risk**: Medium — touches core data structure used by every render path

This is the foundation that Phase 2-4 build on. Currently `Scene::clear()`
wipes everything and `Scene::replay()` bulk-copies from prev frame. The goal
is a persistent scene that applies diffs.

#### 1.1 Persistent Scene Structure
- [x] Remove `Scene::clear()` call from the frame cycle
- [x] Instead of clearing, maintain scene across frames
- [x] Each view's primitives are stored in a contiguous range
  (they already are — `paint_operations` tracks ranges per view)
- [x] When a view is dirty: replace its range in the scene
- [x] When a view is clean: leave its range untouched (no replay/copy)

**Key insight**: The current `Scene::replay(range, prev_scene)` copies
primitives from rendered_frame to next_frame. If the scene is persistent,
clean views don't need ANY copy — their data is still there.

**Files to modify**:
- `crates/gpui/src/scene.rs` — persistent storage, range-based updates
- `crates/gpui/src/window.rs` — remove clear/swap cycle, use single scene
- `crates/gpui/src/view.rs` — update `reuse_prepaint`/`reuse_paint` to skip
  scene ops entirely for clean views

#### 1.2 Incremental Sort
- [x] `Scene::finish()` currently sorts ALL primitives by draw order
- [x] With persistent scene, maintain sorted order incrementally:
  - When a view's range changes, re-sort only the affected region
  - Use insertion-based approach for small changes
  - Fall back to full sort if > 30% of scene changed
- [x] Track a `scene_generation: u64` counter incremented on changes

**Files to modify**:
- `crates/gpui/src/scene.rs` — `finish()` → `update()` with partial sort

#### 1.3 Frame Double-Buffering Adjustment
- [ ] Currently: `rendered_frame` and `next_frame` are swapped each frame
- [ ] With persistent scene: single scene + dirty flag per range
- [ ] `rendered_frame` becomes the previous state for diffing
- [ ] `next_frame` accumulates only CHANGES, then patches the persistent scene

**Acceptance criteria**:
- Frame time for static UI (no dirty views) drops to near-zero CPU
  for scene construction (target: < 0.1ms for 1000-element UI)
- Frame time for single dirty view scales with view complexity, not
  total scene complexity
- All existing rendering output is pixel-identical
- `cargo check --workspace` passes
- Benchmark numbers recorded in `.sisyphus/evidence/phase1.md`

---

### Phase 2: Damage Region Tracking
**Estimated effort**: 3-5 days
**Dependencies**: Phase 1
**Risk**: Medium

Once the scene is persistent and we know which ranges changed, we can compute
damage rectangles and scope the render pass.

#### 2.1 Damage Rect Computation
- [x] When a view's scene range is updated, compute its bounding rect
- [x] Union all dirty view bounding rects into a damage region
- [x] Store `damage_rects: Vec<Rect>` on the Frame/Scene
- [x] Merge overlapping rects to avoid redundant passes

**Reference**: NVIDIA DRM driver uses per-property dirty bits
(`surfaceChanged`, `srcXYChanged`, etc.) — same concept at UI level.

**Files to modify**:
- `crates/gpui/src/scene.rs` — damage rect computation
- `crates/gpui/src/window.rs` — propagate damage rects to renderer

#### 2.2 Scissored Render Pass
- [x] Pass damage rects to `WgpuRenderer::draw()`
- [x] Set scissor rect on the render pass to clip to damaged region
- [x] Skip `load_op: Clear` for the full framebuffer — only clear damaged rects
- [x] For non-damaged regions, use `load_op: Load` to preserve previous frame

**Note**: wgpu supports `set_scissor_rect()` on the render pass encoder.
Multiple scissor rects require multiple render passes or a single merged rect.
Start with single merged rect (union of all damage rects).

**Files to modify**:
- `crates/gpui_wgpu/src/wgpu_renderer.rs` — scissor rect, conditional clear

#### 2.3 Full-Redraw Fallback
- [x] If damage region covers > 70% of screen area, fall back to full redraw
- [x] Window resize always triggers full redraw
- [x] First frame after window open is always full redraw

**Acceptance criteria**:
- Cursor blink in a text field triggers damage rect covering only the
  cursor area, not the full screen
- Frame time for localized changes (single button hover) drops by > 50%
  compared to Phase 1 baseline
- Full-redraw fallback works correctly for resize
- Benchmark numbers recorded in `.sisyphus/evidence/phase2.md`

---

### Phase 3: GPU-Resident Persistent Buffers
**Estimated effort**: 5-7 days
**Dependencies**: Phase 1 (persistent scene needed to know what changed)
**Risk**: High — directly touches GPU memory management

The current `gpui_wgpu` renderer reuploads ALL instance data every frame via
`queue.write_buffer()`. The goal is persistent GPU buffers that only receive
diffs.

#### 3.1 Persistent Typed Buffers
- [x] Pre-allocate typed GPU buffers at renderer init (following Pulsar pattern):
  ```
  quads_buffer:         wgpu::Buffer  // persistent
  shadows_buffer:       wgpu::Buffer  // persistent
  underlines_buffer:    wgpu::Buffer  // persistent
  mono_sprites_buffer:  wgpu::Buffer  // persistent
  poly_sprites_buffer:  wgpu::Buffer  // persistent
  paths_vertices_buffer: wgpu::Buffer // persistent
  ```
- [x] Size buffers with headroom (2x expected max, grow-only)
- [x] Replace per-frame `write_buffer(entire_data)` with partial writes

**Reference**: `crates/gpui/src/platform/cross/render_context.rs` lines 83-145
shows exactly this pattern. Port the allocation strategy, not the whole file.

**Files to modify**:
- `crates/gpui_wgpu/src/wgpu_renderer.rs` — buffer management overhaul
- `crates/gpui_wgpu/src/wgpu_context.rs` — if buffer init needs to move here

#### 3.2 Diff-Based Upload
- [x] Scene provides `changed_ranges()` — which primitive indices changed
  (from Phase 1's persistent scene tracking)
- [x] Renderer only calls `queue.write_buffer()` for changed byte ranges
- [x] Use `write_buffer_with()` or offset writes for sub-buffer updates
- [x] Track per-buffer dirty ranges: `quads_dirty: Option<Range<u64>>`

**Key optimization**: If scene says "quads 50-75 changed", only upload
those 25 quads (25 * sizeof(Quad) bytes) instead of all 500 quads.

#### 3.3 Buffer Compaction
- [ ] When views are removed, their GPU buffer slots become holes
- [ ] Track free ranges per buffer type
- [ ] Compact when fragmentation exceeds 30% (copy remaining data, defrag)
- [ ] Compaction is a full reupload — amortized cost

**Acceptance criteria**:
- Static UI frame: zero bytes uploaded to GPU (target: 0 write_buffer calls)
- Single dirty view frame: bytes uploaded proportional to that view's
  primitive count, not total primitive count
- Buffer memory usage is bounded (no unbounded growth)
- GPU memory reported in benchmark overlay
- Benchmark numbers recorded in `.sisyphus/evidence/phase3.md`

---

### Phase 4: Fiber-Style Partial Redraws
**Estimated effort**: 7-10 days
**Dependencies**: Phase 1 + Phase 2 + Phase 3
**Risk**: High — most complex change, touches element tree traversal

This is the "React Fibers" equivalent: interrupt and resume layout/paint work,
and skip entire subtrees that haven't changed. Currently `draw_roots()` walks
the ENTIRE element tree even if only one leaf changed.

#### 4.1 Subtree Skip During Prepaint
- [x] During `DrawPhase::Prepaint`, check if a view is in `dirty_views`
- [x] If not dirty AND all descendants are not dirty: skip entire subtree
  (currently `reuse_prepaint` replays — goal is to skip entirely)
- [ ] Requires knowing if ANY descendant is dirty — extend `mark_view_dirty()`
  to propagate a "has_dirty_descendant" flag up the tree
- [ ] Skip means: don't call `request_layout`, don't call `prepaint`,
  don't touch taffy layout nodes for this subtree

**Files to modify**:
- `crates/gpui/src/window.rs` — `draw_roots()`, `mark_view_dirty()`
- `crates/gpui/src/view.rs` — subtree skip logic in `AnyView::draw()`
- `crates/gpui/src/element.rs` — `Element::prepaint` skip path

#### 4.2 Subtree Skip During Paint
- [x] Same skip logic for `DrawPhase::Paint`
- [x] Clean subtrees don't generate ANY new `PaintOperation`s
- [ ] Their data persists in the persistent scene (Phase 1)

#### 4.3 Layout Caching
- [x] Cache taffy layout results per view
- [x] If a view's input constraints haven't changed AND it's not dirty,
  reuse cached layout (don't call taffy at all)
- [ ] Invalidate layout cache when:
  - View is dirty (content changed)
  - Parent's available space changed (window resize, sibling layout shift)
  - Explicit `cx.notify()` was called

**Files to modify**:
- `crates/gpui/src/window.rs` — layout cache storage
- `crates/gpui/src/taffy.rs` — cache integration

#### 4.4 Interruptible Layout (Stretch Goal)
- [ ] For very large element trees (10,000+ elements), allow layout to
  be interrupted and resumed across frames
- [ ] Process N elements per frame, prioritizing visible/dirty ones
- [ ] This is the true "fiber" concept — cooperative scheduling of UI work
- [ ] **Only attempt if Phase 4.1-4.3 are complete and stable**

**Acceptance criteria**:
- Frame time for 1000-element UI with single dirty leaf: < 1ms total
  (most subtrees skipped entirely)
- Frame time for fully-dirty UI: no regression vs Phase 3
- Element tree traversal count measurable and proportional to dirty subtree
  size, not total tree size
- Benchmark numbers recorded in `.sisyphus/evidence/phase4.md`

---

### Phase 5: List Element Caching (Independent Track)
**Estimated effort**: 3-4 days
**Dependencies**: None (can run in parallel with Phase 1-4)
**Risk**: Low

#### 5.1 Element Cache for uniform_list
- [x] Cache `AnyElement` instances across frames for visible items
- [x] When scroll position changes, reuse elements that are still visible
- [x] Only create new elements for items entering the viewport
- [x] Destroy elements for items leaving the viewport

**Files to modify**:
- `crates/gpui/src/elements/uniform_list.rs`

#### 5.2 Element Cache for list (Variable-Height)
- [x] Same caching for `list.rs` but respecting height changes
- [x] `SumTree<ListItem>` already tracks `Rendered { element }` vs
  `Unrendered { height }` — extend to keep rendered elements longer
- [ ] Tune `overdraw` parameter: profile to find optimal value
  (currently hardcoded, should be adaptive based on scroll velocity)

**Files to modify**:
- `crates/gpui/src/elements/list.rs`

#### 5.3 Hybrid Overdraw
- [x] Implement velocity-based overdraw: fast scroll = more overdraw,
  slow/stopped scroll = less overdraw
- [ ] Cap overdraw at 2x viewport height

**Acceptance criteria**:
- Scrolling a uniform_list of 10,000 items: element creation count per
  frame proportional to newly-visible items, not total visible items
- No visible pop-in during normal scroll speeds
- Memory usage bounded (cached elements evicted when far from viewport)

---

### Phase 6: Batch Merging Optimization (Quick Win)
**Estimated effort**: 1-2 days
**Dependencies**: None (can run in parallel)
**Risk**: Low

#### 6.1 Sort Primitives to Maximize Batching
- [x] Currently `PrimitiveBatch` merges consecutive same-type primitives
- [x] If two quad batches are separated by a single underline, that's 3 draw
  calls instead of potentially 2 (if underline could be reordered)
- [ ] Profile to measure actual batch count in typical UIs
- [ ] If batch count is high (> 50 per frame), investigate sort key
  adjustment to group same-type primitives while respecting draw order

**Files to modify**:
- `crates/gpui/src/scene.rs` — `finish()` sort key, `batches()` iterator

#### 6.2 Instanced Multi-Draw (Stretch Goal)
- [ ] If wgpu supports `multi_draw_indirect`, batch multiple draw calls
  into a single GPU command
- [ ] Reduces CPU→GPU command overhead

**Acceptance criteria**:
- Draw call count per frame reduced by > 20% for typical UI
- No visual artifacts from primitive reordering
- Benchmark numbers recorded

---

## 4. Dependency Graph

```
Phase 0 (Instrumentation)
    │
    ├── Phase 1 (Incremental Scene) ──┬── Phase 2 (Damage Regions)
    │                                  │
    │                                  └── Phase 3 (GPU Persistent Buffers)
    │                                        │
    │                                        └── Phase 4 (Fiber Redraws)
    │
    ├── Phase 5 (List Caching) ── independent, can parallelize
    │
    └── Phase 6 (Batch Merging) ── independent, can parallelize
```

**Critical path**: 0 → 1 → 3 → 4 (longest chain, ~20-27 days)
**Parallel track A**: 5 (list caching, 3-4 days, any time)
**Parallel track B**: 6 (batch merging, 1-2 days, any time)
**Phase 2** can start after Phase 1, runs parallel with Phase 3.

---

## 5. Anti-Goals (Explicitly Out of Scope)

- **Rewriting the element tree API** (Render, RenderOnce, IntoElement traits).
  Too much blast radius. Optimize within the existing API surface.
- **Unifying the two wgpu renderers** (`gpui_wgpu` vs vendored Pulsar).
  Separate effort. Use Pulsar as reference only.
- **macOS Metal renderer optimization**. `gpui_macos` is a different backend.
  Separate effort, different skill set.
- **New widget/component development**. This is a rendering engine overhaul,
  not a UI component library expansion.
- **WebGPU/WASM support**. `gpui_web` exists but is not in scope.
- **Changing the wgpu backend version**. Stay on current pinned wgpu fork.
- **Async/parallel rendering**. All rendering stays on the foreground thread
  as GPUI requires. Background work is for data prep only.

---

## 6. Risk Register

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Persistent scene causes stale rendering artifacts | Medium | High | Compare frame output against full-redraw reference in tests |
| GPU buffer fragmentation causes memory growth | Medium | Medium | Compaction pass (Phase 3.3), memory budget cap |
| Subtree skip misses dirty elements | Medium | High | Conservative: if uncertain, redraw. Add debug mode that full-redraws and compares |
| Layout cache invalidation misses edge cases | High | Medium | Test with resize, scroll, animation, focus change, IME |
| wgpu scissor rect overhead exceeds savings for small damage | Low | Low | Full-redraw fallback when damage > 70% of screen |
| Incremental sort is slower than full sort for large changes | Medium | Low | Threshold-based: incremental for < 30% changed, full sort otherwise |
| Phase 4 interruptible layout introduces jank | Medium | Medium | Stretch goal only, not in critical path |

---

## 7. File Inventory (All Files Expected to Change)

### Core rendering pipeline:
- `crates/gpui/src/scene.rs` — Phases 1, 2, 6
- `crates/gpui/src/window.rs` — Phases 1, 2, 4
- `crates/gpui/src/view.rs` — Phases 1, 4
- `crates/gpui/src/element.rs` — Phase 4
- `crates/gpui/src/taffy.rs` — Phase 4

### GPU renderer:
- `crates/gpui_wgpu/src/wgpu_renderer.rs` — Phases 0, 2, 3

### List elements:
- `crates/gpui/src/elements/uniform_list.rs` — Phase 5
- `crates/gpui/src/elements/list.rs` — Phase 5

### Instrumentation:
- `crates/perf/src/perf.rs` — Phase 0

### New files:
- `crates/gpui/examples/bench_render.rs` — Phase 0
- `.sisyphus/evidence/baseline.md` — Phase 0
- `.sisyphus/evidence/phase{1,2,3,4}.md` — Each phase

### Reference (read-only):
- `crates/gpui/src/platform/cross/render_context.rs` — Buffer patterns
- `crates/gpui/src/platform/cross/renderer.rs` — Renderer patterns

---

## 8. Verification Strategy

### Per-Phase:
1. `cargo check --workspace` passes
2. Benchmark harness (Phase 0) shows improvement or no regression
3. Visual diff: screenshot comparison between optimized and full-redraw paths
4. Edge case tests: resize, scroll, animation, focus, IME, drag-and-drop

### End-to-End:
1. Build a sample app that exercises all element types
2. Run with frame overlay showing timing breakdown
3. Compare against Phase 0 baseline
4. Static UI frame: target < 0.5ms total (CPU + GPU)
5. Single dirty view frame: target CPU time proportional to view size only
6. Full animation frame: no regression vs current

---

## 9. Open Questions (Non-Blocking)

1. Should the `perf` crate become a real profiling framework or stay minimal?
2. Is there an existing GPUI test suite we should run against? (Tests may have
   been stripped with Zed — need to check.)
3. Does Tristan's team have a target application to benchmark against, or
   should we use synthetic benchmarks only?
4. Should Phase 5 (list caching) optimize for specific list sizes (e.g.,
   the sizes used in their product) or be general-purpose?
