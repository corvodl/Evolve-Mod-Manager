//! Blender-style viewport mouse gestures, shared by GPU and fallback canvas.
//! Left/middle-drag orbits, Shift+drag/right-drag pans, wheel zooms. Gesture state
//! belongs to each widget/window; the camera itself belongs to the app.
use iced::{Event, Point, Rectangle, mouse};

#[derive(Default)]
pub struct Navigation {
    last: Option<Point>,
    panning: bool,
    active_button: Option<mouse::Button>,
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
            Event::Mouse(mouse::Event::ButtonPressed(button))
                if matches!(
                    button,
                    mouse::Button::Left | mouse::Button::Middle | mouse::Button::Right
                ) =>
            {
                // Keep the original drag active even if another button is pressed.
                if self.active_button.is_some() {
                    return None;
                }
                if let Some(position) = cursor.position_in(bounds) {
                    self.last = Some(Point::new(position.x + bounds.x, position.y + bounds.y));
                    self.active_button = Some(*button);
                    self.panning = shift_held || *button == mouse::Button::Right;
                }
                None
            }
            Event::Mouse(mouse::Event::ButtonReleased(button))
                if self.active_button == Some(*button) =>
            {
                self.last = None;
                self.active_button = None;
                self.panning = false;
                None
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) => {
                let last = self.last?;
                // Pointer capture is bounded: discard an abandoned drag once
                // the pointer leaves the viewport, instead of jumping on re-entry.
                if cursor.position_in(bounds).is_none() {
                    self.last = None;
                    self.active_button = None;
                    self.panning = false;
                    return None;
                }
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
    fn left_drag_orbits_without_special_mouse_buttons() {
        let bounds = Rectangle {
            x: 0.0,
            y: 0.0,
            width: 250.0,
            height: 200.0,
        };
        let mut nav = Navigation::default();
        let start = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        let cursor = mouse::Cursor::Available(Point::new(60.0, 80.0));
        assert!(nav.handle(&start, bounds, cursor, false).is_none());
        let moved = Event::Mouse(mouse::Event::CursorMoved {
            position: Point::new(80.0, 70.0),
        });
        assert!(
            matches!(nav.handle(&moved, bounds, cursor, false), Some(Gesture::Orbit(dx, dy)) if dx > 0.0 && dy < 0.0)
        );
        let release = Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
        nav.handle(&release, bounds, cursor, false);
        assert!(nav.handle(&moved, bounds, cursor, false).is_none());
    }

    #[test]
    fn shift_left_and_right_drag_pan() {
        let bounds = Rectangle {
            x: 0.0,
            y: 0.0,
            width: 250.0,
            height: 200.0,
        };
        let cursor = mouse::Cursor::Available(Point::new(60.0, 80.0));
        for (button, shift) in [(mouse::Button::Left, true), (mouse::Button::Right, false)] {
            let mut nav = Navigation::default();
            nav.handle(
                &Event::Mouse(mouse::Event::ButtonPressed(button)),
                bounds,
                cursor,
                shift,
            );
            let moved = Event::Mouse(mouse::Event::CursorMoved {
                position: Point::new(80.0, 70.0),
            });
            assert!(
                matches!(nav.handle(&moved, bounds, cursor, shift), Some(Gesture::Pan(x, y)) if x > 0.0 && y > 0.0)
            );
        }
    }

    #[test]
    fn secondary_button_does_not_replace_active_orbit_drag() {
        let bounds = Rectangle {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 200.0,
        };
        let cursor = mouse::Cursor::Available(Point::new(50.0, 50.0));
        let mut nav = Navigation::default();
        nav.handle(
            &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            bounds,
            cursor,
            false,
        );
        nav.handle(
            &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)),
            bounds,
            cursor,
            false,
        );
        let move_event = Event::Mouse(mouse::Event::CursorMoved {
            position: Point::new(55.0, 53.0),
        });
        assert!(matches!(
            nav.handle(&move_event, bounds, cursor, false),
            Some(Gesture::Orbit(..))
        ));
    }

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
