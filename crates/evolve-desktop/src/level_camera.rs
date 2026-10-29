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
    /// Derive a starting position from the verified geometry bounds, not a
    /// guessed Desert Canyon origin. Also works on maps with negative coords.
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
        let center: [f32; 3] = std::array::from_fn(|axis| (lo[axis] + hi[axis]) * 0.5);
        let extent = (0..3)
            .map(|axis| hi[axis] - lo[axis])
            .fold(1.0f32, f32::max);
        let distance = (extent * 1.45).max(10.0);
        let (forward, _, _) = basis(yaw, pitch);
        let position = std::array::from_fn(|i| center[i] - forward[i] * distance);
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
        let mut vector = [
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
    fn first_person_camera_moves_in_view_direction_and_strafes() {
        let mut camera = Camera { speed: 10.0, ..Camera::default() };
        camera.advance(&HashSet::from(['w']), 0.0, 0.0, 1.0 / 10.0);
        assert!((camera.position[1] - 1.0).abs() < 0.001);
        camera.advance(&HashSet::from(['d']), 0.0, 0.0, 0.1);
        assert!((camera.position[0] - 1.0).abs() < 0.001);
        camera.advance(&HashSet::from(['e']), 0.0, 0.0, 0.1);
        assert!((camera.position[2] - 1.0).abs() < 0.001);
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
