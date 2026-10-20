//! Real GPU triangle pipeline for validated CryEngine source mesh streams.
//! Samples user-selected original DDS diffuse pixels against source UV0; it
//! never guesses cross-PAK materials or fabricates a complete character rig.
use evolve_core::RenderMesh;
use iced::wgpu;
use iced::wgpu::util::{DeviceExt, TextureDataOrder};
use iced::widget::shader::{self, Viewport};
use iced::{Event, Rectangle, mouse};
use std::borrow::Cow;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Surface {
    pub first_index: u32,
    pub index_count: u32,
    /// Explicit CryEngine NoDraw subset. Preserve its index span, but do not render it.
    pub visible: bool,
    pub transparent: bool,
    pub diffuse: Option<Arc<image::RgbaImage>>,
    pub normal: Option<Arc<image::RgbaImage>>,
    pub emissive: Option<Arc<image::RgbaImage>>,
    pub specular: Option<Arc<image::RgbaImage>>,
    pub gloss: Option<Arc<image::RgbaImage>>,
    /// Verified authored opacity DDS; not interchangeable with diffuse alpha.
    pub opacity: Option<Arc<image::RgbaImage>>,
}

#[derive(Clone)]
pub struct Scene {
    pub mesh: Arc<RenderMesh>,
    /// Per-attachment material ranges for complete CDF previews.
    pub surfaces: Option<Arc<Vec<Surface>>>,
    pub diffuse: Option<Arc<image::RgbaImage>>,
    pub normal_map: Option<Arc<image::RgbaImage>>,
    pub emissive: Option<Arc<image::RgbaImage>>,
    pub specular: Option<Arc<image::RgbaImage>>,
    pub gloss: Option<Arc<image::RgbaImage>>,
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    pub light_strength: f32,
    pub pan_x: f32,
    pub pan_y: f32,
    pub shift: bool,
}

#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct GroundVertex {
    position: [f32; 3],
    rgba: [f32; 4],
}
impl GroundVertex {
    const ATTRS: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![
        0 => Float32x3,
        1 => Float32x4,
    ];
    fn descriptor<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRS,
        }
    }
}

#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct Vertex {
    xyz: [f32; 3],
    uv: [f32; 2],
    normal: [f32; 3],
    view_position: [f32; 3],
    light_strength: f32,
}
impl Vertex {
    const ATTRS: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
        0 => Float32x3,
        1 => Float32x2,
        2 => Float32x3,
        3 => Float32x3,
        4 => Float32,
    ];
    fn descriptor<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRS,
        }
    }
}
#[derive(Debug)]
pub struct Primitive {
    points: Vec<Vertex>,
    ground: Vec<GroundVertex>,
    indices: Vec<u32>,
    surfaces: Option<Arc<Vec<Surface>>>,
    diffuse: Option<Arc<image::RgbaImage>>,
    normal_map: Option<Arc<image::RgbaImage>>,
    emissive: Option<Arc<image::RgbaImage>>,
    specular: Option<Arc<image::RgbaImage>>,
    gloss: Option<Arc<image::RgbaImage>>,
    mesh_identity: usize,
    texture_identity: usize,
    normal_identity: usize,
    emissive_identity: usize,
    specular_identity: usize,
    gloss_identity: usize,
}
impl shader::Program<super::Message> for Scene {
    type State = super::view_navigation::Navigation;
    fn update(
        &self,
        state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<shader::Action<super::Message>> {
        state
            .handle(event, bounds, cursor, self.shift)
            .map(super::view_navigation::message)
            .map(|message| shader::Action::publish(message).and_capture())
    }
    type Primitive = Primitive;
    fn draw(
        &self,
        _state: &Self::State,
        _cursor: mouse::Cursor,
        bounds: Rectangle,
    ) -> Self::Primitive {
        let data = &self.mesh;
        let mut low = [f32::INFINITY; 3];
        let mut high = [f32::NEG_INFINITY; 3];
        for point in &data.positions {
            for c in 0..3 {
                low[c] = low[c].min(point[c]);
                high[c] = high[c].max(point[c]);
            }
        }
        let center = [
            (low[0] + high[0]) * 0.5,
            (low[1] + high[1]) * 0.5,
            (low[2] + high[2]) * 0.5,
        ];
        let extent = (0..3).map(|i| high[i] - low[i]).fold(1e-6f32, f32::max);
        let (sy, cy) = self.yaw.to_radians().sin_cos();
        let (sp, cp) = self.pitch.to_radians().sin_cos();
        let ratio = (bounds.width.max(1.) / bounds.height.max(1.)).max(0.01);
        let mut points = Vec::with_capacity(data.positions.len());
        for (i, raw) in data.positions.iter().enumerate() {
            let x = (raw[0] - center[0]) / extent;
            let y = (raw[1] - center[1]) / extent;
            let z = (raw[2] - center[2]) / extent;
            let xr = x * cy - y * sy;
            let back = x * sy + y * cy;
            let yr = z * cp - back * sp;
            let depth = z * sp + back * cp;
            // 3D perspective and depth test, with original source UV winding.
            let perspective = (2.5 - depth).max(0.12);
            let scale = self.zoom.clamp(0.2, 4.0) * 1.6;
            let projected = [
                xr * scale / perspective / ratio + self.pan_x,
                yr * scale / perspective + self.pan_y,
                ((2.5 - depth) / 5.0).clamp(0., 1.),
            ];
            let mut normal = data
                .normals
                .as_ref()
                .and_then(|v| v.get(i))
                .copied()
                .unwrap_or([0., 0., 0.]); // fragment shader derives face normal from geometry
            let nx = normal[0] * cy - normal[1] * sy;
            let nb = normal[0] * sy + normal[1] * cy;
            normal = [nx, normal[2] * cp - nb * sp, normal[2] * sp + nb * cp];
            points.push(Vertex {
                xyz: projected,
                uv: data
                    .uv0
                    .as_ref()
                    .and_then(|v| v.get(i))
                    .copied()
                    .unwrap_or([0., 0.]),
                normal,
                view_position: [xr, yr, depth],
                light_strength: self.light_strength.clamp(0.1, 2.0),
            });
        }
        // Generate a perspective/depth-correct grid beneath the actual
        // lowest vertex. CPU preview math is identical for 2012 and Stage 2.
        let scale = self.zoom.clamp(0.2, 4.0) * 1.6;
        let mut ground = Vec::with_capacity(164);
        for line in super::viewer_grid::floor_lines(&data.positions) {
            for raw in [line.start, line.end] {
                let x = (raw[0] - center[0]) / extent;
                let y = (raw[1] - center[1]) / extent;
                let z = (raw[2] - center[2]) / extent;
                let xr = x * cy - y * sy;
                let back = x * sy + y * cy;
                let yr = z * cp - back * sp;
                let depth = z * sp + back * cp;
                let perspective = (2.5 - depth).max(0.12);
                ground.push(GroundVertex {
                    position: [
                        xr * scale / perspective / ratio + self.pan_x,
                        yr * scale / perspective + self.pan_y,
                        ((2.5 - depth) / 5.0).clamp(0., 1.),
                    ],
                    rgba: line.color,
                });
            }
        }
        Primitive {
            points,
            ground,
            indices: data.indices.clone(),
            surfaces: self.surfaces.clone(),
            diffuse: self.diffuse.clone(),
            normal_map: self.normal_map.clone(),
            emissive: self.emissive.clone(),
            specular: self.specular.clone(),
            gloss: self.gloss.clone(),
            mesh_identity: Arc::as_ptr(&self.mesh) as usize,
            texture_identity: self.diffuse.as_ref().map_or(0, |t| Arc::as_ptr(t) as usize),
            normal_identity: self
                .normal_map
                .as_ref()
                .map_or(0, |t| Arc::as_ptr(t) as usize),
            emissive_identity: self
                .emissive
                .as_ref()
                .map_or(0, |t| Arc::as_ptr(t) as usize),
            specular_identity: self
                .specular
                .as_ref()
                .map_or(0, |t| Arc::as_ptr(t) as usize),
            gloss_identity: self.gloss.as_ref().map_or(0, |t| Arc::as_ptr(t) as usize),
        }
    }
}

struct GpuSurface {
    range: std::ops::Range<u32>,
    transparent: bool,
    opacity_map: bool,
    bind_group: wgpu::BindGroup,
    // Bind groups must retain the original uploaded texture allocations.
    _textures: Vec<wgpu::Texture>,
}

pub struct Pipeline {
    render_pipeline: wgpu::RenderPipeline,
    translucent_pipeline: wgpu::RenderPipeline,
    opacity_pipeline: wgpu::RenderPipeline,
    ground_pipeline: wgpu::RenderPipeline,
    ground_vertex: wgpu::Buffer,
    ground_capacity: usize,
    ground_count: u32,
    layout: wgpu::BindGroupLayout,
    vertex: wgpu::Buffer,
    index: wgpu::Buffer,
    index_count: u32,
    vertex_capacity: usize,
    index_capacity: usize,
    last_mesh_identity: usize,
    last_texture_identity: usize,
    last_normal_identity: usize,
    last_emissive_identity: usize,
    last_specular_identity: usize,
    last_gloss_identity: usize,
    last_surfaces_identity: usize,
    gpu_surfaces: Vec<GpuSurface>,
    surface_table_valid: bool,
    specular_texture: wgpu::Texture,
    specular_view: wgpu::TextureView,
    gloss_texture: wgpu::Texture,
    gloss_view: wgpu::TextureView,
    opacity_texture: wgpu::Texture,
    opacity_view: wgpu::TextureView,
    normal_texture: wgpu::Texture,
    normal_view: wgpu::TextureView,
    emissive_texture: wgpu::Texture,
    emissive_view: wgpu::TextureView,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    group: wgpu::BindGroup,
    depth: wgpu::TextureView,
    depth_dimensions: (u32, u32),
}

impl Pipeline {
    fn depth_view(device: &wgpu::Device, size: (u32, u32)) -> wgpu::TextureView {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Evolve scene depth"),
                size: wgpu::Extent3d {
                    width: size.0.max(1),
                    height: size.1.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default())
    }
    fn upload_map(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: Option<&Arc<image::RgbaImage>>,
        fallback: &[u8; 4],
        format: wgpu::TextureFormat,
        label: &'static str,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let (width, height, bytes) = if let Some(source) = source {
            (source.width(), source.height(), source.as_raw().as_slice())
        } else {
            (1, 1, &fallback[..])
        };
        let texture = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: width.max(1),
                    height: height.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            TextureDataOrder::LayerMajor,
            bytes,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }
    fn new_impl(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Evolve scene diffuse bind layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let program = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("CryEngine DDS textured mesh shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("textured.wgsl"))),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Evolve 3D material pipeline layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Evolve textured source geometry"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &program,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::descriptor()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &program,
                entry_point: Some("fs_opaque"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let translucent_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Evolve authored transparent material"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &program,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::descriptor()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &program,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let opacity_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Evolve explicit authored opacity DDS material"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &program,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::descriptor()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &program,
                entry_point: Some("fs_opacity"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let ground_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Model-floor untextured line layout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });
        let ground_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Blender-style source-relative ground grid"),
            layout: Some(&ground_layout),
            vertex: wgpu::VertexState {
                module: &program,
                entry_point: Some("vs_floor"),
                buffers: &[GroundVertex::descriptor()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &program,
                entry_point: Some("fs_floor"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let ground_vertex = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Viewport model-floor vertices"),
            size: 32,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("CryEngine filtered diffuse sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let placeholder = [210u8, 205, 205, 255];
        let texture = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("Source fallback diffuse texture"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            TextureDataOrder::LayerMajor,
            &placeholder,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        // Linear normal maps, sRGB emissive color maps; no speculative texture matching.
        let (normal_texture, normal_view) = Self::upload_map(
            device,
            queue,
            None,
            &[128, 128, 255, 255],
            wgpu::TextureFormat::Rgba8Unorm,
            "Neutral surface normal",
        );
        let (emissive_texture, emissive_view) = Self::upload_map(
            device,
            queue,
            None,
            &[0, 0, 0, 255],
            wgpu::TextureFormat::Rgba8UnormSrgb,
            "No source emission",
        );
        let (specular_texture, specular_view) = Self::upload_map(
            device,
            queue,
            None,
            &[0, 0, 0, 255],
            wgpu::TextureFormat::Rgba8UnormSrgb,
            "No authored specular reflectance",
        );
        let (gloss_texture, gloss_view) = Self::upload_map(
            device,
            queue,
            None,
            &[128, 128, 128, 255],
            wgpu::TextureFormat::Rgba8Unorm,
            "Neutral unverified gloss",
        );
        let (opacity_texture, opacity_view) = Self::upload_map(
            device,
            queue,
            None,
            &[255, 255, 255, 255],
            wgpu::TextureFormat::Rgba8Unorm,
            "Neutral authored opacity",
        );
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Evolve actual texture sampler group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&normal_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&emissive_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&specular_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&gloss_view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&opacity_view),
                },
            ],
        });
        let vertex = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Evolve source vertex buffer"),
            size: 32,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let index = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Evolve source triangle indices"),
            size: 4,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let depth = Self::depth_view(device, (1, 1));
        Self {
            render_pipeline,
            translucent_pipeline,
            opacity_pipeline,
            ground_pipeline,
            ground_vertex,
            ground_capacity: 32,
            ground_count: 0,
            layout,
            vertex,
            index,
            index_count: 0,
            vertex_capacity: 32,
            index_capacity: 4,
            last_mesh_identity: 0,
            last_texture_identity: usize::MAX,
            last_normal_identity: usize::MAX,
            last_emissive_identity: usize::MAX,
            last_specular_identity: usize::MAX,
            last_gloss_identity: usize::MAX,
            last_surfaces_identity: usize::MAX,
            gpu_surfaces: Vec::new(),
            surface_table_valid: false,
            specular_texture,
            specular_view,
            gloss_texture,
            gloss_view,
            opacity_texture,
            opacity_view,
            normal_texture,
            normal_view,
            emissive_texture,
            emissive_view,
            texture,
            view,
            sampler,
            group,
            depth,
            depth_dimensions: (1, 1),
        }
    }
    fn upload_surfaces(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surfaces: Option<&Arc<Vec<Surface>>>,
        index_count: usize,
    ) {
        let identity = surfaces.map_or(0, |parts| Arc::as_ptr(parts) as usize);
        if identity == self.last_surfaces_identity {
            return;
        }
        self.last_surfaces_identity = identity;
        self.gpu_surfaces.clear();
        self.surface_table_valid = false;
        let Some(surfaces) = surfaces else { return };
        // Validate the complete contiguous index coverage before rendering;
        // an incomplete table must never paint unknown faces with a random map.
        let mut cursor = 0u32;
        for surface in surfaces.iter() {
            if surface.first_index != cursor
                || surface.index_count == 0
                || surface.index_count % 3 != 0
            {
                return;
            }
            let Some(end) = cursor.checked_add(surface.index_count) else {
                return;
            };
            if usize::try_from(end)
                .ok()
                .is_none_or(|end| end > index_count)
            {
                return;
            }
            cursor = end;
        }
        if usize::try_from(cursor).ok() != Some(index_count) {
            return;
        }
        self.surface_table_valid = true;
        for part in surfaces.iter() {
            if !part.visible {
                continue;
            }
            let inputs = [
                (
                    part.diffuse.as_ref(),
                    [210, 205, 205, 255],
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    "CDF part diffuse",
                ),
                (
                    part.normal.as_ref(),
                    [128, 128, 255, 255],
                    wgpu::TextureFormat::Rgba8Unorm,
                    "CDF part normal",
                ),
                (
                    part.emissive.as_ref(),
                    [0, 0, 0, 255],
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    "CDF part emission",
                ),
                (
                    part.specular.as_ref(),
                    [0, 0, 0, 255],
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    "CDF part specular",
                ),
                (
                    part.gloss.as_ref(),
                    [128, 128, 128, 255],
                    wgpu::TextureFormat::Rgba8Unorm,
                    "CDF part gloss",
                ),
                (
                    part.opacity.as_ref(),
                    [255, 255, 255, 255],
                    wgpu::TextureFormat::Rgba8Unorm,
                    "CDF authored opacity",
                ),
            ];
            let mut textures = Vec::with_capacity(6);
            let mut views = Vec::with_capacity(6);
            for (image, fallback, format, label) in inputs {
                let (texture, view) =
                    Self::upload_map(device, queue, image, &fallback, format, label);
                textures.push(texture);
                views.push(view);
            }
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Exact CDF attachment material"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&views[0]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&views[1]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&views[2]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(&views[3]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(&views[4]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::TextureView(&views[5]),
                    },
                ],
            });
            self.gpu_surfaces.push(GpuSurface {
                range: part.first_index..part.first_index + part.index_count,
                transparent: part.transparent,
                opacity_map: part.opacity.is_some(),
                bind_group: group,
                _textures: textures,
            });
        }
    }

    fn prepare_scene(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: (u32, u32),
        primitive: &Primitive,
    ) {
        if self.depth_dimensions != size {
            self.depth = Self::depth_view(device, size);
            self.depth_dimensions = size;
        }
        let floor_bytes = bytemuck::cast_slice(&primitive.ground);
        self.ground_count = primitive.ground.len() as u32;
        if !floor_bytes.is_empty() {
            if floor_bytes.len() > self.ground_capacity {
                self.ground_vertex = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("CryEngine model-foot grid buffer"),
                    size: floor_bytes.len() as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                self.ground_capacity = floor_bytes.len();
            }
            queue.write_buffer(&self.ground_vertex, 0, floor_bytes);
        }
        self.upload_surfaces(
            device,
            queue,
            primitive.surfaces.as_ref(),
            primitive.indices.len(),
        );
        if !primitive.points.is_empty() && !primitive.indices.is_empty() {
            let vertex_bytes = bytemuck::cast_slice(&primitive.points);
            if vertex_bytes.len() > self.vertex_capacity {
                self.vertex = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("CryEngine source GPU vertex stream"),
                    size: vertex_bytes.len() as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                self.vertex_capacity = vertex_bytes.len();
            }
            queue.write_buffer(&self.vertex, 0, vertex_bytes);
            self.index_count = primitive.indices.len() as u32;
            if primitive.mesh_identity != self.last_mesh_identity {
                let bytes = bytemuck::cast_slice(&primitive.indices);
                if bytes.len() > self.index_capacity {
                    self.index = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Unchanged CryEngine triangle index stream"),
                        contents: bytes,
                        // Switching between a 2012 mesh and Stage 2 mesh
                        // may reuse this buffer. Keep COPY_DST for uploads.
                        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                    });
                    self.index_capacity = bytes.len();
                } else {
                    queue.write_buffer(&self.index, 0, bytes);
                }
                self.last_mesh_identity = primitive.mesh_identity;
            }
        } else {
            self.index_count = 0;
        }
        if primitive.texture_identity != self.last_texture_identity
            || primitive.normal_identity != self.last_normal_identity
            || primitive.emissive_identity != self.last_emissive_identity
            || primitive.specular_identity != self.last_specular_identity
            || primitive.gloss_identity != self.last_gloss_identity
        {
            let (w, h, bytes) = if let Some(image) = primitive.diffuse.as_ref() {
                (image.width(), image.height(), image.as_raw().as_slice())
            } else {
                (1, 1, &[210u8, 205, 205, 255][..])
            };
            self.texture = device.create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: Some("Original highest-resolution DDS GPU texture"),
                    size: wgpu::Extent3d {
                        width: w.max(1),
                        height: h.max(1),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                },
                TextureDataOrder::LayerMajor,
                bytes,
            );
            self.view = self
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            (self.normal_texture, self.normal_view) = Self::upload_map(
                device,
                queue,
                primitive.normal_map.as_ref(),
                &[128, 128, 255, 255],
                wgpu::TextureFormat::Rgba8Unorm,
                "Source tangent-space normal map",
            );
            (self.emissive_texture, self.emissive_view) = Self::upload_map(
                device,
                queue,
                primitive.emissive.as_ref(),
                &[0, 0, 0, 255],
                wgpu::TextureFormat::Rgba8UnormSrgb,
                "Source emission mask",
            );
            (self.specular_texture, self.specular_view) = Self::upload_map(
                device,
                queue,
                primitive.specular.as_ref(),
                &[0, 0, 0, 255],
                wgpu::TextureFormat::Rgba8UnormSrgb,
                "Verified authored specular color",
            );
            (self.gloss_texture, self.gloss_view) = Self::upload_map(
                device,
                queue,
                primitive.gloss.as_ref(),
                &[128, 128, 128, 255],
                wgpu::TextureFormat::Rgba8Unorm,
                "Verified authored separate gloss",
            );
            self.group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Exact CryEngine DDS texture sampler group"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&self.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&self.normal_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&self.emissive_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(&self.specular_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(&self.gloss_view),
                    },
                ],
            });
            self.last_texture_identity = primitive.texture_identity;
            self.last_normal_identity = primitive.normal_identity;
            self.last_emissive_identity = primitive.emissive_identity;
            self.last_specular_identity = primitive.specular_identity;
            self.last_gloss_identity = primitive.gloss_identity;
        }
    }
    fn render_scene(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip: Rectangle<u32>,
    ) {
        if self.index_count == 0 || clip.width == 0 || clip.height == 0 {
            return;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Evolve source DDS mesh render pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_viewport(
            clip.x as f32,
            clip.y as f32,
            clip.width as f32,
            clip.height as f32,
            0.,
            1.,
        );
        pass.set_scissor_rect(clip.x, clip.y, clip.width, clip.height);
        if self.ground_count > 0 {
            pass.set_pipeline(&self.ground_pipeline);
            pass.set_vertex_buffer(0, self.ground_vertex.slice(..));
            pass.draw(0..self.ground_count, 0..1);
        }
        pass.set_pipeline(&self.render_pipeline);
        pass.set_vertex_buffer(0, self.vertex.slice(..));
        pass.set_index_buffer(self.index.slice(..), wgpu::IndexFormat::Uint32);
        if self.surface_table_valid {
            // NoDraw-only characters intentionally have zero visible parts.
            // Do not silently fall back to rendering the whole mesh.
            // Write the depth of every opaque original face before any
            // transparent section; authoring order is not render order.
            for surface in self.gpu_surfaces.iter().filter(|part| !part.transparent) {
                pass.set_pipeline(&self.render_pipeline);
                pass.set_bind_group(0, &surface.bind_group, &[]);
                pass.draw_indexed(surface.range.clone(), 0, 0..1);
            }
            for surface in self.gpu_surfaces.iter().filter(|part| part.transparent) {
                pass.set_pipeline(if surface.opacity_map {
                    &self.opacity_pipeline
                } else {
                    &self.translucent_pipeline
                });
                pass.set_bind_group(0, &surface.bind_group, &[]);
                pass.draw_indexed(surface.range.clone(), 0, 0..1);
            }
        } else {
            pass.set_bind_group(0, &self.group, &[]);
            pass.draw_indexed(0..self.index_count, 0, 0..1);
        }
    }
}
impl shader::Pipeline for Pipeline {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        Self::new_impl(device, queue, format)
    }
}
impl shader::Primitive for Primitive {
    type Pipeline = Pipeline;
    fn prepare(
        &self,
        pipeline: &mut Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _bounds: &Rectangle,
        viewport: &Viewport,
    ) {
        let target = viewport.physical_size();
        pipeline.prepare_scene(device, queue, (target.width, target.height), self);
    }
    fn render(
        &self,
        pipeline: &Pipeline,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        pipeline.render_scene(encoder, target, *clip_bounds);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn cryengine_gpu_material_shader_parses_and_validates() {
        let module = naga::front::wgsl::parse_str(include_str!("textured.wgsl"))
            .expect("Material WGSL must parse");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("GPU normal/emissive shader must pass Naga validation");
        assert!(
            module
                .entry_points
                .iter()
                .any(|entry| entry.name == "fs_opacity")
        );
        assert!(
            module
                .entry_points
                .iter()
                .any(|entry| entry.name == "fs_main")
        );
        assert!(
            module
                .entry_points
                .iter()
                .any(|entry| entry.name == "fs_opaque")
        );
    }
}
