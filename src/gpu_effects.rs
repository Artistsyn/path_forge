use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct EffectParams {
    width: u32,
    height: u32,
    _pad0: u32,
    _pad1: u32,
    saturation: f32,
    contrast: f32,
    brightness: f32,
    effect_mix: f32,
}

#[derive(Clone, Copy)]
pub struct GpuEffectSettings {
    pub saturation: f32,
    pub contrast: f32,
    pub brightness: f32,
    pub effect_mix: f32,
}

impl Default for GpuEffectSettings {
    fn default() -> Self {
        Self {
            saturation: 1.0,
            contrast: 1.0,
            brightness: 1.0,
            effect_mix: 0.0,
        }
    }
}

pub struct GpuEffects {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

fn aligned_bytes_per_row(width: u32) -> u32 {
    let raw = width.saturating_mul(4);
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    ((raw + align - 1) / align) * align
}

impl GpuEffects {
    pub fn new() -> Result<Self, String> {
        pollster::block_on(Self::new_async())
    }

    async fn new_async() -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            dx12_shader_compiler: Default::default(),
            ..Default::default()
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .ok_or_else(|| "No GPU adapter available".to_string())?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("path_forge_gpu_device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await
            .map_err(|e| format!("request_device failed: {e}"))?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("path_forge_gpu_effects_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_WGSL.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("path_forge_gpu_effects_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("path_forge_gpu_effects_pl"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("path_forge_gpu_effects_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: "main",
            compilation_options: Default::default(),
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("path_forge_gpu_effects_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Ok(Self {
            device,
            queue,
            pipeline,
            bind_group_layout,
            sampler,
        })
    }

    pub fn process_rgba(
        &self,
        input_rgba: &[u8],
        width: u32,
        height: u32,
        settings: &GpuEffectSettings,
    ) -> Result<Vec<u8>, String> {
        if width == 0 || height == 0 {
            return Err("Invalid zero-size frame".to_string());
        }
        let expected = width as usize * height as usize * 4;
        if input_rgba.len() != expected {
            return Err(format!(
                "Frame size mismatch: got {}, expected {}",
                input_rgba.len(),
                expected
            ));
        }

        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let in_tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("path_forge_gpu_in_tex"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &in_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            input_rgba,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            extent,
        );

        let out_tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("path_forge_gpu_out_tex"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let params = EffectParams {
            width,
            height,
            _pad0: 0,
            _pad1: 0,
            saturation: settings.saturation.clamp(0.0, 2.5),
            contrast: settings.contrast.clamp(0.2, 3.0),
            brightness: settings.brightness.clamp(0.2, 3.0),
            effect_mix: settings.effect_mix.clamp(0.0, 1.0),
        };

        let params_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("path_forge_gpu_params"),
            size: std::mem::size_of::<EffectParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue
            .write_buffer(&params_buf, 0, bytemuck::bytes_of(&params));

        let in_view = in_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let out_view = out_tex.create_view(&wgpu::TextureViewDescriptor::default());

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("path_forge_gpu_effects_bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&in_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&out_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: params_buf.as_entire_binding(),
                },
            ],
        });

        let tight_bpr = width.saturating_mul(4);
        let padded_bpr = aligned_bytes_per_row(width);
        let out_buf_size = (padded_bpr as u64).saturating_mul(height as u64);
        let out_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("path_forge_gpu_readback"),
            size: out_buf_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("path_forge_gpu_effects_encoder"),
            });

        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("path_forge_gpu_effects_pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.pipeline);
            cpass.set_bind_group(0, &bind_group, &[]);
            let gx = width.div_ceil(8);
            let gy = height.div_ceil(8);
            cpass.dispatch_workgroups(gx, gy, 1);
        }

        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &out_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &out_buf,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bpr),
                    rows_per_image: Some(height),
                },
            },
            extent,
        );

        self.queue.submit(Some(encoder.finish()));
        let slice = out_buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = tx.send(res);
        });
        self.device.poll(wgpu::Maintain::Wait);
        match rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(format!("GPU readback map failed: {e}")),
            Err(_) => return Err("GPU readback channel closed".to_string()),
        }

        let mapped = slice.get_mapped_range();
        let mut out = vec![0u8; expected];
        let padded_bpr_usize = padded_bpr as usize;
        let tight_bpr_usize = tight_bpr as usize;
        for row in 0..height as usize {
            let src_off = row * padded_bpr_usize;
            let dst_off = row * tight_bpr_usize;
            out[dst_off..dst_off + tight_bpr_usize]
                .copy_from_slice(&mapped[src_off..src_off + tight_bpr_usize]);
        }
        drop(mapped);
        out_buf.unmap();
        Ok(out)
    }
}

const SHADER_WGSL: &str = r#"
struct Params {
    width: u32,
    height: u32,
    _pad0: u32,
    _pad1: u32,
    saturation: f32,
    contrast: f32,
    brightness: f32,
    effect_mix: f32,
};

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;
@group(0) @binding(2) var dst_tex: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(3) var<uniform> params: Params;

fn to_luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.width || gid.y >= params.height) {
        return;
    }

    let uv = (vec2<f32>(f32(gid.x), f32(gid.y)) + vec2<f32>(0.5, 0.5))
        / vec2<f32>(f32(params.width), f32(params.height));

    let src = textureSampleLevel(src_tex, src_sampler, uv, 0.0);
    let l = to_luma(src.rgb);
    let sat_col = mix(vec3<f32>(l, l, l), src.rgb, params.saturation);
    let ctr_col = (sat_col - vec3<f32>(0.5, 0.5, 0.5)) * params.contrast + vec3<f32>(0.5, 0.5, 0.5);
    let brt_col = ctr_col * params.brightness;

    let cool_tone = vec3<f32>(brt_col.r * 0.98, brt_col.g * 1.00, brt_col.b * 1.03);
    let out_rgb = mix(src.rgb, cool_tone, params.effect_mix);
    textureStore(dst_tex, vec2<i32>(i32(gid.x), i32(gid.y)), vec4<f32>(clamp(out_rgb, vec3<f32>(0.0), vec3<f32>(1.0)), src.a));
}
"#;
