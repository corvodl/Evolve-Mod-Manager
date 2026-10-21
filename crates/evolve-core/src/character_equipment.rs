//! Read-only options derived from an authored character CDF and exact indexed
//! render siblings. 2012 Scorpid's independent L/R equipment is a common
//! example; never swap stages, characters, or attachment kinds by basename.
use crate::cdf::{AssetResolution, AttachmentKind, CdfDefinition, resolve_virtual};
use crate::inventory::Inventory;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

pub type EquipmentOverrides = BTreeMap<String, String>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EquipmentChoice {
    pub label: String,
    pub binding: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EquipmentSlot {
    pub name: String,
    pub original: String,
    pub choices: Vec<EquipmentChoice>,
}

/// Original authoring families such as Scorpid3_L_Basher and
/// Scorpid3_L_Shield share the same source skeleton and the left-arm slot.
/// The right-arm family remains independent. Other CDF attachments are never
/// silently classified as an arm based on a shared character directory.
fn family(binding: &str) -> Option<(&str, &str, &str)> {
    let (parent, filename) = binding.rsplit_once('/')?;
    let (stem, ext) = filename.rsplit_once('.')?;
    if !ext.eq_ignore_ascii_case("chr") {
        return None;
    }
    let lower = stem.to_ascii_lowercase();
    for marker in ["_l_", "_r_"] {
        if let Some(index) = lower.find(marker) {
            let head = &stem[..index + marker.len()];
            if index > 0 && !stem[index + marker.len()..].is_empty() {
                return Some((parent, head, ext));
            }
        }
    }
    None
}

pub fn discover_equipment(cdf: &CdfDefinition, inventory: &Inventory) -> Vec<EquipmentSlot> {
    let mut result = Vec::new();
    for attachment in &cdf.attachments {
        if !matches!(
            attachment.kind,
            AttachmentKind::Skin | AttachmentKind::UniqueSkin
        ) {
            continue;
        }
        let Some((folder, prefix, extension)) = family(&attachment.binding) else {
            continue;
        };
        let Some((_, original_file)) = attachment.binding.rsplit_once('/') else {
            continue;
        };
        let mut found = Vec::new();
        let mut seen = HashSet::new();
        for candidate in &inventory.entries {
            let Some(file) = candidate.relative_path.rsplit('/').next() else {
                continue;
            };
            if !file
                .to_ascii_lowercase()
                .starts_with(&prefix.to_ascii_lowercase())
                || !file
                    .to_ascii_lowercase()
                    .ends_with(&format!(".{}", extension.to_ascii_lowercase()))
            {
                continue;
            }
            let Some((stem, _)) = file.rsplit_once('.') else {
                continue;
            };
            let label = &stem[prefix.len()..];
            if label.is_empty() || label.to_ascii_lowercase().contains("collision") {
                continue;
            }
            let virtual_path = format!("{folder}/{file}");
            if !matches!(
                resolve_virtual(inventory, &virtual_path),
                AssetResolution::Found(_)
            ) || !seen.insert(virtual_path.to_ascii_lowercase())
            {
                continue;
            }
            found.push(EquipmentChoice {
                label: label.to_owned(),
                binding: virtual_path,
            });
        }
        // Always include the exact CDF binding, even if indexed paths use
        // another case; the original cannot be dropped by the UI.
        if !found
            .iter()
            .any(|v| v.binding.eq_ignore_ascii_case(&attachment.binding))
            && matches!(
                resolve_virtual(inventory, &attachment.binding),
                AssetResolution::Found(_)
            )
        {
            found.push(EquipmentChoice {
                label: original_file
                    .rsplit_once('.')
                    .map(|(stem, _)| &stem[prefix.len()..])
                    .unwrap_or(original_file)
                    .into(),
                binding: attachment.binding.clone(),
            });
        }
        if found.len() < 2 {
            continue;
        }
        found.sort_by_key(|a| a.label.to_ascii_lowercase());
        result.push(EquipmentSlot {
            name: attachment.name.clone(),
            original: attachment.binding.clone(),
            choices: found,
        });
    }
    result
}

/// This is also enforced in the assembly worker, never only by UI buttons.
pub fn validated_equipment_overrides(
    cdf: &CdfDefinition,
    inventory: &Inventory,
    requested: &EquipmentOverrides,
) -> Result<EquipmentOverrides, String> {
    let slots = discover_equipment(cdf, inventory);
    let mut result = BTreeMap::new();
    for (slot, binding) in requested {
        let matches = slots
            .iter()
            .filter(|candidate| candidate.name == *slot)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(format!("Missing or ambiguous equipment attachment: {slot}"));
        }
        let choices = matches[0]
            .choices
            .iter()
            .filter(|choice| choice.binding.eq_ignore_ascii_case(binding))
            .collect::<Vec<_>>();
        if choices.len() != 1 {
            return Err(format!(
                "Unverified equipment binding for {slot}: {binding}"
            ));
        }
        result.insert(slot.clone(), choices[0].binding.clone());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdf::parse_cdf;
    use crate::inventory::{AssetEntry, AssetKind};

    #[test]
    fn historical_scorpid_arms_swap_independently_without_cross_stage_assets() {
        let xml = br#"<CharacterDefinition>
            <Model File="objects/characters/monsters/scorpid_2/scorpid3_skeleton.chr"/>
            <AttachmentList>
                <Attachment AName="basher_l" Type="CA_SKIN" Binding="objects/characters/monsters/scorpid_2/scorpid3_l_basher.chr"/>
                <Attachment AName="basher_r" Type="CA_SKIN" Binding="objects/characters/monsters/scorpid_2/scorpid3_r_basher.chr"/>
            </AttachmentList></CharacterDefinition>"#;
        let cdf = parse_cdf(xml).unwrap();
        let files = [
            "Scorpid3_L_Basher.chr",
            "Scorpid3_L_Shield.chr",
            "Scorpid3_L_Poison.chr",
            "Scorpid3_R_Basher.chr",
            "Scorpid3_R_Shield.chr",
            "Scorpid3_R_Poison.chr",
            "Scorpid2_L_Shield.chr",
            "Scorpid3_L_Shield_Collision.cgf",
        ];
        let inventory = Inventory {
            schema: 1,
            root: "C:/extracted/characters".into(),
            entries: files
                .iter()
                .map(|file| AssetEntry {
                    relative_path: format!("monsters/Scorpid_2/{file}"),
                    byte_size: 20,
                    kind: AssetKind::Mesh,
                })
                .collect(),
            total_bytes: 160,
            skipped_symlinks: 0,
        };
        let slots = discover_equipment(&cdf, &inventory);
        assert_eq!(slots.len(), 2);
        assert!(slots.iter().all(|s| s.choices.len() == 3));
        let mut overrides = EquipmentOverrides::new();
        overrides.insert(
            "basher_l".into(),
            slots[0]
                .choices
                .iter()
                .find(|x| x.label.eq_ignore_ascii_case("shield"))
                .unwrap()
                .binding
                .clone(),
        );
        overrides.insert(
            "basher_r".into(),
            slots[1]
                .choices
                .iter()
                .find(|x| x.label.eq_ignore_ascii_case("poison"))
                .unwrap()
                .binding
                .clone(),
        );
        assert_eq!(
            validated_equipment_overrides(&cdf, &inventory, &overrides)
                .unwrap()
                .len(),
            2
        );
        overrides.insert(
            "basher_l".into(),
            "objects/characters/monsters/scorpid_2/scorpid2_l_shield.chr".into(),
        );
        assert!(validated_equipment_overrides(&cdf, &inventory, &overrides).is_err());
    }
}
