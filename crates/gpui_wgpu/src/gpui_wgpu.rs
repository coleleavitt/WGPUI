mod cosmic_text_system;
mod wgpu_atlas;
mod wgpu_context;
mod wgpu_renderer;

use gpui::{GpuTextureBackend, GpuTextureFormat, GpuTextureHandle, GpuTextureResource};
use std::any::Any;
use std::sync::Arc;

pub use cosmic_text_system::*;
pub use wgpu;
pub use wgpu_atlas::*;
pub use wgpu_context::*;
pub use wgpu_renderer::{GpuContext, WgpuRenderer, WgpuSurfaceConfig};

/// A `wgpu` texture view that can be painted by GPUI's WGPU surface renderer.
pub struct WgpuTextureResource {
    view: Arc<wgpu::TextureView>,
}

impl WgpuTextureResource {
    /// Wrap a `wgpu` texture view in a GPUI external texture handle.
    pub fn handle(
        view: Arc<wgpu::TextureView>,
        width: u32,
        height: u32,
        format: GpuTextureFormat,
    ) -> GpuTextureHandle {
        GpuTextureHandle::from_resource(Arc::new(Self { view }), width, height, format)
    }

    pub(crate) fn view(&self) -> &wgpu::TextureView {
        &self.view
    }
}

impl GpuTextureResource for WgpuTextureResource {
    fn backend(&self) -> GpuTextureBackend {
        GpuTextureBackend::Wgpu
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
