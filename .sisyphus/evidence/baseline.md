# Baseline Benchmark Results

## Status: PENDING — Requires display to run

## How to Record Baselines

Run the benchmark example on a machine with a display:

```bash
cargo run -p gpui --example bench_render
```

Switch scenarios with keyboard:
- `1` = Grid (1000 divs)
- `2` = List (10,000 items uniform_list)
- `3` = Animated (200 color-cycling divs)

Frame timing is printed to stdout every 60 frames.

## Expected Metrics to Record

| Metric | Grid | List | Animated |
|---|---|---|---|
| Frame time p50 (ms) | | | |
| Frame time p95 (ms) | | | |
| Frame time p99 (ms) | | | |
| Invalidate duration (ms) | | | |
| Prepaint duration (ms) | | | |
| Paint duration (ms) | | | |
| Scene finish duration (ms) | | | |
| Present duration (ms) | | | |
| GPU upload bytes/frame | | | |
| GPU upload count/frame | | | |
| Draw call count/frame | | | |
| Dirty views/frame | | | |
| Total views | | | |
| Batch count | | | |

## Notes

- Record on target hardware (Linux/Vulkan)
- Run each scenario for at least 60 seconds
- Record after warmup (skip first 5 seconds)
- Use `--features perf-overlay` for visual timing overlay
