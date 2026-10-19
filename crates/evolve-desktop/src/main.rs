//! Native wgpu-backed desktop shell. Asset operations use blocking workers, so
//! file I/O/decompression never runs in the interactive Iced view/update path.
mod textured;
mod viewport;
use evolve_core::{
    AssetEntry, CdfDefinition, DdsInfo, Inventory, LevelScene, ModelInfo, NativeCafMotion,
    PakInventory, RenderMesh, RigReport, SignedPakInfo, TerrainInfo, decode_dds_rgba,
    export_dds_png, export_level_scene_glb, export_rigged_character_fbx,
    export_rigged_character_glb, extract_legacy_pak, extract_signed_pak, inspect_dds,
    inspect_legacy_pak, inspect_signed_pak, load_level_scene, read_cdf, read_model,
    read_native_caf, read_terrain, rebuild_legacy_pak, rebuild_signed_pak_entry, scan_directory,
};
use iced::widget::{
    button, canvas, column, container, image, row, scrollable, shader, slider, svg, text,
    text_input, tooltip,
};
use iced::{Element, Fill, Length, Subscription, Task, Theme};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use textured::Scene as TexturedScene;
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
    diffuse_input: String,
    diffuse: Option<Arc<::image::RgbaImage>>,
    diffuse_status: String,
    caf_path: String,
    skeleton_path: String,
    blender_path: String,
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
    model: Option<Arc<RenderMesh>>,
    level_scene: Option<LevelScene>,
    terrain: Option<TerrainInfo>,
    selected: Option<usize>,
    status: String,
    busy: bool,
}

#[derive(Debug, Clone)]
enum Operation {
    Scan(PathBuf),
    InspectPak(PathBuf),
    InspectDds(PathBuf),
    LoadDiffuse(PathBuf),
    LoadCaf(PathBuf),
    RiggedGlb(PathBuf, PathBuf, PathBuf),
    RiggedFbx(PathBuf, PathBuf, PathBuf, PathBuf),
    InspectCdf(PathBuf),
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
    Caf(NativeCafMotion),
    RiggedExport(RigReport, PathBuf),
    Character(CdfDefinition),
    Model(RenderMesh),
    Level(LevelScene),
    Terrain(RenderMesh, TerrainInfo),
    SignedPak(SignedPakInfo),
    ExtractedSigned(usize, PathBuf),
    RebuiltSigned(String, PathBuf),
    LevelExported(usize, usize, PathBuf),
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
            Self::LoadCaf(source) => read_native_caf(&source)
                .map(Completed::Caf)
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
    InputChanged(String),
    OutputChanged(String),
    ReplacementsChanged(String),
    PublicKeyChanged(String),
    SignedNameChanged(String),
    SigningPrivateChanged(String),
    SigningPublicChanged(String),
    SignedMemberChanged(String),
    SignedReplacementChanged(String),
    DiffuseInputChanged(String),
    CafInputChanged(String),
    SkeletonChanged(String),
    BlenderChanged(String),
    MotionScrub(f32),
    ToggleMotion,
    Tick,
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
    StartCafLoad,
    StartRigGlb,
    StartRigFbx,
    StartCdfInspection,
    StartModelInspection,
    StartLevelInspection,
    StartTerrainInspection,
    StartSignedInspection,
    StartSignedExtraction,
    StartSignedRebuild,
    StartLevelGlbExport,
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

fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::Navigate(page) => state.page = Some(page),
        Message::InputChanged(input) => state.input = input,
        Message::OutputChanged(output) => state.output = output,
        Message::ReplacementsChanged(replacements) => state.replacements = replacements,
        Message::PublicKeyChanged(key) => state.public_key = key,
        Message::SignedNameChanged(name) => state.signed_name = name,
        Message::SigningPrivateChanged(value) => state.signing_private = value,
        Message::SigningPublicChanged(value) => state.signing_public = value,
        Message::SignedMemberChanged(value) => state.signed_entry = value,
        Message::SignedReplacementChanged(value) => state.signed_replacement = value,
        Message::DiffuseInputChanged(value) => state.diffuse_input = value,
        Message::CafInputChanged(value) => state.caf_path = value,
        Message::SkeletonChanged(value) => state.skeleton_path = value,
        Message::BlenderChanged(value) => state.blender_path = value,
        Message::MotionScrub(value) => state.motion_time = value,
        Message::ToggleMotion => state.playing = !state.playing && state.native_caf.is_some(),
        Message::Tick => {
            if state.playing {
                if let Some(motion) = &state.native_caf {
                    state.motion_time += 0.04;
                    let duration = motion.duration_seconds();
                    if duration <= 0.0 {
                        state.playing = false;
                    } else if state.motion_time >= duration {
                        if motion.cyclic {
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
        Message::StartCafLoad => {
            if state.caf_path.trim().is_empty() {
                state.status="Select the original native CAF to inspect/play its exact bone controller tracks.".into();
                return Task::none();
            }
            return launch(state, Operation::LoadCaf(PathBuf::from(&state.caf_path)));
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
                Ok(Completed::Character(character)) => {
                    state.status = format!(
                        "CDF: {} | {} validated skin/CHR attachment(s)",
                        character.skeleton,
                        character.attachments.len()
                    );
                    state.character = Some(character);
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
            text("ZIP-compatible 2012 PAKs can be inspected, extracted and rebuilt from exact replacements; signed Stage 2 CryPAKs are NOT supported yet."),
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
                text("GPU viewer draws real triangles and samples verified DDS diffuse with original UV0. Automatic material matching and skeletal skinning remain gated."),
                row![
                    text_input("Exact DDS diffuse (.dds or .dds.0) path...",&state.diffuse_input)
                        .on_input(Message::DiffuseInputChanged).width(Fill),
                    button("Load GPU diffuse").on_press(Message::StartDiffuseLoad),
                ].spacing(8),
                text(state.diffuse_status.as_str()),
                if let Some(mesh)=&state.model {
                    let display:Element<'_,Message> = if mesh.uv0.is_some(){
                        shader(TexturedScene{
                            mesh:mesh.clone(),diffuse:state.diffuse.clone(),
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
                button("Export complete geometry scene GLB").on_press(Message::StartLevelGlbExport),
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
            text("Scene GLB includes verified mesh instances and optional terrain. Texture/material, compiled vegetation, scripting and full map fidelity remain gated."),
        ].spacing(14).into(),
        Page::Settings => column![
            text("Workspace safety").size(19),
            text("The ZIP-compatible PAK extractor and rebuilder never overwrite existing destinations and stage outputs before publishing them."),
            text("Only a directory you explicitly enter can receive extraction files. Stage 2 signed PAK import/mod injection are disabled."),
            text("All expensive file operations run outside the UI thread."),
        ].spacing(14).into(),
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

fn main() -> iced::Result {
    let _supported_backends = wgpu::Backends::PRIMARY;
    iced::application(State::default, update, view)
        .subscription(|state: &State| {
            if state.playing {
                iced::time::every(Duration::from_millis(40)).map(|_| Message::Tick)
            } else {
                Subscription::none()
            }
        })
        .title("Evolve Mod Manager — Rust Development Preview")
        .theme(|_state: &State| {
            Theme::custom(
                "Evolve Black/Red",
                iced::theme::Palette {
                    background: iced::Color::from_rgb8(15, 15, 18),
                    text: iced::Color::from_rgb8(234, 234, 236),
                    primary: iced::Color::from_rgb8(205, 52, 59),
                    success: iced::Color::from_rgb8(70, 170, 130),
                    warning: iced::Color::from_rgb8(233, 170, 80),
                    danger: iced::Color::from_rgb8(205, 52, 59),
                },
            )
        })
        .run()
}
