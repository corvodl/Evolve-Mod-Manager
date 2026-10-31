//! Dedicated level-world GPU renderer. Model geometry is uploaded once per
//! exact virtual source/material pair; authored placements remain a separate
//! instanced vertex stream (not baked into one enormous CPU mesh).
use crate::{level_camera, textured::Surface};
use evolve_core::{RenderMesh, Transform};
use iced::wgpu;
use iced::wgpu::util::DeviceExt;
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::{
    Arc,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};

// Ordinary world-model diffuse textures are limited to 32 MiB so one map
// cannot flood GPU memory with hundreds of oversized props. Baked ground
// color is a single, verified atlas: the original Ice_Crashsite layout is
// 4096×4096 RGBA8 (64 MiB), which must NOT silently fall back to gray.
const MAX_WORLD_DIFFUSE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_TERRAIN_ATLAS_BYTES: u64 = 64 * 1024 * 1024;
const MAX_FULL_TEXTURE_BYTES: u64 = u64::MAX;
const MAX_SCENE_BALANCED_TEXTURE_BYTES: u64 = 384 * 1024 * 1024;
// Upload a bounded number of new groups each redraw. Once uploaded, GPU
// meshes and textures remain resident until the level closes or a deliberate
// quality/scene change rebuilds the renderer. In particular, camera movement
// must never evict already-loaded groups.
const MAX_NEW_GROUPS_PER_FRAME: usize = 2;

fn residency_target_bytes(full_textures: bool) -> u64 {
    if full_textures {
        // Full quality is constrained by the GPU/driver, not an arbitrary
        // software residency cap. Balanced mode retains its lower-memory cap.
        u64::MAX
    } else {
        MAX_SCENE_BALANCED_TEXTURE_BYTES
    }
}
const MAX_MIP_WRITE_BYTES_PER_FRAME: usize = 8 * 1024 * 1024;

// Hash exact source pixel dimensions+bytes, not an Arc address. Distinct
// imported PAK entries containing identical decoded pixels deduplicate while
// their roles and GPU formats remain separate. The pointer memo is scoped to
// the current scene; it never persists across a different World.
type TextureKey = ([u8; 32], u8, bool);
type TextureCache = HashMap<TextureKey, (wgpu::Texture, wgpu::TextureView, u64)>;
type SourceKeys = HashMap<usize, [u8; 32]>;

fn texture_key(
    image: Option<&Arc<image::RgbaImage>>,
    role: u8,
    ground: bool,
    hashes: &mut SourceKeys,
) -> TextureKey {
    let Some(image) = image else {
        return ([0; 32], role, ground);
    };
    let address = Arc::as_ptr(image) as usize;
    let digest = *hashes.entry(address).or_insert_with(|| {
        let mut hash = Sha256::new();
        hash.update(image.width().to_le_bytes());
        hash.update(image.height().to_le_bytes());
        hash.update(image.as_raw());
        hash.finalize().into()
    });
    (digest, role, ground)
}

fn cache_bytes(cache: &TextureCache) -> u64 {
    cache
        .values()
        .fold(0u64, |sum, (_, _, bytes)| sum.saturating_add(*bytes))
}

/// Atomic GPU estimates shared with the UI; not an OS/driver VRAM query.
#[derive(Debug)]
pub(super) struct GpuProgress {
    pub pending: AtomicUsize,
    pub loaded: AtomicUsize,
    pub textures: AtomicU64,
    pub geometry: AtomicU64,
    pub peak: AtomicU64,
    pub deferred: AtomicUsize,
}
impl Default for GpuProgress {
    fn default() -> Self {
        Self {
            pending: AtomicUsize::new(1),
            loaded: AtomicUsize::new(0),
            textures: AtomicU64::new(0),
            geometry: AtomicU64::new(0),
            peak: AtomicU64::new(0),
            deferred: AtomicUsize::new(0),
        }
    }
}

/// Choose a size within the device and the per-material upload budget.
/// Non-terrain 4K DDS assets must retain their colors and alpha mask; a
/// bounded downscale is safer than substituting an opaque 1×1 fallback.
#[cfg(test)]
fn upload_dimensions(width: u32, height: u32, ground: bool, gpu_max: u32) -> Option<(u32, u32)> {
    if width == 0 || height == 0 || gpu_max == 0 {
        return None;
    }
    let cap = if ground {
        MAX_TERRAIN_ATLAS_BYTES
    } else {
        MAX_WORLD_DIFFUSE_BYTES
    };
    let (mut w, mut h) = (width, height);
    while w > gpu_max || h > gpu_max || u64::from(w) * u64::from(h) * 4 > cap {
        if w == 1 && h == 1 {
            return None;
        }
        w = w.div_ceil(2);
        h = h.div_ceil(2);
    }
    Some((w, h))
}

#[cfg(test)]
fn gpu_diffuse<'a>(
    img: &'a image::RgbaImage,
    ground: bool,
    gpu_max: u32,
) -> Option<Cow<'a, image::RgbaImage>> {
    let dims = upload_dimensions(img.width(), img.height(), ground, gpu_max)?;
    if dims == img.dimensions() {
        Some(Cow::Borrowed(img))
    } else {
        Some(Cow::Owned(image::imageops::resize(
            img,
            dims.0,
            dims.1,
            image::imageops::FilterType::Triangle,
        )))
    }
}

fn full_quality_dimensions(
    width: u32,
    height: u32,
    ground: bool,
    gpu_max: u32,
    full: bool,
    remaining: u64,
) -> Option<(u32, u32)> {
    if width == 0 || height == 0 || gpu_max == 0 || (!full && remaining < 4) {
        return None;
    }
    // Full quality is constrained by device dimensions, not the current
    // transient upload budget. Residency is separately admitted/deferred
    // by the streaming scheduler, which retains original source pixels.
    let cap = if full {
        MAX_FULL_TEXTURE_BYTES
    } else if ground {
        MAX_TERRAIN_ATLAS_BYTES.min(remaining)
    } else {
        MAX_WORLD_DIFFUSE_BYTES.min(remaining)
    };
    let (mut w, mut h) = (width, height);
    while w > gpu_max || h > gpu_max || u64::from(w) * u64::from(h) * 4 > cap {
        if w == 1 && h == 1 {
            return None;
        }
        w = w.div_ceil(2);
        h = h.div_ceil(2);
    }
    Some((w, h))
}

fn gpu_diffuse_quality<'a>(
    img: &'a image::RgbaImage,
    ground: bool,
    gpu_max: u32,
    full: bool,
    remaining: u64,
) -> Option<Cow<'a, image::RgbaImage>> {
    let dims =
        full_quality_dimensions(img.width(), img.height(), ground, gpu_max, full, remaining)?;
    if dims == img.dimensions() {
        Some(Cow::Borrowed(img))
    } else {
        Some(Cow::Owned(image::imageops::resize(
            img,
            dims.0,
            dims.1,
            image::imageops::FilterType::Triangle,
        )))
    }
}

fn mip_chain_bytes(mut width: u32, mut height: u32) -> u64 {
    let mut total = 0u64;
    loop {
        total = total.saturating_add(u64::from(width) * u64::from(height) * 4);
        if width == 1 && height == 1 {
            break;
        }
        width = (width / 2).max(1);
        height = (height / 2).max(1);
    }
    total
}

fn upload_rgba_mips(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: Option<&image::RgbaImage>,
    fallback: [u8; 4],
    format: wgpu::TextureFormat,
    label: &'static str,
) -> (wgpu::Texture, wgpu::TextureView) {
    let (width, height) = source.map_or((1, 1), image::RgbaImage::dimensions);
    let mip_count = width.max(height).ilog2() + 1;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: mip_count,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut current = source.map(Cow::Borrowed);
    for level in 0..mip_count {
        let w = (width >> level).max(1);
        let h = (height >> level).max(1);
        let pixels = current
            .as_ref()
            .map_or(fallback.as_slice(), |img| img.as_raw());
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        if level + 1 < mip_count
            && let Some(img) = current.as_ref()
        {
            current = Some(Cow::Owned(image::imageops::resize(
                img.as_ref(),
                (w / 2).max(1),
                (h / 2).max(1),
                image::imageops::FilterType::Triangle,
            )));
        }
    }
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

// A source image is resident at its original dimensions, but new mip pixels
// arrive one level at a time. The shader clamps LOD to the lowest *uploaded*
// level, so never samples uninitialized GPU texture data.
struct PendingDiffuseMip {
    source: Arc<image::RgbaImage>,
    next_level: u32,
    next_row: u32,
    prepared: Option<image::RgbaImage>,
}

fn write_rgba_level(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    image: &image::RgbaImage,
    level: u32,
) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: level,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        image.as_raw(),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(image.width() * 4),
            rows_per_image: Some(image.height()),
        },
        wgpu::Extent3d {
            width: image.width(),
            height: image.height(),
            depth_or_array_layers: 1,
        },
    );
}

// Upload useful low-resolution imagery immediately instead of leaving new
// geometry painted with a single 1x1 average-color pixel for many ticks.
// Every coarser level is also populated, because distant GPU samples may
// legally request LODs larger than the first valid texture level.
const INITIAL_DIFFUSE_MAX_DIMENSION: u32 = 128;
fn initial_diffuse_mip(source_width: u32, source_height: u32) -> u32 {
    let largest = source_width.max(source_height).max(1);
    let count = largest.ilog2() + 1;
    (0..count)
        .find(|level| {
            (source_width >> level).max(1) <= INITIAL_DIFFUSE_MAX_DIMENSION
                && (source_height >> level).max(1) <= INITIAL_DIFFUSE_MAX_DIMENSION
        })
        .unwrap_or(count - 1)
}

fn create_streaming_diffuse(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: &image::RgbaImage,
) -> (wgpu::Texture, wgpu::TextureView, u32) {
    let count = source.width().max(source.height()).ilog2() + 1;
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Full source diffuse streamed mip-by-mip"),
        size: wgpu::Extent3d {
            width: source.width(),
            height: source.height(),
            depth_or_array_layers: 1,
        },
        mip_level_count: count,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let first_valid = initial_diffuse_mip(source.width(), source.height());
    for level in first_valid..count {
        let coarse = next_diffuse_mip(source, level);
        write_rgba_level(queue, &tex, &coarse, level);
    }
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view, first_valid)
}

// Queue writes are striped so a single original 8K image cannot enqueue a
// >256 MiB transfer in one frame. The shader's mip floor is lowered *only*
// after all rows of the new level have been submitted.
fn write_mip_rows(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    mip: &image::RgbaImage,
    level: u32,
    start_row: u32,
) -> u32 {
    let stride = mip.width() as usize * 4;
    let rows = (MAX_MIP_WRITE_BYTES_PER_FRAME / stride).max(1) as u32;
    let end_row = start_row.saturating_add(rows).min(mip.height());
    let start = start_row as usize * stride;
    let end = end_row as usize * stride;
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: level,
            origin: wgpu::Origin3d {
                x: 0,
                y: start_row,
                z: 0,
            },
            aspect: wgpu::TextureAspect::All,
        },
        &mip.as_raw()[start..end],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(mip.width() * 4),
            rows_per_image: Some(end_row - start_row),
        },
        wgpu::Extent3d {
            width: mip.width(),
            height: end_row - start_row,
            depth_or_array_layers: 1,
        },
    );
    end_row
}

fn next_diffuse_mip(source: &image::RgbaImage, level: u32) -> image::RgbaImage {
    image::imageops::resize(
        source,
        (source.width() >> level).max(1),
        (source.height() >> level).max(1),
        image::imageops::FilterType::Triangle,
    )
}

/// Bounded, source-verified auxiliary MTL maps (normal/specular/gloss/emissive).
/// Original decoded Arc texture data is shared between world meshes. When a
/// GPU limit is reached the exact neutral fallback is used, never an unrelated
/// texture from a different game build.
#[allow(clippy::too_many_arguments)]
fn auxiliary_map(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: Option<&Arc<image::RgbaImage>>,
    role: u8,
    fallback: [u8; 4],
    format: wgpu::TextureFormat,
    label: &'static str,
    full_textures: bool,
    device_limit: u32,
    cache: &mut TextureCache,
    hashes: &mut SourceKeys,
    used_bytes: &mut u64,
) -> (wgpu::Texture, wgpu::TextureView) {
    let key = texture_key(source, role, false, hashes);
    if let Some((texture, view, _)) = cache.get(&key) {
        return (texture.clone(), view.clone());
    }
    let remaining = if full_textures {
        u64::MAX
    } else {
        MAX_SCENE_BALANCED_TEXTURE_BYTES.saturating_sub(*used_bytes)
    };
    let selected = source.and_then(|img| {
        gpu_diffuse_quality(
            img,
            false,
            device_limit,
            full_textures,
            remaining.saturating_mul(3) / 4,
        )
    });
    let (w, h) = selected.as_ref().map_or((1, 1), |img| img.dimensions());
    let (texture, view) =
        upload_rgba_mips(device, queue, selected.as_deref(), fallback, format, label);
    let bytes = mip_chain_bytes(w, h);
    *used_bytes = used_bytes.saturating_add(bytes);
    cache.insert(key, (texture.clone(), view.clone(), bytes));
    (texture, view)
}

// Preflight the next visible GPU group before making any wgpu allocations.
// This counts exact decoded content identities, not repeated Arc pointers,
// and includes every mip. Limits control residency, never original resolution.
fn group_new_bytes(
    group: &ModelGroup,
    cache: &TextureCache,
    hashes: &mut SourceKeys,
    gpu_max_dimension: u32,
) -> u64 {
    let mut seen = HashSet::new();
    let mut bytes = (group.mesh.positions.len() * std::mem::size_of::<Vertex>()
        + group.mesh.indices.len() * std::mem::size_of::<u32>()
        + group.instances.len().max(1) * std::mem::size_of::<InstanceGpu>()
        + group.surfaces.len() * 16) as u64;
    for surface in group
        .surfaces
        .iter()
        .filter(|s| s.visible && s.index_count > 0)
    {
        for (role, ground, source) in [
            (0, is_ground(group), surface.diffuse.as_ref()),
            (1, false, surface.opacity.as_ref()),
            (2, false, surface.normal.as_ref()),
            (3, false, surface.specular.as_ref()),
            (4, false, surface.gloss.as_ref()),
            (5, false, surface.emissive.as_ref()),
        ] {
            let key = texture_key(source, role, ground, hashes);
            if !cache.contains_key(&key) && seen.insert(key) {
                let footprint = source.map_or(4, |img| {
                    let (width, height) = full_quality_dimensions(
                        img.width(),
                        img.height(),
                        ground,
                        gpu_max_dimension,
                        true,
                        u64::MAX,
                    )
                    .unwrap_or((1, 1));
                    mip_chain_bytes(width, height)
                });
                bytes = bytes.saturating_add(footprint);
            }
        }
    }
    bytes
}

fn group_distance_sq(group: &ModelGroup, camera: level_camera::Camera) -> f32 {
    if is_ground(group) {
        return -1.0;
    }
    // Camera position only determines load order. Camera direction must never
    // prevent a source group from becoming GPU-resident in the Level Editor.
    group
        .instances
        .iter()
        .map(|instance| {
            instance
                .transform
                .position
                .iter()
                .zip(camera.position)
                .map(|(point, origin)| (point - origin) * (point - origin))
                .sum::<f32>()
        })
        .filter(|distance| distance.is_finite())
        .fold(f32::INFINITY, f32::min)
}

fn resident_geometry_bytes(groups: &[Option<GpuGroup>]) -> u64 {
    groups.iter().flatten().fold(0u64, |total, group| {
        total.saturating_add(group.geometry_bytes)
    })
}

fn is_ground(group: &ModelGroup) -> bool {
    group.instances.len() == 1 && group.instances[0].object_index == usize::MAX
}

#[derive(Clone)]
pub(super) struct ModelGroup {
    pub mesh: Arc<RenderMesh>,
    pub surfaces: Vec<Surface>,
    pub instances: Vec<Instance>,
    pub radius: f32,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Instance {
    pub object_index: usize,
    pub transform: Transform,
}
#[derive(Clone, Default)]
pub(super) struct World {
    pub groups: Vec<ModelGroup>,
    pub instance_count: usize,
    pub time_of_day: Option<Arc<super::level_environment::TimeOfDay>>,
    pub fog_profile: Option<super::level_environment::FogProfile>,
    pub source_resources: super::level_environment::ResourceSummary,
    /// Signed high-pass detail overlay; painted layer composition is prepared
    /// from verified same-install sources in the blocking map-open worker.
    pub terrain_detail: Option<Arc<image::RgbaImage>>,
    pub gpu_progress: Arc<GpuProgress>,
    // Prehashed by the existing map assembly worker, never by the UI thread.
    source_pixel_hashes: Arc<SourceKeys>,
}
impl std::fmt::Debug for World {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstancedWorld")
            .field("unique_meshes", &self.groups.len())
            .field("placements", &self.instance_count)
            .finish()
    }
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    xyz: [f32; 3],
    uv: [f32; 2],
    normal: [f32; 3],
}
impl Vertex {
    const ATTRS: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![
        0 => Float32x3, 1 => Float32x2, 2 => Float32x3
    ];
    fn layout<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRS,
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct InstanceGpu {
    cols: [[f32; 4]; 4],
}
impl InstanceGpu {
    const ATTRS: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
        3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4
    ];
    fn layout<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRS,
        }
    }
    fn from_transform(t: Transform) -> Self {
        let [mut w, mut x, mut y, mut z] = t.quaternion;
        let norm = (w * w + x * x + y * y + z * z).sqrt();
        if norm > 1.0e-8 {
            w /= norm;
            x /= norm;
            y /= norm;
            z /= norm;
        } else {
            w = 1.0;
            x = 0.0;
            y = 0.0;
            z = 0.0;
        }
        let [sx, sy, sz] = t.scale;
        Self {
            cols: [
                [
                    (1.0 - 2.0 * (y * y + z * z)) * sx,
                    (2.0 * (x * y + w * z)) * sx,
                    (2.0 * (x * z - w * y)) * sx,
                    0.0,
                ],
                [
                    (2.0 * (x * y - w * z)) * sy,
                    (1.0 - 2.0 * (x * x + z * z)) * sy,
                    (2.0 * (y * z + w * x)) * sy,
                    0.0,
                ],
                [
                    (2.0 * (x * z + w * y)) * sz,
                    (2.0 * (y * z - w * x)) * sz,
                    (1.0 - 2.0 * (x * x + y * y)) * sz,
                    0.0,
                ],
                [t.position[0], t.position[1], t.position[2], 1.0],
            ],
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, PartialEq)]
struct CameraGpu {
    origin: [f32; 4],
    forward: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
}
impl CameraGpu {
    fn make(camera: level_camera::Camera, yaw: f32, pitch: f32, aspect: f32, zoom: f32) -> Self {
        let (f, r, u) = level_camera::basis(yaw, pitch);
        let t = (camera.fov_degrees.clamp(25.0, 110.0).to_radians() * 0.5).tan()
            / zoom.clamp(0.2, 80.0);
        Self {
            origin: [
                camera.position[0],
                camera.position[1],
                camera.position[2],
                0.0,
            ],
            forward: [f[0], f[1], f[2], 0.1],
            right: [r[0], r[1], r[2], (t * aspect.max(0.01)).max(0.0001)],
            up: [u[0], u[1], u[2], t.max(0.0001)],
        }
    }
}
impl World {
    /// Compute exact content identities on the import worker before the GPU
    /// viewport starts. Avoid hashing multi-megabyte source textures on the
    /// rendering thread, where it would produce the very stalls we are fixing.
    pub fn precompute_gpu_source_hashes(&mut self) {
        let mut hashes = SourceKeys::new();
        for group in &self.groups {
            for surface in &group.surfaces {
                for source in [
                    surface.diffuse.as_ref(),
                    surface.opacity.as_ref(),
                    surface.normal.as_ref(),
                    surface.specular.as_ref(),
                    surface.gloss.as_ref(),
                    surface.emissive.as_ref(),
                ] {
                    if source.is_some() {
                        let _ = texture_key(source, 0, false, &mut hashes);
                    }
                }
            }
        }
        if let Some(ref detail) = self.terrain_detail {
            let _ = texture_key(Some(detail), 6, false, &mut hashes);
        }
        self.source_pixel_hashes = Arc::new(hashes);
    }

    /// Update only authored placement transforms in an editor-owned scene copy.
    /// GPU mesh buffers and original CryPAK file bytes remain immutable.
    pub fn updated(&self, scene: &evolve_core::LevelScene) -> Self {
        let mut next = self.clone();
        for group in &mut next.groups {
            for instance in &mut group.instances {
                if let Some(object) = scene.objects.get(instance.object_index) {
                    instance.transform = object.world;
                }
            }
        }
        next
    }

    /// Conservative ray/sphere picking for original world instances. It is
    /// intentionally approximate until per-mesh BVH ray traversal is ready.
    pub fn pick(
        &self,
        camera: level_camera::Camera,
        yaw: f32,
        pitch: f32,
        aspect: f32,
        zoom: f32,
        point: [f32; 2],
    ) -> Option<usize> {
        let (forward, right, up) = level_camera::basis(yaw, pitch);
        let t = (camera.fov_degrees.clamp(25.0, 110.0).to_radians() * 0.5).tan()
            / zoom.clamp(0.2, 80.0);
        let mut direction = [
            forward[0] + point[0] * t * aspect * right[0] + point[1] * t * up[0],
            forward[1] + point[0] * t * aspect * right[1] + point[1] * t * up[1],
            forward[2] + point[0] * t * aspect * right[2] + point[1] * t * up[2],
        ];
        let len = direction.iter().map(|x| x * x).sum::<f32>().sqrt();
        if len <= 1e-6 || !len.is_finite() {
            return None;
        }
        for x in &mut direction {
            *x /= len;
        }
        let mut closest = (f32::INFINITY, None);
        for group in &self.groups {
            for instance in &group.instances {
                if instance.object_index == usize::MAX {
                    continue;
                }
                let p = instance.transform.position;
                let delta = [
                    p[0] - camera.position[0],
                    p[1] - camera.position[1],
                    p[2] - camera.position[2],
                ];
                let dot = delta.iter().zip(direction).map(|(a, b)| a * b).sum::<f32>();
                if dot < 0.0 || dot > closest.0 {
                    continue;
                }
                let radius = group.radius
                    * instance
                        .transform
                        .scale
                        .iter()
                        .copied()
                        .map(f32::abs)
                        .fold(0.0, f32::max);
                let radial_sq = delta.iter().map(|x| x * x).sum::<f32>() - dot * dot;
                if radial_sq <= radius * radius && radius > 0.0 {
                    let hit = (radius * radius - radial_sq.max(0.0)).max(0.0).sqrt();
                    let distance = (dot - hit).max(0.0);
                    if distance < closest.0 {
                        closest = (distance, Some(instance.object_index));
                    }
                }
            }
        }
        closest.1
    }
}

pub(super) fn radius(mesh: &RenderMesh) -> f32 {
    mesh.positions
        .iter()
        .map(|v| v.iter().map(|f| f * f).sum::<f32>().sqrt())
        .filter(|r| r.is_finite())
        .fold(0.0f32, f32::max)
}
/// Instance transformations are static within a particular World identity.
/// Upload every placement once, irrespective of camera yaw, pitch or frustum.
fn all_group_instances(group: &ModelGroup) -> Vec<InstanceGpu> {
    group
        .instances
        .iter()
        .map(|instance| InstanceGpu::from_transform(instance.transform))
        .collect()
}
struct GpuSurface {
    diffuse_key: TextureKey,
    first: u32,
    count: u32,
    transparent: bool,
    foliage_cutout: bool,
    bind: wgpu::BindGroup,
    _image: wgpu::Texture,
    _opacity_image: wgpu::Texture,
    _normal_image: wgpu::Texture,
    _specular_image: wgpu::Texture,
    _gloss_image: wgpu::Texture,
    _emissive_image: wgpu::Texture,
    _detail_image: wgpu::Texture,
    _alpha_uniform: wgpu::Buffer,
}
struct GpuGroup {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    instances: wgpu::Buffer,
    visible: u32,
    surfaces: Vec<GpuSurface>,
    terrain: bool,
    geometry_bytes: u64,
}
pub(super) struct Renderer {
    solid: wgpu::RenderPipeline,
    terrain_source: wgpu::RenderPipeline,
    terrain_source_unlit: bool,
    foliage_cutout: wgpu::RenderPipeline,
    transparent: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    camera_uniform: wgpu::Buffer,
    lighting_uniform: wgpu::Buffer,
    fog_uniform: wgpu::Buffer,
    height_fog_uniform: wgpu::Buffer,
    last_lighting_hour: f32,
    sampler: wgpu::Sampler,
    terrain_sampler: wgpu::Sampler,
    world_identity: usize,
    last_full_textures: bool,
    groups: Vec<Option<GpuGroup>>,
    cache: TextureCache,
    source_hashes: SourceKeys,
    pending_diffuse: HashMap<TextureKey, PendingDiffuseMip>,
    mip_min_level: HashMap<TextureKey, u32>,
    mip_round_robin_cursor: usize,
    peak_gpu_bytes: u64,
    last_camera: Option<CameraGpu>,
}
impl Renderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Original level full source material bindings"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    // Fragment shading reads camera.origin for specular highlights
                    // and distance fog, in addition to vertex projection.
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
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
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Source normal, specular, gloss and emissive textures.
                // Keep all four in linear space where encoded data demands it;
                // colors use sRGB decoding when uploaded in the GPU helper.
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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
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
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // Distance-only Environment.xml fog preview. Kept world-only.
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Original TimeOfDay top/bottom fog density/height, if present.
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Exact source high-pass terrain overlay. Non-ground geometry
                // binds a neutral RGBA pixel with zero alpha.
                wgpu::BindGroupLayoutEntry {
                    binding: 12,
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
        let camera_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Level fly camera (GPU)"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let lighting_uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Level time-of-day lighting preview"),
            contents: bytemuck::cast_slice(&super::level_lighting::Lighting::at(12.0).uniforms()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let fog_uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Original map distance fog - non-native preview"),
            contents: bytemuck::cast_slice(&[[0.0_f32; 4]; 2]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let height_fog_uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Source TimeOfDay analytical height fog preview"),
            contents: bytemuck::cast_slice(&[[0.0_f32; 4]; 2]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        // Baked terrain UVs are normalized across the entire map: an edge
        // at exactly 1.0 must not repeat the opposite side of the atlas.
        let terrain_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("CryEngine instanced Level Editor"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "world_instances.wgsl"
            ))),
        });
        let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Instanced level model pipeline"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let make = |blend: bool, cutout: bool, raw_terrain: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(if raw_terrain {
                    "Unlit original terrain color atlas"
                } else if blend {
                    "Transparent instanced world geometry"
                } else if cutout {
                    "Alpha-tested CryEngine vegetation (depth-writing)"
                } else {
                    "Opaque instanced world geometry"
                }),
                layout: Some(&pipe_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[Vertex::layout(), InstanceGpu::layout()],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    // Solid world meshes must NOT forward packed diffuse alpha
                    // into the Iced window target; it can make the actual
                    // window transparent to applications beneath it.
                    entry_point: Some(if raw_terrain {
                        "fs_terrain_source"
                    } else if cutout {
                        "fs_foliage"
                    } else if blend {
                        "fs_main"
                    } else {
                        "fs_opaque"
                    }),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: if blend {
                            Some(wgpu::BlendState::ALPHA_BLENDING)
                        } else {
                            None
                        },
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
                    depth_write_enabled: !blend,
                    depth_compare: wgpu::CompareFunction::Less,
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            })
        };
        Self {
            solid: make(false, false, false),
            terrain_source: make(false, false, true),
            terrain_source_unlit: false,
            foliage_cutout: make(false, true, false),
            transparent: make(true, false, false),
            layout,
            camera_uniform,
            lighting_uniform,
            fog_uniform,
            height_fog_uniform,
            last_lighting_hour: f32::NAN,
            sampler,
            terrain_sampler,
            world_identity: 0,
            last_full_textures: true,
            groups: Vec::new(),
            cache: TextureCache::new(),
            source_hashes: SourceKeys::new(),
            pending_diffuse: HashMap::new(),
            mip_min_level: HashMap::new(),
            mip_round_robin_cursor: 0,
            peak_gpu_bytes: 0,
            last_camera: None,
        }
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "Explicit GPU upload state keeps residency and mip tracking scoped to each renderer"
    )]
    fn group(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        model: &ModelGroup,
        terrain_detail: Option<&Arc<image::RgbaImage>>,
        full_textures: bool,
        cache: &mut TextureCache,
        hashes: &mut SourceKeys,
        pending_diffuse: &mut HashMap<TextureKey, PendingDiffuseMip>,
        mip_min_level: &mut HashMap<TextureKey, u32>,
        used_bytes: &mut u64,
    ) -> GpuGroup {
        let vert: Vec<Vertex> = model
            .mesh
            .positions
            .iter()
            .enumerate()
            .map(|(i, &xyz)| Vertex {
                xyz,
                uv: model
                    .mesh
                    .uv0
                    .as_ref()
                    .and_then(|s| s.get(i))
                    .copied()
                    .unwrap_or([0.0; 2]),
                // Zero explicitly requests geometric face-normal recovery in
                // WGSL when a compiled mesh contains no usable normal stream.
                normal: model
                    .mesh
                    .normals
                    .as_ref()
                    .and_then(|s| s.get(i))
                    .copied()
                    .filter(|n| n.iter().all(|component| component.is_finite()))
                    .unwrap_or([0.0; 3]),
            })
            .collect();
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Unique original world mesh vertices"),
            contents: bytemuck::cast_slice(&vert),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Unique original world mesh indices"),
            contents: bytemuck::cast_slice(&model.mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let capacity = model.instances.len().max(1);
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("All persistent world-placement transforms"),
            size: (capacity * std::mem::size_of::<InstanceGpu>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let placements = all_group_instances(model);
        if !placements.is_empty() {
            queue.write_buffer(&instances, 0, bytemuck::cast_slice(&placements));
        }
        let mut surfaces = Vec::new();
        let fallback = [210u8, 205, 205, 255];
        let terrain = is_ground(model);
        let limit_2d = device.limits().max_texture_dimension_2d;
        for surf in &model.surfaces {
            if !surf.visible
                || surf.index_count == 0
                || surf.first_index as usize + surf.index_count as usize > model.mesh.indices.len()
            {
                continue;
            }
            let key = texture_key(surf.diffuse.as_ref(), 0, terrain, hashes);
            let (tex, view, diffuse_uploaded) = if let Some((tex, view, _)) = cache.get(&key) {
                (tex.clone(), view.clone(), surf.diffuse.is_some())
            } else {
                let remaining = if full_textures {
                    u64::MAX
                } else {
                    MAX_SCENE_BALANCED_TEXTURE_BYTES.saturating_sub(*used_bytes)
                };
                let source = surf.diffuse.as_ref().and_then(|img| {
                    gpu_diffuse_quality(
                        img,
                        terrain,
                        limit_2d,
                        full_textures,
                        remaining.saturating_mul(3) / 4,
                    )
                });
                let (w, h) = source.as_ref().map_or((1, 1), |img| img.dimensions());
                let (tex, view) = if let Some(ref selected) = source {
                    let original = match selected {
                        Cow::Borrowed(_)
                            if surf
                                .diffuse
                                .as_ref()
                                .is_some_and(|img| img.dimensions() == (w, h)) =>
                        {
                            surf.diffuse
                                .as_ref()
                                .expect("borrowed diffuse source")
                                .clone()
                        }
                        _ => Arc::new(selected.as_ref().clone()),
                    };
                    let (tex, view, lowest) = create_streaming_diffuse(device, queue, &original);
                    mip_min_level.insert(key, lowest);
                    if lowest > 0 {
                        pending_diffuse.insert(
                            key,
                            PendingDiffuseMip {
                                source: original,
                                next_level: lowest - 1,
                                next_row: 0,
                                prepared: None,
                            },
                        );
                    }
                    (tex, view)
                } else {
                    upload_rgba_mips(
                        device,
                        queue,
                        None,
                        fallback,
                        wgpu::TextureFormat::Rgba8UnormSrgb,
                        "Missing source diffuse (neutral)",
                    )
                };
                let bytes = mip_chain_bytes(w, h);
                *used_bytes = used_bytes.saturating_add(bytes);
                cache.insert(key, (tex.clone(), view.clone(), bytes));
                (tex, view, source.is_some())
            };
            // A distinct authored opacity texture must control foliage
            // cutouts. Never misinterpret a packed diffuse alpha channel
            // (which may be gloss or specular) as vegetation coverage.
            let mask_key = texture_key(surf.opacity.as_ref(), 1, false, hashes);
            let (opacity_tex, opacity_view) = if let Some((tex, view, _)) = cache.get(&mask_key) {
                (tex.clone(), view.clone())
            } else {
                let opaque = [255u8, 255, 255, 255];
                let remaining = if full_textures {
                    u64::MAX
                } else {
                    MAX_SCENE_BALANCED_TEXTURE_BYTES.saturating_sub(*used_bytes)
                };
                let mask = surf.opacity.as_ref().and_then(|img| {
                    gpu_diffuse_quality(
                        img,
                        false,
                        limit_2d,
                        full_textures,
                        remaining.saturating_mul(3) / 4,
                    )
                });
                let (mw, mh) = mask.as_ref().map_or((1, 1), |img| img.dimensions());
                let (opacity_tex, view) = upload_rgba_mips(
                    device,
                    queue,
                    mask.as_deref(),
                    opaque,
                    wgpu::TextureFormat::Rgba8Unorm,
                    "Original authored opacity mask",
                );
                let bytes = mip_chain_bytes(mw, mh);
                *used_bytes = used_bytes.saturating_add(bytes);
                cache.insert(mask_key, (opacity_tex.clone(), view.clone(), bytes));
                (opacity_tex, view)
            };
            let (normal_tex, normal_view) = auxiliary_map(
                device,
                queue,
                surf.normal.as_ref(),
                2,
                [128, 128, 255, 255],
                wgpu::TextureFormat::Rgba8Unorm,
                "Original linear BC5 normal",
                full_textures,
                limit_2d,
                cache,
                hashes,
                used_bytes,
            );
            let (specular_tex, specular_view) = auxiliary_map(
                device,
                queue,
                surf.specular.as_ref(),
                3,
                [0, 0, 0, 255],
                wgpu::TextureFormat::Rgba8UnormSrgb,
                "Original specular reflectance",
                full_textures,
                limit_2d,
                cache,
                hashes,
                used_bytes,
            );
            let (gloss_tex, gloss_view) = auxiliary_map(
                device,
                queue,
                surf.gloss.as_ref(),
                4,
                [128, 128, 128, 255],
                wgpu::TextureFormat::Rgba8Unorm,
                "Original gloss smoothness",
                full_textures,
                limit_2d,
                cache,
                hashes,
                used_bytes,
            );
            let (emissive_tex, emissive_view) = auxiliary_map(
                device,
                queue,
                surf.emissive.as_ref(),
                5,
                [0, 0, 0, 255],
                wgpu::TextureFormat::Rgba8UnormSrgb,
                "Original emissive color",
                full_textures,
                limit_2d,
                cache,
                hashes,
                used_bytes,
            );
            let (detail_tex, detail_view) = auxiliary_map(
                device,
                queue,
                if terrain { terrain_detail } else { None },
                6,
                [128, 128, 128, 0],
                wgpu::TextureFormat::Rgba8Unorm,
                "Original terrain signed high-pass detail overlay",
                full_textures,
                limit_2d,
                cache,
                hashes,
                used_bytes,
            );
            let alpha_uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Authored per-material Vegetation AlphaTest"),
                contents: bytemuck::cast_slice(&[
                    surf.alpha_cutoff.clamp(0.0, 1.0),
                    surf.emission_strength.clamp(0.0, 30.0),
                    if surf.normal.is_some() { 1.0 } else { 0.0 },
                    mip_min_level.get(&key).copied().unwrap_or(0) as f32,
                ]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Exact source material per instanced mesh"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.camera_uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(if terrain {
                            &self.terrain_sampler
                        } else {
                            &self.sampler
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&opacity_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: alpha_uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: self.lighting_uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::TextureView(&normal_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: wgpu::BindingResource::TextureView(&specular_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 8,
                        resource: wgpu::BindingResource::TextureView(&gloss_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 9,
                        resource: wgpu::BindingResource::TextureView(&emissive_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 10,
                        resource: self.fog_uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 11,
                        resource: self.height_fog_uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 12,
                        resource: wgpu::BindingResource::TextureView(&detail_view),
                    },
                ],
            });
            surfaces.push(GpuSurface {
                diffuse_key: key,
                first: surf.first_index,
                count: surf.index_count,
                transparent: surf.transparent,
                foliage_cutout: surf.foliage_cutout && diffuse_uploaded,
                bind,
                _image: tex,
                _opacity_image: opacity_tex,
                _normal_image: normal_tex,
                _specular_image: specular_tex,
                _gloss_image: gloss_tex,
                _emissive_image: emissive_tex,
                _detail_image: detail_tex,
                _alpha_uniform: alpha_uniform,
            });
        }
        if surfaces.is_empty() {
            // NoDraw-only source geometry must stay invisible, even when a
            // decoder exposes indices for its hidden material subsets.
        }
        GpuGroup {
            vertices,
            indices,
            instances,
            visible: model.instances.len().min(u32::MAX as usize) as u32,
            surfaces,
            terrain,
            geometry_bytes: (vert.len() * std::mem::size_of::<Vertex>()
                + model.mesh.indices.len() * std::mem::size_of::<u32>()
                + capacity * std::mem::size_of::<InstanceGpu>()
                + 16 * model.surfaces.len()) as u64,
        }
    }
    #[allow(clippy::too_many_arguments)] // Upload needs device, queue, world and camera projection parameters.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        world: Option<&Arc<World>>,
        camera: Option<level_camera::Camera>,
        yaw: f32,
        pitch: f32,
        aspect: f32,
        zoom: f32,
        hour: f32,
        full_textures: bool,
        terrain_source_unlit: bool,
    ) {
        let Some(world) = world else {
            self.groups.clear();
            self.cache.clear();
            self.source_hashes.clear();
            self.pending_diffuse.clear();
            self.mip_min_level.clear();
            self.mip_round_robin_cursor = 0;
            self.peak_gpu_bytes = 0;
            self.world_identity = 0;
            self.last_camera = None;
            return;
        };
        self.terrain_source_unlit = terrain_source_unlit;
        let key = Arc::as_ptr(world) as usize;
        if key != self.world_identity || self.last_full_textures != full_textures {
            // Drop the former GPU scene before creating any new allocations.
            // Full decoded source assets remain on World, not in this cache.
            self.groups = std::iter::repeat_with(|| None)
                .take(world.groups.len())
                .collect();
            self.cache.clear();
            self.source_hashes = (*world.source_pixel_hashes).clone();
            self.pending_diffuse.clear();
            self.mip_min_level.clear();
            self.mip_round_robin_cursor = 0;
            self.peak_gpu_bytes = 0;
            self.world_identity = key;
            self.last_full_textures = full_textures;
            self.last_camera = None;
            self.last_lighting_hour = f32::NAN;
            world.gpu_progress.pending.store(1, Ordering::Relaxed);
        }
        let hour = if hour.is_finite() {
            hour.clamp(0.0, 24.0)
        } else {
            12.0
        };
        if self.last_lighting_hour != hour {
            let lighting = world.time_of_day.as_ref().map_or_else(
                || super::level_lighting::Lighting::at(hour),
                |profile| super::level_lighting::Lighting::from_original(hour, profile),
            );
            let values = lighting.uniforms();
            queue.write_buffer(&self.lighting_uniform, 0, bytemuck::cast_slice(&values));
            let fog = world.fog_profile.map_or([[0.0_f32; 4]; 2], |profile| {
                profile.preview_uniform(world.time_of_day.as_deref(), hour)
            });
            queue.write_buffer(&self.fog_uniform, 0, bytemuck::cast_slice(&fog));
            let height = world.fog_profile.map_or([[0.0_f32; 4]; 2], |profile| {
                profile.height_preview_uniform(world.time_of_day.as_deref(), hour)
            });
            queue.write_buffer(&self.height_fog_uniform, 0, bytemuck::cast_slice(&height));
            self.last_lighting_hour = hour;
        }
        let Some(camera) = camera else {
            return;
        };
        let uniform = CameraGpu::make(camera, yaw, pitch, aspect, zoom);
        let camera_changed = self.last_camera != Some(uniform);
        let mut cache = std::mem::take(&mut self.cache);
        let mut hashes = std::mem::take(&mut self.source_hashes);
        // Loaded GPU groups and all of their transforms remain resident until
        // an explicit world/quality reset; camera motion changes neither.
        self.pending_diffuse
            .retain(|key, _| cache.contains_key(key));
        self.mip_min_level.retain(|key, _| cache.contains_key(key));
        let target_bytes = residency_target_bytes(full_textures);
        let mut used_bytes = cache_bytes(&cache);
        let mut deferred = 0usize;
        let mut pending = 0usize;
        let mut uploaded = 0usize;
        // Group creation borrows the pipeline layouts from &self. Keep the
        // mutable streaming tables local until all groups are built to avoid
        // aliasing the immutable Renderer borrow with mutable fields.
        let mut pending_diffuse = std::mem::take(&mut self.pending_diffuse);
        let mut mip_min_level = std::mem::take(&mut self.mip_min_level);
        // Ground first, then all unresolved source groups by camera distance.
        // No camera-facing/frustum test may exclude a group from streaming:
        // every resolved scene placement is loaded in bounded GPU batches.
        let mut candidates = (0..world.groups.len())
            .filter(|&idx| !world.groups[idx].instances.is_empty() && self.groups[idx].is_none())
            .collect::<Vec<_>>();
        candidates.sort_by(|&left, &right| {
            group_distance_sq(&world.groups[left], camera)
                .total_cmp(&group_distance_sq(&world.groups[right], camera))
        });
        for idx in candidates {
            if uploaded == MAX_NEW_GROUPS_PER_FRAME {
                pending += 1;
                continue;
            }
            let requested = group_new_bytes(
                &world.groups[idx],
                &cache,
                &mut hashes,
                device.limits().max_texture_dimension_2d,
            );
            // Never evict previously loaded groups to make space for a new
            // one. Full quality admits all resolved groups (subject to physical
            // GPU limits); Balanced defers new groups if its cap is reached.
            // This prevents camera-motion thrashing and persistent pop-out.
            if used_bytes
                .saturating_add(resident_geometry_bytes(&self.groups))
                .saturating_add(requested)
                > target_bytes
            {
                // Balanced mode: source CPU data remains intact, and this
                // not-yet-loaded group may be retried after a quality change.
                deferred += 1;
                continue;
            }
            let gpu = self.group(
                device,
                queue,
                &world.groups[idx],
                if is_ground(&world.groups[idx]) { world.terrain_detail.as_ref() } else { None },
                full_textures,
                &mut cache,
                &mut hashes,
                &mut pending_diffuse,
                &mut mip_min_level,
                &mut used_bytes,
            );
            self.groups[idx] = Some(gpu);
            uploaded += 1;
        }
        self.pending_diffuse = pending_diffuse;
        self.mip_min_level = mip_min_level;
        // Advance one independent diffuse mip each redraw. Existing materials
        // keep sampling their previously uploaded coarser mip until this write
        // is submitted, so no undefined/uninitialized GPU texture reads occur.
        // A HashMap's first key is stable enough to starve all other mip
        // chains until a giant texture completes. Rotate across every
        // verified, resident diffuse so surfaces refine together.
        let mut mip_keys = self
            .pending_diffuse
            .keys()
            .copied()
            .filter(|key| cache.contains_key(key))
            .collect::<Vec<_>>();
        mip_keys.sort_unstable();
        let next_key = if mip_keys.is_empty() {
            None
        } else {
            let key = mip_keys[self.mip_round_robin_cursor % mip_keys.len()];
            self.mip_round_robin_cursor = self.mip_round_robin_cursor.wrapping_add(1);
            Some(key)
        };
        let mut mip_uploads = 0usize;
        let mut finished_key = None;
        if let Some(key) = next_key
            && let Some(pending) = self.pending_diffuse.get_mut(&key)
            && let Some((texture, _, _)) = cache.get(&key)
        {
            let level = pending.next_level;
            let mip = if level == 0 {
                &*pending.source
            } else {
                pending
                    .prepared
                    .get_or_insert_with(|| next_diffuse_mip(&pending.source, level))
            };
            let row = write_mip_rows(queue, texture, mip, level, pending.next_row);
            let completed = row == mip.height();
            pending.next_row = row;
            mip_uploads += 1;
            if completed {
                self.mip_min_level.insert(key, level);
                for gpu in self.groups.iter().flatten() {
                    for surface in &gpu.surfaces {
                        if surface.diffuse_key == key {
                            queue.write_buffer(
                                &surface._alpha_uniform,
                                12,
                                bytemuck::bytes_of(&(level as f32)),
                            );
                        }
                    }
                }
                if level == 0 {
                    finished_key = Some(key);
                } else {
                    pending.next_level -= 1;
                    pending.next_row = 0;
                    pending.prepared = None;
                }
            }
        }
        if let Some(key) = finished_key {
            self.pending_diffuse.remove(&key);
        }
        let texture_bytes = cache_bytes(&cache);
        let geometry_bytes = resident_geometry_bytes(&self.groups);
        let resident = texture_bytes.saturating_add(geometry_bytes);
        self.peak_gpu_bytes = self.peak_gpu_bytes.max(resident);
        let progress = &world.gpu_progress;
        progress
            .pending
            .store(pending + self.pending_diffuse.len(), Ordering::Relaxed);
        progress.deferred.store(deferred, Ordering::Relaxed);
        progress
            .loaded
            .store(self.groups.iter().flatten().count(), Ordering::Relaxed);
        progress.textures.store(texture_bytes, Ordering::Relaxed);
        progress.geometry.store(geometry_bytes, Ordering::Relaxed);
        progress.peak.store(self.peak_gpu_bytes, Ordering::Relaxed);
        self.cache = cache;
        self.source_hashes = hashes;
        if (uploaded != 0 || mip_uploads != 0)
            && (progress.loaded.load(Ordering::Relaxed) % 16 == 0
                || (pending == 0 && self.pending_diffuse.is_empty()))
        {
            crate::crash_diagnostics::record(
                "level_gpu_residency",
                &format!(
                    "{} / {} resident groups, {} MiB texture mips, {} MiB geometry, peak {} MiB, {} queued, {} deferred; adapter free VRAM unknown",
                    progress.loaded.load(Ordering::Relaxed),
                    world.groups.len(),
                    texture_bytes / 1048576,
                    geometry_bytes / 1048576,
                    self.peak_gpu_bytes / 1048576,
                    pending + self.pending_diffuse.len(),
                    deferred,
                ),
            );
        }
        if camera_changed || uploaded > 0 {
            // The GPU clips primitives outside the viewport; there is no
            // per-camera CPU culling or repeated instance-buffer upload.
            queue.write_buffer(&self.camera_uniform, 0, bytemuck::bytes_of(&uniform));
        }
        self.last_camera = Some(uniform);
    }
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        // Opaque, masked foliage, then blended glass/water. Alpha-tested
        // vegetation writes depth, unlike translucent overlays.
        for mode in 0..3 {
            for group in self.groups.iter().flatten() {
                if group.visible == 0 {
                    continue;
                }
                pass.set_pipeline(match mode {
                    0 if group.terrain && self.terrain_source_unlit => &self.terrain_source,
                    0 => &self.solid,
                    1 => &self.foliage_cutout,
                    _ => &self.transparent,
                });
                pass.set_vertex_buffer(0, group.vertices.slice(..));
                pass.set_vertex_buffer(1, group.instances.slice(..));
                pass.set_index_buffer(group.indices.slice(..), wgpu::IndexFormat::Uint32);
                for surf in group.surfaces.iter().filter(|s| match mode {
                    0 => !s.transparent && !s.foliage_cutout,
                    1 => s.foliage_cutout,
                    _ => s.transparent,
                }) {
                    pass.set_bind_group(0, &surf.bind, &[]);
                    pass.draw_indexed(surf.first..surf.first + surf.count, 0, 0..group.visible);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_terrain_shader_does_not_apply_lighting_or_fog_to_original_pixels() {
        let shader = include_str!("world_instances.wgsl");
        let entry = shader
            .split("fn fs_terrain_source(input:Output)")
            .nth(1)
            .expect("unlit source terrain shader")
            .split("@fragment")
            .next()
            .unwrap();
        assert!(entry.contains("sample_source_albedo(input.uv)"));
        assert!(!entry.contains("distance_fog("));
        assert!(!entry.contains("shade("));
        naga::front::wgsl::parse_str(shader).expect("terrain source shader parses");
    }

    #[test]
    fn group_loading_priority_uses_position_not_camera_direction() {
        let dummy_mesh = Arc::new(RenderMesh {
            info: evolve_core::ModelInfo {
                format: "test".into(),
                version: 1,
                chunk_count: 0,
                vertex_count: 0,
                triangle_count: 0,
                original_uv0: true,
                original_normals: true,
            },
            positions: Vec::new(),
            indices: Vec::new(),
            uv0: None,
            normals: None,
        });
        let group = ModelGroup {
            mesh: dummy_mesh,
            surfaces: Vec::new(),
            instances: vec![
                Instance {
                    object_index: 1,
                    transform: Transform::default(),
                },
                Instance {
                    object_index: 2,
                    transform: Transform::default(),
                },
            ],
            radius: 1.0,
        };
        let distant_visible = Transform {
            position: [100.0, 0.0, 0.0],
            ..Transform::default()
        };
        let mut group = group;
        group.instances[0].transform = distant_visible;
        group.instances[1].transform = distant_visible;
        let camera = level_camera::Camera::default();
        assert_eq!(group_distance_sq(&group, camera), 10000.0);
        assert_eq!(all_group_instances(&group).len(), 2);
        let mut opposite_camera = camera;
        opposite_camera.position = [200.0, 0.0, 0.0];
        assert_eq!(group_distance_sq(&group, opposite_camera), 10000.0);
        assert_eq!(all_group_instances(&group).len(), 2);
    }

    #[test]
    fn streamed_mip_writes_are_bounded_independently_of_full_resolution() {
        assert_eq!(MAX_MIP_WRITE_BYTES_PER_FRAME, 8 * 1024 * 1024);
        let rows_8k = (MAX_MIP_WRITE_BYTES_PER_FRAME / (8192 * 4)).max(1);
        assert_eq!(rows_8k, 256);
        assert_eq!(8192usize.div_ceil(rows_8k), 32);
    }

    #[test]
    fn streaming_diffuse_shader_never_reads_uninitialized_high_mips() {
        let shader = include_str!("world_instances.wgsl");
        assert!(shader.contains("let safe_lod=max(implicit_lod, authored_cutoff.w);"));
        assert!(shader.contains("textureSampleLevel(albedo,tex_sampler,uv,safe_lod)"));
        assert!(shader.contains("let leaf_alpha=sample_source_albedo(input.uv).a;"));
        assert!(shader.contains("let base=sample_source_albedo(input.uv);"));
        naga::front::wgsl::parse_str(shader).expect("streamed diffuse shader must parse");
    }

    #[test]
    fn initial_diffuse_upload_shows_detail_and_allows_valid_coarser_mips() {
        assert_eq!(initial_diffuse_mip(8192, 8192), 6);
        assert_eq!(initial_diffuse_mip(4096, 4096), 5);
        assert_eq!(initial_diffuse_mip(1000, 500), 3);
        assert_eq!(initial_diffuse_mip(128, 64), 0);
        assert_eq!(initial_diffuse_mip(1, 1), 0);
        assert!((8192 >> initial_diffuse_mip(8192, 8192)) <= INITIAL_DIFFUSE_MAX_DIMENSION);
    }

    #[test]
    fn diffuse_mip_upload_dimensions_are_independent() {
        let img = image::RgbaImage::from_pixel(8, 4, image::Rgba([3, 4, 5, 255]));
        assert_eq!(next_diffuse_mip(&img, 3).dimensions(), (1, 1));
        assert_eq!(next_diffuse_mip(&img, 2).dimensions(), (2, 1));
        assert_eq!(next_diffuse_mip(&img, 1).dimensions(), (4, 2));
        assert_eq!(next_diffuse_mip(&img, 0).dimensions(), (8, 4));
    }

    #[test]
    fn content_identity_deduplicates_independent_imported_rgba_buffers() {
        let original = Arc::new(image::RgbaImage::from_pixel(
            8,
            8,
            image::Rgba([20, 40, 60, 255]),
        ));
        let duplicate = Arc::new((*original).clone());
        let changed = Arc::new(image::RgbaImage::from_pixel(
            8,
            8,
            image::Rgba([20, 40, 61, 255]),
        ));
        assert!(!Arc::ptr_eq(&original, &duplicate));
        let mut hashes = SourceKeys::new();
        let key = texture_key(Some(&original), 0, false, &mut hashes);
        assert_eq!(key, texture_key(Some(&duplicate), 0, false, &mut hashes));
        assert_ne!(key, texture_key(Some(&changed), 0, false, &mut hashes));
        assert_ne!(key, texture_key(Some(&duplicate), 2, false, &mut hashes));
        assert_ne!(key, texture_key(Some(&duplicate), 0, true, &mut hashes));
        assert_eq!(hashes.len(), 3);
    }

    #[test]
    fn full_source_mips_and_residency_budget_are_distinct() {
        assert_eq!(
            full_quality_dimensions(8192, 8192, false, 16384, true, u64::MAX),
            Some((8192, 8192))
        );
        assert_eq!(
            full_quality_dimensions(8192, 8192, true, 16384, true, u64::MAX),
            Some((8192, 8192))
        );
        assert!(mip_chain_bytes(8192, 8192) > 340 * 1024 * 1024);
        assert_eq!(residency_target_bytes(true), u64::MAX);
        assert_eq!(
            residency_target_bytes(false),
            MAX_SCENE_BALANCED_TEXTURE_BYTES
        );
        assert_eq!(MAX_NEW_GROUPS_PER_FRAME, 2);
    }

    #[test]
    fn world_camera_binding_allows_fragment_shading() {
        // Both editors initialize the shared world renderer even in model-only views.
        let shader = include_str!("world_instances.wgsl");
        assert!(shader.contains("@group(0) @binding(0) var<uniform> camera: Camera;"));
        assert!(shader.contains("let view_direction=normalize(camera.origin.xyz-input.world);"));
        assert!(shader.contains("let distance=length(camera.origin.xyz-input.world);"));

        let source = include_str!("world_instances.rs");
        let layout = source
            .split(r#"label: Some("Original level full source material bindings")"#)
            .nth(1)
            .expect("world pipeline bind-group layout");
        let camera = layout
            .split("binding: 0,")
            .nth(1)
            .expect("camera binding")
            .split("binding: 1,")
            .next()
            .unwrap();
        assert!(camera.contains("wgpu::ShaderStages::VERTEX"));
        assert!(camera.contains("wgpu::ShaderStages::FRAGMENT"));
    }
    #[test]
    fn solid_world_shader_forces_opaque_alpha_but_glass_retains_blending() {
        let shader = include_str!("world_instances.wgsl");
        // Native window alpha must be opaque for every solid placed mesh,
        // including source DDS textures with packed spec/gloss alpha.
        assert!(shader.contains("fn fs_opaque(input:Output)"));
        assert!(shader.contains("return vec4<f32>(fogged.rgb,1.0);"));
        assert!(shader.contains("fn fs_foliage(input:Output)"));
        assert!(shader.contains("let leaf_alpha=sample_source_albedo(input.uv).a;"));
        assert!(shader.contains("leaf_alpha < authored_cutoff.x"));
        assert!(shader.contains("let color=shade(input,false);"));
        // Explicitly transparent materials still use authored coverage.
        assert!(shader.contains("fn fs_main(input:Output)"));
        assert!(shader.contains("return vec4<f32>(fogged.rgb,color.a*alpha);"));
        let pipeline_source = include_str!("world_instances.rs");
        // Search only production code; searching this entire file would let
        // the assertion's own string literal create a false positive.
        let production = pipeline_source
            .split("\n#[cfg(test)]\nmod tests {")
            .next()
            .unwrap();
        assert!(production.contains("entry_point: Some(if cutout {"));
        assert!(production.contains("\"fs_foliage\""));
        assert!(production.contains("foliage_cutout: make(false, true)"));
        assert!(production.contains("blend: if blend {"));
        assert!(production.contains("depth_write_enabled: !blend"));
        assert!(production.contains("foliage_cutout: surf.foliage_cutout && diffuse_uploaded"));
        assert!(shader.contains("@group(0) @binding(5) var<uniform> light"));
        assert!(shader.contains("@group(0) @binding(10) var<uniform> fog"));
        assert!(shader.contains("@group(0) @binding(11) var<uniform> height_fog"));
        let module = naga::front::wgsl::parse_str(shader)
            .expect("time-of-day world shader must parse as WGSL");
        assert!(
            module
                .entry_points
                .iter()
                .any(|entry| entry.name == "fs_opaque")
        );
        assert!(
            module
                .entry_points
                .iter()
                .any(|entry| entry.name == "fs_foliage")
        );
    }

    #[test]
    fn height_fog_uniform_stays_world_only_and_shader_parses() {
        let shader = include_str!("world_instances.wgsl");
        assert!(shader.contains("fn height_fog_depth"));
        assert!(shader.contains("if (height_fog.control.x > 0.5)"));
        let module = naga::front::wgsl::parse_str(shader).expect("source-backed height fog WGSL");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("analytical height fog shader must pass full Naga validation");
    }

    #[test]
    fn world_only_distance_fog_preview_is_disabled_without_verified_sources() {
        let shader = include_str!("world_instances.wgsl");
        assert!(shader.contains("if (fog.params.x < 0.5) { return color; }"));
        assert!(shader.contains("let distance=length(camera.origin.xyz-input.world);"));
        assert!(shader.contains("fn distance_fog(input:Output, color:vec4<f32>)"));
        naga::front::wgsl::parse_str(shader).expect("distance fog preview shader is valid");
    }

    #[test]
    fn world_material_shader_uses_original_bump_specular_gloss_and_emittance() {
        let shader = include_str!("world_instances.wgsl");
        for source in ["normal_map", "specular_map", "gloss_map", "emissive_map"] {
            assert!(shader.contains(source));
        }
        assert!(shader.contains("cross(b,c)"));
        assert!(shader.contains("let face_normal = cross(dpdx(input.world), dpdy(input.world));"));
        let rust = include_str!("world_instances.rs");
        for field in [
            "surf.normal.as_ref()",
            "surf.specular.as_ref()",
            "surf.gloss.as_ref()",
            "surf.emissive.as_ref()",
        ] {
            assert!(rust.contains(field));
        }
        naga::front::wgsl::parse_str(shader).expect("full-source world material WGSL");
    }
    #[test]
    fn baked_terrain_atlas_is_not_silently_replaced_by_32mb_world_cap() {
        assert_eq!(MAX_TERRAIN_ATLAS_BYTES, 64 * 1024 * 1024);
        assert_eq!(MAX_WORLD_DIFFUSE_BYTES, 32 * 1024 * 1024);
        assert_eq!(
            upload_dimensions(4096, 4096, true, 4096),
            Some((4096, 4096))
        );
        assert_eq!(
            upload_dimensions(4096, 4096, false, 4096),
            Some((2048, 2048))
        );
        assert_eq!(
            upload_dimensions(4096, 4096, true, 2048),
            Some((2048, 2048))
        );
    }

    #[test]
    fn oversized_valid_model_texture_is_downscaled_not_discarded() {
        assert_eq!(
            upload_dimensions(4096, 4096, false, 4096),
            Some((2048, 2048))
        );
        assert_eq!(
            upload_dimensions(4096, 4096, true, 4096),
            Some((4096, 4096))
        );
        assert_eq!(
            upload_dimensions(2048, 2048, true, 1024),
            Some((1024, 1024))
        );
        let mut card = image::RgbaImage::from_pixel(64, 64, image::Rgba([3, 9, 15, 0]));
        card.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        let decoded = gpu_diffuse(&card, false, 32).unwrap();
        assert_eq!(decoded.dimensions(), (32, 32));
        assert!(decoded.pixels().any(|px| px[3] < 255));
        assert!(decoded.pixels().any(|px| px[3] > 0));
    }

    #[test]
    fn mipmaps_are_bounded_and_accounted_for_in_shared_texture_budget() {
        assert_eq!(mip_chain_bytes(1, 1), 4);
        assert_eq!(mip_chain_bytes(4, 4), 64 + 16 + 4);
        assert!(mip_chain_bytes(4096, 4096) < 90 * 1024 * 1024);
        assert!(mip_chain_bytes(4096, 4096) > 64 * 1024 * 1024);
    }

    #[test]
    fn full_quality_retains_original_4096_model_textures_without_manager_budget() {
        assert_eq!(
            full_quality_dimensions(4096, 4096, false, 8192, true, 256 * 1024 * 1024),
            Some((4096, 4096))
        );
        assert_eq!(
            full_quality_dimensions(4096, 4096, false, 8192, false, 256 * 1024 * 1024),
            Some((2048, 2048))
        );
        assert_eq!(
            full_quality_dimensions(4096, 4096, false, 8192, true, 4 * 1024 * 1024),
            Some((4096, 4096))
        );
        assert_eq!(
            full_quality_dimensions(4096, 4096, false, 2048, true, 256 * 1024 * 1024),
            Some((2048, 2048))
        );
        assert_eq!(
            full_quality_dimensions(4096, 4096, false, 8192, true, 0),
            Some((4096, 4096))
        );
    }

    #[test]
    fn terrain_is_identified_by_nonselectable_ground_instance_not_asset_name() {
        let mesh = Arc::new(RenderMesh {
            info: evolve_core::ModelInfo {
                format: "test".into(),
                version: 1,
                chunk_count: 0,
                vertex_count: 0,
                triangle_count: 0,
                original_uv0: true,
                original_normals: true,
            },
            positions: Vec::new(),
            indices: Vec::new(),
            uv0: None,
            normals: None,
        });
        let mut group = ModelGroup {
            mesh,
            surfaces: Vec::new(),
            instances: vec![Instance {
                object_index: usize::MAX,
                transform: Transform::default(),
            }],
            radius: 0.0,
        };
        assert!(is_ground(&group));
        group.instances[0].object_index = 0;
        assert!(!is_ground(&group));
    }

    #[test]
    fn geometry_is_not_duplicated_for_instances() {
        let t = Transform::default();
        let a = InstanceGpu::from_transform(t);
        assert_eq!(a.cols[3], [0., 0., 0., 1.]);
        assert_eq!(a.cols[0], [1., 0., 0., 0.]);
    }
}
