# Optional Evolve Legacy interface theme

**Settings → Appearance → Interface Theme → Evolve Legacy** enables the native Iced visual skin. **Modern** remains the default for existing and new installations; the theme changes immediately and is saved per user.

The first implementation applies a restrained original-menu-inspired system across the manager: graphite and near-black backgrounds, steel panel borders, subdued red focus/selection, sharp one-pixel corners, and the white-on-hover/dark-text interaction seen in Legacy menu references. The styling is applied by the normal Iced render system and does not alter launch logic, imported assets, project files or game data. The existing Modern palette, custom accent colors and Light/Dark choice are not overwritten or cleared when Legacy is chosen; returning to Modern restores them.

## User-supplied Legacy UI archive

The uploaded `UI_Assets.zip` contains **one large `UI_Assets.pak`** rather than a directly extractable folder of DDS and GFX files. That retail game archive is **not** a conventional ZIP, and the original art is not embedded or republished in this PR. It must first be authenticated and imported through the manager's configured PAK handling, then its individual artwork can be mapped to the proper widgets.

A future asset pass can add the original Legacy button states, panel backgrounds and menu dividers from the user's authenticated extracted `UI_Assets.pak`. The portable visual skin deliberately works without that archive, avoids unverified offsets, and does not present external user-supplied game assets as a bundled resource. It also does not execute the game's Scaleform scripts in native manager controls.

## Intended visual behavior

- Modern: existing panel rounding, Dark/Light toggle, custom colors and accent presets.
- Evolve Legacy: deep black/graphite background, angular panel borders, muted crimson selection, white menu/button hover with dark active text. Legacy stays dark even when the saved Modern appearance is Light.
- Changing the theme affects Home, workspace cards, buttons, Settings panels, sidebar and shared controls without restarting.
- High-contrast active/hover states remain keyboard/mouse accessible; reduced-motion preference remains independent.

## Verification

`settings.rs` tests cover optional-theme migration and preference preservation. `design.rs` tests cover palette isolation, hover contrast, rectangular borders, and switching back to Modern. Run full Windows Iced UI smoke tests after CI for scaling and color contrast on different monitors.
