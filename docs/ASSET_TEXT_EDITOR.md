# In-viewer XML / Lua / TXT editor

The Asset Viewer shares the same indexed file selection used by Modding.
Clicking an extracted `.xml`, `.lua` or `.txt` member opens a multiline
text editor instead of attempting to render it as a model. A separate entry
is exposed under **Modding → Edit → Open XML / Lua / TXT editor** (opens
the selected managed project's explorer). The viewer's **Edit → Open XML /
Lua / TXT** command opens the selected text member.

## Data boundaries

- **Asset Library imports remain immutable.** Opening their extracted
  assets displays read-only UTF-8 source; the Save button is disabled.
- Editing requires an **existing, selected Modding project** whose
  `files` directory exactly matches the indexed viewer root.
- Save goes through `modding_projects::import_raw_member_checked`, the
  same manifest-verified `EditorBackups` transaction used by external
  file replacement. It verifies the original bytes have not changed since
  the editor opened, makes a verified backup, then atomically installs the
  updated project member. A source PAK and an installed game PAK are
  **never** modified by this editor.
- The changed member is picked up by the existing project scan and
  normal single-PAK official encoder/build path. Save does **not**
  implicitly rebuild/sign, activate, or launch the mod.
- Read/write is bounded to regular, UTF-8 sources of at most 2 MiB.
  Symlinks, paths escaping the inventory, binary NUL, compiled Lua
  bytecode and undecoded CryXML are rejected; `Lua` source editing
  does not compile/decompile LuaJIT bytecode. Converted editable CryXML
  members are validated by the existing encoder upon save.
- When a buffer is dirty, selecting another file or closing is refused
  until the user chooses Save or Discard. An externally changed source
  cannot be silently overwritten.

## Test cases

1. Open an editable `ArenaMode.xml`, edit, Save, and verify a new
   `EditorBackups` revision. Build the project; only its target PAK
   should be rebuilt and signed.
2. Open a `.lua` source and a `.txt` source; Save and reload both.
3. Open a read-only XML from an imported Asset Library PAK; verify the
   editor allows browsing text but not Save.
4. Attempt to open LuaJIT binary bytecode or a NUL/binary file: reject
   it with guidance rather than displaying corrupted text.
5. Modify the same member externally while the editor is open:
   Save must refuse, retaining external bytes and unsaved editor text.
6. Edit text and select a different asset: selection must not change
   until the user Saves or Discards. The same applies to closing the
   editor.
7. Check that running game files, signing keys and untouched library
   source PAKs are never modified by text edit operations.

## Known limitation

This is a safe basic UTF-8 text editor, not a scripting IDE or a
compiler. It does not yet offer syntax highlighting, autocomplete,
line numbers or a LuaJIT bytecode decompiler.
