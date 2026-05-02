# Draft: WGPUI Rendering Performance Overhaul

## Requirements (confirmed)
- **Fiber-style partial redraws**: Only redraw dirty screen regions, not entire screen every frame
- **GPU-resident buffers**: UI elements should persist on GPU rather than CPU-side reupload each frame
- **Draw call reduction**: Less draw-call-heavy UI primitives
- **Virtual scrolling**: Investigate built-in virtual scrolling in GPUI (CONFIRMED: exists via uniform_list + list)
- **Motivation**: 3D renderer had same 10 FPS bug with reupload approach; fixed by switching to persistent GPU buffers

## Technical Decisions
- **Framework**: WGPUI = Zed GPUI fork, 15 crates (stripped from 234)
- **Renderer**: BladeRenderer using blade-graphics (wgpu-like)
- **Existing caching**: dirty_views + reuse_prepaint/paint + Scene::replay — view-level only
- **NVIDIA driver pattern**: Per-property dirty bits, persistent GPU surfaces — model to follow

## Research Findings
- **Scene**: Cleared + rebuilt every frame. Even cached views replay (clone) primitives. finish() re-sorts all.
- **GPU submission**: instance_belt ring allocator, ALL data reuploaded each frame
- **Scrolling**: uniform_list (fixed-height virtual) and list (variable-height via SumTree) both exist
- **Dirty tracking**: WindowInvalidator tracks dirty entities, mark_view_dirty() walks ancestors

## Scope Boundaries
- INCLUDE: Fiber/partial redraws, GPU buffer persistence, incremental scene updates, damage-region rendering, virtual scroll tuning, element caching, batch optimization
- EXCLUDE: 3D rendering FPS bug (separate piece), new widget/component development, platform backend changes (macOS/Linux/Windows layer)

## Test Strategy Decision
- **Infrastructure exists**: Need to check (Zed had tests, fork may have stripped them)
- **Automated tests**: TBD — rendering performance is hard to unit test, likely QA-heavy
- **Agent-Executed QA**: MANDATORY — FPS benchmarks, frame timing, GPU memory monitoring
