use gpui::GpuTextureFormat;
use gpui_wgpu::{WgpuTextureResource, wgpu};
use std::sync::Arc;

fn main() -> anyhow::Result<()> {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            flags: wgpu::InstanceFlags::default(),
            backend_options: wgpu::BackendOptions::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            display: None,
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await?;
        let (device, _queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("wgpu_texture_handle_example_device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults()
                    .using_resolution(adapter.limits())
                    .using_alignment(adapter.limits()),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
            })
            .await?;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("wgpu_texture_handle_example_texture"),
            size: wgpu::Extent3d {
                width: 16,
                height: 16,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = Arc::new(texture.create_view(&wgpu::TextureViewDescriptor::default()));
        let handle = WgpuTextureResource::handle(view, 16, 16, GpuTextureFormat::RGBA8);

        anyhow::ensure!(handle.width == 16);
        anyhow::ensure!(handle.height == 16);
        anyhow::ensure!(handle.bytes_per_pixel() == 4);
        Ok(())
    })
}
