//! Searchable gameplay forms and model shortcuts over editable project manifests.
use super::{Message, State};
use crate::{
    design,
    modding_projects::{self, ProjectRow},
};
use evolve_core::gameplay_values::{self, SourceField, ValueField};
use evolve_core::lua51;
use iced::widget::{Id, column, row, scrollable, text};
use iced::{Element, Fill, Task};
use std::path::Path;
use std::sync::Arc;

#[derive(Default)]
pub struct Editor {
    pub open: bool,
    pub models: bool,
    target: Option<Target>,
    build: Option<String>,
    imports: Vec<(Arc<evolve_core::library_catalog::ImportedPak>, String)>,
    search: String,
    fields_search: String,
    fields_page: usize,
    imports_page: usize,
    rows: Vec<(Arc<ProjectRow>, String)>,
    selected: Option<usize>,
    snapshot: Vec<u8>,
    fields: Vec<ValueField>,
    preview: Option<(usize, Vec<SourceField>)>,
    preview_request: u64,
    links_request: u64,
    links_status: LinkStatus,
    appearance: VerifiedAppearance,
    note: String,
}
#[derive(Default)]
enum LinkStatus {
    #[default]
    Idle,
    Pending,
    Ready(usize),
    Failed(String),
}
#[derive(Default, Debug, Clone)]
pub struct VerifiedAppearance {
    models: Vec<(evolve_core::ImportedPak, String)>,
    materials: Vec<(evolve_core::ImportedPak, String)>,
    textures: Vec<(evolve_core::ImportedPak, String)>,
}
#[derive(Debug, Clone)]
pub enum Target {
    Character(String),
    Wildlife(String),
}
impl Target {
    fn name(&self) -> &str {
        match self {
            Self::Character(name) | Self::Wildlife(name) => name,
        }
    }
    fn owns_document(&self, path: &str) -> bool {
        match self {
            Self::Character(name) => evolve_core::character_gameplay::owns_document(name, path),
            Self::Wildlife(name) => {
                evolve_core::character_gameplay::owns_wildlife_document(name, path)
            }
        }
    }
    fn owns_global(&self, path: &str, global: &str) -> bool {
        use evolve_core::character_gameplay::{RosterEntry, script_global_owner};
        match (self, script_global_owner(path, global)) {
            (Self::Character(name), Some(RosterEntry::Character(owner)))
            | (Self::Wildlife(name), Some(RosterEntry::Wildlife(owner))) => name == owner,
            _ => false,
        }
    }
}

const SCRIPT_LIMIT: usize = 4 * 1024 * 1024;

/// A compiled actor script whose literal tables were read without running it.
pub(crate) struct DecodedScript {
    pub archive: evolve_core::ImportedPak,
    pub path: String,
    pub globals: Vec<(String, lua51::Table)>,
}

/// Decode the Lua 5.1 actor scripts in `archives`. Plain-text 2012 scripts
/// and Stage 2's custom bytecode are not Lua 5.1 and are skipped here.
pub(crate) fn decoded_scripts(
    archives: &[evolve_core::ImportedPak],
) -> Result<Vec<DecodedScript>, String> {
    let mut scripts = Vec::new();
    for archive in archives {
        for path in modding_projects::imported_actor_scripts(archive)? {
            let bytes = evolve_core::library_catalog::read_imported_member_bounded(
                archive,
                &path,
                SCRIPT_LIMIT,
            )
            .map_err(|error| error.to_string())?;
            if !lua51::is_bytecode(&bytes) {
                continue;
            }
            let globals =
                lua51::authored_globals(&bytes).map_err(|error| format!("{path}: {error}"))?;
            scripts.push(DecodedScript {
                archive: archive.clone(),
                path,
                globals,
            });
        }
    }
    Ok(scripts)
}

#[derive(Default)]
struct ScriptSources {
    /// Scripts assigning at least one global the target owns.
    owned: Vec<(evolve_core::ImportedPak, String)>,
    /// Scripts defining a `baseType` the target inherits from.
    bases: Vec<(evolve_core::ImportedPak, String)>,
    references: Vec<evolve_core::character_gameplay::DocumentReference>,
}

/// Follow the target's own globals and their authored `baseType` chain.
/// A base resolves in the same script first, else to the single script in
/// the build that defines it; a name defined in several scripts stays unlinked.
fn script_sources(target: &Target, scripts: &[DecodedScript]) -> Result<ScriptSources, String> {
    let mut sources = ScriptSources::default();
    let mut pending = Vec::new();
    for (index, script) in scripts.iter().enumerate() {
        let mut owned = false;
        for (global, _) in &script.globals {
            if target.owns_global(&script.path, global) {
                pending.push((index, global.clone()));
                owned = true;
            }
        }
        if owned {
            sources
                .owned
                .push((script.archive.clone(), script.path.clone()));
        }
    }
    let defines = |index: usize, name: &str| {
        scripts[index]
            .globals
            .iter()
            .any(|(global, _)| global == name)
    };
    let mut visited = std::collections::HashSet::new();
    let mut base_files = std::collections::BTreeSet::new();
    while let Some((index, global)) = pending.pop() {
        if !visited.insert((index, global.clone())) {
            continue;
        }
        if visited.len() > 512 {
            return Err("Script inheritance chain is too long".into());
        }
        let Some((_, table)) = scripts[index]
            .globals
            .iter()
            .find(|(name, _)| *name == global)
        else {
            continue;
        };
        sources
            .references
            .extend(evolve_core::character_gameplay::script_references(table));
        let Some(lua51::Value::Global(base)) = table.get("baseType") else {
            continue;
        };
        let base_index = if defines(index, base) {
            Some(index)
        } else {
            let mut found = (0..scripts.len()).filter(|&other| defines(other, base));
            match (found.next(), found.next()) {
                (Some(other), None) => Some(other),
                _ => None,
            }
        };
        if let Some(base_index) = base_index {
            pending.push((base_index, base.clone()));
            base_files.insert(base_index);
        }
    }
    for index in base_files {
        let script = &scripts[index];
        if !sources
            .owned
            .iter()
            .any(|(archive, path)| archive.id == script.archive.id && *path == script.path)
        {
            sources
                .bases
                .push((script.archive.clone(), script.path.clone()));
        }
    }
    Ok(sources)
}
#[derive(Debug, Clone)]
pub enum Action {
    Open(bool),
    OpenTarget {
        models: bool,
        target: Target,
        build: String,
    },
    Close,
    Search(String),
    FieldSearch(String),
    FieldsPage(bool),
    ImportsPage(bool),
    Select(usize),
    Value(usize, String),
    Save,
    Prepare(usize),
    PreviewImport(usize),
    PreviewReady(u64, usize, Result<Vec<SourceField>, String>),
    LinksReady(u64, Result<Vec<(evolve_core::ImportedPak, String)>, String>),
    ModelsReady(u64, Result<VerifiedAppearance, String>),
    Discard,
    Refresh,
}
fn message(action: Action) -> Message {
    Message::GuidedEdit(action)
}
fn search_id() -> Id {
    Id::new("guided-document-search")
}

fn preview_imported_values(
    archive: &evolve_core::ImportedPak,
    path: &str,
    target: Option<&Target>,
) -> Result<Vec<SourceField>, String> {
    if evolve_core::character_gameplay::actor_script(path) {
        if !modding_projects::imported_actor_scripts(archive)?
            .iter()
            .any(|candidate| candidate == path)
        {
            return Err("Imported source changed since discovery".into());
        }
        let bytes =
            evolve_core::library_catalog::read_imported_member_bounded(archive, path, SCRIPT_LIMIT)
                .map_err(|error| error.to_string())?;
        if !lua51::is_bytecode(&bytes) {
            return Err("Only compiled Lua 5.1 scripts can be previewed".into());
        }
        let globals = lua51::authored_globals(&bytes)?;
        // A shared script (Goliath's also defines Meteor Goliath) shows only
        // the target's own tables; a base script shows everything it defines.
        let owns = |global: &str| target.is_some_and(|target| target.owns_global(path, global));
        let owned = globals.iter().any(|(global, _)| owns(global));
        let mut flat = Vec::new();
        for (global, table) in &globals {
            if !owned || owns(global) {
                table.flatten(global, &mut flat);
            }
        }
        return Ok(flat
            .into_iter()
            .map(|(label, value)| SourceField { label, value })
            .collect());
    }
    // The source identity is checked again, not just when the list was built.
    if !modding_projects::guided_import_members(archive, false)?
        .iter()
        .any(|candidate| candidate == path)
    {
        return Err("Imported source changed since discovery".into());
    }
    let bytes = evolve_core::library_catalog::read_imported_gameplay_member(
        archive,
        path,
        gameplay_values::LIMIT,
    )
    .map_err(|error| error.to_string())?;
    gameplay_values::source_fields(&bytes)
}

/// `seed` carries references already read from the target's scripts.
fn actor_linked_imports(
    direct: &[(Arc<evolve_core::ImportedPak>, String)],
    archives: &[evolve_core::ImportedPak],
    build: &str,
    seed: Vec<evolve_core::character_gameplay::DocumentReference>,
) -> Result<Vec<(evolve_core::ImportedPak, String)>, String> {
    let mut imports = Vec::new();
    for archive in archives
        .iter()
        .filter(|archive| archive.builds.iter().any(|label| label == build))
    {
        let paths = modding_projects::guided_import_members(archive, false)?;
        for path in paths {
            imports.push((archive.clone(), path));
        }
    }
    let candidates: Vec<_> = imports
        .iter()
        .map(|(archive, path)| (archive.id.clone(), path.clone()))
        .collect();
    use evolve_core::character_gameplay::{self as gameplay, ReferenceKind};
    let read = |archive: &evolve_core::ImportedPak, path: &str| {
        evolve_core::library_catalog::read_imported_gameplay_member(
            archive,
            path,
            gameplay_values::LIMIT,
        )
        .map_err(|error| error.to_string())
    };
    let matching = |reference: &gameplay::DocumentReference| -> Vec<usize> {
        candidates
            .iter()
            .enumerate()
            .filter(|(_, (_, path))| gameplay::referenced_document(reference, path))
            .map(|(index, _)| index)
            .collect()
    };
    let mut actors = Vec::new();
    for (archive, path) in direct {
        if !archive.builds.iter().any(|label| label == build) {
            return Err("Actor source no longer belongs to the selected build".into());
        }
        if path.to_ascii_lowercase().ends_with(".actorparams") {
            actors.push((archive.as_ref().clone(), path.clone()));
        }
    }
    // Inherited parameters apply to the character, so each base actor's own
    // equipment and abilities are followed too.
    let mut references = seed;
    let mut actor_links = Vec::new();
    let mut visited = std::collections::HashSet::new();
    while let Some((archive, path)) = actors.pop() {
        if !visited.insert((archive.id.clone(), path.clone())) {
            continue;
        }
        if visited.len() > 64 {
            return Err("Actor inheritance chain is too long".into());
        }
        for reference in gameplay::actor_references(&read(&archive, &path)?)? {
            if reference.kind != ReferenceKind::ActorBase {
                references.push(reference);
                continue;
            }
            let base = match matching(&reference).as_slice() {
                [index] => Some(*index),
                [] => match gameplay::leveled_base(&reference) {
                    Some((actor, level)) => match matching(&actor).as_slice() {
                        [index]
                            if gameplay::actor_levels(&read(
                                &imports[*index].0,
                                &imports[*index].1,
                            )?)?
                            .is_some_and(|count| level <= count) =>
                        {
                            Some(*index)
                        }
                        _ => None,
                    },
                    None => None,
                },
                _ => None,
            };
            if let Some(index) = base
                && !actor_links.contains(&index)
            {
                actor_links.push(index);
                actors.push(imports[index].clone());
            }
        }
    }
    actor_links.extend(gameplay::unique_referenced_members(
        &references,
        &candidates,
    ));
    let mut effect_refs = Vec::new();
    let mut inspected_items = std::collections::HashSet::new();
    for (archive, path) in direct
        .iter()
        .map(|(archive, path)| (archive.as_ref(), path.as_str()))
        .chain(
            actor_links
                .iter()
                .map(|&index| (&imports[index].0, imports[index].1.as_str())),
        )
    {
        if !path.to_ascii_lowercase().starts_with("libs/items/")
            || !inspected_items.insert((archive.id.clone(), path.to_owned()))
        {
            continue;
        }
        let bytes = evolve_core::library_catalog::read_imported_gameplay_member(
            archive,
            path,
            gameplay_values::LIMIT,
        )
        .map_err(|error| error.to_string())?;
        effect_refs.extend(evolve_core::character_gameplay::item_effect_set_references(
            &bytes,
        )?);
    }
    let mut linked = Vec::new();
    for index in
        actor_links
            .into_iter()
            .chain(evolve_core::character_gameplay::unique_referenced_members(
                &effect_refs,
                &candidates,
            ))
    {
        let (archive, path) = &imports[index];
        if !direct
            .iter()
            .any(|(other, member)| other.id == archive.id && member == path)
            && !linked
                .iter()
                .any(|(other, member): &(evolve_core::ImportedPak, String)| {
                    other.id == archive.id && member == path
                })
        {
            linked.push((archive.clone(), path.clone()));
        }
    }
    Ok(linked)
}

fn authored_model_imports(
    archives: &[evolve_core::ImportedPak],
    build: &str,
    character: &str,
) -> Result<VerifiedAppearance, String> {
    let id = evolve_core::character_gameplay::internal_id(character)
        .ok_or("Unknown playable character")?;
    let subtype = format!(
        "scripts/entities/actor/subtypes_{}.lua",
        id.to_ascii_lowercase()
    );
    let mut sources = Vec::new();
    for archive in archives.iter().filter(|archive| {
        archive.builds.iter().any(|label| label == build) && archive.name == "Scripts.pak"
    }) {
        let inventory = evolve_core::library_catalog::imported_archive_preview_inventory(archive)
            .map_err(|error| error.to_string())?;
        for entry in inventory.entries {
            if entry.relative_path.eq_ignore_ascii_case(&subtype) {
                sources.push((archive, entry.relative_path));
            }
        }
    }
    let mut references = Vec::new();
    match sources.as_slice() {
        [] => {}
        [(source, path)] => {
            let bytes = evolve_core::library_catalog::read_imported_member_bounded(
                source,
                path,
                256 * 1024,
            )
            .map_err(|error| error.to_string())?;
            // The 2012 layout is plain text; compiled scripts are read below.
            if !lua51::is_bytecode(&bytes) {
                references = evolve_core::character_gameplay::authored_subtype_models(&bytes)?;
            }
        }
        _ => return Err("More than one authored subtype source in the selected build".into()),
    }
    let in_build: Vec<_> = archives
        .iter()
        .filter(|archive| archive.builds.iter().any(|label| label == build))
        .cloned()
        .collect();
    let target = Target::Character(character.to_owned());
    for script in decoded_scripts(&in_build)? {
        for (global, table) in &script.globals {
            if target.owns_global(&script.path, global)
                && let Some(model) = evolve_core::character_gameplay::script_model(table)?
            {
                references.push(model);
            }
        }
    }
    references.sort();
    references.dedup();
    if references.is_empty() {
        return Ok(VerifiedAppearance::default());
    }
    let mut models = Vec::new();
    for archive in archives
        .iter()
        .filter(|archive| archive.builds.iter().any(|label| label == build))
    {
        let paths = modding_projects::guided_import_members(archive, true)?;
        for path in paths {
            if path.to_ascii_lowercase().ends_with(".cdf")
                && references
                    .iter()
                    .any(|reference| reference.eq_ignore_ascii_case(&path))
            {
                models.push((archive.clone(), path));
            }
        }
    }
    let mut verified = VerifiedAppearance::default();
    let mut material_references = std::collections::BTreeSet::new();
    for reference in references {
        let mut matches = models
            .iter()
            .filter(|(_, path)| path.eq_ignore_ascii_case(&reference));
        if let Some((archive, path)) = matches.next()
            && matches.next().is_none()
        {
            let bytes = evolve_core::library_catalog::read_imported_member_bounded(
                archive,
                path,
                2 * 1024 * 1024,
            )
            .map_err(|error| error.to_string())?;
            material_references.extend(evolve_core::character_gameplay::authored_cdf_materials(
                &bytes,
            )?);
            verified.models.push((archive.clone(), path.clone()));
        }
    }
    if verified.models.is_empty() {
        return Ok(verified);
    }
    let mut candidates = Vec::new();
    for archive in archives
        .iter()
        .filter(|archive| archive.builds.iter().any(|label| label == build))
    {
        let inventory = evolve_core::library_catalog::imported_archive_preview_inventory(archive)
            .map_err(|error| error.to_string())?;
        candidates.extend(
            inventory
                .entries
                .into_iter()
                .map(|entry| (archive.clone(), entry.relative_path)),
        );
    }
    let unique = |path: &str| {
        let mut found = candidates
            .iter()
            .filter(|(_, entry)| entry.eq_ignore_ascii_case(path));
        let first = found.next()?;
        found.next().is_none().then(|| first.clone())
    };
    let mut texture_references = Vec::new();
    for material in material_references {
        let Some((archive, path)) = unique(&material) else {
            continue;
        };
        let bytes = evolve_core::library_catalog::read_imported_member_bounded(
            &archive,
            &path,
            2 * 1024 * 1024,
        )
        .map_err(|error| error.to_string())?;
        let document = evolve_core::parse_material(&bytes).map_err(|error| error.to_string())?;
        for slot in document.slots {
            for texture in slot.maps {
                texture_references.push((path.clone(), texture.path));
            }
        }
        verified.materials.push((archive, path));
    }
    let mut textures = std::collections::BTreeMap::new();
    for (material, authored) in texture_references {
        let Ok(paths) = evolve_core::scoped_texture_candidates(&authored, &material) else {
            continue;
        };
        for path in paths {
            let mut matches = candidates
                .iter()
                .filter(|(_, entry)| entry.eq_ignore_ascii_case(&path));
            let Some((archive, member)) = matches.next() else {
                continue;
            };
            if matches.next().is_some() {
                break;
            }
            evolve_core::library_catalog::verify_imported_member(archive, member)
                .map_err(|error| error.to_string())?;
            textures.insert(
                (archive.id.clone(), member.clone()),
                (archive.clone(), member.clone()),
            );
            break;
        }
    }
    verified.textures = textures.into_values().collect();
    Ok(verified)
}

pub(super) fn link_task(state: &mut State) -> Task<Message> {
    let editor = &mut state.guided_editor;
    let (Some(build), Some(library)) = (&editor.build, &state.library_catalog) else {
        return Task::none();
    };
    if editor.target.is_none() {
        return Task::none();
    }
    let direct = editor.imports.clone();
    let archives = library.archives.clone();
    let build = build.clone();
    let request = editor.links_request;
    editor.links_status = LinkStatus::Pending;
    if editor.models {
        let Target::Character(name) = editor.target.as_ref().unwrap() else {
            editor.links_status = LinkStatus::Idle;
            return Task::none();
        };
        let name = name.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    authored_model_imports(&archives, &build, &name)
                })
                .await
                .map_err(|error| error.to_string())?
            },
            move |result| message(Action::ModelsReady(request, result)),
        )
    } else {
        let target = editor.target.clone().unwrap();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    let in_build: Vec<_> = archives
                        .iter()
                        .filter(|archive| archive.builds.contains(&build))
                        .cloned()
                        .collect();
                    let scripts = script_sources(&target, &decoded_scripts(&in_build)?)?;
                    let mut linked = scripts.owned;
                    linked.extend(scripts.bases);
                    linked.extend(actor_linked_imports(
                        &direct,
                        &archives,
                        &build,
                        scripts.references,
                    )?);
                    let mut seen = std::collections::HashSet::new();
                    linked
                        .retain(|(archive, path)| seen.insert((archive.id.clone(), path.clone())));
                    Ok(linked)
                })
                .await
                .map_err(|error| error.to_string())?
            },
            move |result| message(Action::LinksReady(request, result)),
        )
    }
}
/// Repopulate imported edit targets from the already-indexed library without
/// resetting unsaved gameplay fields or requiring another source-PAK import.
pub fn refresh_imports(state: &mut State) {
    if !state.guided_editor.open {
        return;
    }
    let editor = &mut state.guided_editor;
    editor.preview_request = editor.preview_request.wrapping_add(1);
    editor.links_request = editor.links_request.wrapping_add(1);
    editor.links_status = LinkStatus::Idle;
    editor.appearance = VerifiedAppearance::default();
    editor.preview = None;
    editor.imports.clear();
    editor.imports_page = 0;
    let mut skipped = 0;
    if let Some(library) = &state.library_catalog {
        let build = editor.build.as_ref().or_else(|| {
            state
                .library_by_build
                .then(|| library.builds.get(state.library_build_index))
                .flatten()
        });
        for archive in &library.archives {
            if build.is_some_and(|b| !archive.builds.contains(b)) {
                continue;
            }
            match modding_projects::guided_import_members(archive, editor.models) {
                Ok(paths) => {
                    let archive = Arc::new(archive.clone());
                    editor.imports.extend(
                        paths
                            .into_iter()
                            .filter(|path| {
                                editor.models
                                    || editor
                                        .target
                                        .as_ref()
                                        .is_none_or(|target| target.owns_document(path))
                            })
                            .map(|path| (archive.clone(), path)),
                    );
                }
                Err(_) => skipped += 1,
            }
        }
    }
    editor.note = if state.library_catalog.is_some() {
        format!(
            "{} assets in editable projects; {} imported assets ready; {skipped} imports unavailable. {} Originals remain unchanged.",
            editor.rows.len(),
            editor.imports.len(),
            if editor.models {
                "Select an imported model to create an editable copy."
            } else {
                "Preview an imported document before creating an editable copy."
            }
        )
    } else if state.library_startup_loading {
        "Loading previously imported Asset Library PAKs; no reimport required.".into()
    } else {
        "No saved imported Asset Library catalog loaded. Select Refresh assets to scan saved manifests without importing PAKs again.".into()
    };
}

pub fn update(state: &mut State, action: Action) -> Task<Message> {
    if state.busy
        && !matches!(
            action,
            Action::PreviewReady(..) | Action::LinksReady(..) | Action::ModelsReady(..)
        )
    {
        state.status = "Wait for the current operation before editing.".into();
        return Task::none();
    }
    let editor = &mut state.guided_editor;
    if matches!(
        action,
        Action::Open(_)
            | Action::OpenTarget { .. }
            | Action::Close
            | Action::Select(_)
            | Action::Prepare(_)
            | Action::PreviewImport(_)
            | Action::Refresh
    ) && editor.fields.iter().any(|f| f.value != f.original)
    {
        editor.note =
            "Save your changes or use Discard edits before switching files or closing.".into();
        return Task::none();
    }
    match action {
        Action::Open(models) | Action::OpenTarget { models, .. } => {
            let (target, build) = match action {
                Action::OpenTarget { target, build, .. } => (Some(target), Some(build)),
                _ => (None, None),
            };
            *editor = Editor {
                open: true,
                models,
                target,
                build,
                preview_request: editor.preview_request.wrapping_add(1),
                links_request: editor.links_request.wrapping_add(1),
                ..Editor::default()
            };
            for project in state.modding_projects.as_deref().unwrap_or(&[]) {
                if editor
                    .build
                    .as_ref()
                    .is_some_and(|build| project.source_build != *build)
                {
                    continue;
                }
                if let Ok(paths) = modding_projects::guided_members(project, models) {
                    let project = Arc::new(project.clone());
                    editor.rows.extend(
                        paths
                            .into_iter()
                            .filter(|path| {
                                editor.models
                                    || editor
                                        .target
                                        .as_ref()
                                        .is_none_or(|target| target.owns_document(path))
                            })
                            .map(|path| (project.clone(), path)),
                    );
                }
            }
            if editor.models {
                editor.search =
                    editor
                        .target
                        .as_ref()
                        .map_or(String::new(), |target| match target {
                            Target::Character(name) => {
                                evolve_core::character_gameplay::internal_id(name)
                                    .unwrap_or(name)
                                    .to_owned()
                            }
                            Target::Wildlife(name) => evolve_core::character_gameplay::WILDLIFE
                                .iter()
                                .find(|(label, _)| *label == name)
                                .map_or(name.as_str(), |(_, id)| id)
                                .to_owned(),
                        });
            }
            refresh_imports(state);
            return Task::batch(vec![
                link_task(state),
                iced::widget::operation::focus(search_id()),
            ]);
        }
        Action::Refresh => {
            if state.library_catalog.is_none() {
                return super::update(state, Message::ModdingReloadLibrary);
            }
            refresh_imports(state);
            return Task::batch(vec![
                super::update(state, Message::ModdingRefresh),
                link_task(state),
            ]);
        }
        Action::Prepare(i) => {
            let Some((archive, path)) = editor.imports.get(i).cloned() else {
                return Task::none();
            };
            if evolve_core::character_gameplay::actor_script(&path) {
                editor.note = "Compiled Lua scripts are read-only.".into();
                return Task::none();
            }
            if state.busy {
                return Task::none();
            }
            let progress = Arc::new(super::ProjectCopyProgress::default());
            state.project_copy_progress = Some(progress.clone());
            let stage = std::path::PathBuf::from(&state.preferences.stage);
            return super::launch(
                state,
                super::Operation::CreateProjectFromLibrary(
                    (*archive).clone(),
                    std::path::PathBuf::from(&state.preferences.projects),
                    stage,
                    progress,
                ),
            );
        }
        Action::PreviewImport(index) => {
            let Some((archive, path)) = editor.imports.get(index).cloned() else {
                return Task::none();
            };
            editor.preview_request = editor.preview_request.wrapping_add(1);
            editor.selected = None;
            editor.fields.clear();
            editor.snapshot.clear();
            let request = editor.preview_request;
            editor.preview = None;
            editor.note = format!("Checking {} before preview…", archive.name);
            let target = editor.target.clone();
            return Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || {
                        preview_imported_values(&archive, &path, target.as_ref())
                    })
                    .await
                    .map_err(|e| e.to_string())?
                },
                move |result| message(Action::PreviewReady(request, index, result)),
            );
        }
        Action::PreviewReady(request, index, result) => {
            if !editor.open || editor.preview_request != request {
                return Task::none();
            }
            match result {
                Ok(fields) => {
                    editor.note = format!(
                        "{} authored fields in this verified source. Preview is read-only.",
                        fields.len()
                    );
                    editor.preview = Some((index, fields));
                    editor.fields_page = 0;
                    editor.fields_search.clear();
                }
                Err(error) => editor.note = format!("Could not preview source values: {error}"),
            }
        }
        Action::LinksReady(request, result) => {
            if !editor.open || editor.links_request != request {
                return Task::none();
            }
            match result {
                Ok(linked) => {
                    let count = linked.len();
                    editor.imports.extend(
                        linked
                            .into_iter()
                            .map(|(archive, path)| (Arc::new(archive), path)),
                    );
                    editor.links_status = LinkStatus::Ready(count);
                }
                Err(error) => editor.links_status = LinkStatus::Failed(error),
            }
        }
        Action::ModelsReady(request, result) => {
            if !editor.open || editor.links_request != request || !editor.models {
                return Task::none();
            }
            match result {
                Ok(appearance) => {
                    editor.links_status = LinkStatus::Ready(appearance.models.len());
                    editor.appearance = appearance;
                }
                Err(error) => editor.links_status = LinkStatus::Failed(error),
            }
        }
        Action::Discard => {
            for f in &mut editor.fields {
                f.value = f.original.clone();
            }
            editor.note = "Unsaved edits discarded.".into();
        }
        Action::Close => {
            editor.open = false;
            editor.preview_request = editor.preview_request.wrapping_add(1);
            editor.links_request = editor.links_request.wrapping_add(1);
        }
        Action::Search(s) => {
            editor.search = s;
            editor.imports_page = 0;
        }
        Action::ImportsPage(next) => {
            editor.imports_page = if next {
                editor.imports_page.saturating_add(1)
            } else {
                editor.imports_page.saturating_sub(1)
            };
        }
        Action::FieldSearch(s) => {
            editor.fields_search = s;
            editor.fields_page = 0;
        }
        Action::FieldsPage(next) => {
            let query = editor.fields_search.to_ascii_lowercase();
            let matches = |label: &str, value: &str| {
                label.to_ascii_lowercase().contains(&query)
                    || value.to_ascii_lowercase().contains(&query)
            };
            let count = if let Some((_, preview)) = &editor.preview {
                preview
                    .iter()
                    .filter(|field| matches(&field.label, &field.value))
                    .count()
            } else {
                editor
                    .fields
                    .iter()
                    .filter(|field| matches(&field.label, &field.original))
                    .count()
            };
            let last = count.saturating_sub(1) / 80;
            editor.fields_page = if next {
                editor.fields_page.saturating_add(1).min(last)
            } else {
                editor.fields_page.saturating_sub(1)
            };
        }
        Action::Value(i, value) => {
            if let Some(f) = editor.fields.get_mut(i) {
                f.value = value;
            }
        }
        Action::Select(i) => {
            editor.preview_request = editor.preview_request.wrapping_add(1);
            editor.preview = None;
            let Some((project, member)) = editor.rows.get(i).cloned() else {
                return Task::none();
            };
            if editor.models {
                if let Some(index) = state
                    .modding_projects
                    .as_ref()
                    .and_then(|ps| ps.iter().position(|p| p.workspace == project.workspace))
                {
                    state.modding_selected = Some(index);
                    state.viewer_collapsed.clear();
                    state.viewer_explorer_visible = true;
                    state.viewer_search = member.clone();
                    state.search = member;
                    return super::update(state, Message::ModdingOpenProject);
                }
            } else {
                let result =
                    modding_projects::read_gameplay_document(&project, &member).and_then(|bytes| {
                        let fields = gameplay_values::fields(&bytes)?;
                        Ok((bytes, fields))
                    });
                match result {
                    Ok((bytes, fields)) => {
                        editor.selected = Some(i);
                        editor.snapshot = bytes;
                        editor.fields = fields;
                        editor.fields_search.clear();
                        editor.fields_page = 0;
                        editor.note = "Gameplay changes are offline only. Save creates a recovery backup. Values use the game's source names; engine-specific ranges are not inferred.".into();
                    }
                    Err(e) => {
                        editor.selected = None;
                        editor.fields.clear();
                        editor.note = e;
                    }
                }
            }
        }
        Action::Save => {
            if let Some((project, member)) = editor.selected.and_then(|i| editor.rows.get(i)) {
                match modding_projects::save_gameplay_document(
                    project,
                    Path::new(&state.preferences.projects),
                    member,
                    &editor.snapshot,
                    &editor.fields,
                ) {
                    Ok(changed) => {
                        editor.note = if changed { "Saved gameplay values with backup. Offline only; refresh projects before building." } else { "No values changed." }.into();
                        // Do not allow a second save from a stale snapshot.
                        editor.selected = None;
                        editor.fields.clear();
                        editor.snapshot.clear();
                        return super::update(state, Message::ModdingRefresh);
                    }
                    Err(e) => editor.note = e,
                }
            }
        }
    }
    Task::none()
}
pub fn view(state: &State) -> Element<'_, Message> {
    let e = &state.guided_editor;
    let mut rows = column![].spacing(3);
    let query = e.search.to_ascii_lowercase();
    let matching: Vec<_> = e
        .rows
        .iter()
        .enumerate()
        .filter(|(_, (p, path))| {
            format!("{} {} {}", p.label, p.source_build, path)
                .to_ascii_lowercase()
                .contains(&query)
        })
        .collect();
    for (i, (project, path)) in matching.iter().take(80) {
        rows = rows.push(
            design::button(
                text(format!(
                    "{} · {} · {}",
                    path, project.label, project.source_build
                ))
                .size(12),
            )
            .on_press(message(Action::Select(*i)))
            .style(design::text_button),
        );
    }
    let mut imported: Vec<_> = e
        .imports
        .iter()
        .enumerate()
        .filter(|(_, (a, path))| {
            format!("{} {} {} {}", path, a.builds.join(" "), a.name, a.source)
                .to_ascii_lowercase()
                .contains(&query)
        })
        .collect();
    let verified = |archive: &evolve_core::ImportedPak, path: &str| {
        e.appearance
            .models
            .iter()
            .any(|(source, member)| source.id == archive.id && member == path)
    };
    if e.models {
        imported.sort_by_key(|(_, (archive, path))| !verified(archive, path));
    }
    let start = e.imports_page.saturating_mul(40).min(imported.len());
    for (i, (archive, path)) in imported.iter().skip(start).take(40) {
        rows = rows.push(
            design::button(
                text(format!(
                    "{}: {} · {} · {} · {}",
                    if e.models {
                        if verified(archive, path) {
                            "Verified subtype model · create editable copy"
                        } else {
                            "Build search result · create editable copy"
                        }
                    } else if evolve_core::character_gameplay::actor_script(path) {
                        "Preview script values (read-only)"
                    } else {
                        "Preview values"
                    },
                    path,
                    archive.name,
                    archive.builds.join(", "),
                    crate::level_archive::source_label(&archive.source)
                ))
                .size(12),
            )
            .on_press(message(if e.models {
                Action::Prepare(*i)
            } else {
                Action::PreviewImport(*i)
            }))
            .style(design::text_button),
        );
    }
    let mut panel = column![
        row![
            text(if e.models {
                "Search models in selected build".to_owned()
            } else if let Some(target) = &e.target {
                format!("{} · gameplay values", target.name())
            } else {
                "Gameplay values · offline only".to_owned()
            })
            .size(17),
            design::button("Refresh assets").on_press(message(Action::Refresh)),
            design::button("Close").on_press(message(Action::Close))
        ]
        .spacing(10),
        text(e.note.as_str()).size(12),
        text(match &e.links_status {
            LinkStatus::Idle => "".to_owned(),
            LinkStatus::Pending if e.models => "Checking authored subtype model links…".to_owned(),
            LinkStatus::Pending => "Checking authored actor references in this build…".to_owned(),
            LinkStatus::Ready(count) if e.models => format!("{count} model definitions, {} explicit materials and {} referenced textures verified. Other files are build search results, not character ownership.", e.appearance.materials.len(), e.appearance.textures.len()),
            LinkStatus::Ready(count) => format!("{count} additional documents linked by authored base actors, equipment, abilities or item effect sets. Ambiguous matches remain unassigned."),
            LinkStatus::Failed(error) if e.models => format!("Could not verify subtype models: {error}"),
            LinkStatus::Failed(error) => format!("Could not check actor references: {error}"),
        }).size(12),
        design::label("Search documents"),
        design::text_input(
            "Find character, ability, perk, effect set, model or build…",
            &e.search,
        )
        .id(search_id())
        .on_input(|s| message(Action::Search(s))),
        text(format!(
            "{} editable matches (first 80 shown) · {}–{} of {} imported",
            matching.len(),
            if imported.is_empty() { 0 } else { start + 1 },
            (start + 40).min(imported.len()),
            imported.len()
        ))
        .size(11),
        scrollable(rows).height(140),
        row![
            design::button("Previous files")
                .on_press_maybe((start > 0).then_some(message(Action::ImportsPage(false)))),
            design::button("Next files").on_press_maybe(
                (start + 40 < imported.len()).then_some(message(Action::ImportsPage(true)))
            ),
        ]
        .spacing(8),
    ]
    .spacing(6);
    if e.models && (!e.appearance.materials.is_empty() || !e.appearance.textures.is_empty()) {
        let mut sources = column![].spacing(3);
        for (archive, path) in &e.appearance.materials {
            sources = sources
                .push(text(format!("Material · {} · {path}", archive.name)).size(design::CAPTION));
        }
        for (archive, path) in &e.appearance.textures {
            sources = sources
                .push(text(format!("Texture · {} · {path}", archive.name)).size(design::CAPTION));
        }
        panel = panel.push(scrollable(sources).height(140));
    }
    if let Some((_, path)) = e.selected.and_then(|i| e.rows.get(i)) {
        let mut fields = column![].spacing(3);
        let query = e.fields_search.to_ascii_lowercase();
        let matching: Vec<_> = e
            .fields
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                f.label.to_ascii_lowercase().contains(&query)
                    || f.original.to_ascii_lowercase().contains(&query)
            })
            .collect();
        let start = e.fields_page.saturating_mul(80).min(matching.len());
        for (i, f) in matching.iter().skip(start).take(80) {
            let i = *i;
            fields = fields.push(
                row![
                    text(f.label.as_str()).size(12).width(Fill),
                    text(format!("Loaded: {}", f.original)).size(11),
                    design::text_input("Value", &f.value)
                        .on_input(move |s| message(Action::Value(i, s)))
                        .width(140)
                ]
                .spacing(8),
            );
        }
        panel = panel
            .push(text(path.as_str()).size(12))
            .push(design::label("Search editable values"))
            .push(
                design::text_input(
                    "Find a value, e.g. cooldown, damage, variant…",
                    &e.fields_search,
                )
                .on_input(|s| message(Action::FieldSearch(s))),
            )
            .push(
                text(format!(
                    "{}–{} of {} editable numeric/boolean fields (authored | stage/variant lists and comma-separated vector widths are validated)",
                    if matching.is_empty() { 0 } else { start + 1 },
                    (start + 80).min(matching.len()),
                    matching.len(),
                ))
                .size(11),
            )
            .push(scrollable(fields).height(180))
            .push(row![
                design::button("Previous values").on_press_maybe(
                    (start > 0).then_some(message(Action::FieldsPage(false)))
                ),
                design::button("Next values").on_press_maybe(
                    (start + 80 < matching.len()).then_some(message(Action::FieldsPage(true)))
                ),
            ].spacing(8))
            .push(
                row![
                    design::button("Save gameplay values").on_press(message(Action::Save)),
                    // Discarding resets every field and cannot be undone.
                    crate::ui_kit::hold_button(
                        &state.ui,
                        "guided-discard",
                        "Hold to discard edits",
                        message(Action::Discard),
                        e.fields.iter().any(|f| f.value != f.original),
                        state.preferences.reduce_motion,
                    ),
                ]
                .spacing(8),
            );
    }
    if let Some((index, values)) = &e.preview
        && let Some((archive, path)) = e.imports.get(*index)
    {
        let query = e.fields_search.to_ascii_lowercase();
        let matching: Vec<_> = values
            .iter()
            .filter(|field| {
                field.label.to_ascii_lowercase().contains(&query)
                    || field.value.to_ascii_lowercase().contains(&query)
            })
            .collect();
        let start = e.fields_page.saturating_mul(80).min(matching.len());
        let mut fields = column![].spacing(3);
        for value in matching.iter().skip(start).take(80) {
            fields = fields.push(
                row![
                    text(value.label.as_str()).size(12).width(Fill),
                    text(value.value.as_str()).size(12).width(Fill),
                ]
                .spacing(8),
            );
        }
        panel = panel
            .push(
                text(format!(
                    "{} · {} · {}",
                    path,
                    archive.name,
                    crate::level_archive::source_label(&archive.source)
                ))
                .size(design::LABEL),
            )
            .push(design::label("Search source fields"))
            .push(
                design::text_input("Find a value…", &e.fields_search)
                    .on_input(|s| message(Action::FieldSearch(s))),
            )
            .push(
                text(format!(
                    "{}–{} of {} read-only values",
                    if matching.is_empty() { 0 } else { start + 1 },
                    (start + 80).min(matching.len()),
                    matching.len()
                ))
                .size(design::CAPTION),
            )
            .push(scrollable(fields).height(180))
            .push(
                row![
                    design::button("Previous values")
                        .on_press_maybe((start > 0).then_some(message(Action::FieldsPage(false)))),
                    design::button("Next values").on_press_maybe(
                        (start + 80 < matching.len()).then_some(message(Action::FieldsPage(true)))
                    ),
                    {
                        let script = evolve_core::character_gameplay::actor_script(path);
                        crate::ui_kit::why_disabled(
                            design::button("Create editable copy")
                                .style(design::primary)
                                .on_press_maybe(
                                    (!script).then_some(message(Action::Prepare(*index))),
                                ),
                            script,
                            "Compiled Lua scripts are read-only",
                        )
                    },
                ]
                .spacing(8),
            );
    }
    panel.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolve_core::{ImportedLibrary, ImportedPak};
    #[test]
    #[ignore = "requires EVOLVE_LEGACY_LIBRARY pointing to an Asset Library with Legacy Scripts and GameData PAKs"]
    fn local_legacy_meteor_goliath_scripts_link_preview_and_model() {
        let root = std::path::PathBuf::from(std::env::var_os("EVOLVE_LEGACY_LIBRARY").unwrap());
        let mut catalog = evolve_core::imported_library(&root).unwrap();
        // A plan-only import has no build snapshot; scope everything to one label.
        for archive in &mut catalog.archives {
            if archive.builds.is_empty() {
                archive.builds = vec!["Legacy".into()];
            }
        }
        let build = catalog.archives[0].builds[0].clone();
        let in_build: Vec<_> = catalog
            .archives
            .iter()
            .filter(|archive| archive.builds.contains(&build))
            .cloned()
            .collect();
        let (characters, wildlife) = crate::modding_page::discover_roster(&in_build).unwrap();
        println!(
            "roster {} characters, {} wildlife",
            characters.len(),
            wildlife.len()
        );
        let missing: Vec<_> = evolve_core::character_gameplay::CHARACTERS
            .iter()
            .enumerate()
            .filter(|(index, _)| !characters.contains(index))
            .map(|(_, (name, _))| *name)
            .collect();
        assert_eq!(
            missing,
            ["Glacial Behemoth", "Quantum Caira", "Battle Cabot"],
            "Legacy defines every other roster character"
        );
        assert_eq!(
            wildlife.len(),
            evolve_core::character_gameplay::WILDLIFE.len()
        );

        let meteor = Target::Character("Meteor Goliath".into());
        let scripts = decoded_scripts(&in_build).unwrap();
        let sources = script_sources(&meteor, &scripts).unwrap();
        let linked = actor_linked_imports(&[], &in_build, &build, sources.references).unwrap();
        let linked: Vec<_> = linked.iter().map(|(_, path)| path.as_str()).collect();
        println!(
            "Meteor links {} documents, bases {:?}",
            linked.len(),
            sources
                .bases
                .iter()
                .map(|(_, path)| path)
                .collect::<Vec<_>>()
        );
        for expected in [
            "Libs/Abilities/Goliath/GoliathMeteorCharge1.xml",
            "Libs/Items/Monsters/Goliath/GoliathMeteorClaw1.xml",
            // Inherited from GoliathA1 -> Goliath1.
            "Libs/Items/Monsters/Goliath/GoliathClaw1.xml",
        ] {
            assert!(
                linked
                    .iter()
                    .any(|path| path.eq_ignore_ascii_case(expected)),
                "{expected} not linked"
            );
        }
        let (archive, path) = &sources.owned[0];
        let fields = preview_imported_values(archive, path, Some(&meteor)).unwrap();
        assert!(
            fields
                .iter()
                .all(|field| field.label.starts_with("GoliathMeteor"))
        );
        assert!(
            fields
                .iter()
                .any(|field| field.label.ends_with("Properties / fileModel"))
        );

        let appearance = authored_model_imports(&in_build, &build, "Meteor Goliath").unwrap();
        println!(
            "Meteor models {:?}, {} materials, {} textures",
            appearance
                .models
                .iter()
                .map(|(_, path)| path)
                .collect::<Vec<_>>(),
            appearance.materials.len(),
            appearance.textures.len()
        );
    }

    #[test]
    fn script_sources_follow_bases_across_files_but_not_ambiguous_names() {
        use evolve_core::lua51::{Table, Value};
        let table = |base: Option<&str>, item: Option<&str>| {
            let mut fields = Vec::new();
            if let Some(base) = base {
                fields.push(("baseType".to_owned(), Value::Global(base.into())));
            }
            if let Some(item) = item {
                let slot = Table {
                    fields: vec![("name".into(), Value::Str(item.into()))],
                    items: vec![],
                };
                let items = Table {
                    fields: vec![("claw".into(), Value::Table(slot))],
                    items: vec![],
                };
                let inventory = Table {
                    fields: vec![("items".into(), Value::Table(items))],
                    items: vec![],
                };
                let game = Table {
                    fields: vec![("inventory".into(), Value::Table(inventory))],
                    items: vec![],
                };
                fields.push(("gameParams".to_owned(), Value::Table(game)));
            }
            Table {
                fields,
                items: vec![],
            }
        };
        let archive = ImportedPak {
            id: "a".repeat(20),
            name: "Scripts.pak".into(),
            source: "C:/game/Scripts.pak".into(),
            directory: String::new(),
            group: String::new(),
            description: String::new(),
            entries: 0,
            extracted_bytes: 0,
            warnings: 0,
            builds: vec!["Legacy".into()],
        };
        let script = |path: &str, globals: Vec<(&str, Table)>| DecodedScript {
            archive: archive.clone(),
            path: path.into(),
            globals: globals
                .into_iter()
                .map(|(name, table)| (name.to_owned(), table))
                .collect(),
        };
        let goliath = "Scripts/Entities/actor/subtypes/subtypes_goliath.lua";
        let scripts = vec![
            script(
                goliath,
                vec![
                    ("GoliathBase", table(Some("Player"), None)),
                    ("Goliath1", table(Some("GoliathBase"), Some("GoliathClaw1"))),
                    ("GoliathA1", table(Some("Goliath1"), None)),
                    (
                        "GoliathMeteor1",
                        table(Some("GoliathA1"), Some("GoliathMeteorClaw1")),
                    ),
                ],
            ),
            script(
                "Scripts/Entities/actor/player.lua",
                vec![("Player", table(None, Some("PlayerFists")))],
            ),
            script(
                "Scripts/Entities/actor/subtypes/subtype_crow.lua",
                vec![("TrackingF", table(Some("Tracking"), None))],
            ),
            script(
                "Scripts/Entities/actor/subtypes_merc.lua",
                vec![("Tracking", table(None, None))],
            ),
            script(
                "Scripts/Entities/actor/subtypes_merc_copy.lua",
                vec![("Tracking", table(None, None))],
            ),
        ];
        let meteor = script_sources(&Target::Character("Meteor Goliath".into()), &scripts).unwrap();
        assert_eq!(meteor.owned.len(), 1);
        assert_eq!(meteor.owned[0].1, goliath);
        assert_eq!(meteor.bases.len(), 1);
        assert_eq!(meteor.bases[0].1, "Scripts/Entities/actor/player.lua");
        let mut items: Vec<_> = meteor
            .references
            .iter()
            .map(|reference| reference.name.as_str())
            .collect();
        items.sort();
        assert_eq!(items, ["GoliathClaw1", "GoliathMeteorClaw1", "PlayerFists"]);
        let crow = script_sources(&Target::Character("Crow".into()), &scripts).unwrap();
        assert_eq!(crow.owned.len(), 1);
        assert!(crow.bases.is_empty());
        let goliath_only = script_sources(&Target::Character("Goliath".into()), &scripts).unwrap();
        assert!(
            !goliath_only
                .references
                .iter()
                .any(|reference| reference.name == "GoliathMeteorClaw1")
        );
    }

    #[test]
    fn authored_2012_model_requires_unique_cdf_in_same_build() {
        let root = tempfile::tempdir().unwrap();
        let subtype = "Scripts/Entities/actor/subtypes_grounder.lua";
        let cdf = "Objects/Characters/Monsters/Grounder/Grounder1.cdf";
        let mut archives = Vec::new();
        for (index, (name, path, bytes)) in [
            (
                "Scripts.pak",
                subtype,
                b"fileModel = \"Objects/Characters/Monsters/Grounder/Grounder1.cdf\",\n".as_slice(),
            ),
            (
                "Objects.pak",
                cdf,
                b"<CharacterDefinition><Model File=\"Objects/rig.chr\"/></CharacterDefinition>"
                    .as_slice(),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let folder = root.path().join(index.to_string());
            let file = folder.join("files").join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, bytes).unwrap();
            let source = format!("C:/2012/{name}");
            std::fs::write(
                folder.join("asset-library.json"),
                serde_json::json!({
                    "version": 1, "name": name, "source": source,
                    "entries": [{"path": path, "bytes": bytes.len()}]
                })
                .to_string(),
            )
            .unwrap();
            archives.push(ImportedPak {
                id: format!("{index:020}"),
                name: name.into(),
                source,
                directory: folder.to_string_lossy().into_owned(),
                group: String::new(),
                description: String::new(),
                entries: 1,
                extracted_bytes: bytes.len() as u64,
                warnings: 0,
                builds: vec!["2012".into()],
            });
        }
        let models = authored_model_imports(&archives, "2012", "Kraken").unwrap();
        assert_eq!(models.models.len(), 1);
        assert_eq!(models.models[0].1, cdf);
        assert!(
            authored_model_imports(&archives, "Legacy", "Kraken")
                .unwrap()
                .models
                .is_empty()
        );
        let duplicate = archives[1].clone();
        archives.push(ImportedPak {
            id: "f".repeat(20),
            ..duplicate
        });
        assert!(
            authored_model_imports(&archives, "2012", "Kraken")
                .unwrap()
                .models
                .is_empty()
        );
        std::fs::write(root.path().join("0/files").join(subtype), b"tampered").unwrap();
        assert!(authored_model_imports(&archives, "2012", "Kraken").is_err());
    }

    #[test]
    fn explicit_cdf_material_and_mtl_texture_cross_paks_without_guessing() {
        let root = tempfile::tempdir().unwrap();
        let cdf = "Objects/Characters/Monsters/Grounder/Grounder1.cdf";
        let material = "Objects/Materials/Blue.mtl";
        let texture = "Objects/Materials/Blue.dds.0";
        let assets: [(&str, &str, &[u8]); 4] = [
            ("Scripts.pak", "Scripts/Entities/actor/subtypes_grounder.lua", b"fileModel = \"Objects/Characters/Monsters/Grounder/Grounder1.cdf\",\n"),
            ("Objects.pak", cdf, b"<CharacterDefinition><Model File=\"Objects/rig.chr\" Material=\"Objects/Materials/Blue\"/></CharacterDefinition>"),
            ("Materials.pak", material, b"<Material><Textures><Texture Map=\"Diffuse\" File=\"Objects/Materials/Blue.tif\"/></Textures></Material>"),
            ("Textures.pak", texture, b"verified cooked texture"),
        ];
        let mut archives = Vec::new();
        for (index, (name, path, bytes)) in assets.into_iter().enumerate() {
            let folder = root.path().join(index.to_string());
            let file = folder.join("files").join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, bytes).unwrap();
            let source = format!("C:/2012/{name}");
            std::fs::write(
                folder.join("asset-library.json"),
                serde_json::json!({
                    "version": 1, "name": name, "source": source,
                    "entries": [{"path": path, "bytes": bytes.len()}]
                })
                .to_string(),
            )
            .unwrap();
            archives.push(ImportedPak {
                id: format!("{index:020}"),
                name: name.into(),
                source,
                directory: folder.to_string_lossy().into_owned(),
                group: String::new(),
                description: String::new(),
                entries: 1,
                extracted_bytes: bytes.len() as u64,
                warnings: 0,
                builds: vec!["2012".into()],
            });
        }
        let appearance = authored_model_imports(&archives, "2012", "Kraken").unwrap();
        assert_eq!(appearance.models.len(), 1);
        assert_eq!(appearance.materials[0].1, material);
        assert_eq!(appearance.textures[0].1, texture);
        archives[3].builds = vec!["Legacy".into()];
        assert!(
            authored_model_imports(&archives, "2012", "Kraken")
                .unwrap()
                .textures
                .is_empty()
        );
        archives[3].builds = vec!["2012".into()];
        let duplicate = archives[3].clone();
        archives.push(ImportedPak {
            id: "f".repeat(20),
            ..duplicate
        });
        assert!(
            authored_model_imports(&archives, "2012", "Kraken")
                .unwrap()
                .textures
                .is_empty()
        );
        archives.pop();
        std::fs::write(root.path().join("3/files").join(texture), b"changed").unwrap();
        assert!(authored_model_imports(&archives, "2012", "Kraken").is_err());
    }
    #[test]
    fn late_actor_links_do_not_change_reopened_or_refreshed_editor() {
        let mut state = State {
            guided_editor: Editor {
                open: true,
                links_request: 9,
                ..Editor::default()
            },
            ..State::default()
        };
        let archive = ImportedPak {
            id: "a".repeat(20),
            name: "items.pak".into(),
            source: "C:/game/items.pak".into(),
            directory: String::new(),
            group: String::new(),
            description: String::new(),
            entries: 1,
            extracted_bytes: 1,
            warnings: 0,
            builds: vec!["Stage 2".into()],
        };
        let linked = vec![(archive.clone(), "libs/Items/Shared/Item.xml".into())];
        let _ = update(&mut state, Action::LinksReady(8, Ok(linked.clone())));
        assert!(state.guided_editor.imports.is_empty());
        let _ = update(&mut state, Action::LinksReady(9, Ok(linked)));
        assert_eq!(state.guided_editor.imports.len(), 1);
        let _ = update(&mut state, Action::Close);
        let _ = update(
            &mut state,
            Action::LinksReady(9, Ok(vec![(archive, "libs/Items/Other.xml".into())])),
        );
        assert_eq!(state.guided_editor.imports.len(), 1);
    }

    #[test]
    fn explicit_actor_item_link_crosses_paks_but_not_builds_or_ambiguity() {
        let root = tempfile::tempdir().unwrap();
        let actor = "libs/Actor/Mercs/Maggie.actorparams";
        let item = "libs/Items/Shared/MobileArenaTriggerWeapon.xml";
        let mut archives = Vec::new();
        for (index, path) in [actor, item, item].into_iter().enumerate() {
            let folder = root.path().join(index.to_string());
            let bytes: &[u8] = if index == 0 {
                br#"<Actor><param name="sItem" value="MobileArenaTriggerWeapon"/></Actor>"#
            } else {
                b"<Item/>"
            };
            let file = folder.join("files").join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, bytes).unwrap();
            let source = format!(
                "C:/game{index}/{}.pak",
                if index == 0 { "libs" } else { "items" }
            );
            let name = if index == 0 { "libs.pak" } else { "items.pak" };
            std::fs::write(
                folder.join("asset-library.json"),
                serde_json::json!({
                    "version": 1, "name": name, "source": source,
                    "entries": [{"path": path, "bytes": bytes.len()}]
                })
                .to_string(),
            )
            .unwrap();
            archives.push(ImportedPak {
                id: format!("{index:020}"),
                name: name.into(),
                source,
                directory: folder.to_string_lossy().into_owned(),
                group: String::new(),
                description: String::new(),
                entries: 1,
                extracted_bytes: bytes.len() as u64,
                warnings: 0,
                builds: vec![if index == 2 { "2012" } else { "Stage 2" }.into()],
            });
        }
        let direct = vec![(Arc::new(archives[0].clone()), actor.to_owned())];
        let linked = actor_linked_imports(&direct, &archives, "Stage 2", vec![]).unwrap();
        assert_eq!(linked.len(), 1);
        assert_eq!(linked[0].0.id, archives[1].id);
        assert!(actor_linked_imports(&direct, &archives, "2012", vec![]).is_err());
        archives[2].builds = vec!["Stage 2".into()];
        assert!(
            actor_linked_imports(&direct, &archives, "Stage 2", vec![])
                .unwrap()
                .is_empty()
        );
        std::fs::write(root.path().join("0/files").join(actor), b"tampered").unwrap();
        assert!(actor_linked_imports(&direct, &archives, "Stage 2", vec![]).is_err());
    }

    #[test]
    fn inherited_bases_link_their_chain_and_levels_only_when_declared() {
        let root = tempfile::tempdir().unwrap();
        let files: [(&str, &[u8]); 5] = [
            (
                "libs/Actor/Monsters/GoliathA.actorparams",
                br#"<Actor name="GoliathA" levels="3" base="BaseMonster1|BaseMonster4"/>"#,
            ),
            (
                "libs/Actor/BaseMonster.actorparams",
                br#"<Actor name="BaseMonster" levels="3" base="BaseGameplayActor"/>"#,
            ),
            (
                "libs/Actor/BaseGameplayActor.actorparams",
                br#"<Actor name="BaseGameplayActor" base="BaseMonster"><param name="sItem" value="SharedRegen"/></Actor>"#,
            ),
            ("libs/Items/Shared/SharedRegen.xml", b"<Item/>"),
            (
                "libs/Actor/Unrelated.actorparams",
                br#"<Actor name="Unrelated"/>"#,
            ),
        ];
        let folder = root.path().join("libs");
        let mut entries = Vec::new();
        for (path, bytes) in files {
            let file = folder.join("files").join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, bytes).unwrap();
            entries.push(serde_json::json!({"path": path, "bytes": bytes.len()}));
        }
        std::fs::write(
            folder.join("asset-library.json"),
            serde_json::json!({
                "version": 1, "name": "libs.pak", "source": "C:/game/libs.pak",
                "entries": entries
            })
            .to_string(),
        )
        .unwrap();
        let archive = ImportedPak {
            id: "a".repeat(20),
            name: "libs.pak".into(),
            source: "C:/game/libs.pak".into(),
            directory: folder.to_string_lossy().into_owned(),
            group: String::new(),
            description: String::new(),
            entries: files.len(),
            extracted_bytes: 1,
            warnings: 0,
            builds: vec!["Stage 2".into()],
        };
        let direct = vec![(Arc::new(archive.clone()), files[0].0.to_owned())];
        let mut linked: Vec<_> = actor_linked_imports(&direct, &[archive], "Stage 2", vec![])
            .unwrap()
            .into_iter()
            .map(|(_, path)| path)
            .collect();
        linked.sort();
        // BaseMonster4 exceeds the declared three levels; the base cycle ends.
        assert_eq!(
            linked,
            [
                "libs/Actor/BaseGameplayActor.actorparams",
                "libs/Actor/BaseMonster.actorparams",
                "libs/Items/Shared/SharedRegen.xml",
            ]
        );
    }

    #[test]
    #[ignore = "requires EVOLVE_BENCH_LIBRARY pointing to a local imported Asset Library"]
    fn local_maggie_actor_links_shared_items_only_within_its_build() {
        let root = std::path::PathBuf::from(std::env::var_os("EVOLVE_BENCH_LIBRARY").unwrap());
        let catalog = evolve_core::imported_library(&root).unwrap();
        let archive = catalog
            .archives
            .iter()
            .find(|archive| archive.name == "libs.pak")
            .unwrap();
        let build = archive.builds.first().unwrap();
        let direct = vec![(
            Arc::new(archive.clone()),
            "libs/Actor/Mercs/Maggie.actorparams".into(),
        )];
        let linked = actor_linked_imports(&direct, &catalog.archives, build, vec![]).unwrap();
        assert!(
            linked
                .iter()
                .any(|(archive, path)| archive.builds.contains(build)
                    && path.eq_ignore_ascii_case(
                        "libs/Items/Mercs/Shared/MobileArenaTriggerWeapon.xml"
                    ))
        );
        assert!(
            linked
                .iter()
                .any(|(archive, path)| archive.builds.contains(build)
                    && path.eq_ignore_ascii_case("libs/EffectSets/MercHealthRegen.xml"))
        );
        for base in ["BaseMerc", "BaseGameplayActor", "BaseActor"] {
            assert!(
                linked.iter().any(|(_, path)| path
                    .eq_ignore_ascii_case(&format!("libs/Actor/{base}.actorparams"))),
                "{base} is not in Maggie's inherited chain"
            );
        }
        assert!(
            !linked
                .iter()
                .any(|(archive, _)| !archive.builds.contains(build))
        );
    }

    #[test]
    #[ignore = "requires EVOLVE_BENCH_LIBRARY pointing to a local imported Asset Library"]
    fn local_2012_grounder_model_is_bound_by_authored_subtype() {
        let root = std::path::PathBuf::from(std::env::var_os("EVOLVE_BENCH_LIBRARY").unwrap());
        let library = evolve_core::imported_library(&root).unwrap();
        let source = library
            .archives
            .iter()
            .find(|archive| archive.name == "Scripts.pak")
            .unwrap();
        let build = source.builds.first().unwrap();
        let models = authored_model_imports(&library.archives, build, "Kraken").unwrap();
        assert!(
            models
                .models
                .iter()
                .any(|(archive, path)| archive.builds.contains(build)
                    && path.eq_ignore_ascii_case(
                        "Objects/Characters/Monsters/Grounder/Grounder1.cdf"
                    ))
        );
        assert!(
            models
                .materials
                .iter()
                .any(|(archive, path)| archive.builds.contains(build)
                    && path.eq_ignore_ascii_case(
                        "Objects/Characters/Monsters/Grounder/Grounder1_Skeleton.mtl"
                    ))
        );
    }

    #[test]
    fn imported_preview_is_read_only_and_rejects_foreign_or_modified_members() {
        let dir = tempfile::tempdir().unwrap();
        let path = "libs/Abilities/Goliath/GoliathCharge.xml";
        let content = br#"<Ability name="Charge" damage="42"/>"#;
        let file = dir.path().join("files").join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, content).unwrap();
        std::fs::write(
            dir.path().join("asset-library.json"),
            serde_json::json!({
                "version": 1, "name": "libs.pak", "source": "C:/Game/libs.pak",
                "entries": [{"path": path, "bytes": content.len()}]
            })
            .to_string(),
        )
        .unwrap();
        let archive = ImportedPak {
            id: "a".repeat(20),
            name: "libs.pak".into(),
            source: "C:/Game/libs.pak".into(),
            directory: dir.path().to_string_lossy().into_owned(),
            group: String::new(),
            description: String::new(),
            entries: 1,
            extracted_bytes: content.len() as u64,
            warnings: 0,
            builds: vec![],
        };
        let fields = preview_imported_values(&archive, path, None).unwrap();
        assert!(fields.iter().any(|field| field.value == "42"));
        assert_eq!(std::fs::read(&file).unwrap(), content);
        assert!(preview_imported_values(&archive, "../outside.xml", None).is_err());
        assert!(
            preview_imported_values(
                &ImportedPak {
                    source: "D:/Other/libs.pak".into(),
                    ..archive.clone()
                },
                path,
                None
            )
            .is_err()
        );
        std::fs::write(&file, b"changed").unwrap();
        assert!(preview_imported_values(&archive, path, None).is_err());
    }

    #[test]
    #[ignore = "requires EVOLVE_BENCH_LIBRARY pointing to a local imported Asset Library"]
    fn local_imported_goliath_values_preview() {
        let root = std::path::PathBuf::from(std::env::var_os("EVOLVE_BENCH_LIBRARY").unwrap());
        let catalog = evolve_core::imported_library(&root).unwrap();
        let archive = catalog
            .archives
            .iter()
            .find(|archive| archive.name == "libs.pak")
            .unwrap();
        let fields =
            preview_imported_values(archive, "libs/Actor/Monsters/GoliathA.actorparams", None)
                .unwrap();
        assert!(
            fields
                .iter()
                .any(|field| field.label.contains("fMaxHealth"))
        );
    }

    #[test]
    fn saved_imports_are_available_to_guided_modding_without_reimport() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = serde_json::json!({
            "version": 1,
            "source": "C:/EvolveGame/models.pak",
            "entries": [{"path": "characters/hunters/merc_emet/emet_3p.cdf"}]
        });
        std::fs::write(dir.path().join("asset-library.json"), manifest.to_string()).unwrap();
        let archive = ImportedPak {
            id: "1".repeat(20),
            name: "models.pak".into(),
            source: "C:/EvolveGame/models.pak".into(),
            directory: dir.path().to_string_lossy().into_owned(),
            group: "Hunters".into(),
            description: "Models".into(),
            entries: 1,
            extracted_bytes: 1,
            warnings: 0,
            builds: vec![],
        };
        let mut state = State {
            library_catalog: Some(ImportedLibrary {
                root: dir.path().to_string_lossy().into_owned(),
                editions: Default::default(),
                archives: vec![archive],
                builds: vec![],
                total_bytes: 1,
                ignored: 0,
            }),
            ..State::default()
        };
        state.guided_editor.open = true;
        state.guided_editor.models = true;
        refresh_imports(&mut state);
        assert_eq!(state.guided_editor.imports.len(), 1);

        // An imported source outside stage/paks must reach the new automatic
        // staging worker, rather than being rejected by the obsolete guard.
        let _ = update(&mut state, Action::Prepare(0));
        assert!(state.busy);
    }

    #[test]
    fn imported_effectset_perks_appear_in_gameplay_values() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = serde_json::json!({
            "version": 1,
            "source": "C:/EvolveGame/items.pak",
            "entries": [{"path": "Libs/Items/Perks/Assault/Perk_Assault_Rearm.effectset"}]
        });
        std::fs::write(dir.path().join("asset-library.json"), manifest.to_string()).unwrap();
        let archive = ImportedPak {
            id: "2".repeat(20),
            name: "items.pak".into(),
            source: "C:/EvolveGame/items.pak".into(),
            directory: dir.path().to_string_lossy().into_owned(),
            group: "Items".into(),
            description: "Items".into(),
            entries: 1,
            extracted_bytes: 1,
            warnings: 0,
            builds: vec![],
        };
        let mut state = State {
            library_catalog: Some(ImportedLibrary {
                root: dir.path().to_string_lossy().into_owned(),
                editions: Default::default(),
                archives: vec![archive],
                builds: vec![],
                total_bytes: 1,
                ignored: 0,
            }),
            ..State::default()
        };
        state.guided_editor.open = true;
        state.guided_editor.models = false;
        refresh_imports(&mut state);
        assert_eq!(state.guided_editor.imports.len(), 1);
        assert_eq!(
            state.guided_editor.imports[0].1,
            "Libs/Items/Perks/Assault/Perk_Assault_Rearm.effectset"
        );
    }

    #[test]
    fn imported_gameplay_keeps_same_named_archives_from_independent_sources() {
        let dir = tempfile::tempdir().unwrap();
        let mut archives = Vec::new();
        for (id, source) in [("a", "C:/GameA/libs.pak"), ("b", "D:/GameB/libs.pak")] {
            let folder = dir.path().join(id);
            std::fs::create_dir(&folder).unwrap();
            std::fs::write(
                folder.join("asset-library.json"),
                serde_json::json!({
                    "version": 1, "source": source,
                    "entries": [{"path": "libs/Abilities/Goliath/GoliathCharge.xml"}]
                })
                .to_string(),
            )
            .unwrap();
            archives.push(ImportedPak {
                id: id.repeat(20),
                name: "libs.pak".into(),
                source: source.into(),
                directory: folder.to_string_lossy().into_owned(),
                group: "Gameplay".into(),
                description: String::new(),
                entries: 1,
                extracted_bytes: 1,
                warnings: 0,
                builds: vec!["Stage 2".into()],
            });
        }
        let mut state = State {
            library_catalog: Some(ImportedLibrary {
                root: dir.path().to_string_lossy().into_owned(),
                editions: Default::default(),
                archives,
                builds: vec!["Stage 2".into()],
                total_bytes: 2,
                ignored: 0,
            }),
            ..State::default()
        };
        state.guided_editor.open = true;
        state.guided_editor.rows.push((
            Arc::new(ProjectRow {
                label: "Existing mod".into(),
                target: "libs.pak".into(),
                source_build: "Stage 2".into(),
                status: String::new(),
                details: String::new(),
                workspace: dir.path().join("project"),
                modified: vec![],
                changed_known: true,
                enabled: false,
            }),
            "libs/Abilities/Goliath/GoliathCharge.xml".into(),
        ));
        refresh_imports(&mut state);
        assert_eq!(state.guided_editor.imports.len(), 2);
        assert_ne!(
            state.guided_editor.imports[0].0.source,
            state.guided_editor.imports[1].0.source
        );
    }

    #[test]
    fn field_pages_reach_late_values_and_reset_on_search() {
        let mut state = State::default();
        state.guided_editor.fields = (0..181)
            .map(|i| ValueField {
                element: i,
                attribute: "value".into(),
                label: format!("field {i}"),
                original: i.to_string(),
                value: i.to_string(),
            })
            .collect();
        let _ = update(&mut state, Action::FieldsPage(true));
        let _ = update(&mut state, Action::FieldsPage(true));
        assert_eq!(state.guided_editor.fields_page, 2);
        let _ = update(&mut state, Action::FieldsPage(true));
        assert_eq!(state.guided_editor.fields_page, 2);
        let _ = update(&mut state, Action::FieldSearch("180".into()));
        assert_eq!(state.guided_editor.fields_page, 0);
    }
}
