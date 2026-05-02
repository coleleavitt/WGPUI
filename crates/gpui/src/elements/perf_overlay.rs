use std::time::Duration;

use crate::perf;
use crate::{
    App, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div, rgba, white,
};

/// Returns a frame-time overlay element for displaying the most recent window metrics.
pub fn perf_overlay() -> PerfOverlay {
    PerfOverlay
}

/// A stateless overlay that renders the most recent frame timing metrics.
#[derive(IntoElement)]
pub struct PerfOverlay;

impl RenderOnce for PerfOverlay {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let lines = window
            .frame_metrics()
            .map(format_metrics)
            .unwrap_or_else(|| vec![SharedString::new_static("No metrics")]);

        div()
            .absolute()
            .top_2()
            .right_2()
            .p_2()
            .flex()
            .flex_col()
            .bg(rgba(0x000000AA))
            .text_color(white())
            .text_sm()
            .font_family("monospace")
            .children(lines)
    }
}

fn format_metrics(metrics: &perf::FrameMetrics) -> Vec<SharedString> {
    vec![
        format!(
            "Frame #{} | {:.1}ms total",
            metrics.frame_number,
            duration_ms(metrics.total_draw_duration)
        )
        .into(),
        format!(
            "Invalidate: {:.1}ms | Prepaint: {:.1}ms",
            duration_ms(metrics.invalidate_entities_duration),
            duration_ms(metrics.prepaint_duration)
        )
        .into(),
        format!(
            "Paint: {:.1}ms | Finish: {:.1}ms",
            duration_ms(metrics.paint_duration),
            duration_ms(metrics.scene_finish_duration)
        )
        .into(),
        format!(
            "GPU: {}KB ({} uploads) | {} draws",
            metrics.gpu_upload_bytes / 1024,
            metrics.gpu_upload_count,
            metrics.draw_call_count
        )
        .into(),
        format!(
            "Views: {}/{} dirty",
            metrics.dirty_view_count, metrics.total_view_count
        )
        .into(),
    ]
}

fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}
