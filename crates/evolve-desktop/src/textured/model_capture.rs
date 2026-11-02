//! Offscreen Model Viewer frame capture for visual parity checks. Ignored by
//! default: it reads a real imported library, which is never distributed, and
//! writes PNGs only to the folder named by EVOLVE_CAPTURE_OUT.
use super::{Pipeline, Scene};
use evolve_core::{KalaTubingState, imported_archive_preview_inventory, imported_library};
use iced::widget::shader::Program;
use iced::{Rectangle, mouse, wgpu};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;

fn kala_state(name: &str) -> KalaTubingState {
    match name {
        "armor" => KalaTubingState::ArmorReducer,
        "rift" => KalaTubingState::RiftDevice,
        _ => KalaTubingState::Banshee,
    }
}

#[test]
#[ignore]
fn capture_real_model_views() {
    let library_root = std::env::var("EVOLVE_BENCH_LIBRARY").unwrap();
    let output = std::env::var("EVOLVE_CAPTURE_OUT").unwrap();
    // name|archive|cdf|yaw|pitch|zoom|pan x|pan y|armor|light|kala, separated by semicolons.
    let views = std::env::var("EVOLVE_CAPTURE_MODELS").unwrap();
    let library = imported_library(Path::new(&library_root)).unwrap();

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
    let (device, queue) = runtime
        .block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            // The limits the application's own device runs under.
            required_limits: wgpu::Limits {
                max_bind_groups: 2,
                max_non_sampler_bindings: 2048,
                ..wgpu::Limits::default()
            },
            ..Default::default()
        }))
        .unwrap();
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let extent = wgpu::Extent3d {
        width: WIDTH,
        height: HEIGHT,
        depth_or_array_layers: 1,
    };
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Model capture target"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&Default::default());

    for view in views.split(';').filter(|view| !view.trim().is_empty()) {
        let fields = view.split('|').map(str::trim).collect::<Vec<_>>();
        let number = |index: usize| fields[index].parse::<f32>().unwrap();
        let archive = library
            .archives
            .iter()
            .find(|archive| {
                archive.name.eq_ignore_ascii_case(fields[1])
                    && archive.source.to_ascii_lowercase().contains("moddedevolve")
            })
            .cloned()
            .unwrap();
        let inventory = imported_archive_preview_inventory(&archive).unwrap();
        let source = Path::new(&inventory.root).join(fields[2]);
        let is_monster = fields[2]
            .to_ascii_lowercase()
            .contains("characters/monsters/");
        let completed = crate::Operation::BuildStaticCdf(
            source,
            inventory,
            Some(Path::new(&library_root).to_path_buf()),
            archive.builds.first().cloned(),
            crate::asset_cache::PreviewCache::for_scene(),
            Default::default(),
            is_monster,
            kala_state(fields[10]),
            Arc::new(AtomicUsize::new(0)),
        )
        .execute();
        let Ok(crate::Completed::StaticCharacter(mesh, _, surfaces, warnings)) = completed else {
            panic!("{} did not load", fields[0]);
        };
        for warning in warnings.iter().filter(|w| w.contains("Glow")) {
            println!("MODEL {} {warning}", fields[0]);
        }
        let scene = Scene {
            mesh: Arc::new(mesh),
            surfaces: Some(Arc::new(surfaces)),
            diffuse: None,
            normal_map: None,
            emissive: None,
            specular: None,
            gloss: None,
            yaw: crate::orientation_gizmo::FRONT_YAW + number(3),
            pitch: number(4),
            zoom: number(5),
            light_strength: number(9),
            emissive_gain: 1.0,
            armor_level: number(8),
            emissive_time: 0.0,
            pan_x: number(6),
            pan_y: number(7),
            shift: false,
            level_camera: None,
            world: None,
            level_hour: 12.0,
            level_full_textures: true,
            level_terrain_unlit: false,
            viewer_full_textures: true,
            viewer_gpu_remaining: None,
        };
        let bounds = Rectangle {
            x: 0.0,
            y: 0.0,
            width: WIDTH as f32,
            height: HEIGHT as f32,
        };
        let primitive = scene.draw(&Default::default(), mouse::Cursor::Unavailable, bounds);
        let mut pipeline = Pipeline::new_impl(&device, &queue, format);
        let parts = scene.surfaces.as_ref().map_or(0, |parts| parts.len());
        for _ in 0..=parts.max(1) * 2 {
            pipeline.prepare_scene(&device, &queue, (WIDTH, HEIGHT), &primitive);
            queue.submit([]);
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
            if pipeline.surface_upload_cursor >= parts {
                break;
            }
        }
        pipeline.prepare_scene(&device, &queue, (WIDTH, HEIGHT), &primitive);
        let mut encoder = device.create_command_encoder(&Default::default());
        pipeline.render_scene(
            &mut encoder,
            &target_view,
            Rectangle {
                x: 0,
                y: 0,
                width: WIDTH,
                height: HEIGHT,
            },
        );
        let bytes_per_row = WIDTH * 4;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Model capture readback"),
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
        let path = Path::new(&output).join(format!("{}.png", fields[0]));
        image::RgbaImage::from_raw(WIDTH, HEIGHT, pixels)
            .unwrap()
            .save(&path)
            .unwrap();
        println!(
            "SAVED {} surfaces {}/{parts}",
            path.display(),
            pipeline.surface_upload_cursor
        );
    }
}
