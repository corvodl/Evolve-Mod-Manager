//! Level Editor timing bench. Ignored by default: it reads a real imported
//! library, which is never distributed. Run with
//! `EVOLVE_BENCH_LIBRARY=<library> EVOLVE_BENCH_MAP=<map> cargo test --release
//! -p evolve-mod-manager level_bench -- --ignored --nocapture`.
use crate::{level_camera, level_library, textured, viewport_hdr, world_instances};
use evolve_core::imported_library;
use iced::widget::shader::Program;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;

fn summary(label: &str, samples: &mut [Duration]) {
    samples.sort_unstable();
    let total: Duration = samples.iter().sum();
    let mean = total / samples.len().max(1) as u32;
    let p95 = samples[(samples.len() * 95 / 100).min(samples.len() - 1)];
    println!(
        "{label}: mean {:.2} ms, p95 {:.2} ms, max {:.2} ms ({} samples)",
        mean.as_secs_f64() * 1e3,
        p95.as_secs_f64() * 1e3,
        samples.last().unwrap().as_secs_f64() * 1e3,
        samples.len()
    );
}

#[test]
#[ignore]
fn level_frame_timings() {
    let library_root = std::env::var("EVOLVE_BENCH_LIBRARY").unwrap();
    let map_name = std::env::var("EVOLVE_BENCH_MAP")
        .unwrap()
        .to_ascii_lowercase();
    let library = imported_library(Path::new(&library_root)).unwrap();
    let archive = library
        .archives
        .iter()
        .find(|archive| {
            archive
                .name
                .eq_ignore_ascii_case(&format!("levels_{map_name}.pak"))
        })
        .cloned()
        .unwrap();

    let started = Instant::now();
    let map = level_library::open(archive, 0).unwrap();
    println!("LOAD open {:.0} ms", started.elapsed().as_secs_f64() * 1e3);
    level_library::LAST_LOAD_TIMING.with(|line| println!("LOAD {}", line.borrow()));
    let assembled = map.full_scene.as_ref().unwrap();
    let world = assembled.world.clone();
    println!(
        "SCENE {} objects, {} groups, {} instances, legacy mesh {} vertices / {} indices",
        map.scene.as_ref().map_or(0, |scene| scene.objects.len()),
        world.groups.len(),
        world.instance_count,
        assembled.mesh.positions.len(),
        assembled.mesh.indices.len()
    );
    let terrain = map.terrain.as_ref().map(|(mesh, _)| mesh).unwrap();
    let mut camera = level_camera::Camera::frame(terrain, -30.0, -32.0);
    let aspect = WIDTH as f32 / HEIGHT as f32;

    // Iced builds this primitive on the UI thread for every redraw.
    let scene = textured::Scene {
        mesh: std::sync::Arc::new(assembled.mesh.clone()),
        framing: None,
        surfaces: Some(std::sync::Arc::new(assembled.surfaces.clone())),
        diffuse: None,
        normal_map: None,
        emissive: None,
        specular: None,
        gloss: None,
        yaw: -30.0,
        pitch: -32.0,
        zoom: 1.0,
        light_strength: 1.0,
        emissive_gain: 1.0,
        armor_level: 1.0,
        emissive_time: 0.0,
        pan_x: 0.0,
        pan_y: 0.0,
        shift: false,
        level_camera: Some(camera),
        world: Some(world.clone()),
        level_hour: 12.0,
        level_full_textures: true,
        level_terrain_unlit: false,
        viewer_full_textures: true,
        viewer_gpu_remaining: None,
    };
    let bounds = iced::Rectangle::new(
        iced::Point::ORIGIN,
        iced::Size::new(WIDTH as f32, HEIGHT as f32),
    );
    let navigation = Default::default();
    let mut samples = (0..60)
        .map(|_| {
            let started = Instant::now();
            let primitive = <textured::Scene as Program<crate::Message>>::draw(
                &scene,
                &navigation,
                iced::mouse::Cursor::Unavailable,
                bounds,
            );
            std::hint::black_box(primitive);
            started.elapsed()
        })
        .collect::<Vec<_>>();
    summary("UI Scene::draw", &mut samples);

    // Every message rebuilds the view of each open window, so the cost of a
    // camera or animation tick includes both of these.
    let mut state = crate::State {
        model: Some(scene.mesh.clone()),
        level_scene: map.scene.clone().map(std::sync::Arc::new),
        level_surfaces: scene.surfaces.clone(),
        level_picks: Some(std::sync::Arc::new(assembled.picks.clone())),
        level_world_source: Some(world.clone()),
        level_camera: Some(camera),
        ..Default::default()
    };
    crate::refresh_wildlife_visibility(&mut state);
    let mut samples = (0..60)
        .map(|_| {
            let started = Instant::now();
            std::hint::black_box(crate::level_editor_contents(&state));
            started.elapsed()
        })
        .collect::<Vec<_>>();
    summary("UI level window view", &mut samples);
    let mut samples = (0..60)
        .map(|_| {
            let started = Instant::now();
            std::hint::black_box(crate::view(&state));
            started.elapsed()
        })
        .collect::<Vec<_>>();
    summary("UI main window view", &mut samples);

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
            required_limits: wgpu::Limits {
                max_bind_groups: 2,
                max_non_sampler_bindings: 2048,
                ..wgpu::Limits::default()
            },
            ..Default::default()
        }))
        .unwrap();
    let mut renderer = world_instances::Renderer::new(&device, &queue, viewport_hdr::SCENE_FORMAT);
    let target_format = wgpu::TextureFormat::Bgra8UnormSrgb;
    let mut hdr = viewport_hdr::ViewportHdr::new(&device, &queue, target_format);
    hdr.resize(&device, (WIDTH, HEIGHT));
    let extent = wgpu::Extent3d {
        width: WIDTH,
        height: HEIGHT,
        depth_or_array_layers: 1,
    };
    let depth = device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("Bench depth"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&Default::default());
    let target = device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("Bench target"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: target_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default());
    let clip = iced::Rectangle {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    };
    let frame = |renderer: &mut world_instances::Renderer,
                 camera: level_camera::Camera,
                 yaw: f32|
     -> (Duration, Duration) {
        let started = Instant::now();
        renderer.prepare(
            &device,
            &queue,
            Some(&world),
            Some(camera),
            yaw,
            -20.0,
            aspect,
            1.0,
            12.0,
            true,
            false,
        );
        let prepared = started.elapsed();
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render_shadows(&mut encoder);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Bench scene"),
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
            renderer.draw_wildlife_overlay(&mut pass);
        }
        renderer.draw_water(&mut encoder, hdr.scene(), &depth, Some(clip));
        hdr.composite(&mut encoder, &target, clip);
        queue.submit([encoder.finish()]);
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        (prepared, started.elapsed())
    };

    let started = Instant::now();
    let mut stream_prepare = Vec::new();
    for _ in 0..100_000 {
        let (prepared, _) = frame(&mut renderer, camera, -30.0);
        stream_prepare.push(prepared);
        if world.gpu_progress.pending.load(Ordering::Relaxed) == 0 {
            break;
        }
    }
    println!(
        "STREAM {} frames until resident, {:.0} ms wall, {} groups loaded",
        stream_prepare.len(),
        started.elapsed().as_secs_f64() * 1e3,
        world.gpu_progress.loaded.load(Ordering::Relaxed)
    );
    summary("STREAM prepare", &mut stream_prepare);

    // Fly through the map while turning: the cost of every redraw once
    // everything is resident, CPU prepare plus GPU completion.
    let mut prepare = Vec::new();
    let mut whole = Vec::new();
    for step in 0..240 {
        camera.position[0] += 4.0;
        camera.position[1] += 2.0;
        let (prepared, total) = frame(&mut renderer, camera, -30.0 + step as f32 * 1.5);
        prepare.push(prepared);
        whole.push(total);
    }
    summary("MOVING prepare (CPU)", &mut prepare);
    summary("MOVING frame (CPU+GPU)", &mut whole);
    let mut still = (0..120)
        .map(|_| frame(&mut renderer, camera, 0.0).1)
        .collect::<Vec<_>>();
    summary("STILL frame (CPU+GPU)", &mut still);
}
