# Manager interface redesign

Design-method reference: [unslop](https://github.com/heydamir/unslop) by
Damir Sokolovsky ([@heydamir](https://x.com/heydamir)), licensed CC BY 4.0.
This project uses its audit-and-pass methodology, not its images or UI assets.

## Audit and direction

- **Product:** Evolve Mod Manager helps players inspect original assets,
  prepare editable projects and launch/test mods with source provenance intact.
- **Character:** exact, grounded, restrained.
- **World:** a dark field workstation; existing Evolve red denotes current
  context and intentional actions, rather than coloring every surface.
- **Moments:** the existing Home Evolve artwork and original model/level
  previews. No generated imagery or new game assets.
- **Signature:** a thin red edge at the selected Modern navigation item.

The old Modern rail started at 64 px, hid page names until hover and moved the
workspace as it expanded. Home combined artwork with many transparent tiles,
identical-sized status cards, uppercase section labels, and mixed 7/9/13 px
spacing. The screen had good content and an existing recognizable brand, so
the redesign retains both and corrects hierarchy instead of replacing them.

## Changes and invariants

- Modern navigation is 240 px, always labelled, with aligned 36 px rows and
  one restrained selection edge. Legacy keeps its expanding rail and palette.
- Shared dark/light surfaces follow a quieter neutral ramp; Evolve red remains
  the accent. Existing user color overrides, light mode, and the Legacy skin
  remain selectable. Controls and panels use a consistent spacing/radius scale.
- Home emphasizes the editable project, launch actions, and real announcement.
  Workspace destinations keep their original order and messages. Status is one
  scannable panel rather than five nested cards; long paths show their tail
  with the full value on hover. Optional wallpaper remains optional.
- Settings and Modding use the shared panel rhythm; no launch, import,
  extraction, editing, or recovery handler is changed. This is a visual
  redesign, not an asset-provenance or security change.

## Acceptance

Review Modern dark and light at 1440×900 and the supported 880×620 minimum;
check Home with/without a recent project and wallpaper, Asset Library with a
large list, Modding, Settings, Level Editor and the separate Asset Viewer.
Confirm all navigation and launch controls still work. Compare Legacy before
and after, verify keyboard focus/readability and custom color overrides.
Windows executable test runs may require elevation because of the app manifest;
CI/build checks do not substitute for an interactive screenshot review.

## Component and interaction survey — 2026-11-15 23:44 EST (UTC-05:00)

Reviewed Magic UI, Aceternity UI, 21st.dev, Awwwards and React Bits. These
are **design references**, not dependencies: their implementations are mainly
React/Tailwind/Motion, while the manager is native Rust/Iced. Adapt only the
interaction and information hierarchy in existing Modern controls; preserve
the Legacy skin, light mode, custom colors and the quiet red selection edge.

### Worth prototyping first

1. **Task stages with truthful progress.** [Aceternity's Stateful
   Button](https://ui.aceternity.com/components/stateful-button) communicates
   idle → working → done, and its [Multi Step
   Loader](https://ui.aceternity.com/components/multi-step-loader) presents
   stages. Apply the pattern to import, prepare, export and verified launch:
   show *actual* operation stage, progress when measurable, failure/retry and
   an accessible text status. Do not use the loader's timer-driven/looping
   states as fake progress or imply success before verification finishes.
2. **Dense, legible asset lists.** The [21st.dev table with row
   selection](https://21st.dev/@originui/components/table/table-with-row-selection)
   and [filtered table](https://21st.dev/@originui/components/table/data-table-with-filters-made-with-tan-stack-table)
   suggest a compact Asset Library/Modding list with explicit selection,
   filter chips, sortable metadata and a stable details pane. Keep full
   archive/path provenance available and actions unambiguous; test large lists
   and the 880×620 minimum before adding more columns. These are references,
   not shadcn components to install in Iced.
3. **Actionable empty states.** [21st.dev's Empty
   patterns](https://21st.dev/community/components/s/empty-state) suggest
   distinct "no game found", "no project", "no imported assets" and "no search
   results" states. Each should explain the condition and offer one relevant
   next action (choose path, create project, import, clear filters), without
   decorative nested cards or misleading error language.
4. **File hierarchy and context.** [Magic UI's File
   Tree](https://magicui.design/docs/components/file-tree) is a useful model
   for archive → folder → asset disclosure in the Asset Library. Prefer a
   keyboard-operable, virtualized native tree with expansion state and a
   persistent source/path detail area; do not animate every row.
5. **Contextual destructive actions.** [React Bits'
   Stepper](https://reactbits.dev/components/stepper) suggests a restrained
   project/import setup flow, while its [Hold
   Button](https://reactbits.dev/micro/hold-button) is a reference for
   deliberate confirmation. Use plain confirmation with clear target, impact
   and cancel for replacing files or clearing staged content; do not rely on
   hold duration as the only safety mechanism.

### Lower priority / visual references

- [Magic UI's Animated List](https://magicui.design/docs/components/animated-list)
  and [React Bits' Animated
  List](https://reactbits.dev/components/animated-list): borrow a brief
  selection/arrival transition for *recent activity*, not the main asset
  table. Preserve stable row positions and provide reduced-motion behavior.
- [Aceternity's Expandable
  Card](https://ui.aceternity.com/components/expandable-card): use a compact
  mod/project summary with details on demand, not another Home card grid.
- [Awwwards' Elements
  index](https://www.awwwards.com/elements/) and
  [notification examples](https://www.awwwards.com/elements/notification/)
  are inspiration galleries, **not** installable components. Borrow the
  principle of small, clearly worded operation feedback; retain persistent
  failure/recovery details rather than making them disappear as toasts.

**Avoid:** docks or hover-expanding navigation (the fixed labelled rail solves
that problem), autoplay carousels, elaborate 3D/glass/particle backgrounds,
cursor-following effects and marketing bento grids. They compete with game
art and dense editing work, increase rendering/accessibility cost and conflict
with the restrained workstation direction above. Validate any prototype in
Modern dark/light and keyboard/reduced-motion modes before adopting it.

### Implementation follow-up — 2026-11-16 00:15 EST (UTC-05:00)

The native UI already has shared work snapshots and real worker stages
(`work_progress.rs`, `activity_window.rs`, `play_dialog.rs`), compact sorted
and filtered/paged lists (`asset_library_page.rs`, `modding_page.rs`), reusable
empty states, a paged keyboard-oriented file hierarchy (`explorer_tree.rs`),
and stage/confirmation helpers (`ui_kit.rs`). Recent activity already appears
in the status island and Activity window; the island respects reduced motion.
The Asset Library's pinnable PAK details sheet exposes exact source and
extraction paths, and now links directly to browsing that archive's files.

This follow-up closes the practical gaps: launch stage markers are **not**
presented as a completion percentage; project copy percentage is computed
from actual copied bytes rather than scaled to 95%; empty search results
offer a real clear-search-and-filters action; the selected-project panel
starts compact with Export visible and expands to editing details; failed launch/task
notices link to persistent Activity details; and managed-build deletion
shows the exact target and consequences with an explicit Cancel and
confirmation instead of relying on a hold gesture. Import stages do not
pretend that an idle import is complete. Home offers setup when no game client
is configured and project-creation/browsing paths rather than a dead Open
Project action when no project exists. The activity island and existing
toasts retain subtle motion rather than animating dense archive rows.

No web runtime or new component dependency is added. These patterns do not
change signing, provenance, safety checks or Legacy navigation.
