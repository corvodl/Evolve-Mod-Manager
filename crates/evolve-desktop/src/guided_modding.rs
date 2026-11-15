//! Searchable gameplay forms and model shortcuts over editable project manifests.
use super::{Message, State};
use crate::{
    design,
    modding_projects::{self, ProjectRow},
};
use evolve_core::gameplay_values::{self, ValueField};
use iced::widget::{column, row, scrollable, text};
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
    preview: Option<(usize, Vec<ValueField>)>,
    preview_request: u64,
    note: String,
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
    PreviewReady(u64, usize, Result<Vec<ValueField>, String>),
    Discard,
    Refresh,
}
fn message(action: Action) -> Message {
    Message::GuidedEdit(action)
}

fn preview_imported_values(
    archive: &evolve_core::ImportedPak,
    path: &str,
) -> Result<Vec<ValueField>, String> {
    // The source identity is checked again, not just when the list was built.
    if !modding_projects::guided_import_members(archive, false)?
        .iter()
        .any(|candidate| candidate == path)
    {
        return Err("Imported source changed since discovery".into());
    }
    let inventory = evolve_core::imported_archive_files(archive).map_err(|e| e.to_string())?;
    let member = inventory
        .entries
        .iter()
        .find(|entry| entry.relative_path == path)
        .ok_or("Imported member is no longer in the selected source")?;
    if member.byte_size > gameplay_values::LIMIT as u64 {
        return Err("Gameplay document exceeds the supported preview limit".into());
    }
    let file = Path::new(&inventory.root).join(path);
    let metadata = std::fs::symlink_metadata(&file).map_err(|e| e.to_string())?;
    if !metadata.file_type().is_file()
        || metadata.len() != member.byte_size
        || !std::fs::canonicalize(&file)
            .map_err(|e| e.to_string())?
            .starts_with(&inventory.root)
    {
        return Err("Imported gameplay document changed since validation".into());
    }
    let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
    if bytes.len() as u64 != member.byte_size {
        return Err("Imported gameplay document changed while reading".into());
    }
    gameplay_values::fields(&bytes)
}
/// Repopulate imported edit targets from the already-indexed library without
/// resetting unsaved gameplay fields or requiring another source-PAK import.
pub fn refresh_imports(state: &mut State) {
    if !state.guided_editor.open {
        return;
    }
    let editor = &mut state.guided_editor;
    editor.preview_request = editor.preview_request.wrapping_add(1);
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
    if state.busy && !matches!(action, Action::PreviewReady(..)) {
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
        }
        Action::Refresh => {
            if state.library_catalog.is_none() {
                return super::update(state, Message::ModdingReloadLibrary);
            }
            refresh_imports(state);
            return super::update(state, Message::ModdingRefresh);
        }
        Action::Prepare(i) => {
            let Some((archive, _)) = editor.imports.get(i).cloned() else {
                return Task::none();
            };
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
            return Task::perform(
                async move {
                    tokio::task::spawn_blocking(move || preview_imported_values(&archive, &path))
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
                        "{} editable values in this verified source. Preview is read-only.",
                        fields.len()
                    );
                    editor.preview = Some((index, fields));
                    editor.fields_page = 0;
                    editor.fields_search.clear();
                }
                Err(error) => editor.note = format!("Could not preview source values: {error}"),
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
            let active_fields = editor
                .preview
                .as_ref()
                .map_or(&editor.fields, |(_, fields)| fields);
            let count = active_fields
                .iter()
                .filter(|field| {
                    field.label.to_ascii_lowercase().contains(&query)
                        || field.original.to_ascii_lowercase().contains(&query)
                })
                .count();
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
    let imported: Vec<_> = e
        .imports
        .iter()
        .enumerate()
        .filter(|(_, (a, path))| {
            format!("{} {} {} {}", path, a.builds.join(" "), a.name, a.source)
                .to_ascii_lowercase()
                .contains(&query)
        })
        .collect();
    let start = e.imports_page.saturating_mul(40).min(imported.len());
    for (i, (archive, path)) in imported.iter().skip(start).take(40) {
        rows = rows.push(
            design::button(
                text(format!(
                    "{}: {} · {} · {} · {}",
                    if e.models {
                        "Create editable copy"
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
        design::text_input(
            "Find character, ability, perk, effect set, model or build…",
            &e.search,
        )
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
                    || field.original.to_ascii_lowercase().contains(&query)
            })
            .collect();
        let start = e.fields_page.saturating_mul(80).min(matching.len());
        let mut fields = column![].spacing(3);
        for value in matching.iter().skip(start).take(80) {
            fields = fields.push(
                row![
                    text(value.label.as_str()).size(12).width(Fill),
                    text(value.original.as_str()).size(12),
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
                    design::button("Create editable copy")
                        .style(design::primary)
                        .on_press(message(Action::Prepare(*index))),
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
        let fields = preview_imported_values(&archive, path).unwrap();
        assert!(fields.iter().any(|field| field.original == "42"));
        assert_eq!(std::fs::read(&file).unwrap(), content);
        assert!(preview_imported_values(&archive, "../outside.xml").is_err());
        assert!(
            preview_imported_values(
                &ImportedPak {
                    source: "D:/Other/libs.pak".into(),
                    ..archive.clone()
                },
                path
            )
            .is_err()
        );
        std::fs::write(&file, b"changed").unwrap();
        assert!(preview_imported_values(&archive, path).is_err());
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
            preview_imported_values(archive, "libs/Actor/Monsters/GoliathA.actorparams").unwrap();
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
