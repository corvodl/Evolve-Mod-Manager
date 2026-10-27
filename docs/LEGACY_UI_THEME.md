# Optional Evolve Legacy interface theme

**Settings → Appearance → Interface Theme → Evolve Legacy** enables the native Iced visual skin. **Modern** remains the default for existing and new installations; the theme changes immediately and is saved per user.

The first implementation applies a restrained original-menu-inspired system across the manager: graphite and near-black backgrounds, steel panel borders, subdued red focus/selection, sharp one-pixel corners, and the white-on-hover/dark-text interaction seen in Legacy menu references. The styling is applied by the normal Iced render system and does not alter launch logic, imported assets, project files or game data. The existing Modern palette, custom accent colors and Light/Dark choice are not overwritten or cleared when Legacy is chosen; returning to Modern restores them.

## User-supplied Legacy UI archive

The original uploaded `UI_Assets.zip` contains one packed `UI_Assets.pak`, which is not itself a conventional ZIP. The newer upload comprises two parts, `UI.zip.001` and `UI.zip.002`, which together form a standard ZIP containing **1,024 entries (113 GFX movies and 849 DDS .0 texture files)** under `UI/`. We inspected the archive without modifying its original contents.

Two very small authentic menu graphics are now bundled in the optional theme as lossless PNGs:
- `assets/legacy/legacy-menu-corner.png`: original `UI/button_I8.dds.0` 32×32 red-corner sprite; displayed next to Legacy Home section headings.
- `assets/legacy/legacy-cursor-art.png`: original `UI/FlashAssets/images/front_end/cursor_evolve.dds.0` 32×32 pointer sprite; displayed beside the Legacy navigation breadcrumb.

Both are embedded at compile time only as part of the optional native skin. The complete ~647 MiB archive, all GFX movies and other game textures are **not** committed or redistributed. Additional atlases, notably `UI/main_menu_I19.dds.0`, `UI/main_menu_I70.dds.0` and `UI/main_menu_ID.dds.0`, are identified for further controlled cropping/integration.

A future asset pass can add the original Legacy button states, panel backgrounds and menu dividers from the user's authenticated extracted `UI_Assets.pak`. The portable visual skin deliberately works without that archive, avoids unverified offsets, and does not present external user-supplied game assets as a bundled resource. It also does not execute the game's Scaleform scripts in native manager controls.

## Intended visual behavior

- Modern: existing panel rounding, Dark/Light toggle, custom colors and accent presets.
- Evolve Legacy: deep black/graphite background, angular panel borders, muted crimson selection, white menu/button hover with dark active text. Legacy stays dark even when the saved Modern appearance is Light.
- Changing the theme affects Home, workspace cards, buttons, Settings panels, sidebar and shared controls without restarting.
- High-contrast active/hover states remain keyboard/mouse accessible; reduced-motion preference remains independent.

## Verification

`settings.rs` tests cover optional-theme migration and preference preservation. `design.rs` tests cover palette isolation, hover contrast, rectangular borders, and switching back to Modern. Run full Windows Iced UI smoke tests after CI for scaling and color contrast on different monitors.
