//! Headless Level Editor commands over an editable Modding project. They use
//! the same draft, validation and checked project save as the desktop app.
use evolve_core::level_editor::level_draft::{self, Draft, Snapshot};
use evolve_core::level_editor::{level_save, level_validation};
use evolve_core::modding_projects::{self, ProjectRow};
use evolve_core::wildlife_spawns::{self, WildlifeEntity};
use evolve_core::{LevelScene, Transform, load_level_scene};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

/// Bounded like every other project member edit.
const MAX_EDITS_BYTES: u64 = 4 * 1024 * 1024;
const MAX_EDITS: usize = 10_000;

/// Run `project …` and the project-level `level …` commands. `None` leaves
/// the arguments to the older file-based `level` commands.
pub fn run(verb: &str, args: &[OsString]) -> Option<Result<(), String>> {
    let text = |at: usize| args.get(at).and_then(|arg| arg.to_str());
    let path = |at: usize| args.get(at).map(PathBuf::from);
    // The optional report path follows `count` required arguments.
    let report = |count: usize| match args.len() {
        n if n == count => Ok(None),
        n if n == count + 1 => Ok(path(count)),
        _ => Err(super::usage()),
    };
    Some(match (verb, text(0)?) {
        ("project", "list") if args.len() == 3 => project_list(&path(1)?, &path(2)?),
        ("level", "open") => match report(3) {
            Ok(output) => level_open(&path(1)?, text(2)?, output.as_deref()),
            Err(usage) => Err(usage),
        },
        ("level", "wildlife") => match report(2) {
            Ok(output) => level_wildlife(&path(1)?, output.as_deref()),
            Err(usage) => Err(usage),
        },
        ("level", "apply") if args.len() == 6 || args.len() == 7 => {
            let dry_run = match text(6) {
                None => false,
                Some("--dry-run") => true,
                Some(_) => return Some(Err(super::usage())),
            };
            level_apply(
                &path(1)?,
                &path(2)?,
                &path(3)?,
                text(4)?,
                &path(5)?,
                dry_run,
            )
        }
        ("project", _) => Err(super::usage()),
        _ => return None,
    })
}

fn project_list(projects_root: &Path, stage: &Path) -> Result<(), String> {
    let projects = modding_projects::discover_projects(projects_root, stage)?;
    if projects.is_empty() {
        println!("No Modding projects under {}", projects_root.display());
    }
    for project in &projects {
        println!(
            "{} | {} | {} | {}",
            project.label,
            project.target,
            project.status,
            project.workspace.display()
        );
    }
    Ok(())
}

/// A project's registered row, matched by its canonical workspace folder.
fn find_project(projects_root: &Path, stage: &Path, folder: &Path) -> Result<ProjectRow, String> {
    let folder = fs::canonicalize(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    modding_projects::discover_projects(projects_root, stage)?
        .into_iter()
        .find(|project| fs::canonicalize(&project.workspace).is_ok_and(|path| path == folder))
        .ok_or_else(|| {
            format!(
                "{} is not a project under the Projects root",
                folder.display()
            )
        })
}

/// The project file for `member`, which must be a relative path that stays
/// inside the project's `files` folder and names a regular XML layer.
fn project_member(project_folder: &Path, member: &str) -> Result<PathBuf, String> {
    let relative = Path::new(member);
    if member.contains('\\')
        || !relative
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return Err("Name the layer as a relative path inside the project's files folder".into());
    }
    let lower = member.to_ascii_lowercase();
    if !(lower.ends_with(".lyr") || lower.ends_with(".xml")) {
        return Err(format!("{member} is not an XML layer (.lyr or .xml)"));
    }
    let root = fs::canonicalize(project_folder.join("files")).map_err(|e| e.to_string())?;
    let source = root.join(relative);
    let metadata = fs::symlink_metadata(&source).map_err(|e| format!("{member}: {e}"))?;
    let canonical = fs::canonicalize(&source).map_err(|e| e.to_string())?;
    if !metadata.file_type().is_file() || !canonical.starts_with(&root) {
        return Err(format!("{member} is not a regular file in this project"));
    }
    Ok(canonical)
}

fn count_by<'a>(items: impl Iterator<Item = &'a str>) -> Vec<(&'a str, usize)> {
    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
    for item in items {
        *counts.entry(item).or_default() += 1;
    }
    counts.into_iter().collect()
}

fn level_open(project_folder: &Path, member: &str, output: Option<&Path>) -> Result<(), String> {
    let source = project_member(project_folder, member)?;
    let scene = load_level_scene(&source).map_err(|e| e.to_string())?;
    let wildlife = scene
        .objects
        .iter()
        .filter(|object| object.is_wildlife_spawn())
        .count();
    println!(
        "{member}: {} placements, {wildlife} wildlife spawn entities, {} diagnostics",
        scene.objects.len(),
        scene.diagnostics.len()
    );
    for (kind, count) in count_by(scene.objects.iter().map(|o| o.kind.as_str())) {
        println!("  {count:>6}  {kind}");
    }
    for diagnostic in &scene.diagnostics {
        println!("  ! {diagnostic}");
    }
    if let Some(path) = output {
        super::write_report(&scene, path)?;
    }
    Ok(())
}

#[derive(Serialize)]
struct SpawnRow<'a> {
    index: usize,
    id: &'a str,
    name: &'a str,
    class: &'static str,
    spawn_type: &'static str,
    position: [f32; 3],
    hidden: bool,
    home_area: &'a str,
    populations: &'a [String],
    links: &'a [String],
}

#[derive(Serialize)]
struct SpawnReport<'a> {
    spawns: Vec<SpawnRow<'a>>,
    links: usize,
    circles: usize,
    unresolved_links: usize,
}

fn level_wildlife(source: &Path, output: Option<&Path>) -> Result<(), String> {
    let scene = load_level_scene(source).map_err(|e| e.to_string())?;
    let spawns = scene
        .objects
        .iter()
        .enumerate()
        .filter_map(|(index, object)| {
            let entity = WildlifeEntity::of(object)?;
            Some(SpawnRow {
                index,
                id: &object.id,
                name: &object.name,
                class: entity.class_name(),
                spawn_type: entity.spawn_type().label(),
                position: object.world.position,
                hidden: object.hidden,
                home_area: &object.home_area,
                populations: &object.populations,
                links: &object.wildlife.links,
            })
        })
        .collect::<Vec<_>>();
    let layout = wildlife_spawns::layout(&scene);
    println!(
        "{} wildlife spawn entities, {} links, {} spawn circles, {} unresolved links",
        spawns.len(),
        layout.links.len(),
        layout.circles.len(),
        layout.unresolved_links
    );
    for spawn in &spawns {
        let [x, y, z] = spawn.position;
        println!(
            "  #{} {} {} \"{}\" ({}) at {x:.2},{y:.2},{z:.2}{}",
            spawn.index,
            spawn.class,
            spawn.id,
            spawn.name,
            spawn.spawn_type,
            if spawn.home_area.is_empty() {
                String::new()
            } else {
                format!(" home {}", spawn.home_area)
            }
        );
    }
    if let Some(path) = output {
        super::write_report(
            &SpawnReport {
                links: layout.links.len(),
                circles: layout.circles.len(),
                unresolved_links: layout.unresolved_links,
                spawns,
            },
            path,
        )?;
    }
    Ok(())
}

/// One edit in an `edits.json` list, applied in order. Positions, rotations
/// and scales are the object's local values, as authored in its layer.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase", deny_unknown_fields)]
enum Edit {
    /// Omitted rotation or scale keeps the object's current value.
    Move {
        guid: String,
        position: [f32; 3],
        rotation: Option<[f32; 4]>,
        scale: Option<[f32; 3]>,
    },
    Copy {
        guid: String,
        new_guid: String,
        position: [f32; 3],
    },
    Delete {
        guid: String,
    },
    Shape {
        guid: String,
        points: Vec<[f32; 3]>,
    },
}

/// The index of the one object with this GUID; several are refused, as in
/// the desktop editor, because the owning declaration would be ambiguous.
fn object_index(scene: &LevelScene, guid: &str) -> Result<usize, String> {
    let mut found = scene
        .objects
        .iter()
        .enumerate()
        .filter(|(_, object)| object.id.eq_ignore_ascii_case(guid) && !object.hidden);
    let (index, _) = found
        .next()
        .ok_or_else(|| format!("No placement {guid} in this layer"))?;
    if found.next().is_some() {
        return Err(format!(
            "{guid} is declared more than once; edit it in the app"
        ));
    }
    Ok(index)
}

/// Apply `edits` to a draft of `scene`, as the desktop draft would.
fn apply_edits(scene: LevelScene, edits: &[Edit]) -> Result<Draft<()>, String> {
    let mut draft = Draft::new(Snapshot {
        scene: Arc::new(scene),
        world: None,
        origins: Arc::default(),
        deleted: Arc::default(),
    });
    for edit in edits {
        let current = &draft.current;
        let next = match edit {
            Edit::Move {
                guid,
                position,
                rotation,
                scale,
            } => {
                let index = object_index(&current.scene, guid)?;
                let local = current.scene.objects[index].local;
                let moved = Transform {
                    position: *position,
                    quaternion: rotation.unwrap_or(local.quaternion),
                    scale: scale.unwrap_or(local.scale),
                };
                level_draft::transform(current, &[(index, moved)])?
            }
            Edit::Copy {
                guid,
                new_guid,
                position,
            } => {
                let index = object_index(&current.scene, guid)?;
                if current
                    .scene
                    .objects
                    .iter()
                    .any(|object| object.id.eq_ignore_ascii_case(new_guid))
                {
                    return Err(format!("{new_guid} is already used in this layer"));
                }
                let local = Transform {
                    position: *position,
                    ..current.scene.objects[index].local
                };
                level_draft::copy(current, index, new_guid.clone(), local)?.0
            }
            Edit::Delete { guid } => {
                let index = object_index(&current.scene, guid)?;
                level_validation::validate_deletion(&current.scene, guid)?;
                level_draft::delete(current, &[index])
            }
            Edit::Shape { guid, points } => {
                let index = object_index(&current.scene, guid)?;
                level_draft::shape(current, index, points.clone())
            }
        };
        draft.push(next);
    }
    Ok(draft)
}

fn read_edits(path: &Path) -> Result<Vec<Edit>, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_EDITS_BYTES {
        return Err("edits.json must be a regular file of at most 4 MiB".into());
    }
    let edits: Vec<Edit> = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("edits.json: {e}"))?;
    if edits.is_empty() || edits.len() > MAX_EDITS {
        return Err(format!("edits.json must list 1 to {MAX_EDITS} edits"));
    }
    Ok(edits)
}

fn level_apply(
    projects_root: &Path,
    stage: &Path,
    project_folder: &Path,
    member: &str,
    edits_path: &Path,
    dry_run: bool,
) -> Result<(), String> {
    let edits = read_edits(edits_path)?;
    let project = find_project(projects_root, stage, project_folder)?;
    let source = project_member(&project.workspace, member)?;
    let scene = load_level_scene(&source).map_err(|e| e.to_string())?;
    let draft = apply_edits(scene, &edits)?;
    // A deletion must also be safe across every other layer of the map.
    let mut snapshots = Vec::new();
    for change in level_draft::changes(&draft.original, &draft.current) {
        if let level_draft::Change::Delete { index } = change {
            let guid = &draft.current.scene.objects[index].id;
            snapshots.push(level_validation::validate_project_deletion(
                &project.workspace.join("files"),
                &source,
                guid,
            )?);
        }
    }
    let files = level_save::file_edits(&draft, |index| {
        Ok((draft.current.scene.objects[index].clone(), source.clone()))
    })?;
    let changes = match files.first() {
        Some((_, level_save::LevelFileEdit::Xml(changes))) => changes.len(),
        _ => 0,
    };
    if dry_run {
        let edited = level_save::edit_files(&project, files)?;
        for member in &edited {
            println!(
                "Dry run: {} would change ({} -> {} bytes, {changes} edits). Nothing written.",
                member.member,
                member.original.len(),
                member.output.len()
            );
        }
        return Ok(());
    }
    for snapshot in &snapshots {
        snapshot.verify()?;
    }
    let saved = level_save::save_files(&project, projects_root, files)?;
    println!(
        "Saved {saved} project member(s) with {changes} edits; the previous file is kept in EditorBackups."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAYER: &[u8] = br#"<ObjectLayer><LayerObjects><Object Id="{11111111-1111-1111-1111-111111111111}" Name="rock" Type="Brush" Prefab="objects/rock.cgf" Pos="1,2,3"/><Object Id="{22222222-2222-2222-2222-222222222222}" Name="parent" Type="Brush" Prefab="objects/rock.cgf"/><Object Id="{33333333-3333-3333-3333-333333333333}" Name="child" Type="Brush" Prefab="objects/rock.cgf" Parent="{22222222-2222-2222-2222-222222222222}"/></LayerObjects></ObjectLayer>"#;
    const ROCK: &str = "{11111111-1111-1111-1111-111111111111}";

    fn scene() -> LevelScene {
        evolve_core::parse_level_scene(&[("main.lyr", LAYER)]).unwrap()
    }

    fn parse(json: &str) -> Vec<Edit> {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn edits_json_becomes_draft_changes() {
        let edits = parse(&format!(
            r#"[{{"op":"move","guid":"{ROCK}","position":[5,6,7]}},
                {{"op":"copy","guid":"{ROCK}","new_guid":"{{44444444-4444-4444-4444-444444444444}}","position":[0,0,0]}}]"#
        ));
        let draft = apply_edits(scene(), &edits).unwrap();
        let rock = &draft.current.scene.objects[0];
        assert_eq!(rock.local.position, [5.0, 6.0, 7.0]);
        assert_eq!(rock.local.scale, [1.0, 1.0, 1.0]);
        assert_eq!(
            level_draft::changes(&draft.original, &draft.current),
            vec![
                level_draft::Change::Copy {
                    index: 3,
                    origin: ROCK.into()
                },
                level_draft::Change::Transform { index: 0 },
                level_draft::Change::Transform { index: 3 },
            ]
        );
    }

    #[test]
    fn unsafe_edits_are_refused_before_anything_is_written() {
        let referenced =
            parse(r#"[{"op":"delete","guid":"{22222222-2222-2222-2222-222222222222}"}]"#);
        assert!(apply_edits(scene(), &referenced).is_err());
        let missing = parse(r#"[{"op":"move","guid":"{missing}","position":[0,0,0]}]"#);
        assert!(apply_edits(scene(), &missing).is_err());
        let taken = parse(&format!(
            r#"[{{"op":"copy","guid":"{ROCK}","new_guid":"{ROCK}","position":[0,0,0]}}]"#
        ));
        assert!(apply_edits(scene(), &taken).is_err());
        assert!(serde_json::from_str::<Vec<Edit>>(r#"[{"op":"format","guid":"x"}]"#).is_err());
    }

    #[test]
    fn members_must_stay_inside_the_project_files() {
        let project = tempfile::tempdir().unwrap();
        let layers = project.path().join("files/levels/test/layers");
        fs::create_dir_all(&layers).unwrap();
        fs::write(layers.join("main.lyr"), LAYER).unwrap();
        assert!(project_member(project.path(), "levels/test/layers/main.lyr").is_ok());
        for refused in [
            "../outside.lyr",
            "levels\\test\\layers\\main.lyr",
            "levels/test/layers/missing.lyr",
            "levels/test/terrain/terrain.dat",
        ] {
            assert!(
                project_member(project.path(), refused).is_err(),
                "{refused}"
            );
        }
    }
}
