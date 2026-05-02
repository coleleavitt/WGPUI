use std::fmt;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Importance {
    Critical,
    Important,
    #[default]
    Average,
    Iffy,
    Fluff,
}

impl fmt::Display for Importance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Importance::Critical => write!(f, "Critical"),
            Importance::Important => write!(f, "Important"),
            Importance::Average => write!(f, "Average"),
            Importance::Iffy => write!(f, "Iffy"),
            Importance::Fluff => write!(f, "Fluff"),
        }
    }
}

pub mod consts {
    pub const SUF_NORMAL: &str = ".perf";
    pub const SUF_MDATA: &str = ".perfm";
    pub const ITER_ENV_VAR: &str = "PERF_ITERATIONS";
    pub const MDATA_LINE_PREF: &str = "PERF_MDATA";
    pub const ITER_COUNT_LINE_NAME: &str = "iterations";
    pub const WEIGHT_LINE_NAME: &str = "weight";
    pub const IMPORTANCE_LINE_NAME: &str = "importance";
    pub const VERSION_LINE_NAME: &str = "version";
    pub const MDATA_VER: u32 = 1;
    pub const WEIGHT_DEFAULT: u32 = 1;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PrimitiveCounts {
    pub shadows: u32,
    pub quads: u32,
    pub paths: u32,
    pub underlines: u32,
    pub monochrome_sprites: u32,
    pub subpixel_sprites: u32,
    pub polychrome_sprites: u32,
    pub surfaces: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameMetrics {
    pub frame_number: u64,
    pub total_draw_duration: Duration,
    pub invalidate_entities_duration: Duration,
    pub prepaint_duration: Duration,
    pub paint_duration: Duration,
    pub scene_finish_duration: Duration,
    pub present_duration: Duration,
    pub gpu_upload_bytes: usize,
    pub gpu_upload_count: u32,
    pub draw_call_count: u32,
    pub dirty_view_count: u32,
    pub total_view_count: u32,
    pub primitive_count: PrimitiveCounts,
}

impl fmt::Display for FrameMetrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "frame {}: total draw {:?} (invalidate {:?}, prepaint {:?}, paint {:?}, scene finish {:?}, present {:?}); GPU uploads: {} bytes across {}; draw calls: {}; views: {}/{} dirty; primitives: shadows {}, quads {}, paths {}, underlines {}, monochrome sprites {}, subpixel sprites {}, polychrome sprites {}, surfaces {}",
            self.frame_number,
            self.total_draw_duration,
            self.invalidate_entities_duration,
            self.prepaint_duration,
            self.paint_duration,
            self.scene_finish_duration,
            self.present_duration,
            self.gpu_upload_bytes,
            self.gpu_upload_count,
            self.draw_call_count,
            self.dirty_view_count,
            self.total_view_count,
            self.primitive_count.shadows,
            self.primitive_count.quads,
            self.primitive_count.paths,
            self.primitive_count.underlines,
            self.primitive_count.monochrome_sprites,
            self.primitive_count.subpixel_sprites,
            self.primitive_count.polychrome_sprites,
            self.primitive_count.surfaces,
        )
    }
}

pub struct PhaseTimer {
    start: Instant,
}

impl PhaseTimer {
    pub fn start() -> Self {
        Self {
            start: Instant::now(),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }
}

pub struct FrameMetricsCollector {
    frame_number: u64,
    phase_timers: [Option<Duration>; 6],
    gpu_upload_bytes: usize,
    gpu_upload_count: u32,
    draw_call_count: u32,
    dirty_view_count: u32,
    total_view_count: u32,
    primitive_count: PrimitiveCounts,
}

impl FrameMetricsCollector {
    pub fn new(frame_number: u64) -> Self {
        Self {
            frame_number,
            phase_timers: [None; 6],
            gpu_upload_bytes: 0,
            gpu_upload_count: 0,
            draw_call_count: 0,
            dirty_view_count: 0,
            total_view_count: 0,
            primitive_count: PrimitiveCounts::default(),
        }
    }

    pub fn record_phase(&mut self, phase_index: usize, duration: Duration) {
        if let Some(phase_timer) = self.phase_timers.get_mut(phase_index) {
            *phase_timer = Some(duration);
        }
    }

    pub fn record_gpu_upload(&mut self, bytes: usize) {
        self.gpu_upload_bytes = self.gpu_upload_bytes.saturating_add(bytes);
        self.gpu_upload_count = self.gpu_upload_count.saturating_add(1);
    }

    pub fn record_draw_call(&mut self) {
        self.draw_call_count = self.draw_call_count.saturating_add(1);
    }

    pub fn set_view_counts(&mut self, dirty: u32, total: u32) {
        self.dirty_view_count = dirty;
        self.total_view_count = total;
    }

    pub fn set_primitive_counts(&mut self, counts: PrimitiveCounts) {
        self.primitive_count = counts;
    }

    pub fn finish(self) -> FrameMetrics {
        let [
            invalidate_entities_duration,
            prepaint_duration,
            paint_duration,
            scene_finish_duration,
            present_duration,
            total_draw_duration,
        ] = self.phase_timers;

        FrameMetrics {
            frame_number: self.frame_number,
            total_draw_duration: total_draw_duration.unwrap_or_default(),
            invalidate_entities_duration: invalidate_entities_duration.unwrap_or_default(),
            prepaint_duration: prepaint_duration.unwrap_or_default(),
            paint_duration: paint_duration.unwrap_or_default(),
            scene_finish_duration: scene_finish_duration.unwrap_or_default(),
            present_duration: present_duration.unwrap_or_default(),
            gpu_upload_bytes: self.gpu_upload_bytes,
            gpu_upload_count: self.gpu_upload_count,
            draw_call_count: self.draw_call_count,
            dirty_view_count: self.dirty_view_count,
            total_view_count: self.total_view_count,
            primitive_count: self.primitive_count,
        }
    }
}
