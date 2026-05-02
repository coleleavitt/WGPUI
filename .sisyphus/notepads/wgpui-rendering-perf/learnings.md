## 2026-05-02 Session Start
- Plan: wgpui-rendering-perf
- Starting Phase 0: Instrumentation & Baselines
- Codebase stripped to 23 crates, cargo check passes
- Pulsar cross-platform renderer vendored as reference

## 2026-05-02 Architecture Deep Dive (from explore agents)

### Scene (scene.rs, 922 lines)
- Scene struct: 11 fields — paint_operations Vec, primitive_bounds BoundsTree, layer_stack, + 8 primitive vecs
- 8 primitive types (not 7): Shadow, Quad, Path, Underline, MonochromeSprite, SubpixelSprite, PolychromeSprite, PaintSurface
- DrawOrder = u32, stored in every primitive
- clear() wipes ALL 11 fields (lines 43-55)
- finish() sorts each primitive vec by order, sprites also by tile_id (lines 137-149)
- replay(range, prev_scene) copies PaintOperations from prev frame (lines 127-135)
- PaintOperation enum: Primitive(Primitive), StartLayer(Bounds), EndLayer (lines 200-204)
- BatchIterator peeks all 8 types, yields PrimitiveBatch by (order, kind) (lines 255-451)
- NO dirty/change tracking in Scene itself

### Window Draw Pipeline (window.rs, 5919 lines)
- draw() at line 2484: invalidate_entities → draw_roots → next_frame.finish → swap → needs_present
- draw_roots() at line 2596: Prepaint phase (2597-2645) then Paint phase (2648-2665)
- present() at line 2582: platform_window.draw(&rendered_frame.scene) → needs_present.set(false)
- invalidate_entities() at line 2573: takes dirty views from invalidator, calls mark_view_dirty for each
- Frame::finish() at line 936: transfers element states, calls scene.finish()
- swap at line 2518: mem::swap(&mut rendered_frame, &mut next_frame)
- next_frame.clear() at line 2519: clears for reuse
- mark_view_dirty() at line 1679: walks ancestors via dispatch_tree.view_path_reversed, inserts into dirty_views, stops if already dirty
- on_request_frame at line 1338: thermal throttling, frame rate limiting, draw-or-present decision
- Frame struct at line 791: scene, element_states, mouse_listeners, dispatch_tree, hitboxes, deferred_draws, etc.
- dirty_views: FxHashSet<EntityId> at line 113 of Window struct
- dirty_views.clear() at line 2502 after draw_roots

### WgpuRenderer (wgpu_renderer.rs, 1956 lines)
- WgpuRenderer struct lines 135-162: context, resources, surface_config, atlas, instance_buffer_capacity, etc.
- WgpuResources struct lines 109-124: device, queue, surface, pipelines, globals_buffer, instance_buffer, path textures
- draw() at line 1073: surface check → ensure_intermediate_textures → create frame view → write globals → batch loop → submit → present
- globals_buffer: 3 structs at aligned offsets (GlobalParams, PathGlobalParams, GammaParams) — 3x queue.write_buffer at lines 1174-1188
- instance_buffer: single STORAGE buffer, initial 2MB, grows by 2x on overflow (line 1702)
- write_to_instance_buffer() at line 1711: aligned offset writes, returns None on overflow
- Per-batch: write_to_instance_buffer → create bind group → set_pipeline → set_bind_groups → pass.draw(0..4, 0..count)
- Main render pass: LoadOp::Clear(TRANSPARENT), StoreOp::Store (lines 1203-1216)
- Path rasterization: separate pass to intermediate texture, then composite back with LoadOp::Load
- Bind groups created per-batch (not cached)
- frame.present() at line 1321 after queue.submit

### View Caching (view.rs, 320 lines)
- AnyView struct line 30: entity, render fn, cached_style: Option<Rc<StyleRefinement>>
- AnyViewState line 14: prepaint_range, paint_range, cache_key (ViewCacheKey), accessed_entities
- ViewCacheKey line 22: bounds, content_mask, text_style
- Cache decision in prepaint() line 155-160: cache_key matches AND !dirty_views.contains AND !refreshing
- reuse_prepaint() at window.rs:2855: extends hitboxes, tooltips, element_states, text layouts, dispatch tree, deferred draws
- reuse_paint() at window.rs:2918: extends cursor_styles, input_handlers, mouse_listeners, tab_stops, text layouts
- Scene replay happens via paint_operations range tracking, NOT explicit in reuse_paint
- Element trait (element.rs:51): request_layout → prepaint → paint, three-phase with state threading

## 2026-05-02 perf crate frame metrics
- Added lightweight std-only frame timing types to `crates/perf`: `FrameMetrics`, `PrimitiveCounts`, `PhaseTimer`, and `FrameMetricsCollector`.
- Preserved the existing `Importance` enum and `consts` module for `util_macros`' glob import.
- Collector phase slots currently map as: 0 invalidate entities, 1 prepaint, 2 paint, 3 scene finish, 4 present, 5 total draw.
