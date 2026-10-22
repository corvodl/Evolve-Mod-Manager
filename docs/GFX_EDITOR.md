# Scaleform GFX editing and layout previews

The public Rust/Iced manager uses the private Qt manager's GFX editor as the
reference. This is an **isolated, opt-in editing workflow**, not an in-game UI
runtime or a replacement for FFDec.

## Working with a GFX

Open a `.gfx` in the extracted Modding file browser. The manager inspects its
native Scaleform header, stage dimensions, tags, frames, imports and resource
identities. The **Windows installer** offers an optional, preselected **GFX
tools** task that bundles pinned FFDec 26.3.0 and Ruffle 0.7.1 for an
additional approximately 40 MiB of compressed download size. If selected,
the manager detects `{app}/third_party/FFDec/ffdec-cli.exe` and
`{app}/third_party/Ruffle/ruffle.exe` automatically without overwriting
existing custom executable paths. The portable ZIP remains small and users
can still select external installations through the file pickers.

The tooling binaries are downloaded during release packaging from the
official upstream projects, pinned and SHA-256 verified; they are **not**
committed to the repository. Upstream license notices are distributed with
the tools. FFDec needs a compatible **Java runtime** on the user's PC (it
is not bundled). No proprietary Evolve assets, RSA keys or original GFX
files are bundled.

Resource replacement, full FFDec editor launch/reload, editable ActionScript
and tag XML round trips, and PNG static-frame preview all work on temporary
copies. **Undo staged edit** restores the previous working movie. Edits reach
a manifest-listed, extracted Mod Project only after explicitly selecting
**Apply to Mod Project**. The existing hash-check and `EditorBackups` transaction
protects the original; this operation never rewrites an installed PAK.

## Visual layout editor

Use **Visual GFX layout editor** in the GFX pane:

- The hierarchy lists first-frame root placements, sorted by depth and real
  character IDs. Names are taken from ExportAssets when authored.
- Native `DefineShape` bounds are outlined in stage coordinates. Select
  and drag those shapes to update X/Y. Optionally show a static, pre-rendered
  FFDec PNG behind outlines; it is never embedded into GFX edits.
- X/Y position, independent X/Y scaling (%), rotation in degrees and
  visible/hidden are editable numerically. Click **Stage selected layout edit**
  to write a validated MATRIX to the isolated movie. Hide removes only the
  one unambiguous first-frame root placement tag.
- Nonvisual sprites/images without verified shape bounds are **numeric-only**.
  Duplicate-depth placements, skew/reflections and PlaceObject3 use diagnostic
  read-only rows. Nested sprite timelines and dynamically created content are
  not edited or guessed.
- **Undo staged edit** and **Apply to Mod Project** remain the same actions as
  the existing FFDec editor. This uses native GFX/CFX reserialization,
  preserves unrelated tag payloads, and bounds matrix bit widths and
  decompression sizes.

A passing synthetic binary-roundtrip test establishes conservative tag editing,
**not** a successful in-game Evolve Scaleform layout result.

## Dependency-aware FFDec rendering

Imported GFX/SWF movies are copied into the temporary FFDec directory only
when the source is a verified local file or a uniquely resolved exact virtual
path in the **selected Asset Library source build**. A compiled GFX/CFX
dependency may be reheadered to FWS/CWS **in the temporary preview copy only**
when an import references SWF and that compiled GFX equivalent is present.
Unresolved, ambiguous or cross-build dependencies are shown in diagnostics.
No game install or imported archive is rewritten.

The **Render selected frame** action now reconstructs recognized Scaleform
external bitmap tags 1008/1009 using matching UI_Assets DDS/DDS.0 textures.
Only exact same-folder or unique manifest-verified matching-build textures are
used. The original shape fills, transforms, nesting, masks and clipping tags
are left untouched, and generated standard SWF bitmap definitions are saved
**only in the temporary FFDec rendering copy**. UI_Data UIElement XML is
resolved as a same-build association, not a source of invented placements.
The status reports how many bitmap characters were recovered/skipped.

This improves the authenticity of button artwork, but it is still a *static*
frame: ActionScript-driven states, hover/pressed transitions, runtime-generated
text, fonts, animations and game callbacks are not executed and cannot yet
be guaranteed pixel-identical to the live game.

## Live UI inspector status

The **Live UI Inspector (read-only)** validates and displays the private
inspector's schema-1 hierarchy snapshots (`movie`, `frame`, up to 128 nodes).
Use **Load externally captured UI snapshot JSON** to inspect a saved snapshot.
JSON is restricted to 12 KB, unique identities, known node types, finite
geometry, valid parents, and acyclic hierarchies.

**Actual live Evolve menu capture is not yet provided by the public manager.**
The private project's read-only event bridge also lacks a verified
build-specific native GFx traversal provider. The public manager therefore
does not fabricate live nodes, hook the game, traverse arbitrary memory, or
expose developer-gated memory tools. A future implementation must explicitly
verify the exact offline game build, its game-thread provider, authenticated
loopback response and safe process boundaries before connecting live capture.

## Validation and limitations

Automated Rust tests cover binary GFX/CFX edits, shape bounds, rotation/scale,
visible/hidden, malformed inputs, read-only hierarchy validation, dependency
path checks, and canvas hit testing. FFDec and original Evolve GFX source files
are required to verify real render quality and game-side behavior on Windows.
Runtime ActionScript callbacks, localization, buttons created in code and
video playback cannot be faithfully rendered from a static movie.


## Why the original Evolve main menu is partially blank in Ruffle

The menu is **not** a self-contained, static SWF screen. A working original
ActionScript callback (such as a patch-note overlay dismiss) only proves that
its AVM handler ran. The other visible screen elements still depend on
CryEngine-hosted UI_Data initialization, localization, fonts, imported GFX
movies, atlas DDS assets and game state.

Symptoms including `BUILD INFO GOES HERE`, raw `@ui_STARTGAME` placeholders,
an otherwise black screen, and outlined but unfilled buttons indicate that
the original movie is running **outside its native game host**. The
standalone Ruffle player cannot supply these native conditions by itself.

The optional **Source UI_Data menu** button in the live preview opens a
separate read-only command inspector. When `UI/MainMenuScreen.xml` exists in
the selected build's verified Asset Library, this inspector shows its original
Main and other nested menu sections, following only explicitly declared
`child_submenu` links. It never invents a Flash display list, localization,
or a runnable native button handler. CryEngine-bound buttons are visibly
disabled. Closing the inspector returns to the same running movie.

The inspector's resource section reports missing GFX/SWF imports (including
font libraries), unresolved bitmap tiles, and whether the exact source build
and Asset Library are available. Imported movie names are taken from the
original compiled GFX. Ruffle `FSCommand` calls are displayed as blocked
diagnostics; ActionScript traces that report missing resources/errors can
also appear in the browser status line. No game process or DLL calls are made.

For full visual parity, source-build-specific localization such as the
original `text_ui_*.xml` resources, verified font imports and UI_Data setup
need to be integrated and validated against the actual movie. The inspector
is a diagnostic/navigation tool, **not a claim of full menu rendering**.

## Live Ruffle Web preview (PR #23)

For GFX files open **GFX Tools → Preview → Ruffle controls**. **Open live preview**
starts a single browser-based Ruffle Web player at a randomized `127.0.0.1`
address. It serves only the isolated edited SWF, its staged imported movies and
the SHA-256-verified selfhosted Ruffle Web distribution bundled by the optional
GFX Tools installer task. The browser needs no CDN, no internet service, and
never receives Evolve game process/DLL access. Desktop `ruffle.exe` remains
available for standalone movie testing, but does not expose a live control API.

Once the live player is open, clicking an ActionScript action in GFX Tools
**uses the same running movie**; it does not patch another call into a new
SWF, restart the window or reset the menu. Clicking an action first can also
open the player and queue that action. The browser's footer reports one of:
*original ActionScript callback invoked*, *callback unavailable*, or *runtime
error*. Invocation confirms the callback returned from ActionScript; it does
not promise an in-game visual transition if native Evolve UI_Data, game
services, localization or imported movie states are missing. **Reset movie**
reloads inside the same browser page.

The manager registers only original first-frame, arity-verified
`main_menu.gfx` AVM1 handlers via
`flash.external.ExternalInterface.addCallback`. The browser calls those
names using Ruffle's `callExternalInterface` API. If a handler cannot be
verified, the manager rejects the command rather than pretending the button
worked. Unmodified GFX bytes remain in the editable session; only temporary
render-only SWF bytes receive the callback wrapper.

**Security:** the preview server binds only IPv4 loopback and uses a
randomized per-session URL; HTTP serves an allowlisted set of SWF and
Ruffle WASM/JS assets, with no upload, file-write, game-launch, or native
host command endpoint. Closing/stopping the session terminates the local
server. Browser tabs opened by the system default browser may remain open
after stopping; close them normally. Ruffle and the original movie remain
subject to Flash emulator limitations.

**Prerequisites:** choose the optional **GFX Tools** component when
installing, which includes both Desktop Ruffle and the additional pinned
`third_party/RuffleWeb/ruffle.js` bundle. Portable users need that bundled
selfhosted folder alongside the application for live mode. An older installed
Ruffle desktop executable alone cannot enable live buttons.

## Interactive ActionScript preview (experimental)

The GFX pane now has separate **Design / layout editor** and
**Interactive ActionScript preview** controls. The former uses conservative
first-frame layout editing and staged Undo/Apply exactly as before.

Interactive GFX Tools actions now use **Ruffle Web** in a persistent local
browser tab, loaded from the installer-bundled selfhosted Ruffle Web package.
Press **Open live preview** to start; subsequent verified menu actions are sent
to the same running player. Use its **Reset movie** button to reload the SWF
without opening another window. Standalone Ruffle Desktop remains selectable
for independent SWF inspection, but does not accept these live commands.

The manager:
- Validates and reads the **staged working GFX** (not the installed game).
- Uses the same manifest/build-scoped `UI_Assets` atlas reconstruction
  as the static FFDec preview.
- Validates same-build imported GFX/SWF movies already staged in the
  isolated FFDec session; converts compiled Scaleform GFX/CFX headers
  to Flash SWF headers for runtime inspection **only in temporary files**.
- Preserves original ActionScript bytecode, tag ordering, timeline states,
  shape placement, and button definitions wherever the Flash emulator
  understands their instructions.
- Registers only source-verified AVM1 callbacks in an isolated Web Ruffle
  runtime; commands never reach native Evolve game services or an installed
  client. Network requests are restricted to the local preview origin.
- Owns the temporary local HTTP server and working movie directory; stopping
  the preview or opening another GFX drops its server and isolated assets.

**Limits:** Ruffle is an independent Flash emulator, not Evolve's original
Scaleform runtime. It does not recreate CryEngine's native GFx callbacks,
game state, menu data, game-specific UI services, or missing imported
assets. Only allowlisted local menu handler invocations are supported; no native
Evolve ExternalInterface/game-service implementation is provided.
Some ActionScript 1/2/3 constructs and Scaleform extensions are unsupported.
A menu may animate or respond to mouse input without populating real
button lists or navigation. No fake button data or guessed image placements
are used. Exact parity requires a verified callback catalog, a compatible
host bridge, matching `UI_Data` and `UI_Assets` files, and comparisons
to the **same frame and input state** in-game.

Tests cover unsafe runtime paths, unsafe imported movie names and invalid
Flash signatures. End-to-end interaction requires a locally installed
Ruffle desktop build and real Evolve files; tests do not claim that every
game GFX can run in this environment.

### Bundled tools and installation size

The installer offers a **GFX editing tools** option (preselected, optional):
Ruffle Desktop v0.7.1 (20.5 MiB ZIP), Ruffle Web v0.7.1
(10.5 MiB ZIP), plus FFDec v26.3.0 portable (19.0 MiB ZIP). They are compiled into the installer only and can be
omitted at installation. Setup adds no third-party shortcuts or services.
Ruffle and FFDec remain independent upstream products distributed under
their own licenses; notices and source locations are installed alongside
them. The portable release intentionally does not include third-party
executables; use separate tools with the existing picker there.

## Settings → Integrations: FFDec and Ruffle

The Integrations page now lists **Blender**, **FFDec**, and **Ruffle**.
The existing Blender detection remains unchanged. FFDec/Ruffle report their
configured executable when available and show an **Install** action only
when missing. The **Select existing…** picker and editable executable path
remain available even when installed.

The Install action downloads the pinned official portable ZIP (FFDec 26.3.0 or
Ruffle 0.7.1 x64), checks the expected SHA-256, rejects invalid archive
entries/symlinks and oversize archives, and installs into the manager's
`third_party/FFDec` or `third_party/Ruffle` directory. An existing directory
is never overwritten by an automatic install. This operation is initiated only
by a click; it does not install tools at startup or contact upstream without
an explicit request. Downloads run off the interface thread. FFDec requires
Java separately; Ruffle is a Flash emulation preview and does not reproduce
Evolve native callbacks. Existing custom tool paths retain priority over
installer-bundled executables. Install actions are Windows-only.

The protected Program Files installation may require administrator privileges,
consistent with the manager's existing elevated Windows manifest; portable
builds use their own executable folder. Users can always choose a manually
installed executable without using the built-in downloader.

## Docked GFX Tools workspace

Opening an indexed `*.gfx` selects a contextual **GFX Tools** tab in the
existing right-hand sidebar, beside Files and Model Options. The left/main
pane is reserved for the **original GFX image** or draggable design canvas;
it no longer contains a column of FFDec/Ruffle controls. The resizable
explorer divider and its left/right placement setting still apply.

GFX Tools is split into six compact sections:

- **Preview:** static FFDec rendering, FFDec executable picker, standalone
  Ruffle launch and status. Ruffle playback is separate from the static
  viewport image.
- **Layout:** root first-frame layer selection, raster backdrop toggle,
  transform inputs, visibility and staged layout edits.
- **Scripts:** FFDec ActionScript and tag XML export/rebuild commands.
- **Resources:** verified imported GFX/SWF references and resource ID
  replacement.
- **Inspector:** externally captured, read-only UI hierarchy snapshots.
- **Project:** staged GFX import, undo, editing folder, and Apply with backups.

Switching between the tabs **does not apply or discard edits**. Layout
selection activates the layout canvas; other sections return to the
FFDec frame preview. The GFX Tools tab is available only during a GFX
session; selecting other preview types returns to Files.

### Why the front menu still needs native callbacks

For `Libs/UI/main_menu.gfx`, the original GFX may create a login,
challenge, or other front panel from game-provided state. This **cannot be
identified from a screenshot**. The manager deliberately does not map an
unknown button to "close" or hide an arbitrary sprite: doing so could give
users a preview that differs from the game.

Ruffle desktop is currently started with an **empty ExternalInterface**,
so source-authored buttons requiring CryEngine's GFx host callbacks do not
perform those game-side transitions. Static rendering can also miss fonts,
localized text and specific external DDS symbols. A faithful close button
requires the exact decompiled button handler and UI_Data event binding (and
its full imported movie chain), followed by a preview-only, allowlisted
host event simulator. Gameplay/network operations must never be executed.


## Main menu original-ActionScript preview presets (PR #21)

The original user-provided `Libs/UI/main_menu.gfx` and its `UI_Data.zip`
were inspected before enabling preview controls. The movie includes compiled
root-frame ActionScript references to `showEngagementPrompt` and
`showSigningInPrompt`, and the UI definition binds `main_menu.gfx` to
the MainMenuNew GFx element. From GFX Tools → Preview → Ruffle controls,
select **Unmodified movie**, **Show/Dismiss engagement prompt**, or
**Show/Dismiss sign-in prompt** to dispatch immediately to the running
Ruffle Web player. No restart is needed.

The live player registers narrowly allowlisted AVM1 callback wrappers in its
**temporary Ruffle SWF only**, after the original first-frame root handlers
have been declared. Buttons call the registered wrappers later, after the
running movie has initialized. The original GFX file, its resource tags, extracted project, and
installed PAK are not modified. The movie's own ActionScript still controls
the visible prompt; no image layer is hidden by arbitrary depth or coordinates.
An unavailable or dynamically resolved original handler fails closed rather
than masquerading as a functional button. Presets are not a claim that the
native in-game button-to-handler relationship is fully traced.

**Still not emulated:** Evolve game services (network, purchases, matchmaking,
account, launcher), Scaleform native callbacks, fully populated localization
and menu data, and complete clickable UI parity. The old desktop fallback uses a dummy ExternalInterface. Live mode instead
uses only the allowlisted browser preview callback bridge; simply detecting a
function's name does not verify every corresponding game UI transition.


### Source-verified local UI actions and UI_Data catalog

The user-supplied original `Libs/UI/main_menu.gfx` root DoAction was analyzed
for AVM1 `DefineFunction` and `DefineFunction2` declarations, including their
actual argument counts. Preview-only presets now include **Back**, **Return to
main menu**, **Show/Dismiss patch notes**, **Show/Hide news**, and **Enable/Disable
challenge controls**, in addition to both engagement and sign-in prompt toggles.
A callback is registered in the temporary SWF when the original root
function and arity are verified. Buttons call it live after initialization,
without relaunching; the browser reports whether the callback returned.
The game's native UI state remains unavailable, so a dispatched call is
**not** proof of end-to-end screen parity.

**GFX Tools → Inspector** now lists menu buttons from a verified, exact-build
`UI/MainMenuScreen.xml` in the Asset Library. It displays the command,
submenu and conditional disabled/hidden status. This is intentionally
read-only: e.g. `OnlineMultiplayer`, `Store`, `Exit`, and tutorial launch
commands are game operations and never forwarded from an interactive preview.
Entries with a `child_submenu` show the original submenu destination, but
this alone does not prove Ruffle can reproduce the game's menu state machine.
Missing/ambiguous UI_Data definitions fail closed rather than using button
maps from another version. No proprietary UI_Data XML is bundled with the app.

The remaining parity work is a game-independent local menu-state host that
feeds the original ActionScript's expected arrays and events, plus a sandboxed
replacement for native callbacks. This requires tracing the exact button
callback and testing the associated animation frames and imported movies. The
current standalone Ruffle desktop executable has a dummy ExternalInterface,
so this work does **not** claim arbitrary native button commands function.
