//! Placement overview and hierarchy inspector for verified CryEngine level
//! scenes. Renders source locations only; does not invent geometry or changes.
use super::{Message, State};
use evolve_core::LevelScene;
use iced::widget::canvas::{self, Path, Stroke};
use iced::widget::{button, canvas as canvas_widget, column, row, scrollable, text, text_input};
use iced::{Color, Element, Fill, Length, Point, Rectangle, Renderer, Theme, mouse};
use std::sync::Arc;

const MUTED: Color = Color::from_rgb(0.72, 0.76, 0.83);

#[derive(Clone)]
struct SceneOverview {
    scene: Arc<LevelScene>,
    selected: Option<usize>,
    zoom: f32,
}
impl SceneOverview {
    fn extents(&self) -> ([f32; 2], f32) {
        let mut low = [f32::INFINITY; 2];
        let mut high = [f32::NEG_INFINITY; 2];
        for object in &self.scene.objects {
            for i in 0..2 {
                let p = object.world.position[i];
                if p.is_finite() {
                    low[i] = low[i].min(p);
                    high[i] = high[i].max(p);
                }
            }
        }
        if !low[0].is_finite() {
            return ([0.0; 2], 1.0);
        }
        (
            [(low[0] + high[0]) * 0.5, (low[1] + high[1]) * 0.5],
            (high[0] - low[0]).max(high[1] - low[1]).max(1.0),
        )
    }
    fn point(&self, coords: [f32; 3], bounds: Rectangle, center: [f32; 2], extent: f32) -> Point {
        let scale = bounds.width.min(bounds.height) * 0.80 * self.zoom.clamp(0.3, 3.0) / extent;
        Point::new(
            bounds.width * 0.5 + (coords[0] - center[0]) * scale,
            bounds.height * 0.5 - (coords[1] - center[1]) * scale,
        )
    }
}
impl canvas::Program<Message> for SceneOverview {
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
        frame.fill_rectangle(
            Point::ORIGIN,
            frame.size(),
            Color::from_rgb(0.17, 0.18, 0.20),
        );
        let axis_x = Path::line(
            Point::new(bounds.width * 0.5, 0.0),
            Point::new(bounds.width * 0.5, bounds.height),
        );
        let axis_y = Path::line(
            Point::new(0.0, bounds.height * 0.5),
            Point::new(bounds.width, bounds.height * 0.5),
        );
        frame.stroke(
            &axis_x,
            Stroke::default().with_color(Color::from_rgb(0.24, 0.27, 0.28)),
        );
        frame.stroke(
            &axis_y,
            Stroke::default().with_color(Color::from_rgb(0.30, 0.24, 0.26)),
        );
        let (center, extent) = self.extents();
        let step = self.scene.objects.len().div_ceil(12000).max(1);
        for (index, obj) in self.scene.objects.iter().enumerate().step_by(step) {
            if obj.hidden {
                continue;
            }
            let point = self.point(obj.world.position, bounds, center, extent);
            if point.x < -10.0
                || point.y < -10.0
                || point.x > bounds.width + 10.0
                || point.y > bounds.height + 10.0
            {
                continue;
            }
            let selected = self.selected == Some(index);
            let dot = Path::circle(point, if selected { 5.0 } else { 2.2 });
            frame.fill(
                &dot,
                if selected {
                    Color::from_rgb(0.96, 0.24, 0.33)
                } else {
                    Color::from_rgb(0.68, 0.75, 0.83)
                },
            );
        }
        vec![frame.into_geometry()]
    }

    fn update(
        &self,
        _state: &mut Self::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if !matches!(
            event,
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
        ) {
            return None;
        }
        let Some(point) = cursor.position_in(bounds) else {
            return (canvas::event::Status::Ignored, None);
        };
        let mut nearest = None;
        let mut distance = f32::INFINITY;
        let (center, extent) = self.extents();
        for (index, obj) in self.scene.objects.iter().enumerate().take(50_000) {
            if obj.hidden {
                continue;
            }
            let world = self.point(obj.world.position, bounds, center, extent);
            let d = (world.x - point.x).powi(2) + (world.y - point.y).powi(2);
            if d < distance {
                distance = d;
                nearest = Some(index);
            }
        }
        if distance <= 144.0 {
            nearest.map(|index| canvas::Action::publish(Message::LevelSelect(index)).and_capture())
        } else {
            None
        }
    }
}
pub(super) fn view(state: &State) -> Element<'_, Message> {
    let Some(scene) = &state.level_scene else {
        return column![
            text("SCENE HIERARCHY").size(15),
            text("Inspect a level to display a placement overview and object hierarchy.")
                .size(12)
                .color(MUTED)
        ]
        .spacing(8)
        .into();
    };
    let mut outliner = column![].spacing(2);
    let query = state.level_filter.to_ascii_lowercase();
    let mut visible = 0usize;
    for (index, obj) in scene.objects.iter().enumerate() {
        if !query.is_empty()
            && !obj.name.to_ascii_lowercase().contains(&query)
            && !obj.layer.to_ascii_lowercase().contains(&query)
            && !obj.model.to_ascii_lowercase().contains(&query)
        {
            continue;
        }
        if visible < 180 {
            outliner = outliner.push(
                button(text(format!("{}  /  {}", obj.layer, obj.name)).size(12))
                    .width(Fill)
                    .padding(4)
                    .style(if state.level_selected == Some(index) {
                        button::primary
                    } else {
                        button::text
                    })
                    .on_press(Message::LevelSelect(index)),
            );
        }
        visible += 1;
    }
    let details: Element<'_, Message> = if let Some(object) = state
        .level_selected
        .and_then(|index| scene.objects.get(index))
    {
        column![
            text(format!("Selected: {} ({})", object.name, object.kind)).size(13),
            text(format!("Position: {:?}", object.world.position)).size(12),
            text(format!("Rotation (wxyz): {:?}", object.world.quaternion)).size(12),
            text(format!("Scale: {:?}", object.world.scale)).size(12),
            text(format!("Model: {}", object.model)).size(12),
            text(format!("Status: {}", object.status))
                .size(12)
                .color(MUTED),
        ]
        .spacing(6)
        .into()
    } else {
        text("Select an object or click its point on the map to inspect original transforms.")
            .size(12)
            .color(MUTED)
            .into()
    };
    row![
        column![
            row![
                text("LEVEL PLACEMENT OVERVIEW").size(14),
                text(format!("{} objects", scene.objects.len())).size(12)
            ]
            .spacing(12),
            canvas_widget(SceneOverview {
                scene: scene.clone(),
                selected: state.level_selected,
                zoom: state.level_zoom.max(0.3),
            })
            .width(Fill)
            .height(Length::Fixed(340.0)),
            row![
                text("Zoom").size(12),
                iced::widget::slider(
                    0.3..=3.0,
                    state.level_zoom.max(0.3),
                    Message::LevelZoomChanged
                )
                .width(Length::Fixed(190.0)),
                text("Read-only source placements").size(12).color(MUTED),
            ]
            .spacing(10),
        ]
        .spacing(8)
        .width(Length::FillPortion(3)),
        column![
            text("SCENE OUTLINER").size(14),
            text_input("Filter layer, object or model...", &state.level_filter)
                .on_input(Message::LevelFilterChanged),
            scrollable(outliner).height(Length::Fixed(240.0)),
            text(format!("{visible} matching placements"))
                .size(12)
                .color(MUTED),
            details,
        ]
        .spacing(7)
        .width(Length::FillPortion(2)),
    ]
    .spacing(12)
    .into()
}
