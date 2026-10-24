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
    imports: Vec<(Arc<evolve_core::library_catalog::ImportedPak>, String)>,
    search: String,
    fields_search: String,
    rows: Vec<(Arc<ProjectRow>, String)>,
    selected: Option<usize>,
    snapshot: Vec<u8>,
    fields: Vec<ValueField>,
    note: String,
}
#[derive(Debug, Clone)]
pub enum Action {
    Open(bool),
    Close,
    Search(String),
    FieldSearch(String),
    Select(usize),
    Value(usize, String),
    Save,
    Prepare(usize),
    Discard,
    Refresh,
}
fn message(action: Action) -> Message {
    Message::GuidedEdit(action)
}
/// Repopulate imported edit targets from the already-indexed library without
/// resetting unsaved gameplay fields or requiring another source-PAK import.
pub fn refresh_imports(state: &mut State) {
    if !state.guided_editor.open {
        return;
    }
    let editor = &mut state.guided_editor;
    editor.imports.clear();
    let mut skipped = 0;
    if let Some(library) = &state.library_catalog {
        let build = state
            .library_by_build
            .then(|| library.builds.get(state.library_build_index))
            .flatten();
        for archive in &library.archives {
            if state.library_by_build && !build.is_some_and(|b| archive.builds.contains(b)) {
                continue;
            }
            if editor.rows.iter().any(|(p, _)| {
                p.target.eq_ignore_ascii_case(&archive.name)
                    && archive.builds.contains(&p.source_build)
            }) {
                continue;
            }
            match modding_projects::guided_import_members(archive, editor.models) {
                Ok(paths) => {
                    let archive = Arc::new(archive.clone());
                    editor
                        .imports
                        .extend(paths.into_iter().map(|path| (archive.clone(), path)));
                }
                Err(_) => skipped += 1,
            }
        }
    }
    editor.note = if state.library_catalog.is_some() {
        format!(
            "{} assets in editable projects; {} imported assets ready; {skipped} imports unavailable. Select an imported asset to create a staged editing copy. Originals remain unchanged.",
            editor.rows.len(), editor.imports.len()
        )
    } else if state.library_startup_loading {
        "Loading previously imported Asset Library PAKs; no reimport required.".into()
    } else {
        "No saved imported Asset Library catalog loaded. Select Refresh assets to scan saved manifests without importing PAKs again.".into()
    };
}

pub fn update(state: &mut State, action: Action) -> Task<Message> {
    if state.busy {
        state.status = "Wait for the current operation before editing.".into();
        return Task::none();
    }
    let editor = &mut state.guided_editor;
    if matches!(
        action,
        Action::Open(_) | Action::Close | Action::Select(_) | Action::Prepare(_) | Action::Refresh
    ) && editor.fields.iter().any(|f| f.value != f.original)
    {
        editor.note =
            "Save your changes or use Discard edits before switching files or closing.".into();
        return Task::none();
    }
    match action {
        Action::Open(models) => {
            *editor = Editor {
                open: true,
                models,
                ..Editor::default()
            };
            let mut skipped = 0;
            for project in state.modding_projects.as_deref().unwrap_or(&[]) {
                match modding_projects::guided_members(project, models) {
                    Ok(paths) => {
                        let project = Arc::new(project.clone());
                        editor
                            .rows
                            .extend(paths.into_iter().map(|path| (project.clone(), path)));
                    }
                    Err(_) => skipped += 1,
                }
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
            let stage = std::path::PathBuf::from(&state.preferences.stage);
            return super::launch(
                state,
                super::Operation::CreateProjectFromLibrary(
                    (*archive).clone(),
                    std::path::PathBuf::from(&state.preferences.projects),
                    stage,
                ),
            );
        }
        Action::Discard => {
            for f in &mut editor.fields {
                f.value = f.original.clone();
            }
            editor.note = "Unsaved edits discarded.".into();
        }
        Action::Close => editor.open = false,
        Action::Search(s) => editor.search = s,
        Action::FieldSearch(s) => editor.fields_search = s,
        Action::Value(i, value) => {
            if let Some(f) = editor.fields.get_mut(i) {
                f.value = value;
            }
        }
        Action::Select(i) => {
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
            format!("{} {}", path, a.builds.join(" "))
                .to_ascii_lowercase()
                .contains(&query)
        })
        .collect();
    for (i, (archive, path)) in imported.iter().take(40) {
        rows = rows.push(
            design::button(
                text(format!(
                    "Create editable copy: {} · {}",
                    path,
                    archive.builds.join(", ")
                ))
                .size(12),
            )
            .on_press(message(Action::Prepare(*i)))
            .style(design::text_button),
        );
    }
    let mut panel = column![
        row![
            text(if e.models {
                "Models in editable projects"
            } else {
                "Gameplay values · offline only"
            })
            .size(17),
            design::button("Refresh assets").on_press(message(Action::Refresh)),
            design::button("Close").on_press(message(Action::Close))
        ]
        .spacing(10),
        text(e.note.as_str()).size(12),
        design::text_input("Find character, ability, model or build…", &e.search)
            .on_input(|s| message(Action::Search(s))),
        text(format!(
            "{} editable / {} imported matches (first 80 / 40 shown; narrow the search)",
            matching.len(),
            imported.len()
        ))
        .size(11),
        scrollable(rows).height(140),
    ]
    .spacing(6);
    if let Some((_, path)) = e.selected.and_then(|i| e.rows.get(i)) {
        let mut fields = column![].spacing(3);
        let query = e.fields_search.to_ascii_lowercase();
        let matching: Vec<_> = e
            .fields
            .iter()
            .enumerate()
            .filter(|(_, f)| f.label.to_ascii_lowercase().contains(&query))
            .collect();
        for (i, f) in matching.iter().take(80) {
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
                    "Find a value, e.g. cooldown, damage, health…",
                    &e.fields_search,
                )
                .on_input(|s| message(Action::FieldSearch(s))),
            )
            .push(
                text(format!(
                    "{} numeric/boolean fields (first 80 shown; | separates stages, commas separate vector components)",
                    matching.len()
                ))
                .size(11),
            )
            .push(scrollable(fields).height(180))
            .push(row![design::button("Save gameplay values").on_press(message(Action::Save)), design::button("Discard edits").on_press(message(Action::Discard))].spacing(8));
    }
    panel.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use evolve_core::{ImportedLibrary, ImportedPak};

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
}
