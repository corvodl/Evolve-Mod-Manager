//! Shared high-dynamic-range viewport frame and bounded bloom compositor.
//! Only internal wgpu resources are allocated. Models, PAK textures and world
//! instances stay in their original build-scoped sources.
use std::borrow::Cow;
use wgpu::util::DeviceExt;

pub(super) const SCENE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const BLOOM_DIVISOR: u32 = 4;

struct Frames {
    _shafts: wgpu::Texture,
    shafts: wgpu::TextureView,
    shaft_group: wgpu::BindGroup,
    _luminance: wgpu::Texture,
    luminance: wgpu::TextureView,
    size: (u32, u32),
    _scene: wgpu::Texture,
    scene: wgpu::TextureView,
    _bright: wgpu::Texture,
    bright: wgpu::TextureView,
    _horizontal: wgpu::Texture,
    horizontal: wgpu::TextureView,
    _blurred: wgpu::Texture,
    blurred: wgpu::TextureView,
    extract_group: wgpu::BindGroup,
    blur_horizontal_group: wgpu::BindGroup,
    blur_vertical_group: wgpu::BindGroup,
    composite_group: wgpu::BindGroup,
    _horizontal_uniform: wgpu::Buffer,
    _vertical_uniform: wgpu::Buffer,
}
pub(super) struct ViewportHdr {
    sampler: wgpu::Sampler,
    filter_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
    extract: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    luminance: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    composite_uniform: wgpu::Buffer,
    shaft_uniform: wgpu::Buffer,
    shafts: wgpu::RenderPipeline,
    chart_view: wgpu::TextureView,
    chart_identity: usize,
    frames: Frames,
}
fn surface(
    device: &wgpu::Device,
    size: (u32, u32),
    label: &'static str,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size.0.max(1),
            height: size.1.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SCENE_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}
fn bind_filter(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    uniform: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Viewport source/blur filter input"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: uniform.as_entire_binding(),
            },
        ],
    })
}
#[allow(clippy::too_many_arguments)]
fn bind_composite(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    scene: &wgpu::TextureView,
    blur: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    uniform: &wgpu::Buffer,
    chart: &wgpu::TextureView,
    luminance: &wgpu::TextureView,
    shafts: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("HDR scene and bloom into Iced window"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(scene),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(blur),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(chart),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::TextureView(luminance),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::TextureView(shafts),
            },
        ],
    })
}
/// A 256x16 color chart. With no map chart, an identity table leaves color as is.
fn upload_color_chart(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    chart: Option<&image::RgbaImage>,
) -> wgpu::TextureView {
    let identity;
    let pixels = match chart {
        Some(image) => image.as_raw().as_slice(),
        None => {
            identity = (0..256u32 * 16)
                .flat_map(|index| {
                    let (x, y) = (index % 256, index / 256);
                    [
                        (x % 16 * 17) as u8,
                        (y * 17) as u8,
                        (x / 16 * 17) as u8,
                        255,
                    ]
                })
                .collect::<Vec<u8>>();
            identity.as_slice()
        }
    };
    let size = wgpu::Extent3d {
        width: 256,
        height: 16,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Original map color chart"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(256 * 4),
            rows_per_image: Some(16),
        },
        size,
    );
    texture.create_view(&Default::default())
}
#[allow(clippy::too_many_arguments)]
fn build_frames(
    device: &wgpu::Device,
    size: (u32, u32),
    sampler: &wgpu::Sampler,
    filter_layout: &wgpu::BindGroupLayout,
    composite_layout: &wgpu::BindGroupLayout,
    composite_uniform: &wgpu::Buffer,
    chart: &wgpu::TextureView,
    shaft_uniform: &wgpu::Buffer,
) -> Frames {
    let size = (size.0.max(1), size.1.max(1));
    let small = (
        (size.0 / BLOOM_DIVISOR).max(1),
        (size.1 / BLOOM_DIVISOR).max(1),
    );
    let (_scene, scene) = surface(device, size, "HDR full resolution linear scene");
    let (_bright, bright) = surface(device, small, "Bloom quarter-resolution bright pass");
    let (_horizontal, horizontal) = surface(device, small, "Bloom horizontal Gaussian pass");
    let (_blurred, blurred) = surface(device, small, "Bloom vertical Gaussian pass");
    let (_luminance, luminance) = surface(device, (1, 1), "Scene mean log luminance");
    let (_shafts, shafts) = surface(device, small, "Sun shafts");
    let shaft_group = bind_filter(device, filter_layout, &scene, sampler, shaft_uniform);
    let upload = |values: [f32; 4], label| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::bytes_of(&values),
            usage: wgpu::BufferUsages::UNIFORM,
        })
    };
    let _horizontal_uniform = upload(
        [1.0 / small.0 as f32, 0.0, 0.0, 0.0],
        "Bloom horizontal texel direction",
    );
    let _vertical_uniform = upload(
        [0.0, 1.0 / small.1 as f32, 0.0, 0.0],
        "Bloom vertical texel direction",
    );
    let extract_group = bind_filter(device, filter_layout, &scene, sampler, &_horizontal_uniform);
    let blur_horizontal_group = bind_filter(
        device,
        filter_layout,
        &bright,
        sampler,
        &_horizontal_uniform,
    );
    let blur_vertical_group = bind_filter(
        device,
        filter_layout,
        &horizontal,
        sampler,
        &_vertical_uniform,
    );
    let composite_group = bind_composite(
        device,
        composite_layout,
        &scene,
        &blurred,
        sampler,
        composite_uniform,
        chart,
        &luminance,
        &shafts,
    );
    Frames {
        _shafts,
        shafts,
        shaft_group,
        _luminance,
        luminance,
        size,
        _scene,
        scene,
        _bright,
        bright,
        _horizontal,
        horizontal,
        _blurred,
        blurred,
        extract_group,
        blur_horizontal_group,
        blur_vertical_group,
        composite_group,
        _horizontal_uniform,
        _vertical_uniform,
    }
}
impl ViewportHdr {
    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Self {
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Clamp HDR bloom to viewport"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let sampled = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampling = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let uniform = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let filter_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("HDR bloom source/blur layout"),
            entries: &[sampled(0), sampling(1), uniform(2)],
        });
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("HDR bloom composite layout"),
            entries: &[
                sampled(0),
                sampled(1),
                sampling(2),
                uniform(3),
                sampled(4),
                sampled(6),
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let bloom_source = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Linear HDR bloom threshold and Gaussian blur"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("viewport_bloom.wgsl"))),
        });
        let composite_source = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Final HDR bloom composite and ACES tone mapping"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("viewport_hdr.wgsl"))),
        });
        let filter_pipeline = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Bloom filter pipeline layout"),
            bind_group_layouts: &[&filter_layout],
            push_constant_ranges: &[],
        });
        let composite_pipeline = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Final HDR to Iced window layout"),
            bind_group_layouts: &[&composite_layout],
            push_constant_ranges: &[],
        });
        let pipeline = |label,
                        layout: &wgpu::PipelineLayout,
                        source: &wgpu::ShaderModule,
                        fragment,
                        output| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: source,
                    entry_point: Some("vs_fullscreen"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: source,
                    entry_point: Some(fragment),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: output,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            })
        };
        let extract = pipeline(
            "HDR source bright threshold",
            &filter_pipeline,
            &bloom_source,
            "fs_extract",
            SCENE_FORMAT,
        );
        let blur = pipeline(
            "Quarter-res Gaussian HDR bloom",
            &filter_pipeline,
            &bloom_source,
            "fs_blur",
            SCENE_FORMAT,
        );
        let luminance = pipeline(
            "Scene mean log luminance",
            &filter_pipeline,
            &bloom_source,
            "fs_luminance",
            SCENE_FORMAT,
        );
        let shafts = pipeline(
            "Sun shafts",
            &filter_pipeline,
            &bloom_source,
            "fs_shafts",
            SCENE_FORMAT,
        );
        let composite = pipeline(
            "Linear HDR scene bloom composite",
            &composite_pipeline,
            &composite_source,
            "fs_composite",
            format,
        );
        let composite_uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("HDR bloom strength, exposure, bypass unlit"),
            // bloom strength=.20; manual exposure=1; bypass source color=0
            contents: bytemuck::cast_slice(&[
                [0.20_f32, 1.0, 0.0, 0.0],
                [1.0, 1.0, 1.0, 4.0],
                [0.18, 1.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 0.0],
            ]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let chart_view = upload_color_chart(device, queue, None);
        let shaft_uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Sun shaft screen position"),
            contents: bytemuck::cast_slice(&[0.0_f32; 4]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let frames = build_frames(
            device,
            (1, 1),
            &sampler,
            &filter_layout,
            &composite_layout,
            &composite_uniform,
            &chart_view,
            &shaft_uniform,
        );
        Self {
            sampler,
            filter_layout,
            composite_layout,
            extract,
            blur,
            luminance,
            composite,
            composite_uniform,
            shaft_uniform,
            shafts,
            chart_view,
            chart_identity: 0,
            frames,
        }
    }
    pub(super) fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        if self.frames.size == size {
            return;
        }
        self.frames = build_frames(
            device,
            size,
            &self.sampler,
            &self.filter_layout,
            &self.composite_layout,
            &self.composite_uniform,
            &self.chart_view,
            &self.shaft_uniform,
        );
    }
    /// Apply a map's authored film curve and color chart. Neutral with no map.
    pub(super) fn set_grade(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        chart: Option<&std::sync::Arc<image::RgbaImage>>,
        film: [f32; 4],
        adaptation: [f32; 4],
    ) {
        queue.write_buffer(&self.composite_uniform, 16, bytemuck::bytes_of(&film));
        queue.write_buffer(&self.composite_uniform, 32, bytemuck::bytes_of(&adaptation));
        let identity = chart.map_or(0, |image| std::sync::Arc::as_ptr(image) as usize);
        if identity == self.chart_identity {
            return;
        }
        self.chart_identity = identity;
        self.chart_view = upload_color_chart(device, queue, chart.map(|image| &**image));
        self.frames = build_frames(
            device,
            self.frames.size,
            &self.sampler,
            &self.filter_layout,
            &self.composite_layout,
            &self.composite_uniform,
            &self.chart_view,
            &self.shaft_uniform,
        );
    }
    /// The sun's screen position and authored shaft strength; zero turns off.
    pub(super) fn set_sun_shafts(&self, queue: &wgpu::Queue, shafts: [f32; 4]) {
        queue.write_buffer(&self.shaft_uniform, 0, bytemuck::cast_slice(&shafts));
        queue.write_buffer(
            &self.composite_uniform,
            48,
            bytemuck::cast_slice(&[shafts[2], 0.0, 0.0, 0.0]),
        );
    }
    pub(super) fn set_source_unlit(&self, queue: &wgpu::Queue, unlit: bool) {
        // Preserve the existing "Source RGB (unlit)" diagnostic: bypass bloom
        // and tonemapping, never grade source ground colors for comparisons.
        let values = [0.20_f32, 1.0, if unlit { 1.0 } else { 0.0 }, 0.0];
        queue.write_buffer(&self.composite_uniform, 0, bytemuck::bytes_of(&values));
    }
    pub(super) fn scene(&self) -> &wgpu::TextureView {
        &self.frames.scene
    }
    pub(super) fn composite(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip: iced::Rectangle<u32>,
    ) {
        if clip.width == 0 || clip.height == 0 {
            return;
        }
        // Bloom texture is a fraction of the full viewport, never a
        // high-resolution duplicate; all passes run without depth writes.
        for (label, view, pipeline, group) in [
            (
                "Scene mean log luminance",
                &self.frames.luminance,
                &self.luminance,
                &self.frames.extract_group,
            ),
            (
                "Sun shafts",
                &self.frames.shafts,
                &self.shafts,
                &self.frames.shaft_group,
            ),
            (
                "Bloom extract",
                &self.frames.bright,
                &self.extract,
                &self.frames.extract_group,
            ),
            (
                "Bloom horizontal",
                &self.frames.horizontal,
                &self.blur,
                &self.frames.blur_horizontal_group,
            ),
            (
                "Bloom vertical",
                &self.frames.blurred,
                &self.blur,
                &self.frames.blur_vertical_group,
            ),
        ] {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("HDR bloom tone map to Iced window"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_viewport(
            clip.x as f32,
            clip.y as f32,
            clip.width as f32,
            clip.height as f32,
            0.0,
            1.0,
        );
        pass.set_scissor_rect(clip.x, clip.y, clip.width, clip.height);
        pass.set_pipeline(&self.composite);
        pass.set_bind_group(0, &self.frames.composite_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn bloom_shader_has_extract_two_axis_blur_and_final_tonemap() {
        let bloom = include_str!("viewport_bloom.wgsl");
        let hdr = include_str!("viewport_hdr.wgsl");
        naga::front::wgsl::parse_str(bloom).expect("Bloom filter shader parses");
        naga::front::wgsl::parse_str(hdr).expect("HDR compositor shader parses");
        assert!(bloom.contains("fn fs_extract("));
        assert!(bloom.contains("fn fs_blur("));
        assert!(hdr.contains("fn fs_composite("));
        assert!(hdr.contains("film_map"));
    }
    #[test]
    fn masked_scene_refraction_shader_parses_without_original_game_shaders() {
        let shader = include_str!("viewport_refraction.wgsl");
        naga::front::wgsl::parse_str(shader).expect("depth-aware scene refraction shader parses");
        assert!(shader.contains("textureLoad(source_depth"));
        assert!(shader.contains("textureSampleLevel(authored_normal"));
        assert!(shader.contains("coverage.a*permitted"));
    }

    #[test]
    fn hdr_frame_format_is_floating_point() {
        assert_eq!(super::SCENE_FORMAT, wgpu::TextureFormat::Rgba16Float);
        assert_eq!(super::BLOOM_DIVISOR, 4);
    }
}
