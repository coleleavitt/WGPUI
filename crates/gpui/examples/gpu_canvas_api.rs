use gpui::{GpuCanvasSource, GpuTextureFormat, GpuTextureHandle, ObjectFit, gpu_canvas, surface};

fn main() {
    let front_buffer = GpuTextureHandle::new_with_format(1, 640, 480, GpuTextureFormat::BGRA8);
    let back_buffer = GpuTextureHandle::new(2, 640, 480);
    let source = GpuCanvasSource::new(front_buffer.clone(), back_buffer);

    source.swap_buffers();
    let active = source.active_buffer();
    assert_eq!(active.width, 640);
    assert_eq!(active.height, 480);
    assert_eq!(active.size_in_bytes(), 640 * 480 * 4);

    let _canvas = gpu_canvas(source).object_fit(ObjectFit::Contain);
    let _surface = surface(front_buffer);
}
