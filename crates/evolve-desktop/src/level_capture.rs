//! Offscreen Level Editor frame capture for visual parity checks. Ignored by
//! default: it reads a real imported library, which is never distributed, and
//! writes PNGs only to the folder named by EVOLVE_CAPTURE_OUT.
use crate::{level_camera, level_library, viewport_hdr, world_instances};
use evolve_core::imported_library;
use std::path::Path;
use std::sync::atomic::Ordering;

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;

/// Ground height at the mesh vertex nearest to an XY position.
fn ground_height(positions: &[[f32; 3]], x: f32, y: f32) -> f32 {
    positions
        .iter()
        .min_by(|a, b| {
            let da = (a[0] - x).powi(2) + (a[1] - y).powi(2);
            let db = (b[0] - x).powi(2) + (b[1] - y).powi(2);
            da.total_cmp(&db)
        })
        .map_or(0.0, |point| point[2])
}

#[test]
#[ignore]
fn capture_real_map_views() {
    let library_root = std::env::var("EVOLVE_BENCH_LIBRARY").unwrap();
    let map_name = std::env::var("EVOLVE_BENCH_MAP")
        .unwrap()
        .to_ascii_lowercase();
    let output = std::env::var("EVOLVE_CAPTURE_OUT").unwrap();
    let library = imported_library(Path::new(&library_root)).unwrap();
    let archive = library
        .archives
        .iter()
        .find(|archive| {
            archive
                .name
                .eq_ignore_ascii_case(&format!("levels_{map_name}.pak"))
                && archive.source.to_ascii_lowercase().contains("moddedevolve")
        })
        .cloned()
        .unwrap();
    let map = level_library::open(archive, 0).unwrap();
    for note in map.diagnostics.iter().filter(|note| {
        note.contains("global probe")
            || note.contains("looping particles")
            || note.contains("local probes")
            || note.contains("Particle")
            || note.contains("particle")
            || note.contains("water volumes")
            || note.contains("celestial")
            || note.contains("terrain color")
            || note.contains("olor chart")
            || note.contains("sun vector")
            || note.contains("Static Sky")
            || note.contains("Terrain.Layer")
    }) {
        println!("DIAG {note}");
    }
    let assembled = map.full_scene.as_ref().unwrap();
    let world = assembled.world.clone();
    let terrain = map
        .terrain
        .as_ref()
        .map(|(mesh, _)| mesh.positions.clone())
        .unwrap_or_default();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let instance = wgpu::Instance::default();
    let adapter = runtime
        .block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .unwrap();
    println!("ADAPTER {:?}", adapter.get_info().name);
    let (device, queue) = runtime
        .block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            // The limits the application's own device runs under, so a capture
            // fails on anything the real editor would reject.
            required_limits: wgpu::Limits {
                max_bind_groups: 2,
                max_non_sampler_bindings: 2048,
                ..wgpu::Limits::default()
            },
            ..Default::default()
        }))
        .unwrap();
    let mut renderer = world_instances::Renderer::new(&device, &queue, viewport_hdr::SCENE_FORMAT);
    let target_format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut hdr = viewport_hdr::ViewportHdr::new(&device, &queue, target_format);
    hdr.resize(&device, (WIDTH, HEIGHT));
    hdr.set_source_unlit(&queue, std::env::var("EVOLVE_CAPTURE_UNLIT").is_ok());
    let extent = wgpu::Extent3d {
        width: WIDTH,
        height: HEIGHT,
        depth_or_array_layers: 1,
    };
    let depth = device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("Capture depth"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default());
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Capture target"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: target_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&Default::default());
    // name,x,y,height above ground,yaw,pitch,hour separated by semicolons.
    let views = std::env::var("EVOLVE_CAPTURE_VIEWS").unwrap();
    let aspect = WIDTH as f32 / HEIGHT as f32;
    let mut uploaded = false;
    for view in views.split(';').filter(|view| !view.trim().is_empty()) {
        let fields = view.split(',').map(str::trim).collect::<Vec<_>>();
        let number = |index: usize| fields[index].parse::<f32>().unwrap();
        let (x, y, lift, yaw, pitch, hour) = (
            number(1),
            number(2),
            number(3),
            number(4),
            number(5),
            number(6),
        );
        let camera = level_camera::Camera {
            position: [x, y, ground_height(&terrain, x, y) + lift],
            ..Default::default()
        };
        hdr.set_grade(
            &device,
            &queue,
            world.color_chart.as_ref(),
            world.film_curve(hour),
            world.eye_adaptation(hour),
        );
        hdr.set_sun_shafts(
            &queue,
            world.sun_shafts(Some(camera), yaw, pitch, aspect, 1.0, hour),
        );
        hdr.reset_adaptation();
        let frames = if uploaded { 2 } else { 40_000 };
        for frame in 0..frames {
            renderer.prepare(
                &device,
                &queue,
                Some(&world),
                Some(camera),
                yaw,
                pitch,
                aspect,
                1.0,
                hour,
                true,
                std::env::var("EVOLVE_CAPTURE_UNLIT").is_ok(),
            );
            queue.submit([]);
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            if frame > 1 && world.gpu_progress.pending.load(Ordering::Relaxed) == 0 {
                break;
            }
        }
        uploaded = true;
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render_shadows(&mut encoder);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Capture scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: hdr.scene(),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            renderer.draw_sky(&mut pass);
            renderer.draw(&mut pass);
            renderer.draw_spotlight_beams(&mut pass);
            renderer.draw_light_points(&mut pass);
        }
        renderer.draw_water(&mut encoder, hdr.scene(), &depth, None);
        hdr.composite(
            &mut encoder,
            &target_view,
            iced::Rectangle {
                x: 0,
                y: 0,
                width: WIDTH,
                height: HEIGHT,
            },
        );
        let bytes_per_row = WIDTH * 4;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Capture readback"),
            size: u64::from(bytes_per_row * HEIGHT),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(HEIGHT),
                },
            },
            extent,
        );
        queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let pixels = readback.slice(..).get_mapped_range().to_vec();
        let path = Path::new(&output).join(format!("{map_name}_{}.png", fields[0]));
        image::RgbaImage::from_raw(WIDTH, HEIGHT, pixels)
            .unwrap()
            .save(&path)
            .unwrap();
        println!(
            "SAVED {} groups {}/{}",
            path.display(),
            world.gpu_progress.loaded.load(Ordering::Relaxed),
            world.groups.len()
        );
    }
}
