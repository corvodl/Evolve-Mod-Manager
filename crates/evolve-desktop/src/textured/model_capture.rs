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
    // name|archive|cdf|yaw|pitch|zoom|pan x|pan y|armor|light|kala, separated by
    // semicolons. Add |clip virtual path|share of its duration to pose the character.
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
                    && !archive.builds.is_empty()
            })
            .cloned()
            .unwrap();
        let inventory = imported_archive_preview_inventory(&archive).unwrap();
        let source = Path::new(&inventory.root).join(fields[2]);
        let build = archive.builds.first().cloned();
        let root = Some(Path::new(&library_root));
        let is_monster = fields[2]
            .to_ascii_lowercase()
            .contains("characters/monsters/");
        let completed = crate::Operation::BuildStaticCdf(
            source,
            inventory.clone(),
            Some(Path::new(&library_root).to_path_buf()),
            archive.builds.first().cloned(),
            crate::asset_cache::PreviewCache::for_scene(),
            Default::default(),
            is_monster,
            kala_state(fields[10]),
            Arc::new(AtomicUsize::new(0)),
        )
        .execute();
        let Ok(crate::Completed::StaticCharacter(mesh, report, surfaces, warnings)) = completed
        else {
            panic!("{} did not load", fields[0]);
        };
        let mut framing = None;
        let mesh = if fields.len() > 12 {
            let _lookup = crate::model_preview_lookup(root, build.as_deref()).unwrap();
            let found =
                crate::auto_resources::discover(fields[2], &inventory, root, build.as_deref());
            let databases = found
                .clips
                .iter()
                .filter(|clip| clip.kind == "dba")
                .map(|clip| clip.virtual_path.clone())
                .collect::<Vec<_>>();
            let (_, motion) = crate::load_declared_motion(
                fields[11],
                &inventory,
                &databases,
                root,
                build.as_deref(),
            )
            .unwrap()
            .unwrap();
            let seconds = motion.duration_seconds() * number(12);
            let bound =
                evolve_core::VerifiedCafAssembly::from_parts(mesh, &report, motion).unwrap();
            framing = Some(bound.rest_bounds());
            bound.mesh_at_layers(seconds, &[], true).unwrap()
        } else {
            mesh
        };
        for warning in warnings.iter().filter(|w| w.contains("Glow")) {
            println!("MODEL {} {warning}", fields[0]);
        }
        let scene = Scene {
            mesh: Arc::new(mesh),
            framing,
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
            emissive_time: std::env::var("EVOLVE_CAPTURE_TIME")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1.0),
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

/// Triangle edges that a pose lengthened to over three times their rest length.
fn stretched_edges(
    rest: &evolve_core::RenderMesh,
    posed: &evolve_core::RenderMesh,
) -> (usize, usize) {
    let length = |points: &[[f32; 3]], a: u32, b: u32| {
        let (a, b) = (points[a as usize], points[b as usize]);
        (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>().sqrt()
    };
    let mut stretched = 0;
    for triangle in rest.indices.as_chunks::<3>().0 {
        for edge in 0..3 {
            let (a, b) = (triangle[edge], triangle[(edge + 1) % 3]);
            let (before, after) = (
                length(&rest.positions, a, b),
                length(&posed.positions, a, b),
            );
            stretched += usize::from(before > 1e-4 && after > 0.1 && after / before > 3.0);
        }
    }
    (stretched, rest.indices.len())
}

/// For each character in EVOLVE_CAPTURE_MODELS (name|archive|cdf), every declared
/// clip must load, bind and pose without tearing the mesh. A skin left in the wrong
/// space rotates about the wrong pivots and stretches thousands of edges.
#[test]
#[ignore]
fn real_character_clips_bind_and_keep_the_mesh_whole() {
    let library_root = std::env::var("EVOLVE_BENCH_LIBRARY").unwrap();
    let characters = std::env::var("EVOLVE_CAPTURE_MODELS").unwrap();
    let library = imported_library(Path::new(&library_root)).unwrap();
    let mut passed = true;
    for view in characters.split(';').filter(|view| !view.trim().is_empty()) {
        let fields = view.split('|').map(str::trim).collect::<Vec<_>>();
        let archive = library
            .archives
            .iter()
            .find(|archive| {
                archive.name.eq_ignore_ascii_case(fields[1])
                    && archive.source.to_ascii_lowercase().contains("moddedevolve")
                    && !archive.source.to_ascii_lowercase().contains("legacy")
                    && !archive.builds.is_empty()
            })
            .cloned()
            .unwrap();
        let inventory = imported_archive_preview_inventory(&archive).unwrap();
        let build = archive.builds.first().cloned();
        let root = Some(Path::new(&library_root));
        let _lookup = crate::model_preview_lookup(root, build.as_deref()).unwrap();
        let found = crate::auto_resources::discover(fields[2], &inventory, root, build.as_deref());
        println!(
            "CHARACTER {} clips {} warnings {:?}",
            fields[0],
            found.clips.len(),
            found.warnings
        );
        let databases = found
            .clips
            .iter()
            .filter(|clip| clip.kind == "dba")
            .map(|clip| clip.virtual_path.clone())
            .collect::<Vec<_>>();
        let source = Path::new(&inventory.root).join(fields[2]);
        let completed = crate::Operation::BuildStaticCdf(
            source,
            inventory.clone(),
            root.map(Path::to_path_buf),
            build.clone(),
            crate::asset_cache::PreviewCache::for_scene(),
            Default::default(),
            fields[2]
                .to_ascii_lowercase()
                .contains("characters/monsters/"),
            KalaTubingState::Banshee,
            Arc::new(AtomicUsize::new(0)),
        )
        .execute();
        let Ok(crate::Completed::StaticCharacter(mesh, report, _, _)) = completed else {
            panic!("{} did not load", fields[0]);
        };
        let (mut ok, mut missing, mut failed) = (0, 0, 0);
        let mut errors = std::collections::BTreeMap::<String, usize>::new();
        let mut torn = Vec::new();
        for clip in found.clips.iter().filter(|clip| clip.kind == "caf") {
            let motion = crate::load_declared_motion(
                &clip.virtual_path,
                &inventory,
                &databases,
                root,
                build.as_deref(),
            );
            let motion = match motion {
                Ok(Some((_, motion))) => motion,
                Ok(None) => {
                    println!("MISSING {}", clip.virtual_path);
                    missing += 1;
                    continue;
                }
                Err(error) => {
                    *errors.entry(error).or_default() += 1;
                    failed += 1;
                    continue;
                }
            };
            let duration = motion.duration_seconds();
            let bound = evolve_core::VerifiedCafAssembly::from_parts(mesh.clone(), &report, motion);
            let bound = match bound {
                Ok(bound) => bound,
                Err(error) => {
                    *errors.entry(error.to_string()).or_default() += 1;
                    failed += 1;
                    continue;
                }
            };
            match bound.mesh_at(duration * 0.5) {
                Ok(posed) => {
                    ok += 1;
                    let (stretched, edges) = stretched_edges(&mesh, &posed);
                    // The Gorgon's web sling is authored to stretch its limbs far.
                    let stretches = clip
                        .virtual_path
                        .to_ascii_lowercase()
                        .ends_with("lurker_web_sling.caf");
                    if stretched * 100 > edges && !stretches {
                        torn.push((clip.virtual_path.clone(), stretched));
                    }
                }
                Err(error) => {
                    *errors.entry(error.to_string()).or_default() += 1;
                    failed += 1;
                }
            }
        }
        println!(
            "RESULT {} parts {} verts {} ok {ok} missing {missing} failed {failed} torn {torn:?} errors {errors:#?}",
            fields[0],
            report.geometry_parts.len(),
            mesh.positions.len(),
        );
        passed &= ok > 0 && failed == 0 && missing == 0 && torn.is_empty();
    }
    assert!(passed);
}
