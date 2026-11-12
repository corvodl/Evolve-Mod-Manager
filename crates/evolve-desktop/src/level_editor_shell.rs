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
/// Brush radius presets (metres) shown as chips in the Brush popover.
const RADIUS_PRESETS: [(&str, f32); 4] = [("S", 2.0), ("M", 5.0), ("L", 12.5), ("XL", 30.0)];
/// Popover ids for the Brush settings and the shortcut sheet.
pub(super) const BRUSH_POPOVER: &str = "level-brush";
pub(super) const KEYS_POPOVER: &str = "level-keys";

fn popover_toggle(id: &'static str, open: bool) -> Message {
    Message::Ui(crate::ui_kit::UiMsg::Popover((!open).then_some(id)))
}

/// The tools of an editable project, shown in the top bar: icon toggles for
/// the tool and its mode, a Brush popover button and the save controls.
fn tool_bar(state: &State) -> iced::widget::Row<'_, Message> {
    use crate::icons;
    use crate::terrain_brush::{LevelTool, SculptMode};
    use crate::ui_kit::{IconToggle, icon_button, icon_segmented};
    let tools = [
        (icons::cursor(), "Select", LevelTool::Select),
        (icons::mountain(), "Sculpt", LevelTool::Sculpt),
        (icons::brush(), "Paint", LevelTool::Paint),
        (icons::tree(), "Vegetation", LevelTool::Vegetation),
    ];
    let selected = tools
        .iter()
        .position(|(_, _, tool)| *tool == state.level_tool)
        .unwrap_or(0);
    let mut bar = row![icon_segmented(
        &state.ui,
        "level-tool",
        tools
            .into_iter()
            .map(|(icon, label, tool)| IconToggle {
                icon,
                label,
                message: Message::LevelSetTool(tool),
            })
            .collect(),
        selected,
    )]
    .spacing(6)
    .align_y(iced::Alignment::Center);
    let brush = state.terrain_brush;
    match state.level_tool {
        LevelTool::Select => {
            // The gizmo the selection shows, as Blender's Move/Rotate/Scale.
            use crate::level_transform::Mode;
            let modes = [
                (icons::move_tool(), "Move", Mode::Move),
                (icons::rotate(), "Rotate", Mode::Rotate),
                (icons::scale(), "Scale", Mode::Scale),
            ];
            let selected = modes
                .iter()
                .position(|(_, _, mode)| *mode == state.level_gizmo)
                .unwrap_or(0);
            bar = bar.push(icon_segmented(
                &state.ui,
                "level-gizmo",
                modes
                    .into_iter()
                    .map(|(icon, label, mode)| IconToggle {
                        icon,
                        label,
                        message: Message::LevelGizmoMode(mode),
                    })
                    .collect(),
                selected,
            ));
        }
        LevelTool::Sculpt => {
            let mut modes = vec![
                (icons::raise(), "Raise", SculptMode::Raise),
                (icons::flatten(), "Flatten", SculptMode::Flatten),
                (icons::smooth(), "Smooth", SculptMode::Smooth),
            ];
            // Legacy-ported maps' terrain.dat has holes to turn into ground.
            if state
                .terrain_session
                .as_ref()
                .is_some_and(|session| session.can_create())
            {
                modes.push((icons::create_ground(), "Create ground", SculptMode::Create));
            }
            let selected = modes
                .iter()
                .position(|(_, _, mode)| *mode == brush.mode)
                .unwrap_or(0);
            bar = bar.push(icon_segmented(
                &state.ui,
                "level-sculpt",
                modes
                    .into_iter()
                    .map(|(icon, label, mode)| IconToggle {
                        icon,
                        label,
                        message: Message::LevelSculptMode(mode),
                    })
                    .collect(),
                selected,
            ));
        }
        LevelTool::Paint => {
            // The material is chosen in the strip below.
            let name = state
                .level_surface_palette
                .as_ref()
                .and_then(|palette| palette.iter().find(|item| item.id == brush.surface))
                .map_or_else(
                    || format!("Surface {}", brush.surface),
                    |item| item.name.clone(),
                );
            bar = bar.push(
                design::tooltip(
                    text(name).size(design::LABEL),
                    format!("Surface ID {}", brush.surface),
                    None,
                    iced::widget::tooltip::Position::Bottom,
                ),
            );
        }
        LevelTool::Vegetation => {}
    }
    if state.level_tool != LevelTool::Select {
        let open = state.ui.popover == Some(BRUSH_POPOVER);
        bar = bar.push(
            design::button(
                row![
                    text(format!("Brush {:.1} m", brush.radius)).size(design::LABEL),
                    icons::glyph(icons::chevron_down(), 13.0),
                ]
                .spacing(5)
                .align_y(iced::Alignment::Center),
            )
            .padding([6, 10])
            .style(move |theme, status| design::rail(theme, status, open))
            .on_press(popover_toggle(BRUSH_POPOVER, open)),
        );
    }
    bar = bar.push(space().width(Fill));
    if matches!(state.level_tool, LevelTool::Select | LevelTool::Vegetation) {
        // The map's unsaved changes: live in the view, written by Save Map.
        let draft = state.level_draft.as_ref();
        let pending = draft.is_some_and(crate::level_draft::Draft::changed);
        let count = draft.map_or(0, |draft| draft.undo.len());
        if pending {
            bar = bar.push(crate::ui_kit::badge(
                crate::ui_kit::BadgeKind::Modified,
                format!("{count} unsaved"),
            ));
        }
        return bar
            .push(icon_button(
                icons::undo(),
                "Undo",
                draft
                    .is_some_and(|d| !d.undo.is_empty())
                    .then_some(Message::LevelDraftUndo),
                Some("Nothing to undo"),
            ))
            .push(icon_button(
                icons::redo(),
                "Redo",
                draft
                    .is_some_and(|d| !d.redo.is_empty())
                    .then_some(Message::LevelDraftRedo),
                Some("Nothing to redo"),
            ))
            .push(icon_button(
                icons::discard(),
                "Discard changes",
                pending.then_some(Message::LevelDiscardMap),
                Some("No unsaved changes"),
            ))
            .push(
                design::button("Save Map")
                    .style(design::primary)
                    .on_press_maybe((pending && !state.busy).then_some(Message::LevelSaveMap)),
            );
    }
    let session = state.terrain_session.as_ref();
    let dirty = session.is_some_and(|s| s.is_dirty());
    bar.push(icon_button(
        icons::undo(),
        "Undo",
        session
            .is_some_and(|s| s.can_undo())
            .then_some(Message::LevelBrushUndo),
        Some("Nothing to undo"),
    ))
    .push(icon_button(
        icons::redo(),
        "Redo",
        session
            .is_some_and(|s| s.can_redo())
            .then_some(Message::LevelBrushRedo),
        Some("Nothing to redo"),
    ))
    .push(icon_button(
        icons::discard(),
        "Discard changes",
        dirty.then_some(Message::LevelBrushDiscard),
        Some("No unsaved terrain changes"),
    ))
    .push(
        design::button("Save Terrain")
            .style(design::primary)
            .on_press_maybe((dirty && !state.busy).then_some(Message::LevelBrushSave)),
    )
}

/// The Brush popover: sliders paired with number fields, radius presets and
/// the tool's own settings.
fn brush_panel(state: &State) -> Option<Element<'_, Message>> {
    use crate::forms::{NumberSpec, bound_number};
    use crate::terrain_brush::LevelTool;
    use iced::widget::slider;
    if state.ui.popover != Some(BRUSH_POPOVER) || state.level_tool == LevelTool::Select {
        return None;
    }
    let brush = state.terrain_brush;
    let modifiers = state.keyboard_modifiers;
    let line = |label: &'static str, control: Element<'static, Message>, field: Element<'static, Message>| {
        row![
            text(label).size(design::LABEL).width(Length::Fixed(78.0)),
            container(control).width(Length::Fixed(170.0)),
            container(field).width(Length::Fixed(150.0)),
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center)
    };
    let mut presets = row![text("Size").size(design::LABEL).width(Length::Fixed(78.0))].spacing(4);
    for (label, radius) in RADIUS_PRESETS {
        let active = (brush.radius - radius).abs() < 0.05;
        presets = presets.push(
            design::button(text(label).size(design::LABEL))
                .padding([4, 10])
                .style(move |theme, status| design::rail(theme, status, active))
                .on_press(Message::LevelBrushRadius(radius)),
        );
    }
    presets = presets.push(
        text("[ and ] shrink or grow the brush")
            .size(design::CAPTION)
            .style(design::muted),
    );
    let mut panel = column![
        line(
            "Radius (m)",
            slider(0.5..=100.0, brush.radius, Message::LevelBrushRadius)
                .step(0.5_f32)
                .into(),
            bound_number(
                &state.forms,
                modifiers,
                "brush.radius",
                f64::from(brush.radius),
                NumberSpec::new(0.5, 100.0, 0.5, 1),
                |v| Message::LevelBrushRadius(v as f32),
            ),
        ),
        presets,
    ]
    .spacing(8);
    match state.level_tool {
        LevelTool::Sculpt => {
            panel = panel
                .push(line(
                    "Strength %",
                    slider(0.01..=1.0, brush.strength, Message::LevelBrushStrength)
                        .step(0.01_f32)
                        .into(),
                    bound_number(
                        &state.forms,
                        modifiers,
                        "brush.strength",
                        f64::from(brush.strength) * 100.0,
                        NumberSpec::new(1.0, 100.0, 1.0, 0),
                        |v| Message::LevelBrushStrength(v as f32 / 100.0),
                    ),
                ))
                .push(line(
                    "Hardness %",
                    slider(0.0..=0.95, brush.hardness, Message::LevelBrushHardness)
                        .step(0.05_f32)
                        .into(),
                    bound_number(
                        &state.forms,
                        modifiers,
                        "brush.hardness",
                        f64::from(brush.hardness) * 100.0,
                        NumberSpec::new(0.0, 95.0, 5.0, 0),
                        |v| Message::LevelBrushHardness(v as f32 / 100.0),
                    ),
                ));
        }
        LevelTool::Vegetation => {
            panel = panel
                .push(line(
                    "Density",
                    slider(1.0..=50.0, state.vegetation_density, Message::LevelVegetationDensity)
                        .step(1.0_f32)
                        .into(),
                    bound_number(
                        &state.forms,
                        modifiers,
                        "brush.density",
                        f64::from(state.vegetation_density),
                        NumberSpec::new(1.0, 50.0, 1.0, 0),
                        |v| Message::LevelVegetationDensity(v as f32),
                    ),
                ))
                .push(line(
                    "Size ± %",
                    slider(0.0..=0.8, state.vegetation_jitter, Message::LevelVegetationJitter)
                        .step(0.05_f32)
                        .into(),
                    bound_number(
                        &state.forms,
                        modifiers,
                        "brush.jitter",
                        f64::from(state.vegetation_jitter) * 100.0,
                        NumberSpec::new(0.0, 80.0, 5.0, 0),
                        |v| Message::LevelVegetationJitter(v as f32 / 100.0),
                    ),
                ));
        }
        LevelTool::Paint => {
            // Unnamed slots can still be painted by number.
            panel = panel.push(line(
                "Advanced ID",
                text("Surface slots without a name in the strip")
                    .size(design::CAPTION)
                    .style(design::muted)
                    .into(),
                bound_number(
                    &state.forms,
                    modifiers,
                    "brush.surface",
                    f64::from(brush.surface),
                    NumberSpec::new(0.0, 255.0, 1.0, 0),
                    |v| Message::LevelPickSurface(v.round() as u8),
                ),
            ));
        }
        LevelTool::Select => {}
    }
    Some(
        container(panel)
            .padding([10, 14])
            .width(Fill)
            .style(|theme| container::Style {
                background: Some(Background::Color(design::surface(theme))),
                border: Border {
                    color: design::border(theme),
                    width: 1.0,
                    radius: 0.0.into(),
                },
                ..Default::default()
            })
            .into(),
    )
}

/// Every level editor shortcut, grouped by tool, read from the shared
/// shortcut table so the sheet matches the real key handling.
fn shortcut_sheet(state: &State) -> Option<Element<'_, Message>> {
    if state.ui.popover != Some(KEYS_POPOVER) {
        return None;
    }
    let mut groups = row![].spacing(28);
    for (group, actions) in crate::chrome::LEVEL_SHORTCUTS {
        let mut list = column![text(*group).size(design::LABEL).style(design::accent)].spacing(6);
        for action in *actions {
            if let Some(keys) = crate::chrome::shortcut_for(action) {
                list = list.push(
                    row![
                        text(*action).size(design::LABEL).width(Length::Fixed(150.0)),
                        design::key_hint(keys),
                    ]
                    .spacing(8)
                    .align_y(iced::Alignment::Center),
                );
            }
        }
        groups = groups.push(list);
    }
    Some(
        container(scrollable(groups).direction(scrollable::Direction::Horizontal(
            scrollable::Scrollbar::new(),
        )))
        .padding([12, 14])
        .width(Fill)
        .style(|theme| container::Style {
            background: Some(Background::Color(design::surface(theme))),
            border: Border {
                color: design::border(theme),
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into(),
    )
}

/// Keys for the active tool, drawn as key caps for the status bar.
fn tool_hints(state: &State) -> Element<'_, Message> {
    use crate::terrain_brush::LevelTool;
    let actions: &[(&str, &str)] = match state.level_tool {
        LevelTool::Select => &[
            ("Select object", "select"),
            ("Move", "move"),
            ("Rotate", "rotate"),
            ("Delete", "delete"),
            ("Duplicate", "duplicate"),
            ("Undo", "undo"),
            ("Save map", "save"),
        ],
        LevelTool::Sculpt => &[
            ("Brush stroke", "raise"),
            ("Lower terrain", "lower"),
            ("Smaller brush", "smaller"),
            ("Larger brush", "larger"),
            ("Look around", "look"),
        ],
        LevelTool::Paint => &[
            ("Brush stroke", "paint"),
            ("Smaller brush", "smaller"),
            ("Larger brush", "larger"),
            ("Look around", "look"),
        ],
        LevelTool::Vegetation => &[
            ("Brush stroke", "scatter"),
            ("Erase vegetation", "erase"),
            ("Undo", "undo"),
            ("Save map", "save"),
        ],
    };
    let mut hints = row![].spacing(10).align_y(iced::Alignment::Center);
    for (action, caption) in actions {
        if let Some(keys) = crate::chrome::shortcut_for(action) {
            hints = hints.push(
                row![
                    design::key_hint(keys),
                    text(*caption).size(design::CAPTION).style(design::muted),
                ]
                .spacing(4)
                .align_y(iced::Alignment::Center),
            );
        }
    }
    let open = state.ui.popover == Some(KEYS_POPOVER);
    hints
        .push(design::tooltip(
            iced::widget::button(crate::icons::glyph(crate::icons::help(), 15.0))
                .padding(3)
                .style(move |theme, status| design::rail(theme, status, open))
                .on_press(popover_toggle(KEYS_POPOVER, open)),
            "All level editor shortcuts",
            None,
            iced::widget::tooltip::Position::Top,
        ))
        .into()
}

/// Number keys pick tools (1–4) and sculpt modes (Shift+1–4); [ and ]
/// resize the brush. Letters stay free for G/R/S and the RMB camera.
pub(super) fn tool_key(state: &State, key: char) -> Option<Message> {
    use crate::terrain_brush::{LevelTool, SculptMode};
    if state.level_project.is_none() || state.level_read_only {
        return None;
    }
    let shifted = match key {
        '!' => Some(1),
        '@' => Some(2),
        '#' => Some(3),
        '$' => Some(4),
        '1'..='4' if state.keyboard_modifiers.shift() => key.to_digit(10),
        _ => None,
    };
    if let Some(index) = shifted {
        if state.level_tool != LevelTool::Sculpt {
            return None;
        }
        let mode = match index {
            1 => SculptMode::Raise,
            2 => SculptMode::Flatten,
            3 => SculptMode::Smooth,
            _ => SculptMode::Create,
        };
        return Some(Message::LevelSculptMode(mode));
    }
    let radius = state.terrain_brush.radius;
    Some(match key {
        '1' => Message::LevelSetTool(LevelTool::Select),
        '2' => Message::LevelSetTool(LevelTool::Sculpt),
        '3' => Message::LevelSetTool(LevelTool::Paint),
        '4' => Message::LevelSetTool(LevelTool::Vegetation),
        '[' => Message::LevelBrushRadius((radius / 1.25).max(0.5)),
        ']' => Message::LevelBrushRadius((radius * 1.25).min(100.0)),
        _ => return None,
    })
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
            item(
                "Save map",
                (crate::draft_pending(state) && !state.busy).then_some(Message::LevelSaveMap)
            ),
            item("Show Activity / import log", Some(Message::ToggleActivity)),
        ]
        .spacing(2)
        .into(),
        "LevelEdit" => column![
            item(
                "Undo",
                state
                    .level_draft
                    .as_ref()
                    .is_some_and(|d| !d.undo.is_empty())
                    .then_some(Message::LevelDraftUndo)
            ),
            item(
                "Redo",
                state
                    .level_draft
                    .as_ref()
                    .is_some_and(|d| !d.redo.is_empty())
                    .then_some(Message::LevelDraftRedo)
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
                "Perspective view",
                scene.then_some(Message::ViewerCameraPreset("Isometric"))
            ),
            item("Top view", scene.then_some(Message::ViewerCameraPreset("Top"))),
            item(
                "Front view",
                scene.then_some(Message::ViewerCameraPreset("Front"))
            ),
            item(
                if state.level_show_overview {
                    "Hide 2D overview"
                } else {
                    "Show 2D overview"
                },
                Some(Message::LevelToggleOverview)
            ),
            item(
                "Focus selection",
                selected.then_some(Message::LevelCameraFocus)
            ),
            item("Reset view", scene.then_some(Message::LevelCameraReset)),
            item(
                "Fullscreen viewport",
                Some(Message::ToggleLevelViewportFullscreen)
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
        "LevelTools" => {
            use crate::terrain_brush::LevelTool;
            if state.level_project.is_some() && !state.level_read_only {
                column![
                    item("Select", Some(Message::LevelSetTool(LevelTool::Select))),
                    item("Sculpt", Some(Message::LevelSetTool(LevelTool::Sculpt))),
                    item("Paint", Some(Message::LevelSetTool(LevelTool::Paint))),
                    item(
                        "Vegetation",
                        Some(Message::LevelSetTool(LevelTool::Vegetation))
                    ),
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
            item(
                "All level editor shortcuts",
                Some(Message::Ui(crate::ui_kit::UiMsg::Popover(Some(KEYS_POPOVER))))
            ),
            text("Hold RMB and drag to look around; WASD and Q/E fly while it is held.")
                .size(design::LABEL),
            text("Wheel · adjust field of view").size(design::LABEL),
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
    let tools: Element<'_, Message> = if state.level_project.is_some() && !state.level_read_only {
        tool_bar(state).spacing(5).into()
    } else {
        space().width(Fill).into()
    };
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
        text("Scene").size(design::CAPTION).style(design::muted),
        text(format!(
            "{} scene objects",
            state.level_scene.as_ref().map_or(0, |s| s.objects.len())
        ))
        .size(11),
        text(format!("{} selected", state.level_selection.indices.len())).size(11),
        text(format!("{} diagnostics", state.level_diagnostics.len())).size(11),
        if state.level_read_only {
            Element::from(text("Source: read only").size(design::CAPTION).style(design::muted))
        } else {
            tool_hints(state)
        },
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

    let palette: Element<'_, Message> = crate::level_palette::strip(state)
        .unwrap_or_else(|| space().height(Length::Fixed(0.0)).into());
    let popover: Element<'_, Message> = brush_panel(state)
        .or_else(|| shortcut_sheet(state))
        .unwrap_or_else(|| space().height(Length::Fixed(0.0)).into());
    let base = column![
        menus,
        popover,
        palette,
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
