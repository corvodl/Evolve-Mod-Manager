//! Native wgpu-backed desktop shell. Asset operations use blocking workers, so
//! file I/O/decompression never runs in the interactive Iced view/update path.
mod asset_viewer;
mod settings;
mod textured;
mod updater;
mod viewport;
use evolve_core::{
    AnimatedGlb, AnimationPolicy, AssetEntry, AssetResolution, BatchJournal, CdfDefinition,
    CdfDependencyReport, ConvertedMotion, DdsInfo, Inventory, LevelScene, MaterialReport,
    ModProfile, ModelInfo, NativeCafMotion, PakInventory, ProfileCatalog, ProfileMod, ProfilePlan,
    RenderMesh, RigReport, SignedPakInfo, StageTransaction, TerrainInfo, add_mod_to_profile,
    apply_profile_stage, apply_stage_pak, batch_stage_status, convert_glb_to_fbx,
    convert_original_motion, create_mod_profile, decode_dds_rgba, export_dds_png,
    export_level_scene_glb, export_rigged_character_fbx, export_rigged_character_glb,
    extract_legacy_pak, extract_signed_pak, import_stage_pak, initialize_stage, inspect_dds,
    inspect_legacy_pak, inspect_signed_pak, list_mod_profiles, load_level_scene, plan_mod_profile,
    read_cdf, read_material, read_model, read_native_caf, read_terrain, rebuild_legacy_pak,
    rebuild_signed_pak_entry, resolve_cdf, resolve_materials, restore_profile_stage, restore_stage,
    scan_directory, stage_status,
};
use iced::widget::{
    button, canvas, column, container, image, row, scrollable, shader, slider, svg, text,
    text_input, tooltip,
};
use iced::{Element, Fill, Length, Subscription, Task, Theme, window};
use settings::{Preferences, load_from, save_to, settings_path};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use textured::Scene as TexturedScene;
use updater::{
    ReleaseUpdate, UpdateChannel, discover_update, download_installer, launch_verified_installer,
};
use viewport::MeshViewport;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Page {
    Home,
    Files,
    AssetLibrary,
    Models,
    Levels,
    Settings,
}

impl Page {
    const ALL: [Self; 6] = [
        Self::Home,
        Self::Files,
        Self::AssetLibrary,
        Self::Models,
        Self::Levels,
        Self::Settings,
    ];

    fn icon(self) -> svg::Handle {
        let data: &'static [u8] = match self {
            Self::Home => include_bytes!("../../../assets/nav/home.svg"),
            Self::Files => include_bytes!("../../../assets/nav/folder.svg"),
            Self::AssetLibrary => include_bytes!("../../../assets/nav/layers.svg"),
            Self::Models => include_bytes!("../../../assets/nav/box.svg"),
            Self::Levels => include_bytes!("../../../assets/nav/map.svg"),
            Self::Settings => include_bytes!("../../../assets/nav/settings.svg"),
        };
        svg::Handle::from_memory(data.to_vec())
    }

    fn title(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Files => "Modding",
            Self::AssetLibrary => "Asset Library",
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
    public_key: String,
    signed_name: String,
    signing_private: String,
    signing_public: String,
    signed_entry: String,
    signed_replacement: String,
    stage_target: String,
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
    texture: Option<DdsInfo>,
    character: Option<CdfDefinition>,
    cdf_dependencies: Option<CdfDependencyReport>,
    model: Option<Arc<RenderMesh>>,
    level_scene: Option<LevelScene>,
    terrain: Option<TerrainInfo>,
    selected: Option<usize>,
    status: String,
    busy: bool,
    preferences: Preferences,
    settings_status: String,
    update_status: String,
    available_update: Option<ReleaseUpdate>,
    staged_installer: Option<PathBuf>,
    startup_update_checked: bool,
    viewer_search: String,
    viewer_menu: Option<&'static str>,
    viewer_explorer_visible: bool,
    viewer_from_library: bool,
}

#[derive(Debug, Clone)]
enum Operation {
    Scan(PathBuf),
    InspectPak(PathBuf),
    InspectDds(PathBuf),
    LoadDiffuse(PathBuf),
    LoadNormal(PathBuf),
    LoadEmissive(PathBuf),
    ResolveMaterial(PathBuf, PathBuf),
    LoadCaf(PathBuf),
    RiggedGlb(PathBuf, PathBuf, PathBuf),
    RiggedFbx(PathBuf, PathBuf, PathBuf, PathBuf),
    ConvertMotion(PathBuf, PathBuf, PathBuf, PathBuf),
    LoadGlbMotion(PathBuf),
    InspectCdf(PathBuf),
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
    Inventory(Inventory),
    Pak(PakInventory),
    Texture(DdsInfo),
    Diffuse(DdsInfo, ::image::RgbaImage),
    Normal(DdsInfo, ::image::RgbaImage),
    Emissive(DdsInfo, ::image::RgbaImage),
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
            Self::Scan(source) => scan_directory(&source)
                .map(Completed::Inventory)
                .map_err(|e| e.to_string()),
            Self::InspectPak(source) => inspect_legacy_pak(&source)
                .map(Completed::Pak)
                .map_err(|e| e.to_string()),
            Self::RiggedGlb(mesh, skeleton, dest) => {
                export_rigged_character_glb(&mesh, &skeleton, &dest)
                    .map(|r| Completed::RiggedExport(r, dest))
                    .map_err(|e| e.to_string())
            }
            Self::RiggedFbx(mesh, skeleton, blender, dest) => {
                export_rigged_character_fbx(&mesh, &skeleton, &blender, &dest)
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
            Self::LoadNormal(source) => decode_dds_rgba(&source)
                .map(|(info, image)| Completed::Normal(info, image))
                .map_err(|e| e.to_string()),
            Self::LoadEmissive(source) => decode_dds_rgba(&source)
                .map(|(info, image)| Completed::Emissive(info, image))
                .map_err(|e| e.to_string()),
            Self::LoadDiffuse(source) => decode_dds_rgba(&source)
                .map(|(info, image)| Completed::Diffuse(info, image))
                .map_err(|e| e.to_string()),
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
    OpenViewer,
    ViewerOpened(window::Id),
    WindowClosed(window::Id),
    ViewerMenu(&'static str),
    ViewerToggleExplorer,
    ViewerResetCamera,
    ViewerSearchChanged(String),
    ViewerPreviewSelected,
    InputChanged(String),
    OutputChanged(String),
    ReplacementsChanged(String),
    PublicKeyChanged(String),
    SignedNameChanged(String),
    SigningPrivateChanged(String),
    SigningPublicChanged(String),
    SignedMemberChanged(String),
    SignedReplacementChanged(String),
    StageTargetChanged(String),
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
    Select(usize),
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

fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::Navigate(page) => state.page = Some(page),
        Message::OpenViewer | Message::ViewerOpened(_) | Message::WindowClosed(_) => {},
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
        Message::ViewerResetCamera => {
            state.yaw = 0.0;
            state.pitch = 0.0;
            state.zoom = 1.0;
        }
        Message::ViewerSearchChanged(filter) => state.viewer_search = filter,
        Message::ViewerPreviewSelected => {
            let Some(inventory) = &state.inventory else {
                state.status = "Extract the PAK into an isolated folder and index it to preview files.".into();
                return Task::none();
            };
            let Some(relative) = state.selected.and_then(|index| inventory.entries.get(index)) else {
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
                state.status = "Selected file is missing or escaped the indexed root; rescan before preview.".into();
                return Task::none();
            }
            state.input = source.to_string_lossy().to_string();
            let filename = relative.relative_path.to_ascii_lowercase();
            let operation = if [".skin", ".skinm", ".chr", ".cgf"].iter().any(|s| filename.ends_with(s)) {
                Operation::InspectModel(source)
            } else if filename.ends_with(".cdf") {
                Operation::InspectCdf(source)
            } else if filename.ends_with(".dds") || filename.ends_with(".dds.0") {
                Operation::InspectDds(source)
            } else if filename.ends_with(".caf") {
                Operation::LoadCaf(source)
            } else if filename.ends_with(".mtl") {
                Operation::ResolveMaterial(source, PathBuf::from(&inventory.root))
            } else {
                state.status = "Selected asset type is not supported by the native preview yet.".into();
                return Task::none();
            };
            return launch(state, operation);
        },
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
        Message::PreferenceStage(value) => state.preferences.stage = value,
        Message::PreferenceSwap(value) => state.preferences.swap = value,
        Message::PreferenceProjects(value) => state.preferences.projects = value,
        Message::PreferenceAssets(value) => state.preferences.asset_library_root = value,
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
        Message::PublicKeyChanged(key) => state.public_key = key,
        Message::SignedNameChanged(name) => state.signed_name = name,
        Message::SigningPrivateChanged(value) => state.signing_private = value,
        Message::SigningPublicChanged(value) => state.signing_public = value,
        Message::SignedMemberChanged(value) => state.signed_entry = value,
        Message::SignedReplacementChanged(value) => state.signed_replacement = value,
        Message::StageTargetChanged(value) => state.stage_target = value,
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
            if [
                &state.preferences.stage,
                &state.profile_id,
                &state.public_key,
                &state.signing_public,
            ]
            .iter()
            .any(|v| v.trim().is_empty())
            {
                state.status =
                    "Choose profile ID, isolated stage and original/new public signing keys."
                        .into();
                return Task::none();
            }
            return launch(
                state,
                Operation::ApplyProfile(
                    PathBuf::from(&state.preferences.stage),
                    state.profile_id.clone(),
                    PathBuf::from(&state.public_key),
                    PathBuf::from(&state.signing_public),
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
            if [
                &state.preferences.stage,
                &state.profile_id,
                &state.input,
                &state.stage_target,
                &state.public_key,
                &state.signing_public,
                &state.signed_name,
            ]
            .iter()
            .any(|v| v.trim().is_empty())
            {
                state.status = "Select a profile, signed replacement PAK, relative stage target, original/mod public keys, and basename.".into();
                return Task::none();
            }
            return launch(
                state,
                Operation::AddProfileMod(
                    PathBuf::from(&state.preferences.stage),
                    state.profile_id.clone(),
                    PathBuf::from(&state.input),
                    state.stage_target.clone(),
                    PathBuf::from(&state.public_key),
                    PathBuf::from(&state.signing_public),
                    state.signed_name.clone(),
                ),
            );
        }
        Message::StartProfilePlan => {
            if [
                &state.preferences.stage,
                &state.profile_id,
                &state.public_key,
                &state.signing_public,
            ]
            .iter()
            .any(|v| v.trim().is_empty())
            {
                state.status =
                    "Select a profile and both public signing keys for a verified read-only plan."
                        .into();
                return Task::none();
            }
            return launch(
                state,
                Operation::PlanProfile(
                    PathBuf::from(&state.preferences.stage),
                    state.profile_id.clone(),
                    PathBuf::from(&state.public_key),
                    PathBuf::from(&state.signing_public),
                ),
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
            if [
                &state.preferences.stage,
                &state.input,
                &state.stage_target,
                &state.public_key,
                &state.signed_name,
            ]
            .iter()
            .any(|x| x.trim().is_empty())
            {
                state.status = "Specify stage root, original PAK, relative target, public key, and signed basename.".into();
                return Task::none();
            }
            return launch(
                state,
                Operation::StageImport(
                    PathBuf::from(&state.preferences.stage),
                    PathBuf::from(&state.input),
                    state.stage_target.clone(),
                    PathBuf::from(&state.public_key),
                    state.signed_name.clone(),
                ),
            );
        }
        Message::StartStageApply => {
            if [
                &state.preferences.stage,
                &state.input,
                &state.stage_target,
                &state.public_key,
                &state.signing_public,
                &state.signed_name,
            ]
            .iter()
            .any(|x| x.trim().is_empty())
            {
                state.status = "Specify stage, replacement PAK path, relative target, original and new public keys, and signed basename.".into();
                return Task::none();
            }
            return launch(
                state,
                Operation::StageApply(
                    PathBuf::from(&state.preferences.stage),
                    PathBuf::from(&state.input),
                    state.stage_target.clone(),
                    PathBuf::from(&state.public_key),
                    PathBuf::from(&state.signing_public),
                    state.signed_name.clone(),
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
                    state.motion_time += 0.04;
                    if duration <= 0.0 {
                        state.playing = false;
                    } else if state.motion_time >= duration {
                        if looping {
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
        Message::Select(index) => state.selected = Some(index),
        Message::StartScan => {
            let source = PathBuf::from(&state.input);
            return launch(state, Operation::Scan(source));
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
                Operation::LoadNormal(PathBuf::from(&state.normal_input)),
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
                Operation::LoadEmissive(PathBuf::from(&state.emissive_input)),
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
                Operation::LoadDiffuse(PathBuf::from(&state.diffuse_input)),
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
            if state.public_key.is_empty() || state.signed_name.is_empty() {
                state.status =
                    "Select a local RSA public key and exact original signed basename.".into();
                return Task::none();
            }
            let source = PathBuf::from(&state.input);
            let key = PathBuf::from(&state.public_key);
            let signed = state.signed_name.clone();
            return launch(state, Operation::InspectSignedPak(source, key, signed));
        }
        Message::StartSignedExtraction => {
            if state.public_key.is_empty()
                || state.signed_name.is_empty()
                || state.output.is_empty()
            {
                state.status = "Select public key, signed basename, and new output folder.".into();
                return Task::none();
            }
            let source = PathBuf::from(&state.input);
            let key = PathBuf::from(&state.public_key);
            let signed = state.signed_name.clone();
            let output = PathBuf::from(&state.output);
            return launch(
                state,
                Operation::ExtractSignedPak(source, key, signed, output),
            );
        }
        Message::StartSignedRebuild => {
            if [
                &state.input,
                &state.public_key,
                &state.signing_private,
                &state.signing_public,
                &state.signed_name,
                &state.signed_entry,
                &state.signed_replacement,
                &state.output,
            ]
            .iter()
            .any(|v| v.trim().is_empty())
            {
                state.status="Choose the source PAK, both public keys, local private key, exact entry, replacement, and NEW output PAK.".into();
                return Task::none();
            }
            return launch(
                state,
                Operation::RebuildSignedPak(
                    PathBuf::from(&state.input),
                    PathBuf::from(&state.public_key),
                    PathBuf::from(&state.signing_private),
                    PathBuf::from(&state.signing_public),
                    state.signed_name.clone(),
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
            match result {
                Ok(Completed::Inventory(inventory)) => {
                    state.status = format!(
                        "{} files indexed; no originals modified.",
                        inventory.entries.len()
                    );
                    state.inventory = Some(inventory);
                    state.pak = None;
                    state.texture = None;
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
                    state.normal_map = Some(Arc::new(pixels));
                    state.page = Some(Page::Models);
                }
                Ok(Completed::Emissive(info, pixels)) => {
                    state.status = format!(
                        "GPU emissive map loaded: {} × {} ({:?}). MTL GlowAmount is not inferred.",
                        info.width, info.height, info.format
                    );
                    state.emissive_map = Some(Arc::new(pixels));
                    state.page = Some(Page::Models);
                }
                Ok(Completed::Diffuse(info, pixels)) => {
                    state.diffuse_status = format!(
                        "GPU diffuse loaded: {} × {} highest-resolution pixels ({:?}).",
                        info.width, info.height, info.format
                    );
                    state.status = state.diffuse_status.clone();
                    state.diffuse = Some(Arc::new(pixels));
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
                    state.level_scene = Some(scene);
                    state.page = Some(Page::Levels);
                }
                Ok(Completed::SignedPak(info)) => {
                    state.status = format!(
                        "Verified signed CryPAK: {} records, {} expanded bytes.",
                        info.entries.len(),
                        info.total_expanded_bytes
                    );
                }
                Ok(Completed::RebuiltSigned(name, dir)) => {
                    state.status = format!(
                        "Rebuilt and verified signed PAK entry {} into {}. Source unchanged; offline test required.",
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

fn file_row<'a>(index: usize, entry: &'a AssetEntry) -> Element<'a, Message> {
    button(
        row![
            text(entry.kind.display()).width(130),
            text(entry.relative_path.as_str()).width(Fill),
            text(format!("{} B", entry.byte_size)),
        ]
        .spacing(12),
    )
    .width(Fill)
    .on_press(Message::Select(index))
    .into()
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
    // Preserve the original compact icon-only sidebar and hover labels.
    // Red selection is only applied to the active workspace; there is no
    // hamburger button or whole-sidebar hover expansion.
    let navigation = Page::ALL.into_iter().fold(
        column![
            image(image::Handle::from_bytes(
                include_bytes!("../../../assets/hunt.png").to_vec()
            ))
            .width(45)
            .height(45),
        ]
        .spacing(16),
        |items, item| {
            let active = page == item;
            let nav_button = button(svg(item.icon()).width(24).height(24))
                .width(50)
                .height(48)
                .padding(12)
                .style(if active {
                    button::danger
                } else {
                    button::secondary
                })
                .on_press(Message::Navigate(item));
            items.push(tooltip(
                nav_button,
                text(item.title()),
                tooltip::Position::Right,
            ))
        },
    );
    let sidebar = container(navigation.padding(10).spacing(11)).width(72);
    let headline = row![
        image(image::Handle::from_bytes(
            include_bytes!("../../../assets/hunt.png").to_vec()
        ))
        .width(46)
        .height(46),
        column![
            text("EVOLVE / MOD MANAGER").size(22),
            text(format!(
                "{}  •  Rust / Iced / wgpu development preview",
                page.title()
            ))
            .size(12),
        ]
        .spacing(5),
    ]
    .spacing(14);

    let location = column![
        row![
            text_input("Local asset folder, .pak or .dds path...", &state.input)
                .on_input(Message::InputChanged)
                .width(Fill),
            button("Scan folder").on_press(Message::StartScan),
            button("Open Asset Viewer").on_press(Message::OpenViewer),
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
            text_input(
                "Local signed archive RSA public DER key...",
                &state.public_key
            )
            .on_input(Message::PublicKeyChanged)
            .width(Fill),
            text_input(
                "Original signed basename (e.g. libs.pak)...",
                &state.signed_name
            )
            .on_input(Message::SignedNameChanged)
            .width(220),
            button("Verify Stage 2 PAK").on_press(Message::StartSignedInspection),
            button("Extract signed PAK").on_press(Message::StartSignedExtraction),
        ]
        .spacing(8),
    ]
    .spacing(9);

    let workspace: Element<'_, Message> = match page {
        Page::Home => column![
            text("Inspect local CryEngine assets safely.").size(19),
            text("Legacy ZIP and signed Stage 2 PAKs support bounded inspection and extraction; signed rebuilding and isolated offline PAK staging are experimental."),
            text("DDS top-mipmap PNG export supports BC1/2/3/4/5 and 32-bit RGB(A); complete split-stream sets are reconstructed first."),
            text("Signed archive rebuilding and Blender geometry-scene GLB export are experimental. Full character FBX, game sign-key acceptance and animation/texture parity remain gated."),
        ].spacing(14).into(),
        Page::Files | Page::AssetLibrary => {
            let mut rows = column![].spacing(3);
            let needle = state.search.to_lowercase();
            let selected_name;
            if let Some(pak) = &state.pak {
                for (index, entry) in pak.entries.iter().enumerate()
                    .filter(|(_, e)| e.path.to_lowercase().contains(&needle))
                    .take(350)
                {
                    rows = rows.push(button(row![
                        text(entry.path.as_str()).width(Fill),
                        text(format!("{} B", entry.size)),
                    ].spacing(8)).width(Fill).on_press(Message::Select(index)));
                }
                selected_name = state.selected
                    .and_then(|n| pak.entries.get(n))
                    .map(|e| format!("Member: {} / {} bytes / ZIP method {}", e.path, e.size, e.method))
                    .unwrap_or_else(|| format!("Archive: {}", pak.source_file));
            } else if let Some(inventory) = &state.inventory {
                for (index, entry) in inventory.entries.iter().enumerate()
                    .filter(|(_, e)| e.relative_path.to_lowercase().contains(&needle))
                    .take(350)
                {
                    rows = rows.push(file_row(index, entry));
                }
                selected_name = state.selected
                    .and_then(|n| inventory.entries.get(n))
                    .map(|e| format!("Asset: {} / {} bytes", e.relative_path, e.byte_size))
                    .unwrap_or_else(|| format!("Local directory: {}", inventory.root));
            } else {
                selected_name = "Scan a folder or inspect a ZIP-compatible PAK to see files.".to_owned();
            }
            column![
                text_input("Search indexed filenames...", &state.search)
                    .on_input(Message::SearchChanged),
                text(selected_name).size(14),
                scrollable(rows).height(Fill),
                text("Showing up to 350 matching rows; data remains available for filtering.").size(12),
                text("Signed CryPAK rebuild (offline/private testing)").size(18),
                row![
                    text_input("Private PEM key (local only)...",&state.signing_private)
                        .on_input(Message::SigningPrivateChanged).width(Fill),
                    text_input("Matching public DER key...",&state.signing_public)
                        .on_input(Message::SigningPublicChanged).width(Fill),
                ].spacing(8),
                row![
                    text_input("Exact signed member virtual path...",&state.signed_entry)
                        .on_input(Message::SignedMemberChanged).width(Fill),
                    text_input("Replacement asset path...",&state.signed_replacement)
                        .on_input(Message::SignedReplacementChanged).width(Fill),
                    button("Rebuild signed PAK").on_press(Message::StartSignedRebuild),
                ].spacing(8),
                text("ISOLATED OFFLINE PAK STAGING / RECOVERY").size(18),
                text("Use a NEW empty workspace. This never swaps installed EvolveGame files. Import a verified original, then select a signed modded PAK in the main input field to apply."),
                row![
                    text_input("NEW isolated stage folder (not game directory)...", &state.preferences.stage)
                        .on_input(Message::PreferenceStage).width(Fill),
                    button("Initialize stage").on_press(Message::StartStageInit),
                    button("Check stage").on_press(Message::StartStageStatus),
                    button("Restore backup").on_press(Message::StartStageRestore),
                ].spacing(8),
                row![
                    text_input("Stage target relative to paks/ (e.g. Game/libs.pak)", &state.stage_target)
                        .on_input(Message::StageTargetChanged).width(Fill),
                    button("Import signed original").on_press(Message::StartStageImport),
                    button("Apply signed replacement").on_press(Message::StartStageApply),
                ].spacing(8),
                text("Apply uses the PAK file in the main input and the original/public signing keys above. Restoring validates all original backup hashes."),
                text(state.stage_feedback.as_str()).size(13),
                text("MOD PROFILES — verified offline catalog (not game installation)").size(18),
                row![
                    text_input("New profile name...", &state.profile_name)
                        .on_input(Message::ProfileNameChanged).width(Fill),
                    button("Create profile").on_press(Message::StartProfileCreate),
                    button("Refresh profiles").on_press(Message::StartProfileList),
                ].spacing(8),
                if let Some(catalog) = &state.profile_catalog {
                    let mut rows = column![].spacing(4);
                    for profile in catalog.profiles.iter().take(100) {
                        rows = rows.push(
                            button(text(format!("{} ({} mod(s))", profile.name, profile.mods.len())))
                                .on_press(Message::SelectProfile(profile.id.clone()))
                        );
                    }
                    scrollable(rows).height(Length::Fixed(145.))
                } else {
                    scrollable(column![text("Refresh to browse available local profiles.")])
                        .height(Length::Fixed(70.))
                },
                row![
                    text_input("Selected profile ID...", &state.profile_id)
                        .on_input(Message::SelectProfile).width(Fill),
                    button("Add signed PAK").on_press(Message::StartProfileAdd),
                    button("Verify profile").on_press(Message::StartProfilePlan),
                ].spacing(8),
                row![
                    button("Apply to offline stage").on_press(Message::StartProfileApply),
                    button("Check batch recovery").on_press(Message::StartProfileStatus),
                    button("Restore staged profile").on_press(Message::StartProfileRecover),
                ].spacing(8),
                text("Profile add uses the signed replacement path, relative PAK target, both public DER keys and basename above. No profile is installed automatically."),
                text(state.profile_feedback.as_str()).size(13),
            ].spacing(10).into()
        }
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
                    text("Uses new output path above. No embedded animations without validated matching tracks."),
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

    let workspace: Element<'_, Message> = if page == Page::Settings {
        scrollable(workspace).height(Fill).into()
    } else {
        workspace
    };
    let body = column![
        headline,
        location,
        text(if state.busy {
            "Working..."
        } else {
            state.status.as_str()
        })
        .size(13),
        workspace,
    ]
    .spacing(16)
    .padding(20)
    .width(Fill)
    .height(Fill);
    container(row![sidebar, body].height(Fill))
        .width(Fill)
        .height(Fill)
        .into()
}

fn initial_state() -> State {
    let mut result = State {
        zoom: 1.0,
        viewer_explorer_visible: true,
        ..State::default()
    };
    if let Ok(path) = settings_path() {
        match load_from(&path) {
            Ok(saved) => {
                result.blender_path = saved.blender_executable.clone();
                result.assets_root = saved.asset_library_root.clone();
                result.preferences = saved;
            }
            Err(error) => {
                result.settings_status = format!(
                    "Could not read preferences; original file was not overwritten: {error}"
                );
            }
        }
    }
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
}

impl Desktop {
    fn new() -> (Self, Task<Message>) {
        let (main_id, opened) = window::open(window::Settings {
            size: iced::Size::new(1440.0, 900.0),
            ..window::Settings::default()
        });
        (
            Self { state: initial_state(), main_id, viewer_id: None },
            opened.map(Message::ViewerOpened),
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
        opened.map(Message::ViewerOpened)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::OpenViewer => self.open_viewer(),
            Message::ViewerOpened(_) => Task::none(),
            Message::WindowClosed(id) => {
                if id == self.main_id {
                    iced::exit()
                } else {
                    if self.viewer_id == Some(id) {
                        self.viewer_id = None;
                        self.state.viewer_menu = None;
                    }
                    Task::none()
                }
            }
            message => {
                let opens_model = matches!(&message, Message::Finished(Ok(Completed::Model(_))));
                let command = update(&mut self.state, message);
                if opens_model && self.viewer_id.is_none() {
                    Task::batch([command, self.open_viewer()])
                } else {
                    command
                }
            }
        }
    }

    fn view(&self, id: window::Id) -> Element<'_, Message> {
        if self.viewer_id == Some(id) {
            asset_viewer::view(&self.state)
        } else if id == self.main_id {
            view(&self.state)
        } else {
            container(text("Window closed")).into()
        }
    }

    fn title(&self, id: window::Id) -> String {
        if self.viewer_id == Some(id) {
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
        let startup = if self.state.preferences.automatic_update_checks
            && !self.state.startup_update_checked {
            iced::time::every(Duration::from_millis(500)).map(|_| Message::CheckUpdates)
        } else {
            Subscription::none()
        };
        Subscription::batch([
            playback,
            startup,
            window::close_events().map(Message::WindowClosed),
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
