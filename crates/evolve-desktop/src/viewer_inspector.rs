//! Interactive viewer inspector: source-grounded dependency diagram and
//! original-vs-modified diagnostics. No graph edges are inferred from names.
use super::{Message, State, ViewerTab, uv_preview};
use crate::design;
use evolve_core::{DependencyGraph, DependencyStatus};
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
                    text("Controllers are source bone CRCs, not yet associated with a validated skin hierarchy.")
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
