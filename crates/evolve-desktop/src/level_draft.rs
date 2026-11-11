//! Unsaved Level Editor changes. Moving, rotating, scaling, copying and
//! deleting placements and editing area polygons change the scene and the
//! drawn world at once, without writing any file or reassembling the map.
//! Each change is a snapshot to undo and redo; saving writes the difference
//! from the map as it was loaded.
use crate::world_instances::{Instance, World};
use evolve_core::{LevelScene, Transform};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// The map as one change left it.
#[derive(Clone)]
pub(super) struct Snapshot {
    pub scene: Arc<LevelScene>,
    pub world: Option<Arc<World>>,
    /// Copies made in this draft: the new object's id, and the id it copies.
    pub origins: Arc<HashMap<String, String>>,
    /// Ids deleted in this draft. Their objects stay, hidden, so every
    /// object index (which the drawn world refers to) stays the same.
    pub deleted: Arc<HashSet<String>>,
}

/// The most changes kept for undo.
const MAX_UNDO: usize = 200;

pub(super) struct Draft {
    /// The map as loaded: what saving compares against.
    pub original: Snapshot,
    pub current: Snapshot,
    pub undo: Vec<Snapshot>,
    pub redo: Vec<Snapshot>,
}

impl Draft {
    pub fn new(loaded: Snapshot) -> Self {
        Self {
            original: loaded.clone(),
            current: loaded,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// Whether anything differs from the map as loaded.
    pub fn changed(&self) -> bool {
        !Arc::ptr_eq(&self.original.scene, &self.current.scene)
    }

    pub fn push(&mut self, next: Snapshot) {
        let previous = std::mem::replace(&mut self.current, next);
        self.undo.push(previous);
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.redo
            .push(std::mem::replace(&mut self.current, previous));
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(&mut self.current, next));
        true
    }
}

/// New local transforms for several objects, by index.
pub(super) fn transform(
    snapshot: &Snapshot,
    edits: &[(usize, Transform)],
) -> Result<Snapshot, String> {
    let named = edits
        .iter()
        .map(|(index, local)| {
            snapshot
                .scene
                .objects
                .get(*index)
                .map(|object| (object.id.clone(), *local))
                .ok_or_else(|| "That placement is no longer in the map".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let scene = evolve_core::edit_level_transforms(&snapshot.scene, &named)
        .map_err(|error| format!("Invalid transform: {error}"))?;
    Ok(Snapshot {
        world: snapshot
            .world
            .as_ref()
            .map(|world| Arc::new(world.updated(&scene))),
        scene: Arc::new(scene),
        ..snapshot.clone()
    })
}

/// A copy of object `index` with id `new_id` and local transform `local`,
/// drawn with the same model. Returns the copy's index.
pub(super) fn copy(
    snapshot: &Snapshot,
    index: usize,
    new_id: String,
    local: Transform,
) -> Result<(Snapshot, usize), String> {
    let source = snapshot
        .scene
        .objects
        .get(index)
        .ok_or("That placement is no longer in the map")?;
    let mut scene = (*snapshot.scene).clone();
    let mut copied = source.clone();
    copied.id = new_id.clone();
    copied.name = format!("{}_copy", source.name);
    let new_index = scene.objects.len();
    scene.objects.push(copied);
    let scene = evolve_core::edit_level_transforms(&scene, &[(new_id.clone(), local)])
        .map_err(|error| format!("Invalid transform: {error}"))?;
    let world = snapshot.world.as_ref().map(|world| {
        let mut next = world.updated(&scene);
        let placed = scene.objects[new_index].world;
        for group in &mut next.groups {
            if group.instances.iter().any(|i| i.object_index == index) {
                group.instances.push(Instance {
                    object_index: new_index,
                    transform: placed,
                });
            }
        }
        next.instance_count = next.groups.iter().map(|g| g.instances.len()).sum();
        Arc::new(next)
    });
    let mut origins = (*snapshot.origins).clone();
    origins.insert(new_id, source.id.clone());
    Ok((
        Snapshot {
            scene: Arc::new(scene),
            world,
            origins: Arc::new(origins),
            deleted: snapshot.deleted.clone(),
        },
        new_index,
    ))
}

/// Delete objects by index: hidden and no longer drawn or picked.
pub(super) fn delete(snapshot: &Snapshot, indices: &[usize]) -> Snapshot {
    let mut scene = (*snapshot.scene).clone();
    let mut deleted = (*snapshot.deleted).clone();
    for &index in indices {
        if let Some(object) = scene.objects.get_mut(index) {
            object.hidden = true;
            deleted.insert(object.id.clone());
        }
    }
    let gone = indices.iter().copied().collect::<HashSet<_>>();
    let world = snapshot.world.as_ref().map(|world| {
        let mut next = world.updated(&scene);
        for group in &mut next.groups {
            group
                .instances
                .retain(|instance| !gone.contains(&instance.object_index));
        }
        next.instance_count = next.groups.iter().map(|g| g.instances.len()).sum();
        Arc::new(next)
    });
    Snapshot {
        scene: Arc::new(scene),
        world,
        origins: snapshot.origins.clone(),
        deleted: Arc::new(deleted),
    }
}

/// New polygon vertices (local) for an area object.
pub(super) fn shape(snapshot: &Snapshot, index: usize, points: Vec<[f32; 3]>) -> Snapshot {
    let mut scene = (*snapshot.scene).clone();
    if let Some(object) = scene.objects.get_mut(index) {
        object.shape_points = points;
    }
    Snapshot {
        world: snapshot
            .world
            .as_ref()
            .map(|world| Arc::new(world.updated(&scene))),
        scene: Arc::new(scene),
        ..snapshot.clone()
    }
}

/// One difference between the loaded map and the draft, by object index in
/// the draft's scene.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Change {
    /// A new object copying the object with this id.
    Copy {
        index: usize,
        origin: String,
    },
    Transform {
        index: usize,
    },
    Shape {
        index: usize,
    },
    Delete {
        index: usize,
    },
}

/// What saving `current` must write, compared with `original`: copies first
/// (in the order made, so a copy of a copy follows its origin), then moves
/// and polygons, then deletions.
pub(super) fn changes(original: &Snapshot, current: &Snapshot) -> Vec<Change> {
    let loaded = original
        .scene
        .objects
        .iter()
        .map(|object| (object.id.as_str(), object))
        .collect::<HashMap<_, _>>();
    let mut copies = Vec::new();
    let mut edits = Vec::new();
    let mut deletes = Vec::new();
    for (index, object) in current.scene.objects.iter().enumerate() {
        let deleted = current.deleted.contains(&object.id);
        match loaded.get(object.id.as_str()) {
            Some(before) => {
                if deleted {
                    deletes.push(Change::Delete { index });
                    continue;
                }
                if object.local != before.local {
                    edits.push(Change::Transform { index });
                }
                if object.shape_points != before.shape_points {
                    edits.push(Change::Shape { index });
                }
            }
            None if !deleted => {
                if let Some(origin) = current.origins.get(&object.id) {
                    copies.push(Change::Copy {
                        index,
                        origin: origin.clone(),
                    });
                    edits.push(Change::Transform { index });
                    if object.shape_points.len() >= 3 {
                        edits.push(Change::Shape { index });
                    }
                }
            }
            None => {}
        }
    }
    copies.into_iter().chain(edits).chain(deletes).collect()
}

/// The object a copy (or a copy of a copy) was made from in the loaded map.
pub(super) fn root_origin<'a>(snapshot: &'a Snapshot, id: &'a str) -> &'a str {
    let mut at = id;
    for _ in 0..snapshot.origins.len() + 1 {
        match snapshot.origins.get(at) {
            Some(origin) => at = origin,
            None => break,
        }
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(id: &str, x: f32) -> evolve_core::PlacedObject {
        let at = Transform {
            position: [x, 0.0, 0.0],
            ..Transform::default()
        };
        evolve_core::PlacedObject {
            id: id.into(),
            parent: String::new(),
            layer: "layers/main.lyr".into(),
            name: id.into(),
            kind: "Brush".into(),
            model: "objects/rock.cgf".into(),
            material: String::new(),
            hidden: false,
            local: at,
            world: at,
            status: String::new(),
            populations: Vec::new(),
            home_area: String::new(),
            shape_points: Vec::new(),
            wildlife: Default::default(),
        }
    }

    fn loaded() -> Snapshot {
        Snapshot {
            scene: Arc::new(LevelScene {
                schema: 1,
                objects: vec![object("a", 0.0), object("b", 5.0)],
                diagnostics: Vec::new(),
            }),
            world: None,
            origins: Arc::default(),
            deleted: Arc::default(),
        }
    }

    #[test]
    fn edits_undo_redo_and_become_the_changes_to_save() {
        let mut draft = Draft::new(loaded());
        assert!(!draft.changed());
        let moved = Transform {
            position: [1.0, 2.0, 3.0],
            ..Transform::default()
        };
        draft.push(transform(&draft.current, &[(0, moved)]).unwrap());
        let (copied, index) = copy(&draft.current, 0, "c".into(), moved).unwrap();
        assert_eq!(index, 2);
        draft.push(copied);
        let (copy_of_copy, _) = copy(&draft.current, 2, "d".into(), moved).unwrap();
        draft.push(copy_of_copy);
        draft.push(delete(&draft.current, &[1]));
        assert!(draft.changed());
        assert_eq!(
            draft.current.scene.objects[0].world.position,
            [1.0, 2.0, 3.0]
        );
        assert_eq!(root_origin(&draft.current, "d"), "a");
        assert_eq!(
            changes(&draft.original, &draft.current),
            vec![
                Change::Copy {
                    index: 2,
                    origin: "a".into()
                },
                Change::Copy {
                    index: 3,
                    origin: "c".into()
                },
                Change::Transform { index: 0 },
                Change::Transform { index: 2 },
                Change::Transform { index: 3 },
                Change::Delete { index: 1 },
            ]
        );
        // Undo all the way back is unchanged; redo restores the edits.
        while draft.undo() {}
        assert!(!draft.changed());
        assert!(changes(&draft.original, &draft.current).is_empty());
        assert!(draft.redo());
        assert_eq!(
            changes(&draft.original, &draft.current),
            vec![Change::Transform { index: 0 }]
        );
        // A copy that is deleted again is simply never saved.
        let (copied, index) = copy(&draft.current, 1, "e".into(), moved).unwrap();
        let removed = delete(&copied, &[index]);
        assert_eq!(
            changes(&draft.original, &removed),
            vec![Change::Transform { index: 0 }]
        );
    }
}
