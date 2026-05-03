use crate::{CompositorGpuHint, WgpuAtlas, WgpuContext, WgpuTextureResource};
use bytemuck::{Pod, Zeroable};
use gpui::{
    AtlasTextureId, Background, Bounds, DevicePixels, GpuSpecs, MonochromeSprite, Path, Point,
    PolychromeSprite, PrimitiveBatch, Quad, ScaledPixels, Scene, Shadow, Size, SubpixelSprite,
    Underline, get_gamma_correction_ratios,
};
use log::warn;
#[cfg(not(target_family = "wasm"))]
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::cell::RefCell;
use std::mem::{size_of, size_of_val};
use std::num::NonZeroU64;
use std::ops::Range;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlobalParams {
    viewport_size: [f32; 2],
    premultiplied_alpha: u32,
    pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PodBounds {
    origin: [f32; 2],
    size: [f32; 2],
}

impl From<Bounds<ScaledPixels>> for PodBounds {
    fn from(bounds: Bounds<ScaledPixels>) -> Self {
        Self {
            origin: [bounds.origin.x.0, bounds.origin.y.0],
            size: [bounds.size.width.0, bounds.size.height.0],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SurfaceParams {
    bounds: PodBounds,
    content_mask: PodBounds,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GammaParams {
    gamma_ratios: [f32; 4],
    grayscale_enhanced_contrast: f32,
    subpixel_enhanced_contrast: f32,
    is_bgr: u32,
    _pad: u32,
}

#[derive(Clone, Copy)]
struct ScissorRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Clone, Debug)]
#[repr(C)]
struct PathSprite {
    bounds: Bounds<ScaledPixels>,
}

#[derive(Clone, Debug)]
#[repr(C)]
struct PathRasterizationVertex {
    xy_position: Point<ScaledPixels>,
    st_position: Point<f32>,
    color: Background,
    bounds: Bounds<ScaledPixels>,
}

pub struct WgpuSurfaceConfig {
    pub size: Size<DevicePixels>,
    pub transparent: bool,
    /// Preferred presentation mode. When `Some`, the renderer will use this
    /// mode if supported by the surface, falling back to `Fifo`.
    /// When `None`, defaults to `Fifo` (VSync).
    ///
    /// Mobile platforms may prefer `Mailbox` (triple-buffering) to avoid
    /// blocking in `get_current_texture()` during lifecycle transitions.
    pub preferred_present_mode: Option<wgpu::PresentMode>,
}

#[derive(Clone, Debug, Default)]
pub struct GpuFrameStats {
    pub upload_bytes: usize,
    pub upload_count: u32,
    pub draw_call_count: u32,
    pub primitive_counts: gpui::perf::PrimitiveCounts,
}

struct WgpuPipelines {
    quads: wgpu::RenderPipeline,
    shadows: wgpu::RenderPipeline,
    path_rasterization: wgpu::RenderPipeline,
    paths: wgpu::RenderPipeline,
    underlines: wgpu::RenderPipeline,
    mono_sprites: wgpu::RenderPipeline,
    subpixel_sprites: Option<wgpu::RenderPipeline>,
    poly_sprites: wgpu::RenderPipeline,
    #[allow(dead_code)]
    surfaces: wgpu::RenderPipeline,
}

struct WgpuBindGroupLayouts {
    globals: wgpu::BindGroupLayout,
    instances: wgpu::BindGroupLayout,
    instances_with_texture: wgpu::BindGroupLayout,
    surfaces: wgpu::BindGroupLayout,
}

/// Shared GPU context reference, used to coordinate device recovery across multiple windows.
pub type GpuContext = Rc<RefCell<Option<WgpuContext>>>;

/// GPU resources that must be dropped together during device recovery.
struct WgpuResources {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    surface: wgpu::Surface<'static>,
    pipelines: WgpuPipelines,
    bind_group_layouts: WgpuBindGroupLayouts,
    atlas_sampler: wgpu::Sampler,
    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    path_globals_bind_group: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    path_intermediate_texture: Option<wgpu::Texture>,
    path_intermediate_view: Option<wgpu::TextureView>,
    path_msaa_texture: Option<wgpu::Texture>,
    path_msaa_view: Option<wgpu::TextureView>,
    frame_texture: Option<wgpu::Texture>,
    frame_view: Option<wgpu::TextureView>,
}

struct PersistentTypeBuffer {
    buffer: wgpu::Buffer,
    usage: wgpu::BufferUsages,
    capacity: usize,
    len: usize,
    fragmentation_bytes: usize,
}

impl PersistentTypeBuffer {
    fn new(device: &wgpu::Device, label: &str, initial_capacity_bytes: usize) -> Self {
        Self::new_with_usage(
            device,
            label,
            initial_capacity_bytes,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        )
    }

    fn new_with_usage(
        device: &wgpu::Device,
        label: &str,
        initial_capacity_bytes: usize,
        usage: wgpu::BufferUsages,
    ) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: initial_capacity_bytes as u64,
            usage,
            mapped_at_creation: false,
        });

        Self {
            buffer,
            usage,
            capacity: initial_capacity_bytes,
            len: 0,
            fragmentation_bytes: 0,
        }
    }

    fn fragmentation_ratio(&self) -> f32 {
        if self.capacity == 0 {
            return 0.0;
        }

        self.fragmentation_bytes as f32 / self.capacity as f32
    }

    fn needs_compaction(&self) -> bool {
        self.fragmentation_ratio() > 0.3
    }

    fn compact(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[u8], label: &str) {
        let new_capacity = data.len().saturating_mul(2).max(4096);
        self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: new_capacity as u64,
            usage: self.usage,
            mapped_at_creation: false,
        });
        self.capacity = new_capacity;

        if !data.is_empty() {
            queue.write_buffer(&self.buffer, 0, data);
        }
        self.len = data.len();
        self.fragmentation_bytes = 0;
    }

    fn upload_full(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        data: &[u8],
        label: &str,
    ) -> bool {
        let mut reallocated = false;
        let old_len = self.len;
        if data.len() > self.capacity {
            let new_capacity = data.len().saturating_mul(2).max(4096);
            self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: new_capacity as u64,
                usage: self.usage,
                mapped_at_creation: false,
            });
            self.capacity = new_capacity;
            self.fragmentation_bytes = 0;
            reallocated = true;
        }

        if !data.is_empty() {
            queue.write_buffer(&self.buffer, 0, data);
        }
        if data.len() < old_len {
            self.fragmentation_bytes = self
                .fragmentation_bytes
                .saturating_add(old_len - data.len());
        }
        self.len = data.len();
        reallocated
    }

    fn upload_ranges(
        &self,
        queue: &wgpu::Queue,
        full_data: &[u8],
        ranges: &[Range<usize>],
        element_size: usize,
    ) -> usize {
        let mut bytes_uploaded = 0usize;
        for range in ranges {
            let byte_start = range.start.saturating_mul(element_size);
            let byte_end = range.end.saturating_mul(element_size).min(full_data.len());
            if byte_start < byte_end && byte_end <= full_data.len() && byte_end <= self.capacity {
                queue.write_buffer(
                    &self.buffer,
                    byte_start as u64,
                    &full_data[byte_start..byte_end],
                );
                bytes_uploaded = bytes_uploaded.saturating_add(byte_end - byte_start);
            }
        }
        bytes_uploaded
    }

    fn binding(&self, offset: usize, size: usize) -> wgpu::BindingResource<'_> {
        wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer: &self.buffer,
            offset: offset as u64,
            size: NonZeroU64::new(size.max(16) as u64),
        })
    }

    fn full_binding(&self) -> wgpu::BindingResource<'_> {
        self.binding(0, self.len)
    }
}

struct PersistentBuffers {
    shadows: PersistentTypeBuffer,
    quads: PersistentTypeBuffer,
    underlines: PersistentTypeBuffer,
    monochrome_sprites: PersistentTypeBuffer,
    subpixel_sprites: PersistentTypeBuffer,
    polychrome_sprites: PersistentTypeBuffer,
    last_shadow_len: usize,
    last_quad_len: usize,
    last_underline_len: usize,
    last_mono_sprite_len: usize,
    last_subpixel_sprite_len: usize,
    last_poly_sprite_len: usize,
    last_generation: u64,
}

impl PersistentBuffers {
    fn new(device: &wgpu::Device) -> Self {
        let initial_capacity = 64 * 1024;
        Self {
            shadows: PersistentTypeBuffer::new(device, "shadows_persistent", initial_capacity),
            quads: PersistentTypeBuffer::new(device, "quads_persistent", initial_capacity),
            underlines: PersistentTypeBuffer::new(
                device,
                "underlines_persistent",
                initial_capacity,
            ),
            monochrome_sprites: PersistentTypeBuffer::new(
                device,
                "monochrome_sprites_persistent",
                initial_capacity,
            ),
            subpixel_sprites: PersistentTypeBuffer::new(
                device,
                "subpixel_sprites_persistent",
                initial_capacity,
            ),
            polychrome_sprites: PersistentTypeBuffer::new(
                device,
                "polychrome_sprites_persistent",
                initial_capacity,
            ),
            last_shadow_len: 0,
            last_quad_len: 0,
            last_underline_len: 0,
            last_mono_sprite_len: 0,
            last_subpixel_sprite_len: 0,
            last_poly_sprite_len: 0,
            last_generation: u64::MAX,
        }
    }
}

fn upload_persistent_instances<T>(
    buffer: &mut PersistentTypeBuffer,
    last_len: &mut usize,
    instances: &[T],
    changed_ranges: &[Range<usize>],
    generation_changed: bool,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    upload_bytes: &mut usize,
    upload_count: &mut u32,
) {
    // SAFETY: This mirrors WgpuRenderer::instance_bytes; these primitive structs are encoded
    // directly into storage buffers and decoded by matching WGSL structs.
    let data = unsafe {
        std::slice::from_raw_parts(instances.as_ptr() as *const u8, size_of_val(instances))
    };
    if instances.len() != *last_len || (generation_changed && changed_ranges.is_empty()) {
        buffer.upload_full(device, queue, data, label);
        *last_len = instances.len();
        if !data.is_empty() {
            *upload_bytes = upload_bytes.saturating_add(data.len());
            *upload_count = upload_count.saturating_add(1);
        }
    } else if !changed_ranges.is_empty() {
        let bytes = buffer.upload_ranges(queue, data, changed_ranges, size_of::<T>());
        *upload_bytes = upload_bytes.saturating_add(bytes);
        *upload_count = upload_count.saturating_add(changed_ranges.len() as u32);
    }
}

fn compact_persistent_instances<T>(
    buffer: &mut PersistentTypeBuffer,
    instances: &[T],
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    upload_bytes: &mut usize,
    upload_count: &mut u32,
) {
    if !buffer.needs_compaction() {
        return;
    }

    // SAFETY: This mirrors WgpuRenderer::instance_bytes; these primitive structs are encoded
    // directly into storage buffers and decoded by matching WGSL structs.
    let data = unsafe {
        std::slice::from_raw_parts(instances.as_ptr() as *const u8, size_of_val(instances))
    };
    buffer.compact(device, queue, data, label);
    if !data.is_empty() {
        *upload_bytes = upload_bytes.saturating_add(data.len());
        *upload_count = upload_count.saturating_add(1);
    }
}

impl WgpuResources {
    fn invalidate_intermediate_textures(&mut self) {
        self.path_intermediate_texture = None;
        self.path_intermediate_view = None;
        self.path_msaa_texture = None;
        self.path_msaa_view = None;
        self.frame_texture = None;
        self.frame_view = None;
    }
}

pub struct WgpuRenderer {
    /// Shared GPU context for device recovery coordination (unused on WASM).
    #[allow(dead_code)]
    context: Option<GpuContext>,
    /// Compositor GPU hint for adapter selection (unused on WASM).
    #[allow(dead_code)]
    compositor_gpu: Option<CompositorGpuHint>,
    resources: Option<WgpuResources>,
    surface_config: wgpu::SurfaceConfiguration,
    atlas: Arc<WgpuAtlas>,
    path_globals_offset: u64,
    gamma_offset: u64,
    instance_buffer_capacity: u64,
    max_buffer_size: u64,
    storage_buffer_alignment: u64,
    rendering_params: RenderingParameters,
    is_bgr: bool,
    dual_source_blending: bool,
    adapter_info: wgpu::AdapterInfo,
    transparent_alpha_mode: wgpu::CompositeAlphaMode,
    opaque_alpha_mode: wgpu::CompositeAlphaMode,
    max_texture_size: u32,
    last_error: Arc<Mutex<Option<String>>>,
    failed_frame_count: u32,
    device_lost: std::sync::Arc<std::sync::atomic::AtomicBool>,
    surface_configured: bool,
    needs_redraw: bool,
    has_drawn_frame: bool,
    surface_copy_supported: bool,
    /// Damage reported by the last draw call, in device (buffer) pixels.
    /// `None` means full redraw (entire surface is dirty).
    /// `Some(vec)` means only those rects changed.
    last_damage_rects: Option<Vec<[i32; 4]>>,
    last_frame_stats: Option<GpuFrameStats>,
    persistent_buffers: PersistentBuffers,
    supports_multi_draw_indirect: bool,
    indirect_buffer: Option<PersistentTypeBuffer>,
}

impl WgpuRenderer {
    fn resources(&self) -> &WgpuResources {
        self.resources
            .as_ref()
            .expect("GPU resources not available")
    }

    fn resources_mut(&mut self) -> &mut WgpuResources {
        self.resources
            .as_mut()
            .expect("GPU resources not available")
    }

    /// Creates a new WgpuRenderer from raw window handles.
    ///
    /// The `gpu_context` is a shared reference that coordinates GPU context across
    /// multiple windows. The first window to create a renderer will initialize the
    /// context; subsequent windows will share it.
    ///
    /// # Safety
    /// The caller must ensure that the window handle remains valid for the lifetime
    /// of the returned renderer.
    #[cfg(not(target_family = "wasm"))]
    pub fn new<W>(
        gpu_context: GpuContext,
        window: &W,
        config: WgpuSurfaceConfig,
        compositor_gpu: Option<CompositorGpuHint>,
    ) -> anyhow::Result<Self>
    where
        W: HasWindowHandle + HasDisplayHandle + std::fmt::Debug + Send + Sync + Clone + 'static,
    {
        let window_handle = window
            .window_handle()
            .map_err(|e| anyhow::anyhow!("Failed to get window handle: {e}"))?;

        let target = wgpu::SurfaceTargetUnsafe::RawHandle {
            // Fall back to the display handle already provided via InstanceDescriptor::display.
            raw_display_handle: None,
            raw_window_handle: window_handle.as_raw(),
        };

        // Use the existing context's instance if available, otherwise create a new one.
        // The surface must be created with the same instance that will be used for
        // adapter selection, otherwise wgpu will panic.
        let instance = gpu_context
            .borrow()
            .as_ref()
            .map(|ctx| ctx.instance.clone())
            .unwrap_or_else(|| WgpuContext::instance(Box::new(window.clone())));

        // Safety: The caller guarantees that the window handle is valid for the
        // lifetime of this renderer. In practice, the RawWindow struct is created
        // from the native window handles and the surface is dropped before the window.
        let surface = unsafe {
            instance
                .create_surface_unsafe(target)
                .map_err(|e| anyhow::anyhow!("Failed to create surface: {e}"))?
        };

        let mut ctx_ref = gpu_context.borrow_mut();
        let context = match ctx_ref.as_mut() {
            Some(context) => {
                context.check_compatible_with_surface(&surface)?;
                context
            }
            None => ctx_ref.insert(WgpuContext::new(instance, &surface, compositor_gpu)?),
        };

        let atlas = Arc::new(WgpuAtlas::from_context(context));

        Self::new_internal(
            Some(Rc::clone(&gpu_context)),
            context,
            surface,
            config,
            compositor_gpu,
            atlas,
        )
    }

    #[cfg(target_family = "wasm")]
    pub fn new_from_canvas(
        context: &WgpuContext,
        canvas: &web_sys::HtmlCanvasElement,
        config: WgpuSurfaceConfig,
    ) -> anyhow::Result<Self> {
        let surface = context
            .instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
            .map_err(|e| anyhow::anyhow!("Failed to create surface: {e}"))?;

        let atlas = Arc::new(WgpuAtlas::from_context(context));

        Self::new_internal(None, context, surface, config, None, atlas)
    }

    fn new_internal(
        gpu_context: Option<GpuContext>,
        context: &WgpuContext,
        surface: wgpu::Surface<'static>,
        config: WgpuSurfaceConfig,
        compositor_gpu: Option<CompositorGpuHint>,
        atlas: Arc<WgpuAtlas>,
    ) -> anyhow::Result<Self> {
        let surface_caps = surface.get_capabilities(&context.adapter);
        let preferred_formats = [
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureFormat::Rgba8Unorm,
        ];
        let surface_format = preferred_formats
            .iter()
            .find(|f| surface_caps.formats.contains(f))
            .copied()
            .or_else(|| surface_caps.formats.iter().find(|f| !f.is_srgb()).copied())
            .or_else(|| surface_caps.formats.first().copied())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Surface reports no supported texture formats for adapter {:?}",
                    context.adapter.get_info().name
                )
            })?;

        let pick_alpha_mode =
            |preferences: &[wgpu::CompositeAlphaMode]| -> anyhow::Result<wgpu::CompositeAlphaMode> {
                preferences
                    .iter()
                    .find(|p| surface_caps.alpha_modes.contains(p))
                    .copied()
                    .or_else(|| surface_caps.alpha_modes.first().copied())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "Surface reports no supported alpha modes for adapter {:?}",
                            context.adapter.get_info().name
                        )
                    })
            };

        let transparent_alpha_mode = pick_alpha_mode(&[
            wgpu::CompositeAlphaMode::PreMultiplied,
            wgpu::CompositeAlphaMode::Inherit,
        ])?;

        let opaque_alpha_mode = pick_alpha_mode(&[
            wgpu::CompositeAlphaMode::Opaque,
            wgpu::CompositeAlphaMode::Inherit,
        ])?;

        let alpha_mode = if config.transparent {
            transparent_alpha_mode
        } else {
            opaque_alpha_mode
        };

        let device = Arc::clone(&context.device);
        let max_texture_size = device.limits().max_texture_dimension_2d;

        let requested_width = config.size.width.0 as u32;
        let requested_height = config.size.height.0 as u32;
        let clamped_width = requested_width.min(max_texture_size);
        let clamped_height = requested_height.min(max_texture_size);

        if clamped_width != requested_width || clamped_height != requested_height {
            warn!(
                "Requested surface size ({}, {}) exceeds maximum texture dimension {}. \
                 Clamping to ({}, {}). Window content may not fill the entire window.",
                requested_width, requested_height, max_texture_size, clamped_width, clamped_height
            );
        }

        let surface_copy_supported = surface_caps.usages.contains(wgpu::TextureUsages::COPY_DST);
        let mut surface_usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
        if surface_copy_supported {
            surface_usage |= wgpu::TextureUsages::COPY_DST;
        }

        let surface_config = wgpu::SurfaceConfiguration {
            usage: surface_usage,
            format: surface_format,
            width: clamped_width.max(1),
            height: clamped_height.max(1),
            present_mode: config
                .preferred_present_mode
                .filter(|mode| surface_caps.present_modes.contains(mode))
                .unwrap_or(wgpu::PresentMode::Fifo),
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
        };
        // Configure the surface immediately. The adapter selection process already validated
        // that this adapter can successfully configure this surface.
        surface.configure(&context.device, &surface_config);

        let queue = Arc::clone(&context.queue);
        let dual_source_blending = context.supports_dual_source_blending();
        let features = device.features();
        let supports_multi_draw_count =
            features.contains(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT);
        let supports_multi_draw_indirect = supports_multi_draw_count;
        log::info!(
            "multi_draw_indirect available: {}",
            supports_multi_draw_indirect
        );

        let rendering_params = RenderingParameters::new(&context.adapter, surface_format);
        let bind_group_layouts = Self::create_bind_group_layouts(&device);
        let pipelines = Self::create_pipelines(
            &device,
            &bind_group_layouts,
            surface_format,
            alpha_mode,
            rendering_params.path_sample_count,
            dual_source_blending,
        );

        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let uniform_alignment = device.limits().min_uniform_buffer_offset_alignment as u64;
        let globals_size = std::mem::size_of::<GlobalParams>() as u64;
        let gamma_size = std::mem::size_of::<GammaParams>() as u64;
        let path_globals_offset = globals_size.next_multiple_of(uniform_alignment);
        let gamma_offset = (path_globals_offset + globals_size).next_multiple_of(uniform_alignment);

        let globals_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals_buffer"),
            size: gamma_offset + gamma_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let max_buffer_size = device.limits().max_buffer_size;
        let storage_buffer_alignment = device.limits().min_storage_buffer_offset_alignment as u64;
        let initial_instance_buffer_capacity = 2 * 1024 * 1024;
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instance_buffer"),
            size: initial_instance_buffer_capacity,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals_bind_group"),
            layout: &bind_group_layouts.globals,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &globals_buffer,
                        offset: 0,
                        size: Some(NonZeroU64::new(globals_size).unwrap()),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &globals_buffer,
                        offset: gamma_offset,
                        size: Some(NonZeroU64::new(gamma_size).unwrap()),
                    }),
                },
            ],
        });

        let path_globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("path_globals_bind_group"),
            layout: &bind_group_layouts.globals,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &globals_buffer,
                        offset: path_globals_offset,
                        size: Some(NonZeroU64::new(globals_size).unwrap()),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &globals_buffer,
                        offset: gamma_offset,
                        size: Some(NonZeroU64::new(gamma_size).unwrap()),
                    }),
                },
            ],
        });

        let adapter_info = context.adapter.get_info();

        let last_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let last_error_clone = Arc::clone(&last_error);
        device.on_uncaptured_error(Arc::new(move |error| {
            let mut guard = last_error_clone.lock().unwrap();
            *guard = Some(error.to_string());
        }));

        let resources = WgpuResources {
            device,
            queue,
            surface,
            pipelines,
            bind_group_layouts,
            atlas_sampler,
            globals_buffer,
            globals_bind_group,
            path_globals_bind_group,
            instance_buffer,
            // Defer intermediate texture creation to first draw call via ensure_intermediate_textures().
            // This avoids panics when the device/surface is in an invalid state during initialization.
            path_intermediate_texture: None,
            path_intermediate_view: None,
            path_msaa_texture: None,
            path_msaa_view: None,
            frame_texture: None,
            frame_view: None,
        };
        let persistent_buffers = PersistentBuffers::new(&context.device);
        let indirect_buffer = supports_multi_draw_indirect.then(|| {
            PersistentTypeBuffer::new_with_usage(
                &context.device,
                "draw_indirect_args",
                4 * 1024,
                wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
            )
        });

        Ok(Self {
            context: gpu_context,
            compositor_gpu,
            resources: Some(resources),
            surface_config,
            atlas,
            path_globals_offset,
            gamma_offset,
            instance_buffer_capacity: initial_instance_buffer_capacity,
            max_buffer_size,
            storage_buffer_alignment,
            rendering_params,
            is_bgr: false,
            dual_source_blending,
            adapter_info,
            transparent_alpha_mode,
            opaque_alpha_mode,
            max_texture_size,
            last_error,
            failed_frame_count: 0,
            device_lost: context.device_lost_flag(),
            surface_configured: true,
            needs_redraw: false,
            has_drawn_frame: false,
            surface_copy_supported,
            last_damage_rects: None,
            last_frame_stats: None,
            persistent_buffers,
            supports_multi_draw_indirect,
            indirect_buffer,
        })
    }

    fn create_bind_group_layouts(device: &wgpu::Device) -> WgpuBindGroupLayouts {
        let globals =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("globals_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: NonZeroU64::new(
                                std::mem::size_of::<GlobalParams>() as u64
                            ),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: NonZeroU64::new(
                                std::mem::size_of::<GammaParams>() as u64
                            ),
                        },
                        count: None,
                    },
                ],
            });

        let storage_buffer_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };

        let instances = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("instances_layout"),
            entries: &[storage_buffer_entry(0)],
        });

        let instances_with_texture =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("instances_with_texture_layout"),
                entries: &[
                    storage_buffer_entry(0),
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let surfaces = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("surfaces_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(
                            std::mem::size_of::<SurfaceParams>() as u64
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        WgpuBindGroupLayouts {
            globals,
            instances,
            instances_with_texture,
            surfaces,
        }
    }

    fn create_pipelines(
        device: &wgpu::Device,
        layouts: &WgpuBindGroupLayouts,
        surface_format: wgpu::TextureFormat,
        alpha_mode: wgpu::CompositeAlphaMode,
        path_sample_count: u32,
        dual_source_blending: bool,
    ) -> WgpuPipelines {
        // Diagnostic guard: verify the device actually has
        // DUAL_SOURCE_BLENDING. We have a crash report (ZED-5G1) where a
        // feature mismatch caused a wgpu-hal abort, but we haven't
        // identified the code path that produces the mismatch. This
        // guard prevents the crash and logs more evidence.
        // Remove this check once:
        // a) We find and fix the root cause, or
        // b) There are no reports of this warning appearing for some time.
        let device_has_feature = device
            .features()
            .contains(wgpu::Features::DUAL_SOURCE_BLENDING);
        if dual_source_blending && !device_has_feature {
            log::error!(
                "BUG: dual_source_blending flag is true but device does not \
                 have DUAL_SOURCE_BLENDING enabled (device features: {:?}). \
                 Falling back to mono text rendering. Please report this at \
                 https://github.com/zed-industries/zed/issues",
                device.features(),
            );
        }
        let dual_source_blending = dual_source_blending && device_has_feature;

        let base_shader_source = include_str!("shaders.wgsl");
        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpui_shaders"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(base_shader_source)),
        });

        let subpixel_shader_source = include_str!("shaders_subpixel.wgsl");
        let subpixel_shader_module = if dual_source_blending {
            let combined = format!(
                "enable dual_source_blending;\n{base_shader_source}\n{subpixel_shader_source}"
            );
            Some(device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("gpui_subpixel_shaders"),
                source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Owned(combined)),
            }))
        } else {
            None
        };

        let blend_mode = match alpha_mode {
            wgpu::CompositeAlphaMode::PreMultiplied => {
                wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING
            }
            _ => wgpu::BlendState::ALPHA_BLENDING,
        };

        let color_target = wgpu::ColorTargetState {
            format: surface_format,
            blend: Some(blend_mode),
            write_mask: wgpu::ColorWrites::ALL,
        };

        let create_pipeline = |name: &str,
                               vs_entry: &str,
                               fs_entry: &str,
                               globals_layout: &wgpu::BindGroupLayout,
                               data_layout: &wgpu::BindGroupLayout,
                               topology: wgpu::PrimitiveTopology,
                               color_targets: &[Option<wgpu::ColorTargetState>],
                               sample_count: u32,
                               module: &wgpu::ShaderModule| {
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(&format!("{name}_layout")),
                bind_group_layouts: &[Some(globals_layout), Some(data_layout)],
                immediate_size: 0,
            });

            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(name),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some(vs_entry),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some(fs_entry),
                    targets: color_targets,
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    unclipped_depth: false,
                    conservative: false,
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState {
                    count: sample_count,
                    mask: !0,
                    alpha_to_coverage_enabled: false,
                },
                multiview_mask: None,
                cache: None,
            })
        };

        let quads = create_pipeline(
            "quads",
            "vs_quad",
            "fs_quad",
            &layouts.globals,
            &layouts.instances,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target.clone())],
            1,
            &shader_module,
        );

        let shadows = create_pipeline(
            "shadows",
            "vs_shadow",
            "fs_shadow",
            &layouts.globals,
            &layouts.instances,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target.clone())],
            1,
            &shader_module,
        );

        let path_rasterization = create_pipeline(
            "path_rasterization",
            "vs_path_rasterization",
            "fs_path_rasterization",
            &layouts.globals,
            &layouts.instances,
            wgpu::PrimitiveTopology::TriangleList,
            &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            path_sample_count,
            &shader_module,
        );

        let paths_blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };

        let paths = create_pipeline(
            "paths",
            "vs_path",
            "fs_path",
            &layouts.globals,
            &layouts.instances_with_texture,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(paths_blend),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            1,
            &shader_module,
        );

        let underlines = create_pipeline(
            "underlines",
            "vs_underline",
            "fs_underline",
            &layouts.globals,
            &layouts.instances,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target.clone())],
            1,
            &shader_module,
        );

        let mono_sprites = create_pipeline(
            "mono_sprites",
            "vs_mono_sprite",
            "fs_mono_sprite",
            &layouts.globals,
            &layouts.instances_with_texture,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target.clone())],
            1,
            &shader_module,
        );

        let subpixel_sprites = if let Some(subpixel_module) = &subpixel_shader_module {
            let subpixel_blend = wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Src1,
                    dst_factor: wgpu::BlendFactor::OneMinusSrc1,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
            };

            Some(create_pipeline(
                "subpixel_sprites",
                "vs_subpixel_sprite",
                "fs_subpixel_sprite",
                &layouts.globals,
                &layouts.instances_with_texture,
                wgpu::PrimitiveTopology::TriangleStrip,
                &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(subpixel_blend),
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
                1,
                subpixel_module,
            ))
        } else {
            None
        };

        let poly_sprites = create_pipeline(
            "poly_sprites",
            "vs_poly_sprite",
            "fs_poly_sprite",
            &layouts.globals,
            &layouts.instances_with_texture,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target.clone())],
            1,
            &shader_module,
        );

        let surfaces = create_pipeline(
            "surfaces",
            "vs_surface",
            "fs_surface",
            &layouts.globals,
            &layouts.surfaces,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target)],
            1,
            &shader_module,
        );

        WgpuPipelines {
            quads,
            shadows,
            path_rasterization,
            paths,
            underlines,
            mono_sprites,
            subpixel_sprites,
            poly_sprites,
            surfaces,
        }
    }

    fn create_path_intermediate(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("path_intermediate"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    fn create_frame_texture(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("persistent_frame_texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    fn create_msaa_if_needed(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        sample_count: u32,
    ) -> Option<(wgpu::Texture, wgpu::TextureView)> {
        if sample_count <= 1 {
            return None;
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("path_msaa"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Some((texture, view))
    }

    pub fn update_drawable_size(&mut self, size: Size<DevicePixels>) {
        let width = size.width.0 as u32;
        let height = size.height.0 as u32;

        tracing::debug!(
            target: "wgpu::renderer",
            requested_w = width,
            requested_h = height,
            current_w = self.surface_config.width,
            current_h = self.surface_config.height,
            "update_drawable_size entry"
        );

        if width != self.surface_config.width || height != self.surface_config.height {
            let clamped_width = width.min(self.max_texture_size);
            let clamped_height = height.min(self.max_texture_size);

            if clamped_width != width || clamped_height != height {
                warn!(
                    "Requested surface size ({}, {}) exceeds maximum texture dimension {}. \
                     Clamping to ({}, {}). Window content may not fill the entire window.",
                    width, height, self.max_texture_size, clamped_width, clamped_height
                );
            }

            self.surface_config.width = clamped_width.max(1);
            self.surface_config.height = clamped_height.max(1);
            let surface_config = self.surface_config.clone();

            let resources = self.resources_mut();

            // Wait for any in-flight GPU work to complete before destroying textures
            if let Err(e) = resources.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            }) {
                warn!("Failed to poll device during resize: {e:?}");
            }

            // Destroy old textures before allocating new ones to avoid GPU memory spikes
            if let Some(ref texture) = resources.path_intermediate_texture {
                texture.destroy();
            }
            if let Some(ref texture) = resources.path_msaa_texture {
                texture.destroy();
            }
            if let Some(ref texture) = resources.frame_texture {
                texture.destroy();
            }

            resources
                .surface
                .configure(&resources.device, &surface_config);

            // Invalidate intermediate textures - they will be lazily recreated
            // in draw() after we confirm the surface is healthy. This avoids
            // panics when the device/surface is in an invalid state during resize.
            resources.invalidate_intermediate_textures();
            tracing::debug!(
                target: "wgpu::renderer",
                surface_w = self.surface_config.width,
                surface_h = self.surface_config.height,
                "surface reconfigured; forcing next frame to full redraw"
            );
            self.has_drawn_frame = false;
        }
    }

    fn ensure_intermediate_textures(&mut self) {
        if self.resources().path_intermediate_texture.is_some()
            && self.resources().frame_texture.is_some()
        {
            return;
        }

        let format = self.surface_config.format;
        let width = self.surface_config.width;
        let height = self.surface_config.height;
        let path_sample_count = self.rendering_params.path_sample_count;
        let resources = self.resources_mut();

        if resources.path_intermediate_texture.is_none() {
            let (t, v) = Self::create_path_intermediate(&resources.device, format, width, height);
            resources.path_intermediate_texture = Some(t);
            resources.path_intermediate_view = Some(v);
        }

        if resources.frame_texture.is_none() {
            let (t, v) = Self::create_frame_texture(&resources.device, format, width, height);
            resources.frame_texture = Some(t);
            resources.frame_view = Some(v);
        }

        if resources.path_msaa_texture.is_none() && path_sample_count > 1 {
            let (path_msaa_texture, path_msaa_view) = Self::create_msaa_if_needed(
                &resources.device,
                format,
                width,
                height,
                path_sample_count,
            )
            .map(|(t, v)| (Some(t), Some(v)))
            .unwrap_or((None, None));
            resources.path_msaa_texture = path_msaa_texture;
            resources.path_msaa_view = path_msaa_view;
        }
    }

    pub fn set_subpixel_layout(&mut self, is_bgr: bool) {
        self.is_bgr = is_bgr;
    }

    pub fn update_transparency(&mut self, transparent: bool) {
        let new_alpha_mode = if transparent {
            self.transparent_alpha_mode
        } else {
            self.opaque_alpha_mode
        };

        if new_alpha_mode != self.surface_config.alpha_mode {
            self.surface_config.alpha_mode = new_alpha_mode;
            let surface_config = self.surface_config.clone();
            let path_sample_count = self.rendering_params.path_sample_count;
            let dual_source_blending = self.dual_source_blending;
            let resources = self.resources_mut();
            resources
                .surface
                .configure(&resources.device, &surface_config);
            resources.pipelines = Self::create_pipelines(
                &resources.device,
                &resources.bind_group_layouts,
                surface_config.format,
                surface_config.alpha_mode,
                path_sample_count,
                dual_source_blending,
            );
            self.has_drawn_frame = false;
        }
    }

    #[allow(dead_code)]
    pub fn viewport_size(&self) -> Size<DevicePixels> {
        Size {
            width: DevicePixels(self.surface_config.width as i32),
            height: DevicePixels(self.surface_config.height as i32),
        }
    }

    pub fn sprite_atlas(&self) -> &Arc<WgpuAtlas> {
        &self.atlas
    }

    pub fn supports_dual_source_blending(&self) -> bool {
        self.dual_source_blending
    }

    pub fn gpu_specs(&self) -> GpuSpecs {
        GpuSpecs {
            is_software_emulated: self.adapter_info.device_type == wgpu::DeviceType::Cpu,
            device_name: self.adapter_info.name.clone(),
            driver_name: self.adapter_info.driver.clone(),
            driver_info: self.adapter_info.driver_info.clone(),
        }
    }

    pub fn max_texture_size(&self) -> u32 {
        self.max_texture_size
    }

    fn viewport_bounds(&self) -> Bounds<ScaledPixels> {
        Bounds::new(
            Point {
                x: ScaledPixels(0.0),
                y: ScaledPixels(0.0),
            },
            Size {
                width: ScaledPixels(self.surface_config.width as f32),
                height: ScaledPixels(self.surface_config.height as f32),
            },
        )
    }

    fn scissor_rect_for_damage(
        damage_rects: &[Bounds<ScaledPixels>],
        surface_width: u32,
        surface_height: u32,
    ) -> Option<ScissorRect> {
        let mut damage_rects = damage_rects.iter();
        let mut damage_rect = *damage_rects.next()?;

        for rect in damage_rects {
            damage_rect = damage_rect.union(rect);
        }

        let x = damage_rect.origin.x.0.max(0.0).min(surface_width as f32) as u32;
        let y = damage_rect.origin.y.0.max(0.0).min(surface_height as f32) as u32;
        let width = (damage_rect.size.width.0 as u32).min(surface_width.saturating_sub(x));
        let height = (damage_rect.size.height.0 as u32).min(surface_height.saturating_sub(y));

        if width == 0 || height == 0 {
            None
        } else {
            Some(ScissorRect {
                x,
                y,
                width,
                height,
            })
        }
    }

    fn apply_scissor_rect(pass: &mut wgpu::RenderPass<'_>, scissor_rect: Option<ScissorRect>) {
        if let Some(scissor_rect) = scissor_rect {
            pass.set_scissor_rect(
                scissor_rect.x,
                scissor_rect.y,
                scissor_rect.width,
                scissor_rect.height,
            );
        }
    }

    pub fn draw(&mut self, scene: &Scene) {
        let viewport_bounds = self.viewport_bounds();
        tracing::trace!(
            target: "wgpu::renderer",
            surface_w = self.surface_config.width,
            surface_h = self.surface_config.height,
            "draw frame"
        );
        tracing::debug!(
            target: "wgpu::renderer",
            surface_w = self.surface_config.width,
            surface_h = self.surface_config.height,
            scene_full_redraw = scene.full_redraw_needed(viewport_bounds),
            damage_rects = scene.damage_rects().len(),
            shadows = scene.shadows.len(),
            quads = scene.quads.len(),
            paths = scene.paths.len(),
            underlines = scene.underlines.len(),
            monochrome_sprites = scene.monochrome_sprites.len(),
            polychrome_sprites = scene.polychrome_sprites.len(),
            surfaces = scene.surfaces.len(),
            "draw scene stats"
        );
        let mut upload_bytes: usize = 0;
        let mut upload_count: u32 = 0;
        let mut draw_call_count: u32 = 0;
        let mut primitive_counts = gpui::perf::PrimitiveCounts::default();

        // Bail out early if the surface has been unconfigured (e.g. during
        // Android background/rotation transitions).  Attempting to acquire
        // a texture from an unconfigured surface can block indefinitely on
        // some drivers (Adreno).
        if !self.surface_configured {
            return;
        }

        let last_error = self.last_error.lock().unwrap().take();
        if let Some(error) = last_error {
            self.failed_frame_count += 1;
            log::error!(
                "GPU error during frame (failure {} of 10): {error}",
                self.failed_frame_count
            );

            // TBD. Does retrying more actually help?
            if self.failed_frame_count > 5 {
                if let Some(res) = self.resources.as_mut() {
                    res.invalidate_intermediate_textures();
                }
                self.atlas.clear();
                self.needs_redraw = true;
                self.has_drawn_frame = false;
                return;
            } else if self.failed_frame_count > 10 {
                panic!("Too many consecutive GPU errors. Last error: {error}");
            }
        } else {
            self.failed_frame_count = 0;
        }

        let scene_full_redraw = scene.full_redraw_needed(viewport_bounds);
        let partial_redraw_enabled = false;
        let full_redraw = !partial_redraw_enabled || !self.has_drawn_frame || scene_full_redraw;
        tracing::debug!(
            target: "wgpu::renderer",
            viewport_bounds = ?viewport_bounds,
            has_drawn_frame = self.has_drawn_frame,
            scene_full_redraw,
            partial_redraw_enabled,
            full_redraw,
            damage_rects = scene.damage_rects().len(),
            "draw damage decision"
        );
        let scissor_rect = if full_redraw {
            None
        } else {
            let damage_rects = scene.damage_rects();
            if damage_rects.is_empty() {
                tracing::debug!(target: "wgpu::renderer", "draw skipped: no damage rects");
                self.last_frame_stats = Some(GpuFrameStats::default());
                self.last_damage_rects = Some(vec![]);
                return;
            }
            let Some(scissor_rect) = Self::scissor_rect_for_damage(
                damage_rects,
                self.surface_config.width,
                self.surface_config.height,
            ) else {
                tracing::debug!(
                    target: "wgpu::renderer",
                    damage_rects = ?damage_rects,
                    surface_w = self.surface_config.width,
                    surface_h = self.surface_config.height,
                    "draw skipped: damage clipped to empty scissor"
                );
                self.last_frame_stats = Some(GpuFrameStats::default());
                self.last_damage_rects = Some(vec![]);
                return;
            };
            tracing::debug!(
                target: "wgpu::renderer",
                scissor_x = scissor_rect.x,
                scissor_y = scissor_rect.y,
                scissor_w = scissor_rect.width,
                scissor_h = scissor_rect.height,
                damage_rects = ?damage_rects,
                "draw using damage scissor"
            );
            Some(scissor_rect)
        };

        self.atlas.before_frame();

        let frame = match self.resources().surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                // Textures must be destroyed before the surface can be reconfigured.
                drop(frame);
                let surface_config = self.surface_config.clone();
                let resources = self.resources_mut();
                resources
                    .surface
                    .configure(&resources.device, &surface_config);
                self.has_drawn_frame = false;
                return;
            }
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                let surface_config = self.surface_config.clone();
                let resources = self.resources_mut();
                resources
                    .surface
                    .configure(&resources.device, &surface_config);
                self.has_drawn_frame = false;
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                *self.last_error.lock().unwrap() =
                    Some("Surface texture validation error".to_string());
                return;
            }
        };

        // Now that we know the surface is healthy, ensure intermediate textures exist
        self.ensure_intermediate_textures();

        let frame_view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let render_to_offscreen = self.surface_copy_supported;

        let gamma_params = GammaParams {
            gamma_ratios: self.rendering_params.gamma_ratios,
            grayscale_enhanced_contrast: self.rendering_params.grayscale_enhanced_contrast,
            subpixel_enhanced_contrast: self.rendering_params.subpixel_enhanced_contrast,
            is_bgr: self.is_bgr as u32,
            _pad: 0,
        };

        let globals = GlobalParams {
            viewport_size: [
                self.surface_config.width as f32,
                self.surface_config.height as f32,
            ],
            premultiplied_alpha: if self.surface_config.alpha_mode
                == wgpu::CompositeAlphaMode::PreMultiplied
            {
                1
            } else {
                0
            },
            pad: 0,
        };

        let path_globals = GlobalParams {
            premultiplied_alpha: 0,
            ..globals
        };

        {
            let resources = self.resources();
            let globals_data = bytemuck::bytes_of(&globals);
            resources
                .queue
                .write_buffer(&resources.globals_buffer, 0, globals_data);
            upload_bytes = upload_bytes.saturating_add(globals_data.len());
            upload_count = upload_count.saturating_add(1);

            let path_globals_data = bytemuck::bytes_of(&path_globals);
            resources.queue.write_buffer(
                &resources.globals_buffer,
                self.path_globals_offset,
                path_globals_data,
            );
            upload_bytes = upload_bytes.saturating_add(path_globals_data.len());
            upload_count = upload_count.saturating_add(1);

            let gamma_data = bytemuck::bytes_of(&gamma_params);
            resources
                .queue
                .write_buffer(&resources.globals_buffer, self.gamma_offset, gamma_data);
            upload_bytes = upload_bytes.saturating_add(gamma_data.len());
            upload_count = upload_count.saturating_add(1);
        }

        {
            let resources = self.resources();
            let device = resources.device.clone();
            let queue = resources.queue.clone();
            let changed = scene.changed_ranges();
            let generation = scene.scene_generation();
            let generation_changed = generation != self.persistent_buffers.last_generation;

            upload_persistent_instances(
                &mut self.persistent_buffers.shadows,
                &mut self.persistent_buffers.last_shadow_len,
                &scene.shadows,
                &changed.shadows,
                generation_changed,
                &device,
                &queue,
                "shadows_persistent",
                &mut upload_bytes,
                &mut upload_count,
            );
            upload_persistent_instances(
                &mut self.persistent_buffers.quads,
                &mut self.persistent_buffers.last_quad_len,
                &scene.quads,
                &changed.quads,
                generation_changed,
                &device,
                &queue,
                "quads_persistent",
                &mut upload_bytes,
                &mut upload_count,
            );
            upload_persistent_instances(
                &mut self.persistent_buffers.underlines,
                &mut self.persistent_buffers.last_underline_len,
                &scene.underlines,
                &changed.underlines,
                generation_changed,
                &device,
                &queue,
                "underlines_persistent",
                &mut upload_bytes,
                &mut upload_count,
            );
            upload_persistent_instances(
                &mut self.persistent_buffers.monochrome_sprites,
                &mut self.persistent_buffers.last_mono_sprite_len,
                &scene.monochrome_sprites,
                &changed.monochrome_sprites,
                generation_changed,
                &device,
                &queue,
                "monochrome_sprites_persistent",
                &mut upload_bytes,
                &mut upload_count,
            );
            upload_persistent_instances(
                &mut self.persistent_buffers.subpixel_sprites,
                &mut self.persistent_buffers.last_subpixel_sprite_len,
                &scene.subpixel_sprites,
                &changed.subpixel_sprites,
                generation_changed,
                &device,
                &queue,
                "subpixel_sprites_persistent",
                &mut upload_bytes,
                &mut upload_count,
            );
            upload_persistent_instances(
                &mut self.persistent_buffers.polychrome_sprites,
                &mut self.persistent_buffers.last_poly_sprite_len,
                &scene.polychrome_sprites,
                &changed.polychrome_sprites,
                generation_changed,
                &device,
                &queue,
                "polychrome_sprites_persistent",
                &mut upload_bytes,
                &mut upload_count,
            );
            compact_persistent_instances(
                &mut self.persistent_buffers.shadows,
                &scene.shadows,
                &device,
                &queue,
                "shadows_persistent",
                &mut upload_bytes,
                &mut upload_count,
            );
            compact_persistent_instances(
                &mut self.persistent_buffers.quads,
                &scene.quads,
                &device,
                &queue,
                "quads_persistent",
                &mut upload_bytes,
                &mut upload_count,
            );
            compact_persistent_instances(
                &mut self.persistent_buffers.underlines,
                &scene.underlines,
                &device,
                &queue,
                "underlines_persistent",
                &mut upload_bytes,
                &mut upload_count,
            );
            compact_persistent_instances(
                &mut self.persistent_buffers.monochrome_sprites,
                &scene.monochrome_sprites,
                &device,
                &queue,
                "monochrome_sprites_persistent",
                &mut upload_bytes,
                &mut upload_count,
            );
            compact_persistent_instances(
                &mut self.persistent_buffers.subpixel_sprites,
                &scene.subpixel_sprites,
                &device,
                &queue,
                "subpixel_sprites_persistent",
                &mut upload_bytes,
                &mut upload_count,
            );
            compact_persistent_instances(
                &mut self.persistent_buffers.polychrome_sprites,
                &scene.polychrome_sprites,
                &device,
                &queue,
                "polychrome_sprites_persistent",
                &mut upload_bytes,
                &mut upload_count,
            );
            self.persistent_buffers.last_generation = generation;
        }

        loop {
            let mut instance_offset: u64 = 0;
            let mut overflow = false;

            let mut encoder =
                self.resources()
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("main_encoder"),
                    });

            {
                let load = if scissor_rect.is_some() {
                    wgpu::LoadOp::Load
                } else {
                    wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                };
                let target_view = if render_to_offscreen {
                    self.resources()
                        .frame_view
                        .as_ref()
                        .expect("frame texture should exist")
                } else {
                    &frame_view
                };
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("main_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load,
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    ..Default::default()
                });
                Self::apply_scissor_rect(&mut pass, scissor_rect);

                if self.supports_multi_draw_indirect && self.indirect_buffer.is_some() {
                    // TODO: collect consecutive same-type batches into DrawIndirectArgs and issue multi_draw_indirect here.
                }

                for batch in scene.batches() {
                    let ok = match batch {
                        PrimitiveBatch::Quads(range) => {
                            Self::add_primitive_count(&mut primitive_counts.quads, range.len());
                            self.draw_quads_persistent(range, &mut pass, &mut draw_call_count)
                        }
                        PrimitiveBatch::Shadows(range) => {
                            Self::add_primitive_count(&mut primitive_counts.shadows, range.len());
                            self.draw_shadows_persistent(range, &mut pass, &mut draw_call_count)
                        }
                        PrimitiveBatch::Paths(range) => {
                            Self::add_primitive_count(&mut primitive_counts.paths, range.len());
                            let paths = &scene.paths[range];
                            if paths.is_empty() {
                                continue;
                            }

                            drop(pass);

                            let did_draw = self.draw_paths_to_intermediate(
                                &mut encoder,
                                paths,
                                &mut instance_offset,
                                &mut upload_bytes,
                                &mut upload_count,
                                &mut draw_call_count,
                            );

                            let target_view = if render_to_offscreen {
                                self.resources()
                                    .frame_view
                                    .as_ref()
                                    .expect("frame texture should exist")
                            } else {
                                &frame_view
                            };

                            pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("main_pass_continued"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: target_view,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Load,
                                        store: wgpu::StoreOp::Store,
                                    },
                                    depth_slice: None,
                                })],
                                depth_stencil_attachment: None,
                                ..Default::default()
                            });
                            Self::apply_scissor_rect(&mut pass, scissor_rect);

                            if did_draw {
                                self.draw_paths_from_intermediate(
                                    paths,
                                    &mut instance_offset,
                                    &mut pass,
                                    &mut upload_bytes,
                                    &mut upload_count,
                                    &mut draw_call_count,
                                )
                            } else {
                                false
                            }
                        }
                        PrimitiveBatch::Underlines(range) => {
                            Self::add_primitive_count(
                                &mut primitive_counts.underlines,
                                range.len(),
                            );
                            self.draw_underlines_persistent(range, &mut pass, &mut draw_call_count)
                        }
                        PrimitiveBatch::MonochromeSprites { texture_id, range } => {
                            Self::add_primitive_count(
                                &mut primitive_counts.monochrome_sprites,
                                range.len(),
                            );
                            self.draw_monochrome_sprites_persistent(
                                texture_id,
                                range,
                                &mut pass,
                                &mut draw_call_count,
                            )
                        }
                        PrimitiveBatch::SubpixelSprites { texture_id, range } => {
                            Self::add_primitive_count(
                                &mut primitive_counts.subpixel_sprites,
                                range.len(),
                            );
                            self.draw_subpixel_sprites_persistent(
                                texture_id,
                                range,
                                &mut pass,
                                &mut draw_call_count,
                            )
                        }
                        PrimitiveBatch::PolychromeSprites { texture_id, range } => {
                            Self::add_primitive_count(
                                &mut primitive_counts.polychrome_sprites,
                                range.len(),
                            );
                            self.draw_polychrome_sprites_persistent(
                                texture_id,
                                range,
                                &mut pass,
                                &mut draw_call_count,
                            )
                        }
                        PrimitiveBatch::Surfaces(range) => {
                            Self::add_primitive_count(&mut primitive_counts.surfaces, range.len());
                            self.draw_surfaces(
                                &scene.surfaces[range],
                                &mut pass,
                                &mut draw_call_count,
                            )
                        }
                    };
                    if !ok {
                        overflow = true;
                        break;
                    }
                }
            }

            if overflow {
                drop(encoder);
                if self.instance_buffer_capacity >= self.max_buffer_size {
                    log::error!(
                        "instance buffer size grew too large: {}",
                        self.instance_buffer_capacity
                    );
                    frame.present();
                    return;
                }
                self.grow_instance_buffer();
                continue;
            }

            if render_to_offscreen {
                let resources = self.resources();
                let frame_texture = resources
                    .frame_texture
                    .as_ref()
                    .expect("frame texture should exist");
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: frame_texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: &frame.texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d {
                        width: self.surface_config.width,
                        height: self.surface_config.height,
                        depth_or_array_layers: 1,
                    },
                );
            }

            self.resources()
                .queue
                .submit(std::iter::once(encoder.finish()));
            frame.present();
            self.has_drawn_frame = true;
            // Store damage info for Wayland damage_buffer() reporting.
            // full_redraw=true → None (entire surface), otherwise specific rects in device pixels.
            if full_redraw {
                self.last_damage_rects = None;
            } else {
                let scene_damage = scene.damage_rects();
                let device_rects: Vec<[i32; 4]> = scene_damage
                    .iter()
                    .map(|b| {
                        [
                            b.origin.x.0 as i32,
                            b.origin.y.0 as i32,
                            b.size.width.0 as i32,
                            b.size.height.0 as i32,
                        ]
                    })
                    .collect();
                self.last_damage_rects = Some(device_rects);
            }
            self.last_frame_stats = Some(GpuFrameStats {
                upload_bytes,
                upload_count,
                draw_call_count,
                primitive_counts,
            });
            return;
        }
    }

    #[allow(dead_code)]
    fn draw_quads(
        &self,
        quads: &[Quad],
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
        upload_bytes: &mut usize,
        upload_count: &mut u32,
        draw_call_count: &mut u32,
    ) -> bool {
        let data = unsafe { Self::instance_bytes(quads) };
        self.draw_instances(
            data,
            quads.len() as u32,
            &self.resources().pipelines.quads,
            instance_offset,
            pass,
            upload_bytes,
            upload_count,
            draw_call_count,
        )
    }

    #[allow(dead_code)]
    fn draw_shadows(
        &self,
        shadows: &[Shadow],
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
        upload_bytes: &mut usize,
        upload_count: &mut u32,
        draw_call_count: &mut u32,
    ) -> bool {
        let data = unsafe { Self::instance_bytes(shadows) };
        self.draw_instances(
            data,
            shadows.len() as u32,
            &self.resources().pipelines.shadows,
            instance_offset,
            pass,
            upload_bytes,
            upload_count,
            draw_call_count,
        )
    }

    #[allow(dead_code)]
    fn draw_underlines(
        &self,
        underlines: &[Underline],
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
        upload_bytes: &mut usize,
        upload_count: &mut u32,
        draw_call_count: &mut u32,
    ) -> bool {
        let data = unsafe { Self::instance_bytes(underlines) };
        self.draw_instances(
            data,
            underlines.len() as u32,
            &self.resources().pipelines.underlines,
            instance_offset,
            pass,
            upload_bytes,
            upload_count,
            draw_call_count,
        )
    }

    #[allow(dead_code)]
    fn draw_monochrome_sprites(
        &self,
        sprites: &[MonochromeSprite],
        texture_id: AtlasTextureId,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
        upload_bytes: &mut usize,
        upload_count: &mut u32,
        draw_call_count: &mut u32,
    ) -> bool {
        let tex_info = self.atlas.get_texture_info(texture_id);
        let data = unsafe { Self::instance_bytes(sprites) };
        self.draw_instances_with_texture(
            data,
            sprites.len() as u32,
            &tex_info.view,
            &self.resources().pipelines.mono_sprites,
            instance_offset,
            pass,
            upload_bytes,
            upload_count,
            draw_call_count,
        )
    }

    #[allow(dead_code)]
    fn draw_subpixel_sprites(
        &self,
        sprites: &[SubpixelSprite],
        texture_id: AtlasTextureId,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
        upload_bytes: &mut usize,
        upload_count: &mut u32,
        draw_call_count: &mut u32,
    ) -> bool {
        let tex_info = self.atlas.get_texture_info(texture_id);
        let data = unsafe { Self::instance_bytes(sprites) };
        let resources = self.resources();
        let pipeline = resources
            .pipelines
            .subpixel_sprites
            .as_ref()
            .unwrap_or(&resources.pipelines.mono_sprites);
        self.draw_instances_with_texture(
            data,
            sprites.len() as u32,
            &tex_info.view,
            pipeline,
            instance_offset,
            pass,
            upload_bytes,
            upload_count,
            draw_call_count,
        )
    }

    #[allow(dead_code)]
    fn draw_polychrome_sprites(
        &self,
        sprites: &[PolychromeSprite],
        texture_id: AtlasTextureId,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
        upload_bytes: &mut usize,
        upload_count: &mut u32,
        draw_call_count: &mut u32,
    ) -> bool {
        let tex_info = self.atlas.get_texture_info(texture_id);
        let data = unsafe { Self::instance_bytes(sprites) };
        self.draw_instances_with_texture(
            data,
            sprites.len() as u32,
            &tex_info.view,
            &self.resources().pipelines.poly_sprites,
            instance_offset,
            pass,
            upload_bytes,
            upload_count,
            draw_call_count,
        )
    }

    fn draw_quads_persistent(
        &self,
        range: Range<usize>,
        pass: &mut wgpu::RenderPass<'_>,
        draw_call_count: &mut u32,
    ) -> bool {
        self.draw_persistent_instances(
            range,
            &self.persistent_buffers.quads,
            &self.resources().pipelines.quads,
            pass,
            draw_call_count,
        )
    }

    fn draw_shadows_persistent(
        &self,
        range: Range<usize>,
        pass: &mut wgpu::RenderPass<'_>,
        draw_call_count: &mut u32,
    ) -> bool {
        self.draw_persistent_instances(
            range,
            &self.persistent_buffers.shadows,
            &self.resources().pipelines.shadows,
            pass,
            draw_call_count,
        )
    }

    fn draw_underlines_persistent(
        &self,
        range: Range<usize>,
        pass: &mut wgpu::RenderPass<'_>,
        draw_call_count: &mut u32,
    ) -> bool {
        self.draw_persistent_instances(
            range,
            &self.persistent_buffers.underlines,
            &self.resources().pipelines.underlines,
            pass,
            draw_call_count,
        )
    }

    fn draw_monochrome_sprites_persistent(
        &self,
        texture_id: AtlasTextureId,
        range: Range<usize>,
        pass: &mut wgpu::RenderPass<'_>,
        draw_call_count: &mut u32,
    ) -> bool {
        let tex_info = self.atlas.get_texture_info(texture_id);
        self.draw_persistent_instances_with_texture(
            range,
            &self.persistent_buffers.monochrome_sprites,
            &tex_info.view,
            &self.resources().pipelines.mono_sprites,
            pass,
            draw_call_count,
        )
    }

    fn draw_subpixel_sprites_persistent(
        &self,
        texture_id: AtlasTextureId,
        range: Range<usize>,
        pass: &mut wgpu::RenderPass<'_>,
        draw_call_count: &mut u32,
    ) -> bool {
        let tex_info = self.atlas.get_texture_info(texture_id);
        let resources = self.resources();
        let pipeline = resources
            .pipelines
            .subpixel_sprites
            .as_ref()
            .unwrap_or(&resources.pipelines.mono_sprites);
        self.draw_persistent_instances_with_texture(
            range,
            &self.persistent_buffers.subpixel_sprites,
            &tex_info.view,
            pipeline,
            pass,
            draw_call_count,
        )
    }

    fn draw_polychrome_sprites_persistent(
        &self,
        texture_id: AtlasTextureId,
        range: Range<usize>,
        pass: &mut wgpu::RenderPass<'_>,
        draw_call_count: &mut u32,
    ) -> bool {
        let tex_info = self.atlas.get_texture_info(texture_id);
        self.draw_persistent_instances_with_texture(
            range,
            &self.persistent_buffers.polychrome_sprites,
            &tex_info.view,
            &self.resources().pipelines.poly_sprites,
            pass,
            draw_call_count,
        )
    }

    fn draw_persistent_instances(
        &self,
        range: Range<usize>,
        buffer: &PersistentTypeBuffer,
        pipeline: &wgpu::RenderPipeline,
        pass: &mut wgpu::RenderPass<'_>,
        draw_call_count: &mut u32,
    ) -> bool {
        let instance_count = range.len() as u32;
        if instance_count == 0 {
            return true;
        }

        let resources = self.resources();
        let bind_group = resources
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &resources.bind_group_layouts.instances,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.full_binding(),
                }],
            });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &resources.globals_bind_group, &[]);
        pass.set_bind_group(1, &bind_group, &[]);
        pass.draw(0..4, range.start as u32..range.end as u32);
        *draw_call_count = draw_call_count.saturating_add(1);
        true
    }

    fn draw_persistent_instances_with_texture(
        &self,
        range: Range<usize>,
        buffer: &PersistentTypeBuffer,
        texture_view: &wgpu::TextureView,
        pipeline: &wgpu::RenderPipeline,
        pass: &mut wgpu::RenderPass<'_>,
        draw_call_count: &mut u32,
    ) -> bool {
        let instance_count = range.len() as u32;
        if instance_count == 0 {
            return true;
        }

        let resources = self.resources();
        let bind_group = resources
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &resources.bind_group_layouts.instances_with_texture,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.full_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(texture_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&resources.atlas_sampler),
                    },
                ],
            });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &resources.globals_bind_group, &[]);
        pass.set_bind_group(1, &bind_group, &[]);
        pass.draw(0..4, range.start as u32..range.end as u32);
        *draw_call_count = draw_call_count.saturating_add(1);
        true
    }

    #[allow(dead_code)]
    fn draw_instances(
        &self,
        data: &[u8],
        instance_count: u32,
        pipeline: &wgpu::RenderPipeline,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
        upload_bytes: &mut usize,
        upload_count: &mut u32,
        draw_call_count: &mut u32,
    ) -> bool {
        if instance_count == 0 {
            return true;
        }
        let Some((offset, size)) =
            self.write_to_instance_buffer(instance_offset, data, upload_bytes, upload_count)
        else {
            return false;
        };
        let resources = self.resources();
        let bind_group = resources
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &resources.bind_group_layouts.instances,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.instance_binding(offset, size),
                }],
            });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &resources.globals_bind_group, &[]);
        pass.set_bind_group(1, &bind_group, &[]);
        pass.draw(0..4, 0..instance_count);
        *draw_call_count = draw_call_count.saturating_add(1);
        true
    }

    fn draw_instances_with_texture(
        &self,
        data: &[u8],
        instance_count: u32,
        texture_view: &wgpu::TextureView,
        pipeline: &wgpu::RenderPipeline,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
        upload_bytes: &mut usize,
        upload_count: &mut u32,
        draw_call_count: &mut u32,
    ) -> bool {
        if instance_count == 0 {
            return true;
        }
        let Some((offset, size)) =
            self.write_to_instance_buffer(instance_offset, data, upload_bytes, upload_count)
        else {
            return false;
        };
        let resources = self.resources();
        let bind_group = resources
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &resources.bind_group_layouts.instances_with_texture,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.instance_binding(offset, size),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(texture_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&resources.atlas_sampler),
                    },
                ],
            });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &resources.globals_bind_group, &[]);
        pass.set_bind_group(1, &bind_group, &[]);
        pass.draw(0..4, 0..instance_count);
        *draw_call_count = draw_call_count.saturating_add(1);
        true
    }

    fn draw_surfaces(
        &self,
        surfaces: &[gpui::PaintSurface],
        pass: &mut wgpu::RenderPass<'_>,
        draw_call_count: &mut u32,
    ) -> bool {
        let resources = self.resources();
        pass.set_pipeline(&resources.pipelines.surfaces);
        pass.set_bind_group(0, &resources.globals_bind_group, &[]);

        for surface in surfaces {
            let Some(texture) = surface.content.external_texture() else {
                continue;
            };
            let Some(resource) = texture
                .resource
                .as_any()
                .downcast_ref::<WgpuTextureResource>()
            else {
                warn!(
                    "ignoring external {:?} texture in wgpu renderer",
                    texture.backend()
                );
                continue;
            };
            let params = SurfaceParams {
                bounds: surface.bounds.into(),
                content_mask: surface.content_mask.bounds.into(),
            };
            let params_buffer =
                resources
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: None,
                        contents: bytemuck::bytes_of(&params),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
            let bind_group = resources
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &resources.bind_group_layouts.surfaces,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: params_buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(resource.view()),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&resources.atlas_sampler),
                        },
                    ],
                });
            pass.set_bind_group(1, &bind_group, &[]);
            pass.draw(0..4, 0..1);
            *draw_call_count = draw_call_count.saturating_add(1);
        }

        true
    }

    unsafe fn instance_bytes<T>(instances: &[T]) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                instances.as_ptr() as *const u8,
                std::mem::size_of_val(instances),
            )
        }
    }

    fn add_primitive_count(count: &mut u32, len: usize) {
        *count = count.saturating_add(u32::try_from(len).unwrap_or(u32::MAX));
    }

    fn draw_paths_from_intermediate(
        &self,
        paths: &[Path<ScaledPixels>],
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
        upload_bytes: &mut usize,
        upload_count: &mut u32,
        draw_call_count: &mut u32,
    ) -> bool {
        let first_path = &paths[0];
        let sprites: Vec<PathSprite> = if paths.last().map(|p| &p.order) == Some(&first_path.order)
        {
            paths
                .iter()
                .map(|p| PathSprite {
                    bounds: p.clipped_bounds(),
                })
                .collect()
        } else {
            let mut bounds = first_path.clipped_bounds();
            for path in paths.iter().skip(1) {
                bounds = bounds.union(&path.clipped_bounds());
            }
            vec![PathSprite { bounds }]
        };

        let resources = self.resources();
        let Some(path_intermediate_view) = resources.path_intermediate_view.as_ref() else {
            return true;
        };

        let sprite_data = unsafe { Self::instance_bytes(&sprites) };
        self.draw_instances_with_texture(
            sprite_data,
            sprites.len() as u32,
            path_intermediate_view,
            &resources.pipelines.paths,
            instance_offset,
            pass,
            upload_bytes,
            upload_count,
            draw_call_count,
        )
    }

    fn draw_paths_to_intermediate(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        paths: &[Path<ScaledPixels>],
        instance_offset: &mut u64,
        upload_bytes: &mut usize,
        upload_count: &mut u32,
        draw_call_count: &mut u32,
    ) -> bool {
        let mut vertices = Vec::new();
        for path in paths {
            let bounds = path.clipped_bounds();
            vertices.extend(path.vertices.iter().map(|v| PathRasterizationVertex {
                xy_position: v.xy_position,
                st_position: v.st_position,
                color: path.color,
                bounds,
            }));
        }

        if vertices.is_empty() {
            return true;
        }

        let vertex_data = unsafe { Self::instance_bytes(&vertices) };
        let Some((vertex_offset, vertex_size)) =
            self.write_to_instance_buffer(instance_offset, vertex_data, upload_bytes, upload_count)
        else {
            return false;
        };

        let resources = self.resources();
        let data_bind_group = resources
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("path_rasterization_bind_group"),
                layout: &resources.bind_group_layouts.instances,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.instance_binding(vertex_offset, vertex_size),
                }],
            });

        let Some(path_intermediate_view) = resources.path_intermediate_view.as_ref() else {
            return true;
        };

        let (target_view, resolve_target) = if let Some(ref msaa_view) = resources.path_msaa_view {
            (msaa_view, Some(path_intermediate_view))
        } else {
            (path_intermediate_view, None)
        };

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("path_rasterization_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });

            pass.set_pipeline(&resources.pipelines.path_rasterization);
            pass.set_bind_group(0, &resources.path_globals_bind_group, &[]);
            pass.set_bind_group(1, &data_bind_group, &[]);
            pass.draw(0..vertices.len() as u32, 0..1);
            *draw_call_count = draw_call_count.saturating_add(1);
        }

        true
    }

    fn grow_instance_buffer(&mut self) {
        let new_capacity = (self.instance_buffer_capacity * 2).min(self.max_buffer_size);
        log::info!("increased instance buffer size to {}", new_capacity);
        let resources = self.resources_mut();
        resources.instance_buffer = resources.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instance_buffer"),
            size: new_capacity,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.instance_buffer_capacity = new_capacity;
    }

    fn write_to_instance_buffer(
        &self,
        instance_offset: &mut u64,
        data: &[u8],
        upload_bytes: &mut usize,
        upload_count: &mut u32,
    ) -> Option<(u64, NonZeroU64)> {
        let offset = (*instance_offset).next_multiple_of(self.storage_buffer_alignment);
        let size = (data.len() as u64).max(16);
        if offset + size > self.instance_buffer_capacity {
            return None;
        }
        let resources = self.resources();
        resources
            .queue
            .write_buffer(&resources.instance_buffer, offset, data);
        *upload_bytes = upload_bytes.saturating_add(data.len());
        *upload_count = upload_count.saturating_add(1);
        *instance_offset = offset + size;
        Some((offset, NonZeroU64::new(size).expect("size is at least 16")))
    }

    fn instance_binding(&self, offset: u64, size: NonZeroU64) -> wgpu::BindingResource<'_> {
        wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer: &self.resources().instance_buffer,
            offset,
            size: Some(size),
        })
    }

    /// Mark the surface as unconfigured so rendering is skipped until a new
    /// surface is provided via [`replace_surface`](Self::replace_surface).
    ///
    /// This does **not** drop the renderer — the device, queue, atlas, and
    /// pipelines stay alive.  Use this when the native window is destroyed
    /// (e.g. Android `TerminateWindow`) but you intend to re-create the
    /// surface later without losing cached atlas textures.
    pub fn unconfigure_surface(&mut self) {
        self.surface_configured = false;
        self.has_drawn_frame = false;
        // Drop intermediate textures since they reference the old surface size.
        if let Some(res) = self.resources.as_mut() {
            res.invalidate_intermediate_textures();
        }
    }

    /// Replace the wgpu surface with a new one (e.g. after Android destroys
    /// and recreates the native window).  Keeps the device, queue, atlas, and
    /// all pipelines intact so cached `AtlasTextureId`s remain valid.
    ///
    /// The `instance` **must** be the same [`wgpu::Instance`] that was used to
    /// create the adapter and device (i.e. from the [`WgpuContext`]).  Using a
    /// different instance will cause a "Device does not exist" panic because
    /// the wgpu device is bound to its originating instance.
    #[cfg(not(target_family = "wasm"))]
    pub fn replace_surface<W: HasWindowHandle>(
        &mut self,
        window: &W,
        config: WgpuSurfaceConfig,
        instance: &wgpu::Instance,
    ) -> anyhow::Result<()> {
        let window_handle = window
            .window_handle()
            .map_err(|e| anyhow::anyhow!("Failed to get window handle: {e}"))?;

        let surface = create_surface(instance, window_handle.as_raw())?;

        let width = (config.size.width.0 as u32).max(1);
        let height = (config.size.height.0 as u32).max(1);

        let alpha_mode = if config.transparent {
            self.transparent_alpha_mode
        } else {
            self.opaque_alpha_mode
        };

        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface_config.alpha_mode = alpha_mode;
        if let Some(mode) = config.preferred_present_mode {
            self.surface_config.present_mode = mode;
        }

        {
            let res = self
                .resources
                .as_mut()
                .expect("GPU resources not available");
            surface.configure(&res.device, &self.surface_config);
            res.surface = surface;

            // Invalidate intermediate textures — they'll be recreated lazily.
            res.invalidate_intermediate_textures();
        }

        self.surface_configured = true;
        self.has_drawn_frame = false;

        Ok(())
    }

    pub fn destroy(&mut self) {
        // Release surface-bound GPU resources eagerly so the underlying native
        // window can be destroyed before the renderer itself is dropped.
        self.resources.take();
    }

    /// Returns true if the GPU device was lost and recovery is needed.
    pub fn device_lost(&self) -> bool {
        self.device_lost.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Returns true if a redraw is needed because GPU state was cleared.
    /// Calling this method clears the flag.
    pub fn needs_redraw(&mut self) -> bool {
        std::mem::take(&mut self.needs_redraw)
    }

    /// Returns the damage rectangles from the last draw call, in device (buffer) pixels.
    /// `None` means the entire surface was redrawn (full redraw).
    /// `Some(&[])` would mean no damage (should not normally occur after a draw).
    /// Each rect is `[x, y, width, height]`.
    pub fn last_damage_rects(&self) -> Option<&[[i32; 4]]> {
        self.last_damage_rects.as_deref()
    }

    pub fn last_frame_stats(&self) -> Option<&GpuFrameStats> {
        self.last_frame_stats.as_ref()
    }

    /// Recovers from a lost GPU device by recreating the renderer with a new context.
    ///
    /// Call this after detecting `device_lost()` returns true.
    ///
    /// This method coordinates recovery across multiple windows:
    /// - The first window to call this will recreate the shared context
    /// - Subsequent windows will adopt the already-recovered context
    #[cfg(not(target_family = "wasm"))]
    pub fn recover<W>(&mut self, window: &W) -> anyhow::Result<()>
    where
        W: HasWindowHandle + HasDisplayHandle + std::fmt::Debug + Send + Sync + Clone + 'static,
    {
        let gpu_context = self.context.as_ref().expect("recover requires gpu_context");

        // Check if another window already recovered the context
        let needs_new_context = gpu_context
            .borrow()
            .as_ref()
            .is_none_or(|ctx| ctx.device_lost());

        let window_handle = window
            .window_handle()
            .map_err(|e| anyhow::anyhow!("Failed to get window handle: {e}"))?;

        let surface = if needs_new_context {
            log::warn!("GPU device lost, recreating context...");

            // Drop old resources to release Arc<Device>/Arc<Queue> and GPU resources
            self.resources = None;
            *gpu_context.borrow_mut() = None;

            // Wait for GPU driver to stabilize (350ms copied from windows :shrug:)
            std::thread::sleep(std::time::Duration::from_millis(350));

            let instance = WgpuContext::instance(Box::new(window.clone()));
            let surface = create_surface(&instance, window_handle.as_raw())?;
            let new_context = WgpuContext::new(instance, &surface, self.compositor_gpu)?;
            *gpu_context.borrow_mut() = Some(new_context);
            surface
        } else {
            let ctx_ref = gpu_context.borrow();
            let instance = &ctx_ref.as_ref().unwrap().instance;
            create_surface(instance, window_handle.as_raw())?
        };

        let config = WgpuSurfaceConfig {
            size: gpui::Size {
                width: gpui::DevicePixels(self.surface_config.width as i32),
                height: gpui::DevicePixels(self.surface_config.height as i32),
            },
            transparent: self.surface_config.alpha_mode != wgpu::CompositeAlphaMode::Opaque,
            preferred_present_mode: Some(self.surface_config.present_mode),
        };
        let gpu_context = Rc::clone(gpu_context);
        let ctx_ref = gpu_context.borrow();
        let context = ctx_ref.as_ref().expect("context should exist");

        self.resources = None;
        self.atlas.handle_device_lost(context);

        *self = Self::new_internal(
            Some(gpu_context.clone()),
            context,
            surface,
            config,
            self.compositor_gpu,
            self.atlas.clone(),
        )?;

        log::info!("GPU recovery complete");
        Ok(())
    }
}

#[cfg(not(target_family = "wasm"))]
fn create_surface(
    instance: &wgpu::Instance,
    raw_window_handle: raw_window_handle::RawWindowHandle,
) -> anyhow::Result<wgpu::Surface<'static>> {
    unsafe {
        instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                // Fall back to the display handle already provided via InstanceDescriptor::display.
                raw_display_handle: None,
                raw_window_handle,
            })
            .map_err(|e| anyhow::anyhow!("{e}"))
    }
}

struct RenderingParameters {
    path_sample_count: u32,
    gamma_ratios: [f32; 4],
    grayscale_enhanced_contrast: f32,
    subpixel_enhanced_contrast: f32,
}

impl RenderingParameters {
    fn new(adapter: &wgpu::Adapter, surface_format: wgpu::TextureFormat) -> Self {
        use std::env;

        let format_features = adapter.get_texture_format_features(surface_format);
        let path_sample_count = [4, 2, 1]
            .into_iter()
            .find(|&n| format_features.flags.sample_count_supported(n))
            .unwrap_or(1);

        let gamma = env::var("ZED_FONTS_GAMMA")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.8_f32)
            .clamp(1.0, 2.2);
        let gamma_ratios = get_gamma_correction_ratios(gamma);

        let grayscale_enhanced_contrast = env::var("ZED_FONTS_GRAYSCALE_ENHANCED_CONTRAST")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0_f32)
            .max(0.0);

        let subpixel_enhanced_contrast = env::var("ZED_FONTS_SUBPIXEL_ENHANCED_CONTRAST")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.5_f32)
            .max(0.0);

        Self {
            path_sample_count,
            gamma_ratios,
            grayscale_enhanced_contrast,
            subpixel_enhanced_contrast,
        }
    }
}
