# Baseline Benchmark Results

## Status: RECORDED — 2026-05-02

## Environment
- **OS**: Linux (Wayland session)
- **Build**: `cargo run --release -p gpui --example bench_render`
- **Profile**: release (optimized)
- **GPU**: wgpu/Vulkan backend

## Grid Scenario (1000 divs, 50×20 grid)

| Frame | Total Frame Time (ms) |
|---|---|
| 60 (warmup) | 93.0 |
| 120 | 82.3 |
| 180 | 84.4 |
| 240 | 82.6 |
| 300 | 83.0 |

**Steady-state average**: ~83ms/frame (12 FPS)

### Analysis
- 83ms for 1000 simple colored divs is high — indicates significant overhead
- This is the FULL pipeline: layout + paint + scene sort + GPU upload + render + present
- The optimizations in Phases 1-6 target each of these stages
- Expected improvement areas:
  - Phase 1 (incremental sort): ~5-10ms savings when scene is partially dirty
  - Phase 2 (scissor): fill rate savings for localized changes
  - Phase 3 (persistent buffers): GPU upload savings for static frames
  - Phase 4 (subtree skip): layout/paint savings for partial updates
  - Phase 6 (batch merging): draw call reduction

## Debug Mode Reference

| Frame | Total Frame Time (ms) |
|---|---|
| 60 | 177.1 |

Debug mode is ~2x slower than release, as expected.

## How to Record Additional Scenarios

```bash
cargo run -p gpui --example bench_render --release
```

Switch scenarios with keyboard:
- `1` = Grid (1000 divs)
- `2` = List (10,000 items uniform_list)
- `3` = Animated (200 color-cycling divs)

Frame timing is printed to stdout every 60 frames.

## Notes

- Recorded on Linux/Wayland with wgpu/Vulkan
- Release profile (optimized)
- Warmup frame (frame 60) excluded from steady-state average
- List and Animated scenarios require keyboard interaction to switch
