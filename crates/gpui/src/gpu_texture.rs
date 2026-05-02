/// Universal GPU texture handle for external texture embedding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuTextureHandle {
    /// Platform-native handle to the shared GPU texture memory.
    pub native_handle: isize,
    /// Width of the texture in pixels.
    pub width: u32,
    /// Height of the texture in pixels.
    pub height: u32,
    /// Texture format.
    pub format: GpuTextureFormat,
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
    /// Create a new GPU texture handle with RGBA8 format.
    pub fn new(native_handle: isize, width: u32, height: u32) -> Self {
        Self {
            native_handle,
            width,
            height,
            format: GpuTextureFormat::RGBA8,
        }
    }

    /// Create a new GPU texture handle with a specific format.
    pub fn new_with_format(
        native_handle: isize,
        width: u32,
        height: u32,
        format: GpuTextureFormat,
    ) -> Self {
        Self {
            native_handle,
            width,
            height,
            format,
        }
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
