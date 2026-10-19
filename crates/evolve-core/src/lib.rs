//! Core boundary for the Rust rewrite.
//! All game data is local and opt-in. No assets are embedded or downloaded.
pub mod archive;
pub mod cdf;
pub mod dds;
pub mod inventory;
pub mod policy;

pub use archive::{PakError, PakInventory, PakMember, extract_legacy_pak, inspect_legacy_pak};
pub use cdf::{
    AssetResolution, CdfAttachment, CdfDefinition, CdfDependencyReport, CdfError, parse_cdf,
    read_cdf, resolve_cdf, resolve_virtual,
};
pub use dds::{DdsError, DdsFormat, DdsInfo, decode_dds_rgba, export_dds_png, inspect_dds};
pub use inventory::{
    AssetEntry, AssetKind, Inventory, InventoryError, MAX_ENTRIES, scan_directory,
    write_inventory_json,
};
pub use policy::{AnimationPolicy, ExportFeature, FeatureStatus, PARITY_MATRIX};
