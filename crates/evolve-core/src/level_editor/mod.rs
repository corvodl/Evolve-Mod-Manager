//! UI-free Level Editor logic: the unsaved-edit draft and how it is saved,
//! committed undo history, terrain and vegetation brushes, and map
//! environment, lighting and material sources read from verified game data.
//! The desktop app draws and drives these; `evolve-cli level` uses them
//! headless.
pub mod level_archive;
pub mod level_atmosphere;
pub mod level_camera;
pub mod level_draft;
pub mod level_edit_history;
pub mod level_effect_sources;
pub mod level_environment;
pub mod level_lighting;
pub mod level_save;
pub mod level_transform;
pub mod level_validation;
pub mod terrain_brush;
pub mod terrain_material_probe;
pub mod terrain_rebake_project;
pub mod vegetation_map;
