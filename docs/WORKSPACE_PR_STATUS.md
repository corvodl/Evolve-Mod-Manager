# Workspace implementation status

The four images in `docs/screenshots/workspace-proposals/` are **browser-rendered
layout references**, not Iced application screenshots and not evidence that
their sample data or visual controls work.

## Wired to application behavior

- Home launch cards and tool shortcuts use existing navigation, Play, project,
  and native-window messages. Legacy Home remains available in its theme.
- Sidebar launches separate Asset Viewer and Level Editor windows with project
  SVG icons. Existing Play with/without mods and Settings actions remain at the
  bottom of the rail.
- Opening the Level Editor explicitly displays a map chooser. Imported archives
  and editable Modding projects are enumerated through validated map discovery
  before opening a selected map; source PAKs are not written. The scene
  outliner panel and its source-layer/wildlife groups can be collapsed.
- Modding shows actual enabled-project manifest collisions and PAK target
  clashes. It offers existing project selection, disable and refresh actions.
  Same-target mods are blocked by the existing launcher; no VFS winner is
  claimed when mount precedence has not been verified.
- Asset Viewer exposes authored CDF bone attachments and exact declared DBA
  paths from currently indexed source data, with existing verify/load actions.
  It does not infer sockets or source builds from display names.

## Not yet implemented (do not mark this PR complete)

- User-controlled persistent load order, a verified CryPAK VFS precedence
  model, collision winner selection, unified patch PAK generation and exports.
- Editable attachment offsets with verified renderer application, durable
  project persistence, sockets JSON export and a calibrated on-model gizmo.
- Terrain splat painting, arbitrary time-of-day authoring and navigation mesh
  regeneration in the Level Editor mockup.
- A native-app screenshot review of the new layouts. The checked-in mockups
  are illustrative only.

The desktop binary's Windows manifest requests elevation; on this machine
`cargo test -p evolve-mod-manager --bin evolve-mod-manager` compiled its test
binary but Windows refused to execute it with `os error 740`. `cargo check`
and formatting verification can run without elevation.
