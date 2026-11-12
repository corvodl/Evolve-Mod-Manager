//! Level-editor camera in original CryEngine world coordinates (XY, Z-up).
//! This is deliberately independent of the model-viewer's orbit controls.
use evolve_core::RenderMesh;
use std::collections::HashSet;

#[derive(Clone, Copy, Debug)]
pub(super) struct Camera {
    pub position: [f32; 3],
    pub speed: f32,
    pub fov_degrees: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            speed: 120.0,
            fov_degrees: 70.0,
        }
    }
}

impl Camera {
    /// Start over the center of the verified ground, rather than one map-width
    /// away or beneath it. Also works on maps with negative world coordinates.
    pub fn frame(mesh: &RenderMesh, yaw: f32, pitch: f32) -> Self {
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for point in &mesh.positions {
            for axis in 0..3 {
                if point[axis].is_finite() {
                    lo[axis] = lo[axis].min(point[axis]);
                    hi[axis] = hi[axis].max(point[axis]);
                }
            }
        }
        if !lo.iter().all(|x| x.is_finite()) || !hi.iter().all(|x| x.is_finite()) {
            return Self::default();
        }
        let center_xy = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5];
        let ground = mesh
            .positions
            .iter()
            .filter(|point| point.iter().all(|v| v.is_finite()))
            .min_by(|a, b| {
                let distance =
                    |p: &[f32; 3]| (p[0] - center_xy[0]).powi(2) + (p[1] - center_xy[1]).powi(2);
                distance(a).total_cmp(&distance(b))
            })
            .map_or((lo[2] + hi[2]) * 0.5, |point| point[2]);
        let extent = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1.0);
        let distance = (extent * 0.025).clamp(30.0, 150.0);
        let (forward, _, _) = basis(yaw, pitch);
        let mut position = [
            center_xy[0] - forward[0] * distance,
            center_xy[1] - forward[1] * distance,
            ground - forward[2] * distance,
        ];
        position[2] = position[2].max(ground + 20.0);
        Self {
            position,
            speed: (extent * 0.12).clamp(1.0, 2500.0),
            ..Self::default()
        }
    }

    pub fn focus(&mut self, point: [f32; 3], yaw: f32, pitch: f32, distance: f32) {
        let (forward, _, _) = basis(yaw, pitch);
        let distance = distance.clamp(0.1, 100_000.0);
        self.position = std::array::from_fn(|i| point[i] - forward[i] * distance);
    }

    /// Camera-relative first-person flight. W/S forward/back, A/D strafe,
    /// E/Q up/down in world Z; diagonals are normalized to constant speed.
    pub fn advance(&mut self, keys: &HashSet<char>, yaw: f32, pitch: f32, seconds: f32) {
        if !seconds.is_finite() || !(0.0..=0.1).contains(&seconds) {
            return;
        }
        let (forward, right, _) = basis(yaw, pitch);
        let axis = |a: char, b: char| -> f32 {
            let positive = if keys.contains(&a) { 1.0 } else { 0.0 };
            let negative = if keys.contains(&b) { 1.0 } else { 0.0 };
            positive - negative
        };
        let f = axis('w', 's');
        let r = axis('d', 'a');
        let z = axis('e', 'q');
        let vector = [
            f * forward[0] + r * right[0],
            f * forward[1] + r * right[1],
            f * forward[2] + z,
        ];
        let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm <= 1e-6 {
            return;
        }
        let distance = self.speed.clamp(0.1, 5000.0) * seconds / norm;
        for (coordinate, displacement) in self.position.iter_mut().zip(vector) {
            *coordinate += displacement * distance;
        }
    }

    /// Returns NDC X/Y and depth with a finite near clip. Geometry beyond the
    /// near plane needs proper clipping in the future GPU-instanced renderer.
    pub fn project(
        &self,
        point: [f32; 3],
        yaw: f32,
        pitch: f32,
        aspect: f32,
        zoom: f32,
    ) -> [f32; 3] {
        let (forward, right, up) = basis(yaw, pitch);
        let delta = std::array::from_fn::<_, 3, _>(|i| point[i] - self.position[i]);
        let dot = |vector: [f32; 3]| -> f32 { (0..3).map(|i| delta[i] * vector[i]).sum() };
        let depth = dot(forward);
        if !depth.is_finite() || depth <= 0.1 {
            return [3.0, 3.0, 1.0];
        }
        let fov = self.fov_degrees.clamp(25.0, 110.0).to_radians();
        let tangent = (fov * 0.5).tan() / zoom.clamp(0.2, 80.0);
        [
            dot(right) / (depth * tangent * aspect.max(0.01)),
            dot(up) / (depth * tangent),
            (1.0 - 0.1 / depth).clamp(0.0, 1.0),
        ]
    }
}

/// World camera basis. The same yaw/pitch axes are used by the original
/// renderer and the scene triangle picker.
/// The unit world ray through a viewport point given in normalized device
/// coordinates (-1..1, Y up), matching the renderer's projection.
pub(super) fn view_ray(
    camera: Camera,
    yaw: f32,
    pitch: f32,
    aspect: f32,
    zoom: f32,
    point: [f32; 2],
) -> Option<[f32; 3]> {
    let (forward, right, up) = basis(yaw, pitch);
    let t =
        (camera.fov_degrees.clamp(25.0, 110.0).to_radians() * 0.5).tan() / zoom.clamp(0.2, 80.0);
    let direction: [f32; 3] = std::array::from_fn(|i| {
        forward[i] + point[0] * t * aspect * right[i] + point[1] * t * up[i]
    });
    let length = direction.iter().map(|v| v * v).sum::<f32>().sqrt();
    (length > 1e-6 && length.is_finite()).then(|| direction.map(|v| v / length))
}

pub(super) fn basis(yaw: f32, pitch: f32) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let (sy, cy) = yaw.to_radians().sin_cos();
    let (sp, cp) = pitch.to_radians().sin_cos();
    (
        [sy * cp, cy * cp, sp],
        [cy, -sy, 0.0],
        [-sy * sp, -cy * sp, cp],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn start_camera_is_above_the_ground_near_the_map_center() {
        let mesh = RenderMesh {
            info: evolve_core::ModelInfo {
                format: String::new(),
                version: 0,
                chunk_count: 0,
                vertex_count: 5,
                triangle_count: 0,
                original_uv0: false,
                original_normals: false,
            },
            positions: vec![
                [-500.0, -500.0, -1000.0],
                [500.0, -500.0, 0.0],
                [-500.0, 500.0, 0.0],
                [500.0, 500.0, 0.0],
                [0.0, 0.0, 10.0],
            ],
            indices: vec![],
            uv0: None,
            normals: None,
            vertex_alpha: None,
            vertex_color: None,
        };
        let camera = Camera::frame(&mesh, -30.0, -32.0);
        assert!(camera.position[0].abs() < 150.0 && camera.position[1].abs() < 150.0);
        assert!(camera.position[2] >= 30.0);
        let target = camera.project([0.0, 0.0, 10.0], -30.0, -32.0, 1.0, 1.0);
        assert!(target[0].abs() < 1.0 && target[1].abs() < 1.0);
        assert!(target[2] > 0.0 && target[2] < 1.0);
    }

    #[test]
    #[ignore = "requires local read-only Tutorial Asset Library (EVOLVE_BENCH_LIBRARY)"]
    fn imported_tutorial_camera_starts_over_the_ground() {
        let library = std::env::var("EVOLVE_BENCH_LIBRARY").unwrap();
        let imports = evolve_core::imported_library(std::path::Path::new(&library)).unwrap();
        let archive = imports
            .archives
            .iter()
            .find(|pak| {
                pak.name.eq_ignore_ascii_case("levels_tutorial.pak")
                    && pak.source.to_ascii_lowercase().contains("moddedevolve")
            })
            .unwrap();
        let inventory = evolve_core::imported_archive_files(archive).unwrap();
        let member = inventory
            .entries
            .iter()
            .find(|entry| {
                entry
                    .relative_path
                    .eq_ignore_ascii_case("levels/Tutorial/leveldata/Heightmap.dat")
            })
            .unwrap();
        let terrain = evolve_core::read_terrain(
            &std::path::Path::new(&inventory.root).join(&member.relative_path),
            64,
        )
        .unwrap();
        let mesh = RenderMesh {
            info: evolve_core::ModelInfo {
                format: String::new(),
                version: 0,
                chunk_count: 0,
                vertex_count: terrain.vertices.len(),
                triangle_count: 0,
                original_uv0: false,
                original_normals: false,
            },
            positions: terrain.vertices,
            indices: vec![],
            uv0: None,
            normals: None,
            vertex_alpha: None,
            vertex_color: None,
        };
        let camera = Camera::frame(&mesh, -30.0, -32.0);
        let (lo, hi) = mesh.positions.iter().fold(
            ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]),
            |(mut lo, mut hi), point| {
                for i in 0..2 {
                    lo[i] = lo[i].min(point[i]);
                    hi[i] = hi[i].max(point[i]);
                }
                (lo, hi)
            },
        );
        let center = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5];
        let sample = mesh
            .positions
            .iter()
            .min_by(|a, b| {
                let distance =
                    |p: &[f32; 3]| (p[0] - center[0]).powi(2) + (p[1] - center[1]).powi(2);
                distance(a).total_cmp(&distance(b))
            })
            .unwrap();
        assert!(camera.position[2] >= sample[2] + 20.0);
        assert!((camera.position[0] - center[0]).abs() <= 150.0);
        assert!((camera.position[1] - center[1]).abs() <= 150.0);
        let projected = camera.project(*sample, -30.0, -32.0, 1.0, 1.0);
        assert!(projected[0].abs() < 1.0 && projected[1].abs() < 1.0);
    }
    #[test]
    fn first_person_camera_moves_in_view_direction_and_strafes() {
        let mut camera = Camera {
            speed: 10.0,
            ..Camera::default()
        };
        camera.advance(&HashSet::from(['w']), 0.0, 0.0, 1.0 / 10.0);
        assert!((camera.position[1] - 1.0).abs() < 0.001);
        camera.advance(&HashSet::from(['d']), 0.0, 0.0, 0.1);
        assert!((camera.position[0] - 1.0).abs() < 0.001);
        camera.advance(&HashSet::from(['e']), 0.0, 0.0, 0.1);
        assert!((camera.position[2] - 1.0).abs() < 0.001);
    }
    #[test]
    fn flight_stops_immediately_when_no_keys_remain() {
        let mut camera = Camera::default();
        let original = camera.position;
        camera.advance(&HashSet::from(['w']), 0.0, 0.0, 0.05);
        assert_ne!(camera.position, original);
        let released_position = camera.position;
        camera.advance(&HashSet::new(), 0.0, 0.0, 0.05);
        assert_eq!(camera.position, released_position);
    }

    #[test]
    fn camera_projects_forward_center_and_rejects_behind() {
        let camera = Camera::default();
        let target = camera.project([0.0, 10.0, 0.0], 0.0, 0.0, 1.0, 1.0);
        assert!(target[0].abs() < 0.01 && target[1].abs() < 0.01);
        assert!(target[2] > 0.0 && target[2] < 1.0);
        assert_eq!(
            camera.project([0.0, -10.0, 0.0], 0.0, 0.0, 1.0, 1.0),
            [3.0, 3.0, 1.0]
        );
    }
}
