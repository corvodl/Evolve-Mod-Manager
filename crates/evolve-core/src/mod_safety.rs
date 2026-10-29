//! Conservative presentation classification. Never authorizes an online launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModKind {
    Unchanged,
    CosmeticCandidate,
    Gameplay,
    NeedsReview,
}
impl ModKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unchanged => "Unchanged",
            Self::CosmeticCandidate => "Cosmetic candidate",
            Self::Gameplay => "Gameplay · offline only",
            Self::NeedsReview => "Needs review · offline only",
        }
    }
}

/// Stable virtual paths, independent of archive name and import directory.
const GAMEPLAY_ROOTS: &[&str] = &[
    "scripts/",
    "entities/",
    "libs/actor/",
    "libs/abilities/",
    "libs/items/",
    "libs/effectsets/",
    "libs/gamerules/",
    "libs/params/",
    "libs/characters/",
    "libs/config/",
    "libs/ai/",
    "libs/entityarchetypes/",
    "libs/progression/",
    "libs/player/",
    "libs/gametokens/",
    "libs/contextualcombat/",
    "libs/zookeeper/",
];
/// Extensions accepted by the Gameplay values editor. `.effectset` documents
/// describe perk/effect data and therefore carry gameplay semantics even when
/// their authored values contain variant pipe lists.
const GAMEPLAY_EXTENSIONS: &[&str] = &[".xml", ".actorparams", ".effectset"];
pub fn gameplay_document(path: &str) -> bool {
    let p = path.replace('\\', "/").to_ascii_lowercase();
    GAMEPLAY_EXTENSIONS.iter().any(|ext| p.ends_with(ext))
        && GAMEPLAY_ROOTS.iter().any(|root| p.starts_with(root))
}
pub fn model_document(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    [".cdf", ".cgf", ".chr", ".skin"]
        .iter()
        .any(|ext| p.ends_with(ext))
}
pub fn member_kind(path: &str) -> ModKind {
    let p = path.replace('\\', "/").to_ascii_lowercase();
    if p.is_empty()
        || p.starts_with('/')
        || p.contains(':')
        || p.split('/').any(|s| s.is_empty() || s == "." || s == "..")
    {
        return ModKind::NeedsReview;
    }
    // Deny rules take precedence even for a cosmetic-looking suffix.
    if GAMEPLAY_ROOTS.iter().any(|root| p.starts_with(root))
        || p.ends_with(".actorparams")
        || p.ends_with(".chrparams")
        || [".lua", ".cfg", ".ent", ".dll", ".exe", ".effectset"]
            .iter()
            .any(|ext| p.ends_with(ext))
    {
        return ModKind::Gameplay;
    }
    let dds = p.rsplit_once(".dds").is_some_and(|(_, tail)| {
        tail.is_empty()
            || tail
                .strip_prefix('.')
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    });
    if dds
        || model_document(&p)
        || [
            ".mtl", ".png", ".tga", ".jpg", ".wav", ".ogg", ".fsb", ".gfx", ".swf", ".usm",
        ]
        .iter()
        .any(|ext| p.ends_with(ext))
        || (p.starts_with("libs/ui/") && p.ends_with(".xml"))
    {
        ModKind::CosmeticCandidate
    } else {
        ModKind::NeedsReview
    }
}
/// `complete` means the whole changed-member inventory was checked, not a sample.
pub fn classify<'a>(paths: impl IntoIterator<Item = &'a str>, complete: bool) -> ModKind {
    let mut result = ModKind::Unchanged;
    for path in paths {
        match member_kind(path) {
            ModKind::Gameplay => return ModKind::Gameplay,
            ModKind::NeedsReview => result = ModKind::NeedsReview,
            ModKind::CosmeticCandidate if result == ModKind::Unchanged => {
                result = ModKind::CosmeticCandidate
            }
            _ => (),
        }
    }
    if complete {
        result
    } else {
        ModKind::NeedsReview
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixed_and_disguised_gameplay_stay_offline() {
        assert_eq!(
            classify(
                ["objects/a.dds", "Scripts/Entities/Abilities/XML/a.xml"],
                true
            ),
            ModKind::Gameplay
        );
        assert_eq!(member_kind("scripts/entities/a.dds"), ModKind::Gameplay);
        assert_eq!(member_kind("objects/a.chrparams"), ModKind::Gameplay);
    }
    #[test]
    fn incomplete_unknown_and_traversal_are_not_cosmetic() {
        for p in ["../a.dds", "C:/a.dds", "objects/a.bin", "a.dds.1.exe"] {
            assert_ne!(member_kind(p), ModKind::CosmeticCandidate);
        }
        assert_eq!(classify(["objects/a.dds"], false), ModKind::NeedsReview);
        assert_eq!(
            classify(["objects/a.dds", "unknown.xml"], true),
            ModKind::NeedsReview
        );
        assert_eq!(classify([], true), ModKind::Unchanged);
    }
    #[test]
    fn stage_two_actor_and_ability_paths_are_gameplay() {
        for p in [
            "libs/Actor/BaseMonster.actorparams",
            "libs/Abilities/Goliath/GoliathCharge.xml",
            "libs/Items/weapon.xml",
        ] {
            assert!(gameplay_document(p));
            assert_eq!(member_kind(p), ModKind::Gameplay);
        }
    }
    #[test]
    fn cosmetic_candidates_include_split_textures_and_models() {
        for p in [
            "Objects/A.DDS.10",
            "Objects/A.skin",
            "sounds/a.fsb",
            "Libs/UI/menu.gfx",
        ] {
            assert_eq!(member_kind(p), ModKind::CosmeticCandidate);
        }
    }
    #[test]
    fn effectset_perks_and_effect_sets_are_editable_gameplay() {
        for p in [
            "Libs/Items/Perks/Goliath/Perk_Goliath_Charge.effectset",
            "libs/items/perks/assault/Perk_Assault_Rearm.effectset",
            "libs/EffectSets/Characters/Monster/EffectSet.effectset",
            "Libs\\Items\\Perks\\Windows\\Perk_Windows.effectset",
        ] {
            assert!(gameplay_document(p), "{p} should be indexed for editing");
            assert_eq!(member_kind(p), ModKind::Gameplay);
        }
        // The suffix alone is gameplay even outside an authored root.
        assert_eq!(
            member_kind("objects/unexpected.effectset"),
            ModKind::Gameplay
        );
        // Path traversal still wins over the recognised extension.
        assert_eq!(member_kind("../perks/evil.effectset"), ModKind::NeedsReview);
        // A whole changed-member inventory containing an effectset stays offline.
        assert_eq!(
            classify(["objects/a.dds", "libs/Items/Perks/x.effectset"], true),
            ModKind::Gameplay
        );
        // Unrelated documents remain unindexed.
        assert!(!gameplay_document("libs/Items/Perks/notes.txt"));
    }
}
