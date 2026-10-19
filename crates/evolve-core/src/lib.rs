//! Core boundary for the Rust rewrite.
//! All game data is local and opt-in. No assets are embedded or downloaded.
pub mod inventory;
pub mod policy;

pub use inventory::{
    AssetEntry, AssetKind, Inventory, InventoryError, MAX_ENTRIES, scan_directory,
    write_inventory_json,
};
pub use policy::{AnimationPolicy, ExportFeature, FeatureStatus, PARITY_MATRIX};
