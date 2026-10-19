//! Core boundary for the Rust rewrite.
//! All game data is local and opt-in. No assets are embedded or downloaded.
pub mod archive;
pub mod cdf;
pub mod dds;
pub mod inventory;
pub mod model;
pub mod policy;
pub mod terrain;
pub mod scene;
pub mod signed_pak;

pub use archive::{
    PakError, PakInventory, PakMember, RebuildReport, extract_legacy_pak, inspect_legacy_pak,
    rebuild_legacy_pak,
};
pub use cdf::{
    AssetResolution, CdfAttachment, CdfDefinition, CdfDependencyReport, CdfError, parse_cdf,
    read_cdf, resolve_cdf, resolve_virtual,
};
pub use dds::{DdsError, DdsFormat, DdsInfo, decode_dds_rgba, export_dds_png, inspect_dds};
pub use model::{ModelError, ModelInfo, RenderMesh, export_model_obj, parse_model_bytes, read_model};
pub use inventory::{
    AssetEntry, AssetKind, Inventory, InventoryError, MAX_ENTRIES, scan_directory,
    write_inventory_json,
};
pub use signed_pak::{SignedPakError, SignedPakInfo, SignedEntry, inspect_signed_pak, extract_signed_pak, open_verified_signed_pak};
pub use scene::{SceneError, LevelScene, PlacedObject, Transform, parse_level_scene, load_level_scene, export_edited_layer};
pub use terrain::{TerrainError, TerrainInfo, TerrainMesh, decode_terrain_bytes, export_terrain_obj, read_terrain};
pub use policy::{AnimationPolicy, ExportFeature, FeatureStatus, PARITY_MATRIX};
