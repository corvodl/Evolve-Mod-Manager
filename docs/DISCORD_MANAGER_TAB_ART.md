# Evolve Mod Manager — Discord tab icons

The manager Rich Presence **large** image is still `evolve` (manager logo).
Its **small** image is the active `Page` icon. Discord's small-image hover
text uses the page name. The exact source icons are the SVGs below, checked
into `assets/nav` and used by the native manager sidebar.

| Manager screen | SVG source | Discord **small_image** key |
| --- | --- | --- |
| Home | `assets/nav/home.svg` | `tab_home` |
| Instructions | `assets/nav/instructions.svg` | `tab_instructions` |
| Asset Library | `assets/nav/asset-library.svg` | `tab_asset_library` |
| Modding | `assets/nav/modding.svg` | `tab_modding` |
| What's New | `assets/nav/whats-new.svg` | `tab_whats_new` |
| Credits | `assets/nav/credits.svg` | `tab_credits` |
| Developer | `assets/nav/developer.svg` | `tab_developer` |
| Model Viewer / Textures & Models | `assets/nav/box.svg` | `tab_models` |
| Level Editor | `assets/nav/map.svg` | `tab_levels` |
| Settings | `assets/nav/settings.svg` | `tab_settings` |

The remaining `assets/nav` SVGs are action/tool buttons, not persistent
navigation pages: `play-clean.svg`, `play-mods.svg`, `folder.svg`,
`layers.svg`, `light.svg`, `lock.svg`.

## Import into Discord

1. Convert these **existing** SVGs to centered, transparent square PNGs; keep
   the original icon shapes/colors and use one consistent canvas size
   (512×512 or 1024×1024).
2. Upload the ten PNGs to **Evolve Mod Manager**, Discord application
   `1563784115785048114`, under **Rich Presence → Art Assets**.
3. Use the exact lower-case keys above. Confirm the keys in the developer
   portal. Uploading to the separate **Evolve** game app will not make the
   manager's icons available.
4. Ensure `evolve` is also present as the manager's large-image key.
5. Navigate through every page with Discord running and presence enabled.

**Important:** artwork is not uploaded by this Rust change. The IPC payload
may specify an unavailable key, but Discord will omit the icon until its image
has been uploaded. There is no fallback image resolution without that upload.
A custom developer application ID also needs the same assets.

## Asset text rules

- Model Viewer: show the actively loaded model/texture name; show the original
  virtual member path for an active GFX editing session when available.
- Level Editor: show the selected map name or level source filename.
- Asset Library: show the selected archive's exact name.
- Modding: show a selected project's single modified member path, or its
  target PAK when more than one member is modified.
- Empty or unverified selections retain the page's general activity text.
  Never use stale typed paths as proof that an asset is open.
- Absolute user/game paths are not sent. All strings are bounded to Discord's
  activity limits, minimal mode omits asset text, and the separate in-game
  **Evolve** app keeps its character portrait logic and no manager tab badge.

The manager publishes activity updates asynchronously using its existing
Discord IPC worker. No account linking, OAuth or game hooks are introduced.

## Acceptance tests

- Verify the ten tab keys and hover labels.
- Confirm the manager logo is retained as the large image.
- Confirm the selected asset name updates when navigating/opening another
  asset, and disappears when no current selection is verified.
- Verify that minimal/privacy mode does not reveal asset names.
- Verify that while Evolve is running the separate game app continues showing
  its character portrait and does not reuse the manager's tab images.
- Run `cargo fmt --all -- --check`, `cargo test -p evolve-mod-manager`,
  and test a live Discord desktop session after uploading the assets.
