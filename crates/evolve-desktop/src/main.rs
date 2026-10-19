//! Native wgpu-backed desktop shell. Asset operations use blocking workers, so
//! file I/O/decompression never runs in the interactive Iced view/update path.
mod asset_cache;
mod asset_library_page;
mod asset_viewer;
mod developer_auth;
mod explorer_tree;
mod level_workspace;
mod modding_keys;
mod modding_page;
mod modding_projects;
mod private_pages;
mod selection;
mod settings;
mod textured;
mod updater;
mod uv_preview;
mod viewer_inspector;
mod viewport;
use asset_cache::{SharedCache, decoded_dds};
use evolve_core::{
    AnimatedGlb, AnimationPolicy, AssetComparison, AssetResolution, BatchJournal, CdfDefinition,
    CdfDependencyReport, ConvertedMotion, DdsInfo, DependencyGraph, ImportedLibrary, ImportedPak,
    Inventory, LevelScene, MaterialReport, ModProfile, ModelInfo, NativeCafMotion, PakInventory,
    ProfileCatalog, ProfileMod, ProfilePlan, RenderMesh, RigReport, SignedPakInfo,
    StageTransaction, TerrainInfo, add_mod_to_profile, apply_profile_stage, apply_stage_pak,
    batch_stage_status, compare_assets, convert_glb_to_fbx, convert_original_motion,
    create_mod_profile, decode_dds_rgba, export_dds_png, export_level_scene_glb,
    export_rigged_character_fbx_textured, export_rigged_character_glb, extract_legacy_pak,
    extract_signed_pak, import_stage_pak, imported_archive_files, imported_library,
    initialize_stage, inspect_dds, inspect_dependencies, inspect_legacy_pak, inspect_signed_pak,
    list_mod_profiles, load_level_scene, plan_mod_profile, read_cdf, read_material, read_model,
    read_native_caf, read_terrain, rebuild_legacy_pak, rebuild_signed_pak_entry, resolve_cdf,
    resolve_materials, restore_profile_stage, restore_stage, scan_directory,
    scan_directory_with_progress, stage_status,
};
use iced::widget::{
    button, canvas, column, container, image, row, scrollable, shader, slider, space, svg, text,
    text_input, tooltip,
};
use iced::{Element, Fill, Length, Subscription, Task, Theme, window};
use modding_projects::{ProjectRow, discover_projects};
use selection::MultiSelection;
use settings::{Preferences, load_from, save_to, settings_path};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;
use textured::Scene as TexturedScene;
use updater::{
    ReleaseUpdate, UpdateChannel, discover_update, download_installer, launch_verified_installer,
};
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

#[derive(Default)]
struct State {
    input: String,
    output: String,
    replacements: String,
    signed_entry: String,
    signed_replacement: String,
    stage_feedback: String,
    profile_name: String,
    profile_id: String,
    profile_catalog: Option<ProfileCatalog>,
    profile_feedback: String,
    diffuse_input: String,
    diffuse: Option<Arc<::image::RgbaImage>>,
    diffuse_status: String,
    normal_input: String,
    normal_map: Option<Arc<::image::RgbaImage>>,
    emissive_input: String,
    emissive_map: Option<Arc<::image::RgbaImage>>,
    material_input: String,
    material_report: Option<MaterialReport>,
    caf_path: String,
    skeleton_path: String,
    blender_path: String,
    converter_path: String,
    converted_glbs: Vec<PathBuf>,
    glb_motion: Option<Arc<AnimatedGlb>>,
    selected_motion_clip: usize,
    native_caf: Option<Arc<NativeCafMotion>>,
    motion_time: f32,
    playing: bool,
    assets_root: String,
    terrain_source: String,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    search: String,
    page: Option<Page>,
    inventory: Option<Inventory>,
    pak: Option<PakInventory>,
    signed_pak: Option<SignedPakInfo>,
    texture: Option<DdsInfo>,
    character: Option<CdfDefinition>,
    cdf_dependencies: Option<CdfDependencyReport>,
    model: Option<Arc<RenderMesh>>,
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
    update_status: String,
    available_update: Option<ReleaseUpdate>,
    staged_installer: Option<PathBuf>,
    startup_update_checked: bool,
    viewer_search: String,
    viewer_collapsed: HashSet<String>,
    viewer_menu: Option<&'static str>,
    viewer_explorer_visible: bool,
    viewer_from_library: bool,
    developer_password: String,
    developer_status: String,
    developer_unlocked: bool,
    developer_attempts: u8,
    developer_checking: bool,
    show_advanced_tools: bool,
    show_activity: bool,
    library_catalog: Option<ImportedLibrary>,
    library_selected: Option<usize>,
    library_root: String,
    library_search: String,
    library_by_build: bool,
    library_build_index: usize,
    library_page: usize,
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
    level_zoom: f32,
    scan_cancel: Option<Arc<AtomicBool>>,
    scan_indexed: Option<Arc<AtomicUsize>>,
    preview_cache: SharedCache,
    modding_projects: Option<Vec<ProjectRow>>,
    modding_selected: Option<usize>,
    modding_filter: String,
    modding_menu: Option<&'static str>,
    modding_advanced: bool,
    modding_profiles: bool,
    modding_open_editor_when_scanned: bool,
    modding_pending_focus: Option<PathBuf>,
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
    UnpackRawProject(PathBuf, PathBuf, PathBuf),
    LoadImportedLibrary(PathBuf),
    OpenImportedPak(ImportedPak),
    InspectDependencies(Inventory, String),
    CompareAssets(PathBuf, PathBuf),
    Scan(PathBuf, Arc<AtomicBool>, Arc<AtomicUsize>),
    InspectPak(PathBuf),
    InspectDds(PathBuf),
    LoadDiffuse(PathBuf, SharedCache),
    LoadNormal(PathBuf, SharedCache),
    LoadEmissive(PathBuf, SharedCache),
    ResolveMaterial(PathBuf, PathBuf),
    LoadCaf(PathBuf),
    RiggedGlb(PathBuf, PathBuf, PathBuf),
    RiggedFbx(PathBuf, PathBuf, PathBuf, Option<PathBuf>, PathBuf),
    ConvertMotion(PathBuf, PathBuf, PathBuf, PathBuf),
    LoadGlbMotion(PathBuf),
    InspectCdf(PathBuf),
    InspectCdfResolved(PathBuf, Inventory),
    ResolveCdf(PathBuf, PathBuf),
    InspectModel(PathBuf),
    InspectLevel(PathBuf),
    InspectTerrain(PathBuf),
    InspectSignedPak(PathBuf, PathBuf, String),
    ExtractSignedPak(PathBuf, PathBuf, String, PathBuf),
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
    ImportedLibrary(ImportedLibrary),
    ImportedPakFiles(Inventory),
    Dependencies(DependencyGraph),
    Comparison(AssetComparison),
    Inventory(Inventory),
    Pak(PakInventory),
    Texture(DdsInfo),
    Diffuse(DdsInfo, Arc<::image::RgbaImage>),
    Normal(DdsInfo, Arc<::image::RgbaImage>),
    Emissive(DdsInfo, Arc<::image::RgbaImage>),
    MaterialResolved(
        MaterialReport,
        Option<::image::RgbaImage>,
        Option<::image::RgbaImage>,
        Option<::image::RgbaImage>,
    ),
    Caf(NativeCafMotion),
    RiggedExport(RigReport, PathBuf),
    MotionConverted(ConvertedMotion, PathBuf),
    MotionGlb(AnimatedGlb),
    Character(CdfDefinition),
    CharacterResolved(CdfDefinition, CdfDependencyReport),
    CharacterDependencies(CdfDependencyReport),
    Model(RenderMesh),
    Level(LevelScene),
    Terrain(RenderMesh, TerrainInfo),
    SignedPak(SignedPakInfo),
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
            Self::OpenImportedPak(pak) => imported_archive_files(&pak)
                .map(Completed::ImportedPakFiles)
                .map_err(|e| e.to_string()),
            Self::Scan(source, cancelled, indexed) => {
                scan_directory_with_progress(&source, &cancelled, &indexed)
                    .map(Completed::Inventory)
                    .map_err(|e| e.to_string())
            }
            Self::InspectPak(source) => inspect_legacy_pak(&source)
                .map(Completed::Pak)
                .map_err(|e| e.to_string()),
            Self::RiggedGlb(mesh, skeleton, dest) => {
                export_rigged_character_glb(&mesh, &skeleton, &dest)
                    .map(|r| Completed::RiggedExport(r, dest))
                    .map_err(|e| e.to_string())
            }
            Self::RiggedFbx(mesh, skeleton, blender, diffuse, dest) => {
                export_rigged_character_fbx_textured(
                    &mesh,
                    &skeleton,
                    &blender,
                    diffuse.as_deref(),
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
            Self::LoadCaf(source) => read_native_caf(&source)
                .map(Completed::Caf)
                .map_err(|e| e.to_string()),
            Self::ResolveMaterial(source, root) => {
                let authored = read_material(&source).map_err(|e| e.to_string())?;
                let library = scan_directory(&root).map_err(|e| e.to_string())?;
                let report = resolve_materials(&library, &authored);
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
                    let source = root.join(relative);
                    let metadata = std::fs::symlink_metadata(&source).map_err(|e| e.to_string())?;
                    if !metadata.file_type().is_file()
                        || !std::fs::canonicalize(&source)
                            .map_err(|e| e.to_string())?
                            .starts_with(std::fs::canonicalize(&root).map_err(|e| e.to_string())?)
                    {
                        return Err("Resolved DDS path is unsafe".into());
                    }
                    let (_, image) = decode_dds_rgba(&source).map_err(|e| e.to_string())?;
                    Ok(Some(image))
                };
                let diffuse = get_pixels(&["diffuse"])?;
                let normal = get_pixels(&["normals", "normal", "bumpmap"])?;
                let emissive = get_pixels(&["emittance", "emissive", "emission", "glow"])?;
                Ok(Completed::MaterialResolved(
                    report, diffuse, normal, emissive,
                ))
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
            Self::InspectModel(source) => read_model(&source)
                .map(Completed::Model)
                .map_err(|e| e.to_string()),
            Self::InspectLevel(source) => load_level_scene(&source)
                .map(Completed::Level)
                .map_err(|e| e.to_string()),
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
            Self::InspectSignedPak(source, key, name) => inspect_signed_pak(&source, &key, &name)
                .map(Completed::SignedPak)
                .map_err(|e| e.to_string()),
            Self::ExtractSignedPak(source, key, name, dest) => {
                extract_signed_pak(&source, &key, &name, &dest)
                    .map(|i| Completed::ExtractedSigned(i.entries.len(), dest))
                    .map_err(|e| e.to_string())
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
            Self::InspectCdfResolved(source, inventory) => {
                let cdf = read_cdf(&source).map_err(|e| e.to_string())?;
                let resolved = resolve_cdf(&inventory, &cdf);
                Ok(Completed::CharacterResolved(cdf, resolved))
            },
            Self::RebuildSignedPak(
                src,
                old_pub,
                signing_key,
                new_pub,
                name,
                member,
                replacement,
                dest,
            ) => {
                let result = rebuild_signed_pak_entry(
                    &src,
                    &old_pub,
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
    LibraryRootChanged(String),
    LibrarySearchChanged(String),
    LibraryMenu(&'static str),
    ModdingMenu(&'static str),
    ModdingFilterChanged(String),
    ModdingSelectProject(usize),
    ModdingRefresh,
    ModdingToggleAdvanced,
    ModdingBrowsePaks,
    ModdingSelectArchive(usize),
    ModdingOpenProject,
    ModdingReviewChanges,
    ModdingBuild,
    ModdingUnpackPak,
    ModdingToggleProfiles,
    LibraryToggleConfiguration,
    LibraryRefresh,
    LibrarySetByBuild(bool),
    LibraryNextBuild,
    LibrarySelect(usize),
    LibraryPreview,
    LibraryPreviousPage,
    LibraryNextPage,
    OpenViewer,
    OpenInspectorWindow,
    InspectorOpened,
    ViewerOpened,
    WindowClosed(window::Id),
    FileDropped(window::Id, PathBuf),
    DropPath(PathBuf),
    CancelScan,
    ScanPulse,
    ViewerMenu(&'static str),
    ViewerToggleExplorer,
    ViewerResetCamera,
    ViewerSearchChanged(String),
    ViewerSelectFile(usize),
    ModifiersChanged(iced::keyboard::Modifiers),
    ViewerToggleFolder(String),
    ViewerPreviewSelected,
    StartDependencyInspection,
    CompareOriginalChanged(String),
    CompareModifiedChanged(String),
    StartAssetCompare,
    ViewerToggleInspector,
    ViewerSplitChanged(f32),
    ViewerSelectTab(ViewerTab),
    ViewerSaveLayout,
    PlaybackSpeedChanged(f32),
    PlaybackLoopChanged,
    ViewerToggleWireframe,
    LightStrengthChanged(f32),
    LevelSelect(usize),
    LevelFilterChanged(String),
    LevelZoomChanged(f32),
    ToggleAdvancedTools,
    ToggleActivity,
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
    PreferenceReduceMotion,
    PreferenceSnapshots,
    PreferenceActivity,
    PreferenceExperimental,
    PreferenceAutoUpdates,
    PreferenceStage(String),
    PreferenceSwap(String),
    PreferenceProjects(String),
    PreferenceAssets(String),
    PreferenceBlender(String),
    PreferenceWorkspace(String),
    SavePreferences,
    CheckUpdates,
    UpdateChecked(Result<Option<ReleaseUpdate>, String>),
    DownloadUpdate,
    UpdateDownloaded(Result<PathBuf, String>),
    RunVerifiedInstaller,
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
    StartLevelGlbExport,
    StartLevelFbxExport,
    StartExtract,
    StartRebuild,
    StartPngExport,
    Finished(Result<Completed, String>),
}

fn launch(state: &mut State, operation: Operation) -> Task<Message> {
    if state.busy {
        return Task::none();
    }
    state.busy = true;
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

fn begin_scan(state: &mut State, source: PathBuf) -> Task<Message> {
    if state.busy {
        return Task::none();
    }
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
                && !state.library_root.trim().is_empty()
            {
                return launch(
                    state,
                    Operation::LoadImportedLibrary(PathBuf::from(&state.library_root)),
                );
            }
        }
        Message::ModdingMenu(menu) => {
            state.modding_menu = if state.modding_menu == Some(menu) {
                None
            } else {
                Some(menu)
            };
        }
        Message::ModdingFilterChanged(value) => state.modding_filter = value,
        Message::ModifiersChanged(modifiers) => state.keyboard_modifiers = modifiers,
        Message::ModdingSelectProject(index) => {
            if let Some(project) = state.modding_projects.as_ref().and_then(|p| p.get(index)) {
                let query = state.modding_filter.to_ascii_lowercase();
                let visible: Vec<usize> = state
                    .modding_projects
                    .as_deref()
                    .unwrap_or(&[])
                    .iter()
                    .enumerate()
                    .filter_map(|(i, p)| {
                        (query.is_empty()
                            || p.label.to_ascii_lowercase().contains(&query)
                            || p.target.to_ascii_lowercase().contains(&query)
                            || p.source_build.to_ascii_lowercase().contains(&query))
                        .then_some(i)
                    })
                    .take(90)
                    .collect();
                state.project_selection.click(
                    &visible,
                    index,
                    state.keyboard_modifiers.shift(),
                    state.keyboard_modifiers.control(),
                );
                state.modding_selected = state.project_selection.primary_after(index);
                state.modding_selected = Some(index);
                state.preferences.current_workspace =
                    project.workspace.to_string_lossy().into_owned();
                state.input = PathBuf::from(&state.preferences.stage)
                    .join("paks")
                    .join(&project.target)
                    .to_string_lossy()
                    .into_owned();
                state.status = format!("Selected {} · {}", project.label, project.details);
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
                state.status = "A signed build needs 1–64 verified changed raw project files. CryXmlB requires the private manager until semantic round-trip conversion is ported.".into();
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
            state.status =
                "Creating an editable raw project from the verified signed PAK...".into();
            return launch(
                state,
                Operation::UnpackRawProject(
                    PathBuf::from(&state.preferences.projects),
                    PathBuf::from(&state.preferences.stage),
                    PathBuf::from(&state.input),
                ),
            );
        }
        Message::LibraryRootChanged(value) => {
            state.library_root = value;
            state.library_catalog = None;
            state.library_selected = None;
            state.library_page = 0;
            refresh_library_matches(state);
        }
        Message::LibrarySearchChanged(value) => {
            state.library_search = value;
            state.library_page = 0;
            refresh_library_matches(state);
        }
        Message::LibraryMenu(menu) => {
            state.library_menu = if state.library_menu == Some(menu) {
                None
            } else {
                Some(menu)
            };
        }
        Message::LibraryToggleConfiguration => {
            state.library_show_configuration = !state.library_show_configuration;
        }
        Message::LibraryRefresh => {
            if state.library_root.trim().is_empty() {
                state.status =
                    "Choose an existing AssetLibrary folder to browse imported PAKs.".into();
                state.library_show_configuration = true;
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
                let begin = state.library_page.saturating_mul(110);
                let visible: Vec<usize> = state
                    .library_matches
                    .iter()
                    .skip(begin)
                    .take(110)
                    .copied()
                    .collect();
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
        Message::ToggleAdvancedTools => state.show_advanced_tools = !state.show_advanced_tools,
        Message::ToggleActivity => state.show_activity = !state.show_activity,
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
        Message::OpenViewer
        | Message::OpenInspectorWindow
        | Message::InspectorOpened
        | Message::ViewerOpened
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
            } else if [".skin", ".skinm", ".chr", ".cgf"]
                .iter()
                .any(|ext| name.ends_with(ext))
            {
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
            state.viewer_menu = if section.is_empty() || state.viewer_menu == Some(section) {
                None
            } else {
                Some(section)
            };
        }
        Message::ViewerToggleExplorer => {
            state.viewer_explorer_visible = !state.viewer_explorer_visible;
        }
        Message::ViewerToggleInspector => {
            state.viewer_show_inspector = !state.viewer_show_inspector
        }
        Message::ViewerSplitChanged(value) => state.viewer_split = value.clamp(0.25, 0.8),
        Message::ViewerSelectTab(tab) => state.viewer_tab = tab,
        Message::PlaybackSpeedChanged(value) => state.playback_speed = value.clamp(0.1, 3.0),
        Message::PlaybackLoopChanged => state.playback_loop = !state.playback_loop,
        Message::ViewerToggleWireframe => state.viewer_wireframe = !state.viewer_wireframe,
        Message::LightStrengthChanged(value) => state.light_strength = value.clamp(0.1, 2.0),
        Message::LevelSelect(index) => {
            if state
                .level_scene
                .as_ref()
                .is_some_and(|scene| index < scene.objects.len())
            {
                state.level_selected = Some(index);
            }
        }
        Message::LevelFilterChanged(value) => state.level_filter = value,
        Message::LevelZoomChanged(value) => state.level_zoom = value.clamp(0.3, 3.0),
        Message::ViewerSaveLayout => {
            state.preferences.viewer_split = state.viewer_split;
            state.preferences.viewer_show_inspector = state.viewer_show_inspector;
            state.preferences.viewer_wireframe = state.viewer_wireframe;
            state.preferences.light_strength = state.light_strength;
            state.preferences.animation_speed = state.playback_speed;
            state.preferences.animation_loop = state.playback_loop;
            state.preferences.saved_yaw = state.yaw;
            state.preferences.saved_pitch = state.pitch;
            state.preferences.saved_zoom = state.zoom;
            persist_preferences(state);
            state.status = "Viewer layout, camera and animation speed saved.".into();
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
        }
        Message::ViewerSearchChanged(filter) => state.viewer_search = filter,
        Message::ViewerSelectFile(index) => {
            let records: Vec<(usize, &str, u64)> = if let Some(inv) = &state.inventory {
                inv.entries
                    .iter()
                    .enumerate()
                    .map(|(i, e)| (i, e.relative_path.as_str(), e.byte_size))
                    .collect()
            } else if let Some(signed) = &state.signed_pak {
                signed
                    .entries
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
            let visible: Vec<usize> = explorer_tree::visible_rows(
                records.iter().map(|(i, path, size)| (*i, *path, *size)),
                &state.viewer_search,
                &state.viewer_collapsed,
                350,
            )
            .into_iter()
            .filter_map(|row| row.index)
            .collect();
            state.file_selection.click(
                &visible,
                index,
                state.keyboard_modifiers.shift(),
                state.keyboard_modifiers.control(),
            );
            state.selected = state.file_selection.primary_after(index);
        }
        Message::ViewerToggleFolder(path) => {
            if !state.viewer_collapsed.insert(path.clone()) {
                state.viewer_collapsed.remove(&path);
            }
        }
        Message::ViewerPreviewSelected => {
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
            let filename = relative.relative_path.to_ascii_lowercase();
            let operation = if [".skin", ".skinm", ".chr", ".cgf"]
                .iter()
                .any(|s| filename.ends_with(s))
            {
                Operation::InspectModel(source)
            } else if filename.ends_with(".cdf") {
                Operation::InspectCdfResolved(source, inventory.clone())
            } else if filename.ends_with(".dds") || filename.ends_with(".dds.0") {
                Operation::InspectDds(source)
            } else if filename.ends_with(".caf") {
                Operation::LoadCaf(source)
            } else if filename.ends_with(".mtl") {
                Operation::ResolveMaterial(source, PathBuf::from(&inventory.root))
            } else {
                state.status =
                    "Selected asset type is not supported by the native preview yet.".into();
                return Task::none();
            };
            return launch(state, operation);
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
        Message::PreferenceExperimental => {
            state.preferences.experimental_updates = !state.preferences.experimental_updates;
            persist_preferences(state);
        }
        Message::PreferenceAutoUpdates => {
            state.preferences.automatic_update_checks = !state.preferences.automatic_update_checks;
            persist_preferences(state);
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
            state.library_catalog = None;
            state.library_selected = None;
            state.preferences.asset_library_root = value;
        }
        Message::PreferenceBlender(value) => state.preferences.blender_executable = value,
        Message::PreferenceWorkspace(value) => state.preferences.current_workspace = value,
        Message::SavePreferences => persist_preferences(state),
        Message::CheckUpdates => {
            state.startup_update_checked = true;
            state.update_status = "Checking public GitHub Releases for verified updates...".into();
            state.available_update = None;
            state.staged_installer = None;
            let channel = if state.preferences.experimental_updates {
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
        Message::DownloadUpdate => {
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
                return Task::none();
            };
            state.busy = true;
            state.update_status = "Downloading and verifying installer checksum...".into();
            return Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        download_installer(&update, &root).map_err(|e| e.to_string())
                    })
                    .await
                    .map_err(|e| format!("Download worker stopped: {e}"))?
                },
                Message::UpdateDownloaded,
            );
        }
        Message::UpdateDownloaded(result) => {
            state.busy = false;
            match result {
                Ok(path) => {
                    state.update_status =
                        "Installer checksum verified. Run it only if you approve this update."
                            .into();
                    state.staged_installer = Some(path);
                }
                Err(error) => {
                    state.update_status = format!("Update rejected: {error}");
                    state.staged_installer = None;
                }
            }
        }
        Message::RunVerifiedInstaller => {
            let (Some(installer), Some(update)) =
                (&state.staged_installer, &state.available_update)
            else {
                return Task::none();
            };
            let root = settings_path()
                .ok()
                .and_then(|p| p.parent().map(PathBuf::from));
            match root {
                Some(root) => {
                    state.update_status = match launch_verified_installer(installer, update, &root) {
                        Ok(()) => "Verified installer opened. Close the manager when the installer requests it.".into(),
                        Err(err) => format!("Could not launch verified installer: {err}"),
                    };
                }
                None => state.update_status = "User update folder unavailable.".into(),
            }
        }

        Message::InputChanged(input) => state.input = input,
        Message::OutputChanged(output) => state.output = output,
        Message::ReplacementsChanged(replacements) => state.replacements = replacements,
        Message::SignedMemberChanged(value) => state.signed_entry = value,
        Message::SignedReplacementChanged(value) => state.signed_replacement = value,
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
        Message::MotionScrub(value) => state.motion_time = value,
        Message::ToggleMotion => {
            state.playing = !state.playing
                && (state.native_caf.is_some()
                    || state
                        .glb_motion
                        .as_ref()
                        .is_some_and(|m| m.list_clips().get(state.selected_motion_clip).is_some()))
        }
        Message::Tick => {
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
            if state.input.trim().is_empty()
                || state.skeleton_path.trim().is_empty()
                || state.blender_path.trim().is_empty()
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
                    PathBuf::from(&state.blender_path),
                    (!state.diffuse_input.trim().is_empty())
                        .then(|| PathBuf::from(&state.diffuse_input)),
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
        Message::StartCafLoad => {
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
            let key = match modding_keys::public_key(&stage) {
                Ok(path) => path,
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
            return launch(state, Operation::InspectSignedPak(source, key, basename));
        }
        Message::StartSignedExtraction => {
            let source = PathBuf::from(&state.input);
            let stage = PathBuf::from(&state.preferences.stage);
            let key = match modding_keys::public_key(&stage) {
                Ok(path) => path,
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
                state.status = "Choose a new extraction folder, or use Unpack PAK in Modding to generate one automatically.".into();
                return Task::none();
            }
            return launch(
                state,
                Operation::ExtractSignedPak(source, key, basename, PathBuf::from(&state.output)),
            );
        }
        Message::StartSignedRebuild => {
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
            if state.signed_entry.trim().is_empty()
                || state.signed_replacement.trim().is_empty()
                || state.output.trim().is_empty()
            {
                state.status = "Select an exact virtual member, a replacement file and a NEW output PAK. Signing keys are resolved automatically.".into();
                return Task::none();
            }
            return launch(
                state,
                Operation::RebuildSignedPak(
                    source,
                    keys.public.clone(),
                    keys.private,
                    keys.public,
                    basename,
                    state.signed_entry.clone(),
                    PathBuf::from(&state.signed_replacement),
                    PathBuf::from(&state.output),
                ),
            );
        }
        Message::StartLevelFbxExport => {
            if [
                &state.input,
                &state.assets_root,
                &state.output,
                &state.blender_path,
            ]
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
                    PathBuf::from(&state.blender_path),
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
            state.scan_cancel = None;
            state.scan_indexed = None;
            match result {
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
                Ok(Completed::UnpackedProject(path, count)) => {
                    state.status = format!(
                        "Unpacked {count} original assets into a new raw editing project; originals unchanged."
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
                    state.project_selection.clear();
                    if let Some(path) = focus {
                        state.modding_open_editor_when_scanned = true;
                        return begin_scan(state, path.join("files"));
                    }
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
                    state.library_show_configuration = false;
                    refresh_library_matches(state);
                    state.page = Some(Page::AssetLibrary);
                    persist_preferences(state);
                }
                Ok(Completed::ImportedPakFiles(inventory)) => {
                    state.status = format!(
                        "Read-only preview: {} manifest-listed files checked for size and location.",
                        inventory.entries.len()
                    );
                    state.input = inventory.root.clone();
                    state.viewer_search.clear();
                    state.inventory = Some(inventory);
                    state.file_selection.clear();
                    state.pak = None;
                    state.signed_pak = None;
                    state.selected = None;
                    state.page = Some(Page::AssetLibrary);
                }
                Ok(Completed::Inventory(inventory)) => {
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
                    state.selected = None;
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
                    state.page = Some(Page::Files);
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
                Ok(Completed::MotionConverted(result, dir)) => {
                    state.status = format!(
                        "Converter produced {} validated GLB animation files, {} source clips from {}.",
                        result.glb_files.len(),
                        result.clips.len(),
                        result.virtual_source
                    );
                    state.converted_glbs =
                        result.glb_files.iter().map(|rel| dir.join(rel)).collect();
                    state.native_caf = None;
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
                    let clips = motion.list_clips();
                    state.status = format!(
                        "Loaded {} real glTF animation clip(s), {} source nodes.",
                        clips.len(),
                        motion.node_names.len()
                    );
                    state.motion_time = 0.0;
                    state.playing = false;
                    state.selected_motion_clip = 0;
                    state.native_caf = None;
                    state.glb_motion = Some(Arc::new(motion));
                    state.page = Some(Page::Models);
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
                    state.native_caf = Some(Arc::new(motion));
                    state.glb_motion = None;
                    state.page = Some(Page::Models);
                }
                Ok(Completed::MaterialResolved(report, diffuse, normal, emissive)) => {
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
                    state.status = format!(
                        "MTL: {} slot(s), {} authored references, {} uniquely resolved, {} ambiguous. No guessing.",
                        report.slots,
                        report.textures.len(),
                        found,
                        ambiguous
                    );
                    // A new authored material must not inherit a previous
                    // character's maps when the reference is missing/ambiguous.
                    state.diffuse = diffuse.map(Arc::new);
                    state.normal_map = normal.map(Arc::new);
                    state.emissive_map = emissive.map(Arc::new);
                    state.material_report = Some(report);
                    state.page = Some(Page::Models);
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
                Ok(Completed::CharacterResolved(character, report)) => {
                    let mut found = 0usize;
                    let mut missing = 0usize;
                    let mut ambiguous = 0usize;
                    for dependency in std::iter::once(&report.skeleton)
                        .chain(report.attachments.iter().map(|(_, status)| status))
                    {
                        match dependency {
                            AssetResolution::Found(_) => found += 1,
                            AssetResolution::Missing => missing += 1,
                            AssetResolution::Ambiguous(_) => ambiguous += 1,
                        }
                    }
                    state.status = format!(
                        "Character definition: {found} exact dependencies, {missing} missing, {ambiguous} ambiguous. Assembly rendering still requires verified skeleton/weights."
                    );
                    state.character = Some(character);
                    state.cdf_dependencies = Some(report);
                    state.viewer_tab = ViewerTab::Dependencies;
                    state.page = Some(Page::Models);
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
                Ok(Completed::Model(mesh)) => {
                    state.status = format!(
                        "Loaded {} source vertices and {} triangles (raw geometry only).",
                        mesh.info.vertex_count, mesh.info.triangle_count
                    );
                    state.model = Some(Arc::new(mesh));
                    state.page = Some(Page::Models);
                    state.zoom = 1.0;
                }
                Ok(Completed::Terrain(mesh, info)) => {
                    state.status = format!(
                        "Decoded source terrain {}×{} (stride {}).",
                        info.width, info.height, info.stride
                    );
                    state.terrain = Some(info);
                    state.model = Some(Arc::new(mesh));
                    state.page = Some(Page::Levels);
                    state.zoom = 1.0;
                }
                Ok(Completed::Level(scene)) => {
                    state.status = format!(
                        "Loaded {} placed level objects; {} diagnostics.",
                        scene.objects.len(),
                        scene.diagnostics.len()
                    );
                    state.level_scene = Some(Arc::new(scene));
                    state.level_selected = None;
                    state.level_zoom = 1.0;
                    state.page = Some(Page::Levels);
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
                Err(e) => state.status = format!("Operation refused: {e}"),
            }
        }
    }
    Task::none()
}

fn download_action(state: &State) -> Element<'_, Message> {
    if let Some(update) = &state.available_update {
        button(text(format!(
            "Download verified {} installer",
            update.version
        )))
        .on_press(Message::DownloadUpdate)
        .into()
    } else {
        text("Stable updates appear after the public release is published.").into()
    }
}
fn run_installer_action(state: &State) -> Element<'_, Message> {
    if state.staged_installer.is_some() {
        button("Run verified installer (requires approval)")
            .on_press(Message::RunVerifiedInstaller)
            .into()
    } else {
        text("No installer staged. The game and its PAKs are never modified by update checks.")
            .into()
    }
}

fn view(state: &State) -> Element<'_, Message> {
    let page = state.page.unwrap_or(Page::Home);
    // Preserve the private manager's compact rail and exact navigation order.
    // Hover reveals one tooltip, never the whole expanded sidebar.
    let navigation = Page::ALL
        .into_iter()
        .fold(column![].spacing(6), |items, item| {
            let active = page == item;
            let nav = button(svg(item.icon()).width(20).height(20))
                .width(46)
                .height(43)
                .padding(10)
                .style(if active {
                    button::primary
                } else {
                    button::text
                })
                .on_press(Message::Navigate(item));
            items.push(tooltip(nav, text(item.title()), tooltip::Position::Right))
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
            button(play_mods_icon).width(46).height(40),
            text("Play with Mods — production launch not yet supported"),
            tooltip::Position::Right
        ),
        tooltip(
            button(play_clean_icon).width(46).height(40),
            text("Play without Mods — production launch not yet supported"),
            tooltip::Position::Right
        ),
        tooltip(
            button(svg(Page::Settings.icon()).width(20).height(20))
                .width(46)
                .height(43)
                .style(if page == Page::Settings {
                    button::primary
                } else {
                    button::text
                })
                .on_press(Message::Navigate(Page::Settings)),
            text("Settings"),
            tooltip::Position::Right
        ),
    ]
    .spacing(5);
    let sidebar = container(
        column![navigation, space().height(Fill), lower,]
            .spacing(8)
            .height(Fill)
            .padding([6, 4]),
    )
    .width(62)
    .height(Fill);
    let headline = row![
        image(image::Handle::from_bytes(
            include_bytes!("../../../assets/hunt.png").to_vec()
        ))
        .width(25)
        .height(25),
        text("EVOLVE / MOD MANAGER").size(18),
        text(format!("  /  {}", page.title())).size(12),
        space().width(Fill),
        button("Help")
            .on_press(Message::Navigate(Page::Instructions))
            .style(button::secondary),
        button("Activity")
            .on_press(Message::ToggleActivity)
            .style(button::secondary),
        if state.scan_cancel.is_some() {
            button("Cancel Scan")
                .on_press(Message::CancelScan)
                .style(button::secondary)
        } else {
            button("Cancel Scan").style(button::secondary)
        },
    ]
    .spacing(13)
    .align_y(iced::Center);

    let location: iced::widget::Column<'_, Message> = column![
        row![
            text_input("Local asset folder, .pak or .dds path...", &state.input)
                .on_input(Message::InputChanged)
                .width(Fill),
            button("Scan folder")
                .on_press(Message::StartScan)
                .style(button::secondary),
            button("Open Asset Viewer")
                .on_press(Message::OpenViewer)
                .style(button::secondary),
        ]
        .spacing(8),
        row![
            button("Inspect ZIP PAK").on_press(Message::StartPakInspection),
            button("Inspect DDS").on_press(Message::StartDdsInspection),
            button("Inspect CDF").on_press(Message::StartCdfInspection),
            button("Inspect 3D model").on_press(Message::StartModelInspection),
        ]
        .spacing(8),
        row![
            text_input("New output directory or .png filename...", &state.output)
                .on_input(Message::OutputChanged)
                .width(Fill),
            button("Extract PAK").on_press(Message::StartExtract),
            button("Export DDS → PNG").on_press(Message::StartPngExport),
        ]
        .spacing(8),
        row![
            text_input(
                "Folder containing exact original PAK member replacements...",
                &state.replacements
            )
            .on_input(Message::ReplacementsChanged)
            .width(Fill),
            button("Rebuild unsigned PAK").on_press(Message::StartRebuild),
        ]
        .spacing(8),
        row![
            text("PAK signing: use the selected stage's local mykeys/ resources.").size(12),
            button("Verify signed PAK")
                .style(button::secondary)
                .on_press(Message::StartSignedInspection),
            button("Extract signed PAK")
                .style(button::secondary)
                .on_press(Message::StartSignedExtraction),
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
                    text_input("Extracted Asset Library root for CDF dependency resolution...", &state.assets_root)
                        .on_input(Message::AssetsRootChanged).width(Fill),
                    button("Resolve CDF parts").on_press(Message::StartCdfResolve),
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
                    text_input("Exact DDS diffuse (.dds or .dds.0) path...",&state.diffuse_input)
                        .on_input(Message::DiffuseInputChanged).width(Fill),
                    button("Load GPU diffuse").on_press(Message::StartDiffuseLoad),
                ].spacing(8),
                text(state.diffuse_status.as_str()),
                row![
                    text_input("Exact source CryEngine .mtl path...", &state.material_input)
                        .on_input(Message::MaterialInputChanged).width(Fill),
                    button("Resolve material DDS maps").on_press(Message::StartMaterialResolve),
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
                    text_input("Source normal map DDS (BC5)...", &state.normal_input)
                        .on_input(Message::NormalInputChanged).width(Fill),
                    button("Load GPU normal").on_press(Message::StartNormalLoad),
                    text_input("Source emissive or glow DDS...", &state.emissive_input)
                        .on_input(Message::EmissiveInputChanged).width(Fill),
                    button("Load GPU emissive").on_press(Message::StartEmissiveLoad),
                ].spacing(8),
                if let Some(mesh)=&state.model {
                    let display:Element<'_,Message> = if mesh.uv0.is_some(){
                        shader(TexturedScene{
                            mesh:mesh.clone(),diffuse:state.diffuse.clone(),
                            normal_map:state.normal_map.clone(),emissive:state.emissive_map.clone(),
                            yaw:state.yaw,pitch:state.pitch,zoom:state.zoom,
                            light_strength:state.light_strength,
                        }).width(Fill).height(Length::Fixed(400.)).into()
                    }else{
                        canvas(MeshViewport{
                            mesh:mesh.clone(),yaw:state.yaw,pitch:state.pitch,zoom:state.zoom,
                        }).width(Fill).height(Length::Fixed(400.)).into()
                    };
                    display
                } else {
                    text("Select a source .cgf/.chr/.skin file and click Inspect 3D model.").into()
                },
                text("Rigged character export — verified original weights").size(18),
                row![
                    text_input("Exact original .chr skeleton path...",&state.skeleton_path)
                        .on_input(Message::SkeletonChanged).width(Fill),
                    text_input("Blender.exe path for FBX...",&state.blender_path)
                        .on_input(Message::BlenderChanged).width(Fill),
                ].spacing(8),
                row![
                    button("Export rigged GLB").on_press(Message::StartRigGlb),
                    button("Export rigged FBX").on_press(Message::StartRigFbx),
                    text("FBX embeds explicitly selected DDS diffuse (Textures); skeletal Actions still need verified matched tracks."),
                ].spacing(9),
                text("Animation controls — exact native CAF tracks").size(18),
                row![
                    text_input("Select original CAF file...",&state.caf_path)
                        .on_input(Message::CafInputChanged).width(Fill),
                    button("Open CAF").on_press(Message::StartCafLoad),
                    button(if state.playing{"Pause"}else{"Play"}).on_press(Message::ToggleMotion),
                ].spacing(8),
                row![
                    text_input("Bundled or independent Cryengine-Converter.exe...",
                        &state.converter_path).on_input(Message::ConverterPathChanged).width(Fill),
                    button("Convert & preview CAF/DBA").on_press(Message::StartMotionConversion),
                ].spacing(8),
                text_input("Exact extracted asset root (for converter dependencies)...",
                    &state.assets_root).on_input(Message::AssetsRootChanged),
                if !state.converted_glbs.is_empty() {
                    let mut items=column![text("Verified converter GLB outputs (select exact file)")].spacing(4);
                    for (index,file) in state.converted_glbs.iter().enumerate().take(32) {
                        items=items.push(button(text(file.file_name().unwrap_or_default().to_string_lossy().to_string()))
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
                        chooser=chooser.push(button(text(format!("{} • {:.2}s • {} channels",clip.name,clip.duration,clip.channels)))
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
                        text("Motion values come directly from CAF keys. They are NOT applied to an unverified skeleton."),
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
                button("Read level placements").on_press(Message::StartLevelInspection),
                button("Load heightmap geometry").on_press(Message::StartTerrainInspection),
                button("Export geometry scene GLB").on_press(Message::StartLevelGlbExport),
                button("Export geometry scene FBX").on_press(Message::StartLevelFbxExport),
            ].spacing(8),
            row![
                text_input("Exact extracted asset root...",&state.assets_root)
                    .on_input(Message::AssetsRootChanged).width(Fill),
                text_input("Optional Heightmap.dat source...",&state.terrain_source)
                    .on_input(Message::TerrainSourceChanged).width(Fill),
            ].spacing(8),
            text(if let Some(scene)=&state.level_scene {
                format!("{} level objects, {} unresolved diagnostics; source transforms preserved.",
                    scene.objects.len(),scene.diagnostics.len())
            }else{"No placement source inspected.".into()}),
            if let Some(mesh)=&state.model {
                let display:Element<'_,Message> = if mesh.uv0.is_some(){
                    shader(TexturedScene{
                        mesh:mesh.clone(),diffuse:state.diffuse.clone(),
                            normal_map:state.normal_map.clone(),emissive:state.emissive_map.clone(),
                        yaw:state.yaw,pitch:state.pitch,zoom:state.zoom.max(0.2),
                            light_strength:state.light_strength,
                    }).width(Fill).height(Length::Fixed(400.)).into()
                }else{
                    canvas(MeshViewport{
                        mesh:mesh.clone(),yaw:state.yaw,pitch:state.pitch,zoom:state.zoom.max(0.2),
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
            text_input("Blender executable for scene FBX...", &state.blender_path)
                .on_input(Message::BlenderChanged),
            text("GLB/FBX includes verified mesh instances and optional terrain. Texture/material, compiled vegetation, scripting and full map fidelity remain gated."),
        ].spacing(14).into(),
        Page::Settings => column![
            text("Settings").size(23),
            text("UPDATES").size(18),
            text(format!("Installed version: {}. Downloads require a verified public release.",env!("CARGO_PKG_VERSION"))),
            button("Check for Updates").on_press(Message::CheckUpdates),
            text(state.update_status.as_str()),
            download_action(state),
            run_installer_action(state),
            button(text(format!("Experimental update channel: {}", if state.preferences.experimental_updates {"Enabled"} else {"Disabled"})))
                .on_press(Message::PreferenceExperimental),
            button(text(format!("Automatic update checks: {}", if state.preferences.automatic_update_checks {"Enabled"} else {"Disabled"})))
                .on_press(Message::PreferenceAutoUpdates),
            text("GENERAL & APPEARANCE").size(18),
            row![
                button(text(format!("Appearance: {}",state.preferences.appearance_mode))).on_press(Message::PreferenceAppearance),
                button(text(format!("Density: {}",state.preferences.ui_density))).on_press(Message::PreferenceDensity),
                button(text(format!("Explorer side: {}",state.preferences.file_explorer_side))).on_press(Message::PreferenceExplorerSide),
            ].spacing(9),
            button(text(format!("Reduce menu animation: {}",state.preferences.reduce_motion)))
                .on_press(Message::PreferenceReduceMotion),
            text("Theme color overrides, presets and font preference persist in settings.json; dedicated pickers are planned."),
            text("GAME STORAGE").size(18),
            text_input("Staging folder", &state.preferences.stage)
                .on_input(Message::PreferenceStage),
            text_input("Swap / restoration journal folder", &state.preferences.swap)
                .on_input(Message::PreferenceSwap),
            text_input("Projects folder", &state.preferences.projects)
                .on_input(Message::PreferenceProjects),
            text_input("Asset Library folder", &state.preferences.asset_library_root)
                .on_input(Message::PreferenceAssets),
            text("BLENDER INTEGRATION").size(18),
            text_input("Blender executable", &state.preferences.blender_executable)
                .on_input(Message::PreferenceBlender),
            text("BACKUPS & RECOVERY").size(18),
            button(text(format!("Keep permanent original snapshots: {}",state.preferences.keep_original_snapshots)))
                .on_press(Message::PreferenceSnapshots),
            text("Recovery snapshots are a preference, not an indication that the Rust mod swap is implemented yet."),
            text("LAUNCH PREFERENCES").size(18),
            button(text(format!("Open activity log during tasks: {}",state.preferences.show_activity_on_tasks)))
                .on_press(Message::PreferenceActivity),
            text("ADVANCED & MAINTENANCE").size(18),
            text_input("Current workspace",&state.preferences.current_workspace)
                .on_input(Message::PreferenceWorkspace),
            button("Save Settings").on_press(Message::SavePreferences),
            text(state.settings_status.as_str()).size(13),
        ].spacing(12).into(),
    };

    let workspace: Element<'_, Message> = if page == Page::Settings || page == Page::Files {
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
                text_input("Selected asset / folder / PAK path...", &state.input)
                    .on_input(Message::InputChanged)
                    .width(Fill),
                button("Scan Folder").on_press(Message::StartScan),
                button("Open Asset Viewer")
                    .on_press(Message::OpenViewer)
                    .style(button::secondary),
                button(if state.show_advanced_tools {
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
    let mut body = column![
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
    .padding([10, 16])
    .width(Fill)
    .height(Fill);
    if state.show_activity {
        body =
            body.push(container(text(format!("Activity: {}", state.status)).size(12)).padding(8));
    }
    container(row![sidebar, body].height(Fill))
        .width(Fill)
        .height(Fill)
        .into()
}

fn initial_state() -> State {
    let mut result = State {
        zoom: 1.0,
        playback_speed: 1.0,
        viewer_split: 0.67,
        level_zoom: 1.0,
        light_strength: 1.0,
        viewer_explorer_visible: true,
        ..State::default()
    };
    if let Ok(path) = settings_path() {
        match load_from(&path) {
            Ok(saved) => {
                result.blender_path = saved.blender_executable.clone();
                result.assets_root = saved.asset_library_root.clone();
                result.viewer_split = saved.viewer_split;
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
    if result.preferences.asset_library_root.trim().is_empty()
        && let Ok(settings) = settings_path()
        && let Some(parent) = settings.parent()
    {
        // The established manager stores persistent imported PAK manifests here.
        // Prefer its existing library without decrypting or copying originals.
        let existing = parent.join("Data").join("AssetLibrary");
        if existing.is_dir() {
            result.preferences.asset_library_root = existing.to_string_lossy().into_owned();
        }
    }
    result.library_root = result.preferences.asset_library_root.clone();
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
/// One native main window and one native model/asset editor window. All costly
/// asset operations stay in the existing worker queue, independent of either
/// window's render loop.
struct Desktop {
    state: State,
    main_id: window::Id,
    viewer_id: Option<window::Id>,
    inspector_id: Option<window::Id>,
}

impl Desktop {
    fn new() -> (Self, Task<Message>) {
        let (main_id, opened) = window::open(window::Settings {
            size: iced::Size::new(1440.0, 900.0),
            ..window::Settings::default()
        });
        (
            Self {
                state: initial_state(),
                main_id,
                viewer_id: None,
                inspector_id: None,
            },
            opened.map(|_| Message::ViewerOpened),
        )
    }

    fn open_viewer(&mut self) -> Task<Message> {
        if self.viewer_id.is_some() {
            return Task::none();
        }
        self.state.viewer_from_library = self.state.page == Some(Page::AssetLibrary);
        self.state.viewer_explorer_visible = true;
        let (id, opened) = window::open(window::Settings {
            size: iced::Size::new(1550.0, 940.0),
            ..window::Settings::default()
        });
        self.viewer_id = Some(id);
        opened.map(|_| Message::ViewerOpened)
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
            Message::OpenViewer => self.open_viewer(),
            Message::OpenInspectorWindow => self.open_inspector(),
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
                    if self.viewer_id == Some(id) {
                        self.viewer_id = None;
                        self.state.viewer_menu = None;
                    }
                    if self.inspector_id == Some(id) {
                        self.inspector_id = None;
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
                    Message::Finished(Ok(Completed::Model(_) | Completed::ImportedPakFiles(_)))
                ) || project_scan_done;
                let launched_from_library = self.state.page == Some(Page::AssetLibrary);
                let command = update(&mut self.state, message);
                if finished {
                    self.state.modding_open_editor_when_scanned = false;
                }
                if opens_model && self.viewer_id.is_none() {
                    let open = self.open_viewer();
                    self.state.viewer_from_library = launched_from_library;
                    Task::batch([command, open])
                } else {
                    command
                }
            }
        }
    }

    fn view(&self, id: window::Id) -> Element<'_, Message> {
        if self.viewer_id == Some(id) {
            asset_viewer::view(&self.state)
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
        if self.inspector_id == Some(id) {
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
        Some(if self.state.preferences.appearance_mode == "Light" {
            Theme::Light
        } else {
            Theme::custom(
                "Evolve Black/Red",
                iced::theme::Palette {
                    background: iced::Color::from_rgb8(32, 34, 39),
                    text: iced::Color::from_rgb8(234, 234, 236),
                    primary: iced::Color::from_rgb8(205, 52, 59),
                    success: iced::Color::from_rgb8(70, 170, 130),
                    warning: iced::Color::from_rgb8(233, 170, 80),
                    danger: iced::Color::from_rgb8(205, 52, 59),
                },
            )
        })
    }

    fn subscription(&self) -> Subscription<Message> {
        let playback = if self.state.playing {
            iced::time::every(Duration::from_millis(40)).map(|_| Message::Tick)
        } else {
            Subscription::none()
        };
        let scan_progress = if self.state.scan_indexed.is_some() {
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
    let _supported_backends = wgpu::Backends::PRIMARY;
    iced::daemon(Desktop::new, Desktop::update, Desktop::view)
        .subscription(Desktop::subscription)
        .title(Desktop::title)
        .theme(Desktop::theme)
        .run()
}
