use gpui::{
    GpuCanvasSource, GpuTextureBackend, GpuTextureFormat, GpuTextureHandle, GpuTextureResource,
    ObjectFit, gpu_canvas, surface,
};
use std::any::Any;
use std::sync::Arc;

struct TestTextureResource;

impl GpuTextureResource for TestTextureResource {
    fn backend(&self) -> GpuTextureBackend {
        GpuTextureBackend::Wgpu
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn main() {
    let front_buffer = GpuTextureHandle::from_resource(
        Arc::new(TestTextureResource),
        640,
        480,
        GpuTextureFormat::BGRA8,
    );
    let back_buffer = GpuTextureHandle::from_resource(
        Arc::new(TestTextureResource),
        640,
        480,
        GpuTextureFormat::RGBA8,
    );
    let source = GpuCanvasSource::new(front_buffer.clone(), back_buffer);

    source.swap_buffers();
    let active = source.active_buffer();
    assert_eq!(active.width, 640);
    assert_eq!(active.height, 480);
    assert_eq!(active.size_in_bytes(), 640 * 480 * 4);

    let _canvas = gpu_canvas(source).object_fit(ObjectFit::Contain);
    let _surface = surface(front_buffer);
}
