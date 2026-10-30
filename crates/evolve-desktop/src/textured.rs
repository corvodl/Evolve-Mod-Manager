//! Real GPU triangle pipeline for validated CryEngine source mesh streams.
//! Samples user-selected original DDS diffuse pixels against source UV0; it
//! never guesses cross-PAK materials or fabricates a complete character rig.
use evolve_core::RenderMesh;
use iced::wgpu;
use iced::wgpu::util::{DeviceExt, TextureDataOrder};
use iced::widget::shader::{self, Viewport};
use iced::{Event, Rectangle, mouse};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

// A large CDF can reuse the same original DDS in dozens of material ranges.
// Allocate each source/role only once per scene and bound unique GPU copies.
pub(crate) const MAX_GPU_PREVIEW_TEXTURE_BYTES: usize = 256 * 1024 * 1024;

fn texture_cache_key(image: Option<&Arc<image::RgbaImage>>, role: usize) -> (usize, usize) {
    (image.map_or(0, |pixels| Arc::as_ptr(pixels) as usize), role)
}

fn gpu_budget_allows(used: usize, additional: usize) -> bool {
    used.checked_add(additional)
        .is_some_and(|total| total <= MAX_GPU_PREVIEW_TEXTURE_BYTES)
}

/// Reserve GPU space for ALL source diffuse maps before accepting optional
/// normal/specular/glow uploads. Without this, early body sections consume
/// the cap and later head/arm sections silently fall back to pale placeholders.
fn allows_cdf_role(
    used: usize,
    additional: usize,
    remaining_diffuse: usize,
    role: usize,
    can_fit_all_diffuse: bool,
) -> bool {
    if role != 0 && !can_fit_all_diffuse {
        return false;
    }
    let reserve = if role == 0 { 0 } else { remaining_diffuse };
    gpu_budget_allows(used.saturating_add(reserve), additional)
}

#[derive(Clone, Debug)]
pub struct Surface {
    pub first_index: u32,
    pub index_count: u32,
    /// Explicit CryEngine NoDraw subset. Preserve its index span, but do not render it.
    pub visible: bool,
    pub transparent: bool,
    /// Vegetation shader diffuse alpha is an alpha-tested cutout with depth writes.
    /// The separate authored Opacity texture is foliage transmittance, not coverage.
    pub foliage_cutout: bool,
    /// Original per-leaf AlphaTest threshold (normalized). Default preview 0.5.
    pub alpha_cutoff: f32,
    pub diffuse: Option<Arc<image::RgbaImage>>,
    pub normal: Option<Arc<image::RgbaImage>>,
    pub emissive: Option<Arc<image::RgbaImage>>,
    /// Optional Stage 2 *_glow_diff overlay, never the source's Emittance.
    pub armor_emissive: Option<Arc<image::RgbaImage>>,
    /// Exact per-leaf authored GlowAmount, not a guessed percentage.
    pub emission_strength: f32,
    /// Source-verified Kala tubing state adjusts only the GPU preview;
    /// original GlowAmount and exported assets remain untouched.
    pub kala_emission_state: Option<evolve_core::KalaTubingState>,
    /// Authored moving Decal UVs used only for this material's emission.
    pub emissive_uv: Option<evolve_core::DecalTexMod>,
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
    /// Viewport-only multiplier, independent of the original material strength.
    pub emissive_gain: f32,
    /// Continuous glow animation clock, independent of CAF playback.
    pub emissive_time: f32,
    pub pan_x: f32,
    pub pan_y: f32,
    pub shift: bool,
    pub level_camera: Option<super::level_camera::Camera>,
    pub world: Option<Arc<super::world_instances::World>>,
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
    emissive_gain: f32,
    emissive_time: f32,
    world: Option<Arc<super::world_instances::World>>,
    camera: Option<super::level_camera::Camera>,
    yaw: f32,
    pitch: f32,
    zoom: f32,
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
            let scale = self.zoom.clamp(0.2, 80.0) * 1.6;
            let projected = if let Some(camera) = self.level_camera {
                camera.project(*raw, self.yaw, self.pitch, ratio, self.zoom)
            } else {
                [
                    xr * scale / perspective / ratio + self.pan_x,
                    yr * scale / perspective + self.pan_y,
                    ((2.5 - depth) / 5.0).clamp(0., 1.),
                ]
            };
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
                light_strength: self.light_strength.clamp(0.2, 4.0),
            });
        }
        // Generate a perspective/depth-correct grid beneath the actual
        // lowest vertex. CPU preview math is identical for 2012 and Stage 2.
        let scale = self.zoom.clamp(0.2, 80.0) * 1.6;
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
                let position = if let Some(camera) = self.level_camera {
                    camera.project(raw, self.yaw, self.pitch, ratio, self.zoom)
                } else {
                    [
                        xr * scale / perspective / ratio + self.pan_x,
                        yr * scale / perspective + self.pan_y,
                        ((2.5 - depth) / 5.0).clamp(0., 1.),
                    ]
                };
                ground.push(GroundVertex {
                    position,
                    rgba: line.color,
                });
            }
        }
        Primitive {
            // In level mode the GPU world renderer clips actual world-space
            // triangles against the camera near plane. Do not draw a second
            // copy of terrain through legacy CPU NDC projection: it turns
            // triangles crossing the camera into giant apparent surfaces.
            points: if self.level_camera.is_some() {
                Vec::new()
            } else {
                points
            },
            ground: if self.level_camera.is_some() {
                Vec::new()
            } else {
                ground
            },
            indices: if self.level_camera.is_some() {
                Vec::new()
            } else {
                data.indices.clone()
            },
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
            emissive_gain: self.emissive_gain,
            emissive_time: self.emissive_time,
            world: self.world.clone(),
            camera: self.level_camera,
            yaw: self.yaw,
            pitch: self.pitch,
            zoom: self.zoom,
        }
    }
}

impl Primitive {
    /// Level-editor-only axis guides in the existing depth-buffered line pass.
    /// Coordinates are projected NDC just like the ground grid.
    pub(super) fn add_level_axis(&mut self, a: [f32; 3], b: [f32; 3], color: [f32; 4]) {
        self.ground.push(GroundVertex {
            position: a,
            rgba: color,
        });
        self.ground.push(GroundVertex {
            position: b,
            rgba: color,
        });
    }
}

struct GpuSurface {
    range: std::ops::Range<u32>,
    transparent: bool,
    masked: bool,
    foliage_cutout: bool,
    opacity_map: bool,
    bind_group: wgpu::BindGroup,
    _emission_uniform: wgpu::Buffer,
    _uv_uniform: wgpu::Buffer,
    source_emission_strength: f32,
    alpha_cutoff: f32,
    kala_emission_state: Option<evolve_core::KalaTubingState>,
    emissive_uv: Option<evolve_core::DecalTexMod>,
}

pub struct Pipeline {
    opaque_viewport_background: wgpu::RenderPipeline,
    world_renderer: super::world_instances::Renderer,
    render_pipeline: wgpu::RenderPipeline,
    masked_pipeline: wgpu::RenderPipeline,
    foliage_pipeline: wgpu::RenderPipeline,
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
    last_emissive_gain: f32,
    last_emissive_time: f32,
    gpu_surfaces: Vec<GpuSurface>,
    // Own every unique GPU source allocation while its bind groups exist.
    _surface_textures: Vec<wgpu::Texture>,
    surface_table_valid: bool,
    specular_texture: wgpu::Texture,
    specular_view: wgpu::TextureView,
    gloss_texture: wgpu::Texture,
    gloss_view: wgpu::TextureView,
    // Keep the neutral opacity allocation alive for the lifetime of the bind group.
    _opacity_texture: wgpu::Texture,
    _opacity_view: wgpu::TextureView,
    normal_texture: wgpu::Texture,
    normal_view: wgpu::TextureView,
    emissive_texture: wgpu::Texture,
    emissive_view: wgpu::TextureView,
    _armor_emissive_texture: wgpu::Texture,
    _armor_emissive_view: wgpu::TextureView,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    group: wgpu::BindGroup,
    _emission_uniform: wgpu::Buffer,
    _uv_uniform: wgpu::Buffer,
    depth: wgpu::TextureView,
    depth_dimensions: (u32, u32),
}

// Every runtime material bind group must follow the same schema used by
// textured.wgsl, including the standalone DDS refresh path. Keep this single
// helper for initialization, CDF attachment ranges and texture reloads.
const MATERIAL_BINDINGS: [u32; 10] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9];

/// Apply the earlier public Qt preview's GlowAmount 3 -> 100 reference to
/// every source-authored emissive material while retaining its GlowAmount.
/// Kala's active equipment glow keeps its separately verified 0.3x/4x
/// correction. The Armor Reducer reference is 2x after the user verified
/// Nightly 13 requires only half of the old 4x multiplier at 1x slider.
/// The user slider remains independent of both factors.
fn emission_uniform_values(
    source_strength: f32,
    preview_gain: f32,
    kala_emission_state: Option<evolve_core::KalaTubingState>,
) -> [f32; 4] {
    let source = if source_strength.is_finite() {
        source_strength.clamp(0.0, 100.0)
    } else {
        0.0
    };
    let gain = if preview_gain.is_finite() {
        preview_gain.clamp(0.0, 4.0)
    } else {
        1.0
    };
    let state_factor = match kala_emission_state {
        Some(evolve_core::KalaTubingState::Banshee) => 0.3,
        Some(evolve_core::KalaTubingState::ArmorReducer) => 2.0,
        None => 1.0,
    };
    let calibration = (100.0 / 3.0) * state_factor;
    [source, gain, calibration, 0.0]
}

/// Source UV tiles and one texture-coordinate translation per axis.
/// Type 2 uses constant motion rather than a sine oscillation.
fn emissive_uv_values(uv: Option<evolve_core::DecalTexMod>, seconds: f32) -> [f32; 4] {
    uv.map_or([1.0, 1.0, 0.0, 0.0], |value| value.sample(seconds))
}

fn material_entries<'a>(
    views: [&'a wgpu::TextureView; 7],
    sampler: &'a wgpu::Sampler,
    emission_uniform: &'a wgpu::Buffer,
    uv_uniform: &'a wgpu::Buffer,
) -> [wgpu::BindGroupEntry<'a>; MATERIAL_BINDINGS.len()] {
    let texture = |binding: usize, channel: usize| wgpu::BindGroupEntry {
        binding: MATERIAL_BINDINGS[binding],
        resource: wgpu::BindingResource::TextureView(views[channel]),
    };
    [
        texture(0, 0), // diffuse
        wgpu::BindGroupEntry {
            binding: MATERIAL_BINDINGS[1],
            resource: wgpu::BindingResource::Sampler(sampler),
        },
        texture(2, 1), // normal
        texture(3, 2), // emissive
        texture(4, 3), // specular
        texture(5, 4), // gloss
        texture(6, 5), // authored opacity or neutral white
        wgpu::BindGroupEntry {
            binding: MATERIAL_BINDINGS[7],
            resource: emission_uniform.as_entire_binding(),
        },
        texture(8, 6), // optional armor glow, not original Emittance
        wgpu::BindGroupEntry {
            binding: MATERIAL_BINDINGS[9],
            resource: uv_uniform.as_entire_binding(),
        },
    ]
}

impl Pipeline {
    fn emission_uniform(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        strength: f32,
        kala_emission_state: Option<evolve_core::KalaTubingState>,
        alpha_cutoff: f32,
    ) -> wgpu::Buffer {
        // Retain the original GlowAmount as source multiplier, with the
        // legacy Qt 3 => 100 visualization reference in a separate uniform.
        let mut values = emission_uniform_values(strength, 1.0, kala_emission_state);
        // vec4.w is unused by the emission shader: keep the source
        // AlphaTest threshold here, avoiding a separate bind group/buffer.
        values[3] = alpha_cutoff.clamp(0.0, 1.0);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Source material emission amount"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&buffer, 0, bytemuck::bytes_of(&values));
        buffer
    }

    fn emissive_uv_uniform(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        uv: Option<evolve_core::DecalTexMod>,
    ) -> wgpu::Buffer {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Independent animated Decal emission UV transform"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&buffer, 0, bytemuck::bytes_of(&emissive_uv_values(uv, 0.0)));
        buffer
    }

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
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
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
        // Alpha-tested solid body surfaces participate in the depth pass.
        // The authored opacity texture clips actual holes, without blending
        // intermediate mask values with the background/grid.
        let masked_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Evolve solid character cutout material"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &program,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::descriptor()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &program,
                entry_point: Some("fs_masked"),
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
        let foliage_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Evolve vegetation diffuse-alpha cutout"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &program,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::descriptor()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &program,
                entry_point: Some("fs_foliage"),
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
        // The shader compositor reuses the native window color target.
        // Explicitly paint an opaque rectangle in the *scissored viewport*
        // before any mesh/grid triangles, or alpha-zero/old window content
        // can remain visible behind an otherwise empty level scene.
        let background_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Opaque level viewport backdrop"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(
                "@vertex fn vs_main(@builtin(vertex_index) id:u32)->@builtin(position) vec4<f32> {\n                 var xy=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));\n                 return vec4<f32>(xy[id],0.99999,1.0); }\n                 @fragment fn fs_main()->@location(0) vec4<f32> { return vec4<f32>(0.105,0.117,0.135,1.0); }"
            )),
        });
        let opaque_viewport_background =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Opaque viewport background / prevent compositing bleed-through"),
                layout: Some(&ground_layout),
                vertex: wgpu::VertexState {
                    module: &background_shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &background_shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: false,
                    depth_compare: wgpu::CompareFunction::Always,
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
        let (armor_emissive_texture, armor_emissive_view) = Self::upload_map(
            device,
            queue,
            None,
            &[0, 0, 0, 255],
            wgpu::TextureFormat::Rgba8UnormSrgb,
            "No optional armor glow",
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
        let emission_uniform = Self::emission_uniform(device, queue, 1.0, None, 0.5);
        let uv_uniform = Self::emissive_uv_uniform(device, queue, None);
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Evolve actual texture sampler group"),
            layout: &layout,
            entries: &material_entries(
                [
                    &view,
                    &normal_view,
                    &emissive_view,
                    &specular_view,
                    &gloss_view,
                    &opacity_view,
                    &armor_emissive_view,
                ],
                &sampler,
                &emission_uniform,
                &uv_uniform,
            ),
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
            opaque_viewport_background,
            world_renderer: super::world_instances::Renderer::new(device, format),
            render_pipeline,
            masked_pipeline,
            foliage_pipeline,
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
            last_emissive_gain: f32::NAN,
            last_emissive_time: f32::NAN,
            gpu_surfaces: Vec::new(),
            _surface_textures: Vec::new(),
            surface_table_valid: false,
            specular_texture,
            specular_view,
            gloss_texture,
            gloss_view,
            _opacity_texture: opacity_texture,
            _opacity_view: opacity_view,
            normal_texture,
            normal_view,
            emissive_texture,
            emissive_view,
            _armor_emissive_texture: armor_emissive_texture,
            _armor_emissive_view: armor_emissive_view,
            texture,
            view,
            sampler,
            group,
            _emission_uniform: emission_uniform,
            _uv_uniform: uv_uniform,
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
        // New bind groups require fresh gain uploads even if the slider did
        // not move. Reuse the already-decoded images on subsequent slider drags.
        self.last_emissive_gain = f32::NAN;
        self.last_emissive_time = f32::NAN;
        self.gpu_surfaces.clear();
        self._surface_textures.clear();
        self.surface_table_valid = false;
        let Some(surfaces) = surfaces else { return };
        super::crash_diagnostics::record(
            "gpu_material_upload_begin",
            &format!("{} sections, {} indices", surfaces.len(), index_count),
        );
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
            super::crash_diagnostics::record(
                "gpu_material_coverage_invalid",
                &format!("{} source indices, {} covered", index_count, cursor),
            );
            return;
        }
        self.surface_table_valid = true;
        let mut uploaded: HashMap<(usize, usize), wgpu::TextureView> = HashMap::new();
        let mut used_bytes = 0usize;
        let mut omitted_count = 0usize;
        let mut unique_diffuse = HashSet::new();
        let mut remaining_diffuse = 0usize;
        for part in surfaces.iter().filter(|part| part.visible) {
            if let Some(diffuse) = part.diffuse.as_ref()
                && unique_diffuse.insert(Arc::as_ptr(diffuse) as usize)
            {
                remaining_diffuse = remaining_diffuse.saturating_add(diffuse.as_raw().len());
            }
        }
        let can_fit_all_diffuse = remaining_diffuse <= MAX_GPU_PREVIEW_TEXTURE_BYTES;
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
                (
                    part.armor_emissive.as_ref(),
                    [0, 0, 0, 255],
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    "Optional monster armor emission",
                ),
            ];
            let mut views = Vec::with_capacity(7);
            let mut opacity_uploaded = false;
            let mut diffuse_uploaded = false;
            for (role, (image, fallback, format, label)) in inputs.into_iter().enumerate() {
                // A DDS may be shared by many assembly sections. View/texture
                // identities are source-arc + GPU role (color-space aware).
                let size = image.map_or(4, |pixels| pixels.as_raw().len());
                let key = texture_cache_key(image, role);
                let already_uploaded = uploaded.contains_key(&key);
                let allow_source = image.is_none()
                    || already_uploaded
                    || allows_cdf_role(
                        used_bytes,
                        size,
                        remaining_diffuse,
                        role,
                        can_fit_all_diffuse,
                    );
                let selected = if allow_source { image } else { None };
                if role == 0 && selected.is_some() && !already_uploaded {
                    remaining_diffuse = remaining_diffuse.saturating_sub(size);
                }
                if !allow_source {
                    omitted_count += 1;
                }
                let selected_key = texture_cache_key(selected, role);
                if let Some(view) = uploaded.get(&selected_key) {
                    views.push(view.clone());
                } else {
                    let (texture, view) =
                        Self::upload_map(device, queue, selected, &fallback, format, label);
                    // The fallback is a four-byte neutral placeholder; large
                    // unverified maps are omitted instead of risking GPU OOM.
                    used_bytes = used_bytes
                        .saturating_add(selected.map_or(4, |pixels| pixels.as_raw().len()));
                    uploaded.insert(selected_key, view.clone());
                    self._surface_textures.push(texture);
                    views.push(view);
                }
                if role == 0 && selected.is_some() {
                    diffuse_uploaded = true;
                }
                if role == 5 && selected.is_some() {
                    opacity_uploaded = true;
                }
            }
            let emission_uniform = Self::emission_uniform(
                device,
                queue,
                part.emission_strength,
                part.kala_emission_state,
                part.alpha_cutoff,
            );
            let uv_uniform = Self::emissive_uv_uniform(device, queue, part.emissive_uv);
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Exact CDF attachment material"),
                layout: &self.layout,
                entries: &material_entries(
                    [
                        &views[0], &views[1], &views[2], &views[3], &views[4], &views[5], &views[6],
                    ],
                    &self.sampler,
                    &emission_uniform,
                    &uv_uniform,
                ),
            });
            self.gpu_surfaces.push(GpuSurface {
                range: part.first_index..part.first_index + part.index_count,
                transparent: part.transparent && (part.opacity.is_none() || opacity_uploaded),
                masked: !part.transparent && opacity_uploaded && !part.foliage_cutout,
                foliage_cutout: part.foliage_cutout && diffuse_uploaded,
                opacity_map: opacity_uploaded,
                bind_group: group,
                _emission_uniform: emission_uniform,
                _uv_uniform: uv_uniform,
                source_emission_strength: part.emission_strength,
                alpha_cutoff: part.alpha_cutoff,
                kala_emission_state: part.kala_emission_state,
                emissive_uv: part.emissive_uv,
            });
        }
        super::crash_diagnostics::record(
            "gpu_material_upload_complete",
            &format!(
                "{} visible sections, {} unique GPU maps, {} bytes, {} budget omissions",
                self.gpu_surfaces.len(),
                self._surface_textures.len(),
                used_bytes,
                omitted_count
            ),
        );
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
        self.world_renderer.prepare(
            device,
            queue,
            primitive.world.as_ref(),
            primitive.camera,
            primitive.yaw,
            primitive.pitch,
            size.0 as f32 / size.1.max(1) as f32,
            primitive.zoom,
        );
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
        if self.last_emissive_time.to_bits() != primitive.emissive_time.to_bits() {
            self.last_emissive_time = primitive.emissive_time;
            for surface in &self.gpu_surfaces {
                if surface.emissive_uv.is_some() {
                    let transform =
                        emissive_uv_values(surface.emissive_uv, primitive.emissive_time);
                    queue.write_buffer(&surface._uv_uniform, 0, bytemuck::bytes_of(&transform));
                }
            }
        }
        if self.last_emissive_gain.to_bits() != primitive.emissive_gain.to_bits() {
            self.last_emissive_gain = primitive.emissive_gain;
            // Buffers already exist on each material: update only 16 bytes
            // per visible material, not CDF geometry or source DDS textures.
            let default = emission_uniform_values(1.0, primitive.emissive_gain, None);
            queue.write_buffer(&self._emission_uniform, 0, bytemuck::bytes_of(&default));
            for surface in &self.gpu_surfaces {
                let mut values = emission_uniform_values(
                    surface.source_emission_strength,
                    primitive.emissive_gain,
                    surface.kala_emission_state,
                );
                // Gain updates must not reset the foliage material cutoff.
                values[3] = surface.alpha_cutoff.clamp(0.0, 1.0);
                queue.write_buffer(&surface._emission_uniform, 0, bytemuck::bytes_of(&values));
            }
        }
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
                entries: &material_entries(
                    [
                        &self.view,
                        &self.normal_view,
                        &self.emissive_view,
                        &self.specular_view,
                        &self.gloss_view,
                        &self._opacity_view,
                        &self._armor_emissive_view,
                    ],
                    &self.sampler,
                    &self._emission_uniform,
                    &self._uv_uniform,
                ),
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
        if clip.width == 0 || clip.height == 0 {
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
        pass.set_pipeline(&self.opaque_viewport_background);
        pass.draw(0..3, 0..1);
        if self.ground_count > 0 {
            pass.set_pipeline(&self.ground_pipeline);
            pass.set_vertex_buffer(0, self.ground_vertex.slice(..));
            pass.draw(0..self.ground_count, 0..1);
        }
        if self.index_count > 0 {
            pass.set_pipeline(&self.render_pipeline);
            pass.set_vertex_buffer(0, self.vertex.slice(..));
            pass.set_index_buffer(self.index.slice(..), wgpu::IndexFormat::Uint32);
            if self.surface_table_valid {
                // NoDraw-only characters intentionally have zero visible parts.
                // Do not silently fall back to rendering the whole mesh.
                // Write the depth of every opaque original face before any
                // transparent section; authoring order is not render order.
                for surface in self.gpu_surfaces.iter().filter(|part| !part.transparent) {
                    pass.set_pipeline(if surface.foliage_cutout {
                        &self.foliage_pipeline
                    } else if surface.masked {
                        &self.masked_pipeline
                    } else {
                        &self.render_pipeline
                    });
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
        self.world_renderer.draw(&mut pass);
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
    use std::sync::Arc;

    #[test]
    fn shared_source_images_share_one_gpu_upload_key_per_role() {
        let image = Arc::new(image::RgbaImage::new(8, 8));
        let same = Arc::clone(&image);
        assert_eq!(
            super::texture_cache_key(Some(&image), 0),
            super::texture_cache_key(Some(&same), 0)
        );
        assert_ne!(
            super::texture_cache_key(Some(&image), 0),
            super::texture_cache_key(Some(&image), 1)
        );
        assert_eq!(
            super::texture_cache_key(None, 0),
            super::texture_cache_key(None, 0)
        );
    }

    #[test]
    fn preview_upload_budget_rejects_oversized_unique_texture_without_overflow() {
        assert!(super::gpu_budget_allows(0, 256 * 1024 * 1024));
        assert!(!super::gpu_budget_allows(256 * 1024 * 1024, 4));
        assert!(!super::gpu_budget_allows(usize::MAX, 64));
        // With 180 MiB still owed to later original diffuse textures,
        // a new 90 MiB optional specular must NOT crowd them out.
        let mib = 1024 * 1024;
        assert!(!super::allows_cdf_role(0, 90 * mib, 180 * mib, 3, true));
        assert!(super::allows_cdf_role(0, 90 * mib, 180 * mib, 0, true));
        // If all diffuse assets exceed the fixed budget, omit optional
        // maps first while still allowing each original diffuse to compete
        // for the remaining protected budget.
        assert!(!super::allows_cdf_role(0, 4 * mib, 300 * mib, 2, false));
        assert!(super::allows_cdf_role(0, 4 * mib, 300 * mib, 0, false));
    }

    #[test]
    fn independent_armor_emission_does_not_inherit_authored_source_strength() {
        let authored = super::emission_uniform_values(0.0, 2.5, None);
        assert_eq!(authored[0], 0.0);
        assert_eq!(authored[1], 2.5);
        // Non-authored optional armor glow does not inherit an unrelated
        // material's GlowAmount; it uses a neutral 1-unit source reference.
        let source = include_str!("textured.wgsl");
        assert!(source.contains("glow * emission_params.x * emission_params.y"));
        assert!(source.contains("armor_glow * emission_params.y"));
        assert!(source.contains("armor_calibrated = armor_source * (100.0 / 3.0)"));
    }

    #[test]
    fn original_emission_calibration_scales_every_material_glow_amount() {
        for authored in [0.0, 0.3, 1.0, 2.0, 3.0, 4.0, 20.0] {
            let params = super::emission_uniform_values(authored, 1.0, None);
            assert_eq!(params[0], authored);
            assert!((params[0] * params[1] * params[2] - authored * (100.0 / 3.0)).abs() < 0.0001);
        }
        let boosted = super::emission_uniform_values(3.0, 2.0, None);
        assert!((boosted[0] * boosted[1] * boosted[2] - 200.0).abs() < 0.0001);
        let disabled = super::emission_uniform_values(3.0, 0.0, None);
        assert_eq!(disabled[0] * disabled[1] * disabled[2], 0.0);
        let invalid = super::emission_uniform_values(f32::NAN, 1.0, None);
        assert_eq!(invalid[0], 0.0);
    }

    #[test]
    fn emissive_slider_multiplies_source_relative_glow_without_replacing_it() {
        assert_eq!(
            super::emission_uniform_values(3.0, 1.0, None)[..2],
            [3.0, 1.0]
        );
        assert_eq!(
            super::emission_uniform_values(3.0, 2.5, None)[..2],
            [3.0, 2.5]
        );
        assert_eq!(
            super::emission_uniform_values(0.0, 4.0, None)[..2],
            [0.0, 4.0]
        );
        assert_eq!(
            super::emission_uniform_values(3.0, 0.0, None)[..2],
            [3.0, 0.0]
        );
        assert_eq!(
            super::emission_uniform_values(f32::NAN, f32::NAN, None)[..2],
            [0.0, 1.0]
        );
    }

    #[test]
    fn kala_state_preview_gain_matches_user_blue_red_and_source_glow_amount() {
        use evolve_core::KalaTubingState;
        let blue = super::emission_uniform_values(3.0, 1.0, Some(KalaTubingState::Banshee));
        let red = super::emission_uniform_values(3.0, 1.0, Some(KalaTubingState::ArmorReducer));
        assert_eq!(blue[0], 3.0);
        assert_eq!(red[0], 3.0);
        assert_eq!(blue[1], 1.0);
        assert_eq!(red[1], 1.0);
        assert!((blue[0] * blue[2] - 30.0).abs() < 0.0001);
        assert!((red[0] * red[2] - 200.0).abs() < 0.0001);
        let half_blue = super::emission_uniform_values(1.5, 1.0, Some(KalaTubingState::Banshee));
        let half_red =
            super::emission_uniform_values(1.5, 1.0, Some(KalaTubingState::ArmorReducer));
        assert!((half_blue[0] * half_blue[2] - 15.0).abs() < 0.0001);
        assert!((half_red[0] * half_red[2] - 100.0).abs() < 0.0001);
        let old = 100.0 / 3.0;
        assert!((blue[0] * blue[1] * blue[2] - 3.0 * old * 0.3).abs() < 0.0001);
        assert!((red[0] * red[1] * red[2] - 3.0 * old * 2.0).abs() < 0.0001);
        let disabled =
            super::emission_uniform_values(3.0, 0.0, Some(KalaTubingState::ArmorReducer));
        assert_eq!(disabled[0] * disabled[1] * disabled[2], 0.0);
        let zero = super::emission_uniform_values(0.0, 1.0, Some(KalaTubingState::Banshee));
        assert_eq!(zero[0] * zero[2], 0.0);
        let unrelated = super::emission_uniform_values(3.0, 1.0, None);
        assert!((unrelated[0] * unrelated[1] * unrelated[2] - 100.0).abs() < 0.0001);
        let shader = include_str!("textured.wgsl");
        assert!(shader.contains("let viewport_glow = calibrated_glow / (1.0 + peak)"));
        assert!(shader.contains("calibrated_glow / (1.0 + peak)"));
        assert!(shader.contains("armor_glow * emission_params.y"));
    }

    #[test]
    fn animated_kala_uv_does_not_change_regular_mesh_sampler() {
        assert_eq!(super::emissive_uv_values(None, 31.0), [1.0, 1.0, 0.0, 0.0]);
        let authored = evolve_core::DecalTexMod {
            tile_u: 3.0,
            tile_v: 4.0,
            u_type: 2,
            u_rate: 1.0,
            u_amplitude: 0.2,
            v_type: 0,
            v_rate: 0.0,
            v_amplitude: 0.1,
        };
        assert_eq!(
            super::emissive_uv_values(Some(authored), 1.0),
            [3.0, 4.0, 0.2, 0.0]
        );
        assert_eq!(
            super::emissive_uv_values(Some(authored), 2.0),
            [3.0, 4.0, 0.4, 0.0]
        );
        let shader = include_str!("textured.wgsl");
        assert!(shader.contains("input.uv * emissive_uv.xy + emissive_uv.zw"));
        assert!(shader.contains("textureSample(diffuse, material_sampler, input.uv)"));
    }

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
        // Regression for Rocky3: all three GPU material group creation sites
        // use material_entries, whose eight bindings must match source WGSL.
        let mut shader_bindings: Vec<u32> = module
            .global_variables
            .iter()
            .filter_map(|(_, variable)| {
                variable
                    .binding
                    .as_ref()
                    .filter(|binding| binding.group == 0)
                    .map(|binding| binding.binding)
            })
            .collect();
        shader_bindings.sort_unstable();
        assert_eq!(shader_bindings, super::MATERIAL_BINDINGS);
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
                .any(|entry| entry.name == "fs_masked")
        );
        assert!(
            module
                .entry_points
                .iter()
                .any(|entry| entry.name == "fs_foliage")
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
