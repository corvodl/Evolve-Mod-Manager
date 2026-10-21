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


## Interactive ActionScript preview (experimental)

The GFX pane now has separate **Design / layout editor** and
**Interactive ActionScript preview** controls. The former uses conservative
first-frame layout editing and staged Undo/Apply exactly as before.

Interactive preview uses **Ruffle desktop** (`ruffle.exe`). When the
optional GFX tools are selected during Windows installation, the manager
automatically detects its bundled path. Users of the portable ZIP or
installations without the GFX tools option can select a separately installed
Ruffle executable. Click **Launch / restart interactive preview** to open
a separate Ruffle window. Close that window or click **Stop preview** to
end the process.

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
- Runs an **empty ExternalInterface** in Ruffle and blocks URL launches,
  without sending commands to CryEngine or a real Evolve session.
  Local filesystem access uses Ruffle's prompt-based mode for imports.
- Owns the child process and temporary working directory, and stops it
  when the user stops/restarts the preview, opens another GFX, or stages
  a movie-changing operation.

**Limits:** Ruffle is an independent Flash emulator, not Evolve's original
Scaleform runtime. It does not recreate CryEngine's native GFx callbacks,
game state, menu data, game-specific UI services, or missing imported
assets. The dummy ExternalInterface returns no Evolve-specific values.
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
Ruffle v0.7.1 Windows x64 (20.5 MiB ZIP) plus FFDec v26.3.0 portable
(19.0 MiB ZIP). They are compiled into the installer only and can be
omitted at installation. Setup adds no third-party shortcuts or services.
Ruffle and FFDec remain independent upstream products distributed under
their own licenses; notices and source locations are installed alongside
them. The portable release intentionally does not include third-party
executables; use separate tools with the existing picker there.
