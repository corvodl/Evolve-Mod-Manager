#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

//! Native wgpu-backed desktop shell. Asset operations use blocking workers, so
//! file I/O/decompression never runs in the interactive Iced view/update path.
mod activity_window;
mod assembly_materials;
mod asset_cache;
mod asset_library_page;
mod asset_viewer;
mod auto_resources;
mod blender_install;
mod crash_diagnostics;
mod design;
mod developer_auth;
mod explorer_tree;
mod gfx_dependencies;
mod gfx_external_textures;
mod gfx_editor;
mod gfx_layout_canvas;
mod gfx_live_inspector;
mod launcher_bridge;
mod level_workspace;
mod menu_popup;
mod modding_keys;
mod modding_page;
mod modding_projects;
mod orientation_gizmo;
mod private_pages;
mod selection;
mod settings;
mod settings_page;
mod storage_usage;
mod system_health;
mod textured;
mod updater;
mod updater_apply;
mod uv_preview;
mod view_navigation;
mod viewer_grid;
mod viewer_inspector;
mod viewport;
use asset_cache::{SharedCache, decoded_dds, decoded_dds_indexed};
use evolve_core::{
    AnimatedGlb, AnimationPolicy, AssemblyReport, AssetComparison, AssetResolution, BatchJournal,
    CdfDefinition, CdfDependencyReport, ConvertedMotion, DdsInfo, DependencyGraph,
    EquipmentOverrides, EquipmentSlot, HookPairReport, ImportedLibrary, ImportedPak, Inventory,
    KalaTubingState, LevelScene, LibraryBatchImportReport, LibraryReferenceStatus, MaterialReport,
    ModProfile, ModelInfo, NativeCafMotion, PakHookPairReport, PakInventory, ProfileCatalog,
    ProfileMod, ProfilePlan, RenderMesh, RigReport, SignedPakInfo, StageTransaction, TerrainInfo,
    VerifiedCafAssembly, VerifiedCafSkin, add_mod_to_profile, apply_profile_stage, apply_stage_pak,
    batch_stage_status, compare_assets, convert_glb_to_fbx, convert_original_motion,
    create_mod_profile, decode_dds_rgba, discover_equipment, export_dds_png,
    export_level_scene_glb, export_rigged_character_fbx_textured_animated,
    export_rigged_character_glb, extract_legacy_pak, extract_signed_pak,
    import_signed_library_selection_with_keys, import_stage_pak, imported_archive_files,
    imported_library, initialize_stage, inspect_dds, inspect_dependencies, inspect_legacy_pak,
    inspect_pak_hook_pair, inspect_signed_pak, list_mod_profiles, load_level_scene,
    material_texture_candidates, plan_mod_profile, prepare_client_hook, read_cdf, read_material,
    read_model, read_native_caf, read_terrain, rebuild_legacy_pak, rebuild_signed_pak_entry,
    resign_signed_pak, resolve_cdf, resolve_imported_reference, resolve_materials,
    restore_profile_stage, restore_stage, scan_directory, scan_directory_with_progress,
    stage_status,
};
use iced::widget::{
    canvas, column, container, image, row, scrollable, shader, slider, space, svg, text, tooltip,
};
use iced::{Element, Fill, Length, Subscription, Task, Theme, window};
use launcher_bridge::{
    LaunchReady, inspect_existing_launcher, launch_existing_offline_client,
    restore_existing_offline_client,
};
use modding_projects::{ProjectRow, discover_projects};
use selection::MultiSelection;
use settings::{Preferences, load_from, save_to, settings_path};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;
use textured::Scene as TexturedScene;
use updater::{ReleaseUpdate, UpdateChannel, discover_update};
use updater_apply::{PreparedUpdate, launch_worker, prepare_update};
use viewport::MeshViewport;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Page {
    Home,
    Instructions,
    AssetLibrary,
    Files,
    WhatsNew,
    Credits,
    Developer,
    Models,
    Levels,
    Settings,
}

impl Page {
    const ALL: [Self; 7] = [
        Self::Home,
        Self::Instructions,
        Self::AssetLibrary,
        Self::Files,
        Self::WhatsNew,
        Self::Credits,
        Self::Developer,
    ];

    fn icon(self) -> svg::Handle {
        let data: &'static [u8] = match self {
            Self::Home => include_bytes!("../../../assets/nav/home.svg"),
            Self::Instructions => include_bytes!("../../../assets/nav/instructions.svg"),
            Self::AssetLibrary => include_bytes!("../../../assets/nav/asset-library.svg"),
            Self::Files => include_bytes!("../../../assets/nav/modding.svg"),
            Self::WhatsNew => include_bytes!("../../../assets/nav/whats-new.svg"),
            Self::Credits => include_bytes!("../../../assets/nav/credits.svg"),
            Self::Developer => include_bytes!("../../../assets/nav/developer.svg"),
            Self::Models => include_bytes!("../../../assets/nav/box.svg"),
            Self::Levels => include_bytes!("../../../assets/nav/map.svg"),
            Self::Settings => include_bytes!("../../../assets/nav/settings.svg"),
        };
        svg::Handle::from_memory(data.to_vec())
    }

    fn title(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Instructions => "Instructions",
            Self::AssetLibrary => "Asset Library",
            Self::Files => "Modding",
            Self::WhatsNew => "What's New",
            Self::Credits => "Credits",
            Self::Developer => "Developer",
            Self::Models => "Textures & Models",
            Self::Levels => "Level Editor",
            Self::Settings => "Settings",
        }
    }
}

#[derive(Debug, Clone)]
struct LevelTransformDiff {
    guid: String,
    old_local: evolve_core::Transform,
    new_local: evolve_core::Transform,
}

#[derive(Default)]
struct State {
    input: String,
    output: String,
    replacements: String,
    signed_entry: String,
    signed_replacement: String,
    hook_original: String,
    hook_prepared: String,
    stage_feedback: String,
    profile_name: String,
    profile_id: String,
    profile_catalog: Option<ProfileCatalog>,
    profile_feedback: String,
    diffuse_input: String,
    diffuse: Option<Arc<::image::RgbaImage>>,
    cdf_surfaces: Option<Arc<Vec<textured::Surface>>>,
    /// The immutable decoded CDF appearance: slider changes never read a CDF again.
    cdf_base_surfaces: Option<Arc<Vec<textured::Surface>>>,
    /// Once-only resolved 100% damage layer, reused for every slider position.
    cdf_full_damage_surfaces: Option<Arc<Vec<textured::Surface>>>,
    cdf_damage_preparing: bool,
    cdf_legacy_2012: bool,
    texture_preview: Option<image::Handle>,
    gfx_session: Option<gfx_editor::Session>,
    gfx_active: bool,
    gfx_busy: bool,
    gfx_preview: Option<image::Handle>,
    gfx_resource: usize,
    gfx_status: String,
    gfx_layout_open: bool,
    gfx_layout_selected: Option<usize>,
    gfx_layout_x: String,
    gfx_layout_y: String,
    gfx_layout_sx: String,
    gfx_layout_sy: String,
    gfx_layout_rotation: String,
    gfx_layout_visible: bool,
    gfx_layout_show_raster: bool,
    gfx_live_open: bool,
    gfx_live_status: String,
    gfx_live_snapshot: Option<gfx_live_inspector::UiSnapshot>,

    diffuse_status: String,
    normal_input: String,
    normal_map: Option<Arc<::image::RgbaImage>>,
    emissive_input: String,
    emissive_map: Option<Arc<::image::RgbaImage>>,
    specular_map: Option<Arc<::image::RgbaImage>>,
    gloss_map: Option<Arc<::image::RgbaImage>>,
    material_input: String,
    material_report: Option<MaterialReport>,
    auto_clips: Vec<auto_resources::AutoClip>,
    auto_resource_note: String,
    auto_clip_filter: String,
    export_include_animation: bool,
    export_dialog_open: bool,
    export_selected: HashSet<usize>,
    export_filter: String,
    export_parent: String,
    export_folder_name: String,
    export_rigged: bool,
    export_embed_textures: bool,
    export_dependencies: bool,
    auto_idle_pending: Option<PathBuf>,
    auto_play_native_caf: bool,
    auto_material_pending: bool,
    caf_path: String,
    skeleton_path: String,
    blender_path: String,
    converter_path: String,
    converted_glbs: Vec<PathBuf>,
    // Ephemeral auto-DBA converter output is deleted when this state drops.
    converted_dba_cache: Option<Arc<tempfile::TempDir>>,
    glb_motion: Option<Arc<AnimatedGlb>>,
    selected_motion_clip: usize,
    native_caf: Option<Arc<NativeCafMotion>>,
    caf_layer: Option<Arc<NativeCafMotion>>,
    caf_layer_label: String,
    caf_layer_weight: f32,
    skin_preview: Option<Arc<VerifiedCafSkin>>,
    cdf_caf_preview: Option<Arc<VerifiedCafAssembly>>,
    cdf_report: Option<Arc<AssemblyReport>>,
    cdf_rest_mesh: Option<Arc<RenderMesh>>,
    motion_time: f32,
    /// Continuous shader Decal clock, separate from CAF animation playback.
    decal_time: f32,
    playing: bool,
    assets_root: String,
    terrain_source: String,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    pan_x: f32,
    pan_y: f32,
    search: String,
    page: Option<Page>,
    inventory: Option<Inventory>,
    pak: Option<PakInventory>,
    signed_pak: Option<SignedPakInfo>,
    texture: Option<DdsInfo>,
    character: Option<CdfDefinition>,
    cdf_dependencies: Option<CdfDependencyReport>,
    model: Option<Arc<RenderMesh>>,
    /// Exact render file (possibly .skinm from an imported PAK), not the
    /// selected authored .skin metadata path.
    render_source: Option<PathBuf>,
    level_scene: Option<Arc<LevelScene>>,
    terrain: Option<TerrainInfo>,
    selected: Option<usize>,
    file_selection: MultiSelection,
    project_selection: MultiSelection,
    library_selection: MultiSelection,
    keyboard_modifiers: iced::keyboard::Modifiers,
    status: String,
    busy: bool,
    preferences: Preferences,
    settings_status: String,
    settings_advanced_paths: bool,
    storage_analyzing: bool,
    storage_usage: Option<storage_usage::StorageUsage>,
    update_status: String,
    update_worker_failure: Option<String>,
    available_update: Option<ReleaseUpdate>,
    startup_update_checked: bool,
    viewer_search: String,
    viewer_rows: Vec<explorer_tree::ExplorerRow>,
    viewer_page: usize,
    viewer_more_rows: bool,
    // Latest selected asset while an earlier worker is running. Never replay
    // a stale index after the indexed source folder changes.
    viewer_queued_preview: Option<usize>,
    viewer_context_index: Option<usize>,
    viewer_collapsed: HashSet<String>,
    viewer_menu: Option<&'static str>,
    viewer_explorer_visible: bool,
    viewer_sidebar_animations: bool,
    viewer_sidebar_dependencies: bool,
    viewer_sidebar_active: ViewerSidebarTab,
    equipment_source: Option<PathBuf>,
    equipment_slots: Vec<EquipmentSlot>,
    equipment_overrides: EquipmentOverrides,
    kala_tubing: KalaTubingState,
    full_armor_glow: bool,
    /// Viewport-only gain over the original source material emission.
    emissive_strength: f32,
    damage_blend_percent: u8,
    damage_blend_enabled: bool,
    /// CDF source-part completion tracked by the loading worker.
    viewer_load_progress: Option<Arc<AtomicUsize>>,
    viewer_from_library: bool,
    developer_password: String,
    developer_status: String,
    developer_unlocked: bool,
    developer_attempts: u8,
    developer_checking: bool,
    show_advanced_tools: bool,
    activity_log: Vec<String>,
    health_expanded: bool,
    health_checking: bool,
    health_report: Option<system_health::Snapshot>,
    library_catalog: Option<ImportedLibrary>,
    library_startup_loading: bool,
    library_selected: Option<usize>,
    library_root: String,
    library_import_source: String,
    library_search: String,
    library_by_build: bool,
    library_build_index: usize,
    library_page: usize,
    settings_tab: settings_page::Tab,
    library_show_configuration: bool,
    library_menu: Option<&'static str>,
    library_matches: Vec<usize>,
    dependency_graph: Option<DependencyGraph>,
    comparison: Option<AssetComparison>,
    compare_original: String,
    compare_modified: String,
    viewer_show_inspector: bool,
    viewer_split: f32,
    viewer_tab: ViewerTab,
    playback_speed: f32,
    playback_loop: bool,
    viewer_wireframe: bool,
    light_strength: f32,
    level_filter: String,
    level_selected: Option<usize>,
    level_selection: MultiSelection,
    level_zoom: f32,
    level_source: Option<PathBuf>,
    level_source_sha: String,
    level_original_transforms: HashMap<String, evolve_core::Transform>,
    level_dirty_count: usize,
    level_undo: VecDeque<LevelTransformDiff>,
    level_redo: VecDeque<LevelTransformDiff>,
    level_edit_position: String,
    level_edit_rotation: String,
    level_edit_scale: String,
    level_edit_output: String,
    scan_cancel: Option<Arc<AtomicBool>>,
    scan_indexed: Option<Arc<AtomicUsize>>,
    preview_cache: SharedCache,
    modding_projects: Option<Vec<ProjectRow>>,
    modding_selected: Option<usize>,
    modding_filter: String,
    modding_page: usize,
    modding_changes_page: usize,
    editor_replace_source: String,
    modding_menu: Option<&'static str>,
    modding_advanced: bool,
    modding_profiles: bool,
    modding_open_editor_when_scanned: bool,
    modding_pending_focus: Option<PathBuf>,
}

/// Docked tools all share the same resizable sidebar. Files is permanent;
/// the other three tabs can be opened and dismissed independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ViewerSidebarTab {
    #[default]
    Files,
    Animations,
    Inspector,
    Dependencies,
    Equipment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ViewerTab {
    #[default]
    Model,
    Uv,
    Dependencies,
    Compare,
}

#[derive(Debug, Clone)]
enum Operation {
    IndexProjects(PathBuf, PathBuf),
    BuildRawProject(ProjectRow, PathBuf, PathBuf),
    ImportRawProjectMember(ProjectRow, PathBuf, String, PathBuf),
    RestoreRawProjectMember(ProjectRow, PathBuf, String),
    UnpackRawProject(PathBuf, PathBuf, PathBuf),
    LoadImportedLibrary(PathBuf),
    ImportSignedLibrary(PathBuf, PathBuf, PathBuf),
    OpenImportedPak(ImportedPak),
    CreateProjectFromLibrary(ImportedPak, PathBuf, PathBuf),
    InspectDependencies(Inventory, String),
    CompareAssets(PathBuf, PathBuf),
    Scan(PathBuf, Arc<AtomicBool>, Arc<AtomicUsize>),
    InspectPak(PathBuf),
    AnalyzeStorage(PathBuf, PathBuf, PathBuf, PathBuf, PathBuf),
    InspectDds(PathBuf),
    PreviewDds(
        PathBuf,
        String,
        Inventory,
        Option<PathBuf>,
        Option<String>,
        SharedCache,
    ),
    LoadDiffuse(PathBuf, SharedCache),
    LoadNormal(PathBuf, SharedCache),
    LoadEmissive(PathBuf, SharedCache),
    ResolveMaterial(PathBuf, PathBuf, Option<PathBuf>, Option<String>),
    LoadModelSurfaces(
        PathBuf,
        PathBuf,
        Inventory,
        Option<PathBuf>,
        Option<String>,
        SharedCache,
    ),
    LoadCaf(PathBuf),
    LoadCafLayer(String, Arc<NativeCafMotion>, String, PathBuf),
    BindCafPreview(PathBuf, PathBuf, NativeCafMotion),
    BindCdfCafPreview(
        PathBuf,
        Arc<RenderMesh>,
        Arc<AssemblyReport>,
        NativeCafMotion,
    ),
    RiggedGlb(PathBuf, PathBuf, PathBuf),
    ExportBundle(Inventory, evolve_core::ExportBundleOptions),
    RiggedFbx(
        PathBuf,
        PathBuf,
        PathBuf,
        Option<PathBuf>,
        Option<PathBuf>,
        PathBuf,
    ),
    ConvertMotion(PathBuf, PathBuf, PathBuf, PathBuf),
    LoadGlbMotion(PathBuf),
    InspectCdf(PathBuf),
    BuildStaticCdf(
        PathBuf,
        Inventory,
        Option<PathBuf>,
        Option<String>,
        SharedCache,
        EquipmentOverrides,
        bool,
        KalaTubingState,
        Arc<AtomicUsize>,
    ),
    PrepareCdfDamage(
        PathBuf,
        Arc<AssemblyReport>,
        Arc<Vec<textured::Surface>>,
        Inventory,
        Option<PathBuf>,
        Option<String>,
        SharedCache,
    ),
    ReblendCdfDamage(
        PathBuf,
        Arc<Vec<textured::Surface>>,
        Arc<Vec<textured::Surface>>,
        u8,
    ),
    ResolveCdf(PathBuf, PathBuf),
    InspectModel(PathBuf),
    InspectIndexedModel(PathBuf, String, Inventory, Option<PathBuf>, Option<String>),
    DiscoverModelResources(String, Inventory, Option<PathBuf>, Option<String>),
    LoadDeclaredClip(usize, String, Inventory, Option<PathBuf>, Option<String>),
    LoadDeclaredDba(String, Inventory, Option<PathBuf>, Option<String>, PathBuf),
    InspectLevel(PathBuf),
    ExportEditedLevel(
        PathBuf,
        PathBuf,
        Vec<(String, evolve_core::Transform)>,
        Vec<PathBuf>,
        String,
    ),
    InspectTerrain(PathBuf),
    InspectSignedPak(PathBuf, PathBuf, String),
    ExtractSignedPak(PathBuf, PathBuf, String, PathBuf),
    ResignSignedPak(PathBuf, PathBuf, PathBuf, PathBuf, String, PathBuf),
    PrepareClientHook(PathBuf, PathBuf, PathBuf, PathBuf),
    VerifySignedHookPair(PathBuf, PathBuf, PathBuf, PathBuf, PathBuf, PathBuf, String),
    CheckExistingOfflineLauncher(PathBuf, PathBuf),
    StartExistingOfflineLauncher(PathBuf, PathBuf),
    RestoreExistingOfflineLauncher(PathBuf, PathBuf),
    RebuildSignedPak(
        PathBuf,
        PathBuf,
        PathBuf,
        PathBuf,
        String,
        String,
        PathBuf,
        PathBuf,
    ),
    ExportLevelGlb(PathBuf, PathBuf, Option<PathBuf>, PathBuf),
    ExportLevelFbx(PathBuf, PathBuf, Option<PathBuf>, PathBuf, PathBuf),
    StageInit(PathBuf),
    StageStatus(PathBuf),
    StageImport(PathBuf, PathBuf, String, PathBuf, String),
    StageApply(PathBuf, PathBuf, String, PathBuf, PathBuf, String),
    StageRestore(PathBuf),
    ListProfiles(PathBuf),
    CreateProfile(PathBuf, String),
    AddProfileMod(PathBuf, String, PathBuf, String, PathBuf, PathBuf, String),
    PlanProfile(PathBuf, String, PathBuf, PathBuf),
    ApplyProfile(PathBuf, String, PathBuf, PathBuf),
    RecoverProfile(PathBuf),
    StatusProfile(PathBuf),
    ExtractPak(PathBuf, PathBuf),
    RebuildPak(PathBuf, PathBuf, PathBuf),
    ExportPng(PathBuf, PathBuf),
}

#[derive(Debug, Clone)]
enum Completed {
    Projects(Vec<ProjectRow>),
    UnpackedProject(PathBuf, usize),
    ImportedRawProjectMember(bool, String, PathBuf),
    RestoredRawProjectMember(bool, String, PathBuf),
    ImportedLibrary(ImportedLibrary),
    ImportedSignedLibrary(LibraryBatchImportReport, ImportedLibrary),
    ImportedPakFiles(Inventory),
    Dependencies(DependencyGraph),
    Comparison(AssetComparison),
    Inventory(Inventory),
    Pak(PakInventory),
    Texture(DdsInfo),
    DdsPreview(PathBuf, DdsInfo, Arc<::image::RgbaImage>),
    StorageAnalyzed(storage_usage::StorageUsage),
    Diffuse(DdsInfo, Arc<::image::RgbaImage>),
    Normal(DdsInfo, Arc<::image::RgbaImage>),
    Emissive(DdsInfo, Arc<::image::RgbaImage>),
    MaterialResolved(
        PathBuf,
        MaterialReport,
        Option<::image::RgbaImage>,
        Option<::image::RgbaImage>,
        Option<::image::RgbaImage>,
        Option<::image::RgbaImage>,
        Option<::image::RgbaImage>,
        Vec<String>,
    ),
    ModelSurfaces(PathBuf, Vec<textured::Surface>, Vec<String>),
    Caf(NativeCafMotion),
    CafLayer(String, Arc<NativeCafMotion>, String, NativeCafMotion),
    BoundCafPreview(PathBuf, Box<VerifiedCafSkin>),
    BoundCdfCafPreview(PathBuf, Box<VerifiedCafAssembly>),
    RiggedExport(RigReport, PathBuf),
    BundleExported(evolve_core::ExportBundleReport),
    MotionConverted(ConvertedMotion, PathBuf),
    DbaConverted(ConvertedMotion, PathBuf, Arc<tempfile::TempDir>),
    MotionGlb(AnimatedGlb),
    Character(CdfDefinition),
    StaticCharacter(
        RenderMesh,
        AssemblyReport,
        Vec<textured::Surface>,
        Vec<String>,
    ),
    PreparedCdfDamage(PathBuf, Vec<textured::Surface>),
    ReblendedCdfDamage(PathBuf, u8, Vec<textured::Surface>),
    CharacterDependencies(CdfDependencyReport),
    Model(RenderMesh, PathBuf),
    ModelResources(String, auto_resources::AutoDiscovery),
    DeclaredClip(usize, String, NativeCafMotion),
    Level(LevelScene, PathBuf, String),
    EditedLevelExported(PathBuf, usize),
    Terrain(RenderMesh, TerrainInfo),
    SignedPak(SignedPakInfo),
    ResignedSignedPak(SignedPakInfo, PathBuf),
    HookPrepared(HookPairReport, PathBuf),
    SignedHookPreflight(PakHookPairReport),
    OfflineLauncherReady(LaunchReady),
    OfflineLauncherStarted(usize, u32),
    OfflineLauncherRestored(usize, bool),
    ExtractedSigned(usize, PathBuf),
    RebuiltSigned(String, PathBuf),
    LevelExported(usize, usize, PathBuf),
    LevelFbxExported(usize, usize, PathBuf),
    StageInitialized(PathBuf),
    StageStatus(Option<StageTransaction>),
    StageImported(String),
    StageApplied(StageTransaction),
    StageRestored(StageTransaction),
    ProfileCatalog(ProfileCatalog),
    ProfileCreated(ModProfile),
    ProfileAdded(ProfileMod),
    ProfilePlanned(ProfilePlan),
    ProfileApplied(BatchJournal),
    ProfileRecovered(BatchJournal),
    ProfileStatus(Option<BatchJournal>),
    Extracted(usize, PathBuf),
    Rebuilt(usize, PathBuf),
    ExportedPng(DdsInfo, PathBuf),
}

impl Operation {
    fn execute(self) -> Result<Completed, String> {
        match self {
            Self::IndexProjects(root, stage) => {
                discover_projects(&root, &stage).map(Completed::Projects)
            }
            Self::BuildRawProject(project, projects, stage) => {
                modding_projects::build_project_raw(&project, &projects, &stage)
                    .map(|(entry, target)| Completed::RebuiltSigned(entry, target))
            }
            Self::ImportRawProjectMember(project, projects, member, source) => {
                modding_projects::import_raw_member(&project, &projects, &member, &source).map(
                    |changed| {
                        Completed::ImportedRawProjectMember(changed, member, project.workspace)
                    },
                )
            }
            Self::RestoreRawProjectMember(project, projects, member) => {
                modding_projects::restore_original_raw_member(&project, &projects, &member).map(
                    |changed| {
                        Completed::RestoredRawProjectMember(changed, member, project.workspace)
                    },
                )
            }
            Self::UnpackRawProject(projects, stage, source) => {
                modding_projects::unpack_raw_project(&projects, &stage, &source)
                    .map(|(path, count)| Completed::UnpackedProject(path, count))
            }
            Self::InspectDependencies(inventory, relative) => {
                inspect_dependencies(&inventory, &relative)
                    .map(Completed::Dependencies)
                    .map_err(|e| e.to_string())
            }
            Self::CompareAssets(original, modified) => compare_assets(&original, &modified)
                .map(Completed::Comparison)
                .map_err(|e| e.to_string()),
            Self::LoadImportedLibrary(root) => imported_library(&root)
                .map(Completed::ImportedLibrary)
                .map_err(|e| e.to_string()),
            Self::ImportSignedLibrary(root, source, stage) => {
                // Resolve all known public keys for each archive; the included
                // original RSA verifier also works without an existing stage.
                let keys = modding_keys::archive_verification_keys(&stage)?;
                if !root.exists() {
                    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
                }
                let report =
                    import_signed_library_selection_with_keys(&root, &source, &keys.paths)?;
                let catalog = imported_library(&root).map_err(|e| e.to_string())?;
                Ok(Completed::ImportedSignedLibrary(report, catalog))
            }
            Self::OpenImportedPak(pak) => imported_archive_files(&pak)
                .map(Completed::ImportedPakFiles)
                .map_err(|e| e.to_string()),
            Self::CreateProjectFromLibrary(pak, projects, stage) => {
                // Exact source PAK must be in the configured stage/paks;
                // never choose a similarly named or cross-build PAK.
                imported_archive_files(&pak).map_err(|e| e.to_string())?;
                modding_projects::unpack_raw_project(
                    &projects,
                    &stage,
                    PathBuf::from(&pak.source).as_path(),
                )
                .map(|(path, count)| Completed::UnpackedProject(path, count))
            }
            Self::Scan(source, cancelled, indexed) => {
                scan_directory_with_progress(&source, &cancelled, &indexed)
                    .map(Completed::Inventory)
                    .map_err(|e| e.to_string())
            }
            Self::AnalyzeStorage(data, games, stage, projects, library) => {
                Ok(Completed::StorageAnalyzed(storage_usage::analyze(
                    &data, &games, &stage, &projects, &library,
                )))
            }
            Self::InspectPak(source) => inspect_legacy_pak(&source)
                .map(Completed::Pak)
                .map_err(|e| e.to_string()),
            Self::BindCdfCafPreview(source, mesh, report, motion) => {
                let preview = VerifiedCafAssembly::from_parts((*mesh).clone(), &report, motion)
                    .map_err(|e| format!("Original multi-part CDF/CAF skin binding: {e}"))?;
                Ok(Completed::BoundCdfCafPreview(source, Box::new(preview)))
            }
            Self::BindCafPreview(mesh, skeleton, motion) => {
                let skin = VerifiedCafSkin::from_originals(&mesh, &skeleton, motion)
                    .map_err(|e| format!("Original CAF skeleton binding: {e}"))?;
                Ok(Completed::BoundCafPreview(mesh, Box::new(skin)))
            }
            Self::RiggedGlb(mesh, skeleton, dest) => {
                export_rigged_character_glb(&mesh, &skeleton, &dest)
                    .map(|r| Completed::RiggedExport(r, dest))
                    .map_err(|e| e.to_string())
            }
            Self::ExportBundle(inventory, options) => {
                evolve_core::export_bundle(&inventory, &options).map(Completed::BundleExported)
            }
            Self::RiggedFbx(mesh, skeleton, blender, diffuse, caf, dest) => {
                export_rigged_character_fbx_textured_animated(
                    &mesh,
                    &skeleton,
                    &blender,
                    diffuse.as_deref(),
                    caf.as_deref(),
                    &dest,
                )
                .map(|(r, _)| Completed::RiggedExport(r, dest))
                .map_err(|e| e.to_string())
            }
            Self::ConvertMotion(exe, source, assets, destination) => {
                convert_original_motion(&exe, &source, &assets, &destination)
                    .map(|result| Completed::MotionConverted(result, destination))
                    .map_err(|e| e.to_string())
            }
            Self::LoadGlbMotion(path) => AnimatedGlb::read(&path)
                .map(Completed::MotionGlb)
                .map_err(|e| e.to_string()),
            Self::LoadCafLayer(source, base, label, path) => {
                let layer = read_native_caf(&path).map_err(|e| e.to_string())?;
                if layer.additive {
                    return Err("Additive CAF layers require a verified additive convention; select a non-additive clip.".into());
                }
                Ok(Completed::CafLayer(source, base, label, layer))
            }
            Self::LoadCaf(source) => read_native_caf(&source)
                .map(Completed::Caf)
                .map_err(|e| e.to_string()),
            Self::LoadDeclaredDba(virtual_path, inventory, imported_root, build, converter) => {
                let source = auto_resources::resolve_declared_dba(
                    &inventory,
                    &virtual_path,
                    imported_root.as_deref(),
                    build.as_deref(),
                )?
                .ok_or("Declared DBA is not present in verified extracted assets")?;
                let local_root =
                    std::fs::canonicalize(&inventory.root).map_err(|e| e.to_string())?;
                let root = if source.starts_with(&local_root) {
                    local_root
                } else if let Some(library_root) = imported_root {
                    let library_root =
                        std::fs::canonicalize(library_root).map_err(|e| e.to_string())?;
                    if !source.starts_with(&library_root) {
                        return Err("DBA escaped verified local and imported asset roots".into());
                    }
                    let files = source
                        .ancestors()
                        .find(|part| {
                            part.starts_with(&library_root)
                                && part.file_name().is_some_and(|f| f == "files")
                        })
                        .ok_or("Imported DBA has no verified extracted files root")?;
                    files.to_path_buf()
                } else {
                    return Err("DBA has no verified extracted source root".into());
                };
                let temp = Arc::new(
                    tempfile::Builder::new()
                        .prefix("evolve-verified-dba-")
                        .tempdir()
                        .map_err(|e| e.to_string())?,
                );
                let dest = temp.path().join("tracks");
                let report = convert_original_motion(&converter, &source, &root, &dest)
                    .map_err(|e| e.to_string())?;
                Ok(Completed::DbaConverted(report, dest, temp))
            }
            Self::ResolveMaterial(source, root, imported_root, build_label) => {
                let authored = read_material(&source).map_err(|e| e.to_string())?;
                let library = scan_directory(&root).map_err(|e| e.to_string())?;
                let mut report = resolve_materials(&library, &authored);
                // Honor the source's cooked mip priority across BOTH roots.
                // A local lower-res .dds must never mask a higher-res .dds.0
                // in an exact, build-compatible imported Asset Library.
                for texture in &mut report.textures {
                    for candidate in material_texture_candidates(&texture.authored_path) {
                        let local = evolve_core::resolve_virtual(&library, &candidate);
                        if !matches!(local, AssetResolution::Missing) {
                            texture.resolution = local;
                            break;
                        }
                        let Some(imported_root) = &imported_root else {
                            continue;
                        };
                        texture.resolution = match resolve_imported_reference(
                            imported_root,
                            &candidate,
                            build_label.as_deref(),
                        )
                        .map_err(|e| e.to_string())?
                        {
                            LibraryReferenceStatus::Missing => AssetResolution::Missing,
                            LibraryReferenceStatus::Found(hit) => {
                                AssetResolution::Found(hit.absolute_path)
                            }
                            LibraryReferenceStatus::Ambiguous(hits) => AssetResolution::Ambiguous(
                                hits.into_iter()
                                    .map(|hit| {
                                        format!(
                                            "{} [{}] / {}",
                                            hit.archive_name, hit.archive_id, hit.relative_path
                                        )
                                    })
                                    .collect(),
                            ),
                        };
                        if !matches!(texture.resolution, AssetResolution::Missing) {
                            break;
                        }
                    }
                }
                let get_pixels = |roles: &[&str]| -> Result<Option<::image::RgbaImage>, String> {
                    if authored.slots.len() != 1 {
                        return Ok(None); // Never mix different material slots.
                    }
                    let candidates = report
                        .textures
                        .iter()
                        .filter(|r| roles.contains(&r.role.as_str()))
                        .collect::<Vec<_>>();
                    if candidates.len() != 1 {
                        return Ok(None); // Missing or multiple authored maps.
                    }
                    let evolve_core::AssetResolution::Found(relative) = &candidates[0].resolution
                    else {
                        return Ok(None);
                    };
                    let relative_path = Path::new(relative);
                    let source = if relative_path.is_absolute() {
                        relative_path.to_path_buf()
                    } else {
                        root.join(relative_path)
                    };
                    let metadata = std::fs::symlink_metadata(&source).map_err(|e| e.to_string())?;
                    let canonical = std::fs::canonicalize(&source).map_err(|e| e.to_string())?;
                    let within_local = canonical
                        .starts_with(std::fs::canonicalize(&root).map_err(|e| e.to_string())?);
                    let within_imported = if let Some(ref library_root) = imported_root {
                        canonical.starts_with(
                            std::fs::canonicalize(library_root).map_err(|e| e.to_string())?,
                        )
                    } else {
                        false
                    };
                    if !metadata.file_type().is_file() || (!within_local && !within_imported) {
                        return Err("Resolved DDS path escaped all verified asset roots".into());
                    }
                    let (_, image) = decode_dds_rgba(&source).map_err(|e| e.to_string())?;
                    Ok(Some(image))
                };
                // Missing/corrupt optional streams must not discard every
                // successfully decoded map. Keep accurate source diagnostics.
                let mut diagnostics = Vec::new();
                if authored.slots.len() != 1 {
                    diagnostics.push(format!(
                        "{} material slots require verified CryEngine mesh-subset assignments; no cross-slot textures were applied",
                        authored.slots.len()
                    ));
                }
                let mut try_map = |role: &str, names: &[&str]| match get_pixels(names) {
                    Ok(image) => image,
                    Err(error) => {
                        diagnostics.push(format!("{role}: {error}"));
                        None
                    }
                };
                let diffuse = try_map("Diffuse", &["diffuse"]);
                let normal = try_map("Normal", &["normals", "normal", "bumpmap"]);
                let emissive = try_map("Emissive", &["emittance", "emissive", "emission", "glow"]);
                let specular = try_map("Specular", &["specular", "specularcolor"]);
                let gloss = try_map("Gloss", &["gloss", "glossiness", "smoothness"]);
                Ok(Completed::MaterialResolved(
                    source,
                    report,
                    diffuse,
                    normal,
                    emissive,
                    specular,
                    gloss,
                    diagnostics,
                ))
            }
            Self::LoadModelSurfaces(model, material, inventory, imported, build, cache) => {
                let (surfaces, warnings) = assembly_materials::load_model(
                    &model,
                    &material,
                    &inventory,
                    imported.as_deref(),
                    build.as_deref(),
                    &cache,
                )?;
                Ok(Completed::ModelSurfaces(model, surfaces, warnings))
            }
            Self::LoadNormal(source, cache) => {
                decoded_dds(&cache, &source).map(|(info, image)| Completed::Normal(info, image))
            }
            Self::LoadEmissive(source, cache) => {
                decoded_dds(&cache, &source).map(|(info, image)| Completed::Emissive(info, image))
            }
            Self::LoadDiffuse(source, cache) => {
                decoded_dds(&cache, &source).map(|(info, image)| Completed::Diffuse(info, image))
            }
            Self::InspectDds(source) => inspect_dds(&source)
                .map(Completed::Texture)
                .map_err(|e| e.to_string()),
            Self::PreviewDds(source, relative, inventory, library, build, cache) => {
                let (info, pixels) = decoded_dds_indexed(
                    &cache,
                    &source,
                    &relative,
                    &inventory,
                    library.as_deref(),
                    build.as_deref(),
                )?;
                Ok(Completed::DdsPreview(source, info, pixels))
            }
            Self::DiscoverModelResources(relative, inventory, library_root, build) => {
                let discovery = auto_resources::discover(
                    &relative,
                    &inventory,
                    library_root.as_deref(),
                    build.as_deref(),
                );
                Ok(Completed::ModelResources(relative, discovery))
            }
            Self::LoadDeclaredClip(index, virtual_path, inventory, library_root, build) => {
                let file = auto_resources::resolve_declared(
                    &inventory,
                    &virtual_path,
                    library_root.as_deref(),
                    build.as_deref(),
                )?
                .ok_or_else(|| format!("Declared CAF is not available: {virtual_path}"))?;
                let motion = read_native_caf(&file).map_err(|e| e.to_string())?;
                Ok(Completed::DeclaredClip(index, virtual_path, motion))
            }
            Self::InspectModel(source) => {
                let render_source = preferred_render_source(&source);
                read_model(&render_source)
                    .map(|mesh| Completed::Model(mesh, render_source.clone()))
                    .map_err(|e| format!("{}: {e}", render_source.display()))
            }
            Self::InspectIndexedModel(source, virtual_path, inventory, imported, build) => {
                let render_source = auto_resources::exact_render_companion(
                    &virtual_path,
                    &inventory,
                    imported.as_deref(),
                    build.as_deref(),
                )?
                .unwrap_or(source);
                read_model(&render_source)
                    .map(|mesh| Completed::Model(mesh, render_source.clone()))
                    .map_err(|e| format!(
                        "Unable to preview original model {}: {e}. For a .skin, index its exact .skinm compiled render companion in the same build.",
                        render_source.display(),
                    ))
            }
            Self::InspectLevel(source) => {
                let checksum = level_source_hash(&source)?;
                let scene = load_level_scene(&source).map_err(|e| e.to_string())?;
                if level_source_hash(&source)? != checksum {
                    return Err("Level source changed during inspection; reload it".into());
                }
                Ok(Completed::Level(scene, source, checksum))
            }
            Self::ExportEditedLevel(source, destination, edits, protected, expected_sha) => {
                evolve_core::export_edited_layers_guarded(
                    &source,
                    &destination,
                    &edits,
                    &protected,
                    &expected_sha,
                )
                .map_err(|e| e.to_string())?;
                Ok(Completed::EditedLevelExported(destination, edits.len()))
            }
            Self::InspectTerrain(source) => {
                let terrain = read_terrain(&source, 129).map_err(|e| e.to_string())?;
                let info = terrain.info.clone();
                let preview = RenderMesh {
                    info: ModelInfo {
                        format: "CryEngine Heightmap.dat".into(),
                        version: 1,
                        chunk_count: 0,
                        vertex_count: terrain.vertices.len(),
                        triangle_count: terrain.indices.len() / 3,
                        original_uv0: true,
                        original_normals: true,
                    },
                    positions: terrain.vertices,
                    indices: terrain.indices,
                    uv0: Some(terrain.uv),
                    normals: Some(terrain.normals),
                };
                Ok(Completed::Terrain(preview, info))
            }
            Self::InspectSignedPak(source, stage, name) => {
                let key = modding_keys::verified_archive_key(&stage, &source, &name)?;
                inspect_signed_pak(&source, &key, &name)
                    .map(Completed::SignedPak)
                    .map_err(|e| e.to_string())
            }
            Self::ExtractSignedPak(source, stage, name, dest) => {
                let key = modding_keys::verified_archive_key(&stage, &source, &name)?;
                extract_signed_pak(&source, &key, &name, &dest)
                    .map(|i| Completed::ExtractedSigned(i.entries.len(), dest))
                    .map_err(|e| e.to_string())
            }
            Self::ResignSignedPak(source, stage, new_private, new_public, basename, dest) => {
                let original_key = modding_keys::verified_archive_key(&stage, &source, &basename)?;
                let report = resign_signed_pak(
                    &source,
                    &original_key,
                    &new_private,
                    &new_public,
                    &basename,
                    &dest,
                )
                .map_err(|e| e.to_string())?;
                Ok(Completed::ResignedSignedPak(report, dest))
            }
            Self::PrepareClientHook(original, old_key, new_key, dest) => {
                let report = prepare_client_hook(&original, &old_key, &new_key, &dest)
                    .map_err(|e| e.to_string())?;
                Ok(Completed::HookPrepared(report, dest))
            }
            Self::VerifySignedHookPair(
                original_pak,
                custom_pak,
                original_hook,
                custom_hook,
                original_key,
                custom_key,
                basename,
            ) => {
                let report = inspect_pak_hook_pair(
                    &original_pak,
                    &custom_pak,
                    &original_hook,
                    &custom_hook,
                    &original_key,
                    &custom_key,
                    &basename,
                )
                .map_err(|e| e.to_string())?;
                Ok(Completed::SignedHookPreflight(report))
            }
            Self::ResolveCdf(source, asset_root) => {
                let definition = read_cdf(&source).map_err(|e| e.to_string())?;
                let inventory = scan_directory(&asset_root).map_err(|e| e.to_string())?;
                Ok(Completed::CharacterDependencies(resolve_cdf(
                    &inventory,
                    &definition,
                )))
            }
            Self::InspectCdf(source) => read_cdf(&source)
                .map(Completed::Character)
                .map_err(|e| e.to_string()),
            Self::BuildStaticCdf(
                source,
                inventory,
                imported,
                build,
                cache,
                equipment,
                glow,
                kala_tubing,
                progress,
            ) => {
                progress.store(5, Ordering::Relaxed);
                crash_diagnostics::record("cdf_worker_begin", "reading authored CDF attachments");
                let (mesh, report) = evolve_core::read_static_cdf_assembly_with_kala_state(
                    &source,
                    &inventory,
                    imported.as_deref(),
                    build.as_deref(),
                    &equipment,
                    kala_tubing,
                )
                .map_err(|e| e.to_string())?;
                crash_diagnostics::record(
                    "cdf_geometry_ready",
                    &format!(
                        "{} parts, {} vertices, {} triangles",
                        report.original_parts.len(),
                        report.vertices,
                        report.triangles
                    ),
                );
                progress.store(35, Ordering::Relaxed);
                crash_diagnostics::record("cdf_materials_begin", "loading authored material maps");
                let (mut surfaces, mut warnings) = assembly_materials::load_with_progress(
                    &report,
                    &inventory,
                    imported.as_deref(),
                    build.as_deref(),
                    &cache,
                    Some(&progress),
                    kala_tubing,
                );
                progress.store(90, Ordering::Relaxed);
                // Historical CryTek 0x745 monster geometry is the 2012 build:
                // it has no Stage 2 armor-glow/damage toggles.
                // Glow overlay is *only* the optional monster
                // *_glow_diff overlay. Other authored emissive textures,
                // GlowAmount and legacy alpha emission remain unchanged when
                // the overlay checkbox is off.
                if glow && !historical_2012_cdf(&report) {
                    match evolve_core::monster_glow_path_with_library(
                        &source,
                        &inventory,
                        imported.as_deref(),
                        build.as_deref(),
                    ) {
                        Ok(Some(texture)) => {
                            // A monster glow from dds_low.pak may need its
                            // original highest mip in the matching dds_high.pak.
                            // Resolve the actual indexed virtual path and let
                            // the same verified cross-archive decoder used for
                            // the body materials assemble every required mip.
                            let virtual_glow = texture
                                .strip_prefix(&inventory.root)
                                .ok()
                                .or_else(|| {
                                    texture
                                        .ancestors()
                                        .find(|parent| parent.file_name().is_some_and(|name| name == "files"))
                                        .and_then(|folder| texture.strip_prefix(folder).ok())
                                })
                                .map(|relative| relative.to_string_lossy().replace('\\', "/"));
                            let decoded = virtual_glow
                                .as_deref()
                                .ok_or_else(|| "Glow DDS outside the indexed Asset Library".to_owned())
                                .and_then(|virtual_name| {
                                    decoded_dds_indexed(
                                        &cache, &texture, virtual_name, &inventory,
                                        imported.as_deref(), build.as_deref()
                                    ).map(|(_, image)| image)
                                });
                            match decoded {
                            Ok(rgba) => {
                                let cdf_dir = source
                                    .strip_prefix(&inventory.root)
                                    .map_err(|_| "Monster CDF outside indexed source root")?
                                    .parent()
                                    .ok_or("Selected CDF lacks a character folder")?
                                    .to_string_lossy()
                                    .replace('\\', "/")
                                    .to_ascii_lowercase();
                                let owner = cdf_dir
                                    .find("characters/monsters/")
                                    .map(|position| &cdf_dir[position..])
                                    .ok_or("Selected CDF is not a monster")?;
                                let mut mapped = 0usize;
                                let mut derived = HashMap::<usize, Arc<::image::RgbaImage>>::new();
                                for (part, surface) in report.source_parts.iter().zip(surfaces.iter_mut()) {
                                    let binding = part.binding.replace('\\', "/").to_ascii_lowercase();
                                    if surface.visible && binding.contains(owner)
                                        && let Some(base) = surface.diffuse.as_ref()
                                    {
                                        let key = Arc::as_ptr(base) as usize;
                                        let image = if let Some(existing) = derived.get(&key) {
                                            Arc::clone(existing)
                                        } else {
                                            let pixels = Arc::new(assembly_materials::armor_emission(base, &rgba)?);
                                            derived.insert(key, Arc::clone(&pixels));
                                            pixels
                                        };
                                        // Separate armor-overlay channel: do not
                                        // replace the material's existing Emittance
                                        // or original per-slot GlowAmount.
                                        surface.armor_emissive = Some(image);
                                        mapped += 1;
                                    }
                                }
                                warnings.push(format!(
                                    "Glow: {mapped} source-verified overlay sections (original Emittance and GlowAmount preserved)"
                                ));
                            }
                            Err(error) => warnings.push(format!(
                                "Glow overlay could not be decoded for this monster: {error}; original materials preserved"
                            )),
                            }
                        }
                        Ok(None) => warnings.push(
                            "Glow: no source-verified variant DDS found in the selected Asset Library/build; original emissive materials remain visible".into(),
                        ),
                        Err(error) => warnings.push(format!(
                            "Glow source could not be uniquely resolved: {error}; retaining original material emission"
                        )),
                    }
                }
                progress.store(98, Ordering::Relaxed);
                crash_diagnostics::record(
                    "cdf_worker_ready",
                    &format!(
                        "{} GPU material ranges, {} diagnostics",
                        surfaces.len(),
                        warnings.len()
                    ),
                );
                Ok(Completed::StaticCharacter(mesh, report, surfaces, warnings))
            }
            Self::PrepareCdfDamage(source, report, base, inventory, imported, build, cache) => {
                let mut full = base.as_ref().clone();
                let _ = assembly_materials::apply_damage_blend(
                    &report,
                    &mut full,
                    100,
                    &inventory,
                    imported.as_deref(),
                    build.as_deref(),
                    &cache,
                )?;
                Ok(Completed::PreparedCdfDamage(source, full))
            }
            Self::ReblendCdfDamage(source, base, full, percent) => {
                let blend = assembly_materials::blend_prepared_damage(&base, &full, percent)?;
                Ok(Completed::ReblendedCdfDamage(source, percent, blend))
            }
            Self::RebuildSignedPak(
                src,
                key_stage,
                signing_key,
                new_pub,
                name,
                member,
                replacement,
                dest,
            ) => {
                let original_key = modding_keys::verified_archive_key(&key_stage, &src, &name)?;
                let result = rebuild_signed_pak_entry(
                    &src,
                    &original_key,
                    &signing_key,
                    &new_pub,
                    &name,
                    &member,
                    &replacement,
                    &dest,
                )
                .map_err(|e| e.to_string())?;
                Ok(Completed::RebuiltSigned(result.changed_entry, dest))
            }
            Self::ApplyProfile(stage, id, original, new_public) => {
                apply_profile_stage(&stage, &id, &original, &new_public)
                    .map(Completed::ProfileApplied)
                    .map_err(|e| e.to_string())
            }
            Self::RecoverProfile(stage) => restore_profile_stage(&stage)
                .map(Completed::ProfileRecovered)
                .map_err(|e| e.to_string()),
            Self::StatusProfile(stage) => batch_stage_status(&stage)
                .map(Completed::ProfileStatus)
                .map_err(|e| e.to_string()),
            Self::ListProfiles(root) => list_mod_profiles(&root)
                .map(Completed::ProfileCatalog)
                .map_err(|e| e.to_string()),
            Self::CreateProfile(root, label) => create_mod_profile(&root, &label)
                .map(Completed::ProfileCreated)
                .map_err(|e| e.to_string()),
            Self::AddProfileMod(stage, id, source, target, original, public, basename) => {
                add_mod_to_profile(&stage, &id, &source, &target, &original, &public, &basename)
                    .map(Completed::ProfileAdded)
                    .map_err(|e| e.to_string())
            }
            Self::PlanProfile(stage, id, original, public) => {
                plan_mod_profile(&stage, &id, &original, &public)
                    .map(Completed::ProfilePlanned)
                    .map_err(|e| e.to_string())
            }
            Self::CheckExistingOfflineLauncher(stage, swap) => {
                let report = inspect_existing_launcher(&stage, &swap)?;
                Ok(Completed::OfflineLauncherReady(report))
            }
            Self::StartExistingOfflineLauncher(stage, swap) => {
                let report = inspect_existing_launcher(&stage, &swap)?;
                let count = report.archive_count;
                let id = launch_existing_offline_client(&report)?;
                Ok(Completed::OfflineLauncherStarted(count, id))
            }
            Self::RestoreExistingOfflineLauncher(stage, swap) => {
                let (count, reusable) = restore_existing_offline_client(&stage, &swap)?;
                Ok(Completed::OfflineLauncherRestored(count, reusable))
            }
            Self::StageInit(root) => {
                initialize_stage(&root).map_err(|e| e.to_string())?;
                Ok(Completed::StageInitialized(root))
            }
            Self::StageStatus(root) => stage_status(&root)
                .map(Completed::StageStatus)
                .map_err(|e| e.to_string()),
            Self::StageImport(root, source, relative, key, basename) => {
                import_stage_pak(&root, &source, &relative, &key, &basename)
                    .map(Completed::StageImported)
                    .map_err(|e| e.to_string())
            }
            Self::StageApply(root, source, relative, old_key, new_key, basename) => {
                apply_stage_pak(&root, &source, &relative, &old_key, &new_key, &basename)
                    .map(Completed::StageApplied)
                    .map_err(|e| e.to_string())
            }
            Self::StageRestore(root) => restore_stage(&root)
                .map(Completed::StageRestored)
                .map_err(|e| e.to_string()),
            Self::ExportLevelFbx(src, root, terrain, blender, dest) => {
                let folder = tempfile::tempdir().map_err(|e| e.to_string())?;
                let scene_glb = folder.path().join("level-scene.glb");
                let scene = export_level_scene_glb(&src, &root, terrain.as_deref(), &scene_glb)
                    .map_err(|e| e.to_string())?;
                convert_glb_to_fbx(&blender, &scene_glb, &dest, AnimationPolicy::default())
                    .map_err(|e| e.to_string())?;
                Ok(Completed::LevelFbxExported(
                    scene.exported_mesh_resources,
                    scene.exported_mesh_instances,
                    dest,
                ))
            }
            Self::ExportLevelGlb(src, root, terrain, dest) => {
                let result = export_level_scene_glb(&src, &root, terrain.as_deref(), &dest)
                    .map_err(|e| e.to_string())?;
                Ok(Completed::LevelExported(
                    result.exported_mesh_resources,
                    result.exported_mesh_instances,
                    dest,
                ))
            }
            Self::ExtractPak(source, dest) => extract_legacy_pak(&source, &dest)
                .map(|pak| Completed::Extracted(pak.entries.len(), dest))
                .map_err(|e| e.to_string()),
            Self::RebuildPak(source, overrides, dest) => {
                rebuild_legacy_pak(&source, &overrides, &dest)
                    .map(|report| Completed::Rebuilt(report.changed_entries, dest))
                    .map_err(|e| e.to_string())
            }
            Self::ExportPng(source, dest) => export_dds_png(&source, &dest)
                .map(|info| Completed::ExportedPng(info, dest))
                .map_err(|e| e.to_string()),
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    Navigate(Page),
    LibraryImportSourceChanged(String),
    LibraryImportStart,
    SettingsTab(settings_page::Tab),
    LibrarySearchChanged(String),
    LibraryMenu(&'static str),
    ModdingMenu(&'static str),
    DismissMenus,
    MenuCommand(Box<Message>),
    ModdingFilterChanged(String),
    ModdingPage(bool),
    ModdingChangesPage(bool),
    ModdingSelectProject(usize),
    ModdingRefresh,
    ModdingToggleAdvanced,
    ModdingBrowsePaks,
    ModdingSelectArchive(usize),
    ModdingOpenProject,
    ModdingReviewChanges,
    ModdingBuild,
    ModdingUnpackPak,
    EditorReplacementChanged(String),
    EditorImportRawMember,
    EditorRestoreRawMember,
    ModdingToggleProfiles,
    OpenLibrarySettings,
    LibraryToggleConfiguration,
    LibraryRefresh,
    LibraryOpenFolder,
    LibraryExport,
    StartupLibraryIndexed(PathBuf, Result<ImportedLibrary, String>),
    LibrarySetByBuild(bool),
    LibraryNextBuild,
    LibrarySelect(usize),
    LibraryPreview,
    LibraryMakeModProject,
    LibraryPreviousPage,
    LibraryNextPage,
    OpenViewer,
    OpenViewerPreferences,
    ViewerSidebarAnimations(bool),
    ViewerSidebarSelect(ViewerSidebarTab),
    ViewerSidebarClose(ViewerSidebarTab),
    ViewerEquip(String, String),
    ViewerKalaTubingState(KalaTubingState),
    ViewerToggleMonsterGlow,
    ViewerEmissiveStrengthChanged(f32),
    ViewerToggleDamageBlend,
    ViewerDamageBlendChanged(u8),
    ViewerDamageBlendCommit,
    ViewerSplitDrag(f32),
    OpenInspectorWindow,
    InspectorOpened,
    ViewerOpened,
    ActivityOpened,
    WindowClosed(window::Id),
    FileDropped(window::Id, PathBuf),
    DropPath(PathBuf),
    CancelScan,
    ScanPulse,
    ViewerMenu(&'static str),
    ViewerToggleExplorer,
    ViewerResetCamera,
    ViewerCameraPreset(&'static str),
    ViewerOrbitDelta(f32, f32),
    ViewerPanDelta(f32, f32),
    ViewerZoomWheel(f32),
    ViewerSearchChanged(String),
    ViewerPreviousPage,
    ViewerNextPage,
    ViewerSelectFile(usize),
    ViewerContextFile(usize),
    ViewerContextClose,
    ViewerCopySelectedPath,
    ViewerRevealSelectedFile,
    ModifiersChanged(iced::keyboard::Modifiers),
    ViewerToggleFolder(String),
    ViewerPreviewSelected,
    GfxOpenSelected,
    GfxOpened(Result<gfx_editor::Session, String>),
    GfxConfigChanged(String),
    GfxPickBackend,
    GfxBackendPicked(Option<PathBuf>),
    GfxSelectResource(usize),
    GfxToggleLayout,
    GfxToggleLayoutRaster,
    GfxLayoutSelect(usize),
    GfxLayoutDragged(usize, f32, f32),
    GfxLayoutValue(&'static str, String),
    GfxLayoutVisible(bool),
    GfxStageLayout,
    GfxToggleLiveInspector,
    GfxImportUiSnapshot,
    GfxUiSnapshotPicked(Option<PathBuf>),
    GfxUiSnapshotLoaded(Result<gfx_live_inspector::UiSnapshot, String>),
    GfxAction(gfx_editor::Action),
    GfxApply,
    GfxPickReplacement,
    GfxReplacementPicked(Option<PathBuf>),
    GfxPickEdited,
    GfxEditedPicked(Option<PathBuf>),
    GfxRevealSession,
    GfxFinished(Result<(gfx_editor::Session, String, Option<Vec<u8>>), String>),
    ViewerAutoClip(usize),
    ViewerAutoClipFilter(String),
    ViewerToggleExportAnimations,
    ExportOpen,
    ExportClose,
    ExportFilterChanged(String),
    ExportFolderChanged(String),
    ExportToggleFile(usize),
    ExportToggleRigged,
    ExportToggleEmbedded,
    ExportToggleDependencies,
    ExportBegin,
    ExportBrowse(&'static str),
    ExportPicked(&'static str, Option<PathBuf>),
    ViewerPreviewStaticCdf,
    ViewerOpenStaticCdf(usize),
    StartDependencyInspection,
    CompareOriginalChanged(String),
    CompareModifiedChanged(String),
    StartAssetCompare,
    ViewerToggleInspector,
    ViewerSelectTab(ViewerTab),
    ViewerSaveLayout,
    PlaybackSpeedChanged(f32),
    PlaybackLoopChanged,
    ViewerToggleWireframe,
    LightStrengthChanged(f32),
    LevelSelect(usize),
    LevelFilterChanged(String),
    LevelZoomChanged(f32),
    LevelEditPositionChanged(String),
    LevelEditRotationChanged(String),
    LevelEditScaleChanged(String),
    LevelEditOutputChanged(String),
    LevelApplyTransform,
    LevelUndoTransform,
    LevelRedoTransform,
    LevelExportEdits,
    ToggleAdvancedTools,
    ToggleActivity,
    ClearActivity,
    ToggleSystemHealth,
    CheckSystemHealth,
    SystemHealthChecked(system_health::Snapshot),
    DeveloperPasswordChanged(String),
    UnlockDeveloper,
    DeveloperChecked(bool),
    LockDeveloper,
    InputChanged(String),
    OutputChanged(String),
    ReplacementsChanged(String),
    SignedMemberChanged(String),
    SignedReplacementChanged(String),
    StartStageInit,
    StartStageStatus,
    StartStageImport,
    StartStageApply,
    StartStageRestore,
    StartOfflineLauncherCheck,
    StartOfflineLauncherLaunch,
    StartOfflineLauncherRestore,
    ProfileNameChanged(String),
    SelectProfile(String),
    StartProfileList,
    StartProfileCreate,
    StartProfileAdd,
    StartProfilePlan,
    StartProfileApply,
    StartProfileRecover,
    StartProfileStatus,
    DiffuseInputChanged(String),
    NormalInputChanged(String),
    EmissiveInputChanged(String),
    MaterialInputChanged(String),
    StartMaterialResolve,
    CafInputChanged(String),
    SkeletonChanged(String),
    BlenderChanged(String),
    ConverterPathChanged(String),
    OpenConvertedGlb(usize),
    SelectMotionClip(usize),
    MotionScrub(f32),
    ToggleMotion,
    Tick,
    PreferenceDensity,
    PreferenceExplorerSide,
    PreferenceAppearance,
    PreferenceAccentPreset(&'static str),
    PreferenceReduceMotion,
    PreferenceSnapshots,
    PreferenceActivity,
    PreferenceCrashLogging,
    OpenCrashLogs,
    OpenCrashLogsFolder,
    PreferenceUpdateChannel,
    PreferenceAutoUpdates,
    PreferenceStage(String),
    PreferenceGameStorage(String),
    SettingsToggleAdvancedPaths,
    SettingsAnalyzeStorage,
    PreferenceSwap(String),
    PreferenceProjects(String),
    PreferenceAssets(String),
    PreferenceBlender(String),
    PreferenceWorkspace(String),
    SavePreferences,
    CheckUpdates,
    UpdateChecked(Result<Option<ReleaseUpdate>, String>),
    ApplyUpdate,
    UpdatePrepared(Result<PreparedUpdate, String>),
    AssetsRootChanged(String),
    TerrainSourceChanged(String),
    YawChanged(f32),
    PitchChanged(f32),
    ZoomChanged(f32),
    SearchChanged(String),
    StartScan,
    StartPakInspection,
    StartDdsInspection,
    StartDiffuseLoad,
    StartNormalLoad,
    StartEmissiveLoad,
    StartCafLoad,
    ViewerLayerClip(usize),
    ViewerLayerWeight(f32),
    ViewerClearLayer,
    StartRigGlb,
    StartRigFbx,
    StartMotionConversion,
    StartCdfInspection,
    StartCdfResolve,
    StartModelInspection,
    StartLevelInspection,
    StartTerrainInspection,
    StartSignedInspection,
    StartSignedExtraction,
    StartSignedRebuild,
    StartSignedResign,
    HookOriginalChanged(String),
    HookPreparedChanged(String),
    StartHookPrepare,
    StartHookPreflight,
    StartLevelGlbExport,
    StartLevelFbxExport,
    StartExtract,
    StartRebuild,
    StartPngExport,
    Finished(Result<Completed, String>),
}

fn level_source_hash(source: &Path) -> Result<String, String> {
    use std::io::Read;
    let file = std::fs::File::open(source).map_err(|e| e.to_string())?;
    let mut reader = std::io::BufReader::new(file);
    let mut hash = Sha256::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let count = reader.read(&mut chunk).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        hash.update(&chunk[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

/// Only exact current model meshes are eligible for verified CAF playback.
/// Decode and bind in a worker, never during the GUI draw or input event.
fn bind_current_native_caf(state: &mut State) -> Option<Task<Message>> {
    let motion = state.native_caf.as_ref()?;
    if state.model.is_none() || state.skeleton_path.trim().is_empty() {
        return None;
    }
    let source = PathBuf::from(&state.input);
    let extension = source.extension().and_then(|ext| ext.to_str())?;
    if extension.eq_ignore_ascii_case("cdf") {
        let report = state.cdf_report.as_ref()?.clone();
        let rest = state.cdf_rest_mesh.as_ref()?.clone();
        let clip = (**motion).clone();
        return Some(launch(
            state,
            Operation::BindCdfCafPreview(source, rest, report, clip),
        ));
    }
    if !["skin", "skinm", "chr", "chrm"]
        .iter()
        .any(|candidate| extension.eq_ignore_ascii_case(candidate))
    {
        return None;
    }
    let compiled = state
        .render_source
        .clone()
        .unwrap_or_else(|| preferred_render_source(&source));
    let skeleton = PathBuf::from(&state.skeleton_path);
    let clip = (**motion).clone();
    Some(launch(
        state,
        Operation::BindCafPreview(compiled, skeleton, clip),
    ))
}

/// Re-evaluate the actual original vertex weights while the existing wgpu
/// viewport continues rendering textured geometry. Source geometry, UVs and
/// on-disk assets are never modified by the animation preview.
fn apply_current_skin_frame(state: &mut State) {
    let layers = state
        .caf_layer
        .as_ref()
        .map(|motion| evolve_core::CafLayer {
            motion,
            seconds: state.motion_time,
            weight: state.caf_layer_weight,
        })
        .into_iter()
        .collect::<Vec<_>>();
    if let Some(assembly) = &state.cdf_caf_preview {
        match assembly.mesh_at_layers(state.motion_time, &layers) {
            Ok(mesh) => state.model = Some(Arc::new(mesh)),
            Err(error) => {
                state.playing = false;
                state.status = format!("Whole-character CAF playback stopped: {error}");
            }
        }
    } else if let Some(skin) = &state.skin_preview {
        match skin.mesh_at_layers(state.motion_time, &layers) {
            Ok(mesh) => state.model = Some(Arc::new(mesh)),
            Err(error) => {
                state.playing = false;
                state.status = format!("CAF playback stopped: {error}");
            }
        }
    }
}

/// Authoring baselines are captured once when loading the source layer.
/// A complete undo leaves zero edited placements, not 200,000 dirty entries.
fn level_pending_edits(state: &State) -> Vec<(String, evolve_core::Transform)> {
    let Some(scene) = &state.level_scene else {
        return Vec::new();
    };
    scene
        .objects
        .iter()
        .filter_map(|object| {
            state
                .level_original_transforms
                .get(&object.id)
                .and_then(|original| {
                    (object.local != *original).then_some((object.id.clone(), object.local))
                })
        })
        .collect()
}

fn track_level_dirty(
    state: &mut State,
    id: &str,
    previous: evolve_core::Transform,
    next: evolve_core::Transform,
) {
    if let Some(original) = state.level_original_transforms.get(id) {
        match (previous != *original, next != *original) {
            (false, true) => state.level_dirty_count += 1,
            (true, false) => state.level_dirty_count = state.level_dirty_count.saturating_sub(1),
            _ => (),
        }
    }
}

/// Exactly the compiled CryEngine mesh families supported by the original
/// renderer and its same-stem .skinm/.chrm/.cgfm companion preference. Do not
/// force the user to press Inspect for compiled-only model variants.
fn is_viewer_model_source(path: &str) -> bool {
    let name = path.to_ascii_lowercase();
    [
        ".skin", ".skinm", ".chr", ".chrm", ".cgf", ".cgfm", ".cga", ".cgam",
    ]
    .iter()
    .any(|suffix| name.ends_with(suffix))
}

fn is_viewer_preview_source(path: &str) -> bool {
    let name = path.to_ascii_lowercase();
    is_viewer_model_source(&name)
        || [".cdf", ".dds", ".caf", ".mtl", ".gfx"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
        || name.rsplit_once(".dds.").is_some_and(|(_, suffix)| {
            let mip = suffix.strip_suffix('a').unwrap_or(suffix);
            mip.parse::<u8>().is_ok_and(|n| n < 20)
        })
}

fn sync_level_fields(state: &mut State) {
    let transform = state
        .level_selected
        .and_then(|index| state.level_scene.as_ref()?.objects.get(index))
        .map(|object| object.local);
    if let Some(transform) = transform {
        (
            state.level_edit_position,
            state.level_edit_rotation,
            state.level_edit_scale,
        ) = level_workspace::format_transform(transform);
    } else {
        state.level_edit_position.clear();
        state.level_edit_rotation.clear();
        state.level_edit_scale.clear();
    }
}

fn restore_level_history(state: &mut State, undo: bool) {
    let entry = if undo {
        state.level_undo.pop_back()
    } else {
        state.level_redo.pop_back()
    };
    let Some(diff) = entry else {
        return;
    };
    let target = if undo { diff.old_local } else { diff.new_local };
    let previous = if undo { diff.new_local } else { diff.old_local };
    let result = state.level_scene.as_ref().and_then(|scene| {
        let current = scene
            .objects
            .iter()
            .find(|object| object.id == diff.guid)?
            .local;
        let expected = if undo { diff.new_local } else { diff.old_local };
        (current == expected).then(|| evolve_core::edit_level_transform(scene, &diff.guid, target))
    });
    match result {
        Some(Ok(scene)) => {
            track_level_dirty(state, &diff.guid, previous, target);
            state.level_scene = Some(Arc::new(scene));
            let other = if undo {
                &mut state.level_redo
            } else {
                &mut state.level_undo
            };
            other.push_back(diff);
            sync_level_fields(state);
            state.status = if undo {
                "Reverted the last level transform (original layer unchanged).".into()
            } else {
                "Reapplied the level transform (original layer unchanged).".into()
            };
        }
        _ => {
            if undo {
                state.level_undo.push_back(diff);
            } else {
                state.level_redo.push_back(diff);
            }
            state.status = "Cannot restore level history; reload the level source.".into();
        }
    }
}

fn resolved_blender(state: &mut State) -> Result<PathBuf, String> {
    let path = blender_install::resolve_blender(&state.blender_path)?;
    state.blender_path = path.to_string_lossy().into_owned();
    Ok(path)
}

/// Native OS picker. Never use a freeform source-path field for export choices.
/// Executed off the UI thread. A canceled dialog is intentionally a no-op.
fn native_export_picker(kind: &str) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let script = if kind == "folder" {
            r#"Add-Type -AssemblyName System.Windows.Forms; $d=New-Object System.Windows.Forms.FolderBrowserDialog; $d.Description='Choose export parent folder'; if($d.ShowDialog() -eq 'OK') { [Console]::Write($d.SelectedPath) }"#
        } else {
            r#"Add-Type -AssemblyName System.Windows.Forms; $d=New-Object System.Windows.Forms.OpenFileDialog; $d.Title='Select existing source file'; if($d.ShowDialog() -eq 'OK') { [Console]::Write($d.FileName) }"#
        };
        let output = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-STA", "-Command", script])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let path = PathBuf::from(String::from_utf8(output.stdout).ok()?.trim());
        if (kind == "folder" && path.is_dir()) || (kind != "folder" && path.is_file()) {
            Some(path)
        } else {
            None
        }
    }
    #[cfg(target_os = "macos")]
    {
        let script = if kind == "folder" {
            "POSIX path of (choose folder)"
        } else {
            "POSIX path of (choose file)"
        };
        let output = std::process::Command::new("osascript")
            .args(["-e", script])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        return Some(PathBuf::from(String::from_utf8(output.stdout).ok()?.trim()));
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let arg = if kind == "folder" {
            "--directory"
        } else {
            "--file-selection"
        };
        let output = std::process::Command::new("zenity")
            .args(["--file-selection", arg])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        Some(PathBuf::from(String::from_utf8(output.stdout).ok()?.trim()))
    }
}

fn exportable_asset(relative: &str) -> bool {
    let lower = relative.to_ascii_lowercase();
    lower.ends_with(".dds")
        || lower.ends_with(".dds.0")
        || lower.ends_with(".cdf")
        || [
            ".skin", ".skinm", ".chr", ".chrm", ".cgf", ".cgfm", ".cga", ".cgam",
        ]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

fn launch(state: &mut State, operation: Operation) -> Task<Message> {
    if state.busy {
        return Task::none();
    }
    state.busy = true;
    if !matches!(&operation, Operation::BuildStaticCdf(..)) {
        state.viewer_load_progress = None;
    }
    state.status = "Working on explicitly selected local files...".into();
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || operation.execute())
                .await
                .map_err(|e| format!("Worker failed: {e}"))?
        },
        Message::Finished,
    )
}

/// The target must belong to a selected editable project or to the exact
/// staged original. A replacement's filename alone is never sufficient.
/// Only a single material-slot, uniquely resolved original diffuse stream is
/// eligible as an automatic FBX texture input. The user can still override it.
/// Compiled CryEngine .skin/.chr files can carry skeletal data while their
/// original render positions live in a same-folder .skinm/.chrm companion.
/// The private GPU viewer checks those companions first. Never basename
/// search or select a different archive's unrelated mesh.
fn preferred_render_source(source: &Path) -> PathBuf {
    let Some(extension) = source.extension().and_then(|value| value.to_str()) else {
        return source.to_path_buf();
    };
    let companion_extension = if extension.eq_ignore_ascii_case("skin") {
        "skinm"
    } else if extension.eq_ignore_ascii_case("chr") {
        "chrm"
    } else {
        return source.to_path_buf();
    };
    let sidecar = source.with_extension(companion_extension);
    // Do not follow links or use an unverified sidecar from another directory.
    if let Ok(meta) = std::fs::symlink_metadata(&sidecar)
        && meta.file_type().is_file()
    {
        return sidecar;
    }
    source.to_path_buf()
}

/// Select the source build from the private manager's readable
/// `<PAK name>__<20-char id>/files` folder and build-collections.json.
/// An archive can belong to multiple snapshots; that is not proof of one
/// build, so only a single exact membership is selected automatically.
fn update_layout_drag(state: &mut State, index: usize, x: f32, y: f32) -> Task<Message> {
    let result = update(state, Message::GfxLayoutSelect(index));
    state.gfx_layout_x = format!("{x:.2}");
    state.gfx_layout_y = format!("{y:.2}");
    state.gfx_status =
        "Placement moved on canvas; click Stage layout change to record an undoable edit.".into();
    result
}

fn resource_build(state: &State) -> Option<String> {
    let catalog = state.library_catalog.as_ref()?;
    if state.library_by_build {
        return catalog.builds.get(state.library_build_index).cloned();
    }
    let source = Path::new(&state.inventory.as_ref()?.root);
    if source.file_name()? != "files" {
        return None;
    }
    let archive_folder = source.parent()?;
    let archive = catalog
        .archives
        .iter()
        .find(|item| Path::new(&item.directory) == archive_folder)?;
    (archive.builds.len() == 1).then(|| archive.builds[0].clone())
}

fn verified_export_diffuse(state: &State, report: &MaterialReport) -> Option<PathBuf> {
    if report.slots != 1 {
        return None;
    }
    let mut refs = report.textures.iter().filter(|r| r.role == "diffuse");
    let reference = refs.next()?;
    if refs.next().is_some() {
        return None;
    }
    let AssetResolution::Found(relative) = &reference.resolution else {
        return None;
    };
    let path = PathBuf::from(relative);
    let path = if path.is_absolute() {
        path
    } else {
        PathBuf::from(&state.inventory.as_ref()?.root).join(path)
    };
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.file_type().is_file() {
        return None;
    }
    let source = std::fs::canonicalize(path).ok()?;
    let local = state
        .inventory
        .as_ref()
        .and_then(|i| std::fs::canonicalize(&i.root).ok());
    let library = state
        .library_catalog
        .as_ref()
        .and_then(|i| std::fs::canonicalize(&i.root).ok());
    (local.as_ref().is_some_and(|r| source.starts_with(r))
        || library.as_ref().is_some_and(|r| source.starts_with(r)))
    .then_some(source)
}

fn selected_stage_target(state: &State) -> Result<String, String> {
    let stage = PathBuf::from(&state.preferences.stage);
    if let Some(project) = state.modding_selected.and_then(|index| {
        state
            .modding_projects
            .as_ref()
            .and_then(|projects| projects.get(index))
    }) {
        return modding_keys::staged_target(&stage, &stage.join("paks").join(&project.target));
    }
    modding_keys::staged_target(&stage, PathBuf::from(&state.input).as_path())
}

/// Identify original historical 2012 geometry by its verified CryTek
/// 0x745 render headers, not a directory name (Goliath and Rocky exist in
/// multiple builds). Read only 16 bytes from each already-verified source.
fn historical_2012_cdf(report: &AssemblyReport) -> bool {
    use std::io::Read;
    !report.geometry_parts.is_empty()
        && report.geometry_parts.iter().all(|part| {
            let mut header = [0u8; 16];
            std::fs::File::open(&part.source)
                .and_then(|mut file| file.read_exact(&mut header))
                .is_ok()
                && &header[..8] == b"CryTek\0\0"
                && u32::from_le_bytes(header[12..16].try_into().unwrap_or_default()) == 0x745
        })
}

/// A file or window transition must never retain the previous character's
/// surfaces, textures, animation or CDF report in a new preview.
fn clear_viewer_asset(state: &mut State) {
    state.model = None;
    state.render_source = None;
    state.cdf_surfaces = None;
    state.cdf_base_surfaces = None;
    state.cdf_full_damage_surfaces = None;
    state.cdf_damage_preparing = false;
    state.cdf_legacy_2012 = false;
    state.cdf_rest_mesh = None;
    state.cdf_caf_preview = None;
    state.cdf_report = None;
    state.equipment_source = None;
    state.equipment_slots.clear();
    state.equipment_overrides.clear();
    state.kala_tubing = KalaTubingState::Banshee;
    state.full_armor_glow = false;
    state.emissive_strength = 1.0;
    state.damage_blend_percent = 0;
    state.damage_blend_enabled = false;
    state.viewer_load_progress = None;
    state.texture_preview = None;
    state.texture = None;
    state.material_report = None;
    state.dependency_graph = None;
    state.comparison = None;
    state.character = None;
    state.cdf_dependencies = None;
    state.diffuse = None;
    state.normal_map = None;
    state.emissive_map = None;
    state.specular_map = None;
    state.gloss_map = None;
    state.native_caf = None;
    state.skin_preview = None;
    state.caf_layer = None;
    state.caf_layer_label.clear();
    state.glb_motion = None;
    state.converted_glbs.clear();
    state.converted_dba_cache = None;
    state.auto_clips.clear();
    state.auto_idle_pending = None;
    state.auto_play_native_caf = false;
    state.auto_material_pending = false;
    state.auto_resource_note.clear();
    state.diffuse_status.clear();
    state.playing = false;
    state.motion_time = 0.0;
    state.viewer_context_index = None;
    state.viewer_queued_preview = None;
}

fn begin_scan(state: &mut State, source: PathBuf) -> Task<Message> {
    if state.busy {
        return Task::none();
    }
    clear_viewer_asset(state);
    state.selected = None;
    state.file_selection.clear();
    let cancelled = Arc::new(AtomicBool::new(false));
    let indexed = Arc::new(AtomicUsize::new(0));
    let task = launch(
        state,
        Operation::Scan(source, cancelled.clone(), indexed.clone()),
    );
    state.scan_cancel = Some(cancelled);
    state.scan_indexed = Some(indexed);
    task
}

/// Resolve a selected extracted file without trusting stale indices, links, or
/// an archive member name that merely resembles a local path.
fn verified_viewer_file(state: &State) -> Result<PathBuf, String> {
    let inventory = state.inventory.as_ref().ok_or("No indexed local files")?;
    let selected = state.selected.ok_or("No file selected")?;
    let entry = inventory
        .entries
        .get(selected)
        .ok_or("Selection is stale")?;
    let root = std::fs::canonicalize(&inventory.root).map_err(|e| e.to_string())?;
    let candidate = PathBuf::from(&inventory.root).join(&entry.relative_path);
    let metadata = std::fs::symlink_metadata(&candidate).map_err(|e| e.to_string())?;
    let canonical = std::fs::canonicalize(&candidate).map_err(|e| e.to_string())?;
    if !metadata.file_type().is_file() || !canonical.starts_with(&root) {
        return Err("Selected file is not a verified regular file inside the indexed root".into());
    }
    Ok(canonical)
}

fn persist_preferences(state: &mut State) {
    state.settings_status =
        match settings_path().and_then(|path| save_to(&path, &state.preferences)) {
            Ok(()) => "Preferences saved to your Windows user profile.".into(),
            Err(err) => format!("Preferences could not be saved: {err}"),
        };
}

fn check_updates_task(channel: UpdateChannel) -> Task<Message> {
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                discover_update(channel, env!("CARGO_PKG_VERSION")).map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| format!("Update checker crashed: {e}"))?
        },
        Message::UpdateChecked,
    )
}

/// Rebuild filtering only when the catalog, query or selected snapshot changes.
/// Validates the currently configured Asset Library directory before shell
/// navigation. Never treat a non-directory, missing or relative path as the
/// library (and never create a directory on a mere File menu action).
fn verified_library_directory(configured: &str) -> Result<PathBuf, String> {
    if configured.trim().is_empty() {
        return Err(
            "Asset Library location is not configured. Select it under Settings → Storage.".into(),
        );
    }
    let path = Path::new(configured);
    if !path.is_absolute() {
        return Err("Configured Asset Library location is not an absolute path.".into());
    }
    let canonical = std::fs::canonicalize(path).map_err(|error| {
        format!("Asset Library folder does not exist or cannot be opened: {error}")
    })?;
    if !canonical.is_dir() {
        return Err("Configured Asset Library location is not a directory.".into());
    }
    // Explorer handles the ordinary configured Windows path more reliably
    // than the \\?\\ long-path prefix returned by canonicalize on Windows.
    Ok(path.to_path_buf())
}

fn refresh_library_matches(state: &mut State) {
    let Some(catalog) = &state.library_catalog else {
        state.library_matches.clear();
        return;
    };
    let needle = state.library_search.to_ascii_lowercase();
    let build = state
        .library_by_build
        .then(|| catalog.builds.get(state.library_build_index))
        .flatten();
    state.library_matches = catalog
        .archives
        .iter()
        .enumerate()
        .filter_map(|(i, entry)| {
            if state.library_by_build && !build.is_some_and(|label| entry.builds.contains(label)) {
                return None;
            }
            if !needle.is_empty()
                && !entry.name.to_ascii_lowercase().contains(&needle)
                && !entry.description.to_ascii_lowercase().contains(&needle)
                && !entry.group.to_ascii_lowercase().contains(&needle)
                && !entry.source.to_ascii_lowercase().contains(&needle)
            {
                return None;
            }
            Some(i)
        })
        .collect();
}

fn refresh_viewer_rows(state: &mut State) {
    // Geometry and animation frames must not re-sort the Asset Library.
    let rows: Vec<(usize, &str, u64)> = if let Some(inv) = &state.inventory {
        inv.entries
            .iter()
            .enumerate()
            .map(|(i, e)| (i, e.relative_path.as_str(), e.byte_size))
            .collect()
    } else if let Some(pak) = &state.signed_pak {
        pak.entries
            .iter()
            .enumerate()
            .map(|(i, e)| (i, e.path.as_str(), e.size))
            .collect()
    } else if let Some(pak) = &state.pak {
        pak.entries
            .iter()
            .enumerate()
            .map(|(i, e)| (i, e.path.as_str(), e.size))
            .collect()
    } else {
        Vec::new()
    };
    const PAGE: usize = 350;
    let offset = state.viewer_page.saturating_mul(PAGE);
    let mut visible = explorer_tree::visible_rows_page(
        rows.iter().map(|(i, path, size)| (*i, *path, *size)),
        &state.viewer_search,
        &state.viewer_collapsed,
        offset,
        PAGE + 1,
    );
    state.viewer_more_rows = visible.len() > PAGE;
    visible.truncate(PAGE);
    state.viewer_rows = visible;
}

fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::Navigate(page) => {
            state.page = Some(page);
            if page == Page::Files
                && state.modding_projects.is_none()
                && !state.preferences.projects.trim().is_empty()
            {
                return launch(
                    state,
                    Operation::IndexProjects(
                        PathBuf::from(&state.preferences.projects),
                        PathBuf::from(&state.preferences.stage),
                    ),
                );
            }
            if page == Page::AssetLibrary
                && state.library_catalog.is_none()
                && !state.library_startup_loading
            {
                let root = PathBuf::from(&state.library_root);
                if root.is_dir() {
                    return launch(state, Operation::LoadImportedLibrary(root));
                }
                state.status = "Asset Library is ready at the detected manager data location. Import a local signed PAK to populate it.".into();
            }
        }
        Message::DismissMenus | Message::MenuCommand(_) => {
            state.viewer_menu = None;
            state.modding_menu = None;
            state.library_menu = None;
        }
        Message::ModdingMenu(menu) => {
            state.library_menu = None;
            state.viewer_menu = None;
            state.modding_menu = if state.modding_menu == Some(menu) {
                None
            } else {
                Some(menu)
            };
        }
        Message::ModdingFilterChanged(value) => {
            state.modding_filter = value;
            state.modding_page = 0;
        }
        Message::ModdingPage(next) => {
            let count = modding_page::matching_projects(state).len();
            let last = count.saturating_sub(1) / modding_page::PAGE;
            state.modding_page = if next {
                state.modding_page.saturating_add(1).min(last)
            } else {
                state.modding_page.saturating_sub(1).min(last)
            };
        }
        Message::ModdingChangesPage(next) => {
            let count = state
                .modding_selected
                .and_then(|i| state.modding_projects.as_ref()?.get(i))
                .map_or(0, |p| p.modified.len());
            let last = count.saturating_sub(1) / modding_page::PAGE;
            state.modding_changes_page = if next {
                state.modding_changes_page.saturating_add(1).min(last)
            } else {
                state.modding_changes_page.saturating_sub(1).min(last)
            };
        }
        Message::ModifiersChanged(modifiers) => state.keyboard_modifiers = modifiers,
        Message::ModdingSelectProject(index) => {
            let matches = modding_page::matching_projects(state);
            let range = modding_page::page_range(matches.len(), state.modding_page);
            let visible = &matches[range];
            if !visible.contains(&index) {
                return Task::none();
            }
            state.project_selection.click(
                visible,
                index,
                state.keyboard_modifiers.shift(),
                state.keyboard_modifiers.control(),
            );
            state.modding_selected = state.project_selection.primary_after(index);
            state.modding_changes_page = 0;
            if let Some(project) = state
                .modding_selected
                .and_then(|i| state.modding_projects.as_ref()?.get(i))
            {
                state.preferences.current_workspace =
                    project.workspace.to_string_lossy().into_owned();
                state.input = PathBuf::from(&state.preferences.stage)
                    .join("paks")
                    .join(&project.target)
                    .to_string_lossy()
                    .into_owned();
                state.status = format!("Selected {} · {}", project.label, project.details);
            } else {
                state.preferences.current_workspace.clear();
                state.status = "No project selected.".into();
            }
        }
        Message::ModdingRefresh => {
            if state.preferences.projects.trim().is_empty() {
                state.status = "Configure your Projects folder in Settings first.".into();
                return Task::none();
            }
            return launch(
                state,
                Operation::IndexProjects(
                    PathBuf::from(&state.preferences.projects),
                    PathBuf::from(&state.preferences.stage),
                ),
            );
        }
        Message::ModdingToggleAdvanced => state.modding_advanced = !state.modding_advanced,
        Message::ModdingToggleProfiles => {
            state.modding_profiles = !state.modding_profiles;
            if state.modding_profiles && !state.preferences.stage.trim().is_empty() {
                return launch(
                    state,
                    Operation::ListProfiles(PathBuf::from(&state.preferences.stage)),
                );
            }
        }
        Message::ModdingBrowsePaks => {
            let root = PathBuf::from(&state.preferences.stage).join("paks");
            if !root.is_dir() {
                state.status = "Configure and prepare a signed PAK stage first (Settings).".into();
                return Task::none();
            }
            state.modding_advanced = true;
            return begin_scan(state, root);
        }
        Message::ModdingSelectArchive(index) => {
            if let Some(inventory) = &state.inventory
                && let Some(entry) = inventory.entries.get(index)
            {
                let candidate = PathBuf::from(&inventory.root).join(&entry.relative_path);
                if modding_keys::staged_target(
                    PathBuf::from(&state.preferences.stage).as_path(),
                    &candidate,
                )
                .is_ok()
                {
                    let filter = state.search.to_ascii_lowercase();
                    let visible: Vec<usize> = inventory
                        .entries
                        .iter()
                        .enumerate()
                        .filter(|(_, row)| {
                            row.relative_path.to_ascii_lowercase().ends_with(".pak")
                                && (filter.is_empty()
                                    || row.relative_path.to_ascii_lowercase().contains(&filter))
                        })
                        .map(|(i, _)| i)
                        .take(90)
                        .collect();
                    state.file_selection.click(
                        &visible,
                        index,
                        state.keyboard_modifiers.shift(),
                        state.keyboard_modifiers.control(),
                    );
                    state.selected = state.file_selection.primary_after(index);
                    state.input = candidate.to_string_lossy().into_owned();
                    state.status = format!("Selected original staged PAK: {}", entry.relative_path);
                } else {
                    state.status = "Select a regular PAK inside the configured stage/paks.".into();
                }
            }
        }
        Message::EditorReplacementChanged(value) => state.editor_replace_source = value,
        Message::EditorImportRawMember | Message::EditorRestoreRawMember => {
            let restore = matches!(message, Message::EditorRestoreRawMember);
            if state.viewer_from_library {
                state.status = "Asset Library source files are always read-only.".into();
                return Task::none();
            }
            let Some(project) = state
                .modding_selected
                .and_then(|i| state.modding_projects.as_ref().and_then(|p| p.get(i)))
                .cloned()
            else {
                state.status = "Open a managed editable project to edit a project member.".into();
                return Task::none();
            };
            let Some(inventory) = &state.inventory else {
                state.status = "Index the editable project files first.".into();
                return Task::none();
            };
            let expected = std::fs::canonicalize(project.workspace.join("files"));
            let actual = std::fs::canonicalize(&inventory.root);
            if !matches!((expected,actual),(Ok(project),Ok(current)) if project==current) {
                state.status =
                    "The visible files do not belong to the selected editable project.".into();
                return Task::none();
            }
            if state.file_selection.indices.len() != 1 {
                state.status = "Select exactly one project asset to edit (Shift/Ctrl selections are supported for inspection).".into();
                return Task::none();
            }
            let Some(member) = state
                .selected
                .and_then(|i| inventory.entries.get(i))
                .map(|e| e.relative_path.clone())
            else {
                state.status = "Select one manifest-listed extracted file.".into();
                return Task::none();
            };
            if restore {
                return launch(
                    state,
                    Operation::RestoreRawProjectMember(
                        project,
                        PathBuf::from(&state.preferences.projects),
                        member,
                    ),
                );
            }
            if state.editor_replace_source.trim().is_empty() {
                state.status = "Choose a separate local replacement file path.".into();
                return Task::none();
            }
            return launch(
                state,
                Operation::ImportRawProjectMember(
                    project,
                    PathBuf::from(&state.preferences.projects),
                    member,
                    PathBuf::from(&state.editor_replace_source),
                ),
            );
        }
        Message::ModdingOpenProject => {
            if state.busy {
                state.status =
                    "Wait for the current asset operation before opening another project.".into();
                return Task::none();
            }
            let Some(project) = state
                .modding_selected
                .and_then(|i| state.modding_projects.as_ref().and_then(|p| p.get(i)))
            else {
                state.status = "Select an editable project first.".into();
                return Task::none();
            };
            let root = project.workspace.join("files");
            if !root.is_dir() {
                state.status = "This project has no extracted files folder.".into();
                return Task::none();
            }
            state.input = root.to_string_lossy().into_owned();
            state.modding_open_editor_when_scanned = true;
            return begin_scan(state, root);
        }
        Message::ModdingReviewChanges => {
            if let Some(project) = state
                .modding_selected
                .and_then(|i| state.modding_projects.as_ref().and_then(|p| p.get(i)))
            {
                state.status = format!("{} · {}", project.label, project.details);
            } else {
                state.status = "Select a project to review its changed assets.".into();
            }
        }
        Message::ModdingBuild => {
            let Some(project) = state.modding_selected.and_then(|i| {
                state
                    .modding_projects
                    .as_ref()
                    .and_then(|projects| projects.get(i))
            }) else {
                state.status = "Select a verified editable project first.".into();
                return Task::none();
            };
            if !project.changed_known || project.modified.is_empty() || project.modified.len() > 64
            {
                state.status = "A signed build needs 1–64 verified changed project files. Refresh the project before building.".into();
                return Task::none();
            }
            return launch(
                state,
                Operation::BuildRawProject(
                    project.clone(),
                    PathBuf::from(&state.preferences.projects),
                    PathBuf::from(&state.preferences.stage),
                ),
            );
        }
        Message::ModdingUnpackPak => {
            if state.input.trim().is_empty() {
                state.status = "Select a signed staged original PAK in the archive browser.".into();
                return Task::none();
            }
            state.status = "Creating an editable project from the verified signed PAK...".into();
            return launch(
                state,
                Operation::UnpackRawProject(
                    PathBuf::from(&state.preferences.projects),
                    PathBuf::from(&state.preferences.stage),
                    PathBuf::from(&state.input),
                ),
            );
        }
        Message::LibraryImportSourceChanged(path) => {
            state.library_import_source = path;
        }
        Message::LibraryImportStart => {
            if state.library_import_source.trim().is_empty() {
                state.status =
                    "Select a signed Stage 2 PAK or folder of PAKs to import. The Asset Library location is detected automatically.".into();
                state.library_show_configuration = true;
                return Task::none();
            }
            return launch(
                state,
                Operation::ImportSignedLibrary(
                    PathBuf::from(&state.library_root),
                    PathBuf::from(&state.library_import_source),
                    PathBuf::from(&state.preferences.stage),
                ),
            );
        }
        Message::SettingsTab(tab) => state.settings_tab = tab,
        Message::LibrarySearchChanged(value) => {
            state.library_search = value;
            state.library_page = 0;
            refresh_library_matches(state);
        }
        Message::LibraryMenu(menu) => {
            state.modding_menu = None;
            state.viewer_menu = None;
            state.library_menu = if state.library_menu == Some(menu) {
                None
            } else {
                Some(menu)
            };
        }
        Message::LibraryToggleConfiguration => {
            state.library_show_configuration = !state.library_show_configuration;
        }
        Message::OpenLibrarySettings => {
            state.settings_tab = settings_page::Tab::Storage;
            state.page = Some(Page::Settings);
            state.library_menu = None;
        }
        Message::StartupLibraryIndexed(expected, result) => {
            state.library_startup_loading = false;
            // Ignore a startup read that finished after the user changed
            // the root or explicitly loaded a newer catalog.
            if Path::new(&state.library_root) != expected.as_path()
                || state.library_catalog.is_some()
            {
                return Task::none();
            }
            match result {
                Ok(catalog) => {
                    state.status = format!(
                        "{} imported Asset Library PAKs discovered automatically; originals untouched.",
                        catalog.archives.len()
                    );
                    state.library_root = catalog.root.clone();
                    state.preferences.asset_library_root = catalog.root.clone();
                    state.library_catalog = Some(catalog);
                    state.library_selection.clear();
                    state.library_selected = None;
                    state.library_page = 0;
                    refresh_library_matches(state);
                }
                Err(error) => {
                    state.status = format!("Could not index saved Asset Library: {error}")
                }
            }
        }
        Message::LibraryOpenFolder => {
            // Open the configured Asset Library root, not an arbitrarily
            // selected imported archive, stale inventory or game directory.
            // File is a shell navigation action: never edit a source PAK.
            match verified_library_directory(&state.library_root) {
                Ok(directory) => {
                    #[cfg(target_os = "windows")]
                    {
                        state.status = match std::process::Command::new("explorer.exe")
                            .arg(directory)
                            .spawn()
                        {
                            Ok(_) => "Opened the Asset Library folder in Windows Explorer.".into(),
                            Err(error) => {
                                format!("Could not open Asset Library in Explorer: {error}")
                            }
                        };
                    }
                    #[cfg(not(target_os = "windows"))]
                    {
                        let _ = directory;
                        state.status =
                            "Open Asset Library Folder is available in Windows builds.".into();
                    }
                }
                Err(error) => state.status = error,
            }
        }
        Message::LibraryRefresh => {
            if state.library_startup_loading {
                state.status = "Indexing the detected Asset Library in the background...".into();
                return Task::none();
            }
            if !PathBuf::from(&state.library_root).is_dir() {
                state.library_catalog = None;
                state.library_selection.clear();
                state.library_selected = None;
                state.library_page = 0;
                refresh_library_matches(state);
                state.status =
                    "Choose an Asset Library location in Settings → Storage, or import a PAK."
                        .into();
                state.page = Some(Page::Settings);
                state.settings_tab = settings_page::Tab::Storage;
                return Task::none();
            }
            return launch(
                state,
                Operation::LoadImportedLibrary(PathBuf::from(&state.library_root)),
            );
        }
        Message::LibrarySetByBuild(value) => {
            state.library_by_build = value;
            state.library_page = 0;
            refresh_library_matches(state);
        }
        Message::LibraryNextBuild => {
            if let Some(catalog) = &state.library_catalog
                && !catalog.builds.is_empty()
            {
                state.library_build_index = (state.library_build_index + 1) % catalog.builds.len();
                state.library_selected = None;
                state.library_page = 0;
                refresh_library_matches(state);
            }
        }
        Message::LibrarySelect(index) => {
            if state
                .library_catalog
                .as_ref()
                .is_some_and(|catalog| index < catalog.archives.len())
            {
                // The full filtered catalog, not the current 110-row page,
                // defines Shift range selection. Anchors survive page changes
                // without selecting entries excluded by the active filter.
                let visible = state.library_matches.clone();
                state.library_selection.click(
                    &visible,
                    index,
                    state.keyboard_modifiers.shift(),
                    state.keyboard_modifiers.control(),
                );
                state.library_selected = state.library_selection.primary_after(index);
            }
        }
        Message::LibraryPreviousPage => {
            state.library_page = state.library_page.saturating_sub(1);
        }
        Message::LibraryNextPage => {
            state.library_page = state.library_page.saturating_add(1);
        }
        Message::LibraryPreview => {
            let Some(pak) = state
                .library_selected
                .and_then(|index| state.library_catalog.as_ref()?.archives.get(index))
                .cloned()
            else {
                state.status = "Select an imported PAK before previewing its files.".into();
                return Task::none();
            };
            return launch(state, Operation::OpenImportedPak(pak));
        }
        Message::LibraryMakeModProject => {
            if state.library_selection.indices.len() != 1 {
                state.status = "Select exactly one imported PAK to make an editable copy.".into();
                return Task::none();
            }
            let Some(pak) = state
                .library_selected
                .filter(|i| state.library_selection.indices.contains(i))
                .and_then(|i| state.library_catalog.as_ref()?.archives.get(i))
                .cloned()
            else {
                state.status = "Choose a verified imported archive first.".into();
                return Task::none();
            };
            let stage = PathBuf::from(&state.preferences.stage);
            if modding_keys::staged_target(&stage, PathBuf::from(&pak.source).as_path()).is_err() {
                state.status = "This imported PAK is not an exact staged original in the configured stage/paks. A separate copy mapping would be required; no target was guessed.".into();
                return Task::none();
            }
            state.modding_open_editor_when_scanned = true;
            return launch(
                state,
                Operation::CreateProjectFromLibrary(
                    pak,
                    PathBuf::from(&state.preferences.projects),
                    stage,
                ),
            );
        }
        Message::ToggleAdvancedTools => state.show_advanced_tools = !state.show_advanced_tools,
        Message::ToggleActivity => {}
        Message::ClearActivity => state.activity_log.clear(),
        Message::ToggleSystemHealth => state.health_expanded = !state.health_expanded,
        Message::CheckSystemHealth => {
            if state.health_checking {
                return Task::none();
            }
            state.health_expanded = true;
            state.health_checking = true;
            let game = state.preferences.game_storage_root.clone();
            let stage = state.preferences.stage.clone();
            let projects = state.preferences.projects.clone();
            return Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        system_health::inspect(&game, &stage, &projects)
                    })
                    .await
                    .unwrap_or_else(|error| system_health::Snapshot {
                        firewall: format!("Read-only health check failed: {error}"),
                        game_process: "Unavailable".into(),
                        storage: "Unavailable".into(),
                        dumps: "Unavailable".into(),
                        environment: "Unavailable".into(),
                        recovery: "Unavailable".into(),
                    })
                },
                Message::SystemHealthChecked,
            );
        }
        Message::SystemHealthChecked(report) => {
            state.health_report = Some(report);
            state.health_checking = false;
            state.status = "Read-only System Health check completed.".into();
        }
        Message::DeveloperPasswordChanged(value) => {
            if !state.developer_checking && value.len() <= 128 {
                state.developer_password = value;
            }
        }
        Message::LockDeveloper => {
            state.developer_unlocked = false;
            state.developer_checking = false;
            state.developer_password.clear();
            state.developer_status.clear();
        }
        Message::UnlockDeveloper => {
            if state.developer_unlocked || state.developer_checking {
                return Task::none();
            }
            if state.developer_attempts >= 5 {
                state.developer_password.clear();
                state.developer_status = "Developer access locked for this session.".into();
                return Task::none();
            }
            let password = zeroize::Zeroizing::new(std::mem::take(&mut state.developer_password));
            state.developer_checking = true;
            state.developer_status = "Checking developer credentials...".into();
            return Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || developer_auth::verify(password.as_str()))
                        .await
                        .unwrap_or(false)
                },
                Message::DeveloperChecked,
            );
        }
        Message::DeveloperChecked(authenticated) => {
            // Ignore a stale verification result if the user locked the panel.
            if !state.developer_checking {
                return Task::none();
            }
            state.developer_checking = false;
            if authenticated {
                state.developer_unlocked = true;
                state.developer_status.clear();
            } else {
                state.developer_attempts = state.developer_attempts.saturating_add(1);
                state.developer_status = if state.developer_attempts >= 5 {
                    "Developer access locked for this session.".into()
                } else {
                    "Invalid developer credentials.".into()
                };
            }
        }
        Message::LibraryExport
        | Message::OpenViewer
        | Message::OpenInspectorWindow
        | Message::OpenViewerPreferences
        | Message::InspectorOpened
        | Message::ViewerOpened
        | Message::ActivityOpened
        | Message::WindowClosed(_)
        | Message::FileDropped(_, _) => {}
        Message::CancelScan => {
            if let Some(cancel) = &state.scan_cancel {
                cancel.store(true, Ordering::Relaxed);
                state.status = "Cancelling asset scan after current entry...".into();
            }
        }
        Message::ScanPulse => {
            if let Some(indexed) = &state.scan_indexed {
                state.status = format!(
                    "Indexed {} local assets; working asynchronously...",
                    indexed.load(Ordering::Relaxed)
                );
            }
        }
        Message::DropPath(path) => {
            let metadata = match std::fs::symlink_metadata(&path) {
                Ok(metadata) if !metadata.file_type().is_symlink() => metadata,
                _ => {
                    state.status = "Dropped path must be a regular local file or directory.".into();
                    return Task::none();
                }
            };
            state.input = path.to_string_lossy().into_owned();
            if metadata.file_type().is_dir() {
                return begin_scan(state, path);
            }
            if !metadata.file_type().is_file() {
                state.status = "Unsupported dropped path type.".into();
                return Task::none();
            }
            let name = state.input.to_ascii_lowercase();
            let operation = if name.ends_with(".dds") || name.ends_with(".dds.0") {
                Some(Operation::InspectDds(path))
            } else if name.ends_with(".cdf") {
                Some(Operation::InspectCdf(path))
            } else if is_viewer_model_source(&name) {
                Some(Operation::InspectModel(path))
            } else if name.ends_with(".caf") {
                Some(Operation::LoadCaf(path))
            } else if name.ends_with(".cry") || name.ends_with(".lvl") {
                Some(Operation::InspectLevel(path))
            } else {
                None
            };
            if let Some(operation) = operation {
                return launch(state, operation);
            }
            state.status =
                "Source path selected. Choose the relevant safe inspector in Modding.".into();
        }
        Message::ViewerMenu(section) => {
            state.modding_menu = None;
            state.library_menu = None;
            state.viewer_menu = if section.is_empty() || state.viewer_menu == Some(section) {
                None
            } else {
                Some(section)
            };
        }
        Message::ViewerToggleExplorer => {
            state.viewer_explorer_visible = !state.viewer_explorer_visible;
        }
        Message::ViewerSidebarAnimations(value) => {
            // Legacy menu action: selecting Files does not close Animations.
            if value {
                state.viewer_sidebar_animations = true;
                state.viewer_sidebar_active = ViewerSidebarTab::Animations;
            } else {
                state.viewer_sidebar_active = ViewerSidebarTab::Files;
            }
            state.viewer_explorer_visible = true;
            state.viewer_menu = None;
        }
        Message::ViewerDamageBlendChanged(percent) => {
            if percent <= 100
                && state.equipment_source.is_some()
                && !state.busy
                && !state.cdf_legacy_2012
            {
                state.damage_blend_percent = percent;
            }
        }
        Message::ViewerDamageBlendCommit => {
            if state.busy || state.cdf_legacy_2012 {
                return Task::none();
            }
            let (Some(source), Some(base)) = (&state.equipment_source, &state.cdf_base_surfaces)
            else {
                return Task::none();
            };
            if state.damage_blend_percent == 0 {
                state.cdf_surfaces = Some(Arc::clone(base));
                state.status =
                    "Original character diffuse restored from memory (no reload).".into();
                return Task::none();
            }
            if let Some(full) = &state.cdf_full_damage_surfaces {
                if state.damage_blend_percent == 100 {
                    state.cdf_surfaces = Some(Arc::clone(full));
                    state.status = "100% damage applied from cached original maps.".into();
                    return Task::none();
                }
                return launch(
                    state,
                    Operation::ReblendCdfDamage(
                        source.clone(),
                        Arc::clone(base),
                        Arc::clone(full),
                        state.damage_blend_percent,
                    ),
                );
            }
            let (Some(report), Some(inventory)) = (&state.cdf_report, &state.inventory) else {
                return Task::none();
            };
            let operation = Operation::PrepareCdfDamage(
                source.clone(),
                Arc::clone(report),
                Arc::clone(base),
                inventory.clone(),
                state
                    .library_catalog
                    .as_ref()
                    .map(|c| PathBuf::from(&c.root)),
                resource_build(state),
                state.preview_cache.clone(),
            );
            state.cdf_damage_preparing = true;
            return launch(state, operation);
        }
        Message::ViewerToggleDamageBlend => {
            if state.busy {
                state.status =
                    "Finish the current model preview before changing damage blend.".into();
                return Task::none();
            }
            let Some(source) = &state.equipment_source else {
                state.status = "Select a monster CDF to preview its authored damage layer.".into();
                return Task::none();
            };
            if !source
                .to_string_lossy()
                .replace('\\', "/")
                .to_ascii_lowercase()
                .contains("/monsters/")
            {
                state.status = "Damage blend is only available for monster CDFs.".into();
                return Task::none();
            }
            if state.cdf_legacy_2012 {
                state.status =
                    "Damage blend is a Stage 2 feature, unavailable for original 2012 monsters."
                        .into();
                return Task::none();
            }
            state.damage_blend_enabled = !state.damage_blend_enabled;
            state.damage_blend_percent = if state.damage_blend_enabled { 100 } else { 0 };
            state.viewer_sidebar_active = ViewerSidebarTab::Equipment;
            return update(state, Message::ViewerDamageBlendCommit);
        }
        Message::ViewerEmissiveStrengthChanged(value) => {
            if value.is_finite()
                && state.model.is_some()
                && (state.emissive_map.is_some()
                    || state.cdf_surfaces.as_ref().is_some_and(|surfaces| {
                        surfaces.iter().any(|part| {
                            part.visible
                                && (part.armor_emissive.is_some()
                                    || (part.emissive.is_some() && part.emission_strength > 0.0))
                        })
                    }))
            {
                state.emissive_strength = (value * 20.0).round().clamp(0.0, 80.0) / 20.0;
            }
        }
        Message::ViewerKalaTubingState(choice) => {
            if state.busy {
                state.status = "Finish loading Kala before changing tubing state.".into();
                return Task::none();
            }
            let Some(source) = state.equipment_source.as_ref() else {
                return Task::none();
            };
            let Ok(definition) = read_cdf(source) else {
                return Task::none();
            };
            if !evolve_core::is_original_kala_3p(&definition) {
                state.status = "Tubing state is only available for original Kala (Tech) 3P.".into();
                return Task::none();
            }
            if choice != state.kala_tubing {
                state.kala_tubing = choice;
                // The state-specific source calibration now carries the
                // reported blue/red brightness. Reset the shared slider to
                // neutral when switching states instead of carrying 0.3/4x.
                state.emissive_strength = 1.0;
                state.viewer_sidebar_active = ViewerSidebarTab::Equipment;
                return update(state, Message::ViewerPreviewStaticCdf);
            }
        }
        Message::ViewerToggleMonsterGlow => {
            if state.busy {
                state.status = "Finish the current preview before changing glow.".into();
                return Task::none();
            }
            let Some(source) = &state.equipment_source else {
                state.status = "Select a Stage 2 monster CDF to enable Glow.".into();
                return Task::none();
            };
            if !source
                .to_string_lossy()
                .replace('\\', "/")
                .to_ascii_lowercase()
                .contains("/monsters/")
            {
                state.status = "Glow overlay is only available for Stage 2 monster CDFs.".into();
                return Task::none();
            }
            if state.cdf_legacy_2012 {
                state.status =
                    "The optional Glow overlay is not available for original 2012 monsters.".into();
                return Task::none();
            }
            state.full_armor_glow = !state.full_armor_glow;
            state.viewer_sidebar_active = ViewerSidebarTab::Equipment;
            return update(state, Message::ViewerPreviewStaticCdf);
        }
        Message::ViewerEquip(slot, binding) => {
            if state.busy {
                state.status =
                    "Finish the current character preview before swapping equipment.".into();
                return Task::none();
            }
            let Some(existing) = state
                .equipment_slots
                .iter()
                .find(|entry| entry.name == slot)
            else {
                state.status = "Equipment attachment was not verified for this character.".into();
                return Task::none();
            };
            if !existing
                .choices
                .iter()
                .any(|c| c.binding.eq_ignore_ascii_case(&binding))
            {
                state.status =
                    "Equipment file is not a verified option for this exact slot.".into();
                return Task::none();
            }
            if binding.eq_ignore_ascii_case(&existing.original) {
                state.equipment_overrides.remove(&slot);
            } else {
                state
                    .equipment_overrides
                    .insert(slot.clone(), binding.clone());
            }
            state.viewer_sidebar_active = ViewerSidebarTab::Equipment;
            return update(state, Message::ViewerPreviewStaticCdf);
        }
        Message::ViewerSidebarSelect(tab) => {
            state.viewer_sidebar_active = tab;
            if tab == ViewerSidebarTab::Animations {
                state.viewer_sidebar_animations = true;
            } else if tab == ViewerSidebarTab::Inspector {
                state.viewer_show_inspector = true;
            } else if tab == ViewerSidebarTab::Dependencies {
                state.viewer_sidebar_dependencies = true;
            }
            state.viewer_explorer_visible = true;
            state.viewer_menu = None;
        }
        Message::ViewerSidebarClose(tab) => {
            match tab {
                ViewerSidebarTab::Files | ViewerSidebarTab::Equipment => {} // Permanent tool tabs.
                ViewerSidebarTab::Animations => state.viewer_sidebar_animations = false,
                ViewerSidebarTab::Inspector => state.viewer_show_inspector = false,
                ViewerSidebarTab::Dependencies => state.viewer_sidebar_dependencies = false,
            }
            if state.viewer_sidebar_active == tab {
                state.viewer_sidebar_active = ViewerSidebarTab::Files;
            }
        }
        Message::ViewerSplitDrag(delta) => {
            if delta.is_finite() {
                // Dragging a 12px grip changes the relative file-pane width.
                state.viewer_split = (state.viewer_split + delta).clamp(0.25, 0.85);
            }
        }
        Message::ViewerToggleInspector => {
            if state.viewer_show_inspector {
                state.viewer_show_inspector = false;
                if state.viewer_sidebar_active == ViewerSidebarTab::Inspector {
                    state.viewer_sidebar_active = ViewerSidebarTab::Files;
                }
            } else {
                state.viewer_show_inspector = true;
                state.viewer_sidebar_active = ViewerSidebarTab::Inspector;
                state.viewer_explorer_visible = true;
            }
        }
        Message::ViewerSelectTab(tab) => {
            state.viewer_tab = tab;
            if tab == ViewerTab::Dependencies {
                state.viewer_sidebar_dependencies = true;
                state.viewer_sidebar_active = ViewerSidebarTab::Dependencies;
                state.viewer_explorer_visible = true;
            }
        }
        Message::PlaybackSpeedChanged(value) => state.playback_speed = value.clamp(0.1, 3.0),
        Message::PlaybackLoopChanged => state.playback_loop = !state.playback_loop,
        Message::ViewerToggleWireframe => state.viewer_wireframe = !state.viewer_wireframe,
        Message::LightStrengthChanged(value) => state.light_strength = value.clamp(0.2, 4.0),
        Message::LevelSelect(index) => {
            if let Some(scene) = &state.level_scene
                && index < scene.objects.len()
            {
                let query = state.level_filter.to_ascii_lowercase();
                let visible: Vec<usize> = scene
                    .objects
                    .iter()
                    .enumerate()
                    .filter(|(_, obj)| {
                        query.is_empty()
                            || obj.name.to_ascii_lowercase().contains(&query)
                            || obj.layer.to_ascii_lowercase().contains(&query)
                            || obj.model.to_ascii_lowercase().contains(&query)
                    })
                    .map(|(i, _)| i)
                    .take(180)
                    .collect();
                if visible.contains(&index) {
                    state.level_selection.click(
                        &visible,
                        index,
                        state.keyboard_modifiers.shift(),
                        state.keyboard_modifiers.control(),
                    );
                } else {
                    // Clicking a source-space point on the map picks an exact
                    // object even when filtered out of the outliner.
                    state.level_selection.click(&[index], index, false, false);
                }
                state.level_selected = state.level_selection.primary_after(index);
                sync_level_fields(state);
            }
        }
        Message::LevelFilterChanged(value) => state.level_filter = value,
        Message::LevelZoomChanged(value) => state.level_zoom = value.clamp(0.3, 3.0),
        Message::LevelEditPositionChanged(value) => state.level_edit_position = value,
        Message::LevelEditRotationChanged(value) => state.level_edit_rotation = value,
        Message::LevelEditScaleChanged(value) => state.level_edit_scale = value,
        Message::LevelEditOutputChanged(value) => state.level_edit_output = value,
        Message::LevelUndoTransform => restore_level_history(state, true),
        Message::LevelRedoTransform => restore_level_history(state, false),
        Message::LevelApplyTransform => {
            let Some(index) = state.level_selected else {
                state.status = "Select a placement to edit.".into();
                return Task::none();
            };
            let Some(scene) = &state.level_scene else {
                state.status = "Inspect the source level first.".into();
                return Task::none();
            };
            let Some(object) = scene.objects.get(index) else {
                state.status = "Selected level placement is no longer available.".into();
                return Task::none();
            };
            let id = object.id.clone();
            let previous = object.local;
            let requested = match level_workspace::parse_transform(
                &state.level_edit_position,
                &state.level_edit_rotation,
                &state.level_edit_scale,
            ) {
                Ok(value) => value,
                Err(error) => {
                    state.status = error;
                    return Task::none();
                }
            };
            match evolve_core::edit_level_transform(scene, &id, requested) {
                Ok(edited) => {
                    let applied = edited.objects[index].local;
                    if applied != previous {
                        // The authored transform map was frozen at load; never
                        // replace its originals with an edited/undone value.
                        if !state.level_original_transforms.contains_key(&id) {
                            state.status = "Original placement missing; reload source.".into();
                            return Task::none();
                        }
                        if state.level_undo.len() >= 64 {
                            state.level_undo.pop_front();
                        }
                        track_level_dirty(state, &id, previous, applied);
                        state.level_undo.push_back(LevelTransformDiff {
                            guid: id,
                            old_local: previous,
                            new_local: applied,
                        });
                        state.level_redo.clear();
                        state.level_scene = Some(Arc::new(edited));
                        sync_level_fields(state);
                        state.status =
                            "Local transform preview updated; export a separate layer to save it."
                                .into();
                    }
                }
                Err(error) => state.status = format!("Invalid level transform: {error}"),
            }
        }
        Message::LevelExportEdits => {
            let Some(source) = &state.level_source else {
                state.status = "Inspect an original level before exporting edits.".into();
                return Task::none();
            };
            if state.level_scene.is_none() {
                return Task::none();
            }
            let destination = PathBuf::from(state.level_edit_output.trim());
            if !destination.is_absolute()
                || destination
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_none_or(|extension| {
                        !extension.eq_ignore_ascii_case("lyr")
                            && !extension.eq_ignore_ascii_case("xml")
                    })
                || destination.exists()
            {
                state.status = "Choose a NEW absolute .lyr or .xml output file.".into();
                return Task::none();
            }
            let Some(parent) = destination.parent() else {
                return Task::none();
            };
            let parent = match std::fs::canonicalize(parent) {
                Ok(parent) => parent,
                Err(error) => {
                    state.status = format!("Export directory unavailable: {error}");
                    return Task::none();
                }
            };
            // Resolve actual filesystem paths, including symlink aliases,
            // instead of comparing unchecked strings. The worker repeats all
            // these checks after serializing and immediately before publish.
            let mut protected = vec![source.parent().unwrap_or(source.as_path()).to_path_buf()];
            if let Some(inventory) = &state.inventory {
                protected.push(PathBuf::from(&inventory.root));
            }
            if let Some(catalog) = &state.library_catalog {
                protected.push(PathBuf::from(&catalog.root));
            }
            for candidate in [
                &state.library_root,
                &state.preferences.asset_library_root,
                &state.preferences.projects,
                &state.preferences.stage,
                &state.preferences.game_storage_root,
            ] {
                if !candidate.trim().is_empty() {
                    let path = PathBuf::from(candidate);
                    // Only existing source/library directories need protection.
                    // Missing paths cannot contain a valid output parent.
                    if path.exists() {
                        protected.push(path);
                    }
                }
            }
            let mut checked = Vec::new();
            for directory in protected {
                match std::fs::canonicalize(&directory) {
                    Ok(root) if root.is_dir() => {
                        if parent.starts_with(&root) {
                            state.status = format!(
                                "Export blocked inside protected source/library directory: {}",
                                root.display()
                            );
                            return Task::none();
                        }
                        if !checked.contains(&root) {
                            checked.push(root);
                        }
                    }
                    _ => {
                        state.status =
                            "A protected asset directory could not be verified; export refused."
                                .into();
                        return Task::none();
                    }
                }
            }
            let edits = level_pending_edits(state);
            if edits.is_empty() || edits.len() > 128 {
                state.status =
                    "No changed level placements, or more than 128 pending edits.".into();
                return Task::none();
            }
            let source = source.clone();
            let expected_sha = state.level_source_sha.clone();
            return launch(
                state,
                Operation::ExportEditedLevel(source, destination, edits, checked, expected_sha),
            );
        }
        Message::ViewerSaveLayout => {
            state.preferences.viewer_split = state.viewer_split;
            state.preferences.viewer_explorer_visible = state.viewer_explorer_visible;
            state.preferences.viewer_show_inspector = state.viewer_show_inspector;
            state.preferences.viewer_wireframe = state.viewer_wireframe;
            state.preferences.light_strength = state.light_strength;
            state.preferences.animation_speed = state.playback_speed;
            state.preferences.animation_loop = state.playback_loop;
            state.preferences.saved_yaw = state.yaw;
            state.preferences.saved_pitch = state.pitch;
            state.preferences.saved_zoom = state.zoom;
            persist_preferences(state);
            state.status =
                "Viewer split, explorer visibility, camera and animation speed saved.".into();
        }
        Message::CompareOriginalChanged(path) => {
            state.compare_original = path;
            state.comparison = None;
        }
        Message::CompareModifiedChanged(path) => {
            state.compare_modified = path;
            state.comparison = None;
        }
        Message::StartAssetCompare => {
            if state.compare_original.is_empty() || state.compare_modified.is_empty() {
                state.status =
                    "Choose an original and modified asset for read-only comparison.".into();
                return Task::none();
            }
            return launch(
                state,
                Operation::CompareAssets(
                    PathBuf::from(&state.compare_original),
                    PathBuf::from(&state.compare_modified),
                ),
            );
        }
        Message::StartDependencyInspection => {
            state.viewer_sidebar_dependencies = true;
            state.viewer_sidebar_active = ViewerSidebarTab::Dependencies;
            state.viewer_explorer_visible = true;
            state.viewer_menu = None;
            let Some(inventory) = &state.inventory else {
                state.status = "Index extracted assets before inspecting dependencies.".into();
                return Task::none();
            };
            let Some(selected) = state
                .selected
                .and_then(|index| inventory.entries.get(index))
            else {
                state.status = "Choose a CDF or MTL in the file explorer first.".into();
                return Task::none();
            };
            state.viewer_tab = ViewerTab::Dependencies;
            return launch(
                state,
                Operation::InspectDependencies(inventory.clone(), selected.relative_path.clone()),
            );
        }
        Message::ViewerResetCamera => {
            state.yaw = 0.0;
            state.pitch = 0.0;
            state.zoom = 1.0;
            state.pan_x = 0.0;
            state.pan_y = 0.0;
        }
        Message::ViewerCameraPreset(preset) => {
            // Match the private viewer's Blender-style axis directions.
            let orientation = match preset {
                "Front" => Some((0.0, 0.0)),
                "Back" => Some((180.0, 0.0)),
                "Left" => Some((-90.0, 0.0)),
                "Right" => Some((90.0, 0.0)),
                "Top" => Some((0.0, 89.5)),
                "Bottom" => Some((0.0, -89.5)),
                "Isometric" => Some((-30.0, 14.0)),
                _ => None,
            };
            if let Some((yaw, pitch)) = orientation {
                state.yaw = yaw;
                state.pitch = pitch;
                state.pan_x = 0.0;
                state.pan_y = 0.0;
            }
        }
        Message::ViewerOrbitDelta(dx, dy) => {
            state.yaw = (state.yaw + dx).rem_euclid(360.0);
            if state.yaw > 180.0 {
                state.yaw -= 360.0;
            }
            state.pitch = (state.pitch + dy).clamp(-85.0, 85.0);
        }
        Message::ViewerPanDelta(dx, dy) => {
            state.pan_x = (state.pan_x + dx).clamp(-4.0, 4.0);
            state.pan_y = (state.pan_y + dy).clamp(-4.0, 4.0);
        }
        Message::ViewerZoomWheel(amount) => {
            state.zoom = (state.zoom * 1.1f32.powf(amount)).clamp(0.2, 4.0);
        }
        Message::ViewerSearchChanged(filter) => {
            state.viewer_context_index = None;
            state.viewer_search = filter;
            state.viewer_page = 0;
            refresh_viewer_rows(state);
        }
        Message::ViewerPreviousPage => {
            state.viewer_page = state.viewer_page.saturating_sub(1);
            refresh_viewer_rows(state);
        }
        Message::ViewerNextPage => {
            if state.viewer_more_rows {
                state.viewer_page += 1;
                refresh_viewer_rows(state);
            }
        }
        Message::ViewerSelectFile(index) => {
            state.viewer_context_index = None;
            // An old dependency diagram must not describe a newly selected file.
            state.dependency_graph = None;
            let visible: Vec<usize> = state
                .viewer_rows
                .iter()
                .filter_map(|row| row.index)
                .collect();
            state.file_selection.click(
                &visible,
                index,
                state.keyboard_modifiers.shift(),
                state.keyboard_modifiers.control(),
            );
            state.selected = state.file_selection.primary_after(index);
            // A newer selection always supersedes an earlier queued preview.
            state.viewer_queued_preview = None;
            // A normal single click previews immediately, like the Qt file
            // editor. Shift/Ctrl remain pure multi-select gestures.
            if !state.keyboard_modifiers.shift()
                && !state.keyboard_modifiers.control()
                && state.selected == Some(index)
                && state
                    .inventory
                    .as_ref()
                    .and_then(|inv| inv.entries.get(index))
                    .is_some_and(|entry| is_viewer_preview_source(&entry.relative_path))
            {
                let is_cdf = state
                    .inventory
                    .as_ref()
                    .and_then(|inv| inv.entries.get(index))
                    .is_some_and(|entry| {
                        entry.relative_path.to_ascii_lowercase().ends_with(".cdf")
                    });
                if state.busy {
                    state.viewer_queued_preview = Some(index);
                    state.status = "Preview queued: the latest selected asset will load when the current operation finishes.".into();
                    return Task::none();
                }
                return update(
                    state,
                    if is_cdf {
                        Message::ViewerPreviewStaticCdf
                    } else {
                        Message::ViewerPreviewSelected
                    },
                );
            }
        }
        Message::ViewerContextFile(index) => {
            // Right click selects a file without triggering an expensive decode.
            if state
                .inventory
                .as_ref()
                .and_then(|inv| inv.entries.get(index))
                .is_none()
                || !state.viewer_rows.iter().any(|row| row.index == Some(index))
            {
                state.viewer_context_index = None;
                state.status = "Right-click target is no longer in the file explorer.".into();
                return Task::none();
            }
            state.file_selection.click(&[index], index, false, false);
            state.selected = Some(index);
            match verified_viewer_file(state) {
                Ok(path) => {
                    state.input = path.to_string_lossy().into_owned();
                    state.viewer_context_index = Some(index);
                }
                Err(error) => {
                    state.viewer_context_index = None;
                    state.status = error;
                }
            }
        }
        Message::ViewerContextClose => state.viewer_context_index = None,
        Message::ViewerCopySelectedPath => match verified_viewer_file(state) {
            Ok(path) => {
                state.status = "Selected file path copied to the clipboard.".into();
                return iced::clipboard::write(path.to_string_lossy().into_owned());
            }
            Err(error) => state.status = error,
        },
        Message::ViewerRevealSelectedFile => match verified_viewer_file(state) {
            Ok(path) => {
                #[cfg(target_os = "windows")]
                {
                    let result = std::process::Command::new("explorer.exe")
                        .arg(format!("/select,{}", path.display()))
                        .spawn();
                    state.status = match result {
                        Ok(_) => "Opened the selected file location in Windows Explorer.".into(),
                        Err(error) => format!("Could not open Windows Explorer: {error}"),
                    };
                }
                #[cfg(not(target_os = "windows"))]
                {
                    let _ = path;
                    state.status = "Show in Explorer is supported on Windows builds.".into();
                }
            }
            Err(error) => state.status = error,
        },
        Message::ViewerToggleFolder(path) => {
            if !state.viewer_collapsed.insert(path.clone()) {
                state.viewer_collapsed.remove(&path);
            }
            state.viewer_context_index = None;
            state.viewer_page = 0;
            refresh_viewer_rows(state);
        }
        Message::ViewerOpenStaticCdf(index) => {
            let valid = state
                .inventory
                .as_ref()
                .and_then(|inventory| inventory.entries.get(index))
                .is_some_and(|entry| entry.relative_path.to_ascii_lowercase().ends_with(".cdf"));
            if !valid {
                state.status = "Virtual character entry no longer refers to a valid CDF.".into();
                return Task::none();
            }
            state.selected = Some(index);
            state.file_selection.click(&[index], index, false, false);
            return update(state, Message::ViewerPreviewStaticCdf);
        }
        Message::ViewerPreviewStaticCdf => {
            if state.busy {
                state.viewer_queued_preview = state.selected;
                state.status =
                    "Static character preview queued until the current operation finishes.".into();
                return Task::none();
            }
            let Some(inventory) = state.inventory.as_ref() else {
                state.status =
                    "Index an extracted asset folder before assembling characters.".into();
                return Task::none();
            };
            let Some(entry) = state.selected.and_then(|i| inventory.entries.get(i)) else {
                state.status = "Select an actual .cdf character definition.".into();
                return Task::none();
            };
            if !entry.relative_path.to_ascii_lowercase().ends_with(".cdf") {
                state.status =
                    "Static character assembly requires selecting a .cdf definition.".into();
                return Task::none();
            }
            let source = PathBuf::from(&inventory.root).join(&entry.relative_path);
            // The auto-preview route bypasses ViewerPreviewSelected, so it
            // must update the active source itself. Otherwise later exports,
            // resource lookup and the viewer title still point at the
            // previous character even after this CDF finished loading.
            let root = std::fs::canonicalize(&inventory.root);
            let checked = std::fs::canonicalize(&source);
            if !matches!((root, checked), (Ok(root), Ok(file))
                if file.starts_with(&root)
                    && std::fs::symlink_metadata(&source)
                        .is_ok_and(|meta| meta.file_type().is_file()))
            {
                state.status = "CDF source changed or escaped the indexed Asset Library.".into();
                return Task::none();
            }
            if state.equipment_source.as_ref() != Some(&source) {
                state.equipment_overrides.clear();
                state.kala_tubing = KalaTubingState::Banshee;
                // Source-verified Kala tubing receives its own blue/red
                // viewport calibration, so the slider starts neutral.
                state.emissive_strength = 1.0;
                // Armor glow is the normal monster preview state. Missing
                // original glow DDS must not block regular diffuse rendering.
                let virtual_source = entry.relative_path.replace('\\', "/").to_ascii_lowercase();
                state.full_armor_glow = virtual_source.contains("characters/monsters/");
                state.damage_blend_percent = 0;
                state.damage_blend_enabled = false;
                state.equipment_source = Some(source.clone());
                state.equipment_slots = read_cdf(&source)
                    .map(|cdf| discover_equipment(&cdf, inventory))
                    .unwrap_or_default();
            }
            crash_diagnostics::record("cdf_preview_selected", &entry.relative_path);
            state.cdf_base_surfaces = None;
            state.cdf_full_damage_surfaces = None;
            state.cdf_damage_preparing = false;
            let progress = Arc::new(AtomicUsize::new(0));
            state.viewer_load_progress = Some(progress.clone());
            state.input = source.to_string_lossy().into_owned();
            state.texture_preview = None;
            state.status =
                "Validating exact CDF parts for a static preview (not a rigged export)...".into();
            let operation = Operation::BuildStaticCdf(
                source,
                inventory.clone(),
                state
                    .library_catalog
                    .as_ref()
                    .map(|c| PathBuf::from(&c.root)),
                resource_build(state),
                state.preview_cache.clone(),
                state.equipment_overrides.clone(),
                state.full_armor_glow,
                state.kala_tubing,
                progress,
            );
            return launch(state, operation);
        }
        Message::ViewerAutoClipFilter(value) => state.auto_clip_filter = value,
        Message::ViewerToggleExportAnimations => {
            state.export_include_animation = !state.export_include_animation;
        }
        Message::ExportOpen => {
            if let Ok(path) = blender_install::resolve_blender(&state.blender_path) {
                state.blender_path = path.to_string_lossy().into_owned();
            }
            state.viewer_menu = None;
            state.export_dialog_open = true;
            state.export_selected = state
                .file_selection
                .indices
                .iter()
                .copied()
                .filter(|index| {
                    state
                        .inventory
                        .as_ref()
                        .and_then(|inv| inv.entries.get(*index))
                        .is_some_and(|item| exportable_asset(&item.relative_path))
                })
                .collect();
            if state.export_selected.is_empty()
                && let Some(index) = state.selected
                && state
                    .inventory
                    .as_ref()
                    .and_then(|inv| inv.entries.get(index))
                    .is_some_and(|item| exportable_asset(&item.relative_path))
            {
                state.export_selected.insert(index);
            }
            state.status =
                "Select models/assemblies and export settings. Originals remain read-only.".into();
        }
        Message::ExportClose => {
            state.export_dialog_open = false;
        }
        Message::ExportFilterChanged(value) => state.export_filter = value,
        Message::ExportBrowse(kind) => {
            return Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || native_export_picker(kind))
                        .await
                        .ok()
                        .flatten()
                },
                move |path| Message::ExportPicked(kind, path),
            );
        }
        Message::ExportPicked(kind, Some(path)) => {
            let value = path.to_string_lossy().into_owned();
            match kind {
                "folder" => state.export_parent = value,
                "blender" => state.blender_path = value,
                "skeleton" => state.skeleton_path = value,
                "caf" => state.caf_path = value,
                "diffuse" => state.diffuse_input = value,
                _ => {}
            }
        }
        Message::ExportPicked(_, None) => {}

        Message::ExportFolderChanged(value) => state.export_folder_name = value,
        Message::ExportToggleFile(index) => {
            let eligible = state
                .inventory
                .as_ref()
                .and_then(|inv| inv.entries.get(index))
                .is_some_and(|item| exportable_asset(&item.relative_path));
            if eligible && !state.export_selected.remove(&index) && state.export_selected.len() < 64
            {
                state.export_selected.insert(index);
            }
        }
        Message::ExportToggleRigged => state.export_rigged = !state.export_rigged,
        Message::ExportToggleEmbedded => state.export_embed_textures = !state.export_embed_textures,
        Message::ExportToggleDependencies => {
            state.export_dependencies = !state.export_dependencies;
        }
        Message::ExportBegin => {
            if state.busy {
                state.status = "Finish the current operation before exporting.".into();
                return Task::none();
            }
            let Some(inventory) = state.inventory.clone() else {
                state.status = "Index a character/asset folder before exporting.".into();
                return Task::none();
            };
            // Own the folder name before Blender resolution updates state.
            let name = state.export_folder_name.trim().to_owned();
            if name.is_empty()
                || name == "."
                || name == ".."
                || name
                    .chars()
                    .any(|ch| ch.is_control() || r#"<>:"/\\|?*"#.contains(ch))
                || name.ends_with('.')
                || name.ends_with(' ')
            {
                state.status = "Use a single valid Windows export-folder name.".into();
                return Task::none();
            }
            let parent = PathBuf::from(state.export_parent.trim());
            if !parent.is_absolute() || !parent.is_dir() {
                state.status = "Choose an existing absolute parent folder for exports.".into();
                return Task::none();
            }
            let mut indices = state.export_selected.iter().copied().collect::<Vec<_>>();
            indices.sort_unstable();
            let selected = indices
                .into_iter()
                .filter_map(|i| inventory.entries.get(i).map(|e| e.relative_path.clone()))
                .filter(|p| exportable_asset(p))
                .collect::<Vec<_>>();
            if selected.is_empty() || selected.len() > 64 {
                state.status = "Select 1 to 64 source models, CDFs, or DDS textures.".into();
                return Task::none();
            }
            // Images export as PNG directly. Model/assembly FBX exports
            // auto-discover Blender; only a genuinely missing installation
            // requires the user to select their own blender.exe.
            let needs_blender = selected.iter().any(|path| {
                let lower = path.to_ascii_lowercase();
                !lower.ends_with(".dds") && !lower.ends_with(".dds.0")
            });
            let blender = if needs_blender {
                match resolved_blender(state) {
                    Ok(path) => path,
                    Err(message) => {
                        state.status = message;
                        return Task::none();
                    }
                }
            } else {
                PathBuf::new()
            };
            let cdf_source = (selected.len() == 1
                && selected[0].to_ascii_lowercase().ends_with(".cdf"))
            .then(|| PathBuf::from(&inventory.root).join(&selected[0]));
            let export_matches_viewport = cdf_source
                .as_ref()
                .is_some_and(|source| state.equipment_source.as_ref() == Some(source));
            if (!state.equipment_overrides.is_empty()
                || state.full_armor_glow
                || state.damage_blend_percent > 0)
                && !export_matches_viewport
            {
                state.status =
                    "To export current Model Options, select only the active CDF assembly.".into();
                return Task::none();
            }
            if (state.full_armor_glow || state.damage_blend_percent > 0)
                && (!state.export_rigged || !state.export_embed_textures)
            {
                state.status =
                    "Glow and damage blend need Rigged and Embedded textures enabled for FBX export."
                        .into();
                return Task::none();
            }
            let equipment = if export_matches_viewport {
                state.equipment_overrides.clone()
            } else {
                EquipmentOverrides::new()
            };
            let original = parent.join(name);
            let options = evolve_core::ExportBundleOptions {
                destination: original,
                selected,
                blender,
                skeleton: (!state.skeleton_path.trim().is_empty())
                    .then(|| PathBuf::from(&state.skeleton_path)),
                diffuse: (!state.diffuse_input.trim().is_empty())
                    .then(|| PathBuf::from(&state.diffuse_input)),
                caf: (state.export_include_animation && !state.caf_path.trim().is_empty())
                    .then(|| PathBuf::from(&state.caf_path)),
                rigged: state.export_rigged,
                embed_textures: state.export_embed_textures,
                export_dependencies: state.export_dependencies,
                equipment,
                full_armor_glow: state.full_armor_glow && export_matches_viewport,
                damage_blend_percent: if export_matches_viewport {
                    state.damage_blend_percent
                } else {
                    0
                },
            };
            state.export_dialog_open = false;
            return launch(state, Operation::ExportBundle(inventory, options));
        }
        Message::ViewerAutoClip(index) => {
            let Some(clip) = state.auto_clips.get(index) else {
                return Task::none();
            };
            if clip.kind == "dba" {
                let Some(inventory) = state.inventory.clone() else {
                    state.status = "Index exact extracted assets to open a declared DBA.".into();
                    return Task::none();
                };
                if state.converter_path.trim().is_empty() {
                    state.status = "Configure an installed Cryengine-Converter in Settings before opening DBA tracks.".into();
                    return Task::none();
                }
                let operation = Operation::LoadDeclaredDba(
                    clip.virtual_path.clone(),
                    inventory,
                    state
                        .library_catalog
                        .as_ref()
                        .map(|c| PathBuf::from(&c.root)),
                    resource_build(state),
                    PathBuf::from(&state.converter_path),
                );
                return launch(state, operation);
            }
            if clip.kind != "caf" {
                state.status = "Blendspaces and combo graphs require verified bone bindings and are not directly playable.".into();
                return Task::none();
            }
            if let Some(path) = clip.source.clone() {
                return launch(state, Operation::LoadCaf(path));
            }
            let Some(inventory) = state.inventory.clone() else {
                state.status = "Index extracted assets before loading character animations.".into();
                return Task::none();
            };
            let operation = Operation::LoadDeclaredClip(
                index,
                clip.virtual_path.clone(),
                inventory,
                state
                    .library_catalog
                    .as_ref()
                    .map(|c| PathBuf::from(&c.root)),
                resource_build(state),
            );
            return launch(state, operation);
        }
        Message::ViewerPreviewSelected => {
            if state.busy {
                state.viewer_queued_preview = state.selected;
                state.status = "Asset preview queued until the current operation finishes.".into();
                return Task::none();
            }
            let Some(inventory) = &state.inventory else {
                state.status =
                    "Extract the PAK into an isolated folder and index it to preview files.".into();
                return Task::none();
            };
            let Some(relative) = state
                .selected
                .and_then(|index| inventory.entries.get(index))
            else {
                state.status = "Select an extracted file in the explorer first.".into();
                return Task::none();
            };
            let source = PathBuf::from(&inventory.root).join(&relative.relative_path);
            let canonical_root = std::fs::canonicalize(&inventory.root);
            let canonical_source = std::fs::canonicalize(&source);
            if !matches!(
                (canonical_root, canonical_source),
                (Ok(root), Ok(file)) if file.starts_with(&root)
                    && std::fs::symlink_metadata(&source)
                        .is_ok_and(|meta| meta.file_type().is_file())
            ) {
                state.status =
                    "Selected file is missing or escaped the indexed root; rescan before preview."
                        .into();
                return Task::none();
            }
            state.input = source.to_string_lossy().to_string();
            state.texture_preview = None;
            let filename = relative.relative_path.to_ascii_lowercase();
            // The explicit Preview button must use the same complete CDF
            // assembly pipeline as single-click auto-preview. Parsing only
            // dependency metadata here left the viewer with no geometry.
            if filename.ends_with(".cdf") {
                state.gfx_active = false;
                return update(state, Message::ViewerPreviewStaticCdf);
            }
            if filename.ends_with(".gfx") {
                return update(state, Message::GfxOpenSelected);
            }
            state.gfx_active = false;
            let operation = if is_viewer_model_source(&filename) {
                Operation::InspectIndexedModel(
                    source,
                    relative.relative_path.clone(),
                    inventory.clone(),
                    state
                        .library_catalog
                        .as_ref()
                        .map(|c| PathBuf::from(&c.root)),
                    resource_build(state),
                )
            } else if filename.ends_with(".dds")
                || filename.rsplit_once(".dds.").is_some_and(|(_, suffix)| {
                    let mip = suffix.strip_suffix('a').unwrap_or(suffix);
                    mip.parse::<u8>().is_ok_and(|n| n < 20)
                })
            {
                Operation::PreviewDds(
                    source,
                    relative.relative_path.clone(),
                    inventory.clone(),
                    state
                        .library_catalog
                        .as_ref()
                        .map(|c| PathBuf::from(&c.root)),
                    resource_build(state),
                    state.preview_cache.clone(),
                )
            } else if filename.ends_with(".caf") {
                Operation::LoadCaf(source)
            } else if filename.ends_with(".mtl") {
                Operation::ResolveMaterial(
                    source,
                    PathBuf::from(&inventory.root),
                    state
                        .library_catalog
                        .as_ref()
                        .map(|catalog| PathBuf::from(&catalog.root)),
                    resource_build(state),
                )
            } else {
                state.status =
                    "Selected asset type is not supported by the native preview yet.".into();
                return Task::none();
            };
            return launch(state, operation);
        }
        Message::GfxConfigChanged(path) => {
            if path.len() <= 4096 {
                state.preferences.ffdec_executable = path;
                persist_preferences(state);
            }
        }
        Message::GfxPickBackend => {
            return Task::perform(
                async {
                    tokio::task::spawn_blocking(|| native_export_picker("file"))
                        .await
                        .ok()
                        .flatten()
                },
                Message::GfxBackendPicked,
            );
        }
        Message::GfxBackendPicked(selected) => {
            if let Some(path) = selected {
                return update(
                    state,
                    Message::GfxConfigChanged(path.to_string_lossy().into()),
                );
            }
        }
        Message::GfxSelectResource(index) => {
            if state
                .gfx_session
                .as_ref()
                .is_some_and(|s| index < s.info.resources.len())
            {
                state.gfx_resource = index;
            }
        }
        Message::GfxOpenSelected => {
            if state.gfx_busy {
                state.status = "GFX edit is still running; wait for FFDec to finish.".into();
                return Task::none();
            }
            let source = match verified_viewer_file(state) {
                Ok(p)
                    if p.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| e.eq_ignore_ascii_case("gfx")) =>
                {
                    p
                }
                _ => {
                    state.status = "Select an indexed original .gfx movie before editing.".into();
                    return Task::none();
                }
            };
            let relative = state
                .selected
                .and_then(|i| state.inventory.as_ref()?.entries.get(i))
                .map(|e| e.relative_path.clone())
                .unwrap_or_default();
            if let Some(current) = &state.gfx_session {
                if current.source == source {
                    state.gfx_active = true;
                    return Task::none();
                }
                if current.edits > 0 && !current.applied {
                    state.gfx_active = true;
                    state.gfx_status =
                        "Unsaved staged GFX edits exist. Apply or undo them before changing movies.".into();
                    return Task::none();
                }
            }
            let project = if state.viewer_from_library {
                None
            } else {
                state
                    .modding_selected
                    .and_then(|i| state.modding_projects.as_ref()?.get(i))
                    .filter(|p| {
                        std::fs::canonicalize(p.workspace.join("files")).ok()
                            == std::fs::canonicalize(
                                state
                                    .inventory
                                    .as_ref()
                                    .map(|i| i.root.as_str())
                                    .unwrap_or(""),
                            )
                            .ok()
                    })
                    .cloned()
            };
            let library_root = state
                .library_catalog
                .as_ref()
                .map(|catalog| PathBuf::from(&catalog.root));
            let selected_build = resource_build(state).or_else(|| {
                project.as_ref().and_then(|p| {
                    (p.source_build != "Unknown build" && !p.source_build.is_empty())
                        .then(|| p.source_build.clone())
                })
            });
            state.gfx_busy = true;
            state.gfx_active = true;
            state.gfx_preview = None;
            state.gfx_status =
                "Inspecting source GFX and resolving verified same-build imports...".into();
            return Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        gfx_editor::create(source, relative, project, library_root, selected_build)
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()))
                },
                Message::GfxOpened,
            );
        }
        Message::GfxOpened(result) => {
            state.gfx_busy = false;
            match result {
                Ok(session) => {
                    state.gfx_status = if session.project.is_some() {
                        "Editable Mod Project GFX session ready. Original PAK remains untouched."
                    } else {
                        "Read-only Asset Library preview. Open a Mod Project for Apply."
                    }
                    .into();
                    state.gfx_session = Some(session);
                    state.gfx_resource = 0;
                    state.gfx_live_snapshot = None;
                    state.gfx_layout_selected = None;
                    state.gfx_active = true;
                }
                Err(e) => {
                    state.gfx_active = false;
                    state.gfx_status = format!("GFX validation failed: {e}");
                    state.status = state.gfx_status.clone();
                }
            }
        }
        Message::GfxToggleLiveInspector => {
            state.gfx_live_open = !state.gfx_live_open;
            state.gfx_live_status = "Read-only hierarchy inspector. A verified native Evolve GFx snapshot provider is not yet available; imported JSON is external data, not a live capture.".into();
        }
        Message::GfxImportUiSnapshot => {
            return Task::perform(
                async {
                    tokio::task::spawn_blocking(|| native_export_picker("file"))
                        .await
                        .ok()
                        .flatten()
                },
                Message::GfxUiSnapshotPicked,
            );
        }
        Message::GfxUiSnapshotPicked(selected) => {
            let Some(source) = selected else {
                return Task::none();
            };
            return Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || gfx_live_inspector::from_file(&source))
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()))
                },
                Message::GfxUiSnapshotLoaded,
            );
        }
        Message::GfxUiSnapshotLoaded(result) => match result {
            Ok(snapshot) => {
                state.gfx_live_status = format!(
                    "Loaded {} read-only nodes from imported snapshot; not captured live by this manager.",
                    snapshot.nodes.len()
                );
                state.gfx_live_snapshot = Some(snapshot);
                state.gfx_live_open = true;
            }
            Err(error) => {
                state.gfx_live_status = format!("Cannot inspect UI hierarchy: {error}");
            }
        },
        Message::GfxToggleLayoutRaster => {
            state.gfx_layout_show_raster = !state.gfx_layout_show_raster
        }
        Message::GfxToggleLayout => {
            state.gfx_layout_open = !state.gfx_layout_open;
            if state.gfx_layout_open
                && state.gfx_layout_selected.is_none()
                && let Some(index) = state
                    .gfx_session
                    .as_ref()
                    .and_then(|s| s.layers.iter().position(|layer| layer.editable))
            {
                return update(state, Message::GfxLayoutSelect(index));
            }
        }
        Message::GfxLayoutSelect(index) => {
            if let Some(layer) = state.gfx_session.as_ref().and_then(|s| s.layers.get(index)) {
                state.gfx_layout_selected = Some(index);
                state.gfx_layout_x = format!("{:.2}", layer.x);
                state.gfx_layout_y = format!("{:.2}", layer.y);
                state.gfx_layout_sx = format!("{:.2}", layer.scale_x * 100.);
                state.gfx_layout_sy = format!("{:.2}", layer.scale_y * 100.);
                state.gfx_layout_rotation = format!("{:.2}", layer.rotation);
                state.gfx_layout_visible = true;
            }
        }
        Message::GfxLayoutDragged(index, dx, dy) => {
            if !state.gfx_busy && dx.is_finite() && dy.is_finite() {
                let Some(layer) = state.gfx_session.as_ref().and_then(|s| s.layers.get(index))
                else {
                    return Task::none();
                };
                if layer.editable && layer.bounds.is_some() {
                    let x = layer.x + dx;
                    let y = layer.y + dy;
                    return update_layout_drag(state, index, x, y);
                }
            }
        }
        Message::GfxLayoutValue(field, value) => {
            if value.len() < 50 {
                match field {
                    "x" => state.gfx_layout_x = value,
                    "y" => state.gfx_layout_y = value,
                    "sx" => state.gfx_layout_sx = value,
                    "sy" => state.gfx_layout_sy = value,
                    "rotation" => state.gfx_layout_rotation = value,
                    _ => (),
                }
            }
        }
        Message::GfxLayoutVisible(visible) => state.gfx_layout_visible = visible,
        Message::GfxStageLayout => {
            let Some(layer) = state
                .gfx_layout_selected
                .and_then(|i| state.gfx_session.as_ref()?.layers.get(i))
            else {
                return Task::none();
            };
            if !layer.editable || state.gfx_busy {
                state.gfx_status = "Select a verified editable root first-frame placement.".into();
                return Task::none();
            }
            let read = |value: &str| value.trim().parse::<f32>().ok().filter(|n| n.is_finite());
            let values = [
                read(&state.gfx_layout_x),
                read(&state.gfx_layout_y),
                read(&state.gfx_layout_sx),
                read(&state.gfx_layout_sy),
                read(&state.gfx_layout_rotation),
            ];
            let [Some(x), Some(y), Some(sx), Some(sy), Some(rotation)] = values else {
                state.gfx_status =
                    "Enter finite numeric position, scale and rotation values.".into();
                return Task::none();
            };
            return update(
                state,
                Message::GfxAction(gfx_editor::Action::StageLayout(evolve_core::LayoutEdit {
                    depth: layer.depth,
                    character: layer.character,
                    x,
                    y,
                    scale_x: sx / 100.,
                    scale_y: sy / 100.,
                    rotation,
                    visible: state.gfx_layout_visible,
                })),
            );
        }
        Message::GfxPickReplacement => {
            return Task::perform(
                async {
                    tokio::task::spawn_blocking(|| native_export_picker("file"))
                        .await
                        .ok()
                        .flatten()
                },
                Message::GfxReplacementPicked,
            );
        }
        Message::GfxPickEdited => {
            return Task::perform(
                async {
                    tokio::task::spawn_blocking(|| native_export_picker("file"))
                        .await
                        .ok()
                        .flatten()
                },
                Message::GfxEditedPicked,
            );
        }
        Message::GfxEditedPicked(selected) => {
            if let Some(path) = selected {
                return update(
                    state,
                    Message::GfxAction(gfx_editor::Action::ImportGfx(path)),
                );
            }
        }
        Message::GfxRevealSession => {
            if let Some(session) = &state.gfx_session {
                #[cfg(target_os = "windows")]
                let command = std::process::Command::new("explorer.exe")
                    .arg(session.folder.path())
                    .spawn();
                #[cfg(not(target_os = "windows"))]
                let command = std::process::Command::new("xdg-open")
                    .arg(session.folder.path())
                    .spawn();
                state.gfx_status = if command.is_ok() {
                    "Opened GFX editing scratch folder; modify exported XML/scripts there.".into()
                } else {
                    format!(
                        "Temporary editor folder: {}",
                        session.folder.path().display()
                    )
                };
            }
        }
        Message::GfxReplacementPicked(selected) => {
            if let Some(replacement) = selected {
                return update(
                    state,
                    Message::GfxAction(gfx_editor::Action::ReplaceResource {
                        index: state.gfx_resource,
                        replacement,
                    }),
                );
            }
        }
        Message::GfxApply => {
            return update(
                state,
                Message::GfxAction(gfx_editor::Action::Apply {
                    projects_root: PathBuf::from(&state.preferences.projects),
                }),
            );
        }
        Message::GfxAction(action) => {
            if state.gfx_busy || state.busy {
                state.gfx_status = "Wait until the current worker finishes.".into();
                return Task::none();
            }
            let Some(session) = state.gfx_session.clone() else {
                state.gfx_status = "Select a GFX movie to start editing.".into();
                return Task::none();
            };
            let configured = state.preferences.ffdec_executable.clone();
            // Any edit/undo/FFDec GUI handoff invalidates the old raster. Do
            // not paint a stale frame behind newly moved GFX placements.
            if !matches!(
                action,
                gfx_editor::Action::PreviewFrame
                    | gfx_editor::Action::ExportXml
                    | gfx_editor::Action::ExportScripts
            ) {
                state.gfx_preview = None;
            }
            state.gfx_busy = true;
            state.gfx_status = "Running checked FFDec operation in private edit copy...".into();
            return Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        gfx_editor::run(session, action, &configured)
                    })
                    .await
                    .unwrap_or_else(|e| Err(format!("GFX worker failed: {e}")))
                },
                Message::GfxFinished,
            );
        }
        Message::GfxFinished(result) => {
            state.gfx_busy = false;
            match result {
                Ok((session, note, preview)) => {
                    let selected_key = state
                        .gfx_layout_selected
                        .and_then(|index| state.gfx_session.as_ref()?.layers.get(index))
                        .map(|layer| (layer.depth, layer.character));
                    let retained = selected_key.and_then(|(depth, character)| {
                        session
                            .layers
                            .iter()
                            .position(|layer| layer.depth == depth && layer.character == character)
                    });
                    state.gfx_status = note;
                    state.gfx_session = Some(session);
                    state.gfx_layout_selected = None;
                    if let Some(index) = retained {
                        let _ = update(state, Message::GfxLayoutSelect(index));
                    }
                    if let Some(image_bytes) = preview {
                        state.gfx_preview = Some(image::Handle::from_bytes(image_bytes));
                    }
                }
                Err(e) => {
                    state.gfx_status = format!("FFDec editing failed: {e}");
                }
            }
        }
        Message::PreferenceDensity => {
            state.preferences.ui_density = if state.preferences.ui_density == "Compact" {
                "Comfortable".into()
            } else {
                "Compact".into()
            };
            persist_preferences(state);
        }
        Message::PreferenceExplorerSide => {
            state.preferences.file_explorer_side = if state.preferences.file_explorer_side == "Left"
            {
                "Right".into()
            } else {
                "Left".into()
            };
            persist_preferences(state);
        }
        Message::PreferenceAccentPreset(preset) => {
            if settings::apply_accent_preset(&mut state.preferences, preset) {
                persist_preferences(state);
                state.settings_status = format!("Appearance preset applied: {preset}");
            }
        }
        Message::PreferenceAppearance => {
            state.preferences.appearance_mode = if state.preferences.appearance_mode == "Dark" {
                "Light".into()
            } else {
                "Dark".into()
            };
            persist_preferences(state);
        }
        Message::PreferenceReduceMotion => {
            state.preferences.reduce_motion = !state.preferences.reduce_motion;
            persist_preferences(state);
        }
        Message::PreferenceSnapshots => {
            state.preferences.keep_original_snapshots = !state.preferences.keep_original_snapshots;
            persist_preferences(state);
        }
        Message::PreferenceActivity => {
            state.preferences.show_activity_on_tasks = !state.preferences.show_activity_on_tasks;
            persist_preferences(state);
        }
        Message::PreferenceCrashLogging => {
            state.preferences.crash_logging_enabled = !state.preferences.crash_logging_enabled;
            crash_diagnostics::set_enabled(state.preferences.crash_logging_enabled);
            persist_preferences(state);
            if !state
                .settings_status
                .starts_with("Preferences could not be saved")
            {
                state.settings_status = if state.preferences.crash_logging_enabled {
                    "Crash logs enabled. New preview events and Rust panics are recorded locally."
                        .into()
                } else {
                    "Crash logs disabled. Existing log files were left untouched.".into()
                };
            }
        }
        Message::OpenCrashLogs | Message::OpenCrashLogsFolder => {
            match crash_diagnostics::log_path() {
                Ok(path) => {
                    let opening_folder = matches!(&message, Message::OpenCrashLogsFolder);
                    let log_folder = path.parent().map(Path::to_path_buf);
                    let target = if opening_folder {
                        log_folder.clone()
                    } else if path.exists() {
                        Some(path)
                    } else {
                        log_folder.clone()
                    };
                    if let Some(target) = target {
                        if let Some(folder) = log_folder {
                            let _ = std::fs::create_dir_all(folder);
                        }
                        #[cfg(windows)]
                        let result = if opening_folder || target.is_dir() {
                            std::process::Command::new("explorer.exe")
                                .arg(&target)
                                .spawn()
                        } else {
                            std::process::Command::new("notepad.exe")
                                .arg(&target)
                                .spawn()
                        };
                        #[cfg(not(windows))]
                        let result = std::process::Command::new("xdg-open").arg(&target).spawn();
                        state.settings_status = match result {
                            Ok(_) => format!("Opening {}", target.display()),
                            Err(error) => format!("Could not open crash log: {error}"),
                        };
                    }
                }
                Err(error) => {
                    state.settings_status = format!("Crash log path unavailable: {error}")
                }
            }
        }
        Message::PreferenceUpdateChannel => {
            if state.preferences.nightly_updates {
                state.preferences.nightly_updates = false;
                state.preferences.experimental_updates = false;
            } else if state.preferences.experimental_updates {
                state.preferences.nightly_updates = true;
                state.preferences.experimental_updates = false;
            } else {
                state.preferences.nightly_updates = false;
                state.preferences.experimental_updates = true;
            }
            state.available_update = None;
            persist_preferences(state);
        }
        Message::PreferenceAutoUpdates => {
            state.preferences.automatic_update_checks = !state.preferences.automatic_update_checks;
            persist_preferences(state);
        }
        Message::SettingsToggleAdvancedPaths => {
            state.settings_advanced_paths = !state.settings_advanced_paths
        }
        Message::PreferenceGameStorage(value) => {
            state.preferences.game_storage_root = value;
            state.storage_usage = None;
        }
        Message::SettingsAnalyzeStorage => {
            if state.busy || state.storage_analyzing {
                return Task::none();
            }
            let games = PathBuf::from(&state.preferences.game_storage_root);
            let data = games
                .parent()
                .map(PathBuf::from)
                .unwrap_or_else(|| games.clone());
            let stage = PathBuf::from(&state.preferences.stage);
            let projects = PathBuf::from(&state.preferences.projects);
            let library = PathBuf::from(&state.library_root);
            state.storage_analyzing = true;
            return launch(
                state,
                Operation::AnalyzeStorage(data, games, stage, projects, library),
            );
        }
        Message::PreferenceStage(value) => {
            state.preferences.stage = value;
            state.modding_projects = None;
        }
        Message::PreferenceSwap(value) => state.preferences.swap = value,
        Message::PreferenceProjects(value) => {
            state.preferences.projects = value;
            state.modding_projects = None;
        }
        Message::PreferenceAssets(value) => {
            state.library_root = value.clone();
            state.storage_usage = None;
            state.library_catalog = None;
            state.library_selection.clear();
            state.library_selected = None;
            state.library_page = 0;
            state.preferences.asset_library_root = value;
            refresh_library_matches(state);
        }
        Message::PreferenceBlender(value) => {
            state.blender_path = value.clone();
            state.preferences.blender_executable = value;
        }
        Message::PreferenceWorkspace(value) => state.preferences.current_workspace = value,
        Message::SavePreferences => persist_preferences(state),
        Message::CheckUpdates => {
            state.startup_update_checked = true;
            state.update_status = "Checking public GitHub Releases for verified updates...".into();
            state.available_update = None;
            let channel = if state.preferences.nightly_updates {
                UpdateChannel::Nightly
            } else if state.preferences.experimental_updates {
                UpdateChannel::Beta
            } else {
                UpdateChannel::Stable
            };
            return check_updates_task(channel);
        }
        Message::UpdateChecked(result) => match result {
            Ok(Some(update)) => {
                state.update_status = format!(
                    "Update {} ({:?}) is available. Portable: {}. Release: {}",
                    update.version, update.channel, update.portable.name, update.release_url
                );
                state.available_update = Some(update);
            }
            Ok(None) => {
                state.update_status = "No newer verified release on the selected channel.".into()
            }
            Err(error) => state.update_status = format!("Cannot verify latest release: {error}"),
        },
        Message::ApplyUpdate => {
            let Some(update) = state.available_update.clone() else {
                return Task::none();
            };
            if state.busy {
                return Task::none();
            }
            let Ok(settings) = settings_path() else {
                state.update_status = "Windows user settings folder could not be located.".into();
                return Task::none();
            };
            let Some(root) = settings.parent().map(PathBuf::from) else {
                state.update_status = "Application update directory is unavailable.".into();
                return Task::none();
            };
            state.busy = true;
            state.update_status =
                "Downloading and checking the verified update. The manager will restart when ready..."
                    .into();
            return Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        prepare_update(&update, &root).map_err(|error| error.to_string())
                    })
                    .await
                    .map_err(|error| format!("Update worker stopped: {error}"))?
                },
                Message::UpdatePrepared,
            );
        }
        Message::UpdatePrepared(result) => {
            state.busy = false;
            match result {
                Ok(update) => match launch_worker(&update) {
                    Ok(()) => {
                        state.update_status =
                            "Verified update prepared. Closing to install and restart...".into();
                        // The external helper replaces the executable only
                        // after this process has released all Windows handles.
                        return iced::exit();
                    }
                    Err(error) => {
                        state.update_status =
                            format!("Update was verified but could not start: {error}");
                    }
                },
                Err(error) => {
                    state.update_status = format!("Could not install update: {error}");
                }
            }
        }

        Message::InputChanged(input) => state.input = input,
        Message::OutputChanged(output) => state.output = output,
        Message::ReplacementsChanged(replacements) => state.replacements = replacements,
        Message::SignedMemberChanged(value) => state.signed_entry = value,
        Message::SignedReplacementChanged(value) => state.signed_replacement = value,
        Message::HookOriginalChanged(value) => state.hook_original = value,
        Message::HookPreparedChanged(value) => state.hook_prepared = value,
        Message::ProfileNameChanged(value) => state.profile_name = value,
        Message::SelectProfile(value) => state.profile_id = value,
        Message::StartProfileRecover => {
            if state.preferences.stage.trim().is_empty() {
                state.status = "Choose an isolated offline stage to recover.".into();
                return Task::none();
            }
            return launch(
                state,
                Operation::RecoverProfile(PathBuf::from(&state.preferences.stage)),
            );
        }
        Message::StartProfileStatus => {
            if state.preferences.stage.trim().is_empty() {
                state.status = "Choose an isolated offline stage.".into();
                return Task::none();
            }
            return launch(
                state,
                Operation::StatusProfile(PathBuf::from(&state.preferences.stage)),
            );
        }
        Message::StartProfileApply => {
            if state.preferences.stage.trim().is_empty() || state.profile_id.trim().is_empty() {
                state.status = "Select an offline profile and prepared stage.".into();
                return Task::none();
            }
            let key =
                match modding_keys::public_key(PathBuf::from(&state.preferences.stage).as_path()) {
                    Ok(value) => value,
                    Err(error) => {
                        state.status = error;
                        return Task::none();
                    }
                };
            return launch(
                state,
                Operation::ApplyProfile(
                    PathBuf::from(&state.preferences.stage),
                    state.profile_id.clone(),
                    key.clone(),
                    key,
                ),
            );
        }
        Message::StartProfileList => {
            if state.preferences.stage.trim().is_empty() {
                state.status = "Choose an isolated PAK stage first.".into();
                return Task::none();
            }
            return launch(
                state,
                Operation::ListProfiles(PathBuf::from(&state.preferences.stage)),
            );
        }
        Message::StartProfileCreate => {
            if state.profile_name.trim().is_empty() || state.preferences.stage.trim().is_empty() {
                state.status = "Enter a profile name and initialize an isolated stage.".into();
                return Task::none();
            }
            return launch(
                state,
                Operation::CreateProfile(
                    PathBuf::from(&state.preferences.stage),
                    state.profile_name.clone(),
                ),
            );
        }
        Message::StartProfileAdd => {
            if state.profile_id.trim().is_empty() || state.input.trim().is_empty() {
                state.status = "Select a profile and signed replacement file.".into();
                return Task::none();
            }
            let target = match selected_stage_target(state) {
                Ok(target) => target,
                Err(error) => {
                    state.status = error;
                    return Task::none();
                }
            };
            let stage = PathBuf::from(&state.preferences.stage);
            let key = match modding_keys::public_key(&stage) {
                Ok(key) => key,
                Err(error) => {
                    state.status = error;
                    return Task::none();
                }
            };
            let basename = match modding_keys::signed_basename(&stage.join("paks").join(&target)) {
                Ok(name) => name,
                Err(error) => {
                    state.status = error;
                    return Task::none();
                }
            };
            return launch(
                state,
                Operation::AddProfileMod(
                    stage,
                    state.profile_id.clone(),
                    PathBuf::from(&state.input),
                    target,
                    key.clone(),
                    key,
                    basename,
                ),
            );
        }
        Message::StartProfilePlan => {
            if state.profile_id.trim().is_empty() {
                state.status = "Select an offline profile first.".into();
                return Task::none();
            }
            let stage = PathBuf::from(&state.preferences.stage);
            let key = match modding_keys::public_key(&stage) {
                Ok(key) => key,
                Err(error) => {
                    state.status = error;
                    return Task::none();
                }
            };
            return launch(
                state,
                Operation::PlanProfile(stage, state.profile_id.clone(), key.clone(), key),
            );
        }
        Message::StartStageInit => {
            if state.preferences.stage.trim().is_empty() {
                state.status = "Choose a NEW isolated PAK workspace path under Settings.".into();
                return Task::none();
            }
            return launch(
                state,
                Operation::StageInit(PathBuf::from(&state.preferences.stage)),
            );
        }
        Message::StartStageStatus => {
            if state.preferences.stage.trim().is_empty() {
                state.status = "Specify the isolated staging workspace path.".into();
                return Task::none();
            }
            return launch(
                state,
                Operation::StageStatus(PathBuf::from(&state.preferences.stage)),
            );
        }
        Message::StartStageImport => {
            let target = match selected_stage_target(state) {
                Ok(target) => target,
                Err(error) => {
                    state.status = error;
                    return Task::none();
                }
            };
            let stage = PathBuf::from(&state.preferences.stage);
            let key = match modding_keys::public_key(&stage) {
                Ok(key) => key,
                Err(error) => {
                    state.status = error;
                    return Task::none();
                }
            };
            let basename = match modding_keys::signed_basename(&stage.join("paks").join(&target)) {
                Ok(name) => name,
                Err(error) => {
                    state.status = error;
                    return Task::none();
                }
            };
            return launch(
                state,
                Operation::StageImport(stage, PathBuf::from(&state.input), target, key, basename),
            );
        }
        Message::StartStageApply => {
            let target = match selected_stage_target(state) {
                Ok(target) => target,
                Err(error) => {
                    state.status = error;
                    return Task::none();
                }
            };
            let stage = PathBuf::from(&state.preferences.stage);
            let key = match modding_keys::public_key(&stage) {
                Ok(key) => key,
                Err(error) => {
                    state.status = error;
                    return Task::none();
                }
            };
            let basename = match modding_keys::signed_basename(&stage.join("paks").join(&target)) {
                Ok(name) => name,
                Err(error) => {
                    state.status = error;
                    return Task::none();
                }
            };
            return launch(
                state,
                Operation::StageApply(
                    stage,
                    PathBuf::from(&state.input),
                    target,
                    key.clone(),
                    key,
                    basename,
                ),
            );
        }
        Message::StartOfflineLauncherCheck
        | Message::StartOfflineLauncherLaunch
        | Message::StartOfflineLauncherRestore => {
            if state.preferences.stage.trim().is_empty() || state.preferences.swap.trim().is_empty()
            {
                state.status =
                    "Configure the private CustomSigning/staged and controlled-swap helper folders in Settings first.".into();
                return Task::none();
            }
            let stage = PathBuf::from(&state.preferences.stage);
            let swap = PathBuf::from(&state.preferences.swap);
            if matches!(message, Message::StartOfflineLauncherCheck) {
                return launch(state, Operation::CheckExistingOfflineLauncher(stage, swap));
            }
            if matches!(message, Message::StartOfflineLauncherRestore) {
                return launch(
                    state,
                    Operation::RestoreExistingOfflineLauncher(stage, swap),
                );
            }
            return launch(state, Operation::StartExistingOfflineLauncher(stage, swap));
        }
        Message::StartStageRestore => {
            if state.preferences.stage.trim().is_empty() {
                state.status = "Specify the isolated staging workspace path.".into();
                return Task::none();
            }
            return launch(
                state,
                Operation::StageRestore(PathBuf::from(&state.preferences.stage)),
            );
        }
        Message::DiffuseInputChanged(value) => state.diffuse_input = value,
        Message::NormalInputChanged(value) => state.normal_input = value,
        Message::EmissiveInputChanged(value) => state.emissive_input = value,
        Message::MaterialInputChanged(value) => state.material_input = value,
        Message::StartMaterialResolve => {
            if state.material_input.trim().is_empty() || state.assets_root.trim().is_empty() {
                state.status =
                    "Choose exact CryEngine .mtl file and extracted Asset Library root.".into();
                return Task::none();
            }
            return launch(
                state,
                Operation::ResolveMaterial(
                    PathBuf::from(&state.material_input),
                    PathBuf::from(&state.assets_root),
                    state
                        .library_catalog
                        .as_ref()
                        .map(|catalog| PathBuf::from(&catalog.root)),
                    resource_build(state),
                ),
            );
        }
        Message::CafInputChanged(value) => state.caf_path = value,
        Message::SkeletonChanged(value) => state.skeleton_path = value,
        Message::BlenderChanged(value) => state.blender_path = value,
        Message::ConverterPathChanged(value) => state.converter_path = value,
        Message::SelectMotionClip(index) => {
            state.selected_motion_clip = index;
            state.motion_time = 0.0;
            state.playing = false;
        }
        Message::OpenConvertedGlb(index) => {
            if let Some(path) = state.converted_glbs.get(index).cloned() {
                return launch(state, Operation::LoadGlbMotion(path));
            }
        }
        Message::MotionScrub(value) => {
            state.motion_time = value;
            apply_current_skin_frame(state);
        }
        Message::ToggleMotion => {
            if state.native_caf.is_some()
                && state.skin_preview.is_none()
                && state.cdf_caf_preview.is_none()
            {
                state.playing = false;
                state.status =
                    "CAF controller keys decoded, but selected mesh has no verified matching skin; cannot animate an unbound model.".into();
            } else {
                state.playing = !state.playing
                    && (state.skin_preview.is_some()
                        || state.cdf_caf_preview.is_some()
                        || state.glb_motion.as_ref().is_some_and(|m| {
                            m.list_clips().get(state.selected_motion_clip).is_some()
                        }));
            }
        }
        Message::Tick => {
            if state.cdf_surfaces.as_ref().is_some_and(|surfaces| {
                surfaces.iter().any(|surface| surface.emissive_uv.is_some())
            }) {
                state.decal_time = (state.decal_time + 0.04).rem_euclid(3600.0);
            }
            if state.playing {
                let duration = if let Some(motion) = &state.native_caf {
                    Some((motion.duration_seconds(), motion.cyclic))
                } else {
                    state.glb_motion.as_ref().and_then(|m| {
                        m.list_clips()
                            .get(state.selected_motion_clip)
                            .map(|clip| (clip.duration, false))
                    })
                };
                if let Some((duration, looping)) = duration {
                    state.motion_time += 0.04 * state.playback_speed.max(0.1);
                    if duration <= 0.0 {
                        state.playing = false;
                    } else if state.motion_time >= duration {
                        if looping || state.playback_loop {
                            state.motion_time %= duration;
                        } else {
                            state.motion_time = duration;
                            state.playing = false;
                        }
                    }
                } else {
                    state.playing = false;
                }
                apply_current_skin_frame(state);
            }
        }
        Message::AssetsRootChanged(value) => state.assets_root = value,
        Message::TerrainSourceChanged(value) => state.terrain_source = value,
        Message::YawChanged(value) => state.yaw = value,
        Message::PitchChanged(value) => state.pitch = value,
        Message::ZoomChanged(value) => state.zoom = value,
        Message::SearchChanged(query) => state.search = query,
        Message::StartScan => {
            let source = PathBuf::from(&state.input);
            return begin_scan(state, source);
        }
        Message::StartPakInspection => {
            let source = PathBuf::from(&state.input);
            return launch(state, Operation::InspectPak(source));
        }
        Message::StartRigGlb => {
            if state.input.trim().is_empty()
                || state.skeleton_path.trim().is_empty()
                || state.output.trim().is_empty()
            {
                state.status="Choose the original skin mesh, EXACT matching CryEngine skeleton and a NEW .glb output.".into();
                return Task::none();
            }
            return launch(
                state,
                Operation::RiggedGlb(
                    PathBuf::from(&state.input),
                    PathBuf::from(&state.skeleton_path),
                    PathBuf::from(&state.output),
                ),
            );
        }
        Message::StartRigFbx => {
            let blender = match resolved_blender(state) {
                Ok(path) => path,
                Err(message) => {
                    state.status = message;
                    return Task::none();
                }
            };
            if state.input.trim().is_empty()
                || state.skeleton_path.trim().is_empty()
                || state.output.trim().is_empty()
            {
                state.status="Choose original skin mesh, matching skeleton, Blender executable and NEW .fbx output.".into();
                return Task::none();
            }
            return launch(
                state,
                Operation::RiggedFbx(
                    PathBuf::from(&state.input),
                    PathBuf::from(&state.skeleton_path),
                    blender,
                    (!state.diffuse_input.trim().is_empty())
                        .then(|| PathBuf::from(&state.diffuse_input)),
                    (state.export_include_animation && !state.caf_path.trim().is_empty())
                        .then(|| PathBuf::from(&state.caf_path)),
                    PathBuf::from(&state.output),
                ),
            );
        }
        Message::StartMotionConversion => {
            if state.converter_path.trim().is_empty()
                || state.caf_path.trim().is_empty()
                || state.assets_root.trim().is_empty()
                || state.output.trim().is_empty()
            {
                state.status="Select Cryengine-Converter.exe, original CAF/DBA, exact asset root and NEW output folder.".into();
                return Task::none();
            }
            return launch(
                state,
                Operation::ConvertMotion(
                    PathBuf::from(&state.converter_path),
                    PathBuf::from(&state.caf_path),
                    PathBuf::from(&state.assets_root),
                    PathBuf::from(&state.output),
                ),
            );
        }
        Message::ViewerLayerClip(index) => {
            if state.busy || (state.skin_preview.is_none() && state.cdf_caf_preview.is_none()) {
                return Task::none();
            }
            let Some(base) = state.native_caf.clone() else {
                return Task::none();
            };
            let Some(clip) = state.auto_clips.get(index) else {
                return Task::none();
            };
            let Some(path) = clip
                .source
                .clone()
                .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("caf")))
            else {
                state.status = "Layer playback requires an extracted native CAF.".into();
                return Task::none();
            };
            return launch(
                state,
                Operation::LoadCafLayer(state.input.clone(), base, clip.label.clone(), path),
            );
        }
        Message::ViewerLayerWeight(weight) => {
            if weight.is_finite() && (0.0..=1.0).contains(&weight) {
                state.caf_layer_weight = weight;
                apply_current_skin_frame(state);
            }
        }
        Message::ViewerClearLayer => {
            state.caf_layer = None;
            state.caf_layer_label.clear();
            apply_current_skin_frame(state);
        }
        Message::StartCafLoad => {
            state.auto_play_native_caf = false;
            if state.caf_path.trim().is_empty() {
                state.status="Select the original native CAF to inspect/play its exact bone controller tracks.".into();
                return Task::none();
            }
            return launch(state, Operation::LoadCaf(PathBuf::from(&state.caf_path)));
        }
        Message::StartNormalLoad => {
            if state.normal_input.trim().is_empty() {
                state.status = "Select a source tangent-space normal DDS (BC5 recommended).".into();
                return Task::none();
            }
            return launch(
                state,
                Operation::LoadNormal(
                    PathBuf::from(&state.normal_input),
                    state.preview_cache.clone(),
                ),
            );
        }
        Message::StartEmissiveLoad => {
            if state.emissive_input.trim().is_empty() {
                state.status =
                    "Select the exact source emissive DDS; no color tint will be guessed.".into();
                return Task::none();
            }
            return launch(
                state,
                Operation::LoadEmissive(
                    PathBuf::from(&state.emissive_input),
                    state.preview_cache.clone(),
                ),
            );
        }
        Message::StartDiffuseLoad => {
            if state.diffuse_input.trim().is_empty() {
                state.status =
                    "Select an exact original DDS diffuse texture or split-stream set.".into();
                return Task::none();
            }
            return launch(
                state,
                Operation::LoadDiffuse(
                    PathBuf::from(&state.diffuse_input),
                    state.preview_cache.clone(),
                ),
            );
        }
        Message::StartDdsInspection => {
            let source = PathBuf::from(&state.input);
            return launch(state, Operation::InspectDds(source));
        }
        Message::StartCdfResolve => {
            if state.input.trim().is_empty() || state.assets_root.trim().is_empty() {
                state.status = "Select character CDF and extracted Asset Library root.".into();
                return Task::none();
            }
            return launch(
                state,
                Operation::ResolveCdf(
                    PathBuf::from(&state.input),
                    PathBuf::from(&state.assets_root),
                ),
            );
        }
        Message::StartCdfInspection => {
            let source = PathBuf::from(&state.input);
            return launch(state, Operation::InspectCdf(source));
        }
        Message::StartModelInspection => {
            let source = PathBuf::from(&state.input);
            return launch(state, Operation::InspectModel(source));
        }
        Message::StartLevelInspection => {
            let source = PathBuf::from(&state.input);
            return launch(state, Operation::InspectLevel(source));
        }
        Message::StartTerrainInspection => {
            let source = PathBuf::from(&state.input);
            return launch(state, Operation::InspectTerrain(source));
        }
        Message::StartSignedInspection => {
            let source = PathBuf::from(&state.input);
            let stage = PathBuf::from(&state.preferences.stage);
            let basename = match modding_keys::signed_basename(&source) {
                Ok(name) => name,
                Err(error) => {
                    state.status = error;
                    return Task::none();
                }
            };
            return launch(state, Operation::InspectSignedPak(source, stage, basename));
        }
        Message::StartSignedExtraction => {
            let source = PathBuf::from(&state.input);
            let stage = PathBuf::from(&state.preferences.stage);
            let basename = match modding_keys::signed_basename(&source) {
                Ok(name) => name,
                Err(error) => {
                    state.status = error;
                    return Task::none();
                }
            };
            if state.output.trim().is_empty() {
                state.status = "Choose a new extraction folder.".into();
                return Task::none();
            }
            return launch(
                state,
                Operation::ExtractSignedPak(source, stage, basename, PathBuf::from(&state.output)),
            );
        }
        Message::StartSignedRebuild | Message::StartSignedResign => {
            let source = PathBuf::from(&state.input);
            let stage = PathBuf::from(&state.preferences.stage);
            let keys = match modding_keys::signing_keys(&stage) {
                Ok(keys) => keys,
                Err(error) => {
                    state.status = error;
                    return Task::none();
                }
            };
            let basename = match modding_keys::signed_basename(&source) {
                Ok(name) => name,
                Err(error) => {
                    state.status = error;
                    return Task::none();
                }
            };
            if state.output.trim().is_empty() {
                state.status =
                    "Select a NEW .pak output; the original game PAK is never overwritten.".into();
                return Task::none();
            }
            let output = PathBuf::from(&state.output);
            if matches!(message, Message::StartSignedResign) {
                return launch(
                    state,
                    Operation::ResignSignedPak(
                        source,
                        stage,
                        keys.private,
                        keys.public,
                        basename,
                        output,
                    ),
                );
            }
            if state.signed_entry.trim().is_empty() || state.signed_replacement.trim().is_empty() {
                state.status = "Select an exact existing virtual member and its replacement source file before rebuilding.".into();
                return Task::none();
            }
            return launch(
                state,
                Operation::RebuildSignedPak(
                    source,
                    stage,
                    keys.private,
                    keys.public,
                    basename,
                    state.signed_entry.clone(),
                    PathBuf::from(&state.signed_replacement),
                    output,
                ),
            );
        }
        Message::StartHookPrepare => {
            let stage = PathBuf::from(&state.preferences.stage);
            let keys = match modding_keys::signing_keys(&stage) {
                Ok(keys) => keys,
                Err(error) => {
                    state.status = error;
                    return Task::none();
                }
            };
            // The established private launcher expects stage/inject-custom.dll,
            // not a manually named inject.dll in an arbitrary export directory.
            // Keep a user-selected path when supplied, but allow fully
            // automatic discovery from this signing stage's source-root plan.
            let original = if state.hook_original.trim().is_empty() {
                match launcher_bridge::original_hook_for_stage(&stage) {
                    Ok(path) => path,
                    Err(error) => {
                        state.status = format!("Cannot locate original client hook: {error}");
                        return Task::none();
                    }
                }
            } else {
                PathBuf::from(&state.hook_original)
            };
            let output = if state.hook_prepared.trim().is_empty() {
                stage.join("inject-custom.dll")
            } else {
                PathBuf::from(&state.hook_prepared)
            };
            return launch(
                state,
                Operation::PrepareClientHook(original, keys.original_public, keys.public, output),
            );
        }
        Message::StartHookPreflight => {
            let stage = PathBuf::from(&state.preferences.stage);
            let keys = match modding_keys::signing_keys(&stage) {
                Ok(keys) => keys,
                Err(error) => {
                    state.status = error;
                    return Task::none();
                }
            };
            if state.input.trim().is_empty() || state.output.trim().is_empty() {
                state.status =
                    "Select the original Stage 2 PAK and its custom signed output before verifying the hook."
                        .into();
                return Task::none();
            }
            // The private manager always stages its shim as inject-custom.dll.
            // Resolve those exact paths automatically; only override them if
            // the user explicitly selected a different compatible source.
            let original_hook = if state.hook_original.trim().is_empty() {
                match launcher_bridge::original_hook_for_stage(&stage) {
                    Ok(path) => path,
                    Err(error) => {
                        state.status = format!("Original hook lookup failed: {error}");
                        return Task::none();
                    }
                }
            } else {
                PathBuf::from(&state.hook_original)
            };
            let custom_hook = if state.hook_prepared.trim().is_empty() {
                stage.join("inject-custom.dll")
            } else {
                PathBuf::from(&state.hook_prepared)
            };
            let source = PathBuf::from(&state.input);
            let basename = match modding_keys::signed_basename(&source) {
                Ok(name) => name,
                Err(error) => {
                    state.status = error;
                    return Task::none();
                }
            };
            return launch(
                state,
                Operation::VerifySignedHookPair(
                    source,
                    PathBuf::from(&state.output),
                    original_hook,
                    custom_hook,
                    keys.original_public,
                    keys.public,
                    basename,
                ),
            );
        }
        Message::StartLevelFbxExport => {
            let blender = match resolved_blender(state) {
                Ok(path) => path,
                Err(message) => {
                    state.status = message;
                    return Task::none();
                }
            };
            if [&state.input, &state.assets_root, &state.output]
                .iter()
                .any(|x| x.trim().is_empty())
                || !state.output.to_ascii_lowercase().ends_with(".fbx")
            {
                state.status = "Select level XML/LYR, extracted asset root, Blender executable and NEW .fbx output.".into();
                return Task::none();
            }
            let terrain = if state.terrain_source.trim().is_empty() {
                None
            } else {
                Some(PathBuf::from(&state.terrain_source))
            };
            return launch(
                state,
                Operation::ExportLevelFbx(
                    PathBuf::from(&state.input),
                    PathBuf::from(&state.assets_root),
                    terrain,
                    blender,
                    PathBuf::from(&state.output),
                ),
            );
        }
        Message::StartLevelGlbExport => {
            if state.input.trim().is_empty()
                || state.assets_root.trim().is_empty()
                || state.output.trim().is_empty()
            {
                state.status="Select level XML/LYR, exact extracted game assets root, and a NEW .glb output.".into();
                return Task::none();
            }
            let terrain = if state.terrain_source.trim().is_empty() {
                None
            } else {
                Some(PathBuf::from(&state.terrain_source))
            };
            return launch(
                state,
                Operation::ExportLevelGlb(
                    PathBuf::from(&state.input),
                    PathBuf::from(&state.assets_root),
                    terrain,
                    PathBuf::from(&state.output),
                ),
            );
        }
        Message::StartExtract => {
            if state.output.trim().is_empty() {
                state.status = "Choose a NEW extraction directory first.".into();
                return Task::none();
            }
            let source = PathBuf::from(&state.input);
            let output = PathBuf::from(&state.output);
            return launch(state, Operation::ExtractPak(source, output));
        }
        Message::StartRebuild => {
            if state.output.trim().is_empty() || state.replacements.trim().is_empty() {
                state.status = "Choose the overrides folder and a NEW output PAK path.".into();
                return Task::none();
            }
            let source = PathBuf::from(&state.input);
            let overrides = PathBuf::from(&state.replacements);
            let output = PathBuf::from(&state.output);
            return launch(state, Operation::RebuildPak(source, overrides, output));
        }
        Message::StartPngExport => {
            if state.output.trim().is_empty() {
                state.status = "Choose a NEW output PNG file first.".into();
                return Task::none();
            }
            let source = PathBuf::from(&state.input);
            let output = PathBuf::from(&state.output);
            return launch(state, Operation::ExportPng(source, output));
        }
        Message::Finished(result) => {
            state.busy = false;
            state.storage_analyzing = false;
            state.scan_cancel = None;
            state.scan_indexed = None;
            match result {
                Ok(Completed::StorageAnalyzed(usage)) => {
                    state.status = format!(
                        "Read-only storage analysis complete: {} files checked.",
                        usage.files
                    );
                    state.storage_usage = Some(usage);
                }
                Ok(Completed::Dependencies(graph)) => {
                    state.status = format!(
                        "Dependency graph: {} explicit links, {} missing, {} ambiguous.",
                        graph.edges.len(),
                        graph.missing,
                        graph.ambiguous
                    );
                    state.dependency_graph = Some(graph);
                    state.viewer_tab = ViewerTab::Dependencies;
                }
                Ok(Completed::Comparison(report)) => {
                    state.status = if report.identical {
                        "Original and modified files are byte-for-byte identical.".into()
                    } else {
                        "Modified asset differs from original; neither file was changed.".into()
                    };
                    state.comparison = Some(report);
                    state.viewer_tab = ViewerTab::Compare;
                }
                Ok(Completed::ImportedRawProjectMember(changed, member, workspace)) => {
                    state.status = if changed {
                        format!(
                            "Imported {member} into isolated project; verified pre-edit backup retained."
                        )
                    } else {
                        format!("{member} is already byte-identical to the selected replacement.")
                    };
                    state.modding_pending_focus = Some(workspace);
                    return launch(
                        state,
                        Operation::IndexProjects(
                            PathBuf::from(&state.preferences.projects),
                            PathBuf::from(&state.preferences.stage),
                        ),
                    );
                }
                Ok(Completed::RestoredRawProjectMember(changed, member, workspace)) => {
                    state.status = if changed {
                        format!(
                            "Restored original {member}; the edited revision is retained in EditorBackups. Preview selected to reload."
                        )
                    } else {
                        format!("{member} already matches its verified original.")
                    };
                    state.modding_pending_focus = Some(workspace);
                    return launch(
                        state,
                        Operation::IndexProjects(
                            PathBuf::from(&state.preferences.projects),
                            PathBuf::from(&state.preferences.stage),
                        ),
                    );
                }
                Ok(Completed::UnpackedProject(path, count)) => {
                    state.status = format!(
                        "Unpacked {count} original assets into a new editing project; originals unchanged."
                    );
                    state.modding_pending_focus = Some(path);
                    return launch(
                        state,
                        Operation::IndexProjects(
                            PathBuf::from(&state.preferences.projects),
                            PathBuf::from(&state.preferences.stage),
                        ),
                    );
                }
                Ok(Completed::Projects(projects)) => {
                    let focus = state.modding_pending_focus.take();
                    let previous = focus.clone().or_else(|| {
                        state
                            .modding_selected
                            .and_then(|i| {
                                state.modding_projects.as_ref().and_then(|old| old.get(i))
                            })
                            .map(|p| p.workspace.clone())
                    });
                    state.modding_selected =
                        previous.and_then(|path| projects.iter().position(|p| p.workspace == path));
                    state.status = format!("Loaded {} editable project(s).", projects.len());
                    state.modding_projects = Some(projects);
                    let matches = modding_page::matching_projects(state);
                    state.modding_page = state
                        .modding_selected
                        .and_then(|i| matches.iter().position(|v| *v == i))
                        .map_or(0, |i| i / modding_page::PAGE);
                    state.modding_changes_page = 0;
                    state.project_selection.clear();
                    if let Some(path) = focus {
                        state.modding_open_editor_when_scanned = true;
                        return begin_scan(state, path.join("files"));
                    }
                }
                Ok(Completed::ImportedSignedLibrary(report, catalog)) => {
                    let already = report
                        .imported
                        .iter()
                        .filter(|r| r.already_imported)
                        .count();
                    let added = report.imported.len() - already;
                    let failed = report.failed.len();
                    let details = report
                        .failed
                        .iter()
                        .take(3)
                        .map(|f| format!("{}: {}", f.archive, f.reason))
                        .collect::<Vec<_>>()
                        .join("; ");
                    state.status = format!(
                        "Asset Library import: {added} new, {already} already verified, {failed} failed. Originals unchanged.{}",
                        if details.is_empty() {
                            String::new()
                        } else {
                            format!(" {details}")
                        }
                    );
                    state.library_root = catalog.root.clone();
                    state.preferences.asset_library_root = catalog.root.clone();
                    state.library_catalog = Some(catalog);
                    state.library_selection.clear();
                    state.library_selected = None;
                    state.library_page = 0;
                    state.library_show_configuration = false;
                    refresh_library_matches(state);
                    state.page = Some(Page::AssetLibrary);
                    persist_preferences(state);
                }
                Ok(Completed::ImportedLibrary(catalog)) => {
                    let count = catalog.archives.len();
                    state.status = format!(
                        "{count} imported Asset Library PAKs indexed from saved manifests; originals unchanged."
                    );
                    state.library_root = catalog.root.clone();
                    state.preferences.asset_library_root = catalog.root.clone();
                    state.library_catalog = Some(catalog);
                    state.library_selection.clear();
                    state.library_selected = None;
                    state.library_page = 0;
                    state.library_menu = None;
                    refresh_library_matches(state);
                    state.page = Some(Page::AssetLibrary);
                    persist_preferences(state);
                }
                Ok(Completed::ImportedPakFiles(inventory)) => {
                    state.viewer_queued_preview = None;
                    state.viewer_page = 0;
                    state.status = format!(
                        "Read-only preview: {} manifest-listed files checked for size and location.",
                        inventory.entries.len()
                    );
                    clear_viewer_asset(state);
                    state.input = inventory.root.clone();
                    state.viewer_search.clear();
                    state.inventory = Some(inventory);
                    state.file_selection.clear();
                    state.pak = None;
                    state.signed_pak = None;
                    state.selected = None;
                    refresh_viewer_rows(state);
                    state.page = Some(Page::AssetLibrary);
                }
                Ok(Completed::Inventory(inventory)) => {
                    clear_viewer_asset(state);
                    state.viewer_page = 0;
                    state.file_selection.clear();
                    state.status = format!(
                        "{} files indexed; no originals modified.",
                        inventory.entries.len()
                    );
                    state.inventory = Some(inventory);
                    state.pak = None;
                    state.signed_pak = None;
                    state.texture = None;
                    state.model = None;
                    state.diffuse = None;
                    state.normal_map = None;
                    state.emissive_map = None;
                    state.specular_map = None;
                    state.gloss_map = None;
                    state.selected = None;
                    refresh_viewer_rows(state);
                    state.page = Some(Page::Files);
                }
                Ok(Completed::Pak(pak)) => {
                    state.status = format!(
                        "Validated ordinary ZIP PAK: {} members ({} bytes expanded).",
                        pak.entries.len(),
                        pak.total_bytes
                    );
                    state.pak = Some(pak);
                    state.file_selection.clear();
                    state.signed_pak = None;
                    state.selected = None;
                    refresh_viewer_rows(state);
                    state.page = Some(Page::Files);
                }
                Ok(Completed::BundleExported(result)) => {
                    state.status = format!(
                        "Exported {} FBX files, {} PNG textures and {} other dependencies into {}. {}",
                        result.exported.len(),
                        result.png_textures,
                        result.other_dependencies,
                        result.destination.display(),
                        result.warnings.join(" | ")
                    );
                }
                Ok(Completed::RiggedExport(info, target)) => {
                    state.status = format!(
                        "Rigged Blender-compatible export: {} original bones, {} weighted vertices, {} active joints, {} palettes: {}",
                        info.bone_count,
                        info.weighted_vertices,
                        info.active_bones,
                        info.subset_count,
                        target.display()
                    );
                    state.page = Some(Page::Models);
                }
                Ok(Completed::DbaConverted(result, dir, temp)) => {
                    state.cdf_caf_preview = None;
                    let count = result.clips.len();
                    state.converted_dba_cache = Some(temp);
                    state.converted_glbs = result
                        .glb_files
                        .iter()
                        .map(|relative| dir.join(relative))
                        .collect();
                    state.caf_layer = None;
                    state.caf_layer_label.clear();
                    state.native_caf = None;
                    state.glb_motion = None;
                    state.playing = false;
                    state.status = format!(
                        "Verified {} converter-backed DBA tracks from {}. Choose an output clip; no native DBA decoder or live mesh binding is implied.",
                        count, result.virtual_source
                    );
                    state.page = Some(Page::Models);
                    if state.converted_glbs.len() == 1 {
                        return launch(
                            state,
                            Operation::LoadGlbMotion(state.converted_glbs[0].clone()),
                        );
                    }
                }
                Ok(Completed::MotionConverted(result, dir)) => {
                    state.cdf_caf_preview = None;
                    state.status = format!(
                        "Converter produced {} validated GLB animation files, {} source clips from {}.",
                        result.glb_files.len(),
                        result.clips.len(),
                        result.virtual_source
                    );
                    state.converted_glbs =
                        result.glb_files.iter().map(|rel| dir.join(rel)).collect();
                    state.caf_layer = None;
                    state.caf_layer_label.clear();
                    state.native_caf = None;
                    state.skin_preview = None;
                    state.glb_motion = None;
                    state.playing = false;
                    state.page = Some(Page::Models);
                    if state.converted_glbs.len() == 1 {
                        return launch(
                            state,
                            Operation::LoadGlbMotion(state.converted_glbs[0].clone()),
                        );
                    }
                }
                Ok(Completed::MotionGlb(motion)) => {
                    state.cdf_caf_preview = None;
                    let clips = motion.list_clips();
                    state.status = format!(
                        "Loaded {} real glTF animation clip(s), {} source nodes.",
                        clips.len(),
                        motion.node_names.len()
                    );
                    state.motion_time = 0.0;
                    state.playing = false;
                    state.selected_motion_clip = 0;
                    state.caf_layer = None;
                    state.caf_layer_label.clear();
                    state.native_caf = None;
                    state.skin_preview = None;
                    state.glb_motion = Some(Arc::new(motion));
                    state.page = Some(Page::Models);
                }
                Ok(Completed::CafLayer(source, base, label, motion)) => {
                    if source != state.input
                        || !state
                            .native_caf
                            .as_ref()
                            .is_some_and(|current| Arc::ptr_eq(current, &base))
                    {
                        return Task::none();
                    }
                    let layers = [evolve_core::CafLayer {
                        motion: &motion,
                        seconds: state.motion_time,
                        weight: 0.5,
                    }];
                    let verified = if let Some(assembly) = &state.cdf_caf_preview {
                        assembly.mesh_at_layers(state.motion_time, &layers)
                    } else if let Some(skin) = &state.skin_preview {
                        skin.mesh_at_layers(state.motion_time, &layers)
                    } else {
                        return Task::none();
                    };
                    match verified {
                        Ok(mesh) => {
                            state.model = Some(Arc::new(mesh));
                            state.caf_layer = Some(Arc::new(motion));
                            state.caf_layer_label = label;
                            state.caf_layer_weight = 0.5;
                            state.status =
                                "Native CAF layer bound to the original skeleton.".into();
                        }
                        Err(error) => state.status = format!("Cannot bind CAF layer: {error}"),
                    }
                }
                Ok(Completed::Caf(motion)) => {
                    state.status = format!(
                        "Native CAF decoded: {} controllers at {:.1} FPS; {} seconds. Bind-pose matching is required for mesh animation.",
                        motion.controllers.len(),
                        motion.fps,
                        motion.duration_seconds()
                    );
                    state.motion_time = 0.0;
                    state.playing = false;
                    state.skin_preview = None;
                    state.cdf_caf_preview = None;
                    if let Some(rest) = &state.cdf_rest_mesh {
                        state.model = Some(rest.clone());
                    }
                    state.caf_layer = None;
                    state.caf_layer_label.clear();
                    state.native_caf = Some(Arc::new(motion));
                    state.glb_motion = None;
                    state.page = Some(Page::Models);
                    if let Some(task) = bind_current_native_caf(state) {
                        return task;
                    }
                }
                Ok(Completed::ModelResources(relative, resources)) => {
                    // A new click might have arrived while the index worker ran.
                    // Never attach another character's maps or CAF catalog.
                    let still_selected = state.viewer_queued_preview.is_none()
                        && state
                            .selected
                            .and_then(|i| state.inventory.as_ref()?.entries.get(i))
                            .is_some_and(|e| e.relative_path.eq_ignore_ascii_case(&relative));
                    if !still_selected {
                        state.auto_idle_pending = None;
                    } else {
                        state.auto_clips = resources.clips;
                        state.auto_idle_pending = resources.idle;
                        if let Some(skeleton) = resources.skeleton {
                            state.skeleton_path = skeleton.to_string_lossy().into_owned();
                        }
                        // CDF assemblies resolve a separate MTL per attachment.
                        // A missing standalone-model MTL is not a CDF material
                        // failure: do not overwrite valid 7/7 GPU material
                        // diagnostics with "No verified material for this mesh".
                        state.auto_resource_note = if relative
                            .to_ascii_lowercase()
                            .ends_with(".cdf")
                            && resources.material.is_none()
                        {
                            "CDF attachment materials are verified per mesh section in the Inspector"
                                    .into()
                        } else {
                            resources.material_note
                        };
                        if !resources.warnings.is_empty() {
                            state
                                .auto_resource_note
                                .push_str(&format!(" | {}", resources.warnings.join("; ")));
                        }
                        state.status = format!(
                            "Automatic model dependencies: {} declared clips. {}",
                            state.auto_clips.len(),
                            state.auto_resource_note
                        );
                        if let Some(source) = resources.material
                            && let Some(inv) = &state.inventory
                        {
                            let operation = Operation::ResolveMaterial(
                                source,
                                PathBuf::from(&inv.root),
                                state
                                    .library_catalog
                                    .as_ref()
                                    .map(|c| PathBuf::from(&c.root)),
                                resource_build(state),
                            );
                            state.auto_material_pending = true;
                            return launch(state, operation);
                        }
                        if let Some(idle) = state.auto_idle_pending.take() {
                            state.auto_play_native_caf = true;
                            return launch(state, Operation::LoadCaf(idle));
                        }
                    }
                }
                Ok(Completed::DeclaredClip(index, virtual_path, motion)) => {
                    let selected = state.viewer_queued_preview.is_none()
                        && state.auto_clips.get(index).is_some_and(|clip| {
                            clip.virtual_path.eq_ignore_ascii_case(&virtual_path)
                        });
                    if selected && !motion.additive {
                        state.skin_preview = None;
                        state.cdf_caf_preview = None;
                        if let Some(rest) = &state.cdf_rest_mesh {
                            state.model = Some(rest.clone());
                        }
                        state.caf_layer = None;
                        state.caf_layer_label.clear();
                        state.native_caf = Some(Arc::new(motion));
                        state.glb_motion = None;
                        state.motion_time = 0.0;
                        state.playing = false;
                        state.status = format!(
                            "Loaded exact character CAF: {virtual_path}; verifying original source bone skin..."
                        );
                        if let Some(task) = bind_current_native_caf(state) {
                            return task;
                        }
                    } else if selected {
                        state.status =
                            "Additive CAF cannot play alone without a compatible base pose.".into();
                    }
                }
                Ok(Completed::BoundCdfCafPreview(source, assembly)) => {
                    if state.viewer_queued_preview.is_none()
                        && Path::new(&state.input) == source
                        && state.native_caf.is_some()
                        && state.cdf_report.is_some()
                    {
                        let part_count = assembly.part_count();
                        let bones = assembly.original_bones();
                        let cyclic = assembly.source_motion().cyclic;
                        state.skin_preview = None;
                        state.cdf_caf_preview = Some(Arc::new(*assembly));
                        state.motion_time = 0.0;
                        apply_current_skin_frame(state);
                        if state.auto_play_native_caf {
                            state.playback_loop = true;
                        }
                        state.playing = cyclic || state.auto_play_native_caf;
                        state.auto_play_native_caf = false;
                        state.status = format!(
                            "Native CAF bound to {part_count} original CDF skin/equipment parts and {bones} source bones. Scrubbing deforms the assembled GPU viewport with verified CPU pose evaluation."
                        );
                        state.page = Some(Page::Models);
                    }
                }
                Ok(Completed::BoundCafPreview(source, skin)) => {
                    let expected = state
                        .render_source
                        .clone()
                        .unwrap_or_else(|| preferred_render_source(Path::new(&state.input)));
                    if state.viewer_queued_preview.is_none()
                        && expected == source
                        && state.native_caf.is_some()
                    {
                        let count = skin.original_vertices();
                        let bones = skin.original_bones();
                        let cyclic = skin.source_motion().cyclic;
                        state.cdf_caf_preview = None;
                        state.skin_preview = Some(Arc::new(*skin));
                        state.motion_time = 0.0;
                        apply_current_skin_frame(state);
                        if state.auto_play_native_caf {
                            state.playback_loop = true;
                        }
                        state.playing = cyclic || state.auto_play_native_caf;
                        state.auto_play_native_caf = false;
                        state.status = format!(
                            "Native CAF bound to {count} original weighted vertices and {bones} compiled bones. Mesh deforms in the GPU viewport (CPU skin evaluation)."
                        );
                        state.page = Some(Page::Models);
                    }
                }
                Ok(Completed::MaterialResolved(
                    material_source,
                    report,
                    diffuse,
                    normal,
                    emissive,
                    specular,
                    gloss,
                    diagnostics,
                )) => {
                    let automatic_model_material = state.auto_material_pending
                        && state.render_source.is_some()
                        && state.model.is_some()
                        && state.viewer_queued_preview.is_none();
                    state.auto_material_pending = false;
                    let found = report
                        .textures
                        .iter()
                        .filter(|t| matches!(t.resolution, AssetResolution::Found(_)))
                        .count();
                    let ambiguous = report
                        .textures
                        .iter()
                        .filter(|t| matches!(t.resolution, AssetResolution::Ambiguous(_)))
                        .count();
                    if state.cdf_report.is_some() {
                        // A separately validated CDF already has original
                        // per-subset GPU material bindings. The generic MTL
                        // resolver does NOT perform cross-slot assignment,
                        // so its informational diagnostic must not replace
                        // the CDF renderer's actual assignment status.
                        let (visible, mapped) =
                            state.cdf_surfaces.as_ref().map_or((0, 0), |parts| {
                                let visible = parts.iter().filter(|part| part.visible).count();
                                let mapped = parts
                                    .iter()
                                    .filter(|part| part.visible && part.diffuse.is_some())
                                    .count();
                                (visible, mapped)
                            });
                        state.status = format!(
                            "CDF GPU: {mapped}/{visible} visible mesh ranges with original diffuse maps; {found}/{} authored DDS references located, {ambiguous} ambiguous.",
                            report.textures.len()
                        );
                        let unresolved = diagnostics
                            .iter()
                            .filter(|note| {
                                !note.contains("require verified CryEngine mesh-subset assignments")
                            })
                            .cloned()
                            .collect::<Vec<_>>();
                        if !unresolved.is_empty() {
                            state.auto_resource_note =
                                format!("CDF material diagnostics: {}", unresolved.join(" | "));
                        }
                    } else {
                        state.status = format!(
                            "MTL: {} slot(s), {} authored references, {} uniquely resolved, {} ambiguous.{}",
                            report.slots,
                            report.textures.len(),
                            found,
                            ambiguous,
                            if diagnostics.is_empty() {
                                " No guessing.".to_string()
                            } else {
                                format!(" Unavailable maps: {}.", diagnostics.join(" | "))
                            }
                        );
                        if !diagnostics.is_empty() {
                            state.auto_resource_note =
                                format!("Material partially loaded: {}", diagnostics.join(" | "));
                        }
                    }
                    // A new authored material must not inherit a previous
                    // character's maps when the reference is missing/ambiguous.
                    state.diffuse_input = verified_export_diffuse(state, &report)
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    state.diffuse = diffuse.map(Arc::new);
                    state.normal_map = normal.map(Arc::new);
                    state.emissive_map = emissive.map(Arc::new);
                    state.specular_map = specular.map(Arc::new);
                    state.gloss_map = gloss.map(Arc::new);
                    state.material_report = Some(report);
                    state.page = Some(Page::Models);
                    if automatic_model_material
                        && let (Some(inventory), Some(model)) =
                            (state.inventory.clone(), state.render_source.clone())
                    {
                        return launch(
                            state,
                            Operation::LoadModelSurfaces(
                                model,
                                material_source,
                                inventory,
                                state
                                    .library_catalog
                                    .as_ref()
                                    .map(|c| PathBuf::from(&c.root)),
                                resource_build(state),
                                state.preview_cache.clone(),
                            ),
                        );
                    }
                    if let Some(idle) = state.auto_idle_pending.take() {
                        state.auto_play_native_caf = true;
                        return launch(state, Operation::LoadCaf(idle));
                    }
                }
                Ok(Completed::ModelSurfaces(source, surfaces, warnings)) => {
                    if state.viewer_queued_preview.is_none()
                        && state.render_source.as_ref() == Some(&source)
                        && state.model.is_some()
                    {
                        let textured = surfaces.iter().filter(|s| s.diffuse.is_some()).count();
                        state.cdf_surfaces = Some(Arc::new(surfaces));
                        state.auto_resource_note = format!(
                            "Original CryEngine GPU mesh subsets: {textured} textured; {} material warnings. {}",
                            warnings.len(),
                            warnings
                                .iter()
                                .take(3)
                                .cloned()
                                .collect::<Vec<_>>()
                                .join(" | "),
                        );
                        state.status = state.auto_resource_note.clone();
                        if let Some(idle) = state.auto_idle_pending.take() {
                            state.auto_play_native_caf = true;
                            return launch(state, Operation::LoadCaf(idle));
                        }
                    }
                }
                Ok(Completed::Normal(info, pixels)) => {
                    state.status = format!(
                        "GPU normal map loaded: {} × {} ({:?}) in linear color space. Tangents derive from original UV0.",
                        info.width, info.height, info.format
                    );
                    state.normal_map = Some(pixels);
                    state.page = Some(Page::Models);
                }
                Ok(Completed::Emissive(info, pixels)) => {
                    state.status = format!(
                        "GPU emissive map loaded: {} × {} ({:?}). MTL GlowAmount is not inferred.",
                        info.width, info.height, info.format
                    );
                    state.emissive_map = Some(pixels);
                    state.page = Some(Page::Models);
                }
                Ok(Completed::Diffuse(info, pixels)) => {
                    state.diffuse_status = format!(
                        "GPU diffuse loaded: {} × {} highest-resolution pixels ({:?}).",
                        info.width, info.height, info.format
                    );
                    state.status = state.diffuse_status.clone();
                    state.diffuse = Some(pixels);
                    state.page = Some(Page::Models);
                }
                Ok(Completed::DdsPreview(source, info, pixels)) => {
                    // Do not replace the latest model/texture with a stale
                    // thumbnail decoded before the user selected another file.
                    if state.viewer_queued_preview.is_none() && Path::new(&state.input) == source {
                        let handle = image::Handle::from_rgba(
                            pixels.width(),
                            pixels.height(),
                            pixels.as_raw().clone(),
                        );
                        state.texture_preview = Some(handle);
                        state.material_report = None;
                        state.model = None;
                        state.render_source = None;
                        state.cdf_surfaces = None;
                        state.cdf_report = None;
                        state.cdf_rest_mesh = None;
                        state.cdf_caf_preview = None;
                        state.skin_preview = None;
                        state.caf_layer = None;
                        state.caf_layer_label.clear();
                        state.native_caf = None;
                        state.glb_motion = None;
                        state.playing = false;
                        state.texture = Some(info.clone());
                        state.auto_resource_note = "Original highest-resolution DDS pixels".into();
                        state.status = format!(
                            "Previewing original DDS {} × {} ({:?}), {} streaming part(s).",
                            info.width, info.height, info.format, info.streaming_parts
                        );
                        state.page = Some(Page::Models);
                    }
                }
                Ok(Completed::Texture(texture)) => {
                    state.status = format!(
                        "DDS: {} × {} • {:?} • {} mipmaps • {} fragment(s)",
                        texture.width,
                        texture.height,
                        texture.format,
                        texture.mipmaps,
                        texture.streaming_parts
                    );
                    state.texture = Some(texture);
                    state.page = Some(Page::Models);
                }
                Ok(Completed::CharacterDependencies(report)) => {
                    let entries = std::iter::once(&report.skeleton)
                        .chain(report.attachments.iter().map(|(_, status)| status));
                    let mut found = 0;
                    let mut missing = 0;
                    let mut ambiguous = 0;
                    for result in entries {
                        match result {
                            AssetResolution::Found(_) => found += 1,
                            AssetResolution::Missing => missing += 1,
                            AssetResolution::Ambiguous(_) => ambiguous += 1,
                        }
                    }
                    state.status = format!(
                        "CDF dependencies: {found} exact resolved, {missing} missing, {ambiguous} ambiguous. No filename guessing."
                    );
                    state.cdf_dependencies = Some(report);
                    state.page = Some(Page::Models);
                }
                Ok(Completed::StaticCharacter(mesh, report, surfaces, material_warnings)) => {
                    state.cdf_legacy_2012 = historical_2012_cdf(&report);
                    if state.cdf_legacy_2012 {
                        state.full_armor_glow = false;
                        state.damage_blend_percent = 0;
                        state.damage_blend_enabled = false;
                    }
                    state.cdf_full_damage_surfaces = None;
                    state.cdf_damage_preparing = false;
                    crash_diagnostics::record(
                        "cdf_ui_complete",
                        &format!(
                            "{} sections, {} triangles",
                            surfaces.len(),
                            report.triangles
                        ),
                    );
                    state.render_source = None;
                    state.cdf_rest_mesh = Some(Arc::new(mesh.clone()));
                    state.cdf_report = Some(Arc::new(report.clone()));
                    state.cdf_caf_preview = None;
                    state.texture_preview = None;
                    state.material_report = None;
                    state.status = format!(
                        "CDF preview: {} attached parts, {} triangles; {} compiled bones verified, {} parts with original skin weights, {} material warnings.",
                        report.original_parts.len(),
                        report.triangles,
                        report.verified_bones.unwrap_or(0),
                        report
                            .source_parts
                            .iter()
                            .filter(|p| p.verified_skin_weights)
                            .count(),
                        material_warnings.len(),
                    );
                    state.auto_resource_note = report
                        .warnings
                        .iter()
                        .chain(&material_warnings)
                        .take(3)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" | ");
                    state.skeleton_path = report
                        .skeleton_source
                        .as_ref()
                        .map(|path| path.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let original = Arc::new(surfaces);
                    state.cdf_base_surfaces = Some(Arc::clone(&original));
                    state.cdf_surfaces = Some(original);
                    state.model = Some(Arc::new(mesh));
                    state.diffuse = None;
                    state.normal_map = None;
                    state.emissive_map = None;
                    state.specular_map = None;
                    state.gloss_map = None;
                    state.caf_layer = None;
                    state.caf_layer_label.clear();
                    state.native_caf = None;
                    state.skin_preview = None;
                    state.glb_motion = None;
                    state.playing = false;
                    state.viewer_tab = ViewerTab::Model;
                    state.zoom = 1.0;
                    state.pan_x = 0.0;
                    state.pan_y = 0.0;
                    state.page = Some(Page::Models);
                    // A CDF selection does not visit the regular single-model
                    // completion path. Discover its explicitly associated
                    // material and animation references here as well, so the
                    // assembly does not remain permanently untextured after
                    // its geometry has been assembled. The existing resolver
                    // refuses ambiguous, unsafe or unrelated resources.
                    if let Some(inv) = &state.inventory {
                        let root = PathBuf::from(&inv.root);
                        let source = PathBuf::from(&state.input);
                        if let Ok(relative) = source.strip_prefix(&root) {
                            let operation = Operation::DiscoverModelResources(
                                relative.to_string_lossy().replace('\\', "/"),
                                inv.clone(),
                                state
                                    .library_catalog
                                    .as_ref()
                                    .map(|c| PathBuf::from(&c.root)),
                                resource_build(state),
                            );
                            return launch(state, operation);
                        }
                    }
                }
                Ok(Completed::PreparedCdfDamage(source, full)) => {
                    state.cdf_damage_preparing = false;
                    if state.equipment_source.as_ref() == Some(&source)
                        && !state.cdf_legacy_2012
                        && state.cdf_base_surfaces.is_some()
                    {
                        state.cdf_full_damage_surfaces = Some(Arc::new(full));
                        return update(state, Message::ViewerDamageBlendCommit);
                    }
                }
                Ok(Completed::ReblendedCdfDamage(source, percent, surfaces)) => {
                    if state.equipment_source.as_ref() == Some(&source)
                        && !state.cdf_legacy_2012
                        && percent == state.damage_blend_percent
                    {
                        state.cdf_surfaces = Some(Arc::new(surfaces));
                        state.status = format!(
                            "Damage blend {percent}%: updated cached GPU textures; CDF geometry and DDS sources unchanged."
                        );
                    }
                }
                Ok(Completed::Character(character)) => {
                    state.status = format!(
                        "CDF: {} | {} validated skin/CHR attachment(s)",
                        character.skeleton,
                        character.attachments.len()
                    );
                    state.character = Some(character);
                    state.cdf_dependencies = None;
                    state.page = Some(Page::Models);
                }
                Ok(Completed::Model(mesh, render_source)) => {
                    state.render_source = Some(render_source);
                    state.cdf_report = None;
                    state.cdf_rest_mesh = None;
                    state.cdf_caf_preview = None;
                    state.texture_preview = None;
                    state.cdf_surfaces = None;
                    state.material_report = None;
                    state.skin_preview = None;
                    state.status = format!(
                        "Loaded {} source vertices and {} triangles (raw geometry only).",
                        mesh.info.vertex_count, mesh.info.triangle_count
                    );
                    state.model = Some(Arc::new(mesh));
                    state.converted_dba_cache = None;
                    state.converted_glbs.clear();
                    state.auto_clips.clear();
                    state.auto_clip_filter.clear();
                    state.export_include_animation = false;
                    state.auto_idle_pending = None;
                    state.auto_play_native_caf = false;
                    state.auto_material_pending = false;
                    state.skeleton_path.clear();
                    state.diffuse_input.clear();
                    state.auto_resource_note =
                        "Discovering verified local textures and character animation references..."
                            .into();
                    // An old character's textures and motion must never appear
                    // on a different mesh while materials are being resolved.
                    state.diffuse = None;
                    state.normal_map = None;
                    state.emissive_map = None;
                    state.specular_map = None;
                    state.gloss_map = None;
                    state.caf_layer = None;
                    state.caf_layer_label.clear();
                    state.native_caf = None;
                    state.glb_motion = None;
                    state.playing = false;
                    state.viewer_tab = ViewerTab::Model;
                    state.page = Some(Page::Models);
                    state.zoom = 1.0;
                    state.pan_x = 0.0;
                    state.pan_y = 0.0;
                    if let Some(inv) = &state.inventory {
                        let root = PathBuf::from(&inv.root);
                        let input = PathBuf::from(&state.input);
                        if let Ok(relative) = input.strip_prefix(&root) {
                            let operation = Operation::DiscoverModelResources(
                                relative.to_string_lossy().replace('\\', "/"),
                                inv.clone(),
                                state
                                    .library_catalog
                                    .as_ref()
                                    .map(|c| PathBuf::from(&c.root)),
                                resource_build(state),
                            );
                            return launch(state, operation);
                        }
                    }
                }
                Ok(Completed::Terrain(mesh, info)) => {
                    state.cdf_surfaces = None;
                    state.cdf_report = None;
                    state.cdf_rest_mesh = None;
                    state.cdf_caf_preview = None;
                    state.status = format!(
                        "Decoded source terrain {}×{} (stride {}).",
                        info.width, info.height, info.stride
                    );
                    state.terrain = Some(info);
                    state.model = Some(Arc::new(mesh));
                    state.skin_preview = None;
                    state.playing = false;
                    state.page = Some(Page::Levels);
                    state.zoom = 1.0;
                }
                Ok(Completed::Level(scene, source, hash)) => {
                    state.status = format!(
                        "Loaded {} placed level objects; {} diagnostics.",
                        scene.objects.len(),
                        scene.diagnostics.len()
                    );
                    state.level_source = Some(source);
                    state.level_source_sha = hash;
                    state.level_original_transforms = scene
                        .objects
                        .iter()
                        .map(|object| (object.id.clone(), object.local))
                        .collect();
                    state.level_dirty_count = 0;
                    state.level_scene = Some(Arc::new(scene));
                    state.level_selected = None;
                    state.level_selection.clear();
                    state.level_undo.clear();
                    state.level_redo.clear();
                    sync_level_fields(state);
                    state.level_zoom = 1.0;
                    state.page = Some(Page::Levels);
                }
                Ok(Completed::EditedLevelExported(destination, count)) => {
                    state.status = format!(
                        "Exported {count} edited placement(s) to {}. Original level unchanged.",
                        destination.display()
                    );
                    state.page = Some(Page::Levels);
                }
                Ok(Completed::ResignedSignedPak(report, destination)) => {
                    state.status = format!(
                        "Re-signed {} original CryPAK members to {} using the configured custom RSA key. Payloads and original PAK unchanged; hook preflight and offline runtime test still required.",
                        report.entries.len(),
                        destination.display(),
                    );
                    state.signed_pak = Some(report);
                    state.page = Some(Page::Files);
                }
                Ok(Completed::HookPrepared(report, destination)) => {
                    state.hook_prepared = destination.to_string_lossy().into_owned();
                    state.status = format!(
                        "Prepared verified hook copy at {}. Original SHA256 {} → staged SHA256 {}. Only the public-key region changed; installed DLL untouched.",
                        destination.display(),
                        report.source_sha256,
                        report.custom_sha256,
                    );
                    state.page = Some(Page::Files);
                }
                Ok(Completed::OfflineLauncherReady(report)) => {
                    state.status = format!(
                        "Offline launcher preflight PASS: {} original/custom signed PAKs and matching inject.dll verified. Private swap state: {}. This does not establish a successful Evolve mount.",
                        report.archive_count,
                        if report.prepared {
                            "prepared"
                        } else {
                            "needs preparation"
                        }
                    );
                    state.page = Some(Page::Files);
                }
                Ok(Completed::OfflineLauncherStarted(count, id)) => {
                    state.status = format!(
                        "Private launcher CreateProcessW hook READY (helper PID {id}) for {count} signed PAKs + matching custom RSA hook. The actual swap occurs only when the exact game launch is intercepted; watch rust-launch-bridge.log and auto-restore.log for swap/restoration. CryEngine mount is NOT yet verified."
                    );
                    state.page = Some(Page::Files);
                }
                Ok(Completed::OfflineLauncherRestored(count, reusable)) => {
                    state.status = format!(
                        "Private offline recovery PASS: originals verified for {count} signed PAKs + the original client hook. No stranded custom-key backups. {}",
                        if reusable {
                            "Verified custom files were recycled to prepared slots for the next offline launch."
                        } else {
                            "Custom files need fresh preparation before the next offline launch."
                        }
                    );
                    state.page = Some(Page::Files);
                }
                Ok(Completed::SignedHookPreflight(report)) => {
                    state.status = format!(
                        "PAK/hook preflight PASS: {} signed original members, {} custom-signed members, exact matching RSA slot. Actual Evolve process mount UNVERIFIED; do not assume online compatibility.",
                        report.signed_original_entries, report.signed_replacement_entries,
                    );
                    state.page = Some(Page::Files);
                }
                Ok(Completed::SignedPak(info)) => {
                    state.status = format!(
                        "Verified signed CryPAK: {} records, {} expanded bytes.",
                        info.entries.len(),
                        info.total_expanded_bytes
                    );
                    state.signed_pak = Some(info);
                    state.pak = None;
                    state.inventory = None;
                    state.selected = None;
                    state.file_selection.clear();
                    refresh_viewer_rows(state);
                }
                Ok(Completed::RebuiltSigned(name, dir)) => {
                    state.status = format!(
                        "Rebuilt and verified signed PAK with {} into {}. Source unchanged; offline test required.",
                        name,
                        dir.display()
                    );
                }
                Ok(Completed::ProfileStatus(tx)) => {
                    state.profile_feedback = if let Some(tx) = tx {
                        format!(
                            "Recovery required: profile {} has {} staged PAKs in journal {}.",
                            tx.profile_id,
                            tx.entries.len(),
                            tx.id
                        )
                    } else {
                        "No multi-PAK recovery transaction pending.".into()
                    };
                    state.status = state.profile_feedback.clone();
                    state.page = Some(Page::Files);
                }
                Ok(Completed::ProfileApplied(tx)) => {
                    state.profile_feedback = format!(
                        "Applied {} signed mod PAKs to ISOLATED offline stage only. Transaction {} requires recovery before another profile.",
                        tx.entries.len(),
                        tx.id
                    );
                    state.status = state.profile_feedback.clone();
                    state.page = Some(Page::Files);
                }
                Ok(Completed::ProfileRecovered(tx)) => {
                    state.profile_feedback = format!(
                        "Restored {} original PAKs from verified offline batch backups.",
                        tx.entries.len()
                    );
                    state.status = state.profile_feedback.clone();
                    state.page = Some(Page::Files);
                }
                Ok(Completed::ProfileCatalog(catalog)) => {
                    let count = catalog.profiles.len();
                    state.profile_catalog = Some(catalog);
                    state.profile_feedback =
                        format!("Loaded {count} isolated offline mod profiles.");
                    state.status = state.profile_feedback.clone();
                    state.page = Some(Page::Files);
                }
                Ok(Completed::ProfileCreated(profile)) => {
                    state.profile_id = profile.id.clone();
                    state.profile_feedback = format!(
                        "Created {} with ID {}. Refresh to list it.",
                        profile.name, profile.id
                    );
                    state.status = state.profile_feedback.clone();
                    state.page = Some(Page::Files);
                    return launch(
                        state,
                        Operation::ListProfiles(PathBuf::from(&state.preferences.stage)),
                    );
                }
                Ok(Completed::ProfileAdded(item)) => {
                    state.profile_feedback = format!(
                        "Registered verified signed mod {} for {}. Profile is not applied.",
                        item.id, item.target
                    );
                    state.status = state.profile_feedback.clone();
                    state.page = Some(Page::Files);
                    return launch(
                        state,
                        Operation::ListProfiles(PathBuf::from(&state.preferences.stage)),
                    );
                }
                Ok(Completed::ProfilePlanned(plan)) => {
                    state.profile_feedback = format!(
                        "Verified {} enabled signed PAKs ({} bytes). This is a read-only plan; no game files changed.",
                        plan.mods.len(),
                        plan.total_bytes
                    );
                    state.status = state.profile_feedback.clone();
                    state.page = Some(Page::Files);
                }
                Ok(Completed::StageInitialized(path)) => {
                    state.stage_feedback =
                        format!("Initialized isolated PAK workspace at {}", path.display());
                    state.status = state.stage_feedback.clone();
                    state.page = Some(Page::Files);
                }
                Ok(Completed::StageStatus(transaction)) => {
                    state.stage_feedback = if let Some(transaction) = transaction {
                        format!(
                            "Swap journal pending: {}. Restore before applying another mod.",
                            transaction.target
                        )
                    } else {
                        "No active stage swap journal. The workspace is ready.".into()
                    };
                    state.status = state.stage_feedback.clone();
                    state.page = Some(Page::Files);
                }
                Ok(Completed::StageImported(hash)) => {
                    state.stage_feedback =
                        format!("Signed archive imported to isolated stage; SHA-256: {hash}");
                    state.status = state.stage_feedback.clone();
                    state.page = Some(Page::Files);
                }
                Ok(Completed::StageApplied(transaction)) => {
                    state.stage_feedback = format!(
                        "Signed replacement staged for {}. Recovery journal {} must be restored before another change.",
                        transaction.target, transaction.id,
                    );
                    state.status = state.stage_feedback.clone();
                    state.page = Some(Page::Files);
                }
                Ok(Completed::StageRestored(transaction)) => {
                    state.stage_feedback = format!(
                        "Restored original PAK in isolated workspace: {}",
                        transaction.target
                    );
                    state.status = state.stage_feedback.clone();
                    state.page = Some(Page::Files);
                }
                Ok(Completed::LevelFbxExported(meshes, placements, dest)) => {
                    state.status = format!(
                        "Exported Blender FBX scene: {meshes} mesh resources, {placements} placements: {} (materials not yet complete).",
                        dest.display()
                    );
                    state.page = Some(Page::Levels);
                }
                Ok(Completed::LevelExported(meshes, placements, dest)) => {
                    state.status = format!(
                        "Exported Blender scene GLB: {meshes} verified models, {placements} placements: {}",
                        dest.display()
                    );
                    state.page = Some(Page::Levels);
                }
                Ok(Completed::ExtractedSigned(count, dir)) => {
                    state.status = format!(
                        "Extracted {} verified signed records into {}",
                        count,
                        dir.display()
                    );
                }
                Ok(Completed::Extracted(count, folder)) => {
                    state.status = format!(
                        "Extracted {count} validated ZIP-compatible PAK entries to {}",
                        folder.display()
                    );
                }
                Ok(Completed::Rebuilt(count, target)) => {
                    state.status = format!(
                        "Rebuilt unsigned ZIP-compatible PAK from {count} replacement(s): {}",
                        target.display()
                    );
                }
                Ok(Completed::ExportedPng(info, target)) => {
                    state.status = format!(
                        "Exported {} × {} highest-resolution DDS mip to {}",
                        info.width,
                        info.height,
                        target.display()
                    );
                }
                Err(e) => {
                    if state.cdf_damage_preparing {
                        state.cdf_damage_preparing = false;
                        state.damage_blend_percent = 0;
                        state.damage_blend_enabled = false;
                        if let Some(base) = &state.cdf_base_surfaces {
                            state.cdf_surfaces = Some(Arc::clone(base));
                        }
                    }
                    crash_diagnostics::record("worker_refused", &e);
                    state.auto_play_native_caf = false;
                    state.status = format!("Operation refused: {e}");
                    if state.auto_material_pending && state.viewer_queued_preview.is_none() {
                        state.auto_material_pending = false;
                        if let Some(idle) = state.auto_idle_pending.take() {
                            state.auto_play_native_caf = true;
                            return launch(state, Operation::LoadCaf(idle));
                        }
                    }
                }
            }
            // A single worker services both windows; preserve the last click
            // rather than requiring users to click a second time after loading.
            if let Some(index) = state.viewer_queued_preview.take()
                && state.selected == Some(index)
                && let Some(entry) = state.inventory.as_ref().and_then(|i| i.entries.get(index))
            {
                let is_cdf = entry.relative_path.to_ascii_lowercase().ends_with(".cdf");
                return update(
                    state,
                    if is_cdf {
                        Message::ViewerPreviewStaticCdf
                    } else {
                        Message::ViewerPreviewSelected
                    },
                );
            }
            // After an equipment swap has rebuilt the geometry, restore the
            // selected damage percentage from the NEW verified base materials,
            // without repeating that geometry work for slider changes.
            if state.damage_blend_percent > 0
                && !state.cdf_legacy_2012
                && !state.cdf_damage_preparing
                && state.cdf_full_damage_surfaces.is_none()
                && state.cdf_base_surfaces.is_some()
                && !state.busy
            {
                return update(state, Message::ViewerDamageBlendCommit);
            }
        }
    }
    Task::none()
}

fn download_action(state: &State) -> Element<'_, Message> {
    if let Some(update) = &state.available_update {
        let button = design::button(text(format!("Update to {} and restart", update.version)));
        if state.busy {
            button.into()
        } else {
            button.on_press(Message::ApplyUpdate).into()
        }
    } else {
        text("Choose Stable, Beta or Nightly and check for a newer verified release.").into()
    }
}

fn view(state: &State) -> Element<'_, Message> {
    let page = state.page.unwrap_or(Page::Home);
    // Preserve the private manager's compact rail and exact navigation order.
    // Hover reveals one tooltip, never the whole expanded sidebar.
    let navigation = Page::ALL
        .into_iter()
        .fold(column![].spacing(7), |items, item| {
            let active = page == item;
            let nav = design::button(svg(item.icon()).style(design::icon).width(23).height(23))
                .width(51)
                .height(43)
                .padding(10)
                .style(move |theme, status| design::rail(theme, status, active))
                .on_press(Message::Navigate(item));
            items.push(tooltip(
                nav,
                text(item.title()).size(11),
                tooltip::Position::Right,
            ))
        });
    let play_mods_icon = svg(svg::Handle::from_memory(
        include_bytes!("../../../assets/nav/play-mods.svg").to_vec(),
    ))
    .width(20)
    .height(20);
    let play_clean_icon = svg(svg::Handle::from_memory(
        include_bytes!("../../../assets/nav/play-clean.svg").to_vec(),
    ))
    .width(20)
    .height(20);
    let lower = column![
        tooltip(
            design::button(play_mods_icon).width(46).height(40),
            text("Play with Mods").size(11),
            tooltip::Position::Right
        ),
        tooltip(
            design::button(play_clean_icon).width(46).height(40),
            text("Play without Mods").size(11),
            tooltip::Position::Right
        ),
        tooltip(
            design::button(svg(Page::Settings.icon()).width(20).height(20))
                .width(46)
                .height(43)
                .padding(10)
                .style(if page == Page::Settings {
                    design::secondary
                } else {
                    design::text_button
                })
                .on_press(Message::Navigate(Page::Settings)),
            text("Settings").size(11),
            tooltip::Position::Right
        ),
    ]
    .spacing(5);
    let sidebar = container(
        column![navigation, space().height(Fill), lower,]
            .spacing(8)
            .height(Fill)
            .padding([12, 7]),
    )
    .width(66)
    .height(Fill)
    .style(|theme| container::Style {
        background: Some(iced::Background::Color(design::sidebar(theme))),
        border: iced::Border {
            color: design::border(theme),
            width: 1.0,
            radius: 12.0.into(),
        },
        ..container::Style::default()
    });
    let mut headline = row![
        design::button(
            row![
                image(image::Handle::from_bytes(
                    include_bytes!("../../../assets/hunt.png").to_vec()
                ))
                .width(34)
                .height(34),
                text("EVOLVE  /  MOD MANAGER").size(18),
            ]
            .spacing(10)
            .align_y(iced::Center)
        )
        .style(design::text_button)
        .on_press(Message::Navigate(Page::Home)),
        text(format!("  /  {}", page.title())).size(12),
        space().width(Fill),
        design::button("Help")
            .on_press(Message::Navigate(Page::Instructions))
            .style(design::secondary),
        design::button("Activity")
            .on_press(Message::ToggleActivity)
            .style(design::secondary),
    ]
    .spacing(13)
    .align_y(iced::Center);
    if state.scan_cancel.is_some() {
        headline = headline.push(
            design::button("Cancel Scan")
                .on_press(Message::CancelScan)
                .style(design::secondary),
        );
    }

    let location: iced::widget::Column<'_, Message> = column![
        row![
            design::text_input("Local asset folder, .pak or .dds path...", &state.input)
                .on_input(Message::InputChanged)
                .width(Fill),
            design::button("Scan folder")
                .on_press(Message::StartScan)
                .style(design::secondary),
            design::button("Open Asset Viewer")
                .on_press(Message::OpenViewer)
                .style(design::secondary),
        ]
        .spacing(8),
        row![
            design::button("Inspect ZIP PAK").on_press(Message::StartPakInspection),
            design::button("Inspect DDS").on_press(Message::StartDdsInspection),
            design::button("Inspect CDF").on_press(Message::StartCdfInspection),
            design::button("Inspect 3D model").on_press(Message::StartModelInspection),
        ]
        .spacing(8),
        row![
            design::text_input("New output directory or .png filename...", &state.output)
                .on_input(Message::OutputChanged)
                .width(Fill),
            design::button("Extract PAK").on_press(Message::StartExtract),
            design::button("Export DDS → PNG").on_press(Message::StartPngExport),
        ]
        .spacing(8),
        row![
            design::text_input(
                "Folder containing exact original PAK member replacements...",
                &state.replacements
            )
            .on_input(Message::ReplacementsChanged)
            .width(Fill),
            design::button("Rebuild unsigned PAK").on_press(Message::StartRebuild),
        ]
        .spacing(8),
        row![
            design::text_input(
                "Exact existing CryPAK member path (e.g. Libs/UI/file.xml)",
                &state.signed_entry
            )
            .on_input(Message::SignedMemberChanged)
            .width(Fill),
            design::text_input("Replacement local file...", &state.signed_replacement)
                .on_input(Message::SignedReplacementChanged)
                .width(Fill),
            design::button("Rebuild + re-sign member")
                .style(design::secondary)
                .on_press(Message::StartSignedRebuild),
        ]
        .spacing(8),
        row![
            text(
                "Stage RSA keys: original RSAKeyData.bin + custom mykeys/ (automatically checked)."
            )
            .size(12),
            design::button("Verify signed PAK")
                .style(design::secondary)
                .on_press(Message::StartSignedInspection),
            design::button("Extract signed PAK")
                .style(design::secondary)
                .on_press(Message::StartSignedExtraction),
            design::button("Re-sign entire PAK")
                .style(design::secondary)
                .on_press(Message::StartSignedResign),
        ]
        .spacing(8),
        row![
            design::text_input(
                "Original client bin64_SteamRetail/inject.dll",
                &state.hook_original
            )
            .on_input(Message::HookOriginalChanged)
            .width(Fill),
            design::text_input("NEW custom hook directory/inject.dll", &state.hook_prepared)
                .on_input(Message::HookPreparedChanged)
                .width(Fill),
        ]
        .spacing(8),
        row![
            design::button("Prepare verified client hook copy")
                .style(design::secondary)
                .on_press(Message::StartHookPrepare),
            design::button("Verify PAK + hook for offline swap")
                .style(design::secondary)
                .on_press(Message::StartHookPreflight),
            text("Hook source and inject-custom.dll are detected from the configured private signing stage; no game files change during verification.").size(12),
        ]
        .spacing(8),
    ]
    .spacing(9);

    let workspace: Element<'_, Message> = match page {
        Page::Home => private_pages::home(state),
        Page::Instructions => private_pages::instructions(),
        Page::WhatsNew => private_pages::whats_new(),
        Page::Credits => private_pages::credits(),
        Page::Developer => private_pages::developer(state),
        Page::AssetLibrary => asset_library_page::view(state),
        Page::Files => modding_page::view(state),
        Page::Models => {
            let description = if let Some(texture) = &state.texture {
                format!(
                    "DDS: {} × {} | {:?} | {} mip(s) | {} part(s) | top mip {} bytes",
                    texture.width, texture.height, texture.format,
                    texture.mipmaps, texture.streaming_parts, texture.top_mip_bytes
                )
            } else {
                "Select a local DDS and choose Inspect DDS.".to_owned()
            };
            column![
                text("Texture inspection and original top-mip export").size(19),
                text(description),
                text("PNG export preserves BC5 channels without pretending to reconstruct normal-map shading."),
                text(if let Some(character) = &state.character {
                    format!("CDF skeleton: {} | {} real skin/CHR attachment(s).",
                        character.skeleton, character.attachments.len())
                } else {
                    "CDF: select a local character definition and choose Inspect CDF.".to_owned()
                }),
                text("GPU viewer uses source UV0 and explicitly selected DDS diffuse, normal and emissive. Automatic MTL matching and skeletal skinning remain gated."),
                row![
                    design::text_input("Extracted Asset Library root for CDF dependency resolution...", &state.assets_root)
                        .on_input(Message::AssetsRootChanged).width(Fill),
                    design::button("Resolve CDF parts").on_press(Message::StartCdfResolve),
                ].spacing(8),
                if let Some(dependencies) = &state.cdf_dependencies {
                    let mut items: iced::widget::Column<'_, Message> = column![
                        text(format!("Skeleton: {:?}", dependencies.skeleton))
                    ].spacing(4);
                    for (binding, status) in dependencies.attachments.iter().take(100) {
                        items = items.push(text(format!("{binding} → {status:?}")));
                    }
                    scrollable(items).height(Length::Fixed(155.)).into()
                } else {
                    Element::<Message>::from(text("Read and resolve a CDF to inspect exact attachments; ambiguous matches are never selected automatically."))
                },
                row![
                    design::text_input("Exact DDS diffuse (.dds or .dds.0) path...",&state.diffuse_input)
                        .on_input(Message::DiffuseInputChanged).width(Fill),
                    design::button("Load GPU diffuse").on_press(Message::StartDiffuseLoad),
                ].spacing(8),
                text(state.diffuse_status.as_str()),
                row![
                    design::text_input("Exact source CryEngine .mtl path...", &state.material_input)
                        .on_input(Message::MaterialInputChanged).width(Fill),
                    design::button("Resolve material DDS maps").on_press(Message::StartMaterialResolve),
                ].spacing(8),
                if let Some(report) = &state.material_report {
                    let mut rows = column![].spacing(3);
                    for texture in report.textures.iter().take(80) {
                        rows = rows.push(text(format!("{} / {} / {}: {:?}",
                            texture.slot, texture.role, texture.authored_path, texture.resolution)));
                    }
                    scrollable(rows).height(Length::Fixed(125.))
                } else {
                    scrollable(column![text("Load an authored .mtl to inspect provenance and GPU DDS maps.")])
                        .height(Length::Fixed(40.))
                },
                row![
                    design::text_input("Source normal map DDS (BC5)...", &state.normal_input)
                        .on_input(Message::NormalInputChanged).width(Fill),
                    design::button("Load GPU normal").on_press(Message::StartNormalLoad),
                    design::text_input("Source emissive or glow DDS...", &state.emissive_input)
                        .on_input(Message::EmissiveInputChanged).width(Fill),
                    design::button("Load GPU emissive").on_press(Message::StartEmissiveLoad),
                ].spacing(8),
                if let Some(mesh)=&state.model {
                    let display:Element<'_,Message> = if mesh.uv0.is_some(){
                        shader(TexturedScene{
                            mesh:mesh.clone(),surfaces:state.cdf_surfaces.clone(),diffuse:state.diffuse.clone(),
                            normal_map:state.normal_map.clone(),emissive:state.emissive_map.clone(),
                            specular:state.specular_map.clone(),gloss:state.gloss_map.clone(),
                            yaw:state.yaw,pitch:state.pitch,zoom:state.zoom,
                            light_strength:state.light_strength,
                            emissive_gain:state.emissive_strength,
                            emissive_time:state.decal_time,
                            pan_x:state.pan_x,pan_y:state.pan_y,shift:state.keyboard_modifiers.shift(),
                        }).width(Fill).height(Length::Fixed(400.)).into()
                    }else{
                        canvas(MeshViewport{
                            mesh:mesh.clone(),yaw:state.yaw,pitch:state.pitch,zoom:state.zoom,
                            pan_x:state.pan_x,pan_y:state.pan_y,shift:state.keyboard_modifiers.shift(),
                        }).width(Fill).height(Length::Fixed(400.)).into()
                    };
                    display
                } else {
                    text("Select a source .cgf/.chr/.skin file and click Inspect 3D model.").into()
                },
                text("Rigged character export — verified original weights").size(18),
                row![
                    design::text_input("Exact original .chr skeleton path...",&state.skeleton_path)
                        .on_input(Message::SkeletonChanged).width(Fill),
                    if blender_install::valid_blender(&state.blender_path).is_some() {
                        Element::from(text("Blender detected automatically for FBX").size(12))
                    } else {
                        Element::from(design::text_input("Find Blender.exe for FBX…", &state.blender_path)
                            .on_input(Message::BlenderChanged).width(Fill))
                    },
                ].spacing(8),
                row![
                    design::button("Export rigged GLB").on_press(Message::StartRigGlb),
                    design::button("Export rigged FBX").on_press(Message::StartRigFbx),
                    text("FBX optionally embeds selected DDS diffuse and exact native CAF tracks when bone CRCs match; DBA layering remains unfinished."),
                ].spacing(9),
                text("Animation controls — exact native CAF tracks").size(18),
                row![
                    design::text_input("Select original CAF file...",&state.caf_path)
                        .on_input(Message::CafInputChanged).width(Fill),
                    design::button("Open CAF").on_press(Message::StartCafLoad),
                    design::button(if state.playing{"Pause"}else{"Play"}).on_press(Message::ToggleMotion),
                ].spacing(8),
                row![
                    design::text_input("Bundled or independent Cryengine-Converter.exe...",
                        &state.converter_path).on_input(Message::ConverterPathChanged).width(Fill),
                    design::button("Convert & preview CAF/DBA").on_press(Message::StartMotionConversion),
                ].spacing(8),
                design::text_input("Exact extracted asset root (for converter dependencies)...",
                    &state.assets_root).on_input(Message::AssetsRootChanged),
                if !state.converted_glbs.is_empty() {
                    let mut items=column![text("Verified converter GLB outputs (select exact file)")].spacing(4);
                    for (index,file) in state.converted_glbs.iter().enumerate().take(32) {
                        items=items.push(design::button(text(file.file_name().unwrap_or_default().to_string_lossy().to_string()))
                            .on_press(Message::OpenConvertedGlb(index)));
                    }
                    scrollable(items).height(Length::Fixed(160.)).into()
                }else{
                    let placeholder:Element<'_,Message>=text("Packed DBA clips use independently installed Cryengine-Converter (GPLv2).").into();
                    placeholder
                },
                if let Some(loaded)=&state.glb_motion {
                    let clips=loaded.list_clips();
                    let mut chooser=column![text(format!("Verified glTF animation clips: {}",clips.len()))].spacing(4);
                    for (i,clip) in clips.iter().enumerate().take(64) {
                        chooser=chooser.push(design::button(text(format!("{} • {:.2}s • {} channels",clip.name,clip.duration,clip.channels)))
                            .on_press(Message::SelectMotionClip(i)));
                    }
                    let chosen=clips.get(state.selected_motion_clip);
                    let current=chosen.map(|clip|{
                        let tracks=loaded.sample(state.selected_motion_clip,state.motion_time)
                            .unwrap_or_default();
                        format!("Selected {} • {:.2}/{:.2}s • {} decoded node keyframes",
                            clip.name,state.motion_time,clip.duration,tracks.len())
                    }).unwrap_or_else(||"Choose a compatible clip.".into());
                    column![
                        scrollable(chooser).height(Length::Fixed(180.)),
                        text(current),
                        slider(0.0..=chosen.map_or(0.001,|clip|clip.duration.max(0.001)),
                            state.motion_time,Message::MotionScrub),
                        text("Converter-backed keys are real. Bone deformation requires matching skin hierarchy."),
                    ].spacing(6).into()
                }else{
                    let placeholder:Element<'_,Message>=text("Native CAF / converter-backed DBA clips appear here.").into();
                    placeholder
                },
                if let Some(clip)=&state.native_caf {
                    let frame=clip.sample_seconds(state.motion_time).unwrap_or_default();
                    let value=frame.iter().take(3).map(|bone|format!(
                        "{:08X}: {:?} ",bone.bone_crc32,bone.translation
                    )).collect::<Vec<_>>().join(" • ");
                    column![
                        text(format!("Motion {:.2}/{:.2}s • {} bone controller(s) • additive={} cyclic={}",
                            state.motion_time,clip.duration_seconds(),clip.controllers.len(),clip.additive,clip.cyclic)),
                        slider(0.0..=clip.duration_seconds().max(0.001),state.motion_time,Message::MotionScrub),
                        text(value),
                        text(if state.skin_preview.is_some() {
                            "Verified native CAF transforms deform the original weighted mesh in the textured viewport."
                        } else {
                            "Native CAF keys decoded; exact original skeleton binding is required before mesh playback."
                        }),
                    ].spacing(7).into()
                }else{
                    let placeholder:Element<'_,Message>=text("Choose original CAF to inspect playable controller data. No skeleton guessed.").into();
                    placeholder
                },
                row![
                    text("Orbit"),
                    slider(-180.0..=180.0,state.yaw,Message::YawChanged).width(Fill),
                    text("Pitch"),
                    slider(-85.0..=85.0,state.pitch,Message::PitchChanged).width(Fill),
                    text("Zoom"),
                    slider(0.2..=4.0,state.zoom.max(0.2),Message::ZoomChanged).width(Fill),
                ].spacing(10),
            ].spacing(14).into()
        }
        Page::Levels => column![
            text("Level source preview").size(19),
            level_workspace::view(state),
            text("Read Level/LYR or Heightmap.dat without touching installed game assets."),
            row![
                design::button("Read level placements").on_press(Message::StartLevelInspection),
                design::button("Load heightmap geometry").on_press(Message::StartTerrainInspection),
                design::button("Export geometry scene GLB").on_press(Message::StartLevelGlbExport),
                design::button("Export geometry scene FBX").on_press(Message::StartLevelFbxExport),
            ].spacing(8),
            row![
                design::text_input("Exact extracted asset root...",&state.assets_root)
                    .on_input(Message::AssetsRootChanged).width(Fill),
                design::text_input("Optional Heightmap.dat source...",&state.terrain_source)
                    .on_input(Message::TerrainSourceChanged).width(Fill),
            ].spacing(8),
            text(if let Some(scene)=&state.level_scene {
                format!("{} level objects, {} unresolved diagnostics; source transforms preserved.",
                    scene.objects.len(),scene.diagnostics.len())
            }else{"No placement source inspected.".into()}),
            if let Some(mesh)=&state.model {
                let display:Element<'_,Message> = if mesh.uv0.is_some(){
                    shader(TexturedScene{
                        mesh:mesh.clone(),surfaces:None,diffuse:state.diffuse.clone(),
                            normal_map:state.normal_map.clone(),emissive:state.emissive_map.clone(),
                            specular:state.specular_map.clone(),gloss:state.gloss_map.clone(),
                        yaw:state.yaw,pitch:state.pitch,zoom:state.zoom.max(0.2),
                            light_strength:state.light_strength,
                            emissive_gain:state.emissive_strength,
                            emissive_time:state.decal_time,
                            pan_x:state.pan_x,pan_y:state.pan_y,shift:state.keyboard_modifiers.shift(),
                    }).width(Fill).height(Length::Fixed(400.)).into()
                }else{
                    canvas(MeshViewport{
                        mesh:mesh.clone(),yaw:state.yaw,pitch:state.pitch,zoom:state.zoom.max(0.2),
                            pan_x:state.pan_x,pan_y:state.pan_y,shift:state.keyboard_modifiers.shift(),
                    }).width(Fill).height(Length::Fixed(400.)).into()
                };
                display
            }else{
                let fallback:Element<'_,Message>=text("Load an original source heightmap to see terrain triangles.").into();
                fallback
            },
            row![
                text("Yaw"),
                slider(-180.0..=180.0,state.yaw,Message::YawChanged).width(Fill),
                text("Pitch"),
                slider(-85.0..=85.0,state.pitch,Message::PitchChanged).width(Fill),
            ].spacing(8),
            if blender_install::valid_blender(&state.blender_path).is_some() {
                Element::from(text("Blender detected automatically for scene FBX").size(12))
            } else {
                Element::from(design::text_input("Find Blender.exe for scene FBX…", &state.blender_path)
                    .on_input(Message::BlenderChanged))
            },
            text("GLB/FBX includes verified mesh instances and optional terrain. Texture/material, compiled vegetation, scripting and full map fidelity remain gated."),
        ].spacing(14).into(),
        Page::Settings => settings_page::view(state),
    };

    let workspace: Element<'_, Message> = if page == Page::Files {
        scrollable(workspace).height(Fill).into()
    } else {
        workspace
    };
    let show_tools = matches!(page, Page::Models | Page::Levels);
    let advanced: Element<'_, Message> = if state.show_advanced_tools {
        container(location).into()
    } else {
        container(text("")).into()
    };
    let tools: Element<'_, Message> = if show_tools {
        column![
            row![
                design::text_input("Selected asset / folder / PAK path...", &state.input)
                    .on_input(Message::InputChanged)
                    .width(Fill),
                design::button("Scan Folder").on_press(Message::StartScan),
                design::button("Open Asset Viewer")
                    .on_press(Message::OpenViewer)
                    .style(design::secondary),
                design::button(if state.show_advanced_tools {
                    "Hide Archive Tools"
                } else {
                    "Show Advanced Archive Tools"
                })
                .on_press(Message::ToggleAdvancedTools),
            ]
            .spacing(8),
            advanced,
        ]
        .spacing(7)
        .into()
    } else {
        container(column![]).into()
    };
    let body = column![
        headline,
        tools,
        workspace,
        row![
            text(if state.busy {
                "Working..."
            } else {
                state.status.as_str()
            })
            .size(12)
            .width(Fill),
            text(format!("v{}  •  Rust preview", env!("CARGO_PKG_VERSION"))).size(12),
        ]
        .spacing(12),
    ]
    .spacing(11)
    .padding([10, 28])
    .width(Fill)
    .height(Fill);
    // Activity is detached so it never steals space from the working page.
    container(row![sidebar, body].height(Fill))
        .width(Fill)
        .height(Fill)
        .into()
}

fn initial_state() -> State {
    let mut result = State {
        zoom: 1.0,
        playback_speed: 1.0,
        export_rigged: true,
        export_embed_textures: true,
        viewer_split: 0.67,
        level_zoom: 1.0,
        light_strength: 1.6,
        emissive_strength: 1.0,
        viewer_explorer_visible: true,
        ..State::default()
    };
    if let Ok(path) = settings_path() {
        match load_from(&path) {
            Ok(saved) => {
                result.blender_path = saved.blender_executable.clone();
                result.assets_root = saved.asset_library_root.clone();
                result.viewer_split = saved.viewer_split;
                result.viewer_explorer_visible = saved.viewer_explorer_visible;
                result.viewer_show_inspector = saved.viewer_show_inspector;
                result.viewer_wireframe = saved.viewer_wireframe;
                result.light_strength = saved.light_strength;
                result.playback_speed = saved.animation_speed;
                result.playback_loop = saved.animation_loop;
                result.yaw = saved.saved_yaw;
                result.pitch = saved.saved_pitch;
                result.zoom = saved.saved_zoom;
                result.preferences = saved;
            }
            Err(error) => {
                result.settings_status = format!(
                    "Could not read preferences; original file was not overwritten: {error}"
                );
            }
        }
    }
    if let Ok(file) = settings_path() {
        if let Some(root) = file.parent()
            && let Some(error) = updater_apply::previous_update_failure(root)
        {
            result.update_worker_failure = Some(error.clone());
            result.update_status = format!(
                "Previous update failed: {error}. Details: {}",
                root.join("Updates").join("last-update-error.txt").display()
            );
        }
        if let Ok(location) =
            settings::library_directory(&file, &result.preferences.asset_library_root)
        {
            result.preferences.asset_library_root = location.to_string_lossy().into_owned();
        }
        if let Ok(location) =
            settings::game_storage_directory(&file, &result.preferences.game_storage_root)
        {
            result.preferences.game_storage_root = location.to_string_lossy().into_owned();
        }
        if result.preferences.projects.trim().is_empty()
            && let Some(parent) = file.parent()
        {
            result.preferences.projects = parent
                .join("Data")
                .join("Projects")
                .to_string_lossy()
                .into_owned();
        }
    }
    // Automatic Blender discovery is intentionally ephemeral: do not save a
    // machine-specific detected path into user preferences.
    if let Ok(path) = blender_install::resolve_blender(&result.blender_path) {
        result.blender_path = path.to_string_lossy().into_owned();
    }
    result.library_root = result.preferences.asset_library_root.clone();
    result.assets_root = result.library_root.clone();
    if let Ok(executable) = std::env::current_exe()
        && let Some(parent) = executable.parent()
    {
        let bundled = parent
            .join("third_party")
            .join("Cryengine-Converter")
            .join("cgf-converter.exe");
        if bundled.is_file() {
            result.converter_path = bundled.to_string_lossy().to_string();
        }
    }
    result
}
/// One native main window, model/asset editor, and separate export settings window. All costly
/// asset operations stay in the existing worker queue, independent of either
/// window's render loop.
struct Desktop {
    state: State,
    main_id: window::Id,
    viewer_id: Option<window::Id>,
    inspector_id: Option<window::Id>,
    preferences_id: Option<window::Id>,
    activity_id: Option<window::Id>,
    export_id: Option<window::Id>,
}

impl Desktop {
    fn new() -> (Self, Task<Message>) {
        let (main_id, opened) = window::open(window::Settings {
            size: iced::Size::new(1440.0, 900.0),
            min_size: Some(iced::Size::new(880.0, 620.0)),
            ..window::Settings::default()
        });
        let mut state = initial_state();
        let library_root = PathBuf::from(&state.library_root);
        state.library_startup_loading = library_root.is_dir();
        // Independent read-only indexing: don't block Modding, updates,
        // previews or Settings with the global worker busy flag.
        let startup = if library_root.is_dir() {
            Task::perform(
                async move {
                    let expected = library_root.clone();
                    let result =
                        tokio::task::spawn_blocking(move || imported_library(&library_root))
                            .await
                            .map_err(|e| format!("Asset Library worker stopped: {e}"))
                            .and_then(|outcome| outcome.map_err(|e| e.to_string()));
                    (expected, result)
                },
                |(expected, result)| Message::StartupLibraryIndexed(expected, result),
            )
        } else {
            Task::none()
        };
        (
            Self {
                state,
                main_id,
                viewer_id: None,
                inspector_id: None,
                preferences_id: None,
                activity_id: None,
                export_id: None,
            },
            Task::batch([opened.map(|_| Message::ViewerOpened), startup]),
        )
    }

    fn open_viewer(&mut self) -> Task<Message> {
        if self.viewer_id.is_some() {
            return Task::none();
        }
        self.state.viewer_from_library = self.state.page == Some(Page::AssetLibrary);
        let (id, opened) = window::open(window::Settings {
            size: iced::Size::new(1550.0, 940.0),
            ..window::Settings::default()
        });
        self.viewer_id = Some(id);
        opened.map(|_| Message::ViewerOpened)
    }

    fn open_viewer_preferences(&mut self) -> Task<Message> {
        if self.preferences_id.is_some() {
            return Task::none();
        }
        let (id, opened) = window::open(window::Settings {
            size: iced::Size::new(700.0, 530.0),
            ..window::Settings::default()
        });
        self.preferences_id = Some(id);
        opened.map(|_| Message::InspectorOpened)
    }

    fn open_export(&mut self) -> Task<Message> {
        if self.export_id.is_some() {
            return Task::none();
        }
        let (id, opened) = window::open(window::Settings {
            size: iced::Size::new(960.0, 740.0),
            min_size: Some(iced::Size::new(720.0, 520.0)),
            ..window::Settings::default()
        });
        self.export_id = Some(id);
        opened.map(|_| Message::ViewerOpened)
    }

    fn open_activity(&mut self) -> Task<Message> {
        if self.activity_id.is_some() {
            return Task::none();
        }
        let (id, opened) = window::open(window::Settings {
            size: iced::Size::new(740.0, 560.0),
            ..window::Settings::default()
        });
        self.activity_id = Some(id);
        opened.map(|_| Message::ActivityOpened)
    }

    fn open_inspector(&mut self) -> Task<Message> {
        if self.inspector_id.is_some() {
            self.state.viewer_show_inspector = true;
            return Task::none();
        }
        self.state.viewer_show_inspector = true;
        let (id, opened) = window::open(window::Settings {
            size: iced::Size::new(950.0, 610.0),
            ..window::Settings::default()
        });
        self.inspector_id = Some(id);
        opened.map(|_| Message::InspectorOpened)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::MenuCommand(inner) => {
                self.state.viewer_menu = None;
                self.state.modding_menu = None;
                self.state.library_menu = None;
                self.update(*inner)
            }
            Message::OpenViewer => self.open_viewer(),
            Message::ExportOpen => {
                let prepare = update(&mut self.state, Message::ExportOpen);
                Task::batch([prepare, self.open_export()])
            }
            Message::ExportClose => {
                self.state.export_dialog_open = false;
                self.export_id.take().map_or_else(Task::none, window::close)
            }
            Message::ExportBegin => {
                let command = update(&mut self.state, Message::ExportBegin);
                if !self.state.export_dialog_open
                    && let Some(id) = self.export_id.take()
                {
                    return Task::batch([command, window::close(id)]);
                }
                command
            }
            Message::LibraryExport => {
                // Never offer an old Modding inventory as if it were the
                // currently selected imported Asset Library content.
                let within_library = self.state.inventory.as_ref().is_some_and(|inventory| {
                    let catalog_root = std::fs::canonicalize(&self.state.library_root);
                    let indexed_root = std::fs::canonicalize(&inventory.root);
                    matches!((catalog_root, indexed_root), (Ok(lib), Ok(index)) if index.starts_with(&lib))
                });
                if !within_library {
                    self.state.status =
                        "Preview an imported Asset Library PAK to index its original files before exporting.".into();
                    return Task::none();
                }
                self.update(Message::ExportOpen)
            }
            Message::OpenViewerPreferences => self.open_viewer_preferences(),
            Message::OpenInspectorWindow => self.open_inspector(),
            Message::ToggleActivity => self.open_activity(),
            Message::ActivityOpened => Task::none(),
            Message::InspectorOpened => Task::none(),
            Message::ViewerOpened => Task::none(),
            Message::FileDropped(id, path) => {
                if (self.viewer_id == Some(id) || self.inspector_id == Some(id))
                    && self.state.viewer_from_library
                {
                    self.state.status = "Asset Library originals are read-only. Drop files in Modding for inspection.".into();
                    Task::none()
                } else {
                    update(&mut self.state, Message::DropPath(path))
                }
            }
            Message::WindowClosed(id) => {
                if id == self.main_id {
                    iced::exit()
                } else {
                    if self.export_id == Some(id) {
                        self.export_id = None;
                        self.state.export_dialog_open = false;
                    }
                    if self.viewer_id == Some(id) {
                        self.viewer_id = None;
                        self.state.viewer_menu = None;
                        clear_viewer_asset(&mut self.state);
                        // The model editor is a separate window; closing it
                        // must return to the originating workspace rather
                        // than exposing the old inline Models tools tab.
                        if self.state.page == Some(Page::Models) {
                            self.state.page = Some(if self.state.viewer_from_library {
                                Page::AssetLibrary
                            } else {
                                Page::Files
                            });
                        }
                    }
                    if self.inspector_id == Some(id) {
                        self.inspector_id = None;
                    }
                    if self.preferences_id == Some(id) {
                        self.preferences_id = None;
                    }
                    if self.activity_id == Some(id) {
                        self.activity_id = None;
                    }
                    Task::none()
                }
            }
            message => {
                let project_scan_done = self.state.modding_open_editor_when_scanned
                    && matches!(&message, Message::Finished(Ok(Completed::Inventory(_))));
                let finished = matches!(&message, Message::Finished(_));
                let opens_model = matches!(
                    &message,
                    Message::Finished(Ok(Completed::Model(_, _)
                        | Completed::StaticCharacter(_, _, _, _)
                        | Completed::ImportedPakFiles(_)))
                ) || project_scan_done;
                let launched_from_library = self.state.page == Some(Page::AssetLibrary);
                let before = self.state.status.clone();
                let was_busy = self.state.busy;
                let command = update(&mut self.state, message);
                if self.state.status != before || self.state.busy != was_busy {
                    let note = if self.state.busy { "RUNNING" } else { "STATUS" };
                    let item = format!("{note}: {}", self.state.status);
                    if self.state.activity_log.last() != Some(&item) {
                        self.state.activity_log.push(item);
                        if self.state.activity_log.len() > 250 {
                            self.state.activity_log.remove(0);
                        }
                    }
                }
                if finished {
                    self.state.modding_open_editor_when_scanned = false;
                }
                let automatic_activity = !was_busy
                    && self.state.busy
                    && self.state.preferences.show_activity_on_tasks
                    && self.activity_id.is_none();
                let activity = if automatic_activity {
                    self.open_activity()
                } else {
                    Task::none()
                };
                if opens_model && self.viewer_id.is_none() {
                    let open = self.open_viewer();
                    self.state.viewer_from_library = launched_from_library;
                    Task::batch([command, open, activity])
                } else {
                    Task::batch([command, activity])
                }
            }
        }
    }

    fn view(&self, id: window::Id) -> Element<'_, Message> {
        if self.export_id == Some(id) {
            asset_viewer::export_dialog(&self.state)
        } else if self.activity_id == Some(id) {
            activity_window::view(&self.state)
        } else if self.viewer_id == Some(id) {
            asset_viewer::view(&self.state)
        } else if self.preferences_id == Some(id) {
            asset_viewer::preferences_view(&self.state)
        } else if self.inspector_id == Some(id) {
            container(viewer_inspector::view(&self.state))
                .padding(12)
                .width(Fill)
                .height(Fill)
                .into()
        } else if id == self.main_id {
            view(&self.state)
        } else {
            container(text("Window closed")).into()
        }
    }

    fn title(&self, id: window::Id) -> String {
        if self.export_id == Some(id) {
            "Export Settings | Evolve Mod Manager".into()
        } else if self.activity_id == Some(id) {
            "Activity | Evolve Mod Manager".into()
        } else if self.preferences_id == Some(id) {
            "Evolve Mod Manager — Viewer Preferences".into()
        } else if self.inspector_id == Some(id) {
            "Evolve Mod Manager — Detached Asset Inspector".into()
        } else if self.viewer_id == Some(id) {
            let source = std::path::Path::new(&self.state.input)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Asset Viewer".to_owned());
            if self.state.viewer_from_library {
                format!("Asset Library — {source} — Read Only")
            } else {
                format!("Modding — {source} — Asset Viewer")
            }
        } else {
            "Evolve Mod Manager".into()
        }
    }

    fn theme(&self, _id: window::Id) -> Option<Theme> {
        Some(design::theme(&self.state.preferences))
    }

    fn subscription(&self) -> Subscription<Message> {
        let animate_decal = self.state.cdf_surfaces.as_ref().is_some_and(|surfaces| {
            surfaces
                .iter()
                .any(|surface| surface.visible && surface.emissive_uv.is_some())
        }) && matches!(self.state.page.as_ref(), Some(Page::Models));
        let playback = if self.state.playing || animate_decal {
            iced::time::every(Duration::from_millis(40)).map(|_| Message::Tick)
        } else {
            Subscription::none()
        };
        let scan_progress = if self.state.scan_indexed.is_some() || self.state.busy {
            iced::time::every(Duration::from_millis(250)).map(|_| Message::ScanPulse)
        } else {
            Subscription::none()
        };
        let startup = if self.state.preferences.automatic_update_checks
            && !self.state.startup_update_checked
        {
            iced::time::every(Duration::from_millis(500)).map(|_| Message::CheckUpdates)
        } else {
            Subscription::none()
        };
        Subscription::batch([
            playback,
            startup,
            scan_progress,
            window::close_events().map(Message::WindowClosed),
            iced::event::listen_with(|event, _status, id| match event {
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                    ..
                }) => Some(Message::DismissMenus),
                iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(modifiers)) => {
                    Some(Message::ModifiersChanged(modifiers))
                }
                iced::Event::Window(window::Event::FileDropped(path)) => {
                    Some(Message::FileDropped(id, path))
                }
                _ => None,
            }),
        ])
    }
}

fn main() -> iced::Result {
    if updater_apply::try_run_helper() {
        return Ok(());
    }
    let logging_enabled = settings_path()
        .ok()
        .and_then(|path| load_from(&path).ok())
        .is_none_or(|settings| settings.crash_logging_enabled);
    crash_diagnostics::install(logging_enabled);
    let _supported_backends = wgpu::Backends::PRIMARY;
    iced::daemon(Desktop::new, Desktop::update, Desktop::view)
        .subscription(Desktop::subscription)
        .title(Desktop::title)
        .theme(Desktop::theme)
        .run()
}

#[cfg(test)]
mod project_edit_tests {
    use super::*;

    #[test]
    fn open_library_folder_validates_configured_directory() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(
            verified_library_directory(temp.path().to_str().unwrap()).unwrap(),
            temp.path()
        );
        assert!(verified_library_directory("").is_err());
        assert!(verified_library_directory("relative/folder").is_err());
        let file = temp.path().join("not-a-folder");
        std::fs::write(&file, b"test").unwrap();
        assert!(verified_library_directory(file.to_str().unwrap()).is_err());
        let missing = temp.path().join("missing");
        assert!(verified_library_directory(missing.to_str().unwrap()).is_err());
    }

    #[test]
    fn layer_controls_ignore_invalid_weights_and_unbound_assets() {
        let mut state = State {
            caf_layer_weight: 0.4,
            ..State::default()
        };
        for weight in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
            let _ = update(&mut state, Message::ViewerLayerWeight(weight));
            assert_eq!(state.caf_layer_weight, 0.4);
        }
        let _ = update(&mut state, Message::ViewerLayerClip(0));
        assert!(!state.busy);
        assert!(state.caf_layer.is_none());
    }

    #[test]
    fn late_layer_completion_cannot_replace_a_new_base_clip() {
        let motion = NativeCafMotion {
            schema: 1,
            controllers: vec![],
            start_frame: 0.0,
            end_frame: 30.0,
            fps: 30.0,
            additive: false,
            cyclic: false,
        };
        let old = Arc::new(motion.clone());
        let current = Arc::new(motion.clone());
        let mut state = State {
            input: "new.cdf".into(),
            native_caf: Some(current.clone()),
            caf_layer: Some(current.clone()),
            caf_layer_label: "keep".into(),
            ..State::default()
        };
        let _ = update(
            &mut state,
            Message::Finished(Ok(Completed::CafLayer(
                "new.cdf".into(),
                old,
                "stale".into(),
                motion,
            ))),
        );
        assert_eq!(state.caf_layer_label, "keep");
        assert!(Arc::ptr_eq(state.caf_layer.as_ref().unwrap(), &current));
    }

    #[test]
    fn plain_click_previews_model_without_second_button_press() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("sample.skin"), b"invalid model").unwrap();
        let inventory = scan_directory(dir.path()).unwrap();
        let mut state = State {
            inventory: Some(inventory),
            ..State::default()
        };
        refresh_viewer_rows(&mut state);
        let _task = update(&mut state, Message::ViewerSelectFile(0));
        assert_eq!(state.selected, Some(0));
        assert!(state.busy, "a normal click must queue a preview");
    }

    #[test]
    fn selecting_another_asset_while_busy_loads_latest_without_second_click() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..2 {
            std::fs::write(dir.path().join(format!("sample_{i}.skin")), b"invalid").unwrap();
        }
        let mut state = State {
            inventory: Some(scan_directory(dir.path()).unwrap()),
            ..State::default()
        };
        refresh_viewer_rows(&mut state);
        let _ = update(&mut state, Message::ViewerSelectFile(0));
        assert!(state.busy);
        let _ = update(&mut state, Message::ViewerSelectFile(1));
        assert_eq!(state.viewer_queued_preview, Some(1));
        assert_eq!(state.selected, Some(1));
        let _ = update(
            &mut state,
            Message::Finished(Err("previous preview failed".into())),
        );
        assert!(
            state.busy,
            "queued preview must start immediately after worker completion"
        );
        assert_eq!(state.viewer_queued_preview, None);
        assert_eq!(state.selected, Some(1));
    }

    #[test]
    fn docked_tools_can_open_switch_and_close_without_closing_files() {
        let mut state = State::default();
        let _ = update(
            &mut state,
            Message::ViewerSidebarSelect(ViewerSidebarTab::Animations),
        );
        let _ = update(
            &mut state,
            Message::ViewerSidebarSelect(ViewerSidebarTab::Inspector),
        );
        let _ = update(
            &mut state,
            Message::ViewerSidebarSelect(ViewerSidebarTab::Dependencies),
        );
        assert!(state.viewer_sidebar_animations);
        assert!(state.viewer_show_inspector);
        assert!(state.viewer_sidebar_dependencies);
        assert_eq!(state.viewer_sidebar_active, ViewerSidebarTab::Dependencies);
        let _ = update(
            &mut state,
            Message::ViewerSidebarClose(ViewerSidebarTab::Inspector),
        );
        assert!(!state.viewer_show_inspector);
        assert!(state.viewer_sidebar_dependencies);
        assert_eq!(state.viewer_sidebar_active, ViewerSidebarTab::Dependencies);
        let _ = update(
            &mut state,
            Message::ViewerSidebarClose(ViewerSidebarTab::Dependencies),
        );
        assert_eq!(state.viewer_sidebar_active, ViewerSidebarTab::Files);
        let _ = update(
            &mut state,
            Message::ViewerSidebarClose(ViewerSidebarTab::Files),
        );
        assert_eq!(state.viewer_sidebar_active, ViewerSidebarTab::Files);
        let _ = update(
            &mut state,
            Message::ViewerSidebarSelect(ViewerSidebarTab::Animations),
        );
        assert!(state.viewer_sidebar_animations);
        let _ = update(
            &mut state,
            Message::ViewerSidebarClose(ViewerSidebarTab::Animations),
        );
        assert!(!state.viewer_sidebar_animations);
        assert_eq!(state.viewer_sidebar_active, ViewerSidebarTab::Files);
    }

    #[test]
    fn damage_slider_zero_and_full_reuse_loaded_materials_without_cdf_worker() {
        let path = PathBuf::from("characters/monsters/goliath/goliath3.cdf");
        let pixels = Arc::new(::image::RgbaImage::from_pixel(
            1,
            1,
            ::image::Rgba([20, 50, 80, 255]),
        ));
        let full_pixels = Arc::new(::image::RgbaImage::from_pixel(
            1,
            1,
            ::image::Rgba([230, 170, 80, 255]),
        ));
        let original = textured::Surface {
            first_index: 0,
            index_count: 3,
            visible: true,
            transparent: false,
            diffuse: Some(Arc::clone(&pixels)),
            normal: None,
            emissive: None,
            armor_emissive: None,
            emission_strength: 1.0,
            kala_emission_state: None,
            emissive_uv: None,
            specular: None,
            gloss: None,
            opacity: None,
        };
        let mut damaged = original.clone();
        damaged.diffuse = Some(Arc::clone(&full_pixels));
        let mut state = State {
            equipment_source: Some(path),
            damage_blend_enabled: true,
            damage_blend_percent: 100,
            cdf_base_surfaces: Some(Arc::new(vec![original])),
            cdf_full_damage_surfaces: Some(Arc::new(vec![damaged])),
            ..State::default()
        };
        let _task = update(&mut state, Message::ViewerDamageBlendCommit);
        assert!(!state.busy, "100% must not spawn a CDF worker");
        assert!(Arc::ptr_eq(
            state.cdf_surfaces.as_ref().unwrap()[0]
                .diffuse
                .as_ref()
                .unwrap(),
            &full_pixels,
        ));
        let _ = update(&mut state, Message::ViewerDamageBlendChanged(0));
        let _ = update(&mut state, Message::ViewerDamageBlendCommit);
        assert!(!state.busy, "0% must not spawn a CDF worker");
        assert!(state.damage_blend_enabled, "slider must stay visible at 0%");
        assert!(Arc::ptr_eq(
            state.cdf_surfaces.as_ref().unwrap()[0]
                .diffuse
                .as_ref()
                .unwrap(),
            &pixels,
        ));
    }

    #[test]
    fn emissive_slider_preserves_original_source_and_requires_a_real_map() {
        let pixels = Arc::new(::image::RgbaImage::from_pixel(
            1,
            1,
            ::image::Rgba([180, 40, 20, 255]),
        ));
        let surface = textured::Surface {
            first_index: 0,
            index_count: 3,
            visible: true,
            transparent: false,
            diffuse: None,
            normal: None,
            emissive: Some(Arc::clone(&pixels)),
            armor_emissive: None,
            emission_strength: 3.0,
            kala_emission_state: None,
            emissive_uv: None,
            specular: None,
            gloss: None,
            opacity: None,
        };
        let model = Arc::new(RenderMesh {
            info: evolve_core::ModelInfo {
                format: "test".into(),
                version: 0,
                chunk_count: 0,
                vertex_count: 3,
                triangle_count: 1,
                original_uv0: false,
                original_normals: false,
            },
            positions: vec![[0.0, 0.0, 0.0]; 3],
            indices: vec![0, 1, 2],
            uv0: None,
            normals: None,
        });
        let mut state = State {
            model: Some(model),
            cdf_surfaces: Some(Arc::new(vec![surface.clone()])),
            emissive_strength: 1.0,
            ..State::default()
        };
        let before = state.cdf_surfaces.clone().unwrap();
        let _ = update(&mut state, Message::ViewerEmissiveStrengthChanged(2.5));
        assert_eq!(state.emissive_strength, 2.5);
        assert!(Arc::ptr_eq(&before, state.cdf_surfaces.as_ref().unwrap()));
        assert_eq!(
            state.cdf_surfaces.as_ref().unwrap()[0].emission_strength,
            3.0
        );
        assert_eq!(
            state.cdf_surfaces.as_ref().unwrap()[0]
                .emissive
                .as_ref()
                .unwrap()
                .as_raw(),
            pixels.as_raw()
        );
        state.cdf_surfaces = None;
        let _ = update(&mut state, Message::ViewerEmissiveStrengthChanged(4.0));
        assert_eq!(state.emissive_strength, 2.5);
    }

    #[test]
    fn legacy_2012_header_removes_stage_two_damage_and_glow_actions() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("old.chr");
        let mut original = [0u8; 16];
        original[..8].copy_from_slice(b"CryTek\0\0");
        original[12..16].copy_from_slice(&0x745u32.to_le_bytes());
        std::fs::write(&model, original).unwrap();
        let report = AssemblyReport {
            cdf_skeleton: "old.chr".into(),
            selected_skin_set: None,
            original_parts: vec!["body".into()],
            vertices: 3,
            triangles: 1,
            original_uv0_complete: true,
            original_normals_complete: true,
            skeletal_animation_enabled: false,
            source_parts: vec![],
            geometry_parts: vec![evolve_core::AssemblyGeometry {
                name: "body".into(),
                source: model.clone(),
                vertex_first: 0,
                vertex_count: 3,
                bone_socket: None,
                verified_skin_weights: false,
            }],
            verified_bones: None,
            skeleton_source: None,
            warnings: vec![],
        };
        assert!(historical_2012_cdf(&report));
        let mut newer = original;
        newer[12..16].copy_from_slice(&0x746u32.to_le_bytes());
        std::fs::write(&model, newer).unwrap();
        assert!(!historical_2012_cdf(&report));
        let mut state = State {
            equipment_source: Some(dir.path().join("characters/monsters/rocky/rocky3.cdf")),
            cdf_legacy_2012: true,
            ..State::default()
        };
        let _ = update(&mut state, Message::ViewerToggleDamageBlend);
        let _ = update(&mut state, Message::ViewerToggleMonsterGlow);
        assert!(!state.full_armor_glow);
        assert!(!state.damage_blend_enabled);
        assert_eq!(state.damage_blend_percent, 0);
    }

    #[test]
    fn equipment_tab_rejects_unverified_cross_character_swaps() {
        let mut state = State::default();
        let _ = update(
            &mut state,
            Message::ViewerSidebarSelect(ViewerSidebarTab::Equipment),
        );
        assert_eq!(state.viewer_sidebar_active, ViewerSidebarTab::Equipment);
        let _ = update(
            &mut state,
            Message::ViewerEquip(
                "basher_l".into(),
                "objects/characters/monsters/scorpid_2/scorpid3_r_shield.chr".into(),
            ),
        );
        assert!(state.equipment_overrides.is_empty());
        assert!(state.status.contains("not verified"));
        let _ = update(
            &mut state,
            Message::ViewerSidebarClose(ViewerSidebarTab::Equipment),
        );
        assert_eq!(state.viewer_sidebar_active, ViewerSidebarTab::Files);
    }

    #[test]
    fn opening_dependency_inspection_docks_tab_even_without_a_source() {
        let mut state = State::default();
        let _ = update(&mut state, Message::StartDependencyInspection);
        assert!(state.viewer_sidebar_dependencies);
        assert_eq!(state.viewer_sidebar_active, ViewerSidebarTab::Dependencies);
        assert!(state.status.contains("Index extracted assets"));
    }

    #[test]
    fn viewer_sidebar_tabs_use_one_shared_explorer_location() {
        let mut state = State::default();
        let _ = update(&mut state, Message::ViewerSidebarAnimations(true));
        assert!(state.viewer_sidebar_animations);
        assert!(state.viewer_explorer_visible);
        let _ = update(&mut state, Message::ViewerSidebarAnimations(false));
        assert_eq!(state.viewer_sidebar_active, ViewerSidebarTab::Files);
        assert!(state.viewer_sidebar_animations);
        let _ = update(
            &mut state,
            Message::ViewerSidebarClose(ViewerSidebarTab::Animations),
        );
        assert!(!state.viewer_sidebar_animations);
    }

    #[test]
    fn dragging_explorer_grip_bounds_split_without_sliders() {
        let mut state = State {
            viewer_split: 0.67,
            ..State::default()
        };
        let _ = update(&mut state, Message::ViewerSplitDrag(0.10));
        assert!((state.viewer_split - 0.77).abs() < 0.0001);
        let _ = update(&mut state, Message::ViewerSplitDrag(-2.0));
        assert_eq!(state.viewer_split, 0.25);
    }

    #[test]
    fn switching_source_folder_clears_all_previous_character_rendering() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = State {
            inventory: Some(scan_directory(dir.path()).unwrap()),
            model: Some(Arc::new(RenderMesh {
                info: evolve_core::ModelInfo {
                    format: "test".into(),
                    version: 7,
                    chunk_count: 1,
                    vertex_count: 3,
                    triangle_count: 1,
                    original_uv0: false,
                    original_normals: false,
                },
                positions: vec![[0.0, 0.0, 0.0]; 3],
                indices: vec![0, 1, 2],
                uv0: None,
                normals: None,
            })),
            auto_resource_note: "Previous character".into(),
            viewer_queued_preview: Some(0),
            ..State::default()
        };
        let _ = begin_scan(&mut state, dir.path().to_path_buf());
        assert!(state.model.is_none());
        assert!(state.cdf_surfaces.is_none());
        assert!(state.texture_preview.is_none());
        assert!(state.auto_resource_note.is_empty());
        assert!(state.viewer_queued_preview.is_none());
    }

    #[test]
    fn rescanning_discards_queued_preview_from_the_old_directory() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        std::fs::write(first.path().join("old.skin"), b"invalid").unwrap();
        std::fs::write(second.path().join("new.dds"), b"invalid").unwrap();
        let mut state = State {
            inventory: Some(scan_directory(first.path()).unwrap()),
            busy: true,
            ..State::default()
        };
        refresh_viewer_rows(&mut state);
        let _ = update(&mut state, Message::ViewerSelectFile(0));
        assert_eq!(state.viewer_queued_preview, Some(0));
        let newer = scan_directory(second.path()).unwrap();
        let _ = update(
            &mut state,
            Message::Finished(Ok(Completed::Inventory(newer))),
        );
        assert!(!state.busy);
        assert_eq!(state.viewer_queued_preview, None);
        assert_eq!(state.selected, None);
        assert!(state.file_selection.indices.is_empty());
    }

    #[test]
    fn shift_click_selects_range_without_launching_an_asset_preview() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..3 {
            std::fs::write(dir.path().join(format!("sample_{i}.skin")), b"invalid").unwrap();
        }
        let mut state = State {
            inventory: Some(scan_directory(dir.path()).unwrap()),
            ..State::default()
        };
        refresh_viewer_rows(&mut state);
        state.keyboard_modifiers = iced::keyboard::Modifiers::SHIFT;
        let _task = update(&mut state, Message::ViewerSelectFile(0));
        assert!(!state.busy);
        let _task = update(&mut state, Message::ViewerSelectFile(2));
        assert!(!state.busy);
        assert_eq!(state.file_selection.indices.len(), 3);
    }

    #[test]
    fn explicit_cdf_preview_uses_full_assembly_worker() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("character.cdf");
        std::fs::write(&source, b"<CharacterDefinition/>").unwrap();
        let inventory = scan_directory(dir.path()).unwrap();
        let mut state = State {
            inventory: Some(inventory),
            selected: Some(0),
            ..State::default()
        };
        let _task = update(&mut state, Message::ViewerPreviewSelected);
        assert!(state.busy, "CDF preview should start a background assembly");
        assert_eq!(
            std::fs::canonicalize(&state.input).unwrap(),
            std::fs::canonicalize(&source).unwrap()
        );
        // A metadata-only CDF inspection used to leave the previously
        // displayed character visible. The full-assembly route clears the
        // previous DDS preview before starting its worker.
        assert!(state.texture_preview.is_none());
    }

    #[test]
    fn compiled_skin_prefers_exact_regular_render_companion() {
        let dir = tempfile::tempdir().unwrap();
        let skin = dir.path().join("tech_3p_lod1.skin");
        let skinm = dir.path().join("tech_3p_lod1.skinm");
        std::fs::write(&skin, b"skeleton without render positions").unwrap();
        assert_eq!(preferred_render_source(&skin), skin);
        std::fs::write(&skinm, b"original geometry stream").unwrap();
        assert_eq!(preferred_render_source(&skin), skinm);
        let other = dir.path().join("unrelated.skinm");
        std::fs::write(other, b"unrelated").unwrap();
        std::fs::remove_file(&skinm).unwrap();
        assert_eq!(preferred_render_source(&skin), skin);
    }

    #[test]
    fn right_click_opens_file_actions_without_starting_a_preview() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("sample.skin"), b"invalid model").unwrap();
        let mut state = State {
            inventory: Some(scan_directory(dir.path()).unwrap()),
            ..State::default()
        };
        refresh_viewer_rows(&mut state);
        let _ = update(&mut state, Message::ViewerContextFile(0));
        assert!(!state.busy);
        assert_eq!(state.selected, Some(0));
        assert_eq!(state.viewer_context_index, Some(0));
        assert!(state.input.ends_with("sample.skin"));
        let _ = update(&mut state, Message::ViewerContextClose);
        assert_eq!(state.viewer_context_index, None);
    }

    #[test]
    fn right_click_never_accepts_unindexed_or_removed_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.skin");
        std::fs::write(&path, b"invalid").unwrap();
        let mut state = State {
            inventory: Some(scan_directory(dir.path()).unwrap()),
            ..State::default()
        };
        refresh_viewer_rows(&mut state);
        let _ = update(&mut state, Message::ViewerContextFile(1234));
        assert_eq!(state.viewer_context_index, None);
        std::fs::remove_file(path).unwrap();
        let _ = update(&mut state, Message::ViewerContextFile(0));
        assert_eq!(state.viewer_context_index, None);
        assert!(!state.busy);
    }

    #[test]
    fn level_transform_changes_are_undoable_without_mutating_original_scene() {
        let xml = br#"<Objects>
            <Object Id="parent" Pos="1,2,3"/>
            <Object Id="child" Parent="parent" Pos="4,5,6"/>
        </Objects>"#;
        let scene = evolve_core::parse_level_scene(&[("test.lyr", xml)]).unwrap();
        let initial = scene.clone();
        let originals = scene
            .objects
            .iter()
            .map(|object| (object.id.clone(), object.local))
            .collect();
        let mut state = State {
            level_original_transforms: originals,
            level_scene: Some(Arc::new(scene)),
            level_selected: Some(0),
            ..State::default()
        };
        sync_level_fields(&mut state);
        let _ = update(
            &mut state,
            Message::LevelEditPositionChanged("11,12,13".into()),
        );
        let _ = update(&mut state, Message::LevelApplyTransform);
        let actual = state.level_scene.as_ref().unwrap();
        assert_eq!(actual.objects[1].world.position, [15.0, 17.0, 19.0]);
        assert_eq!(state.level_undo.len(), 1);
        assert!(state.level_redo.is_empty());
        let _ = update(&mut state, Message::LevelUndoTransform);
        assert_eq!(
            state.level_scene.as_ref().unwrap().objects[1]
                .world
                .position,
            [5.0, 7.0, 9.0]
        );
        assert_eq!(state.level_redo.len(), 1);
        assert_eq!(state.level_dirty_count, 0);
        assert!(level_pending_edits(&state).is_empty());
        let _ = update(&mut state, Message::LevelRedoTransform);
        assert_eq!(
            state.level_scene.as_ref().unwrap().objects[1]
                .world
                .position,
            [15.0, 17.0, 19.0]
        );
        assert_eq!(initial.objects[1].world.position, [5.0, 7.0, 9.0]);
        assert_eq!(state.level_original_transforms.len(), 2);
        assert_eq!(state.level_dirty_count, 1);
    }

    #[test]
    fn cdf_single_click_sets_active_character_source_before_background_assembly() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("hunter.cdf");
        std::fs::write(&file, b"<CharacterDefinition/>").unwrap();
        let inventory = scan_directory(dir.path()).unwrap();
        let index = inventory
            .entries
            .iter()
            .position(|e| e.relative_path.ends_with("hunter.cdf"))
            .unwrap();
        let mut state = State {
            inventory: Some(inventory),
            selected: Some(index),
            ..State::default()
        };
        let _ = update(&mut state, Message::ViewerPreviewStaticCdf);
        assert_eq!(
            std::fs::canonicalize(&state.input).unwrap(),
            std::fs::canonicalize(&file).unwrap(),
            "Windows short 8.3 and extended-path spellings must resolve to the same CDF",
        );
        assert!(state.busy, "CDF assembly must be dispatched on a worker");
    }

    #[test]
    fn selecting_new_dds_during_preview_queues_only_latest_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("first.dds"), b"stand-in DDS").unwrap();
        std::fs::write(dir.path().join("second.dds"), b"stand-in DDS").unwrap();
        let inventory = scan_directory(dir.path()).unwrap();
        let first = inventory
            .entries
            .iter()
            .position(|e| e.relative_path == "first.dds")
            .unwrap();
        let second = inventory
            .entries
            .iter()
            .position(|e| e.relative_path == "second.dds")
            .unwrap();
        let mut state = State {
            inventory: Some(inventory),
            ..State::default()
        };
        refresh_viewer_rows(&mut state);
        let _ = update(&mut state, Message::ViewerSelectFile(first));
        assert!(state.busy);
        assert!(state.input.ends_with("first.dds"));
        let _ = update(&mut state, Message::ViewerSelectFile(second));
        assert_eq!(state.viewer_queued_preview, Some(second));
    }

    #[test]
    fn model_preview_dispatch_includes_compiled_cryengine_variants_without_inspect() {
        for name in [
            "Hunters/Kala.SKIN",
            "Monsters/Goliath.skinm",
            "Hunters/Griffin.CHR",
            "Hunters/Griffin.chrm",
            "Objects/tree.CGF",
            "Objects/tree.cgfm",
            "Hunters/Jack.cga",
            "Hunters/Jack.cgam",
        ] {
            assert!(
                is_viewer_model_source(name),
                "failed to auto-preview {name}"
            );
            assert!(is_viewer_preview_source(name));
        }
        for name in [
            "Hunters/Kala.cdf",
            "models/body.mtl",
            "tex/body.dds.0",
            "tex/body.dds.5",
            "tex/body_ddn.dds.0a",
            "tex/body_ddn.dds.3a",
        ] {
            assert!(is_viewer_preview_source(name));
            assert!(!is_viewer_model_source(name));
        }
        for name in ["model.skin.tmp", "unknown.bin", "../evil.exe"] {
            assert!(!is_viewer_preview_source(name));
        }
    }

    #[test]
    fn authored_undo_history_keeps_exact_64_latest_diffs() {
        let xml = br#"<Objects><Object Id="parent" Pos="1,0,0"/></Objects>"#;
        let scene = evolve_core::parse_level_scene(&[("test.lyr", xml)]).unwrap();
        let baseline = scene.objects[0].local;
        let mut state = State {
            level_scene: Some(Arc::new(scene)),
            level_selected: Some(0),
            level_original_transforms: HashMap::from([("parent".into(), baseline)]),
            ..State::default()
        };
        sync_level_fields(&mut state);
        for i in 2..=70 {
            let _ = update(
                &mut state,
                Message::LevelEditPositionChanged(format!("{i},0,0")),
            );
            let _ = update(&mut state, Message::LevelApplyTransform);
        }
        assert_eq!(state.level_undo.len(), 64);
        assert_eq!(state.level_dirty_count, 1);
        for _ in 0..64 {
            let _ = update(&mut state, Message::LevelUndoTransform);
        }
        // The five oldest edits aged out; no fabricated full-history undo.
        assert_eq!(
            state.level_scene.as_ref().unwrap().objects[0]
                .local
                .position,
            [6., 0., 0.]
        );
        assert_eq!(state.level_redo.len(), 64);
        assert_eq!(state.level_dirty_count, 1);
        for _ in 0..64 {
            let _ = update(&mut state, Message::LevelRedoTransform);
        }
        assert_eq!(
            state.level_scene.as_ref().unwrap().objects[0]
                .local
                .position,
            [70., 0., 0.]
        );
        assert_eq!(state.level_original_transforms["parent"], baseline);
    }

    #[test]
    fn library_configuration_routes_to_storage_and_preserves_edits_between_tabs() {
        let mut state = State {
            page: Some(Page::AssetLibrary),
            ..State::default()
        };
        let _ = update(&mut state, Message::OpenLibrarySettings);
        assert_eq!(state.page, Some(Page::Settings));
        assert_eq!(state.settings_tab, settings_page::Tab::Storage);
        state.library_page = 3;
        state.library_matches = vec![1, 2];
        state.library_selection.indices.insert(1);
        let _ = update(&mut state, Message::PreferenceAssets("C:/Library".into()));
        let _ = update(
            &mut state,
            Message::SettingsTab(settings_page::Tab::Updates),
        );
        let _ = update(
            &mut state,
            Message::SettingsTab(settings_page::Tab::Storage),
        );
        assert_eq!(state.preferences.asset_library_root, "C:/Library");
        assert_eq!(state.library_root, "C:/Library");
        assert_eq!(state.library_page, 0);
        assert!(state.library_selection.indices.is_empty());
        assert!(state.library_matches.is_empty());
        let _ = update(&mut state, Message::PreferenceAssets(String::new()));
        state.page = Some(Page::AssetLibrary);
        let _ = update(&mut state, Message::LibraryRefresh);
        assert_eq!(state.page, Some(Page::Settings));
        assert!(!state.busy);
    }

    #[test]
    fn project_paging_filtering_and_control_deselection_use_visible_rows() {
        let mut state = State {
            modding_projects: Some(
                (0..185)
                    .map(|i| ProjectRow {
                        label: format!("Project {i}"),
                        target: format!("{i}.pak"),
                        source_build: String::new(),
                        status: "Modified".into(),
                        details: String::new(),
                        workspace: PathBuf::from(format!("projects/{i}")),
                        modified: (0..181).map(|j| format!("file{j}.xml")).collect(),
                        changed_known: true,
                    })
                    .collect(),
            ),
            ..State::default()
        };
        let _ = update(&mut state, Message::ModdingPage(true));
        assert_eq!(state.modding_page, 1);
        let _ = update(&mut state, Message::ModdingSelectProject(0));
        assert_eq!(state.modding_selected, None);
        let _ = update(&mut state, Message::ModdingSelectProject(95));
        assert_eq!(state.modding_selected, Some(95));
        let _ = update(&mut state, Message::ModdingChangesPage(true));
        let _ = update(&mut state, Message::ModdingChangesPage(true));
        let _ = update(&mut state, Message::ModdingChangesPage(true));
        assert_eq!(state.modding_changes_page, 2);
        state.keyboard_modifiers = iced::keyboard::Modifiers::CTRL;
        let _ = update(&mut state, Message::ModdingSelectProject(95));
        assert_eq!(state.modding_selected, None);
        assert!(state.preferences.current_workspace.is_empty());
        let _ = update(&mut state, Message::ModdingPage(true));
        assert_eq!(modding_page::page_range(185, state.modding_page), 180..185);
        let _ = update(
            &mut state,
            Message::ModdingFilterChanged("Project 184".into()),
        );
        assert_eq!(state.modding_page, 0);
        assert_eq!(modding_page::matching_projects(&state), vec![184]);
        let _ = update(&mut state, Message::ModdingSelectProject(184));
        assert_eq!(state.modding_selected, Some(184));
        assert_eq!(modding_page::page_range(0, usize::MAX), 0..0);
    }

    #[test]
    fn restore_rejects_protected_library_and_unmanaged_viewers() {
        let mut state = State {
            viewer_from_library: true,
            ..State::default()
        };
        let _ = update(&mut state, Message::EditorRestoreRawMember);
        assert!(!state.busy);
        assert!(state.status.contains("read-only"));
        state.viewer_from_library = false;
        let _ = update(&mut state, Message::EditorRestoreRawMember);
        assert!(!state.busy);
        assert!(state.status.contains("managed editable project"));
    }

    #[test]
    fn restore_rejects_an_inventory_outside_the_selected_project() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let other = temp.path().join("other");
        std::fs::create_dir_all(project.join("files")).unwrap();
        std::fs::create_dir(&other).unwrap();
        let mut state = State {
            modding_selected: Some(0),
            modding_projects: Some(vec![ProjectRow {
                label: "test".into(),
                target: "test.pak".into(),
                source_build: String::new(),
                status: "Modified".into(),
                details: String::new(),
                workspace: project,
                modified: vec![],
                changed_known: true,
            }]),
            inventory: Some(scan_directory(&other).unwrap()),
            ..State::default()
        };
        let _ = update(&mut state, Message::EditorRestoreRawMember);
        assert!(!state.busy);
        assert!(state.status.contains("do not belong"));
    }
    #[test]
    fn popup_menus_are_mutually_exclusive_and_dismiss_without_removing_page() {
        let mut state = State {
            page: Some(Page::Files),
            ..State::default()
        };
        let _ = update(&mut state, Message::ModdingMenu("File"));
        assert_eq!(state.modding_menu, Some("File"));
        let _ = update(&mut state, Message::LibraryMenu("View"));
        assert_eq!(state.library_menu, Some("View"));
        assert!(state.modding_menu.is_none());
        let _ = update(&mut state, Message::ViewerMenu("Edit"));
        assert_eq!(state.viewer_menu, Some("Edit"));
        assert!(state.library_menu.is_none());
        let _ = update(&mut state, Message::DismissMenus);
        assert!(state.viewer_menu.is_none());
        assert!(state.library_menu.is_none());
        assert!(state.modding_menu.is_none());
        assert_eq!(state.page, Some(Page::Files));
    }
}
