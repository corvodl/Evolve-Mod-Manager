//! Custom colour picker for Settings › Appearance: a saturation/value
//! square, a hue strip and a hex field, editing one palette role at a time.
//! It writes the existing `appearance_colors` map, so presets and custom
//! colours keep working together; each role has its own Reset.

use iced::widget::canvas::{self, Frame, Path, Stroke};
use iced::widget::{column, container, row, text};
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Size, Theme, mouse};

use crate::design;
use crate::{Message, State};

pub const ROLES: [(&str, &str); 9] = [
    ("accent", "Accent"),
    ("background", "Background"),
    ("surface", "Surface"),
    ("field", "Field"),
    ("border", "Border"),
    ("text", "Text"),
    ("muted", "Muted text"),
    ("button", "Button"),
    ("selection", "Selection"),
];

#[derive(Debug, Clone)]
pub enum Msg {
    SelectRole(&'static str),
    /// Saturation/value from the square, both 0..1.
    SatVal(f32, f32),
    /// Hue in degrees.
    Hue(f32),
    HexInput(String),
    /// Mouse released on the square or strip: save preferences once.
    Commit,
    Reset(&'static str),
}

#[derive(Debug, Clone)]
pub struct PickerState {
    pub role: &'static str,
    pub hue: f32,
    pub sat: f32,
    pub val: f32,
    pub hex: String,
}

impl Default for PickerState {
    fn default() -> Self {
        Self {
            role: "accent",
            hue: 352.0,
            sat: 0.72,
            val: 0.79,
            hex: String::new(),
        }
    }
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> Color {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    Color::from_rgb(r + m, g + m, b + m)
}

pub fn rgb_to_hsv(color: Color) -> (f32, f32, f32) {
    let (r, g, b) = (color.r, color.g, color.b);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let hue = if delta == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    let sat = if max == 0.0 { 0.0 } else { delta / max };
    (hue, sat, max)
}

pub fn to_hex(color: Color) -> String {
    let [r, g, b, _] = color.into_rgba8();
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// The role's current effective colour from the live theme.
fn current(state: &State, role: &str) -> Color {
    let theme = design::theme(&state.preferences);
    let palette = theme.palette();
    let extended = theme.extended_palette();
    match role {
        "accent" => palette.primary,
        "background" => palette.background,
        "text" => palette.text,
        "surface" => extended.background.weak.color,
        "field" => extended.background.neutral.color,
        "border" => extended.background.strong.color,
        "muted" => extended.background.strongest.text,
        "button" => extended.secondary.base.color,
        "selection" => extended.primary.weak.color,
        _ => palette.primary,
    }
}

fn load_role(state: &mut State, role: &'static str) {
    let color = current(state, role);
    let (hue, sat, val) = rgb_to_hsv(color);
    let picker = &mut state.color_picker;
    picker.role = role;
    // Keep the previous hue for greys so the strip doesn't jump to red.
    if sat > 0.0 {
        picker.hue = hue;
    }
    picker.sat = sat;
    picker.val = val;
    picker.hex = to_hex(color);
}

fn apply(state: &mut State) {
    let picker = &state.color_picker;
    let hex = to_hex(hsv_to_rgb(picker.hue, picker.sat, picker.val));
    let role = picker.role;
    state.color_picker.hex = hex.clone();
    state
        .preferences
        .appearance_colors
        .insert(role.into(), hex);
}

/// Returns whether preferences should be persisted.
pub fn update(state: &mut State, message: Msg) -> bool {
    if state.preferences.ui_theme != "Modern" {
        return false;
    }
    match message {
        Msg::SelectRole(role) => {
            load_role(state, role);
            false
        }
        Msg::SatVal(sat, val) => {
            state.color_picker.sat = sat.clamp(0.0, 1.0);
            state.color_picker.val = val.clamp(0.0, 1.0);
            apply(state);
            false
        }
        Msg::Hue(hue) => {
            state.color_picker.hue = hue.clamp(0.0, 359.9);
            apply(state);
            false
        }
        Msg::HexInput(value) => {
            let trimmed = value.trim().to_owned();
            state.color_picker.hex = trimmed.clone();
            let normalized = if trimmed.starts_with('#') {
                trimmed
            } else {
                format!("#{trimmed}")
            };
            if let Some(color) = design::hex(&normalized) {
                let (hue, sat, val) = rgb_to_hsv(color);
                let role = state.color_picker.role;
                if sat > 0.0 {
                    state.color_picker.hue = hue;
                }
                state.color_picker.sat = sat;
                state.color_picker.val = val;
                state
                    .preferences
                    .appearance_colors
                    .insert(role.into(), normalized.to_ascii_lowercase());
                return true;
            }
            false
        }
        Msg::Commit => true,
        Msg::Reset(role) => {
            state.preferences.appearance_colors.remove(role);
            if state.color_picker.role == role {
                load_role(state, role);
            }
            true
        }
    }
}

fn picker(message: Msg) -> Message {
    Message::ColorPicker(message)
}

pub fn view(state: &State) -> Element<'_, Message> {
    let modern = state.preferences.ui_theme == "Modern";
    let picker_state = &state.color_picker;
    let mut roles = row![].spacing(4);
    for (role, label) in ROLES {
        let color = current(state, role);
        let selected = picker_state.role == role;
        let custom = state.preferences.appearance_colors.contains_key(role);
        roles = roles.push(
            iced::widget::button(
                row![
                    container(iced::widget::space().width(10).height(10)).style(move |theme| {
                        container::Style {
                            background: Some(iced::Background::Color(color)),
                            border: iced::Border {
                                color: design::border(theme),
                                width: 1.0,
                                radius: design::radius(theme, design::Radius::Pill).into(),
                            },
                            ..container::Style::default()
                        }
                    }),
                    text(if custom { format!("{label}*") } else { label.to_owned() }).size(11),
                ]
                .spacing(5)
                .align_y(iced::Center),
            )
            .padding([4, 8])
            .style(move |theme, status| {
                if selected {
                    design::primary(theme, status)
                } else {
                    design::secondary(theme, status)
                }
            })
            .on_press_maybe(modern.then_some(picker(Msg::SelectRole(role)))),
        );
    }
    let square = iced::widget::canvas(SatValSquare {
        hue: picker_state.hue,
        sat: picker_state.sat,
        val: picker_state.val,
        enabled: modern,
    })
    .width(Length::Fixed(200.0))
    .height(Length::Fixed(140.0));
    let strip = iced::widget::canvas(HueStrip {
        hue: picker_state.hue,
        enabled: modern,
    })
    .width(Length::Fixed(18.0))
    .height(Length::Fixed(140.0));
    let preview_color = hsv_to_rgb(picker_state.hue, picker_state.sat, picker_state.val);
    let hex_value = if picker_state.hex.is_empty() {
        to_hex(current(state, picker_state.role))
    } else {
        picker_state.hex.clone()
    };
    let role = picker_state.role;
    let controls = column![
        container(iced::widget::space().width(64).height(28)).style(move |theme| {
            container::Style {
                background: Some(iced::Background::Color(preview_color)),
                border: iced::Border {
                    color: design::border(theme),
                    width: 1.0,
                    radius: design::radius(theme, design::Radius::Small).into(),
                },
                ..container::Style::default()
            }
        }),
        design::text_input("#rrggbb", &hex_value)
            .on_input_maybe(modern.then_some(|value| picker(Msg::HexInput(value))))
            .width(Length::Fixed(120.0)),
        design::button("Reset")
            .style(design::secondary)
            .on_press_maybe(
                (modern && state.preferences.appearance_colors.contains_key(role))
                    .then_some(picker(Msg::Reset(role))),
            ),
        text("* custom colour").size(10).style(design::muted),
    ]
    .spacing(8);
    column![
        iced::widget::scrollable(roles).direction(iced::widget::scrollable::Direction::Horizontal(
            iced::widget::scrollable::Scrollbar::new().width(4).scroller_width(4),
        )),
        row![square, strip, controls].spacing(12),
    ]
    .spacing(10)
    .into()
}

#[derive(Default)]
pub struct DragState {
    dragging: bool,
}

struct SatValSquare {
    hue: f32,
    sat: f32,
    val: f32,
    enabled: bool,
}

impl canvas::Program<Message> for SatValSquare {
    type State = DragState;

    fn update(
        &self,
        state: &mut DragState,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if !self.enabled {
            return None;
        }
        let pick = |position: Point| {
            let x = ((position.x - bounds.x) / bounds.width).clamp(0.0, 1.0);
            let y = ((position.y - bounds.y) / bounds.height).clamp(0.0, 1.0);
            canvas::Action::publish(picker(Msg::SatVal(x, 1.0 - y))).and_capture()
        };
        match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if cursor.is_over(bounds) =>
            {
                state.dragging = true;
                cursor.position().map(pick)
            }
            iced::Event::Mouse(mouse::Event::CursorMoved { position }) if state.dragging => {
                Some(pick(*position))
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                if state.dragging =>
            {
                state.dragging = false;
                Some(canvas::Action::publish(picker(Msg::Commit)))
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &DragState,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        // A grid of cells approximates the 2D gradient; 24×16 is smooth
        // enough at this size and cheap to tessellate.
        let (cols, rows) = (24, 16);
        let cell = Size::new(bounds.width / cols as f32, bounds.height / rows as f32);
        for cx in 0..cols {
            for cy in 0..rows {
                let s = (cx as f32 + 0.5) / cols as f32;
                let v = 1.0 - (cy as f32 + 0.5) / rows as f32;
                frame.fill_rectangle(
                    Point::new(cx as f32 * cell.width, cy as f32 * cell.height),
                    Size::new(cell.width + 0.5, cell.height + 0.5),
                    hsv_to_rgb(self.hue, s, v),
                );
            }
        }
        let marker = Point::new(self.sat * bounds.width, (1.0 - self.val) * bounds.height);
        frame.stroke(
            &Path::circle(marker, 6.0),
            Stroke::default().with_width(2.0).with_color(Color::WHITE),
        );
        frame.stroke(
            &Path::circle(marker, 7.5),
            Stroke::default()
                .with_width(1.0)
                .with_color(Color::from_rgba(0.0, 0.0, 0.0, 0.5)),
        );
        frame.stroke(
            &Path::rectangle(Point::ORIGIN, bounds.size()),
            Stroke::default().with_width(1.0).with_color(design::border(theme)),
        );
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &DragState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if self.enabled && cursor.is_over(bounds) {
            mouse::Interaction::Crosshair
        } else {
            mouse::Interaction::default()
        }
    }
}

struct HueStrip {
    hue: f32,
    enabled: bool,
}

impl canvas::Program<Message> for HueStrip {
    type State = DragState;

    fn update(
        &self,
        state: &mut DragState,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if !self.enabled {
            return None;
        }
        let pick = |position: Point| {
            let y = ((position.y - bounds.y) / bounds.height).clamp(0.0, 1.0);
            canvas::Action::publish(picker(Msg::Hue(y * 360.0))).and_capture()
        };
        match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if cursor.is_over(bounds) =>
            {
                state.dragging = true;
                cursor.position().map(pick)
            }
            iced::Event::Mouse(mouse::Event::CursorMoved { position }) if state.dragging => {
                Some(pick(*position))
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                if state.dragging =>
            {
                state.dragging = false;
                Some(canvas::Action::publish(picker(Msg::Commit)))
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _state: &DragState,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let steps = 36;
        let step = bounds.height / steps as f32;
        for index in 0..steps {
            let hue = (index as f32 + 0.5) / steps as f32 * 360.0;
            frame.fill_rectangle(
                Point::new(0.0, index as f32 * step),
                Size::new(bounds.width, step + 0.5),
                hsv_to_rgb(hue, 1.0, 1.0),
            );
        }
        let y = self.hue / 360.0 * bounds.height;
        frame.stroke(
            &Path::rectangle(Point::new(0.5, y - 2.0), Size::new(bounds.width - 1.0, 4.0)),
            Stroke::default().with_width(2.0).with_color(Color::WHITE),
        );
        frame.stroke(
            &Path::rectangle(Point::ORIGIN, bounds.size()),
            Stroke::default().with_width(1.0).with_color(design::border(theme)),
        );
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_round_trips_through_rgb() {
        for (h, s, v) in [(0.0, 1.0, 1.0), (120.0, 0.5, 0.8), (240.0, 0.25, 0.4), (300.0, 0.9, 0.6)] {
            let (h2, s2, v2) = rgb_to_hsv(hsv_to_rgb(h, s, v));
            assert!((h - h2).abs() < 0.5, "{h} {h2}");
            assert!((s - s2).abs() < 0.01);
            assert!((v - v2).abs() < 0.01);
        }
        assert_eq!(to_hex(Color::from_rgb8(201, 56, 74)), "#c9384a");
    }

    #[test]
    fn picking_writes_appearance_colors_and_reset_removes() {
        let mut state = State::default();
        state.preferences.ui_theme = "Modern".into();
        update(&mut state, Msg::SelectRole("surface"));
        update(&mut state, Msg::HexInput("123456".into()));
        assert_eq!(state.preferences.appearance_colors["surface"], "#123456");
        update(&mut state, Msg::Hue(120.0));
        assert_ne!(state.preferences.appearance_colors["surface"], "#123456");
        update(&mut state, Msg::Reset("surface"));
        assert!(!state.preferences.appearance_colors.contains_key("surface"));
        // Legacy never edits Modern colours.
        state.preferences.ui_theme = "Evolve Legacy".into();
        update(&mut state, Msg::HexInput("#abcdef".into()));
        assert!(!state.preferences.appearance_colors.contains_key("surface"));
    }
}
