---
name: ui-behaviors
description: Keyboard, focus, dismissal, selection and naming rules for the manager's iced UI (crates/evolve-desktop), adapted from Vercel's UI SDK behavior library. Use when adding or changing a page, dialog, popover, menu, list, palette entry or keyboard shortcut in the desktop app, or when asked to review the UI for keyboard and accessibility gaps.
---

# UI behaviors for the iced desktop app

Adapted from the behavior rules in [vercel-labs/ui](https://github.com/vercel-labs/ui) (UI SDK, Apache-2.0). UI SDK itself is a web library (React, Vue, Svelte, Solid, web components) and can't be installed into this Rust/iced app. What carries over is the behavior contract: every interactive surface gets the same keyboard, focus and dismissal rules. Below, each rule is mapped to where this codebase already implements it.

## Before you build: reuse the kit

Look in `ui_kit.rs`, `design.rs`, `forms.rs`, `chrome.rs` and `toast.rs` before writing a widget. The common needs:

| The person needs to | Use |
| --- | --- |
| Run an action | `design::button`, `icon_button`, `hold_button` for destructive |
| Turn a setting on or off | `switch` |
| Choose one of a few visible options | `segmented`, `segmented_tabs`, `icon_segmented` |
| Enter text, a number, a path | `design::text_input`, `forms::number_field`, `forms::path_field`, `forms::search_field` |
| Label a control | `design::label` above it; `forms::check` for path errors |
| Confirm before an action | `chrome::Modal` via `chrome::modal_overlay` |
| A larger task over the page | `chrome::dialog` |
| Edit beside its trigger | `ui_kit::popover_form` |
| Actions from a button | `menu_panel`, `menu_item`, `menu_entry` |
| Search and run commands | `command_palette` (register a command there) |
| Reorder a list | `reorder_list` (Alt+↑/↓ already wired) |
| Name a control or add a hint | `design::tooltip`, `key_hint`, `design::kbd` |
| Show and hide a section | `ui_kit::disclosure` |
| Notify | `toast` |
| Progress | `meter`, `skeleton`, the footer work indicator |

Don't hand-roll a second version of any of these.

## Keyboard

All global keys are in `App::subscription` (`iced::event::listen_with`) in `main.rs`. Rules:

- **Respect `event::Status`.** A key a focused widget used (typing in a text field) arrives as `Captured`. Page shortcuts, camera keys, Enter-to-confirm and Delete must match only `Status::Ignored`. Only true global shortcuts (Ctrl+K, Ctrl+B) ignore status.
- **Esc closes the topmost layer, one per press.** The order lives in `Message::ViewportEscape`: modal → palette → popover → lightbox → map lightbox → sidebar panel → side sheet → transform / fullscreen → clears the window's search → closes menus. A new layer gets a line in that chain at its z-order, not its own Esc handler.
- **Enter confirms** the open modal (`chrome::Msg::Enter`) when focus isn't in a field. Show `design::kbd("Enter")` / `kbd("Esc")` beside the buttons, as `modal_overlay` does.
- **Tab / Shift+Tab move focus** through text inputs and editors in the current window only, wrapping at the ends (`focus_scope.rs`). iced's own `focus_next` walks every window, so never call it directly.
- **Arrow keys in a list move the highlight and wrap**; Enter runs it; hovering moves the same highlight (see `command_palette::Msg::Move` / `Hover`). Key a list's highlight by the item's id when the list can change underneath it; an index is only safe when the list is rebuilt on every query, like the palette.
- **Every new shortcut is discoverable**: add it to `chrome::SHORTCUTS` (tooltips and menus read hints from there) and to the command palette.

## Focus

- **Opening a surface with a text field focuses it** (`iced::widget::operation::focus(id)`), like the palette does on `Msg::Toggle`.
- **Overlays trap focus.** Tab stays inside the topmost of: the palette card (`focus_scope::PALETTE`), a `chrome::dialog` card (`focus_scope::DIALOG`), the window root (`focus_scope::window(id)`). A new modal surface built without `chrome::dialog` puts `.id(focus_scope::DIALOG)` on its card container.
- **Focus never strands.** After deleting or hiding the focused row, move the selection to the next row (or the previous one at the end), not to nothing.
- **Clicking the backdrop dismisses** a palette or dialog (`chrome::dialog`'s `outside` message), and the backdrop swallows hover and scroll (`Interaction::Idle`).

## Naming and state

- **Name every control.** An icon-only button gets a `tooltip` with its action; a field gets a `design::label` above it. People skimming and agents driving the app both find controls by name.
- **Explain disabled controls** with `ui_kit::why_disabled` instead of silently greying them out.
- **Style from state.** Hover, pressed, selected and disabled come from the widget `Status` in the style closure and the theme tokens in `design.rs` (`selection`, `surface`, `border`, `tone_color`), never hard-coded colors per call site.
- **Check contrast.** Text needs 4.5:1 against its background, and control edges, focus rings and selected fills 3:1. Check both light and dark themes and the theme presets.
- **Honour `preferences.reduce_motion`** for any new animation.

## Code style for UI changes

From UI SDK's "unslop" pass:

- Comments say why (an iced quirk, a trade-off, a game constraint), in three lines at most. No comments restating the next line, no section banners, no history ("now we…", "fixed…").
- No defensive checks the types rule out, no one-caller wrappers that only rename, no options nobody passes.
- Names say what a thing is: `highlighted_row`, `on_confirm`, not `data`, `temp`, `flag`.

## When you're done

1. `cargo fmt -p evolve-mod-manager` and `cargo check -p evolve-mod-manager --tests`.
2. Walk the change with the keyboard only: Tab through it, Esc out of every layer you opened, Enter on its confirm.
3. Say what you couldn't check, for example tests that need an elevated shell.
