//! Separate Asset Library and Modding preview window. Matches the private
//! manager's viewport/explorer split; Asset Library originals are read-only.
//! Heavy decoding and indexing are run by the main application's worker tasks.
use super::{
    GfxToolsSection, MeshViewport, Message, State, TexturedScene, ViewerSidebarTab,
    viewer_inspector,
};
use crate::design;
use iced::widget::{
    canvas, column, container, image, mouse_area, progress_bar, row, scrollable, shader, slider,
    stack, svg, text, text_editor, text_input, tooltip,
};
use iced::{
    Background, Border, Color, Element, Event, Fill, Length, Point, Rectangle, Renderer, Theme,
    mouse,
};

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

fn strip(_state: &State) -> Element<'_, Message> {
    row![
        menu_action("File").on_press(Message::ViewerMenu("File")),
        menu_action("Edit").on_press(Message::ViewerMenu("Edit")),
        menu_action("View").on_press(Message::ViewerMenu("View")),
        menu_action("Model").on_press(Message::ViewerMenu("Model")),
        menu_action("Textures").on_press(Message::ViewerMenu("Textures")),
        menu_action("Animation").on_press(Message::ViewerMenu("Animation")),
        menu_action("Tools").on_press(Message::ViewerMenu("Tools")),
        menu_action("Help").on_press(Message::ViewerMenu("Help")),
    ]
    .spacing(3)
    .into()
}

fn dropdown(state: &State) -> Option<Element<'_, Message>> {
    let content: Element<'_, Message> = match state.viewer_menu? {
        "File" => column![
            design::button("Inspect original PAK").on_press(Message::MenuCommand(Box::new(Message::StartPakInspection))),
            design::button("Export…").on_press(Message::ExportOpen),
            design::button("Export selected textures / models…").on_press(Message::ExportOpen),
            design::button("Save viewer layout").on_press(Message::MenuCommand(Box::new(Message::ViewerSaveLayout))),
        ].spacing(3).into(),
        "Edit" => column![
            design::button("Open XML / Lua / TXT")
                .on_press(Message::MenuCommand(Box::new(Message::TextEditorOpen))),
            design::button("Save text changes")
                .on_press(Message::MenuCommand(Box::new(Message::TextEditorSave))),
            design::button("Edit Scaleform GFX movie…")
                .on_press(Message::MenuCommand(Box::new(Message::GfxOpenSelected))),
            design::button("Preferences…").on_press(Message::MenuCommand(Box::new(Message::OpenViewerPreferences))),
            text("Read-only library assets. Edit copies in Modding.")
                .size(11).style(design::muted),
        ].spacing(3).into(),
        "View" => column![
            design::button("Toggle Explorer").on_press(Message::MenuCommand(Box::new(Message::ViewerToggleExplorer))),
            design::button("Move Explorer").on_press(Message::MenuCommand(Box::new(Message::PreferenceExplorerSide))),
            design::button("Reset Camera").on_press(Message::MenuCommand(Box::new(Message::ViewerResetCamera))),
            design::button("Detach Inspector").on_press(Message::MenuCommand(Box::new(Message::OpenInspectorWindow))),
            design::button("Inspector").on_press(Message::MenuCommand(Box::new(Message::ViewerSidebarSelect(ViewerSidebarTab::Inspector)))),
        ].spacing(3).into(),
        "Model" => column![
            design::button("Retry selected preview").on_press(Message::MenuCommand(Box::new(Message::ViewerPreviewSelected))),
            design::button("Inspect CDF").on_press(Message::MenuCommand(Box::new(Message::StartCdfInspection))),
            design::button("Model Options").on_press(Message::MenuCommand(Box::new(
                Message::ViewerSidebarSelect(ViewerSidebarTab::Equipment)
            ))),
            design::button("Preview CDF assembly (static)").on_press(Message::MenuCommand(Box::new(Message::ViewerPreviewStaticCdf))),
            design::button("Export models / CDF…").on_press(Message::ExportOpen),
            text("Rigged FBX, embedded textures, original UV0 repair, and PNG dependencies are configured in File → Export.")
                .size(11).style(design::muted),
        ].spacing(5).into(),
        "Textures" => column![
            design::button("Resolve .mtl").on_press(Message::MenuCommand(Box::new(Message::StartMaterialResolve))),
            design::button("Load diffuse").on_press(Message::MenuCommand(Box::new(Message::StartDiffuseLoad))),
            design::button("Load normal").on_press(Message::MenuCommand(Box::new(Message::StartNormalLoad))),
            design::button("Load emissive").on_press(Message::MenuCommand(Box::new(Message::StartEmissiveLoad))),
        ].spacing(3).into(),
        "Animation" => design::button("Open Animation Browser")
            .on_press(Message::MenuCommand(Box::new(Message::ViewerSidebarAnimations(true)))).into(),
        "Tools" => column![
            design::button("Index extracted files").on_press(Message::MenuCommand(Box::new(Message::StartScan))),
            design::button("Inspect selected asset").on_press(Message::MenuCommand(Box::new(Message::ViewerPreviewSelected))),
            design::button("Dependency Graph").on_press(Message::MenuCommand(Box::new(Message::StartDependencyInspection))),
        ].spacing(3).into(),
        "Help" => text("Read-only preview. Select extracted assets in Files. Original PAK members must be extracted before preview.")
            .size(12).into(),
        _ => return None,
    };
    Some(content)
}

fn dropdown_x(name: Option<&str>) -> f32 {
    match name {
        Some("File") => 9.0,
        Some("Edit") => 64.0,
        Some("View") => 121.0,
        Some("Model") => 179.0,
        Some("Textures") => 247.0,
        Some("Animation") => 334.0,
        Some("Tools") => 429.0,
        Some("Help") => 484.0,
        _ => 9.0,
    }
}

fn animation_browser(state: &State) -> Element<'_, Message> {
    let content: Element<'_, Message> = {
        let mut clips = column![
            row![
                design::button("Open CAF").on_press(Message::StartCafLoad),
                design::button(if state.playing { "Pause" } else { "Play" })
                    .on_press(Message::ToggleMotion),
                text(format!(
                    "{} character-declared animations",
                    state.auto_clips.len()
                ))
                .size(12),
            ]
            .spacing(8),
        ]
        .spacing(5);
        if state.caf_layer.is_some() {
            clips = clips.push(
                row![
                    text(format!("Layer: {}", state.caf_layer_label)).size(12),
                    slider(
                        0.0..=1.0,
                        state.caf_layer_weight,
                        Message::ViewerLayerWeight
                    )
                    .width(160),
                    text(format!("{:.0}%", state.caf_layer_weight * 100.0)).size(12),
                    design::button("Remove layer").on_press(Message::ViewerClearLayer),
                ]
                .spacing(8),
            );
        }
        clips = clips.push(
            design::text_input("Filter character animations…", &state.auto_clip_filter)
                .on_input(Message::ViewerAutoClipFilter),
        );
        let needle = state.auto_clip_filter.to_ascii_lowercase();
        for (index, clip) in state
            .auto_clips
            .iter()
            .enumerate()
            .filter(|(_, clip)| {
                needle.is_empty()
                    || clip.label.to_ascii_lowercase().contains(&needle)
                    || clip.kind.to_ascii_lowercase().contains(&needle)
            })
            .take(80)
        {
            let label = format!(
                "{} · {}{}",
                clip.label,
                clip.kind.to_ascii_uppercase(),
                if clip.source.is_none() {
                    " (not extracted)"
                } else {
                    ""
                }
            );
            clips = clips.push(
                row![
                    design::button(text(label).size(12))
                        .on_press(Message::ViewerAutoClip(index))
                        .style(design::text_button)
                        .width(Fill),
                    design::button("Blend layer").on_press_maybe(
                        (!state.busy
                            && (state.skin_preview.is_some() || state.cdf_caf_preview.is_some())
                            && clip.kind.eq_ignore_ascii_case("caf")
                            && clip.source.is_some())
                        .then_some(Message::ViewerLayerClip(index))
                    ),
                ]
                .spacing(8),
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
                let name = source
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| format!("Track {}", index + 1));
                clips = clips.push(
                    design::button(text(name).size(12))
                        .on_press(Message::OpenConvertedGlb(index))
                        .style(design::text_button),
                );
            }
        }
        scrollable(clips).height(Fill).into()
    };
    panel(
        container(
            column![
                text("ANIMATION BROWSER").size(15),
                text("Only verified clips declared for the selected character are playable.")
                    .size(11)
                    .style(design::muted),
                content,
            ]
            .spacing(9),
        )
        .padding(12),
    )
    .width(Fill)
    .height(Fill)
    .into()
}

fn equipment_browser(state: &State) -> Element<'_, Message> {
    let mut entries = column![
        text("MODEL OPTIONS").size(15),
        text("Equipment changes affect the viewport and CDF FBX export. Stage 2 monsters also support Glow and damage blend. Original assets remain unchanged.")
            .size(11).style(design::muted),
    ].spacing(10);
    let monster_selected = state.equipment_source.as_ref().is_some_and(|path| {
        path.to_string_lossy()
            .replace('\\', "/")
            .to_ascii_lowercase()
            .contains("/monsters/")
    });
    if monster_selected && state.cdf_legacy_2012 {
        entries = entries.push(
            text(
                "2012 original monster: optional Stage 2 Glow and damage blend are not available.",
            )
            .size(11)
            .style(design::muted),
        );
    }
    if monster_selected && !state.cdf_legacy_2012 {
        entries = entries.push(
            design::button(if state.full_armor_glow {
                "☑ Glow"
            } else {
                "☐ Glow"
            })
            .on_press_maybe((!state.busy).then_some(Message::ViewerToggleMonsterGlow)),
        );
        entries = entries.push(
            text("Optional source-verified [name]_glow_diff overlay for Stage 2 monsters. Turning Glow off removes only this alternate overlay; original materials, diffuse and authored emission remain intact.")
                .size(11)
                .style(design::muted),
        );
        entries = entries.push(
            design::button(if state.damage_blend_enabled {
                "☑ Damage blend"
            } else {
                "☐ Damage blend"
            })
            .on_press_maybe((!state.busy).then_some(Message::ViewerToggleDamageBlend)),
        );
        if state.damage_blend_enabled {
            entries = entries.push(
                row![
                    text(format!("Damage intensity: {}%", state.damage_blend_percent)).size(11),
                    slider(
                        0..=100,
                        state.damage_blend_percent,
                        Message::ViewerDamageBlendChanged
                    )
                    .on_release(Message::ViewerDamageBlendCommit)
                    .width(Fill),
                ]
                .spacing(8),
            );
        }
        entries = entries.push(
            text("Approximates the original CryEngine BLENDLAYER using only this monster's verified Custom damage diffuse and Opacity damage-blend mask. Keeps geometry opaque and exports with Rigged + Embedded textures.")
                .size(11)
                .style(design::muted),
        );
    }
    if let Some(emet) = state.emet_options.as_ref() {
        entries = entries.push(text("E.M.E.T. · FACE / EQUIPMENT").size(13));
        if !emet.faces.is_empty() {
            entries = entries.push(
                text("Face · only one original CDF expression is rendered at a time.")
                    .size(11)
                    .style(design::muted),
            );
            let current = state
                .equipment_overrides
                .get(evolve_core::character_emet::FACE)
                .map(String::as_str)
                .unwrap_or_else(|| {
                    emet.faces
                        .first()
                        .map(String::as_str)
                        .unwrap_or(evolve_core::character_emet::NO_FACE)
                });
            let mut faces = column![].spacing(4);
            for face in &emet.faces {
                let active = face == current;
                faces = faces.push(
                    design::button(text(format!(
                        "{} {}",
                        if active { "✓" } else { "○" },
                        face.replace('_', " ")
                    )))
                    .style(if active {
                        design::primary
                    } else {
                        design::secondary
                    })
                    .on_press_maybe(
                        (!state.busy && !active).then_some(Message::ViewerEmetFace(face.clone())),
                    ),
                );
            }
            let none = current == evolve_core::character_emet::NO_FACE;
            faces = faces.push(
                design::button(if none {
                    "✓ No expression"
                } else {
                    "○ No expression"
                })
                .style(if none {
                    design::primary
                } else {
                    design::secondary
                })
                .on_press_maybe((!state.busy && !none).then_some(Message::ViewerEmetFace(
                    evolve_core::character_emet::NO_FACE.into(),
                ))),
            );
            entries = entries.push(faces);
        } else {
            entries = entries.push(
                text("No separately authored face attachments were found in this Emet CDF.")
                    .size(11)
                    .style(design::muted),
            );
        }
        if !emet.beacons.is_empty() {
            let visible = state
                .equipment_overrides
                .get(evolve_core::character_emet::BEACON)
                .is_none_or(|value| value == evolve_core::character_emet::BEACON_ON);
            entries = entries.push(text("Respawn Beacon").size(12));
            entries = entries.push(
                design::button(if visible {
                    "☑ Show Respawn Beacon"
                } else {
                    "☐ Show Respawn Beacon"
                })
                .on_press_maybe((!state.busy).then_some(Message::ViewerEmetBeacon(!visible))),
            );
        } else {
            entries = entries.push(
                text("No attached Respawn Beacon mesh is declared in this CDF; a beacon cannot be placed at a guessed position.")
                    .size(11).style(design::muted),
            );
        }
        entries = entries.push(
            text("These changes rebuild Emet's source-authored CDF assembly and are included when exporting this active CDF. No other hunter is affected.")
                .size(11).style(design::muted),
        );
    }
    // This is a local viewport-only choice for the original Kala 3P CDF.
    // No other character gets alternate colors or fabricated emissive maps.
    let kala_selected = state.cdf_report.as_ref().is_some_and(|report| {
        report
            .cdf_skeleton
            .eq_ignore_ascii_case("characters/hunters/merc_tech/tech_3p_skeleton.chr")
    });
    if kala_selected {
        entries = entries.push(text("KALA (TECH) · ARM TUBING").size(13));
        entries = entries.push(
            row![
                design::button(
                    if state.kala_tubing == evolve_core::KalaTubingState::Banshee {
                        "✓ Banshee · Blue"
                    } else {
                        "○ Banshee · Blue"
                    }
                )
                .style(
                    if state.kala_tubing == evolve_core::KalaTubingState::Banshee {
                        design::primary
                    } else {
                        design::secondary
                    }
                )
                .on_press_maybe(
                    (!state.busy && state.kala_tubing != evolve_core::KalaTubingState::Banshee)
                        .then_some(Message::ViewerKalaTubingState(
                            evolve_core::KalaTubingState::Banshee
                        ))
                ),
                design::button(
                    if state.kala_tubing == evolve_core::KalaTubingState::ArmorReducer {
                        "✓ Armor Reducer · Red"
                    } else {
                        "○ Armor Reducer · Red"
                    }
                )
                .style(
                    if state.kala_tubing == evolve_core::KalaTubingState::ArmorReducer {
                        design::primary
                    } else {
                        design::secondary
                    }
                )
                .on_press_maybe(
                    (!state.busy
                        && state.kala_tubing != evolve_core::KalaTubingState::ArmorReducer)
                        .then_some(Message::ViewerKalaTubingState(
                            evolve_core::KalaTubingState::ArmorReducer
                        ))
                ),
            ]
            .spacing(7),
        );
        entries = entries.push(
            text("Kala's named Glow material stays source-blue in both modes. Body's separate ALPHAGLOW highlights retain their actual diffuse RGB (which can be red); only the equipment glow follows the Banshee/Armor Reducer choice. Every emission uses the 3 = 100 preview reference. Original textures and FBX exports stay unchanged.")
                .size(11)
                .style(design::muted),
        );
    }
    // Only show emission controls when an actual loaded GPU source carries
    // authored emission or the optional verified armor glow overlay.
    // A surface's strength alone is not proof of an emissive texture.
    let emissive_available = state.model.is_some()
        && (state.emissive_map.is_some()
            || state.cdf_surfaces.as_ref().is_some_and(|surfaces| {
                surfaces.iter().any(|surface| {
                    surface.visible
                        && (surface.armor_emissive.is_some()
                            || (surface.emissive.is_some() && surface.emission_strength > 0.0))
                })
            }));
    if emissive_available {
        entries = entries.push(
            row![
                text(format!(
                    "Emissive strength: {:.2}×",
                    state.emissive_strength
                ))
                .size(12),
                slider(
                    0.0..=4.0,
                    state.emissive_strength,
                    Message::ViewerEmissiveStrengthChanged,
                )
                .step(0.05_f32)
                .width(Fill),
                design::button("Reset").on_press(Message::ViewerEmissiveStrengthChanged(1.0)),
            ]
            .spacing(7),
        );
        entries = entries.push(
            text("Live viewport adjustment to authored emission and the optional Glow overlay. Original source values and exported FBX remain unchanged.")
                .size(11)
                .style(design::muted),
        );
    }
    if state.equipment_slots.is_empty() {
        entries = entries.push(
            text("Open a character .cdf to choose model options. Verified interchangeable left/right arms appear separately.")
                .size(12).style(design::muted)
        );
    }
    for slot in &state.equipment_slots {
        let current = state
            .equipment_overrides
            .get(&slot.name)
            .map(String::as_str)
            .unwrap_or(&slot.original);
        entries = entries.push(text(&slot.name).size(13));
        let mut choices = column![].spacing(3);
        for choice in &slot.choices {
            let active = choice.binding.eq_ignore_ascii_case(current);
            choices = choices.push(
                design::button(
                    row![
                        text(if active { "✓" } else { "○" }).size(13),
                        text(choice.label.as_str()).size(12).width(Fill),
                    ]
                    .spacing(8),
                )
                .width(Fill)
                .style(if active {
                    design::primary
                } else {
                    design::text_button
                })
                .on_press_maybe((!state.busy && !active).then_some(Message::ViewerEquip(
                    slot.name.clone(),
                    choice.binding.clone(),
                ))),
            );
        }
        entries = entries.push(choices);
    }
    panel(container(scrollable(entries).height(Fill)).padding(12))
        .width(Fill)
        .height(Fill)
        .into()
}

fn dock_tab<'a>(
    label: &'static str,
    tab: ViewerSidebarTab,
    active: ViewerSidebarTab,
) -> Element<'a, Message> {
    let select = design::button(text(label).size(11))
        .padding([6, 5])
        .style(if active == tab {
            design::primary
        } else {
            design::secondary
        })
        .on_press(Message::ViewerSidebarSelect(tab));
    if matches!(
        tab,
        ViewerSidebarTab::Files | ViewerSidebarTab::Equipment | ViewerSidebarTab::GfxTools
    ) {
        return select.into();
    }
    row![
        select,
        tooltip(
            design::button(text("×").size(14))
                .padding([5, 4])
                .style(design::text_button)
                .on_press(Message::ViewerSidebarClose(tab)),
            text(format!("Close {label} tab")).size(11),
            tooltip::Position::Bottom,
        ),
    ]
    .spacing(0)
    .align_y(iced::Center)
    .into()
}

fn browser(state: &State) -> Element<'_, Message> {
    let active = state.viewer_sidebar_active;
    let mut tabs = row![
        dock_tab("Files", ViewerSidebarTab::Files, active),
        dock_tab("Model Options", ViewerSidebarTab::Equipment, active),
    ]
    .spacing(4);
    if state.gfx_active {
        tabs = tabs.push(dock_tab("GFX Tools", ViewerSidebarTab::GfxTools, active));
    }
    if state.viewer_sidebar_animations {
        tabs = tabs.push(dock_tab("Animations", ViewerSidebarTab::Animations, active));
    }
    if state.viewer_show_inspector {
        tabs = tabs.push(dock_tab("Inspector", ViewerSidebarTab::Inspector, active));
    }
    if state.viewer_sidebar_dependencies {
        tabs = tabs.push(dock_tab(
            "Dependencies",
            ViewerSidebarTab::Dependencies,
            active,
        ));
    }
    let content = match active {
        ViewerSidebarTab::Files => explorer(state),
        ViewerSidebarTab::Equipment => equipment_browser(state),
        ViewerSidebarTab::GfxTools if state.gfx_active => gfx_tools(state),
        ViewerSidebarTab::Animations if state.viewer_sidebar_animations => animation_browser(state),
        ViewerSidebarTab::Inspector if state.viewer_show_inspector => {
            viewer_inspector::sidebar_inspector(state)
        }
        ViewerSidebarTab::Dependencies if state.viewer_sidebar_dependencies => {
            viewer_inspector::sidebar_dependencies(state)
        }
        _ => explorer(state),
    };
    column![tabs, content]
        .spacing(5)
        .height(Fill)
        .width(Fill)
        .into()
}

fn gfx_nav_section<'a>(
    state: &State,
    label: &'static str,
    section: GfxToolsSection,
) -> Element<'a, Message> {
    design::button(text(label).size(12))
        .padding([7, 9])
        .style(if state.gfx_tools_section == section {
            design::primary
        } else {
            design::secondary
        })
        .on_press(Message::GfxToolsSection(section))
        .into()
}

/// All GFX commands live in the same docked right sidebar as Files/Model
/// Options. Sections retain the existing staged edit operations and status.
fn gfx_tools(state: &State) -> Element<'_, Message> {
    let mut actions = column![
        row![
            text("GFX TOOLS").size(16),
            iced::widget::space().width(Fill),
            text(if state.gfx_busy {
                "Working…"
            } else {
                "Ready"
            })
            .size(11)
            .style(design::muted),
        ]
        .align_y(iced::Center),
        row![
            gfx_nav_section(state, "Preview", GfxToolsSection::Preview),
            gfx_nav_section(state, "Layout", GfxToolsSection::Layout),
            gfx_nav_section(state, "Featured", GfxToolsSection::Featured),
        ]
        .spacing(5),
        row![
            gfx_nav_section(state, "Scripts", GfxToolsSection::Scripts),
            gfx_nav_section(state, "Resources", GfxToolsSection::Resources),
            gfx_nav_section(state, "Inspector", GfxToolsSection::Inspector),
            gfx_nav_section(state, "Project", GfxToolsSection::Project),
        ]
        .spacing(5),
    ]
    .spacing(10);
    if let Some(session) = &state.gfx_session {
        let info = &session.info;
        match state.gfx_tools_section {
            GfxToolsSection::Preview => {
                actions = actions
                    .push(text("FRAME RENDERING").size(13))
                    .push(
                        text(format!(
                            "{} · {}×{} · {} frame(s) · {:.1} FPS",
                            session.relative, info.width, info.height, info.frames, info.fps
                        ))
                        .size(11),
                    )
                    .push(
                        text(format!(
                            "{} sprites · {} tags · {} bitmap resources · {} imports",
                            info.sprites.len(),
                            info.tags,
                            info.resources.len(),
                            info.imports.len()
                        ))
                        .size(11)
                        .style(design::muted),
                    )
                    .push(
                        row![
                            design::button("Render frame").on_press_maybe(
                                (!state.gfx_busy).then_some(Message::GfxAction(
                                    crate::gfx_editor::Action::PreviewFrame
                                ))
                            ),
                            design::button("Open FFDec").on_press_maybe(
                                (!state.gfx_busy).then_some(Message::GfxAction(
                                    crate::gfx_editor::Action::OpenGui
                                ))
                            ),
                            design::button("Reload").on_press_maybe((!state.gfx_busy).then_some(
                                Message::GfxAction(crate::gfx_editor::Action::ReloadGui)
                            )),
                        ]
                        .spacing(5),
                    )
                    .push(
                        text(state.gfx_status.as_str())
                            .size(11)
                            .style(design::muted),
                    )
                    .push(text("FFDec executable").size(12))
                    .push(
                        text_input(
                            "Path to ffdec-cli.exe or ffdec.jar",
                            &state.preferences.ffdec_executable,
                        )
                        .on_input(Message::GfxConfigChanged)
                        .padding(8),
                    )
                    .push(design::button("Choose FFDec…").on_press(Message::GfxPickBackend))
                    .push(text("INTERACTIVE PLAYBACK").size(13))
                    .push(
                        design::button(if state.gfx_interactive_open {
                            "Hide Ruffle controls"
                        } else {
                            "Show Ruffle controls"
                        })
                        .on_press(Message::GfxToggleInteractive),
                    );
                if state.gfx_interactive_open {
                    if session
                        .relative
                        .eq_ignore_ascii_case("Libs/UI/main_menu.gfx")
                    {
                        actions = actions
                            .push(text("ORIGINAL MENU ACTIONSCRIPT").size(12))
                            .push(text("Control the SAME running Ruffle Web preview; no restart per action. Only source-verified handlers are dispatched. Native game services remain blocked.").size(11).style(design::muted));
                        use crate::gfx_interactive::PreviewPreset as MenuPreset;
                        for (label, choices) in [
                            ("MOVIE", &[MenuPreset::AsAuthored][..]),
                            (
                                "ENGAGEMENT PROMPT",
                                &[MenuPreset::HideEngagement, MenuPreset::ShowEngagement][..],
                            ),
                            (
                                "SIGN-IN PROMPT",
                                &[MenuPreset::HideSigningIn, MenuPreset::ShowSigningIn][..],
                            ),
                            (
                                "MENU NAVIGATION",
                                &[MenuPreset::Back, MenuPreset::ReturnToMainMenu][..],
                            ),
                            (
                                "PATCH NOTES",
                                &[MenuPreset::ShowPatchNotes, MenuPreset::HidePatchNotes][..],
                            ),
                            (
                                "NEWS PANEL",
                                &[MenuPreset::ShowNews, MenuPreset::HideNews][..],
                            ),
                            (
                                "CHALLENGE CONTROLS",
                                &[MenuPreset::ShowChallenges, MenuPreset::HideChallenges][..],
                            ),
                        ] {
                            actions = actions.push(text(label).size(10).style(design::muted));
                            let mut choices_row = row![].spacing(6);
                            for &preset in choices {
                                let available = state
                                    .gfx_interactive_running
                                    .as_ref()
                                    .filter(|live| live.is_running())
                                    .is_none_or(|live| live.can_dispatch_live(preset));
                                let description = if available {
                                    preset.description().to_owned()
                                } else {
                                    format!("{} (not registered)", preset.description())
                                };
                                choices_row = choices_row.push(
                                    design::button(text(description)).on_press_maybe(
                                        available
                                            .then_some(Message::GfxSelectPreviewPreset(preset)),
                                    ),
                                );
                            }
                            actions = actions.push(choices_row);
                        }
                    }
                    actions = actions
                        .push(text("Live mode opens a local browser window using the bundled Ruffle Web runtime. Desktop Ruffle does not support live button commands.").size(11).style(design::muted))
                        .push(
                            row![
                                design::button("Open / focus live preview").on_press_maybe(
                                    (!state.gfx_busy).then_some(Message::GfxRunInteractive)
                                ),
                                design::button("Stop").on_press_maybe(
                                    state
                                        .gfx_interactive_running
                                        .as_ref()
                                        .map(|_| Message::GfxStopInteractive)
                                )
                            ]
                            .spacing(6),
                        )
                        .push(
                            text(
                                if state
                                    .gfx_interactive_running
                                    .as_ref()
                                    .is_some_and(|p| p.is_running())
                                {
                                    "Ruffle Web: running in local browser — controls apply live"
                                } else {
                                    "Ruffle Web: stopped"
                                },
                            )
                            .size(11)
                            .style(design::muted),
                        )
                        .push(
                            text(state.gfx_interactive_status.as_str())
                                .size(11)
                                .style(design::muted),
                        );
                }
                actions = actions.push(
                    text("Only source-verified callbacks are offered in live playback. Browser status confirms dispatch, not native game transitions. Unsupported handlers remain unavailable until their original dependencies are traced.")
                        .size(11).style(design::muted)
                );
            }
            GfxToolsSection::Layout => {
                actions = actions.push(text("ROOT FIRST-FRAME LAYERS").size(13))
                    .push(text("Select a layer, drag its bounds in the viewport or edit its transform, then stage the change.")
                        .size(11).style(design::muted));
                if !session.layers.is_empty() {
                    actions = actions.push(
                        design::button(if state.gfx_layout_show_raster {
                            "Hide FFDec raster backdrop"
                        } else {
                            "Show FFDec raster backdrop (requires rendered frame)"
                        })
                        .on_press(Message::GfxToggleLayoutRaster),
                    );
                    actions = actions
                        .push(
                            text_input(
                                "Filter layers by name, ID or depth…",
                                &state.gfx_layer_filter,
                            )
                            .on_input(Message::GfxLayerFilterChanged)
                            .padding(8),
                        )
                        .push(
                            text(format!("{} original root layer(s)", session.layers.len()))
                                .size(11)
                                .style(design::muted),
                        );
                    for (i, layer) in session.layers.iter().enumerate() {
                        let query = state.gfx_layer_filter.trim().to_ascii_lowercase();
                        if !query.is_empty()
                            && !layer.label.to_ascii_lowercase().contains(&query)
                            && !layer.depth.to_string().contains(&query)
                            && !layer.character.to_string().contains(&query)
                        {
                            continue;
                        }
                        actions = actions.push(
                            design::button(text(format!(
                                "Depth {} · {} [#{}] {}{}",
                                layer.depth,
                                layer.label,
                                layer.character,
                                if layer.editable { "" } else { "· read-only " },
                                if state.gfx_layout_selected == Some(i) {
                                    "✓"
                                } else {
                                    ""
                                }
                            )))
                            .on_press(Message::GfxLayoutSelect(i)),
                        );
                    }
                    if let Some(layer) = state
                        .gfx_layout_selected
                        .and_then(|i| session.layers.get(i))
                    {
                        actions = actions
                            .push(
                                text(format!(
                                    "Selected depth {} / symbol {} — {}",
                                    layer.depth, layer.character, layer.reason
                                ))
                                .size(11)
                                .style(design::muted),
                            )
                            .push(text("X / Y (pixels)").size(12))
                            .push(
                                row![
                                    text_input("X", &state.gfx_layout_x)
                                        .on_input(|v| Message::GfxLayoutValue("x", v)),
                                    text_input("Y", &state.gfx_layout_y)
                                        .on_input(|v| Message::GfxLayoutValue("y", v))
                                ]
                                .spacing(4),
                            )
                            .push(text("X / Y scale (%)").size(12))
                            .push(
                                row![
                                    text_input("X scale", &state.gfx_layout_sx)
                                        .on_input(|v| Message::GfxLayoutValue("sx", v)),
                                    text_input("Y scale", &state.gfx_layout_sy)
                                        .on_input(|v| Message::GfxLayoutValue("sy", v))
                                ]
                                .spacing(4),
                            )
                            .push(text("Rotation (degrees)").size(12))
                            .push(
                                text_input("Rotation", &state.gfx_layout_rotation)
                                    .on_input(|v| Message::GfxLayoutValue("rotation", v)),
                            )
                            .push(
                                design::button(if state.gfx_layout_visible {
                                    "☑ Visible (click to hide)"
                                } else {
                                    "☐ Hidden (click to show)"
                                })
                                .on_press(Message::GfxLayoutVisible(!state.gfx_layout_visible)),
                            )
                            .push(
                                design::button("Stage selected layout edit").on_press_maybe(
                                    (layer.editable && !state.gfx_busy)
                                        .then_some(Message::GfxStageLayout),
                                ),
                            )
                            .push(text("For static first-frame elements, toggle Hidden and Stage to remove that placement. Timeline-recreated items need a persistent ActionScript edit, such as the Featured tab.").size(11).style(design::muted));
                    }
                }
            }
            GfxToolsSection::Featured => {
                actions = actions
                    .push(text("FEATURED PROMOTION").size(13))
                    .push(text("Edit the original main-menu promotion: title, TRY/BUY buttons, or the entire offer. Hiding/removing uses a frame guard so rotating offers cannot simply re-show it.").size(11).style(design::muted));
                if !session
                    .relative
                    .eq_ignore_ascii_case("Libs/UI/main_menu.gfx")
                {
                    actions = actions.push(
                        text("Open Libs/UI/main_menu.gfx to edit featured promotion controls.")
                            .size(11)
                            .style(design::muted),
                    );
                } else if session.project.is_none() {
                    actions = actions.push(text("Open the GFX from an editable Mod Project. Asset Library movies remain read-only.").size(11).style(design::muted));
                } else {
                    use crate::gfx_featured_promo::{Mode, Target};
                    actions = actions.push(text("CHOOSE TARGET").size(12));
                    for target in [Target::All, Target::Title, Target::Try, Target::Buy] {
                        actions = actions.push(
                            design::button(text(format!(
                                "{} {}",
                                if state.gfx_featured_target == target {
                                    "●"
                                } else {
                                    "○"
                                },
                                target.label()
                            )))
                            .on_press(Message::GfxFeaturedTarget(target)),
                        );
                    }
                    actions = actions
                        .push(text("PERSISTENT VISIBILITY / REMOVAL").size(12))
                        .push(row![
                            design::button("Hide every time").on_press_maybe(
                                (!state.gfx_busy).then_some(Message::GfxFeaturedStage(Mode::Hide))
                            ),
                            design::button("Remove / disable every time").on_press_maybe(
                                (!state.gfx_busy).then_some(Message::GfxFeaturedStage(Mode::Remove))
                            ),
                        ].spacing(4))
                        .push(text("Removal hides all targeted clips on every frame and disables/removes dynamically attached controls where supported. Static timeline definitions are preserved.").size(11).style(design::muted))
                        .push(text("RUNTIME TEXT REPLACEMENT").size(12))
                        .push(text_input(
                            "Enter replacement for title, TRY, or BUY…",
                            &state.gfx_featured_text,
                        ).on_input(Message::GfxFeaturedText).padding(8))
                        .push(design::button("Stage persistent text edit").on_press_maybe(
                            (!state.gfx_busy
                                && state.gfx_featured_target != Target::All
                                && !state.gfx_featured_text.is_empty())
                                .then_some(Message::GfxFeaturedStage(Mode::Text))
                        ))
                        .push(text("Each action is staged in an isolated session via FFDec. Undo before changing the same target; Apply to Mod Project commits the staged GFX only after validation. In-game offline verification is still required.").size(11).style(design::muted));
                }
            }
            GfxToolsSection::Scripts => {
                actions = actions
                    .push(text("SOURCE & ACTIONSCRIPT").size(13))
                    .push(text(if info.script_present {
                        "Compiled ActionScript detected. FFDec exports editable scripts from the staged GFX."
                    } else {
                        "No embedded ActionScript detected in this movie."
                    }).size(11).style(design::muted))
                    .push(design::button("Export ActionScript…").on_press_maybe(
                        (!state.gfx_busy).then_some(Message::GfxAction(
                            crate::gfx_editor::Action::ExportScripts
                        ))
                    ))
                    .push(design::button("Rebuild staged ActionScript…").on_press_maybe(
                        (!state.gfx_busy).then_some(Message::GfxAction(
                            crate::gfx_editor::Action::ImportScripts
                        ))
                    ))
                    .push(text("DISPLAY TAG XML").size(13))
                    .push(design::button("Export tag XML…").on_press_maybe(
                        (!state.gfx_busy).then_some(Message::GfxAction(
                            crate::gfx_editor::Action::ExportXml
                        ))
                    ))
                    .push(design::button("Rebuild staged XML…").on_press_maybe(
                        (!state.gfx_busy).then_some(Message::GfxAction(
                            crate::gfx_editor::Action::ImportXml
                        ))
                    ))
                    .push(text("Edits remain in the isolated workspace until applied to the Mod Project.")
                        .size(11).style(design::muted));
            }
            GfxToolsSection::Resources => {
                actions = actions.push(text("IMPORTS & RESOURCE IDS").size(13));
                if !session.dependencies.is_empty() {
                    actions = actions.push(text("SAME-BUILD GFX/SWF DEPENDENCIES").size(13));
                    for dep in &session.dependencies {
                        actions = actions.push(
                            text(format!("{} — {}", dep.reference, dep.outcome))
                                .size(11)
                                .style(design::muted),
                        );
                    }
                }
                if !info.resources.is_empty() {
                    actions =
                        actions.push(text("REPLACE EXISTING RESOURCE · CHARACTER ID").size(13));
                    for (index, item) in info.resources.iter().enumerate().take(80) {
                        actions = actions.push(
                            design::button(text(format!(
                                "{} #{} {}",
                                item.kind,
                                item.id,
                                if state.gfx_resource == index {
                                    " ✓"
                                } else {
                                    ""
                                }
                            )))
                            .on_press(Message::GfxSelectResource(index)),
                        );
                    }
                    if info.resources.len() > 80 {
                        actions = actions.push(
                    text("Only the first 80 tags are selectable here; use full FFDec for others.")
                        .size(11)
                        .style(design::muted),
                );
                    }
                    actions =
                        actions.push(design::button("Replace selected resource…").on_press_maybe(
                            (!state.gfx_busy).then_some(Message::GfxPickReplacement),
                        ));
                    if info
                        .resources
                        .get(state.gfx_resource)
                        .is_some_and(|resource| resource.kind == "text")
                    {
                        actions = actions
                            .push(text("EDIT SELECTED TEXT RESOURCE").size(12))
                            .push(text_input("Replacement UTF-8 text", &state.gfx_resource_text)
                                .on_input(Message::GfxResourceTextChanged)
                                .padding(8))
                            .push(design::button("Stage text resource edit").on_press_maybe(
                                (!state.gfx_busy
                                    && !state.gfx_resource_text.is_empty()
                                    && session.project.is_some())
                                    .then_some(Message::GfxResourceTextStage)
                            ))
                            .push(text("Changes the selected original text resource by character ID through FFDec. Runtime-created ActionScript text is separate. Undo or Apply to Mod Project.").size(11).style(design::muted));
                    }
                }
                if !info.imports.is_empty() {
                    actions = actions.push(
                        text(format!(
                            "Imported movies (same-build assets required): {}",
                            info.imports.join(", ")
                        ))
                        .size(11)
                        .style(design::muted),
                    );
                }
            }
            GfxToolsSection::Inspector => {
                if session
                    .relative
                    .eq_ignore_ascii_case("Libs/UI/main_menu.gfx")
                {
                    actions = actions
                        .push(text("ORIGINAL UI_DATA MENU COMMANDS").size(13))
                        .push(
                            text(session.menu_command_status.as_str())
                                .size(11)
                                .style(design::muted),
                        );
                    if !session.menu_commands.is_empty() {
                        actions = actions.push(
                            text("These are source-defined commands, not yet native Evolve handlers. Local submenu targets can be mapped to ActionScript; external game commands stay disabled.")
                                .size(11)
                                .style(design::muted),
                        );
                        for item in &session.menu_commands {
                            actions = actions
                                .push(
                                    text(format!(
                                        "{} · {} → {}",
                                        item.submenu, item.label, item.command
                                    ))
                                    .size(11),
                                )
                                .push(
                                    text(format!(
                                        "{}{}{}",
                                        item.outcome(),
                                        if item.disabled_when.is_empty() {
                                            ""
                                        } else {
                                            " · conditional disabled"
                                        },
                                        if item.hidden_when.is_empty() {
                                            ""
                                        } else {
                                            " · conditional hidden"
                                        },
                                    ))
                                    .size(10)
                                    .style(design::muted),
                                );
                        }
                    }
                }
                actions = actions.push(text("UI HIERARCHY").size(13));
                actions = actions.push(
                    design::button(if state.gfx_live_open {
                        "Close Live UI Inspector (read-only)"
                    } else {
                        "Live UI Inspector (read-only)…"
                    })
                    .on_press(Message::GfxToggleLiveInspector),
                );
                if state.gfx_live_open {
                    actions = actions
                        .push(text("LIVE UI INSPECTOR · READ-ONLY").size(14))
                        .push(
                            text(state.gfx_live_status.as_str())
                                .size(11)
                                .style(design::muted),
                        )
                        .push(
                            design::button("Load externally captured UI snapshot JSON…")
                                .on_press(Message::GfxImportUiSnapshot),
                        );
                    if let Some(snapshot) = &state.gfx_live_snapshot {
                        actions = actions.push(
                            text(format!(
                                "{} · frame {} · {} nodes",
                                snapshot.movie,
                                snapshot.frame,
                                snapshot.nodes.len()
                            ))
                            .size(12),
                        );
                        for node in &snapshot.nodes {
                            let indent = "  "
                                .repeat(super::gfx_live_inspector::indent(snapshot, node).min(12));
                            actions = actions.push(
                                text(format!(
                                    "{}{} · {} · {}  at ({:.1},{:.1})  {}×{}  {}",
                                    indent,
                                    node.name,
                                    node.kind,
                                    node.text,
                                    node.x,
                                    node.y,
                                    node.width,
                                    node.height,
                                    if node.visible { "visible" } else { "hidden" },
                                ))
                                .size(11),
                            );
                        }
                    } else {
                        actions = actions.push(text(
                    "Actual live game hierarchy capture requires a separately verified offline GFx game-thread provider. No guessed pointers, process injection, or simulated live nodes."
                ).size(11).style(design::muted));
                    }
                }
            }
            GfxToolsSection::Project => {
                actions = actions
                    .push(text("STAGED GFX CHANGES").size(13))
                    .push(text(format!(
                        "{} · {} staged edit(s) · {}",
                        if session.project.is_some() { "Mod Project" } else { "Read-only library" },
                        session.edits,
                        if session.applied { "Applied" } else { "Not applied" },
                    )).size(11).style(design::muted))
                    .push(design::button("Import compiled GFX…")
                        .on_press_maybe((!state.gfx_busy).then_some(Message::GfxPickEdited)))
                    .push(design::button("Undo last staged edit")
                        .on_press_maybe((!state.gfx_busy && session.edits > 0)
                            .then_some(Message::GfxAction(crate::gfx_editor::Action::Undo))))
                    .push(design::button("Open editing files…")
                        .on_press(Message::GfxRevealSession))
                    .push(design::button("Apply to Mod Project")
                        .style(design::primary)
                        .on_press_maybe((
                            !state.gfx_busy && session.project.is_some()
                                && session.edits > 0 && !session.applied
                        ).then_some(Message::GfxApply)))
                    .push(text("The original PAK remains unchanged. Applying keeps EditorBackups in the extracted Mod Project.")
                        .size(11).style(design::muted));
            }
        }
    } else {
        actions = actions
            .push(
                text(if state.gfx_busy {
                    "Loading selected GFX..."
                } else {
                    "Select a .gfx in Files to inspect or edit it."
                })
                .size(12)
                .style(design::muted),
            )
            .push(
                text(state.gfx_status.as_str())
                    .size(11)
                    .style(design::muted),
            )
            .push(
                design::button("Inspect selected GFX")
                    .on_press_maybe((!state.gfx_busy).then_some(Message::GfxOpenSelected)),
            );
    }
    scrollable(panel(actions.padding(12)).width(Fill))
        .height(Fill)
        .width(Fill)
        .into()
}

/// Only the original stage or layout canvas occupies the viewport. All
/// FFDec/Ruffle/project controls are docked with the other sidebar tools.
fn gfx_viewport(state: &State) -> Element<'_, Message> {
    if state.gfx_layout_open
        && let Some(session) = &state.gfx_session
    {
        let stage = iced::Size::new(session.info.width as f32, session.info.height as f32);
        let show_raster = state.gfx_layout_show_raster && state.gfx_preview.is_some();
        let canvas = canvas(super::gfx_layout_canvas::GfxLayoutCanvas {
            layers: &session.layers,
            selected: state.gfx_layout_selected,
            stage,
            raster_background: show_raster,
        })
        .width(Fill)
        .height(Fill);
        let surface: Element<'_, Message> =
            if let (true, Some(handle)) = (show_raster, &state.gfx_preview) {
                stack![
                    image(handle.clone())
                        .content_fit(iced::ContentFit::Contain)
                        .width(Fill)
                        .height(Fill),
                    canvas,
                ]
                .width(Fill)
                .height(Fill)
                .into()
            } else {
                canvas.into()
            };
        return column![
            text("GFX LAYOUT · FIRST FRAME")
                .size(12)
                .style(design::muted),
            surface,
            text("Drag bounded shapes to move them; stage changes in GFX Tools → Layout.")
                .size(11)
                .style(design::muted),
        ]
        .spacing(8)
        .height(Fill)
        .width(Fill)
        .into();
    }
    let surface: Element<'_, Message> = if let Some(handle) = &state.gfx_preview {
        image(handle.clone())
            .content_fit(iced::ContentFit::Contain)
            .width(Fill)
            .height(Fill)
            .into()
    } else {
        text(if state.gfx_busy || state.gfx_session.is_none() {
            state.gfx_status.as_str()
        } else {
            "GFX loaded. Use GFX Tools → Preview to render a frame or open the interactive Ruffle player."
        })
        .size(12)
        .style(design::muted)
        .into()
    };
    container(surface)
        .center(Fill)
        .width(Fill)
        .height(Fill)
        .into()
}

fn text_viewport(state: &State) -> Element<'_, Message> {
    let doc = state
        .text_document
        .as_ref()
        .expect("text viewport is displayed only for a loaded text document");
    let label = if doc.editable {
        "Editable Modding project"
    } else {
        "Read-only Asset Library source"
    };
    let tools = row![
        text("TEXT EDITOR").size(15),
        text(doc.member.as_str()).size(12).width(Fill),
        text(if doc.dirty() { "● Unsaved" } else { "Saved" })
            .size(12)
            .style(design::muted),
        design::button("Save")
            .style(design::primary)
            .on_press_maybe(
                (doc.editable && doc.dirty() && !state.busy).then_some(Message::TextEditorSave)
            ),
        design::button("Discard")
            .style(design::secondary)
            .on_press_maybe(doc.dirty().then_some(Message::TextEditorDiscard)),
        design::button("Close")
            .style(design::secondary)
            .on_press(Message::TextEditorClose),
    ]
    .spacing(9)
    .align_y(iced::Center);
    let content = if doc.editable {
        text_editor(&doc.content)
            .on_action(Message::TextEditorAction)
            .height(Fill)
    } else {
        text_editor(&doc.content).height(Fill)
    };
    // Iced's text editor is read-only when no on_action handler is attached.
    column![
        tools,
        text(label).size(11).style(design::muted),
        container(content)
            .width(Fill)
            .height(Fill)
            .padding(9)
            .style(|theme| container::Style {
                background: Some(Background::Color(design::field(theme))),
                ..container::Style::default()
            }),
    ]
    .spacing(8)
    .height(Fill)
    .into()
}

fn viewport(state: &State) -> Element<'_, Message> {
    if state.text_document.is_some() {
        return text_viewport(state);
    }
    if state.gfx_active {
        return gfx_viewport(state);
    }
    let header = row![
        text("User Perspective").size(12),
        text(" • ").size(12),
        text(if state.model.is_some() {
            "GPU Model Preview"
        } else if state.texture_preview.is_some() {
            "Original DDS Texture Preview"
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
                surfaces: state.cdf_surfaces.clone(),
                diffuse: state.diffuse.clone(),
                normal_map: state.normal_map.clone(),
                emissive: state.emissive_map.clone(),
                specular: state.specular_map.clone(),
                gloss: state.gloss_map.clone(),
                yaw: state.yaw,
                pitch: state.pitch,
                zoom: state.zoom.max(0.2),
                light_strength: state.light_strength,
                emissive_gain: state.emissive_strength,
                emissive_time: state.decal_time,
                pan_x: state.pan_x,
                pan_y: state.pan_y,
                shift: state.keyboard_modifiers.shift(),
                level_camera: None,
                world: None,
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
    } else if let Some(handle) = &state.texture_preview {
        container(
            image(handle.clone())
                .content_fit(iced::ContentFit::Contain)
                .width(Fill)
                .height(Fill),
        )
        .center(Fill)
        .style(|theme| container::Style {
            background: Some(Background::Color(design::field(theme))),
            ..container::Style::default()
        })
        .into()
    } else {
        container(
            column![
            text("3D / ASSET PREVIEW").size(19),
            text("Choose a model, CDF, or DDS from extracted files.").size(14).style(design::muted),
            text("This workspace does not invent missing character pieces, textures or animations.")
                .size(12).style(design::muted),
            design::button("Retry selected preview").on_press(Message::ViewerPreviewSelected)
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

    let camera_icon = |glyph: &'static str, hint: &'static str, action| {
        tooltip(
            design::button(text(glyph).size(18))
                .width(39)
                .height(36)
                .on_press(action),
            text(hint).size(12),
            tooltip::Position::Bottom,
        )
    };
    let toolbar = row![
        camera_icon("⌖", "Frame all", Message::ViewerResetCamera),
        camera_icon("↑", "Front view", Message::ViewerCameraPreset("Front")),
        camera_icon("→", "Right view", Message::ViewerCameraPreset("Right")),
        camera_icon("⊤", "Top view", Message::ViewerCameraPreset("Top")),
        camera_icon(
            "◇",
            "Isometric view",
            Message::ViewerCameraPreset("Isometric")
        ),
        camera_icon(
            if state.viewer_wireframe { "◉" } else { "▧" },
            if state.viewer_wireframe {
                "Shaded display"
            } else {
                "Wireframe display"
            },
            Message::ViewerToggleWireframe
        ),
        camera_icon("−", "Zoom out", Message::ViewerZoomWheel(-2.0)),
        camera_icon("+", "Zoom in", Message::ViewerZoomWheel(2.0)),
        tooltip(
            row![
                svg(svg::Handle::from_memory(include_bytes!(
                    "../../../assets/nav/light.svg"
                )))
                .style(design::icon)
                .width(Length::Fixed(18.0))
                .height(Length::Fixed(18.0)),
                slider(
                    0.2..=4.0,
                    state.light_strength.clamp(0.2, 4.0),
                    Message::LightStrengthChanged
                )
                .width(Length::Fixed(136.0)),
                text(format!("{:.1}×", state.light_strength)).size(11),
            ]
            .align_y(iced::Center)
            .spacing(6),
            text("Viewport light intensity (preview only)").size(12),
            tooltip::Position::Bottom,
        ),
    ]
    .spacing(4)
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

    // While an asset worker is running, replace the tiny animation
    // timeline with one conspicuous progress area in exactly the same
    // bottom position. Only report a percentage when the worker supplies
    // measured stage/mesh-range completion.
    let footer: Element<'_, Message> = if state.busy {
        if let Some(progress) = &state.viewer_load_progress {
            let percent = progress.load(std::sync::atomic::Ordering::Relaxed).min(100);
            let phase = match percent {
                0..=34 => "Assembling CDF geometry",
                35..=84 => "Decoding original material and texture sections",
                _ => "Finalizing glow and GPU materials",
            };
            panel(
                row![
                    text(format!("{phase} · {percent}%")).size(12),
                    container(progress_bar(0.0..=100.0, percent as f32)).width(Fill),
                ]
                .spacing(12)
                .align_y(iced::Center),
            )
            .height(Length::Fixed(43.0))
            .padding(8)
            .into()
        } else {
            panel(
                row![
                    text("Loading original assets…").size(12),
                    container(progress_bar(0.0..=100.0, 0.0)).width(Fill),
                    text("Progress unavailable").size(11).style(design::muted),
                ]
                .spacing(12)
                .align_y(iced::Center),
            )
            .height(Length::Fixed(43.0))
            .padding(8)
            .into()
        }
    } else {
        panel(container(timeline).padding(5))
            .height(Length::Fixed(43.0))
            .into()
    };
    column![header, toolbar, preview, footer]
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

fn file_context_actions(state: &State) -> Element<'_, Message> {
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
                    column![
                        design::button("Preview")
                            .width(Fill)
                            .style(design::text_button)
                            .on_press(Message::ViewerPreviewSelected),
                        design::button("Import over…")
                            .width(Fill)
                            .style(design::text_button)
                            .on_press_maybe(
                                (!state.viewer_from_library && !state.busy)
                                    .then_some(Message::EditorOpenReplacement)
                            ),
                        design::button("Restore original")
                            .width(Fill)
                            .style(design::text_button)
                            .on_press_maybe(
                                (!state.viewer_from_library && !state.busy)
                                    .then_some(Message::EditorRestoreRawMember)
                            ),
                        design::button("Copy path")
                            .width(Fill)
                            .style(design::text_button)
                            .on_press(Message::ViewerCopySelectedPath),
                        design::button("Show in Explorer")
                            .width(Fill)
                            .style(design::text_button)
                            .on_press(Message::ViewerRevealSelectedFile),
                        design::button("Dependencies")
                            .width(Fill)
                            .style(design::text_button)
                            .on_press(Message::StartDependencyInspection),
                        design::button("Preview full CDF")
                            .width(Fill)
                            .style(design::text_button)
                            .on_press_maybe(is_cdf.then_some(Message::ViewerPreviewStaticCdf)),
                        design::button("Export DDS → PNG")
                            .width(Fill)
                            .style(design::text_button)
                            .on_press_maybe(is_dds.then_some(Message::ExportOpen)),
                        design::button("Rigged FBX")
                            .width(Fill)
                            .style(design::text_button)
                            .on_press_maybe(
                                (is_model && !state.busy).then_some(Message::ExportOpen)
                            ),
                    ]
                    .spacing(2),
                    if filename.to_ascii_lowercase().ends_with(".fsb")
                        || filename.to_ascii_lowercase().ends_with(".fdp")
                        || filename.to_ascii_lowercase().ends_with(".fev")
                    {
                        let fsb = filename.to_ascii_lowercase().ends_with(".fsb");
                        let fdp = filename.to_ascii_lowercase().ends_with(".fdp");
                        let editable = !state.viewer_from_library && !state.busy;
                        Element::from(
                            column![
                                text("AUDIO TOOLS · FMOD").size(13),
                                design::button("Inspect FMOD asset").on_press_maybe(
                                    (!state.audio_busy).then_some(Message::AudioInspect)
                                ),
                                design::text_input(
                                    if fdp {
                                        "Waveform # (starts at 1)"
                                    } else {
                                        "Sound sample # (starts at 1)"
                                    },
                                    &state.audio_subsong
                                )
                                .on_input(Message::AudioSubsongChanged),
                                row![
                                    design::button("Preview sound").on_press_maybe(
                                        (fsb && !state.audio_busy)
                                            .then_some(Message::AudioPreviewSample)
                                    ),
                                    design::button("Export selected WAV").on_press_maybe(
                                        (fsb && !state.audio_busy)
                                            .then_some(Message::AudioExportSample)
                                    ),
                                ]
                                .spacing(6),
                                row![
                                    design::button("Open FMOD Designer").on_press_maybe(
                                        (fdp && editable).then_some(Message::AudioOpenDesigner)
                                    ),
                                    design::button("Replace waveform WAV").on_press_maybe(
                                        (fdp && editable && !state.audio_busy)
                                            .then_some(Message::AudioReplaceWaveform)
                                    ),
                                ]
                                .spacing(6),
                                design::button("Rebuild Sound Bank · Replace WAV/OGG")
                                    .on_press_maybe(
                                        (fsb && editable && !state.audio_busy)
                                            .then_some(Message::AudioNativeRebuild)
                                    ),
                                design::button("Import rebuilt FSB/FEV").on_press_maybe(
                                    (!fdp && editable && !state.audio_busy)
                                        .then_some(Message::AudioImportRebuilt)
                                ),
                                text(state.audio_status.as_str())
                                    .size(11)
                                    .style(design::muted),
                            ]
                            .spacing(7),
                        )
                    } else {
                        Element::from(text(""))
                    },
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
    }
}

fn replacement_sheet(state: &State, index: usize) -> Element<'_, Message> {
    let target = state
        .inventory
        .as_ref()
        .and_then(|inv| inv.entries.get(index))
        .map_or("No target selected", |entry| entry.relative_path.as_str());
    let lower = target.to_ascii_lowercase();
    let texture = lower.ends_with(".dds") || lower.ends_with(".dds.0");
    panel(
        column![
            row![
                text("REPLACE PROJECT ASSET").size(13).width(Fill),
                design::button("✕").on_press(Message::EditorCancelReplacement)
            ].spacing(8),
            text(target).size(12),
            text(if texture {
                "PNG images are converted into the original DDS format and mip layout when supported."
            } else {
                "Choose a separate source file. The exact destination path is already selected."
            }).size(11).style(design::muted),
            row![
                design::button("Browse files…")
                    .on_press_maybe((!state.busy).then_some(Message::EditorBrowseReplacement)),
                text("or drag a file into this window").size(11).style(design::muted)
            ].spacing(8),
            design::text_input("Selected source file (optional manual path)", &state.editor_replace_source)
                .on_input(Message::EditorReplacementChanged),
            row![
                design::button("Import replacement")
                    .on_press_maybe((!state.busy && !state.editor_replace_source.trim().is_empty())
                        .then_some(Message::EditorImportRawMember)),
                design::button("Cancel").on_press(Message::EditorCancelReplacement),
            ].spacing(8),
            text("The extracted original is backed up and verified before replacement. Source PAKs are never modified.")
                .size(11).style(design::muted),
        ].spacing(8)
    ).into()
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
        if entry.index.is_some() && entry.index == state.viewer_context_index {
            rows = rows.push(file_context_actions(state));
        }
        if let Some(index) = entry.index
            && Some(index) == state.editor_replace_dialog
        {
            rows = rows.push(replacement_sheet(state, index));
        }
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
                text("Right-click a file to open actions beside it.")
                    .size(11)
                    .style(design::muted),
                text(if state.viewer_from_library {
                    "Source assets are read-only. Create a Modding project to replace them."
                } else {
                    "Select a project file, right-click → Import over…; browse for a file or drag it into the viewer."
                })
                .size(11)
                .style(design::muted),
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

fn compact_export_source(path: &str, empty: &str) -> String {
    if path.is_empty() {
        return empty.to_owned();
    }
    let filename = std::path::Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned());
    format!("Selected: {filename}")
}

pub(super) fn export_dialog(state: &State) -> Element<'_, Message> {
    let mut picker = column![].spacing(3);
    let filter = state.export_filter.to_ascii_lowercase();
    if let Some(inventory) = &state.inventory {
        for (index, item) in inventory
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| super::exportable_asset(&e.relative_path))
            .filter(|(_, e)| {
                filter.is_empty() || e.relative_path.to_ascii_lowercase().contains(&filter)
            })
            .take(80)
        {
            let selected = state.export_selected.contains(&index);
            picker = picker.push(
                design::button(
                    text(format!(
                        "{} {}",
                        if selected { "☑" } else { "☐" },
                        item.relative_path
                    ))
                    .size(11),
                )
                .on_press(Message::ExportToggleFile(index))
                .style(design::text_button)
                .width(Fill),
            );
        }
    }
    let flags = row![
        design::button(if state.export_rigged {
            "☑ Rigged"
        } else {
            "☐ Rigged"
        })
        .on_press(Message::ExportToggleRigged),
        design::button(if state.export_embed_textures {
            "☑ Embedded textures"
        } else {
            "☐ Embedded textures"
        })
        .on_press(Message::ExportToggleEmbedded),
        design::button(if state.export_combine_meshes {
            "☑ Combine meshes"
        } else {
            "☐ Combine meshes"
        })
        .on_press(Message::ExportToggleCombineMeshes),
        design::button(if state.export_dependencies {
            "☑ Export dependencies"
        } else {
            "☐ Export dependencies"
        })
        .on_press(Message::ExportToggleDependencies),
    ]
    .spacing(5);
    let controls = column![
        row![
            text("Export files and assemblies").size(17),
            iced::widget::space().width(Fill),
            design::button("✕").on_press(Message::ExportClose),
        ].spacing(8),
        text(format!("{} selected — Ctrl/Shift selections from Files are included when opened.",
            state.export_selected.len())).size(11).style(design::muted),
        design::text_input("Filter indexed models, CDFs, and DDS textures…", &state.export_filter)
            .on_input(Message::ExportFilterChanged),
        scrollable(picker).height(Length::Fixed(295.0)),
        flags,
        text("FBX creates a character hierarchy with named original parts. Combine meshes is optional; distinct historical armatures stay separate.").size(11).style(design::muted),
        text(if state.export_embed_textures {
            "Embedded textures selected: FBX export verifies source diffuse images on every visible CDF part."
        } else {
            "CAUTION: Embedded textures is OFF. The FBX will contain geometry and UVs, but no packed character images; assign your own verified diffuse maps or enable embedding."
        }).size(11).style(design::muted),
        text(if !state.equipment_overrides.is_empty() || state.full_armor_glow || state.damage_blend_percent > 0 {
            format!("Model Options: {} equipment override(s), Glow {}, damage blend {}%. Export only the active CDF with Rigged + Embedded textures.",
                state.equipment_overrides.len(),
                if state.full_armor_glow { "ON" } else { "OFF" },
                state.damage_blend_percent)
        } else {
            "Model Options: default equipment, Glow off, damage blend off".to_owned()
        }).size(11).style(design::muted),
        row![
            text(format!("Destination: {}", if state.export_parent.is_empty() { "Not selected" } else { &state.export_parent })).width(Fill),
            design::button("Browse folder…").on_press(Message::ExportBrowse("folder")),
        ].spacing(8),
        design::text_input("New export folder name (e.g. Scorpid)", &state.export_folder_name)
            .on_input(Message::ExportFolderChanged),
        if crate::blender_install::valid_blender(&state.blender_path).is_some() {
            Element::from(text(format!("Blender ready: {}", state.blender_path))
                .size(11).style(design::muted))
        } else {
            Element::from(row![
                text("Blender executable not found").width(Fill),
                design::button("Select Blender…").on_press(Message::ExportBrowse("blender")),
            ].spacing(8))
        },
        if state.export_rigged {
            Element::from(column![
                row![
                    text(compact_export_source(&state.skeleton_path, "Skeleton: auto from CDF")).width(Fill),
                    design::button("Select skeleton…").on_press(Message::ExportBrowse("skeleton")),
                ].spacing(8),
                row![
                    text(compact_export_source(&state.diffuse_input, "Standalone diffuse: auto from original MTL")).width(Fill),
                    design::button("Select diffuse DDS…").on_press(Message::ExportBrowse("diffuse")),
                ].spacing(8),
                design::button(if state.export_include_animation {
                    "☑ Include verified CAF" } else { "☐ Include verified CAF" })
                    .on_press(Message::ViewerToggleExportAnimations),
                row![
                    text(compact_export_source(&state.caf_path, "CAF: none selected")).width(Fill),
                    design::button("Select CAF…").on_press(Message::ExportBrowse("caf")),
                ].spacing(8),
            ].spacing(4))
        } else {
            Element::from(text("Static FBX preserves source mesh geometry and UV0.").size(11))
        },
        text("Files: <name>/<model>.fbx · dependencies: <name>/textures/*.png and <name>/dependencies/…")
            .size(11).style(design::muted),
    ].spacing(12);
    // The bottom actions stay visible even when a long source path or narrow
    // display requires the settings above to scroll. File list scrolls inside.
    column![
        scrollable(controls).height(Fill),
        row![
            design::button("Export").on_press_maybe(
                (!state.busy && !state.export_selected.is_empty()).then_some(Message::ExportBegin)
            ),
            design::button("Cancel").on_press(Message::ExportClose),
        ]
        .spacing(10),
    ]
    .height(Fill)
    .spacing(12)
    .padding(20)
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
        iced::widget::space().width(Fill),
        text(if state.gfx_active {
            "GFX: controls in right-hand GFX Tools tab"
        } else {
            "Orbit: drag  •  Pan: Shift/right drag  •  Zoom: wheel"
        })
        .size(11)
        .style(design::muted),
    ]
    .spacing(10)
    .align_y(iced::Center);
    let ratio = state.viewer_split.clamp(0.25, 0.85);
    let view_weight = (ratio * 100.0).round() as u16;
    let list_weight = (100.0 - ratio * 100.0).round() as u16;
    let panes: Element<'_, Message> = if state.viewer_explorer_visible {
        let preview = container(viewport(state)).width(Length::FillPortion(view_weight.max(1)));
        let files = container(browser(state)).width(Length::FillPortion(list_weight.max(1)));
        let grip = canvas(ExplorerGrip {
            direction: if explorer_on_left(&state.preferences.file_explorer_side) {
                -1.0
            } else {
                1.0
            },
        })
        .width(Length::Fixed(12.0))
        .height(Fill);
        let panes = if explorer_on_left(&state.preferences.file_explorer_side) {
            row![files, grip, preview]
        } else {
            row![preview, grip, files]
        };
        panes.spacing(10).height(Fill).into()
    } else {
        viewport(state)
    };
    // Resize directly on the vertical divider, not through a separate slider.
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

    let base = container(
        column![strip(state), heading, panes, footer]
            .spacing(7)
            .padding(7)
            .height(Fill),
    )
    .height(Fill)
    .width(Fill)
    .style(|theme| container::Style {
        background: Some(Background::Color(theme.palette().background)),
        ..container::Style::default()
    });
    crate::menu_popup::overlay(
        base,
        dropdown(state),
        dropdown_x(state.viewer_menu),
        36.0,
        if state.viewer_menu == Some("Model") {
            460.0
        } else {
            310.0
        },
    )
}

/// The narrow vertical separator is the only splitter control: drag it
/// horizontally to resize the File/Animation sidebar without stealing camera
/// gestures from the viewport canvas.
struct ExplorerGrip {
    direction: f32,
}

#[derive(Default)]
struct ExplorerGripState {
    dragging: bool,
    previous_x: f32,
}

impl canvas::Program<Message> for ExplorerGrip {
    type State = ExplorerGripState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if cursor.position_in(bounds).is_some() =>
            {
                state.dragging = true;
                state.previous_x = cursor.position()?.x;
                None
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.dragging = false;
                None
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) if state.dragging => {
                let delta = position.x - state.previous_x;
                state.previous_x = position.x;
                if delta.is_finite() && delta != 0.0 {
                    Some(
                        canvas::Action::publish(Message::ViewerSplitDrag(
                            self.direction * delta / 1000.0,
                        ))
                        .and_capture(),
                    )
                } else {
                    None
                }
            }
            _ => None,
        }
    }

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
            Point::new(bounds.width * 0.5 - 1.0, 0.0),
            iced::Size::new(2.0, bounds.height),
            Color::from_rgb8(90, 94, 106),
        );
        vec![frame.into_geometry()]
    }
}

pub(super) fn preferences_view(state: &State) -> Element<'_, Message> {
    let mode = state.preferences.appearance_mode.as_str();
    let pane = panel(
        container(
            column![
                text("Viewer Preferences").size(22),
                text("Appearance").size(17),
                row![
                    design::button(text(format!("Theme: {mode}")))
                        .on_press(Message::PreferenceAppearance),
                    design::button(text(format!("Density: {}", state.preferences.ui_density)))
                        .on_press(Message::PreferenceDensity),
                ]
                .spacing(10),
                text("Accent color").size(14),
                row![
                    design::button("Default").on_press(Message::PreferenceAccentPreset("Default")),
                    design::button("Classic Red")
                        .on_press(Message::PreferenceAccentPreset("Classic Red")),
                    design::button("Evolve Red")
                        .on_press(Message::PreferenceAccentPreset("Evolve Red")),
                    design::button("Deep Red")
                        .on_press(Message::PreferenceAccentPreset("Deep Red")),
                ]
                .spacing(8),
                text("Viewport").size(17),
                row![
                    design::button(if state.viewer_wireframe {
                        "Display: Wireframe"
                    } else {
                        "Display: Shaded"
                    })
                    .on_press(Message::ViewerToggleWireframe),
                    design::button(
                        if state
                            .preferences
                            .file_explorer_side
                            .eq_ignore_ascii_case("left")
                        {
                            "Sidebar: Left"
                        } else {
                            "Sidebar: Right"
                        }
                    )
                    .on_press(Message::PreferenceExplorerSide),
                ]
                .spacing(8),
                row![
                    text("Lighting").size(13),
                    slider(
                        0.2..=4.0,
                        state.light_strength.clamp(0.2, 4.0),
                        Message::LightStrengthChanged
                    )
                    .width(Length::Fixed(240.0)),
                ]
                .spacing(8)
                .align_y(iced::Center),
                design::button("Save preferences").on_press(Message::ViewerSaveLayout),
            ]
            .spacing(15),
        )
        .padding(20),
    );
    container(pane)
        .width(Fill)
        .height(Fill)
        .center(Fill)
        .padding(18)
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
