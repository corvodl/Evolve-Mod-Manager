//! Source-geometry 3D picking for the Level Editor. The renderer remains the
//! existing textured wgpu pipeline; this adds accurate triangle selection in
//! screen space rather than picking the nearest 2D overview dot.
use super::{level_scene_renderer::PickRange, textured, Message};
use evolve_core::RenderMesh;
use iced::{widget::shader, Event, Rectangle, mouse};
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct LevelViewport {
    pub inner: textured::Scene,
    pub picks: Arc<Vec<PickRange>>,
}

struct Projection {
    center: [f32; 3],
    extent: f32,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    pan_x: f32,
    pan_y: f32,
    width: f32,
    height: f32,
}
impl Projection {
    fn new(scene: &textured::Scene, bounds: Rectangle) -> Option<Self> {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for p in &scene.mesh.positions {
            for i in 0..3 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        }
        if !min.iter().chain(&max).all(|v| v.is_finite()) { return None }
        let center = std::array::from_fn(|i| (min[i] + max[i]) * 0.5);
        let extent = (0..3).map(|i| max[i] - min[i]).fold(1e-6f32, f32::max);
        Some(Self {
            center, extent,
            yaw: scene.yaw.to_radians(), pitch: scene.pitch.to_radians(),
            zoom: scene.zoom.clamp(0.2, 4.0),
            pan_x: scene.pan_x, pan_y: scene.pan_y,
            width: bounds.width.max(1.0), height: bounds.height.max(1.0),
        })
    }
    fn project(&self, raw: [f32; 3]) -> [f32; 3] {
        let x = (raw[0] - self.center[0]) / self.extent;
        let y = (raw[1] - self.center[1]) / self.extent;
        let z = (raw[2] - self.center[2]) / self.extent;
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let xr = x * cy - y * sy;
        let back = x * sy + y * cy;
        let yr = z * cp - back * sp;
        let depth = z * sp + back * cp;
        let perspective = (2.5 - depth).max(0.12);
        let scale = self.zoom * 1.6;
        let aspect = (self.width / self.height).max(0.01);
        let ndc_x = xr * scale / perspective / aspect + self.pan_x;
        let ndc_y = yr * scale / perspective + self.pan_y;
        [(ndc_x + 1.0) * self.width * 0.5, (1.0 - ndc_y) * self.height * 0.5, (2.5 - depth) / 5.0]
    }
}

fn hit_triangle(p: [f32; 2], a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> Option<f32> {
    let cross = |u: [f32;2], v: [f32;2]| u[0] * v[1] - u[1] * v[0];
    let ab = [b[0]-a[0], b[1]-a[1]];
    let ac = [c[0]-a[0], c[1]-a[1]];
    let ap = [p[0]-a[0], p[1]-a[1]];
    let area = cross(ab, ac);
    if area.abs() < 1e-7 { return None }
    let u = cross(ap, ac) / area;
    let v = cross(ab, ap) / area;
    if u < -1e-4 || v < -1e-4 || u + v > 1.0001 { return None }
    let d = a[2] * (1.0-u-v) + b[2] * u + c[2] * v;
    d.is_finite().then_some(d)
}

/// Click-only pixel triangle test. Index ranges are derived at assembly time
/// from the original object's exact mesh instances. Geometry, not the nearest
/// pivot or arbitrary object name, decides selection.
fn pick(mesh: &RenderMesh, ranges: &[PickRange], projector: &Projection, click: [f32; 2]) -> Option<usize> {
    let mut closest = (f32::INFINITY, None);
    // This bounded routine is for selection, not frame-by-frame rendering.
    // The source mesh stays immutable. Under heavy maps a future spatial BVH
    // will replace this budgeted exact subset test.
    let mut tested = 0usize;
    for range in ranges {
        let end = range.first_index.saturating_add(range.index_count).min(mesh.indices.len());
        for tri in mesh.indices[range.first_index.min(end)..end].chunks_exact(3) {
            if tested >= 300_000 { return closest.1 }
            tested += 1;
            let Some(a) = mesh.positions.get(tri[0] as usize) else { continue };
            let Some(b) = mesh.positions.get(tri[1] as usize) else { continue };
            let Some(c) = mesh.positions.get(tri[2] as usize) else { continue };
            let a = projector.project(*a);
            let b = projector.project(*b);
            let c = projector.project(*c);
            if let Some(distance) = hit_triangle(click, a, b, c)
                && distance >= 0.0 && distance < closest.0 {
                closest = (distance, Some(range.object_index));
            }
        }
    }
    closest.1
}

impl shader::Program<Message> for LevelViewport {
    type State = super::view_navigation::Navigation;
    type Primitive = textured::Primitive;
    fn update(&self, state: &mut Self::State, event: &Event, bounds: Rectangle, cursor: mouse::Cursor)
        -> Option<shader::Action<Message>> {
        // Initialise the existing orbit/pan gesture for subsequent mouse moves
        // even when an object is selected by this click.
        let navigation = state.handle(event, bounds, cursor, self.inner.shift);
        if matches!(event, Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)))
            && !self.inner.shift
            && let Some(position) = cursor.position_in(bounds)
            && let Some(projection) = Projection::new(&self.inner, bounds)
            && let Some(index) = pick(&self.inner.mesh, &self.picks, &projection, [position.x, position.y]) {
            return Some(shader::Action::publish(Message::LevelSelect(index)).and_capture());
        }
        navigation
            .map(super::view_navigation::message)
            .map(|message| shader::Action::publish(message).and_capture())
    }
    fn draw(&self, state: &Self::State, cursor: mouse::Cursor, bounds: Rectangle) -> Self::Primitive {
        <textured::Scene as shader::Program<Message>>::draw(&self.inner, state, cursor, bounds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn triangle_hit_uses_barycentric_depth() {
        let a = [0.0, 0.0, 0.4];
        let b = [100.0, 0.0, 0.4];
        let c = [0.0, 100.0, 0.4];
        assert!(hit_triangle([25.0, 25.0], a, b, c).is_some());
        assert!(hit_triangle([90.0, 90.0], a, b, c).is_none());
    }
}
