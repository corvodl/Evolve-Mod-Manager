//! Interactive viewer inspector: source-grounded dependency diagram and
//! original-vs-modified diagnostics. No graph edges are inferred from names.
use super::{Message, State, ViewerTab, uv_preview};
use crate::design;
use evolve_core::{AssetResolution, DependencyGraph, DependencyStatus};
use iced::widget::canvas::{self, Path, Stroke};
use iced::widget::{canvas as canvas_widget, column, container, row, scrollable, space, text};
use iced::{Color, Element, Fill, Length, Point, Rectangle, Renderer, Theme, mouse};
use std::sync::Arc;

const OK: Color = Color::from_rgb(0.26, 0.73, 0.51);
const BAD: Color = Color::from_rgb(0.94, 0.34, 0.38);
const WARN: Color = Color::from_rgb(0.93, 0.70, 0.32);

fn mark(status: &DependencyStatus) -> Color {
    match status {
        DependencyStatus::Source => Color::from_rgb(0.83, 0.37, 0.43),
        DependencyStatus::Resolved => OK,
        DependencyStatus::Missing => BAD,
        DependencyStatus::Ambiguous => WARN,
    }
}

struct DependencyCanvas {
    graph: Arc<DependencyGraph>,
}
impl<Message> canvas::Program<Message> for DependencyCanvas {
    type State = ();
    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, frame.size(), design::field(theme));
        let count = self.graph.nodes.len().saturating_sub(1).min(12);
        let root = Point::new(16.0, bounds.height * 0.5 - 24.0);
        frame.fill_rectangle(
            root,
            iced::Size::new(142.0, 48.0),
            mark(&DependencyStatus::Source),
        );
        frame.fill_text(canvas::Text {
            content: "CDF / MTL source".into(),
            position: Point::new(root.x + 8.0, root.y + 29.0),
            color: Color::WHITE,
            size: iced::Pixels(13.0),
            ..canvas::Text::default()
        });
        for (i, node) in self.graph.nodes.iter().skip(1).take(count).enumerate() {
            let row_h = (bounds.height - 14.0).max(5.0) / count.max(1) as f32;
            let y = 8.0 + i as f32 * row_h;
            let x = (bounds.width * 0.47).max(168.0);
            let w = (bounds.width - x - 12.0).max(30.0);
            let height = (row_h - 4.0).clamp(12.0, 34.0);
            let line = Path::line(
                Point::new(root.x + 142.0, root.y + 24.0),
                Point::new(x, y + height * 0.5),
            );
            frame.stroke(
                &line,
                Stroke::default()
                    .with_color(mark(&node.status))
                    .with_width(1.0),
            );
            frame.fill_rectangle(
                Point::new(x, y),
                iced::Size::new(w, height),
                mark(&node.status),
            );
            if height > 20.0 {
                frame.fill_text(canvas::Text {
                    content: node.label.chars().take(42).collect(),
                    position: Point::new(x + 6.0, y + 19.0),
                    color: Color::WHITE,
                    size: iced::Pixels(11.0),
                    ..canvas::Text::default()
                });
            }
        }
        vec![frame.into_geometry()]
    }
}

/// Beginner-facing docked overview. A selected model's metadata, texture
/// references and animations remain separate so a missing map is never
/// mistaken for a broken mesh.
pub(super) fn sidebar_inspector(state: &State) -> Element<'_, Message> {
    let mut summary = column![
        text("INSPECTOR").size(16),
        text("Read-only information for the selected asset. Nothing here edits your game files.")
            .size(11).style(design::muted),
        text("1 · Selected asset").size(14),
    ].spacing(8);
    let source = state.selected.and_then(|i| state.inventory.as_ref()?.entries.get(i));
    summary = summary.push(
        text(source.map_or("No file selected".to_owned(), |e| e.relative_path.clone()))
            .size(12),
    );

    summary = summary.push(text("2 · Model geometry").size(14));
    if let Some(mesh) = &state.model {
        summary = summary.push(text(format!(
            "{} vertices · {} triangles",
            mesh.positions.len(),
            mesh.indices.len() / 3
        )).size(12));
        summary = summary.push(
            text(if mesh.uv0.is_some() {
                "UV coordinates: available (needed for textures)"
            } else {
                "UV coordinates: unavailable (textures cannot be positioned reliably)"
            }).size(12).style(design::muted)
        );
    } else {
        summary = summary.push(
            text("No 3D model loaded. Choose a .skinm, .chrm or CDF in Files.")
                .size(12).style(design::muted)
        );
    }

    summary = summary.push(text("3 · Materials and textures").size(14));
    if let Some(report) = &state.material_report {
        summary = summary.push(
            text(format!(
                "{} original material slots · {} texture references",
                report.slots,
                report.textures.len()
            )).size(12)
        );
        // Consistent, beginner-readable order; never imply a referenced map
        // was loaded when it exists only as an authored MTL path.
        for (friendly, roles, explanation) in [
            ("Color", &["diffuse"][..], "Surface color and patterns"),
            ("Normal", &["normal", "normals", "bumpmap"][..], "Fine surface detail"),
            ("Specular", &["specular", "specularcolor"][..], "Reflected-light color"),
            ("Gloss", &["gloss", "glossiness", "smoothness"][..], "Shininess"),
            ("Glow", &["glow", "emissive", "emission", "emittance"][..], "Self-lit parts"),
            ("Transparency", &["opacity"][..], "See-through areas"),
        ] {
            let refs: Vec<_> = report.textures.iter()
                .filter(|r| roles.contains(&r.role.as_str())).collect();
            if refs.is_empty() {
                continue;
            }
            let found = refs.iter()
                .filter(|r| matches!(r.resolution, AssetResolution::Found(_))).count();
            summary = summary.push(
                text(format!("{friendly}: {found}/{} located — {explanation}", refs.len()))
                    .size(12)
                    .color(if found == refs.len() { OK } else { WARN })
            );
        }
        let unresolved = report.textures.iter()
            .filter(|r| !matches!(r.resolution, AssetResolution::Found(_))).count();
        if unresolved > 0 {
            summary = summary.push(
                text(format!("{unresolved} texture reference(s) still need verification. Some surfaces may look incomplete."))
                    .size(12).color(WARN)
            );
        }
        if let Some(surfaces) = &state.cdf_surfaces {
            let textured = surfaces.iter().filter(|s| s.diffuse.is_some()).count();
            summary = summary.push(
                text(format!(
                    "GPU material assignments: {textured}/{} model sections have color maps",
                    surfaces.len()
                )).size(12)
            );
        } else if report.slots > 1 {
            summary = summary.push(
                text("Several material slots exist, but verified mesh-to-material mapping is still needed.")
                    .size(12).color(WARN)
            );
        }
    } else {
        summary = summary.push(
            text("No verified material report. Mesh geometry can load even when its texture files cannot be located.")
                .size(12).style(design::muted)
        );
    }

    summary = summary.push(text("4 · Animation").size(14));
    summary = summary.push(
        text(format!("{} animation(s) identified for this character", state.auto_clips.len()))
            .size(12)
    );
    summary = summary.push(
        text(if state.skin_preview.is_some() || state.cdf_caf_preview.is_some() {
            "Original skeleton and animation are connected to the model."
        } else if state.native_caf.is_some() {
            "Animation file loaded; its bone connection has not been verified."
        } else {
            "No animation currently attached to this model."
        }).size(12).style(design::muted)
    );
    summary = summary.push(
        row![
            design::button("Animation Browser")
                .on_press(Message::ViewerSidebarAnimations(true)),
            design::button("Dependency Graph")
                .on_press(Message::StartDependencyInspection),
        ].spacing(6)
    );
    if !state.auto_resource_note.is_empty() {
        summary = summary.push(text("5 · Diagnostic details").size(14));
        summary = summary.push(
            text(state.auto_resource_note.as_str()).size(11).style(design::muted)
        );
    }
    scrollable(
        container(summary).padding(12).width(Fill)
    ).height(Fill).width(Fill).into()
}

/// Dock the actual authored-reference graph next to Files, with unresolved
/// links first. Label order is deterministic, independent of archive order.
pub(super) fn sidebar_dependencies(state: &State) -> Element<'_, Message> {
    let mut content = column![
        text("DEPENDENCY GRAPH").size(16),
        text("Shows files explicitly referenced by the selected CDF or material. Green means found; amber means multiple matches; red means missing.")
            .size(11).style(design::muted),
        design::button("Check selected asset")
            .on_press(Message::StartDependencyInspection),
    ].spacing(9);
    if let Some(graph) = &state.dependency_graph {
        content = content.push(
            text(format!(
                "{} file links · {} missing · {} ambiguous",
                graph.edges.len(), graph.missing, graph.ambiguous
            )).size(12).color(if graph.missing + graph.ambiguous == 0 { OK } else { WARN })
        );
        content = content.push(canvas_widget(DependencyCanvas {
            graph: Arc::new(graph.clone())
        }).height(Length::Fixed(200.0)).width(Fill));
        content = content.push(text("Referenced files (problems first)").size(14));
        let mut links: Vec<_> = graph.edges.iter().filter_map(|edge| {
            graph.nodes.get(edge.to).map(|node| (
                dependency_order(&node.status),
                edge.label.clone(),
                node.label.clone(),
                friendly_dependency_status(&node.status),
                mark(&node.status),
            ))
        }).collect();
        links.sort_by(|a,b| (a.0, a.2.to_ascii_lowercase(), a.1.to_ascii_lowercase())
            .cmp(&(b.0, b.2.to_ascii_lowercase(), b.1.to_ascii_lowercase())));
        for (_, relation, target, status, color) in links.iter().take(120) {
            content = content.push(
                column![
                    row![
                        text(*status).size(12).color(*color),
                        text(relation.as_str()).size(11).style(design::muted)
                    ].spacing(8),
                    text(target.as_str()).size(12),
                ].spacing(3)
            );
        }
        if links.len() > 120 {
            content = content.push(
                text(format!("Showing 120 of {} links.", links.len()))
                    .size(11).style(design::muted)
            );
        }
    } else {
        content = content.push(
            text("Choose a CDF or MTL in Files, then use Check selected asset to find linked files.")
                .size(12).style(design::muted)
        );
    }
    scrollable(container(content).padding(12).width(Fill))
        .height(Fill).width(Fill).into()
}

fn friendly_dependency_status(status: &DependencyStatus) -> &'static str {
    match status {
        DependencyStatus::Missing => "Missing",
        DependencyStatus::Ambiguous => "Multiple matches",
        DependencyStatus::Resolved => "Found",
        DependencyStatus::Source => "Starting file",
    }
}
fn dependency_order(status: &DependencyStatus) -> u8 {
    match status {
        DependencyStatus::Missing => 0,
        DependencyStatus::Ambiguous => 1,
        DependencyStatus::Resolved => 2,
        DependencyStatus::Source => 3,
    }
}

#[cfg(test)]
mod dock_tests {
    use super::*;

    #[test]
    fn beginner_dependency_labels_and_problem_order_are_stable() {
        assert_eq!(dependency_order(&DependencyStatus::Missing), 0);
        assert!(dependency_order(&DependencyStatus::Ambiguous) < dependency_order(&DependencyStatus::Resolved));
        assert_eq!(friendly_dependency_status(&DependencyStatus::Ambiguous), "Multiple matches");
        assert_eq!(friendly_dependency_status(&DependencyStatus::Resolved), "Found");
    }
}

fn tab_button(
    label: &'static str,
    tab: ViewerTab,
    selected: ViewerTab,
) -> iced::widget::Button<'static, Message> {
    design::button(label)
        .style(if tab == selected {
            design::primary
        } else {
            design::secondary
        })
        .on_press(Message::ViewerSelectTab(tab))
}

pub(super) fn view(state: &State) -> Element<'_, Message> {
    let tabs = row![
        tab_button("MODEL", ViewerTab::Model, state.viewer_tab),
        tab_button("UV0", ViewerTab::Uv, state.viewer_tab),
        tab_button("DEPENDENCIES", ViewerTab::Dependencies, state.viewer_tab),
        tab_button("COMPARE", ViewerTab::Compare, state.viewer_tab),
        space().width(Fill),
        design::button(if state.viewer_show_inspector {
            "Hide"
        } else {
            "Show"
        })
        .on_press(Message::ViewerToggleInspector),
    ]
    .spacing(7);
    if !state.viewer_show_inspector {
        return tabs.into();
    }
    let content: Element<'_, Message> = match state.viewer_tab {
        ViewerTab::Model => {
            let motion_details: Element<'_, Message> = if let Some(clip) = &state.native_caf {
                let mut tracks = column![
                    text(format!("Native CAF • {} controller(s) • {:.2}s • cyclic={} additive={}",
                        clip.controllers.len(), clip.duration_seconds(), clip.cyclic, clip.additive))
                        .size(12),
                    text(if let Some(skin) = &state.skin_preview {
                        format!(
                            "Bound to {} verified original bones and {} weighted vertices; CAF playback updates the wgpu mesh with CPU skinning.",
                            skin.original_bones(), skin.original_vertices()
                        )
                    } else {
                        "Controllers are source bone CRCs; select an exact compiled mesh and source skeleton to verify skin playback.".into()
                    })
                        .size(12).style(design::muted),
                ].spacing(5);
                for controller in clip.controllers.iter().take(14) {
                    tracks = tracks.push(
                        text(format!(
                            "CRC {:08X}: {} translation keys / {} rotation keys",
                            controller.bone_crc32,
                            controller.positions.len(),
                            controller.rotations.len()
                        ))
                        .size(12),
                    );
                }
                scrollable(tracks).height(Length::Fixed(145.0)).into()
            } else if let Some(animation) = &state.glb_motion {
                text(format!(
                    "Validated converter animation: {} clips and {} source nodes.                      Skeleton binding must be verified for actual skinned playback.",
                    animation.list_clips().len(), animation.node_names.len()
                )).size(12).style(design::muted).into()
            } else {
                text("Open a native CAF or compatible converted GLB to inspect source animation tracks.")
                    .size(12).style(design::muted).into()
            };
            let material_details: Element<'_, Message> = if let Some(report) =
                &state.material_report
            {
                let mut rows = column![
                    text(format!(
                        "Authored CryEngine material • {} slot(s), {} texture reference(s)",
                        report.slots,
                        report.textures.len()
                    ))
                    .size(12),
                ]
                .spacing(3);
                let verified_surfaces = state.cdf_surfaces.as_ref();
                for (label, roles, loaded, subset_count) in [
                    (
                        "Diffuse",
                        &["diffuse"][..],
                        state.diffuse.is_some(),
                        verified_surfaces.map_or(0, |parts| {
                            parts.iter().filter(|p| p.diffuse.is_some()).count()
                        }),
                    ),
                    (
                        "Normal",
                        &["normals", "normal", "bumpmap"][..],
                        state.normal_map.is_some(),
                        verified_surfaces.map_or(0, |parts| {
                            parts.iter().filter(|p| p.normal.is_some()).count()
                        }),
                    ),
                    (
                        "Emissive",
                        &["emittance", "emissive", "emission", "glow"][..],
                        state.emissive_map.is_some(),
                        verified_surfaces.map_or(0, |parts| {
                            parts.iter().filter(|p| p.emissive.is_some()).count()
                        }),
                    ),
                    (
                        "Specular",
                        &["specular", "specularcolor"][..],
                        state.specular_map.is_some(),
                        verified_surfaces.map_or(0, |parts| {
                            parts.iter().filter(|p| p.specular.is_some()).count()
                        }),
                    ),
                    (
                        "Gloss",
                        &["gloss", "glossiness", "smoothness"][..],
                        state.gloss_map.is_some(),
                        verified_surfaces.map_or(0, |parts| {
                            parts.iter().filter(|p| p.gloss.is_some()).count()
                        }),
                    ),
                ] {
                    let authored: Vec<_> = report
                        .textures
                        .iter()
                        .filter(|entry| roles.contains(&entry.role.as_str()))
                        .collect();
                    let exact = authored
                        .iter()
                        .filter(|entry| matches!(entry.resolution, AssetResolution::Found(_)))
                        .count();
                    let unresolved = authored.len() - exact;
                    rows = rows.push(
                        text(format!(
                            "{label}: {} GPU • {exact}/{} authored exact • {unresolved} unresolved",
                            if subset_count > 0 {
                                format!("{subset_count} original subset(s)")
                            } else if loaded {
                                "loaded single-map preview".into()
                            } else {
                                "not loaded".into()
                            },
                            authored.len()
                        ))
                        .size(12)
                        .style(if loaded || subset_count > 0 {
                            design::accent
                        } else {
                            design::muted
                        }),
                    );
                }
                if report.slots > 1 {
                    let note = match verified_surfaces {
                        Some(parts) => format!(
                            "{} original index range(s) mapped to authored GPU materials; any unresolved parts stay untextured.",
                            parts.len()
                        ),
                        None => "Waiting for original CryEngine mesh-subset assignments; never reuse another slot's texture.".into(),
                    };
                    rows = rows.push(text(note).size(12).style(design::muted));
                }
                scrollable(rows).height(Length::Fixed(165.0)).into()
            } else {
                text("Material channels are resolved automatically from the selected model's exact authored MTL. Missing or ambiguous assets are never substituted.")
                    .size(12).style(design::muted).into()
            };
            column![
                text("Viewer controls").size(14),
                text("Original CryEngine mesh and UV0 remain unmodified. Camera and panel layout can be saved.")
                    .size(12).style(design::muted),
                row![
                    design::button("Inspect CDF/MTL links").on_press(Message::StartDependencyInspection),
                    design::button(if state.viewer_wireframe { "Shaded view" } else { "Wireframe view" })
                        .on_press(Message::ViewerToggleWireframe),
                    design::button("Save workspace").on_press(Message::ViewerSaveLayout),
                ].spacing(8),
                motion_details,
                material_details,
            ].spacing(6).into()
        }
        ViewerTab::Uv => {
            if let Some(mesh) = &state.model {
                if let Some(uv) = &mesh.uv0 {
                    let tiled = uv
                        .iter()
                        .filter(|uv| {
                            uv.iter()
                                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                        })
                        .count();
                    column![
                        text(format!(
                            "Source UV0: {} coordinates, {} outside the 0–1 tile (preserved).",
                            uv.len(), tiled
                        )).size(12),
                        canvas_widget(uv_preview::SourceUv { mesh: mesh.clone() })
                            .width(Fill).height(Length::Fixed(260.0)),
                        text("Red wireframe displays verified UV0 source triangles. No destructive seam removal or UV reindexing.")
                            .size(12).style(design::muted),
                    ].spacing(9).into()
                } else {
                    text("No verified UV0 stream in this model; UVs will not be fabricated.")
                        .size(12)
                        .style(design::muted)
                        .into()
                }
            } else {
                text("Load a source CryEngine mesh before inspecting UV0.")
                    .size(12)
                    .style(design::muted)
                    .into()
            }
        }
        ViewerTab::Dependencies => {
            if let Some(report) = &state.dependency_graph {
                let mut nodes = column![].spacing(4);
                for edge in report.edges.iter().take(80) {
                    if let Some(node) = report.nodes.get(edge.to) {
                        nodes = nodes.push(
                            row![
                                text(edge.label.as_str())
                                    .size(12)
                                    .width(Length::FillPortion(2)),
                                text(node.label.as_str())
                                    .size(12)
                                    .width(Length::FillPortion(4)),
                                text(format!("{:?}", node.status))
                                    .size(12)
                                    .color(mark(&node.status))
                                    .width(Length::FillPortion(1)),
                            ]
                            .spacing(8),
                        );
                    }
                }
                column![
                    row![
                        text(format!(
                            "{} explicit references · {} missing · {} ambiguous",
                            report.edges.len(),
                            report.missing,
                            report.ambiguous
                        ))
                        .size(12),
                        design::button("Refresh").on_press(Message::StartDependencyInspection),
                    ]
                    .spacing(10),
                    canvas_widget(DependencyCanvas {
                        graph: Arc::new(report.clone())
                    })
                    .height(Length::Fixed(156.0))
                    .width(Fill),
                    scrollable(nodes).height(Length::Fixed(105.0)),
                ]
                .spacing(8)
                .into()
            } else {
                column![
                    text("Inspect a selected CDF or MTL to see the actual authored references.")
                        .size(12)
                        .style(design::muted),
                    design::button("Build dependency graph")
                        .on_press(Message::StartDependencyInspection),
                ]
                .spacing(7)
                .into()
            }
        }
        ViewerTab::Compare => {
            let mut rows = column![
                row![
                    design::text_input("Original extracted asset...", &state.compare_original)
                        .on_input(Message::CompareOriginalChanged)
                        .width(Fill),
                    design::text_input("Modified working copy...", &state.compare_modified)
                        .on_input(Message::CompareModifiedChanged)
                        .width(Fill),
                    design::button("Compare").on_press(Message::StartAssetCompare),
                ]
                .spacing(7),
            ]
            .spacing(8);
            if let Some(result) = &state.comparison {
                rows = rows.push(
                    text(if result.identical {
                        "Identical contents"
                    } else {
                        "Changed contents (SHA-256 verified)"
                    })
                    .size(14)
                    .color(if result.identical { OK } else { WARN }),
                );
                rows = rows.push(
                    row![
                        text(format!("Original: {} bytes", result.original.bytes)).size(12),
                        text(format!("Modified: {} bytes", result.modified.bytes)).size(12),
                        text(match result.same_dds_dimensions {
                            Some(true) => "Same DDS dimensions",
                            Some(false) => "DDS dimensions differ",
                            None => "No paired DDS dimensions",
                        })
                        .size(12)
                    ]
                    .spacing(16),
                );
                rows = rows.push(text("Read-only comparison. Visual pixel and geometry diffs are not implemented yet.")
                    .size(12).style(design::muted));
            } else {
                rows = rows.push(text("Files are compared without modifying originals, using full content hashes and DDS metadata.")
                    .size(12).style(design::muted));
            }
            rows.into()
        }
    };
    column![tabs, container(content).width(Fill).padding(8)]
        .spacing(6)
        .into()
}
