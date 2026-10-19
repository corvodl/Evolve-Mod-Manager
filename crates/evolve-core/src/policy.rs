//! Parity contract for the existing desktop manager.
//! This module is descriptive; it is not evidence that unported actions work.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimationPolicy {
    pub embed_verified_animations: bool,
}

impl Default for AnimationPolicy {
    fn default() -> Self {
        Self { embed_verified_animations: false }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureStatus {
    Implemented,
    Planned,
}

#[derive(Debug, Clone, Copy)]
pub struct ExportFeature {
    pub name: &'static str,
    pub status: FeatureStatus,
}

pub const PARITY_MATRIX: &[ExportFeature] = &[
    ExportFeature { name: "Read-only scoped asset inventory and JSON report", status: FeatureStatus::Implemented },
    ExportFeature { name: "PAK verification, extraction, rebuilding and signing", status: FeatureStatus::Planned },
    ExportFeature { name: "Staged offline mods, backups and crash-safe restoration", status: FeatureStatus::Planned },
    ExportFeature { name: "Asset Library, dependency resolution and texture provenance", status: FeatureStatus::Planned },
    ExportFeature { name: "DDS high-resolution split streams, PNG and normal maps", status: FeatureStatus::Planned },
    ExportFeature { name: "CDF character assembly, original UV0, rigs and skeletons", status: FeatureStatus::Planned },
    ExportFeature { name: "FBX export: all stages, LOD0, hunter 1P/3P and equipment", status: FeatureStatus::Planned },
    ExportFeature { name: "Optional verified CAF/DBA FBX Actions (default off)", status: FeatureStatus::Planned },
    ExportFeature { name: "Blender Animation Browser interoperability", status: FeatureStatus::Planned },
    ExportFeature { name: "GPU model renderer, materials and animation timeline", status: FeatureStatus::Planned },
    ExportFeature { name: "CryEngine GFX and Level Editor with Blender scene export", status: FeatureStatus::Planned },
    ExportFeature { name: "Mod profiles, settings and signed release updater", status: FeatureStatus::Planned },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_policy_never_embeds_animations_by_default() {
        assert!(!AnimationPolicy::default().embed_verified_animations);
    }
}
