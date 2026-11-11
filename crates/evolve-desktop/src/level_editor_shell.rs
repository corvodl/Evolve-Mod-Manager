//! Native-window scene-editor chrome inspired by familiar professional
//! 3D authoring applications. The actual world view remains the existing
//! Iced/wgpu widget; this only supplies task-specific menus and panels.
use crate::{Message, State, design, level_workspace, menu_popup};
use iced::widget::{button, column, container, row, scrollable, space, text};
use iced::{Background, Border, Element, Fill, Length};

fn menu_button(name: &'static str, id: &'static str) -> iced::widget::Button<'static, Message> {
    button(text(name).size(12))
        .padding([5, 10])
        .style(design::text_button)
        .on_press(Message::LibraryMenu(id))
}
/// The tools of an editable project, shown in the top bar: Select, Sculpt
/// and Paint, the active brush's settings and the unsaved-terrain controls.
fn tool_bar(state: &State) -> iced::widget::Row<'_, Message> {
    use crate::terrain_brush::{LevelTool, SculptMode};
    use iced::widget::{slider, text_input};
    let tool = |label: &'static str, value: LevelTool| {
        design::button(label)
            .style(if state.level_tool == value {
                design::primary
            } else {
                design::secondary
            })
            .on_press(Message::LevelSetTool(value))
    };
    let mut bar = row![
        text("│").size(15).style(design::muted),
        tool("Select", LevelTool::Select),
        tool("Sculpt", LevelTool::Sculpt),
        tool("Paint", LevelTool::Paint),
    ]
    .spacing(5)
    .align_y(iced::Alignment::Center);
    let brush = state.terrain_brush;
    if state.level_tool == LevelTool::Select {
        // The gizmo the selection shows, as Blender's Move/Rotate/Scale tools.
        use crate::level_transform::Mode;
        let gizmo = |label: &'static str, value: Mode| {
            design::button(label)
                .style(if state.level_gizmo == value {
                    design::primary
                } else {
                    design::secondary
                })
                .on_press(Message::LevelGizmoMode(value))
        };
        bar = bar
            .push(text("│").size(15).style(design::muted))
            .push(gizmo("Move", Mode::Move))
            .push(gizmo("Rotate", Mode::Rotate))
            .push(gizmo("Scale", Mode::Scale));
    }
    if state.level_tool != LevelTool::Select {
        bar = bar.push(text("│").size(15).style(design::muted));
        if state.level_tool == LevelTool::Sculpt {
            let mode = |label: &'static str, value: SculptMode| {
                design::button(label)
                    .style(if brush.mode == value {
                        design::primary
                    } else {
                        design::secondary
                    })
                    .on_press(Message::LevelSculptMode(value))
            };
            bar = bar
                .push(mode("Raise", SculptMode::Raise))
                .push(mode("Flatten", SculptMode::Flatten))
                .push(mode("Smooth", SculptMode::Smooth));
            // Legacy-ported maps' terrain.dat has holes to turn into ground.
            if state
                .terrain_session
                .as_ref()
                .is_some_and(|session| session.can_create())
            {
                bar = bar.push(mode("Create", SculptMode::Create));
            }
        } else {
            bar = bar.push(text("Surface").size(11)).push(
                text_input("ID", &state.terrain_brush_surface)
                    .on_input(Message::LevelBrushSurface)
                    .size(12)
                    .width(Length::Fixed(48.0)),
            );
        }
        let setting = |label: &'static str,
                       range: std::ops::RangeInclusive<f32>,
                       value: f32,
                       step: f32,
                       shown: String,
                       on_change: fn(f32) -> Message| {
            row![
                text(label).size(11),
                slider(range, value, on_change)
                    .step(step)
                    .width(Length::Fixed(90.0)),
                text(shown).size(11).width(Length::Fixed(44.0)),
            ]
            .spacing(4)
            .align_y(iced::Alignment::Center)
        };
        bar = bar.push(setting(
            "Radius",
            0.5..=100.0,
            brush.radius,
            0.5,
            format!("{:.1} m", brush.radius),
            Message::LevelBrushRadius,
        ));
        if state.level_tool == LevelTool::Sculpt {
            bar = bar
                .push(setting(
                    "Strength",
                    0.01..=1.0,
                    brush.strength,
                    0.01,
                    format!("{:.0}%", brush.strength * 100.0),
                    Message::LevelBrushStrength,
                ))
                .push(setting(
                    "Hardness",
                    0.0..=0.95,
                    brush.hardness,
                    0.05,
                    format!("{:.0}%", brush.hardness * 100.0),
                    Message::LevelBrushHardness,
                ));
        }
    }
    let session = state.terrain_session.as_ref();
    let dirty = session.is_some_and(|s| s.is_dirty());
    let action = |label: &'static str, primary: bool, on: bool, message: Message| {
        let button = design::button(label).style(if primary {
            design::primary
        } else {
            design::secondary
        });
        if on { button.on_press(message) } else { button }
    };
    if state.level_tool == LevelTool::Select {
        // The map's unsaved changes: live in the view, written by Save Map.
        let draft = state.level_draft.as_ref();
        let pending = draft.is_some_and(crate::level_draft::Draft::changed);
        let count = draft.map_or(0, |draft| draft.undo.len());
        return bar
            .push(space().width(Fill))
            .push(
                text(if pending {
                    format!(
                        "{count} unsaved change{}",
                        if count == 1 { "" } else { "s" }
                    )
                } else {
                    String::new()
                })
                .size(11)
                .style(design::muted),
            )
            .push(action(
                "Undo",
                false,
                draft.is_some_and(|d| !d.undo.is_empty()),
                Message::LevelDraftUndo,
            ))
            .push(action(
                "Redo",
                false,
                draft.is_some_and(|d| !d.redo.is_empty()),
                Message::LevelDraftRedo,
            ))
            .push(action("Discard", false, pending, Message::LevelDiscardMap))
            .push(action(
                "Save Map",
                true,
                pending && !state.busy,
                Message::LevelSaveMap,
            ));
    }
    bar.push(space().width(Fill))
        .push(action(
            "Undo",
            false,
            session.is_some_and(|s| s.can_undo()),
            Message::LevelBrushUndo,
        ))
        .push(action(
            "Redo",
            false,
            session.is_some_and(|s| s.can_redo()),
            Message::LevelBrushRedo,
        ))
        .push(action("Discard", false, dirty, Message::LevelBrushDiscard))
        .push(action(
            "Save Terrain",
            true,
            dirty && !state.busy,
            Message::LevelBrushSave,
        ))
}

/// Keys for the active tool, shown in the status bar.
fn tool_hint(state: &State) -> &'static str {
    use crate::terrain_brush::LevelTool;
    match state.level_tool {
        LevelTool::Select => {
            "Click select · G move · R/S rotate/scale gizmo · Del delete · Ctrl+C/V copy, paste · Shift+D duplicate · Ctrl+Z undo · Ctrl+S save map"
        }
        LevelTool::Sculpt => {
            "LMB brush · Ctrl+LMB lower · Ctrl+Z undo · Ctrl+S save terrain · RMB look"
        }
        LevelTool::Paint => "LMB paint · RMB look · WASD fly",
    }
}

fn command<'a>(caption: &'static str, msg: Message) -> iced::widget::Button<'a, Message> {
    button(text(caption).size(12))
        .style(design::text_button)
        .padding([6, 12])
        .width(Fill)
        .on_press(Message::MenuCommand(Box::new(msg)))
}

fn map_chooser(state: &State) -> Element<'_, Message> {
    let mut choices = column![
        text("Choose a map to open").size(24),
        text("Select an imported map archive, then a verified map inside it. Original PAKs remain unchanged.")
            .size(12).style(design::muted),
    ].spacing(12);

    if state.busy {
        choices = choices.push(text("Discovering or loading maps…").size(13));
    }
    if let Some((project, maps)) = &state.level_project_choices {
        choices =
            choices.push(text(format!("Maps in editable project {}", project.label)).size(17));
        for (index, map) in maps.iter().enumerate() {
            choices = choices.push(
                design::button(text(&map.name).size(13))
                    .on_press_maybe((!state.busy).then_some(Message::LevelChooseProjectMap(index)))
                    .width(Fill),
            );
        }
        choices = choices.push(
            design::button("Choose a different source").on_press(Message::LevelShowMapChooser),
        );
    } else if let Some((archive, maps)) = &state.level_map_choices {
        choices = choices.push(text(format!("Maps in {}", archive.name)).size(17));
        for (index, map) in maps.iter().enumerate() {
            choices = choices.push(
                design::button(text(&map.name).size(13))
                    .on_press_maybe((!state.busy).then_some(Message::LevelChooseMap(index)))
                    .width(Fill),
            );
        }
        choices = choices.push(
            design::button("Choose a different archive").on_press(Message::LevelShowMapChooser),
        );
    } else {
        if let Some((_, maps, current)) = &state.library_map {
            choices = choices.push(text("Current source / project maps").size(17));
            for (index, map) in maps.iter().enumerate() {
                choices = choices.push(
                    design::button(text(format!(
                        "{}{}",
                        if index == *current { "✓ " } else { "" },
                        map.name
                    )))
                    .on_press_maybe((!state.busy).then_some(Message::LevelMapSelect(index)))
                    .width(Fill),
                );
            }
        }
        choices = choices.push(text("Imported map archives").size(17));
        if let Some(projects) = &state.modding_projects {
            choices = choices.push(text("Editable Modding projects").size(17));
            for (index, project) in projects.iter().enumerate() {
                choices = choices.push(
                    design::button(text(&project.label).size(13))
                        .on_press_maybe((!state.busy).then_some(Message::LevelChooseProject(index)))
                        .width(Fill),
                );
            }
        }
        let mut found = false;
        if let Some(catalog) = &state.library_catalog {
            for (index, archive) in catalog.archives.iter().enumerate() {
                if !crate::level_library::likely_level_pak(archive) {
                    continue;
                }
                found = true;
                choices = choices.push(
                    design::button(text(&archive.name).size(13))
                        .on_press_maybe((!state.busy).then_some(Message::LevelChooseArchive(index)))
                        .width(Fill),
                );
            }
        }
        if !found {
            choices = choices.push(
                text("No imported map archives found. Import a level PAK in Asset Library first.")
                    .size(12)
                    .style(design::muted),
            );
        }
    }
    if state.level_scene.is_some() || state.library_map.is_some() {
        choices = choices
            .push(design::button("Return to open scene").on_press(Message::LevelDismissMapChooser));
    }
    choices = choices.push(text(&state.status).size(11).style(design::muted));
    container(scrollable(choices).width(Length::Fixed(600.0)).height(Fill))
        .center_x(Fill)
        .padding(32)
        .width(Fill)
        .height(Fill)
        .into()
}
fn menu_content(state: &State) -> Option<Element<'_, Message>> {
    let content: Element<'_, Message> = match state.library_menu? {
        "LevelFile" => column![
            command("Open as Modding Project", Message::LibraryMakeModProject),
            command(
                "Open selected Modding project",
                Message::LevelOpenSelectedModProject
            ),
            command(
                "Save edited project layer",
                Message::LevelSaveProjectChanges
            ),
            command("Export edited level copy", Message::LevelExportEdits),
            command("Show Activity / import log", Message::ToggleActivity),
        ]
        .spacing(2)
        .into(),
        "LevelEdit" => column![
            command(
                "Save selected transform to project",
                Message::LevelApplyTransform
            ),
            text("Committed source changes have verified editor backups.")
                .size(11)
                .style(design::muted),
        ]
        .spacing(2)
        .into(),
        "LevelWindow" => column![
            command("Perspective view", Message::ViewerCameraPreset("Isometric")),
            command("Top view", Message::ViewerCameraPreset("Top")),
            command("Front view", Message::ViewerCameraPreset("Front")),
            command(
                if state.level_show_overview {
                    "Hide 2D overview"
                } else {
                    "Show 2D overview"
                },
                Message::LevelToggleOverview
            ),
            command("Focus selection (F)", Message::LevelCameraFocus),
            command("Reset view", Message::LevelCameraReset),
            command(
                "Fullscreen viewport",
                Message::ToggleLevelViewportFullscreen
            ),
            command("Activity and importer diagnostics", Message::ToggleActivity),
        ]
        .spacing(2)
        .into(),
        "LevelTools" => {
            use crate::terrain_brush::LevelTool;
            if state.level_project.is_some() && !state.level_read_only {
                column![
                    command("Select", Message::LevelSetTool(LevelTool::Select)),
                    command("Sculpt terrain", Message::LevelSetTool(LevelTool::Sculpt)),
                    command("Paint terrain", Message::LevelSetTool(LevelTool::Paint)),
                ]
                .spacing(2)
                .into()
            } else {
                column![
                    text("Open the map as a Modding project to use the editing tools.")
                        .size(11)
                        .style(design::muted),
                ]
                .into()
            }
        }
        "LevelBuild" => column![
            command("Export original scene GLB", Message::StartLevelGlbExport),
            command("Export original scene FBX", Message::StartLevelFbxExport),
            text("Exports create new external files; original PAKs remain read-only.")
                .size(11)
                .style(design::muted),
        ]
        .spacing(2)
        .into(),
        "LevelSelectMenu" => column![
            command("Focus selected placement (F)", Message::LevelCameraFocus),
            command("Show 2D placement overview", Message::LevelToggleOverview),
        ]
        .spacing(2)
        .into(),
        "LevelActor" => column![
            command(
                "Apply position / rotation / scale",
                Message::LevelApplyTransform
            ),
            text("Select an actor in the Outliner or the 3D viewport.")
                .size(11)
                .style(design::muted),
        ]
        .spacing(2)
        .into(),
        "LevelHelp" => column![
            text("WASD · fly relative to camera").size(12),
            text("Q / E · descend / ascend").size(12),
            text("Hold RMB and drag · look around").size(12),
            text("Wheel · adjust field of view").size(12),
            text("F · focus selection").size(12),
            text("File → Open as Modding Project to edit").size(12),
        ]
        .spacing(7)
        .into(),
        _ => return None,
    };
    Some(content)
}

pub(super) fn view(state: &State) -> Element<'_, Message> {
    if state.level_map_prompt {
        return map_chooser(state);
    }
    if state.level_viewport_fullscreen {
        return level_workspace::fullscreen_view(state);
    }
    let tools: Element<'_, Message> = if state.level_project.is_some() && !state.level_read_only {
        tool_bar(state).spacing(5).into()
    } else {
        space().width(Fill).into()
    };
    let menus = container(
        row![
            text("LEVEL EDITOR").size(13),
            space().width(Length::Fixed(16.0)),
            menu_button("File", "LevelFile"),
            menu_button("Edit", "LevelEdit"),
            menu_button("Window", "LevelWindow"),
            menu_button("Tools", "LevelTools"),
            menu_button("Build", "LevelBuild"),
            menu_button("Select", "LevelSelectMenu"),
            menu_button("Actor", "LevelActor"),
            menu_button("Help", "LevelHelp"),
            design::button("Choose Map…").on_press(Message::LevelShowMapChooser),
            space().width(Length::Fixed(8.0)),
            tools,
        ]
        .spacing(2)
        .align_y(iced::Alignment::Center),
    )
    .padding([5, 9])
    .width(Fill)
    .style(|theme| container::Style {
        background: Some(Background::Color(design::sidebar(theme))),
        border: Border {
            color: design::border(theme),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    });

    let status = row![
        text("Scene").size(11).style(design::muted),
        text(format!(
            "{} scene objects",
            state.level_scene.as_ref().map_or(0, |s| s.objects.len())
        ))
        .size(11),
        text(format!("{} selected", state.level_selection.indices.len())).size(11),
        text(format!("{} diagnostics", state.level_diagnostics.len())).size(11),
        text(if state.level_read_only {
            "Source: read only"
        } else {
            tool_hint(state)
        })
        .size(11)
        .style(design::muted),
        space().width(Fill),
        // Edits report success and refusals here; without it a refused edit
        // looked like the button did nothing.
        text(if state.status.is_empty() {
            "Original PAKs preserved"
        } else {
            state.status.as_str()
        })
        .size(11)
        .style(design::muted),
    ]
    .spacing(14)
    .align_y(iced::Alignment::Center);

    let base = column![
        menus,
        container(level_workspace::view(state))
            .padding(8)
            .width(Fill)
            .height(Fill),
        container(status)
            .padding([5, 12])
            .width(Fill)
            .style(|theme| container::Style {
                background: Some(Background::Color(design::sidebar(theme))),
                border: Border {
                    color: design::border(theme),
                    width: 1.0,
                    radius: 0.0.into()
                },
                ..Default::default()
            }),
    ]
    .spacing(0)
    .width(Fill)
    .height(Fill);

    let left = match state.library_menu {
        Some("LevelFile") => 150.0,
        Some("LevelEdit") => 199.0,
        Some("LevelWindow") => 249.0,
        Some("LevelTools") => 318.0,
        Some("LevelBuild") => 374.0,
        Some("LevelSelectMenu") => 428.0,
        Some("LevelActor") => 487.0,
        Some("LevelHelp") => 537.0,
        _ => 0.0,
    };
    menu_popup::overlay(base, menu_content(state), left, 39.0, 290.0)
}
