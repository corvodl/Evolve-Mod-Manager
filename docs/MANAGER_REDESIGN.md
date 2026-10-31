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
