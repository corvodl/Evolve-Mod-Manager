//! Interactive CryEngine geometry wireframe view on Iced's wgpu-backed canvas.
//! Renderable source triangles only; no invented skin weights/materials and
//! no claim of full shader-based character rendering.
use evolve_core::RenderMesh;
use iced::widget::canvas::{self, Path, Stroke};
use iced::{Color, Event, Point, Rectangle, Renderer, Theme, mouse};
use std::sync::Arc;

#[derive(Clone)]
pub struct MeshViewport {
    pub mesh: Arc<RenderMesh>,
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    pub pan_x: f32,
    pub pan_y: f32,
    pub shift: bool,
}

impl canvas::Program<super::Message> for MeshViewport {
    type State = super::view_navigation::Navigation;
    fn update(
        &self,
        state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<super::Message>> {
        state
            .handle(event, bounds, cursor, self.shift)
            .map(super::view_navigation::message)
            .map(|message| canvas::Action::publish(message).and_capture())
    }
    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, frame.size(), Color::from_rgb8(19, 19, 24));
        if bounds.width < 5. || bounds.height < 5. || self.mesh.indices.len() < 3 {
            return vec![frame.into_geometry()];
        }
        let mut minimum = [f32::MAX; 3];
        let mut maximum = [f32::MIN; 3];
        for v in &self.mesh.positions {
            for c in 0..3 {
                minimum[c] = minimum[c].min(v[c]);
                maximum[c] = maximum[c].max(v[c]);
            }
        }
        let center = [
            (minimum[0] + maximum[0]) * 0.5,
            (minimum[1] + maximum[1]) * 0.5,
            (minimum[2] + maximum[2]) * 0.5,
        ];
        let extent = (0..3)
            .map(|c| maximum[c] - minimum[c])
            .fold(0f32, f32::max)
            .max(0.00001);
        let (yaw_sin, yaw_cos) = self.yaw.to_radians().sin_cos();
        let (pitch_sin, pitch_cos) = self.pitch.to_radians().sin_cos();
        let scale = bounds.width.min(bounds.height) * 0.65 * self.zoom.clamp(0.2, 4.0);
        let project = |p: [f32; 3]| -> Option<Point> {
            let x = (p[0] - center[0]) / extent;
            let y = (p[1] - center[1]) / extent;
            let z = (p[2] - center[2]) / extent;
            // Same right-handed camera as the textured GPU viewport:
            // looking from +Y toward -Y puts world +X on screen-left.
            let horiz = y * yaw_sin - x * yaw_cos;
            let depth = x * yaw_sin + y * yaw_cos;
            let vertical = z * pitch_cos - depth * pitch_sin;
            let behind = z * pitch_sin + depth * pitch_cos;
            let perspective = (2.5 - behind).max(0.1);
            let rx = bounds.width * (0.5 + self.pan_x * 0.5) + horiz * scale / perspective;
            let ry = bounds.height * (0.5 - self.pan_y * 0.5) - vertical * scale / perspective;
            if rx.is_finite() && ry.is_finite() {
                Some(Point::new(rx, ry))
            } else {
                None
            }
        };
        // Ground plane for the historical 2012 wireframe fallback uses
        // the same source-space feet as the textured GPU viewport.
        let floor = super::viewer_grid::floor_lines(&self.mesh.positions);
        for kind in 0..4 {
            let plane = Path::new(|path| {
                for (i, line) in floor.iter().enumerate() {
                    let category = if i == 40 {
                        2
                    } else if i == 41 {
                        3
                    } else if (i / 2) % 5 == 0 {
                        1
                    } else {
                        0
                    };
                    if category != kind {
                        continue;
                    }
                    if let (Some(a), Some(b)) = (project(line.start), project(line.end)) {
                        path.move_to(a);
                        path.line_to(b);
                    }
                }
            });
            let (color, width) = match kind {
                0 => (Color::from_rgba(0.42, 0.46, 0.51, 0.22), 0.65),
                1 => (Color::from_rgba(0.52, 0.56, 0.61, 0.4), 0.9),
                2 => (Color::from_rgba(0.79, 0.30, 0.34, 0.75), 1.2),
                _ => (Color::from_rgba(0.35, 0.73, 0.46, 0.75), 1.2),
            };
            frame.stroke(
                &plane,
                Stroke::default().with_color(color).with_width(width),
            );
        }
        // Cap the displayed triangles to keep UI responsive even when real
        // Stage 2 character meshes contain hundreds of thousands of faces.
        let count = self.mesh.indices.len() / 3;
        let step = count.div_ceil(16000).max(1);
        let source = &self.mesh.positions;
        let lines = Path::new(|path| {
            for tri in self.mesh.indices.as_chunks::<3>().0.iter().step_by(step) {
                let a = source.get(tri[0] as usize).copied().and_then(project);
                let b = source.get(tri[1] as usize).copied().and_then(project);
                let c = source.get(tri[2] as usize).copied().and_then(project);
                if let (Some(a), Some(b), Some(c)) = (a, b, c) {
                    path.move_to(a);
                    path.line_to(b);
                    path.line_to(c);
                    path.close();
                }
            }
        });
        frame.stroke(
            &lines,
            Stroke::default()
                .with_color(Color::from_rgb8(217, 62, 79))
                .with_width(0.75),
        );
        vec![frame.into_geometry()]
    }
}
