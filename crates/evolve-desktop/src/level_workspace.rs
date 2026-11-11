//! Placement overview and hierarchy inspector for verified CryEngine level
//! scenes. Renders source locations only; does not invent geometry or changes.
use super::{Message, State};
use crate::design;
use evolve_core::{LevelScene, Transform};
use iced::widget::canvas::{self, Path, Stroke};
use iced::widget::{
    button, canvas as canvas_widget, column, container, row, scrollable, space, text,
};
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
    show_wildlife: bool,
    camera: Option<[f32; 3]>,
    spawn_distances: Option<evolve_core::zookeeper::SpawnDistances>,
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
            if obj.hidden || (obj.is_wildlife_spawn() && !self.show_wildlife) {
                continue;
            }
            // Area outlines are authored local-space polygons. Rotate by the
            // placement's world quaternion rather than displaying an invented
            // axis-aligned rectangle.
            if (obj.kind.eq_ignore_ascii_case("ZooKeeperShape")
                || obj.kind.eq_ignore_ascii_case("AreaShape"))
                && obj.shape_points.len() >= 2
            {
                let q = obj.world.quaternion;
                let mut vertices = Vec::with_capacity(obj.shape_points.len());
                for vertex in &obj.shape_points {
                    let v = [
                        vertex[0] * obj.world.scale[0],
                        vertex[1] * obj.world.scale[1],
                        vertex[2] * obj.world.scale[2],
                    ];
                    let u = [q[1], q[2], q[3]];
                    let cross = [
                        u[1] * v[2] - u[2] * v[1],
                        u[2] * v[0] - u[0] * v[2],
                        u[0] * v[1] - u[1] * v[0],
                    ];
                    let dot = u[0] * v[0] + u[1] * v[1] + u[2] * v[2];
                    let factor = q[0] * q[0] - u[0] * u[0] - u[1] * u[1] - u[2] * u[2];
                    let rotated = [
                        factor * v[0] + 2.0 * dot * u[0] + 2.0 * q[0] * cross[0],
                        factor * v[1] + 2.0 * dot * u[1] + 2.0 * q[0] * cross[1],
                    ];
                    vertices.push(self.point(
                        [
                            obj.world.position[0] + rotated[0],
                            obj.world.position[1] + rotated[1],
                            0.0,
                        ],
                        bounds,
                        center,
                        extent,
                    ));
                }
                let path = Path::new(|builder| {
                    builder.move_to(vertices[0]);
                    for vertex in vertices.iter().skip(1) {
                        builder.line_to(*vertex);
                    }
                    builder.close();
                });
                frame.stroke(
                    &path,
                    Stroke::default()
                        .with_width(1.5)
                        .with_color(Color::from_rgb(0.95, 0.70, 0.16)),
                );
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
                } else if obj.is_wildlife_spawn() {
                    if let (Some(camera), Some(distances)) = (self.camera, self.spawn_distances) {
                        let dx = obj.world.position[0] - camera[0];
                        let dy = obj.world.position[1] - camera[1];
                        let range = (dx * dx + dy * dy).sqrt();
                        if range < distances.minimum {
                            Color::from_rgb(0.90, 0.35, 0.27)
                        } else if range <= distances.maximum {
                            Color::from_rgb(0.30, 0.88, 0.48)
                        } else {
                            Color::from_rgb(0.95, 0.70, 0.16)
                        }
                    } else {
                        Color::from_rgb(0.95, 0.70, 0.16)
                    }
                } else {
                    Color::from_rgb(0.68, 0.75, 0.83)
                },
            );
        }
        if self.show_wildlife
            && let (Some(camera), Some(distances)) = (self.camera, self.spawn_distances)
        {
            let origin = self.point(camera, bounds, center, extent);
            let scale = bounds.width.min(bounds.height) * 0.80 * self.zoom.clamp(0.3, 3.0) / extent;
            for (radius, color) in [
                (distances.minimum, Color::from_rgb(0.90, 0.35, 0.27)),
                (distances.maximum, Color::from_rgb(0.30, 0.88, 0.48)),
                (distances.despawn, Color::from_rgb(0.91, 0.73, 0.26)),
            ] {
                if radius > 0.0 && radius * scale < 10000.0 {
                    frame.stroke(
                        &Path::circle(origin, radius * scale),
                        Stroke::default().with_width(1.3).with_color(color),
                    );
                }
            }
            frame.fill(&Path::circle(origin, 3.0), Color::WHITE);
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
            if obj.hidden || (obj.is_wildlife_spawn() && !self.show_wildlife) {
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
            // The axes follow a transform in progress.
            selected_pivot: state
                .level_transform
                .as_ref()
                .and_then(|operation| operation.results().first().map(|t| t.position))
                .or_else(|| {
                    state
                        .level_selected
                        .and_then(|i| state.level_scene.as_ref()?.objects.get(i))
                        .map(|o| o.world.position)
                }),
            transforming: state.level_transform.is_some(),
            brush: state.level_project.is_some()
                && !state.level_read_only
                && state.level_tool != super::terrain_brush::LevelTool::Select,
            brush_ring: state.terrain_brush_ring.clone(),
            brush_invert: state.keyboard_modifiers.control(),
            editable: state.level_project.is_some()
                && state
                    .level_selected
                    .and_then(|i| state.level_scene.as_ref()?.objects.get(i))
                    .is_some_and(|o| !o.kind.eq_ignore_ascii_case("Vegetation")),
            inner: super::textured::Scene {
                framing: None,
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
                armor_level: 1.0,
                emissive_time: state.decal_time,
                pan_x: state.pan_x,
                pan_y: state.pan_y,
                shift: state.keyboard_modifiers.shift(),
                level_camera: state.level_camera,
                world: state.level_world.clone(),
                level_hour: state.level_time_of_day,
                level_full_textures: state.level_full_textures,
                level_terrain_unlit: state.level_terrain_unlit,
                viewer_full_textures: state.level_full_textures,
                viewer_gpu_remaining: None,
            },
        })
        .width(Fill)
        .height(Fill)
        .into()
    } else {
        text("No verified level geometry loaded. Select a map PAK containing mesh or heightmap resources.")
            .size(12).color(MUTED).into()
    }
}

pub(super) fn fullscreen_view(state: &State) -> Element<'_, Message> {
    column![
        row![
            text("Level Editor  /  Perspective").size(13),
            space().width(Fill),
            design::button("Exit Fullscreen (Esc)")
                .style(design::secondary)
                .on_press(Message::ToggleLevelViewportFullscreen),
        ]
        .spacing(8)
        .align_y(iced::Center),
        world_viewport(state),
    ]
    .spacing(4)
    .width(Fill)
    .height(Fill)
    .into()
}

pub(super) fn view(state: &State) -> Element<'_, Message> {
    let Some(scene) = &state.level_scene else {
        return container(
            column![
                text("Perspective  /  Lit").size(13),
                world_viewport(state),
                text("No object placement layer · Available terrain remains visible")
                    .size(12)
                    .color(MUTED),
            ]
            .spacing(8),
        )
        .padding(10)
        .width(Fill)
        .height(Fill)
        .style(design::panel)
        .into();
    };
    let mut outliner = column![].spacing(2);
    let query = state.level_filter.to_ascii_lowercase();
    let mut visible = 0usize;
    let mut groups = std::collections::BTreeMap::<String, Vec<(usize, &_)>>::new();
    let wildlife_count = scene
        .objects
        .iter()
        .filter(|obj| obj.is_wildlife_spawn())
        .count();
    for (index, obj) in scene.objects.iter().enumerate() {
        if !query.is_empty()
            && !obj.name.to_ascii_lowercase().contains(&query)
            && !obj.layer.to_ascii_lowercase().contains(&query)
            && !obj.model.to_ascii_lowercase().contains(&query)
        {
            continue;
        }
        let group = if obj.is_wildlife_spawn() || obj.kind.eq_ignore_ascii_case("ZooKeeperShape") {
            "Wildlife Spawns (Source-Backed)".to_owned()
        } else {
            obj.layer.clone()
        };
        groups.entry(group).or_default().push((index, obj));
        visible += 1;
    }
    let mut shown = 0;
    for (group, objects) in groups {
        let collapsed = state.level_collapsed_groups.contains(&group);
        outliner = outliner.push(
            design::button(
                text(format!(
                    "{} {} ({})",
                    if collapsed { "▸" } else { "▾" },
                    group,
                    objects.len()
                ))
                .size(12),
            )
            .width(Fill)
            .style(design::secondary)
            .on_press(Message::LevelToggleOutlinerGroup(group)),
        );
        if collapsed {
            continue;
        }
        for (index, obj) in objects {
            if shown >= 180 {
                break;
            }
            outliner = outliner.push(
                button(
                    text(if obj.kind.eq_ignore_ascii_case("CreatureSpawn") {
                        format!(
                            "CreatureSpawn · {} / {} [{}]",
                            obj.layer,
                            obj.name,
                            if obj.populations.is_empty() {
                                "unknown".to_owned()
                            } else {
                                obj.populations.join(", ")
                            }
                        )
                    } else if obj.kind.eq_ignore_ascii_case("ZooKeeperShape") {
                        format!("Spawn area · {} / {}", obj.layer, obj.name)
                    } else {
                        format!("{}  /  {}", obj.layer, obj.name)
                    })
                    .size(12),
                )
                .width(Fill)
                .padding([7, 9])
                .style(if state.level_selection.indices.contains(&index) {
                    design::primary
                } else {
                    design::text_button
                })
                .on_press(Message::LevelSelect(index)),
            );
            shown += 1;
        }
    }
    let details: Element<'_, Message> = if let Some(object) = state
        .level_selected
        .and_then(|index| scene.objects.get(index))
    {
        column![
            text(object.name.as_str()).size(16),
            text(object.kind.as_str()).size(12).style(design::muted),
            text(format!("Position: {:?}", object.world.position)).size(12),
            text(format!("Rotation (wxyz): {:?}", object.world.quaternion)).size(12),
            text(format!("Scale: {:?}", object.world.scale)).size(12),
            text(format!(
                "Model: {}",
                if object.model.is_empty() && object.is_wildlife_spawn() {
                    "Unresolved source population (marker only)"
                } else {
                    &object.model
                }
            ))
            .size(12),
            text(if object.kind.eq_ignore_ascii_case("CreatureSpawn") {
                format!(
                    "Populations: {} · Home area: {}",
                    object.populations.join(", "),
                    object.home_area
                )
            } else if object.kind.eq_ignore_ascii_case("ZooKeeperShape") {
                format!(
                    "Spawn region · {} authored polygon points",
                    object.shape_points.len()
                )
            } else {
                String::new()
            })
            .size(12),
            text(format!("Status: {}", object.status))
                .size(12)
                .color(MUTED),
            text(if state.level_read_only {
                "Source transform · read-only"
            } else {
                "Local transform"
            })
            .size(12)
            .style(design::muted),
            // G move · R rotate · S scale in the viewport; or type exact
            // values here and press Enter, which saves them.
            design::text_input("Position: x, y, z", &state.level_edit_position)
                .on_input(Message::LevelEditPositionChanged)
                .on_submit_maybe(transform_submit(state, object)),
            design::text_input("Rotation: w, x, y, z", &state.level_edit_rotation)
                .on_input(Message::LevelEditRotationChanged)
                .on_submit_maybe(transform_submit(state, object)),
            design::text_input("Scale: x, y, z", &state.level_edit_scale)
                .on_input(Message::LevelEditScaleChanged)
                .on_submit_maybe(transform_submit(state, object)),
            text(if state.level_read_only || state.level_project.is_none() {
                ""
            } else {
                "G move · R rotate · S scale in the view, or edit a value and press Enter"
            })
            .size(11)
            .style(design::muted),
            crate::level_authoring::placement_editor(state, object),
        ]
        .spacing(6)
        .into()
    } else {
        text("Select an object or click its point on the map to inspect original transforms.")
            .size(12)
            .color(MUTED)
            .into()
    };
    let viewport = container(
        column![
            row![
                text("Perspective  /  Lit").size(13),
                space().width(Fill),
                text(if state.level_read_only {
                    "SOURCE SCENE"
                } else {
                    "PROJECT PREVIEW"
                })
                .size(11)
                .style(design::muted),
            ]
            .align_y(iced::Center),
            world_viewport(state),
            row![
                text("Camera speed").size(12),
                iced::widget::slider(
                    1.0..=2500.0,
                    state.level_camera.map_or(120.0, |c| c.speed),
                    Message::LevelCameraSpeedChanged
                )
                .width(Length::Fixed(160.0)),
                text(format!(
                    "{:.0} units/s",
                    state.level_camera.map_or(0.0, |c| c.speed)
                ))
                .size(12)
                .style(design::muted),
                text(format!("Zoom {:.1}×", state.zoom)).size(12),
                iced::widget::slider(
                    0.2..=80.0,
                    state.zoom.clamp(0.2, 80.0),
                    Message::ZoomChanged
                )
                .width(Length::Fixed(160.0)),
                space().width(Fill),
                text("Hold RMB: look, WASD fly, Q/E elevate · F focus")
                    .size(11)
                    .style(design::muted),
            ]
            .spacing(10)
            .align_y(iced::Center),
            if state.level_show_overview {
                Element::from(
                    column![
                        row![
                            text("2D Placement Overview").size(13),
                            text(format!("{} authored objects", scene.objects.len())).size(12),
                        ]
                        .spacing(10),
                        canvas_widget(SceneOverview {
                            scene: scene.clone(),
                            selected_indices: state.level_selection.indices.clone(),
                            zoom: state.level_zoom.max(0.3),
                            show_wildlife: state.level_show_wildlife,
                            camera: state.level_camera.map(|camera| camera.position),
                            spawn_distances: state.level_spawn_distances,
                        })
                        .width(Fill)
                        .height(Length::Fixed(175.0)),
                        row![
                            text("Overview zoom").size(12),
                            iced::widget::slider(
                                0.3..=3.0,
                                state.level_zoom.max(0.3),
                                Message::LevelZoomChanged
                            )
                            .width(Length::Fixed(170.0)),
                            text(if state.level_show_wildlife && state.level_spawn_distances.is_some() {
                                "Camera rings: red min · green max · amber despawn (authored, not simulated)"
                            } else { "Click a point to inspect its placement" })
                                .size(12)
                                .color(MUTED),
                        ]
                        .spacing(10),
                    ]
                    .spacing(7),
                )
            } else {
                Element::from(space().height(Length::Fixed(0.0)))
            },
        ]
        .spacing(10),
    )
    .padding(12)
    .width(Length::FillPortion(4))
    .height(Fill)
    .style(design::panel);
    let mut outliner_content = column![
        design::button(if state.level_outliner_collapsed {
            "▸ Scene Outliner"
        } else {
            "▾ Scene Outliner"
        })
        .style(design::text_button)
        .on_press(Message::LevelToggleOutliner),
    ]
    .spacing(8);
    if !state.level_outliner_collapsed {
        outliner_content = outliner_content
            .push(
                row![
                    design::button(if state.level_show_wildlife {
                        "☑ Wildlife Spawns"
                    } else {
                        "☐ Wildlife Spawns"
                    })
                    .style(design::secondary)
                    .on_press(Message::LevelToggleWildlife),
                    text(format!("{wildlife_count} markers / areas"))
                        .size(11)
                        .style(design::muted),
                ]
                .spacing(8)
                .align_y(iced::Center),
            )
            .push(
                design::text_input("Search scene objects", &state.level_filter)
                    .on_input(Message::LevelFilterChanged),
            )
            .push(scrollable(outliner).height(Fill))
            .push(
                text(format!(
                    "{visible} objects · {} selected · Shift/Ctrl to multi-select",
                    state.level_selection.indices.len()
                ))
                .size(11)
                .style(design::muted),
            );
    }
    let outliner_panel = container(outliner_content)
        .padding(12)
        .width(Fill)
        .height(if state.level_outliner_collapsed {
            Length::Shrink
        } else {
            Length::FillPortion(3)
        })
        .style(design::panel);
    let details_panel =
        container(column![text("Details").size(15), scrollable(details).height(Fill),].spacing(8))
            .padding(12)
            .width(Fill)
            .height(Length::FillPortion(3))
            .style(design::panel);
    // The right panel is an editor first. Time and texture choices stay
    // compact; long engine/source diagnostics have their own scrollable tab.
    let scene_settings_panel = container(
        column![
            text("Scene Settings").size(15),
            row![
                text("Time of day").size(12),
                space().width(Fill),
                text(super::level_lighting::Lighting::label(
                    state.level_time_of_day
                ))
                .size(12)
                .style(design::muted),
            ]
            .align_y(iced::Center),
            iced::widget::slider(
                0.0..=24.0,
                state.level_time_of_day,
                Message::LevelTimeOfDayChanged,
            )
            .step(0.25_f32)
            .width(Fill),
            row![
                design::button("Dawn").on_press(Message::LevelTimePreset(6.0)),
                design::button("Noon").on_press(Message::LevelTimePreset(12.0)),
                design::button("Dusk").on_press(Message::LevelTimePreset(18.0)),
                design::button("Night").on_press(Message::LevelTimePreset(0.0)),
            ]
            .spacing(4),
            design::button(if state.level_full_textures {
                "Textures: Full quality"
            } else {
                "Textures: Balanced"
            })
            .on_press(Message::LevelFullTexturesToggled),
            design::button(if state.level_terrain_unlit {
                "Terrain: Source RGB (unlit)"
            } else {
                "Terrain: Lit preview"
            })
            .on_press(Message::LevelTerrainSourceColorToggled),
            text(state.level_world.as_ref().map_or_else(
                || "No world GPU resources loaded.".to_owned(),
                |world| {
                    use std::sync::atomic::Ordering::Relaxed;
                    let p = &world.gpu_progress;
                    format!(
                        "Scene loaded: {} / {} groups · {} pending · {} deferred",
                        p.loaded.load(Relaxed),
                        world.groups.len(),
                        p.pending.load(Relaxed),
                        p.deferred.load(Relaxed),
                    )
                },
            ))
            .size(11)
            .style(design::muted),
            crate::level_authoring::terrain_editor(state),
            text("Rendering details and source warnings are in Diagnostics.")
                .size(11)
                .style(design::muted),
        ]
        .spacing(8),
    )
    .padding(12)
    .width(Fill)
    .style(design::panel);
    let diagnostics_panel = container(
        column![
            row![
                text("Rendering Diagnostics").size(15),
                space().width(Fill),
                text(format!("{} scene notes", state.level_diagnostics.len()))
                    .size(11)
                    .style(design::muted),
            ]
            .align_y(iced::Center),
            scrollable(
                column![
            text(state.level_world.as_ref().map_or_else(
                || "GPU residency: no world loaded.".to_owned(),
                |world| {
                    use std::sync::atomic::Ordering::Relaxed;
                    let p = &world.gpu_progress;
                    let mib = 1_048_576_f64;
                    format!(
                        "GPU streaming: {} of {} resident groups; {} pending, {} deferred. Textures {:.1} MiB, geometry {:.1} MiB, peak {:.1} MiB (estimated). Full source resolution retained; available VRAM not exposed by wgpu.",
                        p.loaded.load(Relaxed), world.groups.len(), p.pending.load(Relaxed),
                        p.deferred.load(Relaxed), p.textures.load(Relaxed) as f64 / mib,
                        p.geometry.load(Relaxed) as f64 / mib,
                        p.peak.load(Relaxed) as f64 / mib,
                    )
                },
            )).size(11).style(design::muted),
            text(state.level_world.as_ref().map_or_else(
                || "Source diffuse coverage: no world loaded.".to_owned(),
                |world| {
                    let (resolved, unresolved) = world
                        .groups
                        .iter()
                        .flat_map(|group| &group.surfaces)
                        .filter(|surface| surface.visible && surface.index_count > 0)
                        .fold((0usize, 0usize), |(resolved, unresolved), surface| {
                            if surface.diffuse.is_some() {
                                (resolved + 1, unresolved)
                            } else {
                                (resolved, unresolved + 1)
                            }
                        });
                    format!(
                        "Source diffuse maps: {resolved} resolved material spans, {unresolved} without verified diffuse. Unresolved source assets cannot be fixed by GPU streaming alone."
                    )
                },
            ))
            .size(11)
            .style(design::muted),
            text("Full: loaded assets remain on the GPU even when offscreen; no manager GPU residency cap. Balanced: new groups may be deferred at its memory target, but loaded assets are never evicted. Hardware VRAM limits still apply.")
                .size(11)
                .style(design::muted),
            text(if let Some(source) = state.level_world.as_ref()
                .and_then(|world| world.time_of_day.as_ref()) {
                format!("Original TimeOfDay.xml: {} variables, {} animated splines, {} out-of-day editor keys ignored; {}. Source sun/sky and tangent-aware curves are preview approximations; native CryEngine particles, fog, HDR, GI and shaders are not rendered.",
                    source.count(), source.keyed_variables, source.ignored_out_of_range_keys, source.playback_window_description())
            } else {
                "No verified original TimeOfDay.xml; using approximate lighting.".to_owned()
            })
                .size(11)
                .style(design::muted),
            text(state.level_world.as_ref().map_or_else(
                || "Fog preview disabled: no verified map world.".to_owned(),
                |world| match (world.fog_profile, world.time_of_day.as_deref()) {
                    (Some(fog), Some(tod)) if fog.preview_uniform(Some(tod), state.level_time_of_day)[1][0] > 0.5 => {
                        if fog.height_preview_uniform(Some(tod), state.level_time_of_day)[1][0] > 0.5 {
                            let height = tod.height_fog(state.level_time_of_day).expect("height preview uniform requires valid source heights");
                            format!(
                                "Source-backed analytical HEIGHT FOG PREVIEW: {:.0}–{:.0}m altitude, density {:.3}→{:.3}; TimeOfDay + Environment.xml. Not native Evolve fog, scattering, shadows, HDR or GI.",
                                height.bottom_height, height.top_height, height.bottom_density, height.top_density
                            )
                        } else {
                            format!(
                                "Distance-only fog PREVIEW: Environment.xml ViewDistance {:.0}m, LDR density multiplier {:.2}; height-density source missing, invalid, or has a zero endpoint (unsupported by exponential model). Not native.",
                                fog.view_distance, fog.ldr_density_multiplier
                            )
                        }
                    },
                    (Some(_), Some(_)) => "Fog preview disabled: original TimeOfDay Fog color missing or density zero.".to_owned(),
                    _ => "Fog preview disabled: exact original Environment.xml and TimeOfDay.xml are both required.".to_owned(),
                },
            ))
                .size(11)
                .style(design::muted),
            text(state.level_world.as_ref().and_then(|world| world.time_of_day.as_ref()).map_or_else(
                || "No verified TimeOfDay fog-variable inventory.".to_owned(),
                |tod| tod.fog_source_diagnostics(state.level_time_of_day),
            ))
                .size(11)
                .style(design::muted),
            text(state.level_world.as_ref().map_or_else(
                || "Original effects/shaders unavailable.".to_owned(),
                |world| world.source_resources.description(),
            ))
                .size(11)
                .style(design::muted),
                    text(format!("Original imported scene audit · {} notes", state.level_diagnostics.len()))
                        .size(12),
                    text(if state.level_diagnostics.is_empty() {
                        "No imported source diagnostics for this level.".to_owned()
                    } else {
                        state.level_diagnostics.join("\n\n")
                    })
                    .size(11)
                    .style(design::muted),
                ]
                .spacing(8),
            )
            .height(Fill),
        ]
        .spacing(8),
    )
    .padding(12)
    .width(Fill)
    .height(Fill)
    .style(design::panel);
    let changes_panel = container(
        column![
            text("Changes").size(15),
            row![
                design::button("Undo").on_press_maybe(
                    (!state.level_read_only && !state.level_undo.is_empty())
                        .then_some(Message::LevelUndoTransform)
                ),
                design::button("Redo").on_press_maybe(
                    (!state.level_read_only && !state.level_redo.is_empty())
                        .then_some(Message::LevelRedoTransform)
                ),
                text(format!("{} preview undo steps", state.level_undo.len()))
                    .size(12)
                    .color(MUTED),
            ]
            .spacing(8),
            row![
                design::button("Undo saved edit").on_press_maybe(
                    (!state.level_read_only
                        && !state.busy
                        && state.level_dirty_count == 0
                        && !state.level_committed_undo.is_empty())
                    .then_some(Message::LevelUndoCommitted)
                ),
                design::button("Redo saved edit").on_press_maybe(
                    (!state.level_read_only
                        && !state.busy
                        && state.level_dirty_count == 0
                        && !state.level_committed_redo.is_empty())
                    .then_some(Message::LevelRedoCommitted)
                ),
            ]
            .spacing(8),
            text(format!(
                "{} saved edit(s) available to undo (session only)",
                state.level_committed_undo.len()
            ))
            .size(12)
            .color(MUTED),
            text(format!("{} unsaved placements", state.level_dirty_count)).size(12),
            design::text_input(
                "New absolute .lyr or .xml output path",
                &state.level_edit_output
            )
            .on_input(Message::LevelEditOutputChanged),
            design::button("Export Edited Level Copy")
                .style(design::primary)
                .on_press_maybe(
                    (!state.level_read_only && state.level_dirty_count > 0)
                        .then_some(Message::LevelExportEdits)
                ),
            text("Original PAK is never changed.")
                .size(11)
                .style(design::muted),
        ]
        .spacing(7),
    )
    .padding(12)
    .width(Fill)
    .style(design::panel);
    let actor_content: Element<'_, Message> = if state.level_read_only {
        column![outliner_panel, details_panel]
            .spacing(8)
            .height(Fill)
            .into()
    } else {
        column![outliner_panel, details_panel, changes_panel]
            .spacing(8)
            .height(Fill)
            .into()
    };
    let tab = state.level_sidebar_tab;
    let tabs = row![
        design::button("Actors")
            .style(if tab == super::LevelSidebarTab::Actors {
                design::primary
            } else {
                design::text_button
            })
            .on_press(Message::LevelSidebarTabSelected(
                super::LevelSidebarTab::Actors,
            )),
        design::button("Scene")
            .style(if tab == super::LevelSidebarTab::Scene {
                design::primary
            } else {
                design::text_button
            })
            .on_press(Message::LevelSidebarTabSelected(
                super::LevelSidebarTab::Scene,
            )),
        design::button("Diagnostics")
            .style(if tab == super::LevelSidebarTab::Diagnostics {
                design::primary
            } else {
                design::text_button
            })
            .on_press(Message::LevelSidebarTabSelected(
                super::LevelSidebarTab::Diagnostics,
            )),
    ]
    .spacing(4);
    let selected_panel: Element<'_, Message> = match tab {
        super::LevelSidebarTab::Actors => actor_content,
        super::LevelSidebarTab::Scene => scene_settings_panel.into(),
        super::LevelSidebarTab::Diagnostics => diagnostics_panel.into(),
    };
    let inspector = column![tabs, selected_panel]
        .spacing(8)
        .width(Length::Fixed(350.0))
        .height(Fill);
    row![viewport, inspector].spacing(8).height(Fill).into()
}

/// Enter in a transform field saves it, for an editable placement.
fn transform_submit(state: &State, object: &evolve_core::PlacedObject) -> Option<Message> {
    (!state.level_read_only
        && state.level_project.is_some()
        && !object.kind.eq_ignore_ascii_case("Vegetation"))
    .then_some(Message::LevelApplyTransform)
}

#[cfg(test)]
mod editing_tests {
    use super::*;

    #[test]
    fn full_imported_scene_notes_remain_available_separately_from_footer() {
        let notes = vec![
            "Missing texture: textures/terrain/rock.dds".to_owned(),
            "Original terrain layer IDs: 14".to_owned(),
        ];
        let state = State {
            level_diagnostics: notes.clone(),
            status: "Map Tutorial opened read-only · 2 source notes (see Diagnostics)".into(),
            ..State::default()
        };
        assert_eq!(state.level_diagnostics, notes);
        assert!(!state.status.contains("Missing texture"));
    }

    #[test]
    fn level_inspector_defaults_to_actors_instead_of_verbose_scene_diagnostics() {
        assert_eq!(
            super::super::LevelSidebarTab::default(),
            super::super::LevelSidebarTab::Actors
        );
        assert_ne!(
            super::super::LevelSidebarTab::default(),
            super::super::LevelSidebarTab::Diagnostics
        );
    }

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
