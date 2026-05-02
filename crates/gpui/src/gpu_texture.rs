use std::any::Any;
use std::fmt;
use std::sync::Arc;

/// Backend that owns an external GPU texture resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuTextureBackend {
    /// A `wgpu` texture view.
    Wgpu,
    /// A Metal texture.
    Metal,
    /// A Direct3D texture.
    Direct3D,
    /// A Vulkan image.
    Vulkan,
}

/// Type-erased backend texture resource.
pub trait GpuTextureResource: Any + Send + Sync {
    /// Backend kind for the resource.
    fn backend(&self) -> GpuTextureBackend;

    /// Downcast hook for renderer-specific resource imports.
    fn as_any(&self) -> &dyn Any;
}

/// Universal GPU texture handle for external texture embedding.
#[derive(Clone)]
pub struct GpuTextureHandle {
    /// Backend-owned texture resource.
    pub resource: Arc<dyn GpuTextureResource>,
    /// Width of the texture in pixels.
    pub width: u32,
    /// Height of the texture in pixels.
    pub height: u32,
    /// Texture format.
    pub format: GpuTextureFormat,
}

impl fmt::Debug for GpuTextureHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GpuTextureHandle")
            .field("backend", &self.resource.backend())
            .field("width", &self.width)
            .field("height", &self.height)
            .field("format", &self.format)
            .finish_non_exhaustive()
    }
}

/// GPU texture format for external texture embedding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuTextureFormat {
    /// 8-bit RGBA.
    RGBA8,
    /// 8-bit BGRA.
    BGRA8,
    /// 16-bit float RGBA.
    RGBA16F,
}

impl GpuTextureHandle {
    /// Create a GPU texture handle from a backend resource.
    pub fn from_resource(
        resource: Arc<dyn GpuTextureResource>,
        width: u32,
        height: u32,
        format: GpuTextureFormat,
    ) -> Self {
        Self {
            resource,
            width,
            height,
            format,
        }
    }

    /// Return the backend that owns this texture resource.
    pub fn backend(&self) -> GpuTextureBackend {
        self.resource.backend()
    }

    /// Get the size in bytes of a single pixel for this format.
    pub fn bytes_per_pixel(&self) -> u32 {
        match self.format {
            GpuTextureFormat::RGBA8 | GpuTextureFormat::BGRA8 => 4,
            GpuTextureFormat::RGBA16F => 8,
        }
    }

    /// Get the total size in bytes of the texture.
    pub fn size_in_bytes(&self) -> usize {
        self.width as usize * self.height as usize * self.bytes_per_pixel() as usize
    }
}
