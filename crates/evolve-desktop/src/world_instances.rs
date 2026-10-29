//! Dedicated level-world GPU renderer. Model geometry is uploaded once per
//! exact virtual source/material pair; authored placements remain a separate
//! instanced vertex stream (not baked into one enormous CPU mesh).
use crate::{level_camera, textured::Surface};
use evolve_core::{RenderMesh, Transform};
use iced::wgpu;
use iced::wgpu::util::{DeviceExt, TextureDataOrder};
use std::sync::Arc;

#[derive(Clone, Debug)]
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
#[derive(Clone, Debug, Default)]
pub(super) struct World {
    pub groups: Vec<ModelGroup>,
    pub instance_count: usize,
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
        let norm = (w*w + x*x + y*y + z*z).sqrt();
        if norm > 1.0e-8 {
            w /= norm; x /= norm; y /= norm; z /= norm;
        } else {
            w=1.0; x=0.0; y=0.0; z=0.0;
        }
        let [sx,sy,sz] = t.scale;
        Self { cols: [
            [(1.0-2.0*(y*y+z*z))*sx, (2.0*(x*y+w*z))*sx, (2.0*(x*z-w*y))*sx, 0.0],
            [(2.0*(x*y-w*z))*sy, (1.0-2.0*(x*x+z*z))*sy, (2.0*(y*z+w*x))*sy, 0.0],
            [(2.0*(x*z+w*y))*sz, (2.0*(y*z-w*x))*sz, (1.0-2.0*(x*x+y*y))*sz, 0.0],
            [t.position[0],t.position[1],t.position[2],1.0],
        ] }
    }
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, PartialEq)]
struct CameraGpu {
    origin: [f32;4],
    forward: [f32;4],
    right: [f32;4],
    up: [f32;4],
}
impl CameraGpu {
    fn make(camera: level_camera::Camera, yaw: f32, pitch: f32, aspect: f32, zoom: f32) -> Self {
        let (f,r,u) = level_camera::basis(yaw,pitch);
        let t = (camera.fov_degrees.clamp(25.0,110.0).to_radians()*0.5).tan()
            / zoom.clamp(0.2,80.0);
        Self {
            origin: [camera.position[0],camera.position[1],camera.position[2],0.0],
            forward: [f[0],f[1],f[2],0.1],
            right: [r[0],r[1],r[2],(t*aspect.max(0.01)).max(0.0001)],
            up: [u[0],u[1],u[2],t.max(0.0001)],
        }
    }
}
pub(super) fn radius(mesh: &RenderMesh) -> f32 {
    mesh.positions.iter().map(|v| v.iter().map(|f| f*f).sum::<f32>().sqrt())
        .filter(|r| r.is_finite()).fold(0.0f32, f32::max)
}
/// Coarse conservative frustum culling before instance-buffer uploads.
/// Does not make distant vegetation disappear arbitrarily by count.
pub(super) fn visible(
    group: &ModelGroup,
    camera: level_camera::Camera,
    yaw: f32,
    pitch: f32,
    aspect: f32,
    zoom: f32,
) -> Vec<InstanceGpu> {
    let (f,r,u)=level_camera::basis(yaw,pitch);
    let tangent=(camera.fov_degrees.clamp(25.0,110.0).to_radians()*0.5).tan()
        / zoom.clamp(0.2,80.0);
    group.instances.iter().filter_map(|instance|{
        let p=instance.transform.position;
        let delta=[p[0]-camera.position[0],p[1]-camera.position[1],p[2]-camera.position[2]];
        let dot=|v:[f32;3]| delta.iter().zip(v).map(|(a,b)|a*b).sum::<f32>();
        let d=dot(f);
        let scale=instance.transform.scale.iter().copied().map(f32::abs).fold(0.0,f32::max);
        let radius=group.radius*scale;
        if !d.is_finite() || !radius.is_finite() || d+radius < 0.1
           || d-radius > 50000.0
           || dot(r).abs() > d.max(0.0)*tangent*aspect.max(0.01)+radius*1.5
           || dot(u).abs() > d.max(0.0)*tangent+radius*1.5
        { return None; }
        Some(InstanceGpu::from_transform(instance.transform))
    }).collect()
}
struct GpuSurface {
    first: u32,
    count: u32,
    transparent: bool,
    bind: wgpu::BindGroup,
    _image: wgpu::Texture,
}
struct GpuGroup {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    instances: wgpu::Buffer,
    capacity: usize,
    visible: u32,
    surfaces: Vec<GpuSurface>,
}
pub(super) struct Renderer {
    solid: wgpu::RenderPipeline,
    transparent: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    camera_uniform: wgpu::Buffer,
    sampler: wgpu::Sampler,
    world_identity: usize,
    groups: Vec<GpuGroup>,
    last_camera: Option<CameraGpu>,
}
impl Renderer {
    pub fn new(device:&wgpu::Device, format:wgpu::TextureFormat) -> Self {
        let layout=device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor{
            label:Some("Original level model diffuse bindings"),
            entries:&[
                wgpu::BindGroupLayoutEntry{binding:0,visibility:wgpu::ShaderStages::VERTEX,
                    ty:wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset:false,min_binding_size:None},count:None},
                wgpu::BindGroupLayoutEntry{binding:1,visibility:wgpu::ShaderStages::FRAGMENT,
                    ty:wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),count:None},
                wgpu::BindGroupLayoutEntry{binding:2,visibility:wgpu::ShaderStages::FRAGMENT,
                    ty:wgpu::BindingType::Texture{sample_type:wgpu::TextureSampleType::Float{filterable:true},
                    view_dimension:wgpu::TextureViewDimension::D2,multisampled:false},count:None},
            ],
        });
        let camera_uniform=device.create_buffer(&wgpu::BufferDescriptor{
            label:Some("Level fly camera (GPU)"),size:64,
            usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST,
            mapped_at_creation:false,
        });
        let sampler=device.create_sampler(&wgpu::SamplerDescriptor{
            address_mode_u:wgpu::AddressMode::Repeat,
            address_mode_v:wgpu::AddressMode::Repeat,
            mag_filter:wgpu::FilterMode::Linear,min_filter:wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shader=device.create_shader_module(wgpu::ShaderModuleDescriptor{
            label:Some("CryEngine instanced Level Editor"),
            source:wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!("world_instances.wgsl"))),
        });
        let pipe_layout=device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor{
            label:Some("Instanced level model pipeline"),bind_group_layouts:&[&layout],
            push_constant_ranges:&[],
        });
        let make=|blend:bool|device.create_render_pipeline(&wgpu::RenderPipelineDescriptor{
            label:Some(if blend{"Transparent instanced world geometry"}else{"Opaque instanced world geometry"}),
            layout:Some(&pipe_layout),
            vertex:wgpu::VertexState{module:&shader,entry_point:Some("vs_main"),
                buffers:&[Vertex::layout(),InstanceGpu::layout()],
                compilation_options:wgpu::PipelineCompilationOptions::default()},
            fragment:Some(wgpu::FragmentState{module:&shader,entry_point:Some("fs_main"),
                targets:&[Some(wgpu::ColorTargetState{format,
                    blend:if blend{Some(wgpu::BlendState::ALPHA_BLENDING)}else{None},
                    write_mask:wgpu::ColorWrites::ALL})],
                compilation_options:wgpu::PipelineCompilationOptions::default()}),
            primitive:wgpu::PrimitiveState{cull_mode:None,..Default::default()},
            depth_stencil:Some(wgpu::DepthStencilState{
                format:wgpu::TextureFormat::Depth32Float,
                depth_write_enabled:!blend,depth_compare:wgpu::CompareFunction::Less,
                stencil:wgpu::StencilState::default(),bias:wgpu::DepthBiasState::default(),
            }),
            multisample:wgpu::MultisampleState::default(),multiview:None,cache:None,
        });
        Self {solid:make(false),transparent:make(true),layout,camera_uniform,sampler,
            world_identity:0,groups:Vec::new(),last_camera:None}
    }
    fn group(&self,device:&wgpu::Device,queue:&wgpu::Queue,model:&ModelGroup)->GpuGroup {
        let vert:Vec<Vertex>=model.mesh.positions.iter().enumerate().map(|(i,&xyz)|Vertex{
            xyz,
            uv:model.mesh.uv0.as_ref().and_then(|s|s.get(i)).copied().unwrap_or([0.0;2]),
            normal:model.mesh.normals.as_ref().and_then(|s|s.get(i)).copied().unwrap_or([0.0,0.0,1.0]),
        }).collect();
        let vertices=device.create_buffer_init(&wgpu::util::BufferInitDescriptor{
            label:Some("Unique original world mesh vertices"),
            contents:bytemuck::cast_slice(&vert),usage:wgpu::BufferUsages::VERTEX,
        });
        let indices=device.create_buffer_init(&wgpu::util::BufferInitDescriptor{
            label:Some("Unique original world mesh indices"),
            contents:bytemuck::cast_slice(&model.mesh.indices),usage:wgpu::BufferUsages::INDEX,
        });
        let capacity=model.instances.len().max(1);
        let instances=device.create_buffer(&wgpu::BufferDescriptor{
            label:Some("Visible world-placement transforms"),
            size:(capacity*std::mem::size_of::<InstanceGpu>()) as u64,
            usage:wgpu::BufferUsages::VERTEX|wgpu::BufferUsages::COPY_DST,
            mapped_at_creation:false,
        });
        let mut surfaces=Vec::new();
        let fallback=[210u8,205,205,255];
        for surf in &model.surfaces {
            if !surf.visible || surf.index_count==0 || surf.first_index as usize+
                surf.index_count as usize > model.mesh.indices.len(){continue;}
            let source=surf.diffuse.as_ref().filter(|img|{
                (img.width() as u64)*(img.height() as u64)*4 <= 32*1024*1024
            });
            let (w,h,bytes) = source.map(|img|(img.width(),img.height(),img.as_raw().as_slice()))
                .unwrap_or((1,1,&fallback[..]));
            let tex=device.create_texture_with_data(queue,&wgpu::TextureDescriptor{
                label:Some("Original world-model diffuse DDS"),
                size:wgpu::Extent3d{width:w,height:h,depth_or_array_layers:1},
                mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,
                format:wgpu::TextureFormat::Rgba8UnormSrgb,
                usage:wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_DST,
                view_formats:&[],
            },TextureDataOrder::LayerMajor,bytes);
            let view=tex.create_view(&wgpu::TextureViewDescriptor::default());
            let bind=device.create_bind_group(&wgpu::BindGroupDescriptor{
                label:Some("Exact source material per instanced mesh"),
                layout:&self.layout,
                entries:&[
                    wgpu::BindGroupEntry{binding:0,resource:self.camera_uniform.as_entire_binding()},
                    wgpu::BindGroupEntry{binding:1,resource:wgpu::BindingResource::Sampler(&self.sampler)},
                    wgpu::BindGroupEntry{binding:2,resource:wgpu::BindingResource::TextureView(&view)},
                ],
            });
            surfaces.push(GpuSurface{first:surf.first_index,count:surf.index_count,
                transparent:surf.transparent,bind,_image:tex});
        }
        if surfaces.is_empty() {
            // NoDraw-only source geometry must stay invisible, even when a
            // decoder exposes indices for its hidden material subsets.
        }
        GpuGroup{vertices,indices,instances,capacity,visible:0,surfaces}
    }
    pub fn prepare(&mut self,device:&wgpu::Device,queue:&wgpu::Queue,world:Option<&Arc<World>>,
        camera:Option<level_camera::Camera>,yaw:f32,pitch:f32,aspect:f32,zoom:f32) {
        let Some(world)=world else {
            self.groups.clear();self.world_identity=0;self.last_camera=None;
            return;
        };
        let key=Arc::as_ptr(world) as usize;
        if key != self.world_identity {
            self.groups=world.groups.iter().map(|g|self.group(device,queue,g)).collect();
            self.world_identity=key;self.last_camera=None;
        }
        let Some(camera)=camera else {return;};
        let uniform=CameraGpu::make(camera,yaw,pitch,aspect,zoom);
        if self.last_camera==Some(uniform){return;}
        queue.write_buffer(&self.camera_uniform,0,bytemuck::bytes_of(&uniform));
        for (gpu,group) in self.groups.iter_mut().zip(&world.groups) {
            let culled=visible(group,camera,yaw,pitch,aspect,zoom);
            gpu.visible=culled.len().min(gpu.capacity) as u32;
            if !culled.is_empty() {
                queue.write_buffer(&gpu.instances,0,bytemuck::cast_slice(&culled[..gpu.visible as usize]));
            }
        }
        self.last_camera=Some(uniform);
    }
    pub fn draw<'a>(&'a self,pass:&mut wgpu::RenderPass<'a>) {
        for transparent in [false,true] {
            pass.set_pipeline(if transparent{&self.transparent}else{&self.solid});
            for group in &self.groups {
                if group.visible==0{continue;}
                pass.set_vertex_buffer(0,group.vertices.slice(..));
                pass.set_vertex_buffer(1,group.instances.slice(..));
                pass.set_index_buffer(group.indices.slice(..),wgpu::IndexFormat::Uint32);
                for surf in group.surfaces.iter().filter(|s|s.transparent==transparent) {
                    pass.set_bind_group(0,&surf.bind,&[]);
                    pass.draw_indexed(surf.first..surf.first+surf.count,0,0..group.visible);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn geometry_is_not_duplicated_for_instances() {
        let t=Transform::default();
        let a=InstanceGpu::from_transform(t);
        assert_eq!(a.cols[3],[0.,0.,0.,1.]);
        assert_eq!(a.cols[0],[1.,0.,0.,0.]);
    }
}
