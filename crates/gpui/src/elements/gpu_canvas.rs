use crate::{
    App, Bounds, Element, ElementId, GlobalElementId, GpuTextureHandle, InspectorElementId,
    IntoElement, LayoutId, ObjectFit, Pixels, Style, StyleRefinement, Styled, Window,
};
use refineable::Refineable;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Double-buffered GPU texture source for external GPU content.
#[derive(Clone)]
pub struct GpuCanvasSource {
    active_buffer: Arc<AtomicUsize>,
    buffers: [GpuTextureHandle; 2],
}

impl GpuCanvasSource {
    /// Create a new double-buffered GPU canvas source.
    pub fn new(buffer0: GpuTextureHandle, buffer1: GpuTextureHandle) -> Self {
        Self {
            active_buffer: Arc::new(AtomicUsize::new(0)),
            buffers: [buffer0, buffer1],
        }
    }

    /// Get the currently active buffer for reading.
    pub fn active_buffer(&self) -> &GpuTextureHandle {
        let index = self.active_buffer.load(Ordering::Acquire) % self.buffers.len();
        &self.buffers[index]
    }

    /// Swap to the other buffer.
    pub fn swap_buffers(&self) {
        self.active_buffer.fetch_xor(1, Ordering::Release);
    }

    /// Set the active buffer index directly.
    pub fn set_active_buffer(&self, index: usize) {
        self.active_buffer
            .store(index % self.buffers.len(), Ordering::Release);
    }
}

/// A GPU canvas element for external GPU content.
pub struct GpuCanvas {
    source: GpuCanvasSource,
    object_fit: ObjectFit,
    style: StyleRefinement,
}

/// Create a new GPU canvas element with the given texture source.
pub fn gpu_canvas(source: GpuCanvasSource) -> GpuCanvas {
    GpuCanvas {
        source,
        object_fit: ObjectFit::Contain,
        style: Default::default(),
    }
}

impl GpuCanvas {
    /// Set how the GPU texture should fit within the element bounds.
    pub fn object_fit(mut self, object_fit: ObjectFit) -> Self {
        self.object_fit = object_fit;
        self
    }
}

impl Element for GpuCanvas {
    type RequestLayoutState = ();
    type PrepaintState = GpuTextureHandle;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.refine(&self.style);
        let layout_id = window.request_layout(style, [], cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        self.source.active_buffer().clone()
    }

    fn paint(
        &mut self,
        _global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        let size = crate::size(prepaint.width.into(), prepaint.height.into());
        let bounds = self.object_fit.get_bounds(bounds, size);
        window.paint_gpu_texture(bounds, prepaint.clone());
    }
}

impl IntoElement for GpuCanvas {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for GpuCanvas {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
