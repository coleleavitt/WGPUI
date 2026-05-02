## Decisions Log
- DP-1: Retrofit gpui_wgpu (not port Pulsar) — accepted recommendation
- DP-2: Public API stable, internal APIs can change — accepted recommendation
- DP-3: Synthetic benchmarks — accepted recommendation
- DP-4: Linux/Vulkan first — accepted recommendation

## 2026-05-02 Phase 1 Architecture Decision (Oracle consultation)
**Decision**: Use CHUNKED SCENE approach, NOT true persistent scene.

**Rationale** (from Oracle):
- replay() is the real bottleneck, not clear() — cached views clone every Primitive
- Full sort in finish() is O(n log n) on ALL primitives every frame
- True persistent scene has fragmentation/compaction problems
- Double-buffer swap is load-bearing (rendered_frame read by GPU renderer)

**Architecture**:
- SceneChunk per view: stores Range<usize> into each primitive vec + dirty flag
- Clean chunks: zero work (no copy, no sort, ranges preserved)
- Dirty chunks: append new primitives, sort only dirty chunk
- finish() becomes k-way merge of pre-sorted chunks (O(n) merge)
- Keep double-buffer swap, but next_frame.clear() only clears dirty chunks
- Reserve DrawOrder ranges per view to avoid interleaving issues
- paint_operations can be eliminated once chunks replace replay log

**Key data structure**:
```rust
struct SceneChunk {
    view_id: EntityId,
    shadows: Range<usize>,
    quads: Range<usize>,
    paths: Range<usize>,
    underlines: Range<usize>,
    monochrome_sprites: Range<usize>,
    subpixel_sprites: Range<usize>,
    polychrome_sprites: Range<usize>,
    surfaces: Range<usize>,
    order_range: Range<DrawOrder>,
    dirty: bool,
}
```
