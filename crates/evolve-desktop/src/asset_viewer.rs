//! Separate Asset Library and Modding preview window. Matches the private
//! manager's viewport/explorer split; Asset Library originals are read-only.
//! Heavy decoding and indexing are run by the main application's worker tasks.
use super::{MeshViewport, Message, State, TexturedScene, viewer_inspector};
use crate::design;
use iced::widget::{
    canvas, column, container, mouse_area, row, scrollable, shader, slider, stack, text, tooltip,
};
use iced::{Background, Border, Element, Fill, Length};

fn panel<'a>(child: impl Into<Element<'a, Message>>) -> iced::widget::Container<'a, Message> {
    container(child).style(|theme| container::Style {
        background: Some(Background::Color(design::surface(theme))),
        border: Border {
            color: design::border(theme),
            width: 1.0,
            radius: 12.0.into(),
        },
        ..container::Style::default()
    })
}

fn menu_action<'a>(label: &'static str) -> iced::widget::Button<'a, Message> {
    design::button(text(label).size(13))
        .style(design::text_button)
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
            design::button("Inspect original PAK").on_press(Message::StartPakInspection),
            design::button("Export selected DDS → PNG").on_press(Message::StartPngExport),
            design::button("Close menu").on_press(Message::ViewerMenu("")),
        ].spacing(8).into(),
        Some("Edit") => text("Asset Library originals are read-only. Edit a separate Modding workspace copy.").size(13).into(),
        Some("View") => row![
            design::button("Toggle File Explorer").on_press(Message::ViewerToggleExplorer),
            design::button("Reset camera").on_press(Message::ViewerResetCamera),
        ].spacing(8).into(),
        Some("Model") => column![
            row![
                design::button("Retry selected preview").on_press(Message::ViewerPreviewSelected),
                design::button("Inspect CDF").on_press(Message::StartCdfInspection),
                design::button("Preview CDF assembly (static)").on_press(Message::ViewerPreviewStaticCdf),
                design::button("Export verified rigged FBX").on_press(Message::StartRigFbx),
                design::button("Flat CDF parts FBX (experimental)")
                    .on_press(Message::ViewerExportFlatCdf),
                design::button("Unified CDF rigged FBX (experimental)")
                    .on_press(Message::ViewerExportUnifiedCdf),
            ].spacing(8),
            text("Single-click an indexed source asset to preview it automatically. The Retry action is optional; no second Inspect click is required.").size(12).style(design::muted),
            text("Blender FBX export requires a verified compiled skeleton and original mesh weights. Source files are never overwritten.").size(12).style(design::muted),
            text("Flat CDF export requires a NEW DIRECTORY. Unified CDF export requires a NEW .fbx file. Both verify original UVs and weights; unified output uses one shared armature, but complete per-part materials and skin animation remain unverified.")
                .size(12).style(design::muted),
            row![
                design::text_input("NEW FBX destination (absolute path)", &state.output)
                    .on_input(Message::OutputChanged).width(Fill),
                design::text_input("Blender executable", &state.blender_path)
                    .on_input(Message::BlenderChanged).width(Fill),
                design::text_input("Matching skeleton (CDF auto-detected where verified)", &state.skeleton_path)
                    .on_input(Message::SkeletonChanged).width(Fill),
            ].spacing(8),
            row![
                design::button(if state.export_include_animation {
                    "Verified CAF Actions: on"
                } else {
                    "Verified CAF Actions: off"
                }).on_press(Message::ViewerToggleExportAnimations),
                design::text_input("Exact CAF path (optional; export opt-in)", &state.caf_path)
                    .on_input(Message::CafInputChanged).width(Fill),
            ].spacing(8),
        ].spacing(7).into(),
        Some("Textures") => row![
            design::button("Resolve .mtl").on_press(Message::StartMaterialResolve),
            design::button("Load diffuse").on_press(Message::StartDiffuseLoad),
            design::button("Load normal").on_press(Message::StartNormalLoad),
            design::button("Load emissive").on_press(Message::StartEmissiveLoad),
        ].spacing(8).into(),
        Some("Animation") => {
            let mut clips = column![
                row![
                    design::button("Open CAF").on_press(Message::StartCafLoad),
                    design::button(if state.playing { "Pause" } else { "Play" }).on_press(Message::ToggleMotion),
                    text(format!("{} character-declared animations", state.auto_clips.len())).size(12),
                ].spacing(8),
            ].spacing(5);
            clips = clips.push(
                design::text_input("Filter character animations…", &state.auto_clip_filter)
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
                    design::button(text(label).size(12))
                        .on_press(Message::ViewerAutoClip(index))
                        .style(design::text_button)
                );
            }
            if !state.converted_glbs.is_empty() {
                clips = clips.push(
                    text(if state.converted_dba_cache.is_some() {
                        "VERIFIED TEMPORARY DBA TRACKS"
                    } else {
                        "VERIFIED CONVERTED MOTION FILES"
                    })
                    .size(12)
                    .style(design::muted),
                );
                for (index, source) in state.converted_glbs.iter().enumerate().take(64) {
                    let name = source.file_name().map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| format!("Track {}", index + 1));
                    clips = clips.push(design::button(text(name).size(12))
                        .on_press(Message::OpenConvertedGlb(index)).style(design::text_button));
                }
            }
            scrollable(clips).height(Length::Fixed(250.0)).into()
        },
        Some("Tools") => row![
            design::button("Index extracted files").on_press(Message::StartScan),
            design::button("Inspect selected asset").on_press(Message::ViewerPreviewSelected),
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
            .style(design::muted),
    ]
    .spacing(2);
    let main: Element<'_, Message> = if let Some(mesh) = &state.model {
        if !state.viewer_wireframe {
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
            text("Choose a model, CDF, or DDS from extracted files.").size(14).style(design::muted),
            text("This workspace does not invent missing character pieces, textures or animations.")
                .size(12).style(design::muted),
            design::button("Inspect selected file").on_press(Message::ViewerPreviewSelected)
        ]
            .spacing(13)
            .align_x(iced::Center),
        )
        .center(Fill)
        .style(|theme| container::Style {
            background: Some(Background::Color(design::field(theme))),
            ..container::Style::default()
        })
        .into()
    };

    let toolbar = row![
        design::button("Reset View").on_press(Message::ViewerResetCamera),
        text("Drag: orbit • Shift+drag/right-drag: pan • Wheel: zoom").size(11),
        design::button("−")
            .on_press(Message::ViewerZoomWheel(-2.0))
            .style(design::secondary),
        design::button("+")
            .on_press(Message::ViewerZoomWheel(2.0))
            .style(design::secondary),
        text("Studio light").size(11),
        slider(
            0.1..=2.0,
            state.light_strength.max(0.1),
            Message::LightStrengthChanged
        )
        .width(Length::Fixed(90.0)),
    ]
    .spacing(8)
    .align_y(iced::Center);

    // Native, perspective-aware signed XYZ gizmo from the private manager:
    // drag to orbit, click an axis to snap. Only the 112px compass covers
    // viewport input; the remaining preview stays fully interactive.
    let preview = stack![
        main,
        container(
            canvas(super::orientation_gizmo::OrientationGizmo {
                yaw: state.yaw,
                pitch: state.pitch,
            })
            .width(Length::Fixed(112.0))
            .height(Length::Fixed(112.0)),
        )
        .width(Fill)
        .height(Fill)
        .align_right(Fill)
        .align_top(Fill)
        .padding(8),
    ]
    .width(Fill)
    .height(Fill);

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
        design::button(if state.playback_loop {
            "Loop on"
        } else {
            "Loop off"
        })
        .on_press(Message::PlaybackLoopChanged),
        design::button("◀").on_press(Message::MotionScrub(0.0)),
        design::button(if state.playing { "Ⅱ" } else { "▶" }).on_press(Message::ToggleMotion),
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
        preview,
        panel(container(timeline).padding(5)).height(Length::Fixed(43.0))
    ]
    .spacing(8)
    .height(Fill)
    .width(Length::FillPortion(2))
    .into()
}

/// Keep the full archive location available on hover, but prevent very
/// long Windows paths from consuming the entire viewer's right-hand pane.
fn compact_source_name(source: &str) -> String {
    let mut parts = source.trim_end_matches(['\\', '/']).rsplit(['\\', '/']);
    let file = parts
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or("No source selected");
    let parent = parts.next().filter(|part| !part.is_empty());
    let short = match parent {
        Some(folder) if !folder.ends_with(':') => format!("{folder} / {file}"),
        _ => file.to_owned(),
    };
    const MAX_LABEL: usize = 100;
    if short.chars().count() <= MAX_LABEL {
        short
    } else {
        let keep = short.chars().rev().take(MAX_LABEL - 1).collect::<String>();
        format!("…{}", keep.chars().rev().collect::<String>())
    }
}

fn explorer_on_left(side: &str) -> bool {
    side.eq_ignore_ascii_case("left")
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
        let item = design::button(
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
                design::primary
            } else {
                design::text_button
            },
        )
        .on_press(action);
        let item: Element<'_, Message> = if let Some(index) = entry.index {
            mouse_area(item)
                .on_right_press(Message::ViewerContextFile(index))
                .into()
        } else {
            item.into()
        };
        rows = rows.push(item);
    }
    if total == 0 {
        rows = rows.push(
            text("No archive or extracted folder indexed.")
                .size(13)
                .style(design::muted),
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
                tooltip(
                    text(compact_source_name(source)).size(12),
                    text(source).size(11),
                    tooltip::Position::Bottom,
                ),
                design::text_input("Search files and folders...", &state.viewer_search)
                    .on_input(Message::ViewerSearchChanged),
                row![
                    design::button("Preview selected").on_press(Message::ViewerPreviewSelected),
                    design::button("Index folder").on_press(Message::StartScan),
                ]
                .spacing(8),
                if let Some(index) = state.viewer_context_index {
                    if let Some(entry) = state
                        .inventory
                        .as_ref()
                        .and_then(|inv| inv.entries.get(index))
                    {
                        let filename = entry.relative_path.as_str();
                        let is_cdf = filename.to_ascii_lowercase().ends_with(".cdf");
                        let is_dds = filename.to_ascii_lowercase().ends_with(".dds")
                            || filename.to_ascii_lowercase().ends_with(".dds.0");
                        let is_model = [".skin", ".skinm", ".chr", ".chrm", ".cgf", ".cdf"]
                            .iter()
                            .any(|extension| filename.to_ascii_lowercase().ends_with(extension));
                        panel(
                            column![
                                row![
                                    text("FILE ACTIONS").size(13),
                                    text(filename).size(11).style(design::muted).width(Fill),
                                    design::button("✕").on_press(Message::ViewerContextClose),
                                ]
                                .spacing(7),
                                row![
                                    design::button("Preview")
                                        .on_press(Message::ViewerPreviewSelected),
                                    design::button("Copy path")
                                        .on_press(Message::ViewerCopySelectedPath),
                                    design::button("Show in Explorer")
                                        .on_press(Message::ViewerRevealSelectedFile),
                                    design::button("Dependencies")
                                        .on_press(Message::StartDependencyInspection),
                                ]
                                .spacing(6),
                                row![
                                    design::button("Preview full CDF").on_press_maybe(
                                        is_cdf.then_some(Message::ViewerPreviewStaticCdf)
                                    ),
                                    design::button("Export DDS → PNG")
                                        .on_press_maybe(is_dds.then_some(Message::StartPngExport)),
                                    design::button("Rigged FBX").on_press_maybe(
                                        (is_model && !state.busy).then_some(Message::StartRigFbx)
                                    ),
                                    design::button("Restore original").on_press_maybe(
                                        (!state.viewer_from_library && !state.busy)
                                            .then_some(Message::EditorRestoreRawMember)
                                    ),
                                ]
                                .spacing(6),
                            ]
                            .spacing(6),
                        )
                        .into()
                    } else {
                        Element::<Message>::from(text("Selected file is no longer indexed."))
                    }
                } else {
                    Element::<Message>::from(
                        text("Right-click a file for actions.")
                            .size(11)
                            .style(design::muted),
                    )
                },
                if state.viewer_from_library {
                    Element::<Message>::from(
                        text(
                            "Protected read-only source. To edit, open a separate Modding project.",
                        )
                        .size(11)
                        .style(design::muted),
                    )
                } else {
                    column![
                        design::text_input(
                            "Existing local file to import over selected project asset...",
                            &state.editor_replace_source
                        )
                        .on_input(Message::EditorReplacementChanged),
                        design::button("Import over selected project asset (backup)")
                            .on_press_maybe(
                                (!state.busy).then_some(Message::EditorImportRawMember)
                            ),
                        design::button("Restore selected original (keep edited backup)")
                            .on_press_maybe(
                                (!state.busy).then_some(Message::EditorRestoreRawMember)
                            ),
                    ]
                    .spacing(5)
                    .into()
                },
                scrollable(rows).height(Fill),
                row![
                    design::button("◀ Previous").on_press_maybe(
                        (state.viewer_page > 0).then_some(Message::ViewerPreviousPage)
                    ),
                    text(format!(
                        "Page {} · up to 350 visible entries",
                        state.viewer_page + 1
                    ))
                    .size(11)
                    .style(design::muted),
                    design::button("Next ▶")
                        .on_press_maybe(state.viewer_more_rows.then_some(Message::ViewerNextPage)),
                ]
                .spacing(8),
                text("Shift: range  •  Ctrl: toggle  •  CDF: assembly definition")
                    .size(11)
                    .style(design::muted),
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
        text(readonly).size(12).style(design::muted),
        text(format!(
            " • {} selected",
            state.file_selection.indices.len()
        ))
        .size(12),
        design::button("Toggle Explorer").on_press(Message::ViewerToggleExplorer),
        design::button("Save layout").on_press(Message::ViewerSaveLayout),
        design::button("Detach Inspector")
            .on_press(Message::OpenInspectorWindow)
            .style(design::secondary),
    ]
    .spacing(14)
    .align_y(iced::Center);
    let ratio = state.viewer_split.clamp(0.25, 0.8);
    let view_weight = (ratio * 100.0).round() as u16;
    let list_weight = (100.0 - ratio * 100.0).round() as u16;
    let panes: Element<'_, Message> = if state.viewer_explorer_visible {
        let preview = container(viewport(state)).width(Length::FillPortion(view_weight.max(1)));
        let files = container(explorer(state)).width(Length::FillPortion(list_weight.max(1)));
        let panes = if explorer_on_left(&state.preferences.file_explorer_side) {
            row![files, preview]
        } else {
            row![preview, files]
        };
        panes.spacing(10).height(Fill).into()
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
            design::button(if state.viewer_show_inspector {
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
        design::button("Show Explorer")
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
        text(if state.viewer_from_library {
            "Asset Library originals protected  •  Preview only"
        } else {
            "Modding project workspace  •  Original PAKs preserved"
        })
        .size(11)
        .style(design::muted),
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
    .style(|theme| container::Style {
        background: Some(Background::Color(theme.palette().background)),
        ..container::Style::default()
    })
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explorer_side_setting_affects_pane_order() {
        assert!(explorer_on_left("Left"));
        assert!(!explorer_on_left("Right"));
        assert!(!explorer_on_left(""));
    }

    #[test]
    fn long_windows_archive_paths_remain_readable() {
        let source = r"\\?\C:\Users\Example\AppData\Local\EvolveModManager\Data\AssetLibrary\characters_hunters_merc_tech_data.pak__1234567890abcdefabcd\files";
        assert_eq!(
            compact_source_name(source),
            "characters_hunters_merc_tech_data.pak__1234567890abcdefabcd / files"
        );
        assert!(compact_source_name(&"x".repeat(140)).chars().count() <= 100);
        assert_eq!(compact_source_name(""), "No source selected");
    }
}
