//! Native-window scene-editor chrome inspired by familiar professional
//! 3D authoring applications. The actual world view remains the existing
//! Iced/wgpu widget; this only supplies task-specific menus and panels.
use crate::{Message, State, design, level_workspace, menu_popup};
use iced::widget::{column, container, row, scrollable, space, text};
use iced::{Background, Border, Element, Fill, Length};

/// A menubar title; hovering it while another menu is open switches menus.
fn menu_button<'a>(state: &State, name: &'static str, id: &'static str) -> Element<'a, Message> {
    crate::chrome::menu_title(
        crate::chrome::MenuBar::Level,
        name,
        id,
        state.library_menu,
        Message::LibraryMenu(id),
        12.0,
        [5, 10],
    )
}
/// The tool bar of an editable project: Select, Sculpt and Paint, with the
/// active brush's settings and the unsaved-terrain controls beside them.
fn tool_bar(state: &State) -> Element<'_, Message> {
    use crate::terrain_brush::{LevelTool, SculptMode};
    use iced::widget::slider;
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
        text("TOOLS").size(design::CAPTION).style(design::accent),
        tool("Select", LevelTool::Select),
        tool("Sculpt", LevelTool::Sculpt),
        tool("Paint", LevelTool::Paint),
    ]
    .spacing(5)
    .align_y(iced::Alignment::Center);
    let brush = state.terrain_brush;
    if state.level_tool != LevelTool::Select {
        bar = bar.push(text("│").size(design::HEADING).style(design::muted));
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
            bar = bar.push(text("Surface").size(design::CAPTION)).push(
                container(crate::forms::number_field(
                    &state.forms,
                    state.keyboard_modifiers,
                    "terrain.surface",
                    &state.terrain_brush_surface,
                    crate::forms::NumberSpec::new(0.0, 255.0, 1.0, 0),
                    Message::LevelBrushSurface,
                ))
                .width(Length::Fixed(120.0)),
            );
        }
        let setting = |label: &'static str,
                       range: std::ops::RangeInclusive<f32>,
                       value: f32,
                       step: f32,
                       shown: String,
                       on_change: fn(f32) -> Message| {
            row![
                text(label).size(design::CAPTION),
                slider(range, value, on_change)
                    .step(step)
                    .width(Length::Fixed(90.0)),
                text(shown).size(design::CAPTION).width(Length::Fixed(44.0)),
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
    bar = bar
        .push(space().width(Fill))
        .push(
            text(match state.level_tool {
                LevelTool::Select => "Click select · Del delete · Ctrl+C/Ctrl+V copy, paste at mouse · Shift+D duplicate · RMB look",
                LevelTool::Sculpt => "LMB brush · Ctrl+LMB lower · RMB look · WASD fly",
                LevelTool::Paint => "LMB paint · RMB look · WASD fly",
            })
            .size(design::CAPTION)
            .style(design::muted),
        )
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
        ));
    container(bar)
        .padding([6, 11])
        .width(Fill)
        .style(|theme| container::Style {
            background: Some(Background::Color(design::sidebar(theme))),
            border: Border {
                color: design::border(theme),
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
}

/// Camera and view actions as a dock that magnifies icons near the cursor.
fn viewport_dock(state: &State) -> Element<'_, Message> {
    use crate::icons;
    use crate::ui_kit::{DockItem, dock};
    let item = |icon, label, message, active| DockItem {
        icon,
        label,
        message: Some(message),
        active,
    };
    dock(
        &state.ui,
        "level",
        vec![
            item(
                icons::view_iso(),
                "Perspective",
                Message::ViewerCameraPreset("Isometric"),
                false,
            ),
            item(
                icons::view_top(),
                "Top",
                Message::ViewerCameraPreset("Top"),
                false,
            ),
            item(
                icons::view_front(),
                "Front",
                Message::ViewerCameraPreset("Front"),
                false,
            ),
            item(
                icons::overview(),
                if state.level_show_overview {
                    "Hide 2D overview"
                } else {
                    "2D overview"
                },
                Message::LevelToggleOverview,
                state.level_show_overview,
            ),
            item(
                icons::focus(),
                "Focus selection",
                Message::LevelCameraFocus,
                false,
            ),
            item(
                icons::reset(),
                "Reset view",
                Message::LevelCameraReset,
                false,
            ),
            item(
                icons::fullscreen(),
                "Fullscreen viewport",
                Message::ToggleLevelViewportFullscreen,
                false,
            ),
        ],
        state.preferences.reduce_motion,
    )
}

/// Unsaved • Saving… • Saved 12 s ago for the editable level project.
fn level_save_status(state: &State) -> Element<'_, Message> {
    if state.level_project.is_none() || state.level_read_only {
        return space().width(0).into();
    }
    let dirty = state.level_dirty_count != 0
        || state
            .terrain_session
            .as_ref()
            .is_some_and(|session| session.is_dirty());
    let (saving, saved_at) = state.ui.save_info("level");
    crate::ui_kit::save_status(dirty, saving, saved_at)
}

fn map_chooser(state: &State) -> Element<'_, Message> {
    let mut choices = column![
        text("Choose a map to open").size(24),
        text("Select an imported map archive, then a verified map inside it. Original PAKs remain unchanged.")
            .size(design::LABEL).style(design::muted),
    ].spacing(12);

    if state.busy {
        choices = choices.push(text("Discovering or loading maps…").size(design::BODY));
    }
    if let Some((project, maps)) = &state.level_project_choices {
        choices =
            choices.push(text(format!("Maps in editable project {}", project.label)).size(17));
        for (index, map) in maps.iter().enumerate() {
            choices = choices.push(
                design::button(text(&map.name).size(design::BODY))
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
                design::button(text(&map.name).size(design::BODY))
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
                    design::button(text(&project.label).size(design::BODY))
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
                    design::button(text(&archive.name).size(design::BODY))
                        .on_press_maybe((!state.busy).then_some(Message::LevelChooseArchive(index)))
                        .width(Fill),
                );
            }
        }
        if !found {
            choices = choices.push(
                text("No imported map archives found. Import a level PAK in Asset Library first.")
                    .size(design::LABEL)
                    .style(design::muted),
            );
        }
    }
    if state.level_scene.is_some() || state.library_map.is_some() {
        choices = choices
            .push(design::button("Return to open scene").on_press(Message::LevelDismissMapChooser));
    }
    choices = choices.push(
        text(&state.status)
            .size(design::CAPTION)
            .style(design::muted),
    );
    container(scrollable(choices).width(Length::Fixed(600.0)).height(Fill))
        .center_x(Fill)
        .padding(32)
        .width(Fill)
        .height(Fill)
        .into()
}
fn menu_content(state: &State) -> Option<Element<'_, Message>> {
    use crate::chrome::menu_item as item;
    let editable = !state.level_read_only && state.level_project.is_some();
    let selected = !state.level_selection.indices.is_empty();
    let scene = state.level_scene.is_some();
    let content: Element<'_, Message> = match state.library_menu? {
        "LevelFile" => column![
            item(
                "Open as Modding Project",
                Some(Message::LibraryMakeModProject)
            ),
            item(
                "Open selected Modding project",
                Some(Message::LevelOpenSelectedModProject)
            ),
            item(
                "Save edited project layer",
                editable.then_some(Message::LevelSaveProjectChanges)
            ),
            item(
                "Export edited level copy",
                editable.then_some(Message::LevelExportEdits)
            ),
            item("Show Activity / import log", Some(Message::ToggleActivity)),
        ]
        .spacing(2)
        .into(),
        "LevelEdit" => column![
            item(
                "Undo transform",
                (!state.level_read_only && !state.level_undo.is_empty())
                    .then_some(Message::LevelUndoTransform)
            ),
            item(
                "Redo transform",
                (!state.level_read_only && !state.level_redo.is_empty())
                    .then_some(Message::LevelRedoTransform)
            ),
            item(
                "Save selected transform to project",
                (editable && selected).then_some(Message::LevelApplyTransform)
            ),
            text("Committed source changes have verified editor backups.")
                .size(design::CAPTION)
                .style(design::muted),
        ]
        .spacing(2)
        .into(),
        "LevelWindow" => column![
            item(
                "Show / hide 2D overview",
                Some(Message::LevelToggleOverview)
            ),
            item(
                "Activity and importer diagnostics",
                Some(Message::ToggleActivity)
            ),
            item(
                "Command palette",
                Some(Message::Ui(crate::ui_kit::UiMsg::Palette(
                    crate::command_palette::Msg::Toggle
                )))
            ),
        ]
        .spacing(2)
        .into(),
        "LevelTools" => column![
            item(
                "Focus selected object",
                selected.then_some(Message::LevelCameraFocus)
            ),
            item("Reset camera", scene.then_some(Message::LevelCameraReset)),
            item(
                "Top view",
                scene.then_some(Message::ViewerCameraPreset("Top"))
            ),
            item(
                "Front view",
                scene.then_some(Message::ViewerCameraPreset("Front"))
            ),
            item(
                "Perspective view",
                scene.then_some(Message::ViewerCameraPreset("Isometric"))
            ),
        ]
        .spacing(2)
        .into(),
        "LevelBuild" => column![
            item(
                "Export original scene GLB",
                (scene && !state.busy).then_some(Message::StartLevelGlbExport)
            ),
            item(
                "Export original scene FBX",
                (scene && !state.busy).then_some(Message::StartLevelFbxExport)
            ),
            text("Exports create new external files; original PAKs remain read-only.")
                .size(design::CAPTION)
                .style(design::muted),
        ]
        .spacing(2)
        .into(),
        "LevelSelectMenu" => column![
            item(
                "Focus selected placement",
                selected.then_some(Message::LevelCameraFocus)
            ),
            item(
                "Show 2D placement overview",
                Some(Message::LevelToggleOverview)
            ),
        ]
        .spacing(2)
        .into(),
        "LevelActor" => column![
            item(
                "Apply position / rotation / scale",
                (editable && selected).then_some(Message::LevelApplyTransform)
            ),
            text("Select an actor in the Outliner or the 3D viewport.")
                .size(design::CAPTION)
                .style(design::muted),
        ]
        .spacing(2)
        .into(),
        "LevelHelp" => column![
            text("WASD · fly relative to camera").size(design::LABEL),
            text("Q / E · descend / ascend").size(design::LABEL),
            text("Hold RMB and drag · look around").size(design::LABEL),
            text("Wheel · adjust field of view").size(design::LABEL),
            text("F · focus selection").size(design::LABEL),
            text("File → Open as Modding Project to edit").size(design::LABEL),
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
    let menus = container(
        row![
            text("LEVEL EDITOR").size(design::BODY),
            space().width(Length::Fixed(16.0)),
            menu_button(state, "File", "LevelFile"),
            menu_button(state, "Edit", "LevelEdit"),
            menu_button(state, "Window", "LevelWindow"),
            menu_button(state, "Tools", "LevelTools"),
            menu_button(state, "Build", "LevelBuild"),
            menu_button(state, "Select", "LevelSelectMenu"),
            menu_button(state, "Actor", "LevelActor"),
            menu_button(state, "Help", "LevelHelp"),
            design::button("Choose Map…").on_press(Message::LevelShowMapChooser),
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

    let toolbar = container(
        row![
            text("VIEWPORT").size(design::CAPTION).style(design::accent),
            viewport_dock(state),
            space().width(Fill),
            level_save_status(state),
            text(if state.level_read_only {
                "SOURCE: READ ONLY"
            } else {
                "EDITABLE PROJECT COPY"
            })
            .size(design::CAPTION)
            .style(design::muted),
        ]
        .spacing(5)
        .align_y(iced::Alignment::Center),
    )
    .padding([7, 11])
    .width(Fill)
    .style(|theme| container::Style {
        background: Some(Background::Color(design::surface(theme))),
        border: Border {
            color: design::border(theme),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    });

    let status = row![
        text("Scene").size(design::CAPTION).style(design::muted),
        text(format!(
            "{} scene objects",
            state.level_scene.as_ref().map_or(0, |s| s.objects.len())
        ))
        .size(design::CAPTION),
        text(format!("{} selected", state.level_selection.indices.len())).size(design::CAPTION),
        text(format!("{} diagnostics", state.level_diagnostics.len())).size(design::CAPTION),
        space().width(Fill),
        // Edits report success and refusals here; without it a refused edit
        // looked like the button did nothing.
        text(if state.status.is_empty() {
            "Original PAKs preserved"
        } else {
            state.status.as_str()
        })
        .size(design::CAPTION)
        .style(design::muted),
    ]
    .spacing(14)
    .align_y(iced::Alignment::Center);

    let tools: Element<'_, Message> = if state.level_project.is_some() && !state.level_read_only {
        tool_bar(state)
    } else {
        space().height(Length::Fixed(0.0)).into()
    };
    let base = column![
        menus,
        toolbar,
        tools,
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
