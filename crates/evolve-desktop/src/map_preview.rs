//! A map chooser preview built from the map's actual content: the hillshaded
//! terrain from [`crate::map_chooser::terrain_thumbnail`] plus every placed
//! object drawn from its source mesh triangles, not from generic dots.
//!
//! The terrain thumbnail already chooses a top-down projection (the two widest
//! axes, with the flattest axis as height). This module reproduces that exact
//! projection so placed geometry lands on the same pixels, then rasterises the
//! placements with a small z-buffer. Only CPU work and bounded budgets are
//! used; the thumbnail can be produced on a background worker without a GPU.

use crate::world_instances::{World, place};
use evolve_core::{RenderMesh, Transform};

/// The chooser's card thumbnail size, matching `map_chooser::THUMB_SIZE`.
const SIZE: usize = 160;
/// Hard budgets: a hostile or merely enormous map can never turn a card
/// thumbnail into an unbounded rasterisation pass.
const MAX_GROUPS: usize = 640;
const MAX_INSTANCES: usize = 4_000;
const MAX_TRIANGLES: usize = 150_000;
const MAX_PIXEL_TESTS: u64 = 4_000_000;
/// A single triangle projected wider than this is almost certainly a corrupt
/// transform rather than authored placement geometry.
const MAX_PROJECTED_SPAN: f32 = SIZE as f32 * 4.0;

const PALETTE: [[u8; 3]; 8] = [
    [235, 133, 52],
    [66, 178, 197],
    [219, 87, 113],
    [127, 190, 93],
    [176, 137, 219],
    [226, 192, 81],
    [80, 137, 224],
    [205, 102, 177],
];

/// The projection `terrain_thumbnail` uses: the two widest axes are the image
/// axes and the flattest axis is height. Recomputing the exact bounds here is
/// what lets placements share the terrain's pixel grid.
struct Projection {
    min: [f32; 3],
    span: [f32; 3],
    up: usize,
    u: usize,
    v: usize,
}

impl Projection {
    fn from_positions(positions: &[[f32; 3]]) -> Option<Self> {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        let mut any = false;
        for point in positions {
            if !point.iter().all(|value| value.is_finite()) {
                continue;
            }
            any = true;
            for axis in 0..3 {
                min[axis] = min[axis].min(point[axis]);
                max[axis] = max[axis].max(point[axis]);
            }
        }
        if !any {
            return None;
        }
        let span = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
        let up = (0..3)
            .min_by(|a, b| span[*a].total_cmp(&span[*b]))
            .unwrap_or(2);
        let (u, v) = match up {
            0 => (1, 2),
            1 => (0, 2),
            _ => (0, 1),
        };
        if !span[u].is_finite() || !span[v].is_finite() || span[u] <= 0.0 || span[v] <= 0.0 {
            return None;
        }
        Some(Self {
            min,
            span,
            up,
            u,
            v,
        })
    }

    /// World point to (pixel x, pixel y, height). `None` for non-finite input.
    fn project(&self, point: [f32; 3]) -> Option<(f32, f32, f32)> {
        if !point.iter().all(|value| value.is_finite()) {
            return None;
        }
        let scale = (SIZE - 1) as f32;
        let x = (point[self.u] - self.min[self.u]) / self.span[self.u] * scale;
        let y = scale - (point[self.v] - self.min[self.v]) / self.span[self.v] * scale;
        (x.is_finite() && y.is_finite() && point[self.up].is_finite()).then_some((
            x,
            y,
            point[self.up],
        ))
    }
}

fn is_ground(group: &crate::world_instances::ModelGroup) -> bool {
    group.instances.len() == 1 && group.instances[0].object_index == usize::MAX
}

/// Prefer the terrain the caller already decoded. When only a world is
/// available, its ground group is the same authored heightmap mesh.
fn terrain_mesh<'a>(
    terrain: Option<&'a RenderMesh>,
    world: Option<&'a World>,
) -> Option<&'a RenderMesh> {
    terrain
        .filter(|mesh| mesh.positions.len() >= 4)
        .or_else(|| {
            world?
                .groups
                .iter()
                .find(|group| is_ground(group))
                .map(|group| group.mesh.as_ref())
                .filter(|mesh| mesh.positions.len() >= 4)
        })
}

fn palette(group_index: usize) -> [u8; 3] {
    PALETTE[group_index % PALETTE.len()]
}

fn transform_is_sane(transform: Transform) -> bool {
    transform
        .position
        .iter()
        .chain(transform.quaternion.iter())
        .chain(transform.scale.iter())
        .all(|value| value.is_finite() && value.abs() <= 1.0e30)
}

/// Render a 160×160 PNG of the map from above. `None` when there is no usable
/// terrain to establish the projection, or when encoding fails.
pub(super) fn render(terrain: Option<&RenderMesh>, world: Option<&World>) -> Option<Vec<u8>> {
    let terrain = terrain_mesh(terrain, world)?;
    let projection = Projection::from_positions(&terrain.positions)?;
    let base = crate::map_chooser::terrain_thumbnail(&terrain.positions)?;
    let mut pixels = ::image::load_from_memory(&base).ok()?.to_rgba8();
    if pixels.dimensions() != (SIZE as u32, SIZE as u32) {
        pixels = ::image::imageops::resize(
            &pixels,
            SIZE as u32,
            SIZE as u32,
            ::image::imageops::FilterType::Triangle,
        );
    }
    if let Some(world) = world {
        draw_placements(&projection, world, &mut pixels);
    }
    encode(pixels)
}

fn encode(pixels: ::image::RgbaImage) -> Option<Vec<u8>> {
    let mut png = Vec::new();
    ::image::DynamicImage::ImageRgba8(pixels)
        .write_to(
            &mut std::io::Cursor::new(&mut png),
            ::image::ImageFormat::Png,
        )
        .ok()?;
    Some(png)
}

fn draw_placements(projection: &Projection, world: &World, pixels: &mut ::image::RgbaImage) {
    let mut depth = vec![f32::NEG_INFINITY; SIZE * SIZE];
    let mut triangles = 0usize;
    let mut instances = 0usize;
    let mut pixel_budget = MAX_PIXEL_TESTS;

    // Selectable authored objects first: merged bulk vegetation must not
    // exhaust the budgets before the map's buildings and props are drawn.
    'groups: for pass in 0..2 {
        for (group_index, group) in world.groups.iter().take(MAX_GROUPS).enumerate() {
            if is_ground(group) || group.mesh.positions.is_empty() || group.mesh.indices.len() < 3 {
                continue;
            }
            let color = palette(group_index);
            for instance in &group.instances {
                let bulk = instance.object_index == crate::world_instances::NOT_SELECTABLE;
                if (pass == 0) == bulk {
                    continue;
                }
                if instances >= MAX_INSTANCES {
                    break 'groups;
                }
                instances += 1;
                if !transform_is_sane(instance.transform) {
                    continue;
                }
                for triangle in group.mesh.indices.as_chunks::<3>().0 {
                    if triangles >= MAX_TRIANGLES || pixel_budget == 0 {
                        break 'groups;
                    }
                    triangles += 1;
                    let (Some(&ia), Some(&ib), Some(&ic)) = (
                        group.mesh.positions.get(triangle[0] as usize),
                        group.mesh.positions.get(triangle[1] as usize),
                        group.mesh.positions.get(triangle[2] as usize),
                    ) else {
                        continue;
                    };
                    let pa = place(instance.transform, ia);
                    let pb = place(instance.transform, ib);
                    let pc = place(instance.transform, ic);
                    let (Some(a), Some(b), Some(c)) = (
                        projection.project(pa),
                        projection.project(pb),
                        projection.project(pc),
                    ) else {
                        continue;
                    };
                    let edge1 = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
                    let edge2 = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
                    let normal = [
                        edge1[1] * edge2[2] - edge1[2] * edge2[1],
                        edge1[2] * edge2[0] - edge1[0] * edge2[2],
                        edge1[0] * edge2[1] - edge1[1] * edge2[0],
                    ];
                    let length =
                        (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2])
                            .sqrt();
                    if !length.is_finite() || length <= 1.0e-12 {
                        continue;
                    }
                    let facing = (normal[projection.up] / length).abs().clamp(0.0, 1.0);
                    let shade = 0.55 + 0.45 * facing;
                    let shaded = [
                        (color[0] as f32 * shade).min(255.0) as u8,
                        (color[1] as f32 * shade).min(255.0) as u8,
                        (color[2] as f32 * shade).min(255.0) as u8,
                    ];
                    rasterise(pixels, &mut depth, a, b, c, shaded, &mut pixel_budget);
                }
            }
        }
    }
}

/// Fill a projected triangle with a depth test. This is an orthographic top
/// view, so screen-space barycentrics also interpolate the world height.
fn rasterise(
    pixels: &mut ::image::RgbaImage,
    depth: &mut [f32],
    a: (f32, f32, f32),
    b: (f32, f32, f32),
    c: (f32, f32, f32),
    color: [u8; 3],
    pixel_budget: &mut u64,
) {
    let (min_x, max_x) = (a.0.min(b.0).min(c.0), a.0.max(b.0).max(c.0));
    let (min_y, max_y) = (a.1.min(b.1).min(c.1), a.1.max(b.1).max(c.1));
    let last = SIZE as f32 - 1.0;
    if !min_x.is_finite()
        || !max_x.is_finite()
        || !min_y.is_finite()
        || !max_y.is_finite()
        || max_x < 0.0
        || max_y < 0.0
        || min_x > last
        || min_y > last
        || max_x - min_x > MAX_PROJECTED_SPAN
        || max_y - min_y > MAX_PROJECTED_SPAN
    {
        return;
    }
    let area = (b.0 - a.0) * (c.1 - a.1) - (c.0 - a.0) * (b.1 - a.1);
    if !area.is_finite() || area.abs() <= 1.0e-6 {
        return;
    }
    let x0 = min_x.floor().max(0.0) as i32;
    let y0 = min_y.floor().max(0.0) as i32;
    let x1 = max_x.ceil().min(last) as i32;
    let y1 = max_y.ceil().min(last) as i32;
    for y in y0..=y1 {
        for x in x0..=x1 {
            if *pixel_budget == 0 {
                return;
            }
            *pixel_budget -= 1;
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let w0 = ((b.0 - px) * (c.1 - py) - (c.0 - px) * (b.1 - py)) / area;
            let w1 = ((c.0 - px) * (a.1 - py) - (a.0 - px) * (c.1 - py)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < -1.0e-4 || w1 < -1.0e-4 || w2 < -1.0e-4 {
                continue;
            }
            let height = w0 * a.2 + w1 * b.2 + w2 * c.2;
            if !height.is_finite() {
                continue;
            }
            let index = y as usize * SIZE + x as usize;
            if height <= depth[index] {
                continue;
            }
            depth[index] = height;
            pixels.put_pixel(
                x as u32,
                y as u32,
                ::image::Rgba([color[0], color[1], color[2], 255]),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world_instances::{Instance, ModelGroup};
    use evolve_core::ModelInfo;
    use std::sync::Arc;

    fn mesh(positions: Vec<[f32; 3]>, indices: Vec<u32>) -> RenderMesh {
        RenderMesh {
            info: ModelInfo {
                format: "test".into(),
                version: 1,
                chunk_count: 0,
                vertex_count: positions.len(),
                triangle_count: indices.len() / 3,
                original_uv0: false,
                original_normals: false,
            },
            positions,
            indices,
            uv0: None,
            normals: None,
            vertex_alpha: None,
            vertex_color: None,
        }
    }

    fn terrain_mesh() -> RenderMesh {
        let mut points = Vec::new();
        for y in 0..20 {
            for x in 0..20 {
                points.push([x as f32 * 10.0, y as f32 * 10.0, ((x * y) % 7) as f32]);
            }
        }
        mesh(points, Vec::new())
    }

    /// A flat 8×8 quad lying just above the terrain.
    fn object_mesh() -> RenderMesh {
        mesh(
            vec![
                [-4.0, -4.0, 0.0],
                [4.0, -4.0, 0.0],
                [4.0, 4.0, 0.0],
                [-4.0, 4.0, 0.0],
            ],
            vec![0, 1, 2, 0, 2, 3],
        )
    }

    fn world_at(position: [f32; 3]) -> World {
        let mesh = Arc::new(object_mesh());
        let mut world = World::default();
        world.groups = vec![ModelGroup {
            mesh,
            surfaces: Vec::new(),
            instances: vec![Instance {
                object_index: 0,
                transform: Transform {
                    position,
                    ..Transform::default()
                },
            }],
            radius: 8.0,
        }];
        world
    }

    fn pixels(png: &[u8]) -> ::image::RgbaImage {
        ::image::load_from_memory(png)
            .expect("preview is a PNG")
            .to_rgba8()
    }

    fn changed_pixels(a: &::image::RgbaImage, b: &::image::RgbaImage) -> usize {
        a.pixels().zip(b.pixels()).filter(|(a, b)| a != b).count()
    }

    #[test]
    fn preview_is_160_png() {
        let terrain = terrain_mesh();
        let png = render(Some(&terrain), None).expect("terrain preview");
        assert_eq!(&png[1..4], b"PNG");
        assert_eq!(pixels(&png).dimensions(), (SIZE as u32, SIZE as u32));
    }

    #[test]
    fn placed_mesh_triangles_are_visible_on_the_thumbnail() {
        let terrain = terrain_mesh();
        let base = pixels(&render(Some(&terrain), None).expect("terrain preview"));
        let world = world_at([50.0, 50.0, 10.0]);
        let with_objects =
            pixels(&render(Some(&terrain), Some(&world)).expect("placed object preview"));
        assert!(
            changed_pixels(&base, &with_objects) > 20,
            "the transformed source mesh must change the thumbnail"
        );
        // The placement is centred at (50, 50); its projection rounds to the
        // same pixel the terrain thumbnail uses for that world position.
        let projection = Projection::from_positions(&terrain.positions).unwrap();
        let (x, y, _) = projection.project([50.0, 50.0, 10.0]).unwrap();
        let pixel = with_objects.get_pixel(x as u32, y as u32);
        let base_pixel = base.get_pixel(x as u32, y as u32);
        assert_ne!(pixel, base_pixel);
    }

    #[test]
    fn moving_the_instance_moves_the_drawn_mesh() {
        let terrain = terrain_mesh();
        let a = pixels(
            &render(Some(&terrain), Some(&world_at([25.0, 25.0, 10.0]))).expect("left preview"),
        );
        let b = pixels(
            &render(Some(&terrain), Some(&world_at([150.0, 150.0, 10.0]))).expect("right preview"),
        );
        assert!(changed_pixels(&a, &b) > 20);
    }

    #[test]
    fn broken_geometry_and_huge_transforms_are_ignored() {
        let terrain = terrain_mesh();
        let mut bad = object_mesh();
        bad.positions[0] = [f32::NAN, f32::INFINITY, 0.0];
        bad.indices.extend([u32::MAX, 1, 2]);
        let bad_mesh = Arc::new(bad);
        let mut world = World::default();
        world.groups = vec![ModelGroup {
            mesh: bad_mesh,
            surfaces: Vec::new(),
            instances: vec![Instance {
                object_index: 0,
                transform: Transform {
                    position: [1.0e30, -1.0e30, 1.0e30],
                    quaternion: [f32::NAN, 0.0, 0.0, 1.0],
                    scale: [1.0e20; 3],
                },
            }],
            radius: 1.0e20,
        }];
        let png = render(Some(&terrain), Some(&world)).expect("safe fallback preview");
        assert_eq!(pixels(&png).dimensions(), (SIZE as u32, SIZE as u32));
        assert!(render(None, None).is_none());
        assert!(render(Some(&mesh(vec![[0.0, 0.0, 0.0]; 2], Vec::new())), None).is_none());
    }
}
