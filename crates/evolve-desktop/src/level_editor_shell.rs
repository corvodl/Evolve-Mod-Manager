//! Native-window scene-editor chrome inspired by familiar professional
//! 3D authoring applications. The actual world view remains the existing
//! Iced/wgpu widget; this only supplies task-specific menus and panels.
use crate::{Message, State, design, level_workspace, menu_popup};
use iced::widget::{button, column, container, row, space, text};
use iced::{Background, Border, Element, Fill, Length};

fn menu_button(name: &'static str, id: &'static str) -> iced::widget::Button<'static, Message> {
    button(text(name).size(12))
        .padding([5, 10])
        .style(design::text_button)
        .on_press(Message::LibraryMenu(id))
}
fn command<'a>(caption: &'static str, msg: Message) -> iced::widget::Button<'a, Message> {
    button(text(caption).size(12))
        .style(design::text_button)
        .padding([6, 12])
        .width(Fill)
        .on_press(Message::MenuCommand(Box::new(msg)))
}
fn menu_content(state: &State) -> Option<Element<'_, Message>> {
    let content: Element<'_, Message> = match state.library_menu? {
        "LevelFile" => column![
            command("Open as Modding Project", Message::LibraryMakeModProject),
            command("Open selected Modding project", Message::LevelOpenSelectedModProject),
            command("Save edited project layer", Message::LevelSaveProjectChanges),
            command("Export edited level copy", Message::LevelExportEdits),
            command("Show Activity / import log", Message::ToggleActivity),
        ].spacing(2).into(),
        "LevelEdit" => column![
            command("Undo transform", Message::LevelUndoTransform),
            command("Redo transform", Message::LevelRedoTransform),
            command("Apply transform to preview", Message::LevelApplyTransform),
        ].spacing(2).into(),
        "LevelWindow" => column![
            command("Show / hide 2D overview", Message::LevelToggleOverview),
            command("Activity and importer diagnostics", Message::ToggleActivity),
        ].spacing(2).into(),
        "LevelTools" => column![
            command("Focus selected object (F)", Message::LevelCameraFocus),
            command("Reset camera", Message::LevelCameraReset),
            command("Top view", Message::ViewerCameraPreset("Top")),
            command("Front view", Message::ViewerCameraPreset("Front")),
            command("Perspective view", Message::ViewerCameraPreset("Isometric")),
        ].spacing(2).into(),
        "LevelBuild" => column![
            command("Export original scene GLB", Message::StartLevelGlbExport),
            command("Export original scene FBX", Message::StartLevelFbxExport),
            text("Exports operate on an editable project copy.").size(11).style(design::muted),
        ].spacing(2).into(),
        "LevelSelectMenu" => column![
            command("Focus selected placement (F)", Message::LevelCameraFocus),
            command("Show 2D placement overview", Message::LevelToggleOverview),
        ].spacing(2).into(),
        "LevelActor" => column![
            command("Apply position / rotation / scale", Message::LevelApplyTransform),
            text("Select an actor in the Outliner or the 3D viewport.").size(11).style(design::muted),
        ].spacing(2).into(),
        "LevelHelp" => column![
            text("WASD · fly relative to camera").size(12),
            text("Q / E · descend / ascend").size(12),
            text("Hold RMB and drag · look around").size(12),
            text("Wheel · adjust field of view").size(12),
            text("F · focus selection").size(12),
            text("File → Open as Modding Project to edit").size(12),
        ].spacing(7).into(),
        _ => return None,
    };
    Some(content)
}

pub(super) fn view(state: &State) -> Element<'_, Message> {
    let menus = container(row![
        text("EVOLVE  /  LEVEL EDITOR").size(12),
        space().width(Length::Fixed(16.0)),
        menu_button("File", "LevelFile"),
        menu_button("Edit", "LevelEdit"),
        menu_button("Window", "LevelWindow"),
        menu_button("Tools", "LevelTools"),
        menu_button("Build", "LevelBuild"),
        menu_button("Select", "LevelSelectMenu"),
        menu_button("Actor", "LevelActor"),
        menu_button("Help", "LevelHelp"),
    ].spacing(2).align_y(iced::Alignment::Center))
    .padding([5, 9])
    .width(Fill)
    .style(|theme| container::Style {
        background: Some(Background::Color(design::sidebar(theme))),
        border: Border { color: design::border(theme), width: 1.0, radius: 0.0.into() },
        ..Default::default()
    });

    let toolbar = container(row![
        button(text("◉  Select").size(12)).on_press(Message::LevelToggleOverview),
        button(text("⊕  Focus").size(12)).on_press(Message::LevelCameraFocus),
        button(text("⌖  Reset view").size(12)).on_press(Message::LevelCameraReset),
        text("│").size(15).style(design::muted),
        button(text("Perspective").size(12)).on_press(Message::ViewerCameraPreset("Isometric")),
        button(text("Top").size(12)).on_press(Message::ViewerCameraPreset("Top")),
        button(text("Front").size(12)).on_press(Message::ViewerCameraPreset("Front")),
        text("│").size(15).style(design::muted),
        text("Imported Scene / Desert Canyon").size(13),
        space().width(Fill),
        text(if state.level_read_only { "SOURCE: READ ONLY" } else { "EDITABLE PROJECT COPY" })
            .size(11).style(design::muted),
    ].spacing(5).align_y(iced::Alignment::Center))
    .padding([7, 11]).width(Fill)
    .style(|theme| container::Style {
        background: Some(Background::Color(design::surface(theme))),
        border: Border { color: design::border(theme), width: 1.0, radius: 0.0.into() },
        ..Default::default()
    });

    let status = row![
        text("◉  LEVEL EDITOR").size(11).style(design::muted),
        text(format!("{} scene objects", state.level_scene.as_ref().map_or(0, |s| s.objects.len()))).size(11),
        text(format!("{} selected", state.level_selection.indices.len())).size(11),
        text(format!("{} diagnostics", state.level_diagnostics.len())).size(11),
        space().width(Fill),
        text("Original PAKs preserved  |  RMB Look · WASD Fly · Q/E Up/Down · F Focus").size(11).style(design::muted),
    ].spacing(14).align_y(iced::Alignment::Center);

    let base = column![
        menus,
        toolbar,
        container(level_workspace::view(state)).padding([7, 10]).width(Fill).height(Fill),
        container(status).padding([5, 12]).width(Fill)
            .style(|theme| container::Style {
                background: Some(Background::Color(design::sidebar(theme))),
                border: Border { color: design::border(theme), width: 1.0, radius: 0.0.into() },
                ..Default::default()
            }),
    ].spacing(0).width(Fill).height(Fill);

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
