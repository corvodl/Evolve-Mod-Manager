# Level Editor from the command line

The Level Editor's UI-free logic lives in `evolve-core` under
`evolve_core::level_editor` (the unsaved-edit draft, how a draft is saved,
deletion checks, committed undo history, the terrain brush, map environment,
atmosphere and terrain-material sources), with Modding-project storage in
`evolve_core::modding_projects`. The desktop app draws and drives these;
`evolve-cli` runs the same code headless, so a CLI save follows the same
rules as **Save map** in the app.

Rendering (`level_scene_renderer`, `world_instances`, `world_mips`, the WGSL
shaders), picking, gizmos and the map loader (`level_library`, which assembles
the GPU scene) stay in `evolve-desktop`.

Build the CLI with `cargo build -p evolve-core --bin evolve-cli`. It needs no
elevation: it reads and writes only inside the Projects root you give it.

## Commands

```text
evolve-cli project list <projects-root> <stage>
evolve-cli level open <project-folder> <layer-member> [scene.json]
evolve-cli level wildlife <mission.xml|layer.lyr> [spawns.json]
evolve-cli level apply <projects-root> <stage> <project-folder> <layer-member> <edits.json> [--dry-run]
```

- `project list` prints each Modding project as `label | target | status |
  workspace`. `<stage>` is the selected game stage folder, as in the app.
- `level open` loads one XML layer of a project and prints its placements by
  kind and its wildlife entity count. `<layer-member>` is the path inside the
  project's `files` folder, with `/` separators, for example
  `levels/Tutorial/layers/objects.lyr`. A `scene.json` report is never
  overwritten.
- `level wildlife` lists the wildlife spawn entities in a mission or layer
  file: object index, class, GUID, name, spawn type, world position and home
  area, then the link, circle and unresolved-link counts. A `spawns.json`
  report also carries populations and links.
- `level apply` applies a list of edits to one layer as one draft, then saves
  it into the project. `--dry-run` checks and rewrites the layer in memory and
  writes nothing.

## edits.json

A JSON list, applied in order. Positions, rotations (`w,x,y,z` quaternion) and
scales are the object's **local** values, as authored in its layer. A GUID
names an object in that layer; a GUID declared more than once is refused.

```json
[
  {"op": "move", "guid": "{…}", "position": [10, 20, 30]},
  {"op": "move", "guid": "{…}", "position": [0, 0, 0], "rotation": [1, 0, 0, 0], "scale": [2, 2, 2]},
  {"op": "copy", "guid": "{…}", "new_guid": "{…}", "position": [0, 0, 5]},
  {"op": "shape", "guid": "{…}", "points": [[0, 0, 0], [8, 0, 0], [8, 8, 0]]},
  {"op": "delete", "guid": "{…}"}
]
```

- `move` keeps the current rotation and scale when they are omitted.
- `copy` needs a `new_guid` not already used in the layer; a later edit can
  refer to the copy by that GUID.
- `shape` replaces an area's polygon vertices, keeping their order.
- `delete` is refused while a child, an entity link or a wildlife home area in
  any layer of the owning map refers to the object.

## What a save guarantees

- The project must be one `project list` finds under `<projects-root>`, and
  the layer must be a regular, manifest-listed `.lyr` or `.xml` file inside
  its `files` folder; paths that escape it are refused.
- Every edit is checked and the whole layer rewritten in memory before
  anything is written. Deletions re-read every layer of the map and fail if
  any changed during the check.
- The write uses the project's checked member save: it fails if the layer
  changed since it was read, and keeps the previous file in the project's
  `EditorBackups` folder. Original game PAKs and the Asset Library are never
  written.
- Compiled `terrain.dat` placements, terrain painting and vegetation edits are
  only available in the app.
