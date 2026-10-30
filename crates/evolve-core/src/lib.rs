//! Core boundary for the Rust rewrite.
//! All game data is local and opt-in. No assets are embedded or downloaded.
mod animated_glb_preview;
pub mod animation;
pub mod archive;
pub mod asset_compare;
pub mod batch;
pub mod caf_layers;
pub mod cdf;
pub mod character_assembly;
pub mod character_emet;
pub mod character_equipment;
pub mod character_folder_fbx;
pub mod character_unified_fbx;
pub mod client_hook;
pub mod converter;
pub mod converted_glb_transfer;
pub mod cryxml;
pub mod dds;
pub mod dependency_graph;
pub mod export_bundle;
pub mod fbx;
pub mod fmod;
pub mod fsb5_rebuild;
pub mod gfx;
pub mod gfx_layout;
pub mod glb;
pub mod inventory;
pub mod legacy_rig;
pub mod level_export;
pub mod library_build_updates;
pub mod library_catalog;
pub mod library_import;
pub mod materials;
pub mod model;
pub mod native_caf;
pub mod native_dba;
pub mod native_offline_launch;
pub mod policy;
pub mod profiles;
pub mod rig;
pub mod scene;
pub mod signed_pak;
pub mod skin_preview;
pub mod staging;
pub mod terrain;
mod terrain_tiles;

pub use animation::{
    AnimatedGlb, AnimationError, AnimationReference, CafInspection, MotionClip, PoseSample,
    animation_references, inspect_caf_bytes, inspect_caf_file,
};
pub use archive::{
    PakError, PakInventory, PakMember, RebuildReport, extract_legacy_pak, inspect_legacy_pak,
    rebuild_legacy_pak,
};
pub use asset_compare::{AssetComparison, AssetFingerprint, CompareError, compare_assets};
pub use caf_layers::{CafLayer, sample_verified_caf_layers};
pub use cdf::{
    AssetResolution, CdfAttachment, CdfDefinition, CdfDependencyReport, CdfError, parse_cdf,
    read_cdf, resolve_cdf, resolve_virtual,
};
pub use character_assembly::{
    AssemblyError, AssemblyGeometry, AssemblyPart, AssemblyReport, KalaTubingState,
    combine_source_meshes, is_original_kala_3p, read_static_cdf_assembly,
    read_static_cdf_assembly_with_equipment, read_static_cdf_assembly_with_imported,
    read_static_cdf_assembly_with_kala_state,
};
pub use character_emet::{EmetOptions, discover as discover_emet_options};
pub use character_equipment::{
    EquipmentChoice, EquipmentOverrides, EquipmentSlot, discover_equipment,
    validated_equipment_overrides,
};
pub use character_folder_fbx::{FlatCdfFbxReport, export_flat_cdf_parts_fbx};
pub use character_unified_fbx::{
    CdfExportAppearance, UnifiedCdfFbxReport, export_unified_cdf_fbx,
    export_unified_cdf_fbx_options, export_unified_cdf_fbx_options_with_glow,
    export_unified_cdf_fbx_with_equipment, monster_glow_path, monster_glow_path_with_library,
};
pub use client_hook::{
    ClientHookError, HookPairReport, PakHookPairReport, inspect_client_hook_pair,
    inspect_pak_hook_pair, prepare_client_hook,
};
pub use converter::{ConvertedMotion, ConverterError, convert_original_motion};
pub use dds::{
    DdsError, DdsFormat, DdsInfo, DdsLayout, decode_dds_rgba, decode_dds_rgba_with_layout,
    export_dds_png, inspect_dds,
};
pub use dependency_graph::{
    DependencyEdge, DependencyError, DependencyGraph, DependencyNode, DependencyStatus,
    inspect_dependencies,
};
pub use export_bundle::{ExportBundleOptions, ExportBundleReport, export_bundle};
pub use fbx::{
    FbxError, FbxReport, convert_glb_to_fbx, convert_glb_to_fbx_with_layout,
    convert_glb_to_fbx_with_options, validate_glb_source,
};
pub use gfx::{GfxError, GfxInfo, GfxResource, inspect_gfx};
pub use gfx_layout::{LayoutEdit, StaticLayer, edit_first_frame, static_layers};
pub use glb::{GlbError, export_model_glb, geometry_glb_bytes};
pub use inventory::{
    AssetEntry, AssetKind, Inventory, InventoryError, MAX_ENTRIES, scan_directory,
    scan_directory_with_progress, write_inventory_json,
};
pub use legacy_rig::{parse_legacy_skeleton, read_legacy_skin};
pub use level_export::{LevelExportError, LevelExportReport, export_level_scene_glb};
pub use library_build_updates::{
    BuildUpdateFile, LibraryBuildUpdatePlan, LibraryBuildUpdateReport, apply_library_build_updates,
    scan_library_build_updates,
};
pub use library_catalog::{
    ImportedLibrary, ImportedPak, LibraryError, LibraryReference, LibraryReferenceStatus,
    describe_archive, imported_archive_files, imported_library, resolve_imported_reference,
};
pub use library_import::{
    LibraryBatchImportReport, LibraryImportFailure, LibraryImportPlan, LibraryImportReport,
    import_library_plan_with_keys, import_signed_library_pak, import_signed_library_pak_with_keys,
    import_signed_library_selection, import_signed_library_selection_with_keys,
    scan_library_import_sources,
};
pub use model::{
    ModelError, ModelInfo, RenderMesh, export_model_obj, has_original_mesh_subset_table,
    original_mesh_subsets, parse_model_bytes, parse_world_model_parts, read_model,
    read_world_model_parts,
};
pub use native_caf::{
    CafBonePose, CafController, NativeCafError, NativeCafMotion, decode_native_caf, read_native_caf,
};
pub use native_dba::{
    DbaAnimationRecord, NativeDbaError, NativeDbaInspection, index_native_dba_bytes,
    inspect_native_dba_bytes, inspect_native_dba_file,
};
pub use native_offline_launch::{
    NativeOfflineReadiness, NativeOfflineSwapPlan, NativeOfflineTarget, NativeRecoveryState,
    audit_native_offline_swap, inspect_native_offline_readiness, plan_native_offline_swap,
};
pub use policy::{AnimationPolicy, ExportFeature, FeatureStatus, PARITY_MATRIX};
pub use rig::{
    CompiledSkeleton, RigError, RigReport, VerifiedWeights, export_rigged_character_fbx,
    export_rigged_character_fbx_options, export_rigged_character_fbx_textured,
    export_rigged_character_fbx_textured_animated, export_rigged_character_glb,
    parse_compiled_skeleton, remap_verified_weights, rigged_glb_bytes,
};
pub use scene::{
    LevelScene, PlacedObject, SceneError, Transform, edit_level_transform, export_edited_layer,
    export_edited_layers, export_edited_layers_guarded, load_level_scene, parse_level_scene,
};
pub use signed_pak::{
    SignedEntry, SignedPakError, SignedPakInfo, SignedRebuildReport, extract_signed_pak,
    inspect_signed_pak, open_verified_signed_pak, rebuild_signed_pak_entries,
    rebuild_signed_pak_entry, resign_signed_pak,
};
pub use skin_preview::{VerifiedCafAssembly, VerifiedCafSkin};
pub use terrain::{
    TerrainError, TerrainInfo, TerrainMesh, TerrainPaintMap, TerrainTextureHeader,
    TerrainTextureLayer, decode_terrain_bytes, decode_terrain_paint_bytes, export_terrain_obj,
    read_terrain, read_terrain_paint, read_terrain_texture_header,
};
pub use terrain_tiles::{TerrainTextureAtlas, read_terrain_texture_atlas};

pub use staging::{
    StageError, StageTransaction, apply_stage_pak, apply_stage_pak_with_additions,
    import_stage_pak, initialize_stage, restore_stage, stage_status,
};

pub use profiles::{
    ModProfile, ProfileCatalog, ProfileError, ProfileMod, ProfilePlan, add_mod_to_profile,
    create_mod_profile, delete_mod_profile, enable_profile_mod, list_mod_profiles,
    plan_mod_profile,
};

pub use batch::{
    BatchEntry, BatchError, BatchJournal, apply_profile_stage, batch_stage_status,
    restore_profile_stage,
};

pub use materials::{
    DecalTexMod, MaterialDocument, MaterialError, MaterialReport, MaterialSlot, ResolvedTexture,
    TextureReference, authored_damage_layer, blend_damage_rgba, material_texture_candidates,
    parse_material, read_material, resolve_materials, scoped_texture_candidates,
};
pub mod gameplay_values;
pub mod mod_safety;
