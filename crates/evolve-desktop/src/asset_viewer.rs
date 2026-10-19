//! Separate Asset Library and Modding preview window. Matches the private
//! manager's viewport/explorer split; Asset Library originals are read-only.
//! Heavy decoding and indexing are run by the main application's worker tasks.
use super::{MeshViewport, Message, State, TexturedScene};
use iced::widget::{
    button, canvas, column, container, row, scrollable, shader, slider, text, text_input,
};
use iced::{Background, Border, Color, Element, Fill, Length};

const SHELL: Color = Color::from_rgb(0.136, 0.145, 0.159);
const PANEL: Color = Color::from_rgb(0.170, 0.180, 0.195);
const PREVIEW: Color = Color::from_rgb(0.275, 0.285, 0.299);
const RULE: Color = Color::from_rgb(0.245, 0.255, 0.280);
const MUTED: Color = Color::from_rgb(0.77, 0.79, 0.85);

fn panel<'a>(child: impl Into<Element<'a, Message>>) -> iced::widget::Container<'a, Message> {
    container(child).style(|_| container::Style {
        background: Some(Background::Color(PANEL)),
        border: Border {
            color: RULE,
            width: 1.0,
            radius: 5.0.into(),
        },
        ..container::Style::default()
    })
}

fn menu_action<'a>(label: &'static str) -> iced::widget::Button<'a, Message> {
    button(text(label).size(13))
        .style(button::text)
        .padding([6, 11])
}

fn strip(state: &State) -> Element<'_, Message> {
    let menu = row![
        menu_action("File").on_press(Message::ViewerMenu("File")),
        menu_action("Edit").on_press(Message::ViewerMenu("Edit")),
        menu_action("View").on_press(Message::ViewerMenu("View")),
        menu_action("Model").on_press(Message::ViewerMenu("Model")),
        menu_action("Textures").on_press(Message::ViewerMenu("Textures")),
        menu_action("Animation").on_press(Message::ViewerMenu("Animation")),
        menu_action("Tools").on_press(Message::ViewerMenu("Tools")),
        menu_action("Help").on_press(Message::ViewerMenu("Help")),
    ]
    .spacing(3);

    let options: Element<'_, Message> = match state.viewer_menu {
        Some("File") => row![
            button("Inspect original PAK").on_press(Message::StartPakInspection),
            button("Export selected DDS → PNG").on_press(Message::StartPngExport),
            button("Close menu").on_press(Message::ViewerMenu("")),
        ].spacing(8).into(),
        Some("Edit") => text("Asset Library originals are read-only. Edit a separate Modding workspace copy.").size(13).into(),
        Some("View") => row![
            button("Toggle File Explorer").on_press(Message::ViewerToggleExplorer),
            button("Reset camera").on_press(Message::ViewerResetCamera),
        ].spacing(8).into(),
        Some("Model") => row![
            button("Inspect selected model").on_press(Message::ViewerPreviewSelected),
            button("Inspect CDF").on_press(Message::StartCdfInspection),
            button("Export rigged FBX").on_press(Message::StartRigFbx),
        ].spacing(8).into(),
        Some("Textures") => row![
            button("Resolve .mtl").on_press(Message::StartMaterialResolve),
            button("Load diffuse").on_press(Message::StartDiffuseLoad),
            button("Load normal").on_press(Message::StartNormalLoad),
            button("Load emissive").on_press(Message::StartEmissiveLoad),
        ].spacing(8).into(),
        Some("Animation") => row![
            button("Open CAF").on_press(Message::StartCafLoad),
            button(if state.playing { "Pause" } else { "Play" }).on_press(Message::ToggleMotion),
        ].spacing(8).into(),
        Some("Tools") => row![
            button("Index extracted files").on_press(Message::StartScan),
            button("Inspect selected asset").on_press(Message::ViewerPreviewSelected),
        ].spacing(8).into(),
        Some("Help") => text("Read-only preview. Use the file explorer to select extracted assets; original PAK members must be extracted before preview.").size(13).into(),
        _ => column![].into(),
    };
    column![menu, options].spacing(5).into()
}

fn viewport(state: &State) -> Element<'_, Message> {
    let header = row![
        text("User Perspective").size(12),
        text(" • ").size(12),
        text(if state.model.is_some() {
            "GPU Model Preview"
        } else {
            "No model loaded"
        })
        .size(12),
    ]
    .spacing(2);
    let main: Element<'_, Message> = if let Some(mesh) = &state.model {
        if mesh.uv0.is_some() {
            shader(TexturedScene {
                mesh: mesh.clone(),
                diffuse: state.diffuse.clone(),
                normal_map: state.normal_map.clone(),
                emissive: state.emissive_map.clone(),
                yaw: state.yaw,
                pitch: state.pitch,
                zoom: state.zoom.max(0.2),
            })
            .width(Fill)
            .height(Fill)
            .into()
        } else {
            canvas(MeshViewport {
                mesh: mesh.clone(),
                yaw: state.yaw,
                pitch: state.pitch,
                zoom: state.zoom.max(0.2),
            })
            .width(Fill)
            .height(Fill)
            .into()
        }
    } else {
        container(
            column![
            text("3D / ASSET PREVIEW").size(19),
            text("Choose a model, CDF, or DDS from extracted files.").size(14).color(MUTED),
            text("This workspace does not invent missing character pieces, textures or animations.")
                .size(12).color(MUTED),
            button("Inspect selected file").on_press(Message::ViewerPreviewSelected)
        ]
            .spacing(13)
            .align_x(iced::Center),
        )
        .center(Fill)
        .style(|_| container::Style {
            background: Some(Background::Color(PREVIEW)),
            ..container::Style::default()
        })
        .into()
    };

    let toolbar = row![
        button("Orbit").on_press(Message::ViewerResetCamera),
        text("Yaw").size(12),
        slider(-180.0..=180.0, state.yaw, Message::YawChanged).width(Length::FillPortion(2)),
        text("Pitch").size(12),
        slider(-85.0..=85.0, state.pitch, Message::PitchChanged).width(Length::FillPortion(2)),
        text("Zoom").size(12),
        slider(0.2..=4.0, state.zoom.max(0.2), Message::ZoomChanged).width(Length::FillPortion(1)),
    ]
    .spacing(8);

    let duration = state
        .native_caf
        .as_ref()
        .map(|clip| clip.duration_seconds())
        .or_else(|| {
            state.glb_motion.as_ref().and_then(|motion| {
                motion
                    .list_clips()
                    .get(state.selected_motion_clip)
                    .map(|clip| clip.duration)
            })
        })
        .unwrap_or(0.03)
        .max(0.03);
    let timeline = row![
        text("Timeline").size(12),
        button("◀").on_press(Message::MotionScrub(0.0)),
        button(if state.playing { "Ⅱ" } else { "▶" }).on_press(Message::ToggleMotion),
        slider(
            0.0..=duration,
            state.motion_time.clamp(0.0, duration),
            Message::MotionScrub
        )
        .width(Fill),
        text(format!("{:.2} / {:.2} s", state.motion_time, duration)).size(12),
    ]
    .spacing(8)
    .align_y(iced::Center);

    column![
        header,
        toolbar,
        main,
        panel(container(timeline).padding(5)).height(Length::Fixed(43.0))
    ]
    .spacing(8)
    .height(Fill)
    .width(Length::FillPortion(2))
    .into()
}

fn explorer(state: &State) -> Element<'_, Message> {
    let mut rows =
        column![row![text("File / Folder").width(Fill), text("Size").width(90)].spacing(8)]
            .spacing(2);
    let filter = state.viewer_search.to_ascii_lowercase();
    let selected = state.selected;
    let total: usize;
    if let Some(inv) = &state.inventory {
        total = inv.entries.len();
        for (index, entry) in inv
            .entries
            .iter()
            .enumerate()
            .filter(|(_, file)| file.relative_path.to_ascii_lowercase().contains(&filter))
            .take(350)
        {
            let label = if selected == Some(index) {
                format!("▸ {}", entry.relative_path)
            } else {
                entry.relative_path.clone()
            };
            rows = rows.push(
                button(
                    row![
                        text(label).size(12).width(Fill),
                        text(format!("{} KiB", entry.byte_size.div_ceil(1024)))
                            .size(12)
                            .width(90),
                    ]
                    .spacing(7),
                )
                .width(Fill)
                .padding(5)
                .style(if selected == Some(index) {
                    button::primary
                } else {
                    button::text
                })
                .on_press(Message::Select(index)),
            );
        }
    } else if let Some(pak) = &state.pak {
        total = pak.entries.len();
        for (index, entry) in pak
            .entries
            .iter()
            .enumerate()
            .filter(|(_, file)| file.path.to_ascii_lowercase().contains(&filter))
            .take(350)
        {
            rows = rows.push(
                button(
                    row![
                        text(entry.path.as_str()).size(12).width(Fill),
                        text(format!("{} KiB", entry.size.div_ceil(1024)))
                            .size(12)
                            .width(90),
                    ]
                    .spacing(7),
                )
                .width(Fill)
                .padding(5)
                .style(button::text)
                .on_press(Message::Select(index)),
            );
        }
    } else {
        total = 0;
        rows = rows.push(
            text("No archive or extracted folder indexed.")
                .size(13)
                .color(MUTED),
        );
    }
    let source = state
        .pak
        .as_ref()
        .map(|pak| pak.source_file.as_str())
        .or_else(|| state.inventory.as_ref().map(|inv| inv.root.as_str()))
        .unwrap_or("No source selected");

    panel(
        container(
            column![
                row![
                    text("FILE EXPLORER").size(16),
                    text(format!("{total} files")).size(12)
                ]
                .spacing(16),
                text("PAK / PROJECT").size(12),
                text(source).size(12),
                text_input("Search files and folders...", &state.viewer_search)
                    .on_input(Message::ViewerSearchChanged),
                row![
                    button("Preview selected").on_press(Message::ViewerPreviewSelected),
                    button("Index folder").on_press(Message::StartScan),
                ]
                .spacing(8),
                scrollable(rows).height(Fill),
                text("Read-only explorer  •  350 matching files per view")
                    .size(11)
                    .color(MUTED),
            ]
            .spacing(9),
        )
        .padding(12),
    )
    .height(Fill)
    .width(Length::FillPortion(1))
    .into()
}

pub(super) fn view(state: &State) -> Element<'_, Message> {
    let readonly = if state.viewer_from_library {
        "Read-only"
    } else {
        "Modding preview"
    };
    let heading = row![
        text("Asset / Model Viewer").size(16),
        text(readonly).size(12).color(MUTED),
        text(if state.selected.is_some() {
            " • 1 selected"
        } else {
            " • 0 selected"
        })
        .size(12),
        button("Toggle Explorer").on_press(Message::ViewerToggleExplorer),
    ]
    .spacing(14)
    .align_y(iced::Center);
    let panes: Element<'_, Message> = if state.viewer_explorer_visible {
        row![viewport(state), explorer(state)]
            .spacing(10)
            .height(Fill)
            .into()
    } else {
        viewport(state)
    };
    let footer = row![
        text(if state.busy {
            "Loading source data..."
        } else {
            state.status.as_str()
        })
        .size(12)
        .width(Fill),
        text("Asset Library originals protected  •  Preview only")
            .size(11)
            .color(MUTED),
    ]
    .spacing(12);

    container(
        column![strip(state), heading, panes, footer]
            .spacing(7)
            .padding(7)
            .height(Fill),
    )
    .height(Fill)
    .width(Fill)
    .style(|_| container::Style {
        background: Some(Background::Color(SHELL)),
        ..container::Style::default()
    })
    .into()
}
