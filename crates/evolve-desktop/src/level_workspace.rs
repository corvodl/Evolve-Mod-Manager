//! Placement overview and hierarchy inspector for verified CryEngine level
//! scenes. Renders source locations only; does not invent geometry or changes.
use super::{Message, State};
use evolve_core::{LevelScene, Transform};
use iced::widget::canvas::{self, Path, Stroke};
use iced::widget::{button, canvas as canvas_widget, column, row, scrollable, text, text_input};
use iced::{Color, Element, Fill, Length, Point, Rectangle, Renderer, Theme, mouse};
use std::sync::Arc;

const MUTED: Color = Color::from_rgb(0.72, 0.76, 0.83);
pub(super) fn format_transform(value: Transform) -> (String, String, String) {
    let fields = |components: &[f32]| {
        components
            .iter()
            .map(|component| component.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    (
        fields(&value.position),
        fields(&value.quaternion),
        fields(&value.scale),
    )
}

fn read_components<const N: usize>(source: &str, label: &str) -> Result<[f32; N], String> {
    let values = source
        .split(',')
        .map(|value| value.trim().parse::<f32>())
        .collect::<Vec<_>>();
    if values.len() != N {
        return Err(format!("{label} must contain {N} comma-separated numbers."));
    }
    let mut result = [0.0; N];
    for (target, component) in result.iter_mut().zip(values) {
        let parsed = component.map_err(|_| format!("Invalid {label} number"))?;
        if !parsed.is_finite() || parsed.abs() > 1e8 {
            return Err(format!("{label} values must be finite and bounded."));
        }
        *target = parsed;
    }
    Ok(result)
}

pub(super) fn parse_transform(
    position: &str,
    rotation: &str,
    scale: &str,
) -> Result<Transform, String> {
    Ok(Transform {
        position: read_components(position, "Position")?,
        quaternion: read_components(rotation, "Rotation (wxyz)")?,
        scale: read_components(scale, "Scale")?,
    })
}

#[derive(Clone)]
struct SceneOverview {
    scene: Arc<LevelScene>,
    selected_indices: std::collections::HashSet<usize>,
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
            let selected = self.selected_indices.contains(&index);
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
        let point = cursor.position_in(bounds)?;
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

/// Unified level scene viewport: authored terrain and every resolved placed
/// mesh share one depth-buffered wgpu scene. The outliner remains interactive.
fn world_viewport(state: &State) -> Element<'_, Message> {
    if let Some(mesh) = &state.model {
        iced::widget::shader(super::level_pick::LevelViewport {
            picks: state
                .level_picks
                .clone()
                .unwrap_or_else(|| Arc::new(Vec::new())),
            selected_pivot: state
                .level_selected
                .and_then(|i| state.level_scene.as_ref()?.objects.get(i))
                .map(|o| o.world.position),
            editable: state.level_project.is_some()
                && state
                    .level_selected
                    .and_then(|i| state.level_scene.as_ref()?.objects.get(i))
                    .is_some_and(|o| !o.kind.eq_ignore_ascii_case("Vegetation")),
            inner: super::textured::Scene {
                mesh: mesh.clone(),
                surfaces: state.level_surfaces.clone(),
                diffuse: None,
                normal_map: None,
                emissive: None,
                specular: None,
                gloss: None,
                yaw: state.yaw,
                pitch: state.pitch,
                zoom: state.zoom.max(0.2),
                light_strength: state.light_strength,
                emissive_gain: state.emissive_strength,
                emissive_time: state.decal_time,
                pan_x: state.pan_x,
                pan_y: state.pan_y,
                shift: state.keyboard_modifiers.shift(),
            },
        })
        .width(Fill)
        .height(Length::Fixed(460.0))
        .into()
    } else {
        text("No verified level geometry loaded. Select a map PAK containing mesh or heightmap resources.")
            .size(12).color(MUTED).into()
    }
}

pub(super) fn view(state: &State) -> Element<'_, Message> {
    let Some(scene) = &state.level_scene else {
        return column![
            text("SCENE VIEWPORT").size(15),
            world_viewport(state),
            text("This map has no readable object placement layer; available terrain is still displayed.")
                .size(12).color(MUTED),
        ].spacing(8).into();
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
                    .style(if state.level_selection.indices.contains(&index) {
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
            text(if state.level_read_only {
                "Imported source transforms · read-only"
            } else {
                "Edit local transform (position XYZ, rotation WXYZ, scale XYZ)"
            })
            .size(12),
            text_input("Position: x, y, z", &state.level_edit_position)
                .on_input(Message::LevelEditPositionChanged),
            text_input("Rotation: w, x, y, z", &state.level_edit_rotation)
                .on_input(Message::LevelEditRotationChanged),
            text_input("Scale: x, y, z", &state.level_edit_scale)
                .on_input(Message::LevelEditScaleChanged),
            button("Apply to preview")
                .on_press_maybe((!state.level_read_only && !object.kind.eq_ignore_ascii_case("Vegetation")).then_some(Message::LevelApplyTransform)),
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
            text("PERSPECTIVE VIEWPORT · ORIGINAL LEVEL GEOMETRY").size(14),
            world_viewport(state),
            row![
                button("Perspective").on_press(Message::ViewerCameraPreset("Isometric")),
                button("Top").on_press(Message::ViewerCameraPreset("Top")),
                button("Front").on_press(Message::ViewerCameraPreset("Front")),
                button(if state.level_show_overview { "Hide 2D Overview" } else { "Show 2D Overview" })
                    .on_press(Message::LevelToggleOverview),
            ].spacing(8),
            row![
                text("Yaw").size(12),
                iced::widget::slider(-180.0..=180.0, state.yaw, Message::YawChanged).width(Fill),
                text("Pitch").size(12),
                iced::widget::slider(-85.0..=85.0, state.pitch, Message::PitchChanged).width(Fill),
                text("Zoom").size(12),
                iced::widget::slider(0.2..=4.0, state.zoom.max(0.2), Message::ZoomChanged).width(Fill),
                button("Reset Camera").on_press(Message::ViewerResetCamera),
            ].spacing(8),
            if state.level_show_overview {
                Element::from(column![
                    row![
                        text("PLACEMENT OVERVIEW").size(12),
                        text(format!("{} authored objects", scene.objects.len())).size(12),
                    ].spacing(10),
                    canvas_widget(SceneOverview {
                        scene: scene.clone(),
                        selected_indices: state.level_selection.indices.clone(),
                        zoom: state.level_zoom.max(0.3),
                    }).width(Fill).height(Length::Fixed(175.0)),
                    row![
                        text("Overview zoom").size(12),
                        iced::widget::slider(
                            0.3..=3.0,
                            state.level_zoom.max(0.3),
                            Message::LevelZoomChanged
                        ).width(Length::Fixed(170.0)),
                        text("Click a point to inspect its placement").size(12).color(MUTED),
                    ].spacing(10),
                ].spacing(7))
            } else {
                Element::from(text("Select objects in the Scene Outliner. Orbit, pan and zoom in the 3D viewport.").size(12).color(MUTED))
            },
        ].spacing(8).width(Length::FillPortion(4)),
        column![
            text("SCENE OUTLINER").size(14),
            text_input("Filter layer, object or model...", &state.level_filter)
                .on_input(Message::LevelFilterChanged),
            scrollable(outliner).height(Length::Fixed(240.0)),
            text(format!(
                "{visible} matching placements · {} selected · Shift range / Ctrl toggle",
                state.level_selection.indices.len()
            ))
            .size(12)
            .color(MUTED),
            details,
            row![
                button("Undo").on_press_maybe(
                    (!state.level_read_only && !state.level_undo.is_empty()).then_some(Message::LevelUndoTransform)
                ),
                button("Redo").on_press_maybe(
                    (!state.level_read_only && !state.level_redo.is_empty()).then_some(Message::LevelRedoTransform)
                ),
                text(format!("{} undo steps", state.level_undo.len()))
                    .size(12)
                    .color(MUTED),
            ]
            .spacing(8),
            text(format!(
                "Save {} changed placement(s) as a new level layer",
                state.level_dirty_count
            ))
            .size(12),
            text_input(
                "New absolute .lyr or .xml output path",
                &state.level_edit_output
            )
            .on_input(Message::LevelEditOutputChanged),
            button("Export edited level copy")
                .on_press_maybe((!state.level_read_only && state.level_dirty_count > 0).then_some(Message::LevelExportEdits)),
            text("Original level is read-only. Child world positions update in the preview.")
                .size(12)
                .color(MUTED),
        ]
        .spacing(7)
        .width(Length::FillPortion(2)),
    ]
    .spacing(12)
    .into()
}

#[cfg(test)]
mod editing_tests {
    use super::*;

    #[test]
    fn transform_fields_preserve_source_order_and_reject_nonfinite_values() {
        let original = Transform {
            position: [1.0, -2.0, 3.0],
            quaternion: [1.0, 0.0, 0.0, 0.0],
            scale: [1.0, 2.0, 3.0],
        };
        let (p, q, scale) = format_transform(original);
        assert_eq!(parse_transform(&p, &q, &scale).unwrap(), original);
        assert!(parse_transform("1, 2", &q, &scale).is_err());
        assert!(parse_transform("NaN, 2, 3", &q, &scale).is_err());
        assert!(parse_transform("1, Infinity, 3", &q, &scale).is_err());
        assert!(parse_transform(&p, "1, NaN, 0, 0", &scale).is_err());
        assert!(parse_transform(&p, &q, "1, 1, -inf").is_err());
        assert!(parse_transform(&p, "1,0,0", &scale).is_err());
        let local = parse_transform("1, 2, 3", "0.70710677, 0.70710677, 0, 0", "2, 3, 4").unwrap();
        assert_eq!(local.quaternion[0], local.quaternion[1]);
        assert_eq!(local.position, [1., 2., 3.]);
    }
}
