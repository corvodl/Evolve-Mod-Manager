//! Canvas-drawn visual effects: card spotlight, border beam and the Home
//! ember field. iced backgrounds only support solid colours and linear
//! gradients, so radial glows are built from stacked translucent circles.
//!
//! Each effect is a pure function of its inputs (cursor, clock time), so it
//! never owns a timer: callers pass `UiState::clock` and only subscribe to
//! frames while the effect is visible and motion is allowed.

use iced::widget::canvas::{self, Frame, Path, Stroke};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme, mouse};

use crate::Message;
use crate::design;

/// Soft accent glow that follows the cursor across a card.
pub struct Spotlight {
    /// Cursor position relative to the card's top-left corner.
    pub cursor: Point,
}

impl canvas::Program<Message> for Spotlight {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let accent = theme.palette().primary;
        let radius = bounds.width.max(bounds.height).clamp(120.0, 260.0) * 0.6;
        // Twelve rings from outside in; alpha accumulates toward the centre.
        let rings = 12;
        for ring in 0..rings {
            let t = ring as f32 / rings as f32;
            let r = radius * (1.0 - t);
            frame.fill(
                &Path::circle(self.cursor, r.max(2.0)),
                design::with_alpha(accent, 0.012 + 0.006 * t),
            );
        }
        vec![frame.into_geometry()]
    }
}

/// Point at distance `d` along the perimeter of a rounded rectangle.
fn perimeter_point(size: Size, radius: f32, d: f32) -> Point {
    let w = size.width;
    let h = size.height;
    let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    let straight_w = (w - 2.0 * r).max(0.0);
    let straight_h = (h - 2.0 * r).max(0.0);
    let arc = std::f32::consts::FRAC_PI_2 * r;
    let total = 2.0 * (straight_w + straight_h) + 4.0 * arc;
    let mut d = d.rem_euclid(total.max(1.0));
    // Top edge, left to right.
    if d < straight_w {
        return Point::new(r + d, 0.0);
    }
    d -= straight_w;
    if d < arc {
        let a = -std::f32::consts::FRAC_PI_2 + d / r.max(0.001);
        return Point::new(w - r + r * a.cos(), r + r * a.sin());
    }
    d -= arc;
    if d < straight_h {
        return Point::new(w, r + d);
    }
    d -= straight_h;
    if d < arc {
        let a = d / r.max(0.001);
        return Point::new(w - r + r * a.cos(), h - r + r * a.sin());
    }
    d -= arc;
    if d < straight_w {
        return Point::new(w - r - d, h);
    }
    d -= straight_w;
    if d < arc {
        let a = std::f32::consts::FRAC_PI_2 + d / r.max(0.001);
        return Point::new(r + r * a.cos(), h - r + r * a.sin());
    }
    d -= arc;
    if d < straight_h {
        return Point::new(0.0, h - r - d);
    }
    d -= straight_h;
    let a = std::f32::consts::PI + d / r.max(0.001);
    Point::new(r + r * a.cos(), r + r * a.sin())
}

fn perimeter_length(size: Size, radius: f32) -> f32 {
    let r = radius.min(size.width / 2.0).min(size.height / 2.0).max(0.0);
    2.0 * ((size.width - 2.0 * r).max(0.0) + (size.height - 2.0 * r).max(0.0))
        + 2.0 * std::f32::consts::PI * r
}

/// A short bright segment that travels around a rounded rectangle while
/// work is running. Draw it stacked over a button or card.
pub struct BorderBeam {
    /// Clock time in seconds.
    pub time: f32,
    /// Seconds per lap.
    pub period: f32,
    /// Overall opacity, 0–1: a faint idle loop or a bright busy one.
    pub strength: f32,
}

impl canvas::Program<Message> for BorderBeam {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let inset = 1.0;
        let size = Size::new(bounds.width - inset * 2.0, bounds.height - inset * 2.0);
        if size.width <= 4.0 || size.height <= 4.0 {
            return vec![frame.into_geometry()];
        }
        let radius = design::radius(theme, design::Radius::Medium);
        let total = perimeter_length(size, radius);
        let head = (self.time / self.period.max(0.1)).fract() * total;
        let length = (total * 0.22).clamp(30.0, 160.0);
        let accent = theme.palette().primary;
        let segments = 18;
        for index in 0..segments {
            let a = index as f32 / segments as f32;
            let b = (index + 1) as f32 / segments as f32;
            let from = perimeter_point(size, radius, head - length * (1.0 - a));
            let to = perimeter_point(size, radius, head - length * (1.0 - b));
            let offset = |p: Point| Point::new(p.x + inset, p.y + inset);
            // Tail fades in toward the bright head.
            let alpha = b * b * self.strength.clamp(0.0, 1.0);
            frame.stroke(
                &Path::line(offset(from), offset(to)),
                Stroke::default()
                    .with_width(2.0)
                    .with_line_cap(canvas::LineCap::Round)
                    .with_color(design::with_alpha(
                        design::mix(accent, Color::WHITE, 0.35 * b),
                        alpha,
                    )),
            );
        }
        vec![frame.into_geometry()]
    }
}

/// Before/after texture comparison with a draggable vertical split.
pub struct ImageCompare {
    pub original: iced::widget::image::Handle,
    pub replacement: iced::widget::image::Handle,
    /// Fraction of the width showing the original (from the left).
    pub split: f32,
}

#[derive(Default)]
pub struct CompareDrag {
    dragging: bool,
}

/// Largest rectangle with the image's aspect ratio centred in `size`.
fn contain(renderer: &Renderer, handle: &iced::widget::image::Handle, size: Size) -> Rectangle {
    use iced::advanced::image::Renderer as _;
    let image = renderer
        .measure_image(handle)
        .map(|s| Size::new(s.width as f32, s.height as f32))
        .filter(|s| s.width > 0.0 && s.height > 0.0)
        .unwrap_or(size);
    let scale = (size.width / image.width).min(size.height / image.height);
    let fitted = Size::new(image.width * scale, image.height * scale);
    Rectangle::new(
        Point::new(
            (size.width - fitted.width) / 2.0,
            (size.height - fitted.height) / 2.0,
        ),
        fitted,
    )
}

impl canvas::Program<Message> for ImageCompare {
    type State = CompareDrag;

    fn update(
        &self,
        state: &mut CompareDrag,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let set = |x: f32| {
            canvas::Action::publish(Message::Ui(crate::ui_kit::UiMsg::CompareSet(
                (x - bounds.x) / bounds.width.max(1.0),
            )))
            .and_capture()
        };
        match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if cursor.is_over(bounds) =>
            {
                state.dragging = true;
                cursor.position().map(|p| set(p.x))
            }
            iced::Event::Mouse(mouse::Event::CursorMoved { position }) if state.dragging => {
                Some(set(position.x))
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.dragging = false;
                None
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &CompareDrag,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let size = bounds.size();
        let split_x = size.width * self.split.clamp(0.0, 1.0);
        frame.draw_image(
            contain(renderer, &self.replacement, size),
            canvas::Image::new(self.replacement.clone()),
        );
        // Clip region starts at the origin, so drawing coordinates match.
        let original_bounds = contain(renderer, &self.original, size);
        frame.with_clip(
            Rectangle::new(Point::ORIGIN, Size::new(split_x, size.height)),
            |clipped| {
                clipped.draw_image(original_bounds, canvas::Image::new(self.original.clone()));
            },
        );
        let accent = theme.palette().primary;
        frame.fill_rectangle(
            Point::new(split_x - 1.0, 0.0),
            Size::new(2.0, size.height),
            accent,
        );
        let grip = Point::new(split_x, size.height / 2.0);
        frame.fill(&Path::circle(grip, 13.0), accent);
        frame.fill_text(canvas::Text {
            content: "⇔".into(),
            position: grip,
            color: Color::WHITE,
            size: 13.0.into(),
            align_x: iced::alignment::Horizontal::Center.into(),
            align_y: iced::alignment::Vertical::Center,
            ..canvas::Text::default()
        });
        for (label, x, align) in [
            ("Original", 10.0, iced::alignment::Horizontal::Left),
            (
                "Replacement",
                size.width - 10.0,
                iced::alignment::Horizontal::Right,
            ),
        ] {
            frame.fill_text(canvas::Text {
                content: label.into(),
                position: Point::new(x, 10.0),
                color: design::with_alpha(Color::WHITE, 0.85),
                size: 12.0.into(),
                align_x: align.into(),
                ..canvas::Text::default()
            });
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &CompareDrag,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if cursor.is_over(bounds) {
            mouse::Interaction::ResizingHorizontally
        } else {
            mouse::Interaction::default()
        }
    }
}

/// Dashed accent border with a faint wash, shown while files are held
/// over a window that accepts drops.
pub struct DropZone {
    pub refused: bool,
}

impl canvas::Program<Message> for DropZone {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let color = if self.refused {
            theme.palette().danger
        } else {
            theme.palette().primary
        };
        frame.fill_rectangle(
            Point::ORIGIN,
            bounds.size(),
            design::with_alpha(color, 0.08),
        );
        let inset = 10.0;
        let radius = design::radius(theme, design::Radius::Large);
        frame.stroke(
            &Path::rounded_rectangle(
                Point::new(inset, inset),
                Size::new(bounds.width - inset * 2.0, bounds.height - inset * 2.0),
                radius.into(),
            ),
            Stroke {
                line_dash: canvas::LineDash {
                    segments: &[10.0, 7.0],
                    offset: 0,
                },
                ..Stroke::default().with_width(2.0).with_color(color)
            },
        );
        vec![frame.into_geometry()]
    }
}

/// Donut chart of storage buckets with the total in the middle. `sweep`
/// (0..1) reveals the segments clockwise once after an analysis.
pub struct RingChart {
    pub values: Vec<u64>,
    pub colors: Vec<Color>,
    pub sweep: f32,
    pub center: String,
}

impl canvas::Program<Message> for RingChart {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        use std::f32::consts::{FRAC_PI_2, TAU};
        let mut frame = Frame::new(renderer, bounds.size());
        let center = frame.center();
        let radius = bounds.width.min(bounds.height) / 2.0 - 8.0;
        let width = (radius * 0.28).max(6.0);
        frame.stroke(
            &Path::circle(center, radius),
            Stroke::default()
                .with_width(width)
                .with_color(design::field(theme)),
        );
        let total: u64 = self.values.iter().sum();
        if total > 0 {
            let reveal = TAU * self.sweep.clamp(0.0, 1.0);
            let mut start = -FRAC_PI_2;
            for (value, color) in self.values.iter().zip(&self.colors) {
                let span = TAU * (*value as f32 / total as f32);
                let visible_end = (start + span).min(-FRAC_PI_2 + reveal);
                if visible_end > start + 0.002 {
                    let arc = Path::new(|builder| {
                        builder.arc(canvas::path::Arc {
                            center,
                            radius,
                            start_angle: iced::Radians(start),
                            end_angle: iced::Radians(visible_end),
                        });
                    });
                    frame.stroke(
                        &arc,
                        Stroke::default()
                            .with_width(width)
                            .with_color(*color)
                            .with_line_cap(canvas::LineCap::Butt),
                    );
                }
                start += span;
            }
        }
        frame.fill_text(canvas::Text {
            content: self.center.clone(),
            position: center,
            color: theme.palette().text,
            size: 15.0.into(),
            align_x: iced::alignment::Horizontal::Center.into(),
            align_y: iced::alignment::Vertical::Center,
            ..canvas::Text::default()
        });
        vec![frame.into_geometry()]
    }
}

/// Track and knob for `ui_kit::switch`; `position` 0 = off, 1 = on.
pub struct SwitchKnob {
    pub position: f32,
}

impl canvas::Program<Message> for SwitchKnob {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let t = self.position.clamp(0.0, 1.0);
        let legacy = design::is_legacy(theme);
        let height = bounds.height;
        let radius = if legacy { 1.0 } else { height / 2.0 };
        let off = design::field(theme);
        let track = design::mix(off, theme.palette().primary, t);
        frame.fill(
            &Path::rounded_rectangle(Point::ORIGIN, bounds.size(), radius.into()),
            track,
        );
        frame.stroke(
            &Path::rounded_rectangle(
                Point::new(0.5, 0.5),
                Size::new(bounds.width - 1.0, height - 1.0),
                radius.into(),
            ),
            Stroke::default().with_width(1.0).with_color(design::mix(
                design::border(theme),
                theme.palette().primary,
                t,
            )),
        );
        let knob = height - 6.0;
        let x = 3.0 + (bounds.width - knob - 6.0) * t;
        let knob_path = if legacy {
            Path::rectangle(Point::new(x, 3.0), Size::new(knob, knob))
        } else {
            Path::circle(Point::new(x + knob / 2.0, height / 2.0), knob / 2.0)
        };
        frame.fill(&knob_path, Color::WHITE);
        vec![frame.into_geometry()]
    }
}

/// Slow rising embers behind the Home wallpaper.
pub struct Embers {
    pub time: f32,
}

const EMBER_COUNT: usize = 46;

/// Deterministic per-ember random in 0..1 (no RNG state to keep).
fn hash(seed: u32) -> f32 {
    let mut x = seed.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^= x >> 15;
    (x & 0x00FF_FFFF) as f32 / 16_777_216.0
}

/// Position (0..1 normalised), radius and alpha of ember `index` at `time`.
pub fn ember(index: usize, time: f32) -> (f32, f32, f32, f32) {
    let seed = index as u32;
    let speed = 0.018 + 0.03 * hash(seed * 3 + 1);
    let phase = hash(seed * 3 + 2);
    let life = (time * speed + phase).fract();
    let x0 = hash(seed * 3 + 3);
    let sway = (time * (0.3 + hash(seed * 7) * 0.5) + phase * std::f32::consts::TAU).sin() * 0.025;
    let x = (x0 + sway).rem_euclid(1.0);
    let y = 1.05 - life * 1.15;
    let size = 0.8 + 2.0 * hash(seed * 5 + 4);
    // Fade in from the bottom and out near the top.
    let alpha = (life * 6.0).min(1.0) * (1.0 - life).powf(1.5) * (0.35 + 0.45 * hash(seed * 11));
    (x, y, size, alpha)
}

impl canvas::Program<Message> for Embers {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let core = Color::from_rgb8(255, 170, 90);
        let glow = Color::from_rgb8(255, 96, 40);
        for index in 0..EMBER_COUNT {
            let (x, y, size, alpha) = ember(index, self.time);
            if alpha <= 0.01 {
                continue;
            }
            let center = Point::new(x * bounds.width, y * bounds.height);
            frame.fill(
                &Path::circle(center, size * 3.2),
                design::with_alpha(glow, alpha * 0.12),
            );
            frame.fill(&Path::circle(center, size), design::with_alpha(core, alpha));
        }
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perimeter_walk_stays_on_the_rectangle_edge() {
        let size = Size::new(200.0, 40.0);
        let total = perimeter_length(size, 8.0);
        for step in 0..200 {
            let point = perimeter_point(size, 8.0, total * step as f32 / 200.0);
            assert!(point.x >= -0.01 && point.x <= 200.01, "{point:?}");
            assert!(point.y >= -0.01 && point.y <= 40.01, "{point:?}");
            // Away from the corner arcs every point lies on an edge.
            if (8.0..=192.0).contains(&point.x) {
                assert!(
                    point.y.abs() < 0.01 || (point.y - 40.0).abs() < 0.01,
                    "{point:?}"
                );
            }
        }
        // Wraps around.
        let start = perimeter_point(size, 8.0, 0.0);
        let lap = perimeter_point(size, 8.0, total);
        assert!((start.x - lap.x).abs() < 0.01 && (start.y - lap.y).abs() < 0.01);
    }

    #[test]
    fn embers_are_deterministic_and_bounded() {
        for index in 0..EMBER_COUNT {
            let a = ember(index, 12.5);
            assert_eq!(a, ember(index, 12.5));
            assert!((0.0..=1.0).contains(&a.0));
            assert!(a.3 >= 0.0 && a.3 <= 1.0);
        }
    }
}
