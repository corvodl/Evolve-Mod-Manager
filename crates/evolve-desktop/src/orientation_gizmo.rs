//! Interactive Blender-inspired XYZ orientation gizmo, matching the private
//! Qt model viewer. Click a signed axis to snap; drag to orbit the camera.
//! This widget changes camera state only, never the asset or skeleton.
use iced::widget::canvas::{self, Path, Stroke};
use iced::{Color, Event, Point, Rectangle, Renderer, Theme, mouse};

#[derive(Clone, Copy)]
pub struct OrientationGizmo {
    pub yaw: f32,
    pub pitch: f32,
}

#[derive(Default)]
pub struct GizmoState {
    last: Option<Point>,
    target: Option<&'static str>,
    moved: bool,
}

#[derive(Clone, Copy)]
struct AxisPoint {
    depth: f32,
    letter: &'static str,
    color: Color,
    point: Point,
    preset: &'static str,
}

/// In the Model Viewer yaw zero is a camera at +Y looking towards -Y.
/// Its screen-right vector points towards source -X. The geometry stays
/// untouched: viewer orbit, grid, and gizmo share this projection only.
pub(crate) const FRONT_YAW: f32 = 0.0;

/// Camera-space horizontal axis shared by GPU material preview, CPU wireframe,
/// floor grid, and the gizmo. This is intentionally NOT the Level Editor's
/// world-camera basis (which looks in a different direction at yaw zero).
pub(crate) fn model_screen_horizontal(x: f32, y: f32, sin_yaw: f32, cos_yaw: f32) -> f32 {
    y * sin_yaw - x * cos_yaw
}

pub(crate) fn model_camera_preset(preset: &str) -> Option<(f32, f32)> {
    match preset {
        "Front" => Some((FRONT_YAW, 0.0)),
        "Back" => Some((180.0, 0.0)),
        "Left" => Some((-90.0, 0.0)),
        "Right" => Some((90.0, 0.0)),
        "Top" => Some((FRONT_YAW, 89.5)),
        "Bottom" => Some((FRONT_YAW, -89.5)),
        "Isometric" => Some((-30.0, 14.0)),
        _ => None,
    }
}

impl OrientationGizmo {
    fn axes(&self, width: f32, height: f32) -> Vec<AxisPoint> {
        let (sy, cy) = self.yaw.to_radians().sin_cos();
        let (sp, cp) = self.pitch.to_radians().sin_cos();
        let center = Point::new(width * 0.5, height * 0.5);
        let mut points = Vec::with_capacity(6);
        for (letter, axis, positive, negative, color) in [
            (
                "X",
                [1.0, 0.0, 0.0],
                "Right",
                "Left",
                Color::from_rgb8(222, 108, 114),
            ),
            (
                "Y",
                [0.0, 1.0, 0.0],
                "Front",
                "Back",
                Color::from_rgb8(117, 195, 132),
            ),
            (
                "Z",
                [0.0, 0.0, 1.0],
                "Top",
                "Bottom",
                Color::from_rgb8(106, 168, 228),
            ),
        ] {
            for direction in [-1.0, 1.0] {
                let [x, y, z] = axis.map(|part| part * direction);
                let horizontal = model_screen_horizontal(x, y, sy, cy);
                let vertical = z * cp - (x * sy + y * cy) * sp;
                let depth = (x * sy + y * cy) * cp + z * sp;
                points.push(AxisPoint {
                    depth,
                    letter: if direction > 0.0 { letter } else { "" },
                    color,
                    point: Point::new(center.x + horizontal * 33.0, center.y - vertical * 33.0),
                    preset: if direction > 0.0 { positive } else { negative },
                });
            }
        }
        points.sort_by(|a, b| a.depth.total_cmp(&b.depth));
        points
    }

    fn target(&self, position: Point, bounds: Rectangle) -> Option<&'static str> {
        self.axes(bounds.width, bounds.height)
            .into_iter()
            .rev()
            .find(|axis| {
                let x = position.x - axis.point.x;
                let y = position.y - axis.point.y;
                x * x + y * y <= 13.0 * 13.0
            })
            .map(|axis| axis.preset)
    }
}

impl canvas::Program<super::Message> for OrientationGizmo {
    type State = GizmoState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<super::Message>> {
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let position = cursor.position_in(bounds)?;
                state.last = Some(position);
                state.target = self.target(position, bounds);
                state.moved = false;
                Some(canvas::Action::capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let last = state.last?;
                let Some(position) = cursor.position_in(bounds) else {
                    state.last = None;
                    state.target = None;
                    return Some(canvas::Action::capture());
                };
                state.last = Some(position);
                let dx = position.x - last.x;
                let dy = position.y - last.y;
                if !dx.is_finite() || !dy.is_finite() {
                    return Some(canvas::Action::capture());
                }
                if dx.abs() + dy.abs() < 0.001 {
                    return Some(canvas::Action::capture());
                }
                if dx.abs() + dy.abs() >= 2.0 {
                    state.moved = true;
                }
                if state.moved {
                    Some(
                        canvas::Action::publish(super::Message::ViewerOrbitDelta(
                            dx * 0.55,
                            dy * 0.45,
                        ))
                        .and_capture(),
                    )
                } else {
                    Some(canvas::Action::capture())
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.last.take()?;
                let target = if state.moved {
                    None
                } else {
                    state.target.take()
                };
                state.target = None;
                state.moved = false;
                Some(match target {
                    Some(preset) => {
                        canvas::Action::publish(super::Message::ViewerCameraPreset(preset))
                            .and_capture()
                    }
                    None => canvas::Action::capture(),
                })
            }
            _ => None,
        }
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if cursor.position_in(bounds).is_some() {
            mouse::Interaction::Grab
        } else {
            mouse::Interaction::default()
        }
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
        let center = frame.center();
        frame.fill(
            &Path::circle(center, 52.0),
            Color::from_rgba8(28, 30, 36, 0.81),
        );
        for axis in self.axes(bounds.width, bounds.height) {
            let front = axis.depth >= 0.0;
            let color = if front {
                axis.color
            } else {
                Color {
                    a: 0.48,
                    ..axis.color
                }
            };
            let line = Path::new(|path| {
                path.move_to(center);
                path.line_to(axis.point);
            });
            frame.stroke(
                &line,
                Stroke::default()
                    .with_color(color)
                    .with_width(if front { 2.0 } else { 1.0 }),
            );
            frame.fill(
                &Path::circle(axis.point, if front { 11.0 } else { 6.0 }),
                color,
            );
            if !axis.letter.is_empty() {
                frame.fill_text(canvas::Text {
                    content: axis.letter.into(),
                    position: Point::new(axis.point.x - 4.5, axis.point.y - 8.0),
                    size: iced::Pixels(13.0),
                    color: if front {
                        Color::WHITE
                    } else {
                        Color::from_rgb8(195, 198, 210)
                    },
                    ..Default::default()
                });
            }
        }
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_only_projection_keeps_left_right_across_orbit() {
        assert_eq!(model_screen_horizontal(1.0, 0.0, 0.0, 1.0), -1.0);
        assert_eq!(model_screen_horizontal(-1.0, 0.0, 0.0, 1.0), 1.0);
        assert_eq!(model_screen_horizontal(1.0, 0.0, 0.0, -1.0), 1.0);
        // At yaw 90, camera right maps to source +Y, not source X.
        assert_eq!(model_screen_horizontal(0.0, 1.0, 1.0, 0.0), 1.0);
    }

    #[test]
    fn cryengine_front_camera_sees_positive_y_and_keeps_right_left_consistent() {
        use std::f32::consts::PI;
        assert_eq!(model_camera_preset("Front"), Some((0.0, 0.0)));
        assert_eq!(model_camera_preset("Back"), Some((180.0, 0.0)));
        let theta = FRONT_YAW * PI / 180.0;
        // From +Y looking toward -Y, the character's +X is camera-left.
        let screen_x = -theta.cos();
        assert!(screen_x < -0.99);
        let view = OrientationGizmo {
            yaw: FRONT_YAW,
            pitch: 0.0,
        };
        let x_axis = view
            .axes(112.0, 112.0)
            .into_iter()
            .find(|axis| axis.preset == "Right")
            .unwrap();
        assert!(x_axis.point.x < 56.0);
        assert_eq!(model_camera_preset("Unknown"), None);
    }

    #[test]
    fn gizmo_signed_axes_follow_private_camera_presets() {
        let view = OrientationGizmo {
            yaw: 0.0,
            pitch: 0.0,
        };
        assert_eq!(view.axes(112.0, 112.0).len(), 6);
        assert_eq!(
            view.target(
                Point::new(23.0, 56.0),
                Rectangle {
                    x: 0.0,
                    y: 0.0,
                    width: 112.0,
                    height: 112.0
                }
            ),
            Some("Right")
        );
        let rotated = OrientationGizmo {
            yaw: 90.0,
            pitch: 0.0,
        };
        assert_ne!(
            view.axes(112.0, 112.0)
                .iter()
                .find(|axis| axis.preset == "Right")
                .unwrap()
                .point,
            rotated
                .axes(112.0, 112.0)
                .iter()
                .find(|axis| axis.preset == "Right")
                .unwrap()
                .point
        );
    }
}
