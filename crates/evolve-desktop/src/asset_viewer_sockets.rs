//! Asset Viewer attachment sockets and DBA source provenance.
//!
//! Everything shown here is data the viewer already verified:
//!
//! * sockets come from the assembled CDF [`AssemblyReport`] and the authored
//!   [`CdfDefinition`] attachment entries (name, exact bone socket, authored
//!   position and WXYZ quaternion);
//! * DBA rows come from the character animation catalog in `State::auto_clips`,
//!   which is built from that character's own CHRPARAMS declarations.
//!
//! No socket is inferred from a part name, and no DBA is matched by file name,
//! build number or CRC. Opening a DBA reuses the existing `ViewerAutoClip`
//! operation, so the loader resolves the exact declared virtual path and keeps
//! refusing a clip that appears in more than one declared database.
//!
//! The panels are intentionally read-only: the CDF assembler applies the
//! authored transforms verbatim, and there is no verified per-socket override
//! in the core assembly API yet. Presenting editable offsets without that
//! plumbing would misrepresent what the viewport shows.

use crate::auto_resources::AutoClip;
use crate::design;
use crate::{Message, State};
use evolve_core::cdf::AttachmentKind;
use evolve_core::{AssemblyReport, CdfDefinition};
use iced::widget::{column, row, text};
use iced::{Color, Element};

const WARN: Color = Color::from_rgb(0.93, 0.70, 0.32);

/// One authored rigid socket and the assembled parts mounted to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SocketMount {
    pub socket: String,
    pub parts: Vec<String>,
}

/// Socket grouping for the assembled preview. Parts without a bone socket use
/// their authored skin binding instead and are kept separate rather than being
/// reported as a missing or broken socket.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SocketInventory {
    pub mounts: Vec<SocketMount>,
    pub skin_bound_parts: Vec<String>,
}

/// Group assembled geometry by its exact authored bone socket.
pub(crate) fn socket_inventory(report: &AssemblyReport) -> SocketInventory {
    let mut inventory = SocketInventory::default();
    for part in &report.geometry_parts {
        let socket = part
            .bone_socket
            .as_deref()
            .map(str::trim)
            .filter(|socket| !socket.is_empty());
        match socket {
            Some(socket) => {
                if let Some(mount) = inventory
                    .mounts
                    .iter_mut()
                    .find(|mount| mount.socket.eq_ignore_ascii_case(socket))
                {
                    mount.parts.push(part.name.clone());
                } else {
                    inventory.mounts.push(SocketMount {
                        socket: socket.to_owned(),
                        parts: vec![part.name.clone()],
                    });
                }
            }
            None => inventory.skin_bound_parts.push(part.name.clone()),
        }
    }
    inventory
}

/// The authored transform of one CA_BONE attachment.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AuthoredSocketTransform {
    pub part: String,
    pub socket: String,
    /// CDF-relative translation; `None` means the CDF omitted it (origin).
    pub position: Option<[f32; 3]>,
    /// Authored CryEngine quaternion in WXYZ order, if present.
    pub rotation: Option<[f32; 4]>,
}

/// Authored socket attachments of an inspected CDF, regardless of whether the
/// preview is currently assembled. Real CDF data only; never synthesized.
pub(crate) fn authored_sockets(definition: &CdfDefinition) -> Vec<AuthoredSocketTransform> {
    definition
        .attachments
        .iter()
        .filter(|attachment| attachment.kind == AttachmentKind::Bone)
        .filter_map(|attachment| {
            let socket = attachment
                .bone_name
                .as_deref()
                .map(str::trim)
                .filter(|socket| !socket.is_empty())?;
            Some(AuthoredSocketTransform {
                part: attachment.name.clone(),
                socket: socket.to_owned(),
                position: attachment.position,
                rotation: attachment.rotation,
            })
        })
        .collect()
}

/// True only when the inspected definition is demonstrably the character that
/// produced `report`: same authored skeleton and every rendered rigid socket
/// maps back to a CA_BONE attachment with the same name and socket.
///
/// `State::character` is populated by the explicit CDF inspector and is not
/// tied to the preview selection by the application yet, so this guard prevents
/// showing another character's offsets as if they belonged to the viewport.
pub(crate) fn definition_matches_report(
    definition: &CdfDefinition,
    report: &AssemblyReport,
) -> bool {
    if !definition
        .skeleton
        .trim()
        .eq_ignore_ascii_case(report.cdf_skeleton.trim())
    {
        return false;
    }
    let mut saw_socket = false;
    for part in &report.geometry_parts {
        let Some(socket) = part
            .bone_socket
            .as_deref()
            .map(str::trim)
            .filter(|socket| !socket.is_empty())
        else {
            continue;
        };
        saw_socket = true;
        let mapped = definition.attachments.iter().any(|attachment| {
            if attachment.kind != AttachmentKind::Bone
                || !attachment.name.eq_ignore_ascii_case(&part.name)
                || !attachment
                    .bone_name
                    .as_deref()
                    .is_some_and(|bone| bone.trim().eq_ignore_ascii_case(socket))
            {
                return false;
            }
            // When the assembler resolved the authored binding, require it to
            // be the same virtual file as this definition's attachment. This
            // rejects another CDF variant that happens to share a skeleton and
            // socket names.
            part.binding.as_ref().is_none_or(|resolved| {
                let resolved = resolved.to_string_lossy().replace('\\', "/");
                resolved
                    .to_ascii_lowercase()
                    .ends_with(&attachment.binding.trim().to_ascii_lowercase())
            })
        });
        if !mapped {
            return false;
        }
    }
    saw_socket
}

/// Authored transforms only when the inspected definition matches the preview.
pub(crate) fn authored_socket_transforms(
    definition: &CdfDefinition,
    report: &AssemblyReport,
) -> Vec<AuthoredSocketTransform> {
    if definition_matches_report(definition, report) {
        authored_sockets(definition)
    } else {
        Vec::new()
    }
}

/// One resolved DBA declaration. `clip_index` is the exact animation-catalog
/// entry that the existing `ViewerAutoClip` operation must open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DbaSource {
    pub declared: String,
    pub virtual_path: String,
    pub clip_index: usize,
}

/// DBA entries of the character catalog, preserving the declared name and the
/// exact resolved virtual path. Duplicate (declared, path) rows are collapsed;
/// distinct paths with the same declared name stay visible because that is the
/// ambiguity the user has to resolve.
pub(crate) fn declared_dba_sources(clips: &[AutoClip]) -> Vec<DbaSource> {
    let mut sources: Vec<DbaSource> = Vec::new();
    for (index, clip) in clips.iter().enumerate() {
        if !clip.kind.eq_ignore_ascii_case("dba") {
            continue;
        }
        let virtual_path = clip.virtual_path.trim();
        if virtual_path.is_empty() {
            continue;
        }
        let declared = clip.label.trim();
        let declared = if declared.is_empty() { "DBA" } else { declared };
        if sources.iter().any(|seen| {
            seen.virtual_path.eq_ignore_ascii_case(virtual_path)
                && seen.declared.eq_ignore_ascii_case(declared)
        }) {
            continue;
        }
        sources.push(DbaSource {
            declared: declared.to_owned(),
            virtual_path: virtual_path.to_owned(),
            clip_index: index,
        });
    }
    sources
}

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
}

/// Reader used by the Model Options tab.
pub(crate) fn attachment_socket_panel(state: &State) -> Element<'_, Message> {
    let mut content = column![
        text("ATTACHMENT SOCKETS").size(13),
        text(
            "Authored CDF socket assignments and transforms. The assembled preview uses these exact values; no socket or offset is estimated from a part name."
        )
        .size(11)
        .style(design::muted),
    ]
    .spacing(5);

    let Some(report) = state.cdf_report.as_deref() else {
        if let Some(character) = state.character.as_ref() {
            let sockets = authored_sockets(character);
            if sockets.is_empty() {
                content = content.push(
                    text("This inspected CDF declares no rigid bone-socket attachment.")
                        .size(11)
                        .style(design::muted),
                );
            } else {
                content = content.push(
                    text("AUTHORED SOCKETS · preview not assembled")
                        .size(10)
                        .style(design::muted),
                );
                content = content.push(authored_socket_rows(&sockets));
            }
        } else {
            content = content.push(
                text(
                    "Build a CDF preview or use Model → Inspect CDF to read its authored sockets.",
                )
                .size(11)
                .style(design::muted),
            );
        }
        return content.into();
    };

    let inventory = socket_inventory(report);
    if inventory.mounts.is_empty() {
        content = content.push(
            text("No rendered part is mounted to a rigid bone socket.")
                .size(11)
                .style(design::muted),
        );
    }
    for mount in &inventory.mounts {
        content = content.push(text(format!("{} · {}", mount.socket, mount.parts.len())).size(12));
        content = content.push(text(mount.parts.join(", ")).size(11).style(design::muted));
    }
    if !inventory.skin_bound_parts.is_empty() {
        content = content.push(
            text(format!(
                "{} skin-bound part(s): {}",
                inventory.skin_bound_parts.len(),
                inventory.skin_bound_parts.join(", ")
            ))
            .size(11)
            .style(design::muted),
        );
    }
    content = content.push(
        text(match report.verified_bones {
            Some(count) => format!("{count} compiled bone(s) verified"),
            None => "No verified compiled skeleton; rigid sockets are disabled".to_owned(),
        })
        .size(11)
        .style(design::muted),
    );

    if let Some(character) = state.character.as_ref() {
        let transforms = authored_socket_transforms(character, report);
        if transforms.is_empty() {
            if !inventory.mounts.is_empty() {
                content = content.push(
                    text("The inspected CDF does not match this preview, so its authored offsets are hidden.")
                        .size(10)
                        .style(design::muted),
                );
            }
        } else {
            content = content.push(
                text("AUTHORED SOCKET TRANSFORMS")
                    .size(10)
                    .style(design::muted),
            );
            content = content.push(authored_socket_rows(&transforms));
        }
    } else {
        content = content.push(
            text(
                "Use Model → Inspect CDF on this same definition to display its authored offsets.",
            )
            .size(10)
            .style(design::muted),
        );
    }
    content.into()
}

fn authored_socket_rows(transforms: &[AuthoredSocketTransform]) -> Element<'static, Message> {
    let mut rows = column![].spacing(3);
    for transform in transforms {
        let position = transform.position.map_or_else(
            || "position: origin (not authored)".to_owned(),
            |position| {
                format!(
                    "position: {:.3}, {:.3}, {:.3}",
                    position[0], position[1], position[2]
                )
            },
        );
        let rotation = transform.rotation.map_or_else(
            || "rotation: none".to_owned(),
            |rotation| {
                format!(
                    "quaternion WXYZ: {:.3}, {:.3}, {:.3}, {:.3}",
                    rotation[0], rotation[1], rotation[2], rotation[3]
                )
            },
        );
        rows = rows.push(text(format!("{} → {}", transform.part, transform.socket)).size(11));
        rows = rows.push(
            text(format!("{position} · {rotation}"))
                .size(10)
                .style(design::muted),
        );
    }
    rows.into()
}

/// Reader used by the Animations tab.
pub(crate) fn dba_source_panel(state: &State) -> Element<'_, Message> {
    let sources = declared_dba_sources(&state.auto_clips);
    let mut content = column![
        text("DBA SOURCES").size(13),
        text(
            "Animation databases declared by this character's own CHRPARAMS and resolved by exact virtual path. The viewer never chooses a database by file name, CRC or build number."
        )
        .size(11)
        .style(design::muted),
    ]
    .spacing(5);

    if sources.is_empty() {
        content = content.push(
            text("No declared DBA is present in the resolved catalog; clips are loose CAF files or none.")
                .size(11)
                .style(design::muted),
        );
    } else {
        let mut shared_names: Vec<(&str, usize)> = Vec::new();
        for source in &sources {
            if let Some(entry) = shared_names
                .iter_mut()
                .find(|(name, _)| name.eq_ignore_ascii_case(&source.declared))
            {
                entry.1 += 1;
            } else {
                shared_names.push((source.declared.as_str(), 1));
            }
        }
        for (name, count) in shared_names.iter().filter(|(_, count)| *count > 1) {
            content = content.push(
                text(format!(
                    "{count} DBA files share the declared name \"{name}\". Open the exact copy you need; a clip found in more than one declared database is refused as ambiguous."
                ))
                .size(11)
                .color(WARN),
            );
        }
        content = content.push(
            text(format!("{} declared DBA file(s)", sources.len()))
                .size(11)
                .style(design::muted),
        );

        let ready =
            !state.busy && state.inventory.is_some() && !state.converter_path.trim().is_empty();
        for source in &sources {
            content = content.push(
                column![
                    text(file_name(&source.virtual_path).to_owned()).size(12),
                    text(format!("{} · {}", source.declared, source.virtual_path))
                        .size(10)
                        .style(design::muted),
                    row![
                        design::button("Verify DBA tracks").on_press_maybe(
                            ready.then_some(Message::ViewerAutoClip(source.clip_index))
                        ),
                        text(if ready {
                            "Converter reads this exact file"
                        } else {
                            "Index the build and configure Cryengine-Converter first"
                        })
                        .size(10)
                        .style(design::muted),
                    ]
                    .spacing(8)
                    .align_y(iced::Center),
                ]
                .spacing(3),
            );
        }
        content = content.push(
            text("Converting stages verified tracks in a temporary folder; the original DBA is never edited.")
                .size(10)
                .style(design::muted),
        );
    }

    if state.converted_dba_cache.is_some() {
        content = content.push(
            text(format!(
                "TEMPORARY CONVERTED DBA · {} verified track(s)",
                state.converted_glbs.len()
            ))
            .size(11),
        );
        content = content.push(
            text("These converter outputs are deleted when this preview closes.")
                .size(10)
                .style(design::muted),
        );
    }
    content.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn report(parts: &[(&str, Option<&str>)]) -> AssemblyReport {
        AssemblyReport {
            cdf_skeleton: "characters/hunters/merc_tech/tech_3p_skeleton.chr".into(),
            selected_skin_set: None,
            original_parts: parts.iter().map(|(name, _)| (*name).to_owned()).collect(),
            vertices: 3 * parts.len(),
            triangles: parts.len(),
            original_uv0_complete: true,
            original_normals_complete: true,
            skeletal_animation_enabled: false,
            source_parts: vec![],
            geometry_parts: parts
                .iter()
                .map(|(name, socket)| evolve_core::AssemblyGeometry {
                    name: (*name).to_owned(),
                    source: PathBuf::from("characters/hunters/merc_tech/tech_3p.skinm"),
                    vertex_first: 0,
                    vertex_count: 3,
                    bone_socket: socket.map(str::to_owned),
                    binding: None,
                    verified_skin_weights: socket.is_none(),
                })
                .collect(),
            verified_bones: Some(24),
            skeleton_source: None,
            warnings: vec![],
        }
    }

    fn attachment(
        name: &str,
        kind: AttachmentKind,
        socket: Option<&str>,
        position: Option<[f32; 3]>,
        rotation: Option<[f32; 4]>,
    ) -> evolve_core::CdfAttachment {
        evolve_core::CdfAttachment {
            name: name.into(),
            binding: format!("characters/hunters/merc_tech/{name}.skin"),
            kind,
            bone_name: socket.map(str::to_owned),
            material: None,
            position,
            rotation,
        }
    }

    fn definition(attachments: Vec<evolve_core::CdfAttachment>) -> CdfDefinition {
        CdfDefinition {
            schema: 2,
            skeleton: "characters/hunters/merc_tech/tech_3p_skeleton.chr".into(),
            attachments,
            skin_sets: BTreeMap::new(),
        }
    }

    fn clip(label: &str, kind: &str, virtual_path: &str) -> AutoClip {
        AutoClip {
            label: label.into(),
            kind: kind.into(),
            virtual_path: virtual_path.into(),
            source: None,
        }
    }

    #[test]
    fn socket_inventory_groups_case_insensitively_and_keeps_skin_parts_separate() {
        let inventory = socket_inventory(&report(&[
            ("arm_eq", Some("Bip01 R Hand")),
            ("scope", Some("bip01 r hand")),
            ("body", None),
        ]));
        assert_eq!(inventory.mounts.len(), 1);
        assert_eq!(inventory.mounts[0].socket, "Bip01 R Hand");
        assert_eq!(inventory.mounts[0].parts, vec!["arm_eq", "scope"]);
        assert_eq!(inventory.skin_bound_parts, vec!["body"]);
    }

    #[test]
    fn authored_socket_rows_only_include_real_ca_bone_sockets() {
        let definition = definition(vec![
            attachment("body", AttachmentKind::Skin, None, None, None),
            attachment(
                "arm_eq",
                AttachmentKind::Bone,
                Some("  Bip01 R Hand  "),
                Some([1.0, 2.0, 3.0]),
                Some([1.0, 0.0, 0.0, 0.0]),
            ),
            attachment(
                "empty_socket",
                AttachmentKind::Bone,
                Some("   "),
                None,
                None,
            ),
        ]);
        let sockets = authored_sockets(&definition);
        assert_eq!(sockets.len(), 1);
        assert_eq!(sockets[0].part, "arm_eq");
        assert_eq!(sockets[0].socket, "Bip01 R Hand");
        assert_eq!(sockets[0].position, Some([1.0, 2.0, 3.0]));
    }

    #[test]
    fn authored_transforms_require_the_same_cdf_as_the_assembled_preview() {
        let authored = definition(vec![attachment(
            "arm_eq",
            AttachmentKind::Bone,
            Some("Bip01 R Hand"),
            Some([0.5, 0.0, -2.0]),
            None,
        )]);
        let assembled = report(&[("arm_eq", Some("Bip01 R Hand"))]);
        let transforms = authored_socket_transforms(&authored, &assembled);
        assert_eq!(transforms.len(), 1);
        assert_eq!(transforms[0].part, "arm_eq");

        let other_skeleton = definition(vec![attachment(
            "arm_eq",
            AttachmentKind::Bone,
            Some("Bip01 R Hand"),
            Some([0.5, 0.0, -2.0]),
            None,
        )]);
        let other_skeleton = CdfDefinition {
            skeleton: "characters/hunters/other/other.chr".into(),
            ..other_skeleton
        };
        assert!(authored_socket_transforms(&other_skeleton, &assembled).is_empty());

        let other_socket = definition(vec![attachment(
            "arm_eq",
            AttachmentKind::Bone,
            Some("Bip01 L Hand"),
            None,
            None,
        )]);
        assert!(authored_socket_transforms(&other_socket, &assembled).is_empty());
    }

    #[test]
    fn another_cdf_variant_with_the_same_socket_but_different_binding_is_rejected() {
        let authored = definition(vec![attachment(
            "arm_eq",
            AttachmentKind::Bone,
            Some("Bip01 R Hand"),
            Some([0.5, 0.0, -2.0]),
            None,
        )]);
        let mut assembled = report(&[("arm_eq", Some("Bip01 R Hand"))]);
        assembled.geometry_parts[0].binding = Some(PathBuf::from(
            "D:/extracted/objects/characters/hunters/merc_tech/arm_eq_1p.skin",
        ));
        assert!(authored_socket_transforms(&authored, &assembled).is_empty());
        assembled.geometry_parts[0].binding = Some(PathBuf::from(
            "D:/extracted/characters/hunters/merc_tech/arm_eq.skin",
        ));
        assert_eq!(authored_socket_transforms(&authored, &assembled).len(), 1);
    }

    #[test]
    fn dba_sources_preserve_exact_declared_paths_and_catalog_indexes() {
        let clips = vec![
            clip(
                "$TracksDatabase",
                "dba",
                "characters/hunters/merc_tech/tech_3p.dba",
            ),
            clip(
                "stand_idle_loop",
                "caf",
                "characters/hunters/merc_tech/idle.caf",
            ),
            clip(
                "$TracksDatabase",
                "dba",
                "characters/hunters/merc_tech/tech_1p.dba",
            ),
            clip(
                "$TracksDatabase",
                "dba",
                "CHARACTERS/HUNTERS/MERC_TECH/TECH_3P.DBA",
            ),
            clip("$TracksDatabase", "dba", "   "),
        ];
        let sources = declared_dba_sources(&clips);
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].clip_index, 0);
        assert_eq!(
            sources[0].virtual_path,
            "characters/hunters/merc_tech/tech_3p.dba"
        );
        assert_eq!(sources[1].clip_index, 2);
        assert_eq!(
            sources[1].virtual_path,
            "characters/hunters/merc_tech/tech_1p.dba"
        );
        // No basename-only source can appear: the catalog has no CAF and no
        // empty path is turned into a fake DBA.
        assert!(
            sources
                .iter()
                .all(|source| source.virtual_path.ends_with(".dba"))
        );
    }

    #[test]
    fn no_declared_catalog_means_no_invented_dba() {
        assert!(declared_dba_sources(&[]).is_empty());
        assert!(declared_dba_sources(&[clip("idle", "caf", "animations/idle.caf")]).is_empty());
    }

    #[test]
    fn dba_display_uses_the_last_virtual_path_component() {
        assert_eq!(
            file_name("characters/hunters/merc_tech/tech_3p.dba"),
            "tech_3p.dba"
        );
        assert_eq!(
            file_name("characters\\hunters\\merc_tech\\tech_3p.dba"),
            "tech_3p.dba"
        );
        assert_eq!(file_name("tech_3p.dba"), "tech_3p.dba");
    }
}
