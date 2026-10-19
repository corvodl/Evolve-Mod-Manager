//! Source UV0 visualization drawn by Iced's GPU-backed canvas. Repeat-tiled UVs
//! are displayed without modifying coordinates or pretending to unwrap them.
use evolve_core::RenderMesh;
use iced::widget::canvas::{self, Path, Stroke};
use iced::{Color, Point, Rectangle, Renderer, Theme, mouse};
use std::sync::Arc;

#[derive(Clone)]
pub struct SourceUv {
    pub mesh: Arc<RenderMesh>,
}
impl<Message> canvas::Program<Message> for SourceUv {
    type State = ();
    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, frame.size(), Color::from_rgb(0.13, 0.14, 0.16));
        let Some(uv) = &self.mesh.uv0 else {
            return vec![frame.into_geometry()];
        };
        let margin = 18.0;
        let side = (bounds.width.min(bounds.height) - 2.0 * margin).max(1.0);
        let origin = Point::new((bounds.width - side) * 0.5, (bounds.height - side) * 0.5);
        frame.fill_rectangle(origin, iced::Size::new(side, side), Color::from_rgb(0.21, 0.22, 0.25));
        let border = Path::rectangle(origin, iced::Size::new(side, side));
        frame.stroke(&border, Stroke::default().with_color(Color::from_rgb(0.8, 0.8, 0.85)));
        for fraction in [0.25, 0.5, 0.75] {
            let pos = origin.x + side * fraction;
            let v = Path::line(Point::new(pos, origin.y), Point::new(pos, origin.y + side));
            frame.stroke(&v, Stroke::default().with_color(Color::from_rgb(0.31, 0.32, 0.35)));
            let row = origin.y + side * fraction;
            let h = Path::line(Point::new(origin.x, row), Point::new(origin.x + side, row));
            frame.stroke(&h, Stroke::default().with_color(Color::from_rgb(0.31, 0.32, 0.35)));
        }
        let project = |uv: [f32; 2]| -> Option<Point> {
            if uv.iter().all(|coord| coord.is_finite() && coord.abs() < 1000.0) {
                Some(Point::new(origin.x + uv[0] * side, origin.y + uv[1] * side))
            } else {
                None
            }
        };
        let count = self.mesh.indices.len() / 3;
        let step = count.div_ceil(16000).max(1);
        let strokes = Path::new(|path| {
            for tri in self.mesh.indices.as_chunks::<3>().0.iter().step_by(step) {
                let a = uv.get(tri[0] as usize).copied().and_then(project);
                let b = uv.get(tri[1] as usize).copied().and_then(project);
                let c = uv.get(tri[2] as usize).copied().and_then(project);
                if let (Some(a), Some(b), Some(c)) = (a, b, c) {
                    path.move_to(a);
                    path.line_to(b);
                    path.line_to(c);
                    path.close();
                }
            }
        });
        frame.stroke(
            &strokes,
            Stroke::default()
                .with_color(Color::from_rgb(0.93, 0.35, 0.41))
                .with_width(0.7),
        );
        vec![frame.into_geometry()]
    }
}
