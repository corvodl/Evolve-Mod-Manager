//! Blender-style viewport mouse gestures, shared by GPU and fallback canvas.
//! Shift + middle-drag pans, middle-drag orbits, wheel zooms. Gesture state
//! belongs to each widget/window; the camera itself belongs to the app.
use iced::{Event, Point, Rectangle, mouse};

#[derive(Default)]
pub struct Navigation {
    last: Option<Point>,
    panning: bool,
}

pub enum Gesture {
    Orbit(f32, f32),
    Pan(f32, f32),
    Zoom(f32),
}

impl Navigation {
    pub fn handle(
        &mut self,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
        shift_held: bool,
    ) -> Option<Gesture> {
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Middle)) => {
                if let Some(position) = cursor.position_in(bounds) {
                    self.last = Some(Point::new(position.x + bounds.x, position.y + bounds.y));
                    self.panning = shift_held;
                }
                None
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Middle)) => {
                self.last = None;
                None
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                let last = self.last?;
                self.last = Some(*position);
                let dx = position.x - last.x;
                let dy = position.y - last.y;
                if !dx.is_finite() || !dy.is_finite() {
                    return None;
                }
                if self.panning {
                    Some(Gesture::Pan(
                        2.0 * dx / bounds.width.max(1.0),
                        -2.0 * dy / bounds.height.max(1.0),
                    ))
                } else {
                    Some(Gesture::Orbit(dx * 0.35, dy * 0.35))
                }
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta })
                if cursor.position_in(bounds).is_some() =>
            {
                let y = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => *y / 40.0,
                };
                if y.is_finite() && y != 0.0 {
                    Some(Gesture::Zoom(y.clamp(-10.0, 10.0)))
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

pub fn message(gesture: Gesture) -> super::Message {
    match gesture {
        Gesture::Orbit(x, y) => super::Message::ViewerOrbitDelta(x, y),
        Gesture::Pan(x, y) => super::Message::ViewerPanDelta(x, y),
        Gesture::Zoom(amount) => super::Message::ViewerZoomWheel(amount),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_unstarted_moves_and_unfocused_scroll() {
        let mut nav = Navigation::default();
        let bounds = Rectangle {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 200.0,
        };
        let event = Event::Mouse(mouse::Event::CursorMoved {
            position: Point::new(10.0, 12.0),
        });
        assert!(
            nav.handle(&event, bounds, mouse::Cursor::Unavailable, false)
                .is_none()
        );
        let wheel = Event::Mouse(mouse::Event::WheelScrolled {
            delta: mouse::ScrollDelta::Lines { x: 0.0, y: 1.0 },
        });
        assert!(
            nav.handle(&wheel, bounds, mouse::Cursor::Unavailable, false)
                .is_none()
        );
    }
}
