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
            let alpha = b * b;
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
    let sway = (time * (0.3 + hash(seed * 7) * 0.5) + phase * 6.28).sin() * 0.025;
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
                assert!(point.y.abs() < 0.01 || (point.y - 40.0).abs() < 0.01, "{point:?}");
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
