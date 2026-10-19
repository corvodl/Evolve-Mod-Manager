//! Core boundary for the Rust rewrite.
//! All game data is local and opt-in. No assets are embedded or downloaded.
pub mod animation;
pub mod archive;
pub mod cdf;
pub mod converter;
pub mod dds;
pub mod fbx;
pub mod glb;
pub mod inventory;
pub mod level_export;
pub mod model;
pub mod native_caf;
pub mod policy;
pub mod rig;
pub mod scene;
pub mod signed_pak;
pub mod terrain;

pub use animation::{
    AnimatedGlb, AnimationError, AnimationReference, CafInspection, MotionClip, PoseSample,
    animation_references, inspect_caf_bytes, inspect_caf_file,
};
pub use archive::{
    PakError, PakInventory, PakMember, RebuildReport, extract_legacy_pak, inspect_legacy_pak,
    rebuild_legacy_pak,
};
pub use converter::{ConverterError,ConvertedMotion,convert_original_motion};
pub use cdf::{
    AssetResolution, CdfAttachment, CdfDefinition, CdfDependencyReport, CdfError, parse_cdf,
    read_cdf, resolve_cdf, resolve_virtual,
};
pub use dds::{DdsError, DdsFormat, DdsInfo, decode_dds_rgba, export_dds_png, inspect_dds};
pub use fbx::{FbxError, FbxReport, convert_glb_to_fbx, validate_glb_source};
pub use glb::{GlbError, export_model_glb, geometry_glb_bytes};
pub use inventory::{
    AssetEntry, AssetKind, Inventory, InventoryError, MAX_ENTRIES, scan_directory,
    write_inventory_json,
};
pub use level_export::{LevelExportError, LevelExportReport, export_level_scene_glb};
pub use model::{
    ModelError, ModelInfo, RenderMesh, export_model_obj, parse_model_bytes, read_model,
};
pub use native_caf::{
    CafBonePose, CafController, NativeCafError, NativeCafMotion, decode_native_caf, read_native_caf,
};
pub use policy::{AnimationPolicy, ExportFeature, FeatureStatus, PARITY_MATRIX};
pub use rig::{
    CompiledSkeleton, RigError, RigReport, VerifiedWeights, export_rigged_character_fbx,
    export_rigged_character_glb, parse_compiled_skeleton, remap_verified_weights, rigged_glb_bytes,
};
pub use scene::{
    LevelScene, PlacedObject, SceneError, Transform, export_edited_layer, load_level_scene,
    parse_level_scene,
};
pub use signed_pak::{
    SignedEntry, SignedPakError, SignedPakInfo, SignedRebuildReport, extract_signed_pak,
    inspect_signed_pak, open_verified_signed_pak, rebuild_signed_pak_entry,
};
pub use terrain::{
    TerrainError, TerrainInfo, TerrainMesh, decode_terrain_bytes, export_terrain_obj, read_terrain,
};
