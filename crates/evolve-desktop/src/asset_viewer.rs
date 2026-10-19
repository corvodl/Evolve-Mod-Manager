//! Separate Asset Library and Modding preview window. Matches the private
//! manager's viewport/explorer split; Asset Library originals are read-only.
//! Heavy decoding and indexing are run by the main application's worker tasks.
use super::{MeshViewport, Message, State, TexturedScene, viewer_inspector};
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
        Some("Model") => column![
            row![
                button("Inspect selected model").on_press(Message::ViewerPreviewSelected),
                button("Inspect CDF").on_press(Message::StartCdfInspection),
                button("Preview CDF assembly (static)").on_press(Message::ViewerPreviewStaticCdf),
                button("Export verified rigged FBX").on_press(Message::StartRigFbx),
            ].spacing(8),
            text("Blender FBX export requires a verified compiled skeleton and original mesh weights. Source files are never overwritten.").size(12).color(MUTED),
            row![
                text_input("NEW FBX destination (absolute path)", &state.output)
                    .on_input(Message::OutputChanged).width(Fill),
                text_input("Blender executable", &state.blender_path)
                    .on_input(Message::BlenderChanged).width(Fill),
                text_input("Matching skeleton (CDF auto-detected where verified)", &state.skeleton_path)
                    .on_input(Message::SkeletonChanged).width(Fill),
            ].spacing(8),
            row![
                button(if state.export_include_animation {
                    "Verified CAF Actions: on"
                } else {
                    "Verified CAF Actions: off"
                }).on_press(Message::ViewerToggleExportAnimations),
                text_input("Exact CAF path (optional; export opt-in)", &state.caf_path)
                    .on_input(Message::CafInputChanged).width(Fill),
            ].spacing(8),
        ].spacing(7).into(),
        Some("Textures") => row![
            button("Resolve .mtl").on_press(Message::StartMaterialResolve),
            button("Load diffuse").on_press(Message::StartDiffuseLoad),
            button("Load normal").on_press(Message::StartNormalLoad),
            button("Load emissive").on_press(Message::StartEmissiveLoad),
        ].spacing(8).into(),
        Some("Animation") => {
            let mut clips = column![
                row![
                    button("Open CAF").on_press(Message::StartCafLoad),
                    button(if state.playing { "Pause" } else { "Play" }).on_press(Message::ToggleMotion),
                    text(format!("{} character-declared animations", state.auto_clips.len())).size(12),
                ].spacing(8),
            ].spacing(5);
            clips = clips.push(
                text_input("Filter character animations…", &state.auto_clip_filter)
                    .on_input(Message::ViewerAutoClipFilter)
            );
            let needle = state.auto_clip_filter.to_ascii_lowercase();
            for (index, clip) in state.auto_clips.iter().enumerate()
                .filter(|(_, clip)| needle.is_empty()
                    || clip.label.to_ascii_lowercase().contains(&needle)
                    || clip.kind.to_ascii_lowercase().contains(&needle))
                .take(80)
            {
                let label = format!("{} · {}{}", clip.label, clip.kind.to_ascii_uppercase(), if clip.source.is_none() { " (not extracted)" } else { "" });
                clips = clips.push(
                    button(text(label).size(12))
                        .on_press(Message::ViewerAutoClip(index))
                        .style(button::text)
                );
            }
            scrollable(clips).height(Length::Fixed(250.0)).into()
        },
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
        text(state.auto_resource_note.as_str())
            .size(11)
            .color(MUTED),
    ]
    .spacing(2);
    let main: Element<'_, Message> = if let Some(mesh) = &state.model {
        if mesh.uv0.is_some() && !state.viewer_wireframe {
            shader(TexturedScene {
                mesh: mesh.clone(),
                diffuse: state.diffuse.clone(),
                normal_map: state.normal_map.clone(),
                emissive: state.emissive_map.clone(),
                yaw: state.yaw,
                pitch: state.pitch,
                zoom: state.zoom.max(0.2),
                light_strength: state.light_strength,
                pan_x: state.pan_x,
                pan_y: state.pan_y,
                shift: state.keyboard_modifiers.shift(),
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
                pan_x: state.pan_x,
                pan_y: state.pan_y,
                shift: state.keyboard_modifiers.shift(),
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
        button("Reset Camera").on_press(Message::ViewerResetCamera),
        text("Drag to orbit · Shift+drag/right-drag to pan · Wheel zoom").size(11),
        text("Light").size(12),
        slider(
            0.1..=2.0,
            state.light_strength.max(0.1),
            Message::LightStrengthChanged
        )
        .width(Length::Fixed(90.0)),
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
        button(if state.playback_loop {
            "Loop on"
        } else {
            "Loop off"
        })
        .on_press(Message::PlaybackLoopChanged),
        button("◀").on_press(Message::MotionScrub(0.0)),
        button(if state.playing { "Ⅱ" } else { "▶" }).on_press(Message::ToggleMotion),
        slider(
            0.0..=duration,
            state.motion_time.clamp(0.0, duration),
            Message::MotionScrub
        )
        .width(Fill),
        text(format!("{:.2} / {:.2} s", state.motion_time, duration)).size(12),
        text(format!("{:.1}×", state.playback_speed)).size(12),
        slider(
            0.1..=3.0,
            state.playback_speed.max(0.1),
            Message::PlaybackSpeedChanged
        )
        .width(Length::Fixed(80.0)),
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
    let total = if let Some(inv) = &state.inventory {
        inv.entries.len()
    } else if let Some(signed) = &state.signed_pak {
        signed.entries.len()
    } else {
        state.pak.as_ref().map_or(0, |pak| pak.entries.len())
    };
    for entry in state.viewer_rows.iter().cloned() {
        let indent = "  ".repeat(entry.depth.min(20));
        let title = if entry.directory {
            format!(
                "{indent}{} 📁 {}",
                if state.viewer_collapsed.contains(&entry.path) && state.viewer_search.is_empty() {
                    "▸"
                } else {
                    "▾"
                },
                entry.name
            )
        } else {
            let label = if entry.name.to_ascii_lowercase().ends_with(".cdf") {
                format!("◇ {} · Character assembly", entry.name)
            } else {
                entry.name.clone()
            };
            format!("{indent}  {label}")
        };
        let action = if entry.directory {
            Message::ViewerToggleFolder(entry.path)
        } else if entry.path.starts_with("@assemblies/") {
            Message::ViewerOpenStaticCdf(entry.index.expect("CDF assembly row has source index"))
        } else {
            Message::ViewerSelectFile(entry.index.expect("file row has source index"))
        };
        rows = rows.push(
            button(
                row![
                    text(title).size(12).width(Fill),
                    text(if entry.directory {
                        String::new()
                    } else {
                        format!("{} KiB", entry.bytes.div_ceil(1024))
                    })
                    .size(12)
                    .width(90),
                ]
                .spacing(7),
            )
            .width(Fill)
            .padding(5)
            .style(
                if entry.index.is_some()
                    && entry
                        .index
                        .is_some_and(|i| state.file_selection.indices.contains(&i))
                {
                    button::primary
                } else {
                    button::text
                },
            )
            .on_press(action),
        );
    }
    if total == 0 {
        rows = rows.push(
            text("No archive or extracted folder indexed.")
                .size(13)
                .color(MUTED),
        );
    }
    let source = state
        .signed_pak
        .as_ref()
        .map(|pak| pak.archive_name.as_str())
        .or_else(|| state.pak.as_ref().map(|pak| pak.source_file.as_str()))
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
                if state.viewer_from_library {
                    Element::<Message>::from(
                        text("Protected read-only source. To edit, open a separate Modding project.")
                            .size(11).color(MUTED)
                    )
                } else {
                    column![
                        text_input("Existing local file to import over selected project asset...", &state.editor_replace_source)
                            .on_input(Message::EditorReplacementChanged),
                        button("Import over selected project asset (backup)")
                            .on_press_maybe((!state.busy).then_some(Message::EditorImportRawMember)),
                        button("Restore selected original (keep edited backup)")
                            .on_press_maybe((!state.busy).then_some(Message::EditorRestoreRawMember)),
                    ].spacing(5).into()
                },
                scrollable(rows).height(Fill),
                row![
                    button("◀ Previous")
                        .on_press_maybe((state.viewer_page > 0).then_some(Message::ViewerPreviousPage)),
                    text(format!("Page {} · up to 350 visible entries", state.viewer_page + 1))
                        .size(11)
                        .color(MUTED),
                    button("Next ▶")
                        .on_press_maybe(state.viewer_more_rows.then_some(Message::ViewerNextPage)),
                ].spacing(8),
                text("Shift: range  •  Ctrl: toggle  •  CDF: assembly definition")
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
        text(format!(
            " • {} selected",
            state.file_selection.indices.len()
        ))
        .size(12),
        button("Toggle Explorer").on_press(Message::ViewerToggleExplorer),
        button("Save layout").on_press(Message::ViewerSaveLayout),
        button("Detach Inspector")
            .on_press(Message::OpenInspectorWindow)
            .style(button::secondary),
    ]
    .spacing(14)
    .align_y(iced::Center);
    let ratio = state.viewer_split.clamp(0.25, 0.8);
    let view_weight = (ratio * 100.0).round() as u16;
    let list_weight = (100.0 - ratio * 100.0).round() as u16;
    let panes: Element<'_, Message> = if state.viewer_explorer_visible {
        row![
            container(viewport(state)).width(Length::FillPortion(view_weight.max(1))),
            container(explorer(state)).width(Length::FillPortion(list_weight.max(1)))
        ]
        .spacing(10)
        .height(Fill)
        .into()
    } else {
        viewport(state)
    };
    let split_controls: Element<'_, Message> = if state.viewer_explorer_visible {
        row![
            text("Viewport").size(12),
            slider(25.0..=80.0, ratio * 100.0, |value| {
                Message::ViewerSplitChanged(value / 100.0)
            })
            .width(Length::Fixed(200.0)),
            text("Explorer").size(12),
            button(if state.viewer_show_inspector {
                "Hide Inspector"
            } else {
                "Inspector"
            })
            .on_press(Message::ViewerToggleInspector),
        ]
        .spacing(10)
        .align_y(iced::Center)
        .into()
    } else {
        button("Show Explorer")
            .on_press(Message::ViewerToggleExplorer)
            .into()
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
        column![
            strip(state),
            heading,
            split_controls,
            panes,
            viewer_inspector::view(state),
            footer
        ]
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
