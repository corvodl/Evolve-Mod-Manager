## 2026-10-25 — Nightly 59 controlled-swap preparation error reporting (PR #79)

- **User's observed result:** Nightly 59 now displays the native Game Launch window, but its only cause is `Launch failed: Private controlled_swap.py prepare failed: exit code: 1`. This is a **Python preparation failure before any verified PAK swap**, not proof the GFX itself is broken, and not evidence that original files require Repair/Verify.
- **Confirmed public Rust defect:** `launch_existing_offline_client` used `Command::status()` to invoke `controlled_swap.py prepare`, discarding all of the Python helper's stdout/stderr. The helper deliberately catches failures and prints `STOPPED: <exception type> <detailed reason>` to stderr. With Windows' hidden subprocess, the user could not see that reason and the popup only had an exit code. **The screenshot does not identify which of the Python helper's checks failed.** Possible but unconfirmed causes include a leftover .partial/.ready file, a recovery journal phase conflict, inadequate game-volume free space, unavailable source PAK, or incompatible shim keys.
- **Change:** Use hidden `Command::output()` with a null stdin, set the helper working directory, write both helper streams to `rust-prepare-bridge.log` in the existing controlled-swap helper folder, and return the bounded actual `STOPPED:` reason to the manager's native launch-error dialog. Show context-specific recovery hints for missing space, running game processes, backups/recovery, leftover ready/partial files, and key mismatches. Never interpret exit code 1 as a specific issue without the emitted Python cause; if logging fails the popup still shows the Python diagnostic. Add synthetic Rust regression tests for the full stopped-reason and fallback behavior.
- **Protection:** No change to PAK swaps, backup/journal processing, signing keys, automatic originals recovery or cache-quarantine logic; no override of Python prepare refusal. The next in-game attempt with this change must provide the actual `STOPPED:` reason, after which any additional repair should be scoped to that verified failure. No live Evolve verification claimed.

## 2026-10-24 — Fix stale prepared-copy refusal and native Play status dialog (PR #78)

- **Observed failure:** During rapid Stage 2/Legacy GFX mod testing, Play with Mods fell back to regular Evolve with `A prepared mod copy changed unexpectedly. Restore original game files first.` This originated in the public manager's `refresh_prepared_offline_cache` comparing an existing disposable `.customkey-ready` file to the *previous* immutable prepared journal's SHA-256/byte count and failing before rebuilding the current enabled mod set. That specific error did **not** establish that an original game backup or installed PAK was damaged.
- **Safe recovery implementation:** Accept only controlled-swap journal phases `prepared`/`restored`, require Evolve and the launcher closed, validate full game/stage/target/source mapping via `guarded_restore_state`, reject any `.customkey-original` backup, unsafe file kind, changed paths, or non-matching journals before touching *any* ready file. An unchanged `.customkey-ready` is a disposable previous cache and can be deleted; unexpected ready bytes are preserved via a unique same-directory `.customkey-ready.stale-*` name, never silently deleted or treated as the new signing source. Mark the journal `restored` after successful cleanup so the existing private helper can rebuild prepared copies from the currently signed isolated stage. Independent PAK/hook verification and rollback remain mandatory. No original installed game PAK or recovery backup is rewritten by this logic.
- **User feedback implementation:** Native `Game Launch` window appears for Play with Mods / Play without Mods and explains preparation, verifies when the hook is armed (which is *not* a completed swap), shows confirmed completion only after swap/game release/helper exit, and prominently displays fallback warnings or actual errors rather than relying on the main footer. Missing setup, busy state and unsaved GFX are surfaced through this window; **any other worker-operation failure** also opens the native status/error dialog. Users can close the window; the launch worker and recovery checks continue.
- **Validation limits:** Unit tests assert that quarantine preserves modified ready bytes and installed originals, and that a preparing hook cannot be mistaken for confirmed play. Windows CI and live Evolve testing remain required. Confirm repeated GFX Test 05/06 changes no longer trigger the stale prepared-copy refusal; deliberately corrupted game backups or journals must still fail safely. The renamed stale copies consume disk space until reviewed and should not be mistaken for recovery backups.
- **Release:** Changes committed on PR #78, **not merged and not published as a nightly**.

## Developer health candidate discovery (next PR after Nightly 39)

- Add read-only, process-bound health candidate finding inside a **single 4 KiB page containing a user-specified address**. It decodes float32 or unsigned 4-byte integer fields, with optional exact expected health and subsequent Decreased/Unchanged/Increased/Changed refinements as combat health changes. Maximum 1024 initial aligned candidates, first 20 shown. The old Python inspector provides the reference approach, not validated Stage 2 actor offsets.
- Query the actual running Evolve.exe executable path on Windows instead of assuming `bin64_SteamRetail/Evolve.exe` under a configured game root. Query its module base when permitted and accept `Evolve.exe+0xOFFSET` syntax in the read-only inspector and bounded scanners. Missing module-base information fails closed for that syntax.
- This is **candidate detection**, not automatic actor/class recognition. No stable actor signatures, struct layouts, health offsets, or authenticated offline session verifier are available yet. Matching a decreasing value cannot prove it belongs to the monster. Real offline Evolve tests and version-specific class/health identification remain necessary.
- No writes, memory injection, spawn commands, persistent memory results, or network activity are added. All candidates are invalidated after process re-detection, address/value edits or developer locking. Windows acceptance still pending.

# Public Rust manager parity — continuation notes

## 2026-10-23 — Developer runtime phase 2/3 previews (new PR)

- Developer page now allows bounded, typed *previews* for health (float), cooldown (float), teleport (three floats), and weapon reference (aligned u64). It validates hexadecimal addresses and reports the proposed little-endian bytes without ever modifying the game. This is an implementation aid for verifying unknown game layouts, not a functional memory editor.
- Phase 3 supplies strict, bounded input validation for proposed entity spawning (1-32), character selection and ability identifiers. It produces inert previews with explicit missing engine class/authority prerequisites; it cannot execute engine functions.
- These tools explicitly fail closed until trusted exact-build actor pointers, an offline-only session guarantee, baseline compare-and-restore transactions and engine class initialization are available. No DLL injection, game write, RMI transmission or live spawning is performed.
- Added private-manager-compatible Ghidra AOB signature matching (including ?? wildcards, overlapping matches and bounded results), exposed as an opt-in 4 KiB read-only Evolve process window scan. These are candidate addresses only and not proof of valid engine functions.\n- Tests cover finite/range checks, bad pointers, class identifiers, excessive spawn counts, signature bounds, and non-execution. Actual Evolve in-game acceptance remains outstanding.

## 2026-10-22 — Developer runtime phases (PR #54)

- Phase 1 now has an opt-in, password-gated external Evolve.exe discovery and read-only 32-byte hex/pointer-chain inspector using a safe Windows process-memory API. No automatic game offsets are guessed. Multiple processes block selection, and the process is rechecked before and after each read. This is a developer diagnostic, not a functioning health/inventory inspector yet.
- Phase 2 is explicitly locked: health, cooldown, teleport and weapon-reference edits require verified game/build layouts, provable offline-only session gating, recoverable writes, and reset/revalidation on match transition. No memory writes implemented.
- Phase 3 is research-only: native spawn, actor switching, custom abilities and RMI/engine calls are not supported. No native hooks, DLL injection or network traffic are performed.
- Real Windows game-process acceptance and CI remain required. The presence of Evolve.exe alone does not prove offline state or running binary identity; do not enable writes based solely on PID/name.


Updated: **2026-10-20** · Repository: `corvodl/Evolve-Mod-Manager` · Working PR: **#6** on `feature/blender-viewport-cdf-autoload`, based on PR #5's `feature/pr5-combined-viewer-preview`. This document is a resumable engineering checklist, **not** a claim of production parity. See [PARITY.md](PARITY.md) for the broader feature matrix.

## 2026-10-23 — original Training cards: PNG URL suffix and live request audit

Nightly 34 confirmed that the original `modeselect` transition selects the
source-qualified `lobby.usm` video, but the image-card display list remains
blank. The runtime diagnostics showed 24/24 MenuModeSelect DDS images
converted and staged, 24 SWF bitmap linkages, and one ImageLoading
`MovieClipLoader.loadClip` `img://` prefix rewrite. **Those counters
measure file preparation, not successful Flash image loads.**

The next isolated preview patch rewrites the source-authored, verified
MenuModeSelect `.dds` suffixes to `.png` in AVM1 DoInitAction constant
strings, in addition to the existing equal-length `img://` → `./img/`
rewrite. No original game files are changed; each rewrite is four bytes
and retains AVM1 bytecode positions. This makes Ruffle's image loader
request browser-native PNG URLs with a matching PNG HTTP response rather
than a .dds-looking URL served as PNG. Only names actually staged from
the exact imported build are eligible. An inspection of the supplied
original `main_menu.gfx` found exactly 24 authored
`Libs/UI/FlashAssets/images/MenuModeSelect/<name>.dds` constants.

The temporary local preview server now records **actual HTTP requests**
by the Flash card loader, their success/failure, and a bounded set of
requested card names. `Check preview assets` surfaces those values in
addition to the existing browser PNG-decode check. If the counters stay
at zero after entering Training, the next issue is ActionScript clip
initialization/path construction. If requests succeed but no cards
appear, inspect `mcButtonImage` display-list placement and
`MovieClipLoader.onLoadInit` visibility/size callbacks, not image
staging. This change alone is **not** proof of card or hover parity:
it needs an installed-build replay against the original GFX.

## 2026-10-22 — original front-end video transitions in Ruffle preview

The main-menu preview now supports the original Flash `updateVideo(name)`
request rather than leaving the Goliath movie looping behind every screen.
The temporary first-frame AVM1 instrumentation verifies an exact one-argument
`updateVideo`, saves the original handler, traces `evolve-video:<name>`, and
then invokes the unchanged handler. Ruffle Web listens for this trace and
switches its independently decoded, non-interactive video layer. Verified
local UI_Data submenu push/goBack controls also request the corresponding
video, including returning to the original default Goliath background.

The menu uses its known `FrontEndVideoList` mapping only:
`modeselect -> lobby.usm`, `options -> options.usm`,
`founderspopup -> FoundersPopup.usm`,
`arena -> front_end_arena.usm`, and
`hunt -> front_end_hunt.usm`. Sources must be unique regular CRID
files within the selected build's verified imported `videos.pak`
`front_end` namespace. The large `mainmenu_<character>.usm` family is
never used as an alternate fallback. Missing Arena/Hunt video names stay
missing rather than switching to unrelated `videos/ambient` clips.

The new loopback video endpoint converts only the requested video, on
demand, with a configured FFmpeg executable or reads a verified
`<source-stem>.preview.mp4`/`.webm` sidecar beside that source.
Staged derivatives stay in an isolated temp directory; no copyrighted
USMs are committed, returned by HTTP, or modified. Video requests and
previews are keyed by a fixed whitelist. Browser request ordering protects
against a slow older conversion overwriting a newly selected screen.
Unavailable sources or invalid decoders emit an explicit diagnostic,
not a simulated placeholder background.

**Remaining verification:** Source runtime `updateVideo` instrumentation
needs actual main_menu.gfx/Ruffle playback confirmation. The server currently
handles conversion synchronously with its bounded timeout; it can make the
local preview temporarily slow on the first request for a large clip.
Native game services, account and matchmaking state, and sound from USM
remain deliberately outside this preview. The fallback UI_Data navigation
path does not claim native menu-command parity.

## 2026-10-22 — source MovieClipLoader cards and legacy-menu replacement plan

The supplied original Stage 2 `main_menu.gfx` confirms the next
image-card blocker. Its `setupMenuBtns()` creates `mcButtonImage` and
`mcModeImageFocus` / `mcModeImageUnfocus` clips, looks up authored
`scripts.asset_helpers.ImageLocationData.MenuModeSelect` values, and
calls `ImageLoading.loadBitmap()` for both artwork states. The actual
`ImageLoading` DoInitAction class uses
`MovieClipLoader.loadClip("img://" + source_dds, holder)`, not
`BitmapData.loadBitmap()`. Merely embedding 24 SWF ExportAssets
linkages does not satisfy the original movie's `loadClip` requests.

For the read-only main-menu preview only, convert the single compiled
`ImageLoading` constant-pool URL prefix from `img://` to equal-size
`./img/` in the *temporary* FWS. The local tokenized server now
maps only `img/Libs/UI/FlashAssets/images/MenuModeSelect/<safe>.dds`
to the already staged, same-build `runtime-images/<safe>.png`.
No source PAK, GFX, or image is modified; all other `img://`
families and game services stay unsupported. Source handler, artwork,
focus/unfocus and timeline are retained. The preview manifest exposes
the number of rewritten source MovieClipLoader URL prefixes.
Acceptance still requires a real Ruffle session with cards and
mouse hover, since web PNG decoding via `MovieClipLoader.loadClip`
has not been verified against this movie.

Other findings in the same original GFX: the mode-card path assigns
`VIDEO_NAME_MODE_SELECT`; `updateVideo(name)` closes the old
`mNetstream`, reads `FrontEndVideoList[name.toLowerCase()]`, starts
a new stream and reveals the mode-select layout. This supports
distinct submenu video behavior, but does not identify specific USM
assets or prove they are available. The existing preview still stages
the single default Goliath background; per-screen video selection
needs source-qualified catalog resolution and safe player switching.

**Long-term legacy menu swap while retaining Stage 2 functionality:**
- Inventory the actual legacy build's `main_menu.gfx`, dependencies,
  fonts, localization/UI_Data command XML, and background videos.
  A Stage 2 archive alone cannot establish legacy ActionScript
  handler names or parameters.
- Implement a versioned native menu compatibility adapter that
  maps verified *legacy* screen transitions and input state to current
  Stage 2 game services. Preserve sign-in, matchmaking, player state,
  mod/offline policy, and inaccessible native commands as explicit
  disabled functions until exact mappings are verified.
- Keep view/asset staging separate from command execution. Ruffle is
  an isolated UI preview, not a replacement for CryEngine Scaleform
  native callbacks. Do not just overwrite Stage 2 `main_menu.gfx`
  with the old movie or claim that such a swap will function.
- Add a selectable legacy menu skin/runtime option only after
  exact-build dependency and callback checks pass; require reversible
  installation/backups and multiplayer/offline safety tests.

Text is mixed: built-in timeline elements, fonts and initial edit text
are authored in GFX, while many labels, localization keys,
descriptions, build/account status, and menu row contents arrive through
ActionScript and CryEngine state. The screenshots' placeholder build
info and unresolved `@ui_SigningInPrompt` remain separate runtime
binding problems.

## 2026-10-22 — Stage 2 image-card blocker traced to numeric CurrentMenu

Recovered the user's original `main_menu.gfx` directly from the previously
supplied `UI(1).zip` (353,610 bytes; SHA-256
`7ac1133f42fcb3a6b74ddd228860beb154965d8f6a1d8f2ab4e02630bc442762`).
This game binary remains outside the repository.

The original AVM1 `shouldUseImageButtons()` (DefineFunction2 in the
first-frame DoAction) invokes
`flash.external.ExternalInterface.call("CurrentMenu")`, converts the
result with ActionToNumber, and compares with three original globals:
`MMS_SOLO`, `MMS_TUTORIALS`, `MMS_NPE`.
Original constant assignments found in that same bytecode are
`MMS_SOLO=0`, `MMS_EXTRAS=1`, `MMS_TUTORIALS=2`,
`MMS_NPE=3`, `MMS_ROOT=4`.
Nightly 31's preview-only adapter returned the literal source submenu
name, such as `"Training"`. `Number("Training")` is NaN, so all
three original image-button comparisons were false. The user's runtime
capture corroborated the issue: eight `CurrentMenu` host calls were
received and all 24/24 source MenuModeSelect images were linked, but
the original menu still rendered its text-only branch.

The preview-only host getter now returns the **numeric** MMS value,
never the visible submenu name. Original `Main` maps to 4;
UI_Data `Singleplayer`/`Training`/`Tutorials` map to 2;
`NPE` maps to 3; `Solo` to 0; and `Extras` to 1.
The inspector displays the numeric enum and lets the user select all
five original values explicitly. Existing original `pushMenuState`
navigation updates it **before** invoking the Flash callback.
The GFX, linked DDS, hover animations, and native game-service
restrictions remain unchanged; no replacement HTML cards are drawn.

Expected acceptance with the real GFX: after entering the original
Training/Tutorials or NPE submenu, `shouldUseImageButtons()` must take
its existing image-button branch and Ruffle must render the source
bitmap links. Passing a source-bytecode assertion or JS unit test is
not yet an end-to-end visual confirmation; if the image branch runs but
is still blank, inspect the original image-loader symbol instances and
Ruffle bitmap linkage rather than revisiting numeric game-state IDs.

## 2026-10-22 — private-style Asset Library full-collection importer

The public Iced Asset Library previously exposed a typed-path popover and only
scanned up to 64 PAK files directly inside the selected folder. It did not
provide the private Qt manager's multi-select file picker or full-game
recursive import with a preview/confirmation step.

The Asset Library now offers native **Add PAK files…** (multi-file) and
**Import ALL PAKs from folder…** pickers in an expanded in-page workflow.
Folder discovery is opt-in and traverses nested directories up to 16 levels,
with a hard 4,096-PAK ceiling and a bounded directory-entry count. Sources
inside the library, source symlinks, and linked directories are excluded.
The review panel lists the number of candidates, packed bytes and already
imported sources before the user confirms. No source bytes are modified by
the scan. Imports reuse the original public/custom RSA verification and
atomic one-archive commit path, with per-file failure reporting and
pre-existing verified archives reused.

A cancel request stops **between archives**, retaining completed imports;
the UI refreshes the counter periodically through the existing pulse.
This initial parity pass does **not** claim within-PAK cancellation or
streamed byte-level progress; those are separate from the verified archive
importer and require a cancellable extraction API. The current public
importer handles **signed Stage 2 PAKs**; the private manager's separate
legacy ZIP/2012 importer and arbitrary build registration are still
not ported by this UI change, and there is no implicit cross-build
signing or modification of original game files. Choosing a collection
is not a request to launch or install the game.

## 2026-10-22 — original Training image cards: preview runtime state

The source `Libs/UI/main_menu.gfx` already contains the original
mode-card display list and the `shouldUseImageButtons()` decision.
The Flash code references the native menu state `CurrentMenu`; the
Training screen's `NPE` and `CustomSolo` conditions can also
suppress authored choices when those game values are unknown.

The isolated Ruffle preview now exposes three **preview-only** JavaScript
ExternalInterface answers: `CurrentMenu` (numeric MMS_* enum),
`NPE` and `CustomSolo` (booleans). Their assumed values can be
changed in the source UI_Data inspector without changing original game
files or injecting into Evolve. The example Training flags initially
default to true **for preview coverage**, not as verified live game
state. The main menu still uses the verified original seven-argument
Flash `openScreen` and `pushMenuState` paths; no HTML buttons or
replacement card graphics are generated.

For the verified `MainMenuScreen.xml` manifest, only the exact
`CustomCondition=[NPE]`, `CustomCondition=[!NPE]`,
`CustomCondition=[CustomSolo]`, and
`CustomCondition=[!CustomSolo]` expressions are evaluated locally.
All unknown game conditions remain blocked. The source inspector and
Flash menu receive rows filtered using these values, while native
game commands remain disabled. The local state tracker reports direct
host query counts to distinguish a called adapter from a nonworking
Flash query.

The pinned Ruffle Web v0.7.1 runtime evaluates Flash-to-JavaScript
`ExternalInterface.call` names using a JavaScript Function constructor.
The isolated loopback preview therefore allows `'unsafe-eval'` in its CSP
in addition to the existing WASM exception; this is deliberately not
enabled in the main desktop application or any public remote host.
Because this broadens what a modified GFX can execute in the local
browser, do not treat this as a general-purpose sandbox for untrusted
Flash files. The preview still serves only its private-token scratch
content and allows no native game-service bridge.

**Updated finding:** The original `CurrentMenu` host-call name and
numeric MMS values were recovered and documented above; the previous
string-returning adapter was incorrect. Full Ruffle image-card and
hover behavior must still be checked in an installed build. No
synthetic SWF artwork or native game-service bridges are added.

## 2026-10-22 — cosmetic/gameplay separation and guided editing

Implemented in the public Rust/Iced repository, on top of `a50e99b` (current main at start of this change). No private repository code or game assets were copied. No new PR or contributor attribution is needed.

### What now works

- Modding projects have **All mods / Cosmetic candidates / Gameplay / Needs review** filters, a mod-type column, and per-changed-file classification. Classification uses the existing changed-member scan, never the mod's display name or PAK basename. Any detected gameplay member dominates a mixed set. Incomplete scans and unknown file types remain offline/review. Untouched projects display Unchanged.
- Explicit gameplay roots cover Scripts, Entities, Libs/Actor, Abilities, Items, GameRules, Config, AI, EntityArchetypes, Progression, Player, GameTokens, ContextualCombat and ZooKeeper, plus parameter roots. `.actorparams`, `.chrparams`, Lua/config/executable files cannot become cosmetic by a cosmetic-looking filename elsewhere. Models, textures (including numbered DDS fragments), sounds, materials and UI assets are **candidates**, not a proof of online eligibility.
- **Gameplay values…** indexes known virtual paths across editable project manifests. Search by character, ability, filename or build, select a document, then search named fields (health, cooldown, damage, etc.). Numeric and boolean attributes become editable form rows; `<param name="..." value="...">` rows show the parameter name and parent context. Repeated names retain distinct element identities. Original string references and other XML content are retained.
- Stage-separated values (`100|200|300`) and vectors (`1,2,3|4,5,6`) are supported with finite-number and original component-count validation. These are syntax checks, **not** guessed engine ranges or a fabricated balance schema. Inherited values are not expanded into child documents. Text-node values and arbitrary string/reference editing are not part of this form yet.
- Save edits the managed project copy through the existing backup/import transaction, validates XML, detects stale open documents both before serialization and at import, and preserves a recovery revision in EditorBackups. Existing build logic re-encodes CryXmlB members from their authenticated original. Unsaved form edits must be saved or explicitly discarded before switching documents. The Loaded value is the value at opening, not necessarily the original game default.
- **Models…** indexes CDF/CGF/CHR/SKIN assets by name and build. Choosing an editable asset opens its existing model/file editor with the exact virtual path search and expanded explorer, exposing the existing preview/import/replacement workflow. This is a navigation shortcut, not an in-app mesh authoring tool.
- Both guided lists can index the loaded Asset Library's manifests (respecting its build filter). Imported rows offer **Create editable copy** without asking the user to browse a PAK. Creation uses the existing verified imported-project operation and requires the imported original to map to the configured signing stage. It returns to the guided list after project creation. **Refresh assets** loads a missing library catalog or refreshes projects; source files remain read-only. Configure storage/stage if no verified mapping exists; never guess across builds.

### Findings from the user's local game files

Build collections identify `libs.pak__a78de803dee8f1530a35` as **Modded Stage 2** and `Scripts.pak__897f46b610c8e2022159` as **2012**. The stable paths differ by generation:

- Stage 2: `libs/Actor/*.actorparams` (192 manifest members), `libs/Abilities/` (54), `libs/Items/` (788), and `libs/GameRules/` (13). For example, BaseMonster.actorparams contains named movement/collider values, inherited `base` identity and pipe-separated stage values. Abilities such as `libs/Abilities/Goliath/GoliathCharge.xml` are XML.
- 2012: `Scripts/Entities/Abilities/XML/<character>/...xml`, e.g. WhipSpiderWebTrap3 contains `energy_cost` and `cooldown_time`; `Scripts/GameRules/CharacterClasses.xml` is another gameplay document.
- Therefore an archive-independent, build-scoped virtual-path index is feasible and preferable to hardcoding one PAK name for all versions. The initial editor consumes verified extracted project copies and imported manifests; it does not discover installed loose files by a global filesystem search.

### Online eligibility: unresolved and intentionally not advertised as working

**This change does not enable online mod launching.** Existing mod/profile launch paths remain offline-only. A cosmetic candidate in this dashboard is only a routing hint. The current project scan is bounded (256 MiB hashing budget) and describes manifest-listed edits; it is not an exhaustive audit of installed PAKs, additions/deletions, loose-file overrides, profile swaps, or hooks. Its metadata is not an online trust boundary.

The user's policy is feasible as a manager-enforced allowlist, but extension checks are insufficient. CGF/CHR files can contain collision/physics data; CDF attachment configuration and materials may affect behavior; GFX/SWF contains executable ActionScript and host commands. All of these need original-versus-modified semantic comparison before a change can be certified as **only** cosmetic. Unknown or mixed content must stay offline even if its category or filename says cosmetic. The manager must not reuse the private-helper or native offline-hook route to claim an online launch.

Next implementation steps, in order:

1. Add a full changed-content audit against an authenticated original for the **selected build**, including every enabled installed mod, archive-member additions/deletions, loose files, and any active stage/hook transaction. Stream hashes without the dashboard's sample budget. Bind the report to original and replacement content hashes and classifier version; revalidate at launch to invalidate stale approval.
2. Add format-aware proofs: payload-only texture/audio changes first; then mesh visual chunks with physics/skeleton/collision invariants; then constrained CDF/material/UI semantic changes. Unknown formats and gameplay/ability/actor changes fail closed. Add adversarial mixed, mislabeled, edited-after-audit, symlink, and baseline-mismatch tests.
3. Expose separate **cosmetic online** and **gameplay offline** installed-profile views using those reports. Gate the actual normal-client launch on the union of all enabled changes, with restoration/verification of offline hooks and incompatible files. Never let an editable user checkbox override detected gameplay.
4. Verify the supported game's normal online behavior with the verified cosmetic-only payload. Manager classification alone cannot prove server compatibility. There is no online acceptance result in this handoff.
5. Improve editor usability further: direct selection of the just-created member, build selection inside the guided panel, inherited/default-value resolution, schema-aware units/ranges/enums, explicit recovery buttons within the form, model replacement browsing, and background/paged indexing for exceptionally large libraries. Imported rows currently create the containing verified project before editing; unsupported stage mappings still require setup. The existing signed builder's 1–64 changed-member limit remains.

### Validation

- Core library: **181 passed, 0 failed**, with one optional real-file test ignored by default. The first sandbox run had Windows fixture replacement failures; rerunning outside the sandbox passed the full suite.
- Real-file read-only acceptance: unchanged round trips are byte-identical for Stage 2 BaseMonster.actorparams (**93 fields**) and GoliathCharge.xml (**29 fields**). No original was modified. Reproduce with `EVOLVE_GAMEPLAY_DOCUMENT=<local file>` and `cargo test -p evolve-core real_gameplay_document_round_trip --lib -- --ignored --nocapture`.
- Desktop suite: **210 passed, 0 failed**, one existing optional GFX test ignored. Includes new guided-save/backup/restore/stale-source rejection test; existing CryXmlB import/build serialization tests remain passing. Windows tests used a copied test executable with an asInvoker test-only manifest; production administrator manifest unchanged.
- Workspace/all-target Clippy passes with `-D warnings`; desktop builds. No live GUI screenshot or installed-game launch was tested in this pass, and no claim of full online or model-editing parity is made.


## Nightly 20 screenshot — original sign-in overlay still visible

The source screenshot shows a near-black Ruffle preview, visible original
`@ui_SigningInPrompt` text, two outlined prompt buttons, and a faint
original red logo. **This does not prove that the source UI failed to load**:
the separate video background may be unavailable, the sign-in layer may
remain active without native network state, and Ruffle may omit original
Scaleform color/shader/font behavior. Screenshot alone cannot distinguish them.

The live preview now exposes a **Dismiss sign-in prompt** action only when
the original first-frame AVM1 `showSigningInPrompt(bool)` function is
signature-verified. Clicking calls that source function with `false` in
the isolated movie. It never fakes successful login, network state, or game
native callbacks. The button stays disabled if unsupported.

When the source background cannot be prepared, a compact on-stage notice
reports the exact `videos.pak` staging status and points to the existing
source-backed sidecar / explicitly configured FFmpeg path. If
`ui_signinginprompt` remains unresolved in the exact-build English
localization report, the notice also calls out the missing translation.
These are **diagnostics and a verified UI-local action**, not a full
rendering fix or complete native main-menu initialization.

Additional follow-up based on the resource diagnostic screenshot: the Asset
Library is configured but the GFX source build is unavailable; 61 bitmap tiles
were restored, **350 were skipped for unknown individual reasons**, one
nested movie is missing, and the main-menu video is not source-qualified.
The previous summary of all 350 as missing source files would be incorrect.

The editor now attempts to identify a build when one and only one library
build contains an exact virtual movie path with byte-for-byte identical
original GFX data. Cross-build identical copies, ambiguous members, and
missing manifests intentionally remain unselected. Nothing is identified
by basename or game version guess. The preview now reports separate bitmap
skip counts for missing DDS source, preview pixel/tile limits, invalid
atlas rectangles/definitions, duplicate/already defined bitmap IDs, and
DDS decoder failures. It includes up to twelve original virtual DDS names
for the missing-source category. Existing safety limits remain enforced.

Still required: inspect the user's `preview-menu.json` resource counts
and `background_video.status`, verify the source
`showSigningInPrompt(false)` visual transition in Ruffle, and compare the
same frame against live CryEngine output. Preserve the original sign-in
text, never invent a translation, and keep the GFX bytes read-only.

## October 22 — exact-build videos.pak inventory and Goliath source lookup

The Stage 2 main-menu background defaults to the original
`front_end/mainmenu_goliath.usm`. If a variant `videos.pak` stores
that exact filename beneath a deeper mount root, the Asset Library
now checks the source build's verified imported archive member inventory,
requires a unique exact Goliath filename, and never substitutes
another monster or unrelated video. When absent, diagnostics list
up to eight verified source member names instead of guessing a mount.

CRI USM still requires a compatible configured FFmpeg executable or
an original-derived MP4/WebM sidecar. This is source discovery and
diagnostics, not a promise of browser playback or native video parity.

## October 22 — preview tile limits and missing video diagnosis

The user's Nightly 22 diagnostic shows **119 DDS tiles restored, 292 skipped
strictly due to preview limits, 0 missing source tiles, 0 DDS decode errors,
and 0 missing imported SWF movies**. Do not describe those 292 as missing
asset files. Most affected original Scaleform bitmap tiles are present but
excluded by the preview's old 128-tile / 8-million-pixel safety ceilings.

The temporary GFX-to-SWF raster path now allows at most **512 restored
bitmaps**, **4 million pixels per cropped tile** and **32 million cropped
pixels per source movie**. A conservative encoded-output check is performed
before adding each bitmap so the original **32 MiB decompressed SWF size
limit** remains effective; rejected tiles are reported without breaking the
entire preview. Runtime diagnostics separate bitmap-count, pixel-budget and
SWF-byte-limit skips. The original game assets and GFX stay unchanged.
Large or incompressible original textures may still be skipped within the
bounds. Real Ruffle memory and fidelity require Windows runtime testing.

Separately, the original moving background was not imported from exact-build
`videos.pak`: `Libs/UI/FlashAssets/videos/main_menu.usm` is required.
That source USM must be imported and explicitly transcoded with configured
FFmpeg or paired with a user-provided `main_menu.preview.mp4` derivative.
Changing bitmap caps cannot produce the original background video.
Font definition verification and dynamic ActionScript `@ui_` localization
are independent pending tasks; 0/6 in-movie static fields does **not**
verify that the native localization engine was successfully initialized.

## October 22 — source-verified videos.pak background discovery

The main-menu browser background remains an optional **original
`main_menu.usm` from `videos.pak`**. The original resource may be stored
relative to either `Libs/UI/FlashAssets/videos`, `FlashAssets/videos`,
`videos` or at the archive root (with optional extracted `files/` prefix).
These are checked **only inside the matching build's actual videos.pak
manifest**; no unrelated movie or nearest-basename substitution is allowed.
Multiple distinct candidates are treated as ambiguous, not silently picked.

The preview now distinguishes a missing `videos.pak` import from an
imported video PAK with no matching source movie, and can stage the verified
CRI USM only after its `CRID` header and bounded size are validated.
A compatible explicit FFmpeg binary or source-adjacent user-provided preview
MP4/WebM remains required for Ruffle playback; Windows conversion hides the
FFmpeg CLI console. The original PAK is unchanged. No video is synthesized
if the source clip is not present in the Asset Library.

## October 22 — verified Stage 2 archive present but 0 English sheets

The user confirmed `USA-EN_xml.pak` is visible under **Asset
Library → By Build → Modded Stage 2**, not just All Assets. Thus an
unassigned archive is *not* the explanation for the existing live-preview
report `0 verified English UI keys from 0 exact-build localization sheets`.

The localization loader previously collected virtual XML member paths and
then resolved each path against **all** PAKs in the selected build. A path
such as `files/text_ui_menus.xml` in two archives becomes globally
ambiguous and was silently skipped, producing a misleading zero-sheet
status. It now retains each candidate's archive name and uses the
archive-scoped **same-build** resolver for each XML. Conflicting values
across verified sheets still fail closed for that key; identical values
can be reused. Ambiguous duplicate copies of the *same archive name*
and missing/modified extracted members now return precise errors instead
of disappearing silently. A Stage 2 `USA-EN_xml.pak` with no compatible
XML manifest entries now has an explicit diagnostic.

Regression coverage includes two original-build language PAK manifests
sharing the exact `files/text_ui_menus.xml` virtual path, which the prior
global resolver considered ambiguous. This is a concrete code defect, but
without the user's actual `asset-library.json` and preview JSON it remains
to be established whether that collision is the sole runtime cause.

Existing preview sessions are snapshots. After updating the manager and
importing/refreshing a PAK, reopen the GFX editor and start a new live
preview so the cached build and localization tables are regenerated.
Runtime ActionScript-assigned `@ui_` strings and font glyph binding remain
separate unfinished work.

## October 22 — inspected USA-EN_xml.pak: original English menu text

The provided `USA-EN_xml.pak` is a **standard unencrypted ZIP PAK**, with
61 entries including `files/text_ui_menus.xml` (815,654 bytes),
`files/text_credits.xml`, and the original root-level
`gfxfontlib_locfont.gfx` and `gfxfontlib_glyphs_locfont.gfx`.
The UI sheet contains 8,282 original named strings.

Unlike spreadsheet-localization releases, this Stage 2 archive stores
`<Strings><Category><String name="ui_..." value="..."/></Category></Strings>`.
The existing reader previously accepted only SpreadsheetML
KEY/TRANSLATED TEXT rows and the name `English_XML.pak`; the update accepts
the original `USA-EN_xml.pak` format and both its root and `files/`-prefixed
manifest member paths, without relaxing exact-build source selection.

Verified examples from the original archive:
`ui_SigningInPrompt = SIGNING IN. PLEASE WAIT...`,
`ui_mp_OnlineGame = PLAY`, `ui_Training = TRAINING`,
`ui_Options = OPTIONS`, `ui_ChallengesMainMenuButton = CHALLENGES`,
`ui_goliath_tutorial = GOLIATH TUTORIAL`.

Only the two authored original locfont GFX imports may fall back to the
selected build's USA-EN archive root. Other missing/ambiguous dependencies
remain blocked, as do cross-build references. Nothing from the source PAK
is copied into the repository, changed, or hardcoded as a translation.

**User procedure:** import the unchanged `USA-EN_xml.pak` into the
**Modded Stage 2** Asset Library build and reopen the GFX preview.
Expect source-derived inspector translations and any matching authored
SWF DefineEditText initial values to resolve from XML. Text dynamically
assigned later by ActionScript still requires a separately verified
Ruffle/Scaleform runtime localization mechanism. Staging the original
font library does not by itself prove correct font binding or glyphs.

## October 22 — linked original GFX screen previews and silent CLI tools

- A same-build screen catalog now tries exact, unambiguous `Libs/UI/*.gfx`
  references for original `options.gfx`, `ChallengesPopup.gfx`,
  `custom_match_options.gfx` and `credits.gfx`. An entry appears only
  when its exact-source Asset Library reference exists and a bounded Ruffle
  movie can be staged. Source `UI/UIElements/<Name>.xml` evidence is
  reported when available, not invented.
- Original linked movies are previewed manually from **Other original GFX
  screens** in the live inspector, with bounded source-relative imported
  assets and DDS atlas reconstruction. A separate Ruffle instance replaces
  the visible view; Back restores the **same** original menu player rather
  than restarting or fabricating its game state. Native FSCommands remain
  blocked. **This is a source screen browser, not original native command
  routing**: `GoToSettings`, `OpenChallenges`, `Credits` etc. must
  remain unbound until source XML and original native contracts prove which
  UI element and handler each command activates.
- Export folder/file selection still uses the native WinForms picker, but
  launches PowerShell with Windows `CREATE_NO_WINDOW`. Background Blender
  processes (both general FBX and character assembly pipelines) use the
  same flag and continue writing bounded diagnostic logs; FFDec CLI
  conversions/frame previews suppress the console while explicit **Open
  FFDec GUI** continues to show the full application.
- The manager uses Iced 0.14 with a wgpu canvas, not an embedded browser
  surface. Full *interactive* Ruffle in the native GFX viewport requires a
  separately integrated WebView2/wry or supported texture-sharing input
  bridge, including mouse/keyboard focus and DPI lifecycle. The standalone
  loopback browser remains the working live preview. The manager's existing
  static FFDec frame preview still works inside Iced; do not claim
  interactive in-viewport parity on a static frame.

Verification: source-qualified links, submenu and Back in Windows Ruffle,
DDS/import and UIElement metadata, Windows CLI window suppression, both
CI jobs, and Windows packaged binaries remain necessary before release.

## October 22 — source-backed GFX parity continuation after submenu wiring

Branch: `feature/gfx-main-menu-text-init`, draft PR #29. This pass builds on
`f19500f` (core and desktop CI succeeded; portable-preview job skipped).
All source game GFX, USM, fonts and localization archives remain read-only;
only the isolated Ruffle copies and optional inspector metadata are changed.

### Implemented since `f19500f`

- **Original submenu synchronization** (`f948c35`): removed duplicate
  inspector click handlers. Source-navigation UI advances only after the
  original `evolvePreview_pushSourceMenu` callback returns `true`;
  `goBack` restores the previous local inspector group only after the
  original callback returns `true`. PC/unscoped source visibility and
  disabled gating are enforced consistently. The stack has a 32-deep cap,
  and original menu reinitialization resets it.
- **Imported-movie localization** (`1a9a458`): walks up to 128 already
  staged, same-source GFX/SWF imports, collects at most 1024 source-authored
  `DefineEditText` initial `@key` tokens and performs one exact-build
  localization lookup. Rewrites only matching initial strings in
  temporary imported copies. Reports imported field, localized, unresolved
  and unsupported counts. Does **not** translate arbitrary later
  `TextField.text` assignments or override the original
  `gfx.utils.Locale.getTranslatedString` bytecode.
- **Font/source binding evidence** (`acf706b`, `9b6f59f`):
  reads bounded embedded DefineFont/DefineFont2/DefineFont3/DefineFont4
  names, IDs, bold/italic flags and glyph counts, plus exact
  `DefineFontName` full-name aliases. IDs are scoped to their source
  movie. The live inspector exposes this evidence; a successful font
  import does not prove Ruffle chose that glyph library for every field.
- **Native command observation** (`948fa31`):
  retains original SWF DefineButton2 ID/condition tracing, adds a
  bounded read-only trace of FSCommand names/arguments, and keeps all
  game-service execution disabled. A button SWF ID is *not* the same as
  the XML command or an authoritative game callback mapping.
- **Scaleform effects inspection** (`6ac89af`, `8ebd0b9`):
  recursively inventories original SWF PlaceObject2/PlaceObject3
  color-transform, filter-list and blend-mode flags, including nested
  DefineSprite timelines. This does **not** implement missing GPU filter,
  noise, glow, native color-multiplier or CryEngine postprocessing paths.
- **Stage/background compositing** (`e30fdb4`, `29190c4`):
  positions the source-derived browser-playable USM video and the
  transparent Ruffle stage in one shared fitted rectangle, using
  Ruffle's actual SWF metadata width/height and container resize.
  This avoids independently cropping or letterboxing the two layers
  in unusual window aspect ratios; pixel/animation parity remains unverified.

### Strict remaining acceptance gates

1. Check final-head GitHub Actions Linux core, Windows desktop
   compile/Clippy/tests and applicable portable preview; do not borrow
   success from earlier commits. Test new synthetic original font and
   source effect tag cases. Validate emitted submenu AVM1 callback with FFDec.
2. Execute the actual main-menu movie and its nested imports in Ruffle;
   capture original and Ruffle frames at identical resolution, language,
   time and state. Verify original button press/rollover navigation, submenu
   stack depth, back/reset, patch notes, background video crop and
   source-authored effects.
3. Dynamic ActionScript translation and actual font alias/glyph binding
   need separate first-frame/source-script proof, as do original native
   button-to-UI-local-handler mappings. Do not map SWF character IDs to
   XML row commands by order and do not silently enable online,
   launch, store, exit, profiles or credentials.
4. Original Scaleform glow/color-transform rendering fidelity and
   CryEngine shader passes remain unimplemented where Ruffle diverges.
   The static effect inventory is diagnostic evidence, not equivalence.
   Do not merge, release or claim pixel-identical visuals on this evidence.

## October 22 — continued original Flash submenu navigation (PR #29)

From `afd8d0d`, the original `openScreen(7)` and `pushMenuState(0)`
first-frame definitions provide an explicit UI-local next step. The isolated
SWF preview now registers an `evolvePreview_pushSourceMenu` ExternalInterface
callback only when the verified main-menu lifecycle is present; the callback
assigns source-supplied rows to the original `menuButtonData` and calls the
original `pushMenuState`. The source inspector can request these transitions
only for existing submenu groups with visible PC/unscoped rows from matching
UI_Data. Its Back control invokes the existing verified `goBack` callback
when a locally initiated submenu is active, and reset clears the local stack.
Native game services and arbitrary button command execution remain blocked.
This does not fabricate Flash display objects or rewrite the original assets.

Commits: `2c77edb`, `eaa286f`, `7f38b33`. This change adds a synthetic
bytecode-registration assertion but **does not prove** Ruffle correctly renders
submenu transition frames, original button press-to-command mapping, or that
`pushMenuState` accepts every parsed UI_Data submenu unchanged. The inspector
button remains a diagnostic control, not a claim that clicking the in-movie
button drives the native host. Ruffle real-asset visual acceptance and final
Windows CI at latest head are still required. Runtime localization outside the
source initial strings, matching DefineFont3 font binding/aliases, and native
Scaleform effects remain open.

## October 22 — source-backed original main-menu population

Worked from `feature/gfx-main-menu-text-init` at `071e2a5`, which contains the
current live Ruffle callback and initial-text localization work. This change does
not create a replacement HTML menu or claim pixel-identical game rendering.

### Evidence from the installed game (read only)

FFDec exported scripts from the imported `Libs/UI/main_menu.gfx` in UI_Assets.
The original root `openScreen` has **seven arguments**, independently confirmed
by `Libs/UI/UIElements/MainMenu.xml` in the matching UI_Data import:
`is_in_world`, engagement prompt, debug info, comm, cinematic playing, background
video, news. The previous zero-argument allowlist rejected this real function and
fell back to `transitionToMainMenu`, which fades existing controls but does not
populate them.

The root's `pushMenuState` copies `menuButtonData` into a menu stack and invokes
`setupMenuBtns`. That function attaches the original `mcButtonMainMenu` symbols,
uses the source label/command/enabled fields, and registers original rollover and
press handlers. Its featured-button sizing compares names against
`MODE_NAME_PLAY` and `MODE_NAME_TRAINING`; translating labels without translating
these two comparison values changes the original layout. Button activation reaches
`makeMenuSelection` and then game ExternalInterface commands/cinematics. UI_Data
command names do **not** establish that a native service is available in Ruffle.

The source menu catalog is **Libs/UI/MainMenuScreen.xml**, not
`UI/MainMenuScreen.xml`; the prior exact resolver asked for the latter and could
miss the installed import entirely. The source contains separate PC/Durango/Orbis
entries, conditional visibility and sign-in restrictions, not one universal list.

### Implemented

- Correct exact same-build menu XML lookup; retain platform along with original
  command, visibility and disabled expressions in the preview manifest.
- Recognize the seven-argument menu only when original first-frame bytecode also
  declares `clearMenuState(1)`, `pushMenuState(0)`, and
  `transitionToMainMenu(0)`. Other arities remain rejected; generic GFX is unchanged.
- The preview callback now clears prior menu state, supplies source-derived menu
  data, calls the original seven-argument `openScreen`, then `pushMenuState` and
  `transitionToMainMenu`. The browser passes localized labels and updates the two
  original featured-label comparisons consistently. Manual initialization, automatic
  startup, reset-button initialization, and the queued Open Screen command use the
  same data path.
- Preview rows retain XML order and exact command identifiers, select PC/unscoped
  entries, omit conditionally hidden entries and disable conditionally gated
  actions. This is an explicitly limited offline preview policy, **not** evaluated
  real game state. Lifecycle values are `(false,false,false,false,true,"",false)`;
  these describe a preview without engagement/debug/comm/news or an in-movie video.
  Do not call these values the authoritative host state for every game session.
- No game services, launch/store/exit commands, profiles or credentials are exposed.
  Original source GFX and imported game resources are untouched.

### Font and localization findings

All four inspected font-library movies contain six standard **DefineFont3** tags,
not only Scaleform-specific glyph data. `gfxfontlib_locfont.gfx` declares Rocketship
Town (190 glyphs), Indento Bold (63), Bender (173), Cabin (two 190-glyph definitions)
and SV Basic Manual (151). Presence/staging alone does not prove runtime binding.
The movie's `gfx.utils.Locale.getTranslatedString` returns its input unchanged;
there is no embedded translation implementation there. Initial-text rewriting
cannot localize arbitrary later `.text = "@key"` assignments. This change localizes
host-supplied main-menu rows only; dynamic prompts and all other UI still require
an appropriate source-backed translator/host model.

### Validation and resume points

- Desktop suite: **191 passed, 1 opt-in real-asset test ignored**. The opt-in test
  then passed separately against the installed original menu and verified source
  bytes unchanged. All-targets workspace Clippy passed; JS syntax check passed.
- Independent FFDec decompilation of the generated SWF confirmed the callback
  parameters, original-variable assignments, seven values and lifecycle call order.
  This validates emitted bytecode, **not** browser-visible frame equivalence.
- Local-only evidence: `work/gfx-main-menu/scripts`, `patched-scripts`, and
  `initialized-preview.swf` under the parent workspace. Do not commit these game
  scripts/assets. Reproduce with the ignored `real_menu_bootstrap_acceptance` test
  using `EVOLVE_GFX_SOURCE` and a new `EVOLVE_GFX_PREVIEW_OUT` outside original assets.
- Next run the original menu in Ruffle and compare captured frames at fixed times
  with the same game build, language, resolution and host state. Verify native
  button attach/press behavior, import aliases/font glyph selection, labels and
  original featured sizing. **Pixel identity has not been achieved or verified.**
- Implement an explicit preview-state model for XML conditions, then local submenu
  navigation using original movie callbacks and source-defined row lists. Native
  commands remain observed/unavailable until deliberately simulated. The current
  implementation initializes Main; it is not complete submenu/service emulation.
- Extract a reusable host-contract model from same-build UIElements definitions:
  function names/arity, variables/arrays, events, constraints and movie references.
  Bind only reviewed UI-local operations to validated original bytecode; do not
  extrapolate this main-menu-specific lifecycle to arbitrary assets automatically.
- Continue dynamic localization, font alias mapping and Scaleform rendering effects;
  missing native color-multiplier/noise effects and the game background composition
  can prevent identical appearance even with correct text and geometry.

## October 22 — retained engine glow research (evidence, not calibration)

Original 2012 Shaders.pak source provides concrete next steps beyond a guessed
`GlowAmount=3 => 100%` scale. In `Illum.cfx` the ALPHAGLOW/ALPHAGLOW_CUSTOM branch
uses the authored `AmbientMultiplier` (default 1, editor range 0–32). In
`fragLib.cfi` lines 754–763 the ambient-pass alpha glow adds diffuse RGB multiplied
by diffuse alpha (or custom-map red) and that multiplier. Separately,
`CommonGlowPass.cfi` lines 109–139 combines the diffuse/decal sample, `GlowParams`,
material diffuse color and masks. **The CPU binding of material GlowAmount into
PB_GlowParams remains unverified**; these are distinct paths, not interchangeable
intensity values. Do not port the 2012 constants blindly to Stage 2.

`HDRPostProcess.cfx` lines 541–568 then applies adapted exposure, bloom, color
balance and a filmic curve. A screenshot's final brightness is not a standalone
material scalar. The sampled Stage 2 Wraith effects actually author GlowAmount
5 (abduction), 10 (decoy), 4 (nebula shroud), and 2 (phasing glass); the base body
has a Decal glow texture but no GlowAmount attribute in the inspected material.
Absence must remain distinct from zero/default/runtime override.

Official reference: [CryEngine 3 Glow Effect](https://www.cryengine.com/docs/static/engines/cryengine-3/categories/1114113/pages/1048623)
describes strength, diffuse RGB and alpha masking; it does not define 3 as 100%.
[Decal Opacity Map](https://www.cryengine.com/docs/static/engines/cryengine-3/categories/1114113/pages/23299074)
describes the shader-dependent Decal glow path. Next inspect the matching build's
CPU constant binding/runtime material overrides and capture linear scene/glow
buffers plus exposure/bloom settings; keep source material gain separate from an
explicit viewer exposure control. No further glow formula was changed here.

## October 20 — follow-up: separate original emission from armor-overlay toggle

The normal/base Stage 2 material is allowed to glow independently of the
optional `[name]_glow_diff` overlay. The previous preview implementation
incorrectly set `emission_strength=0` on *every* material when Full armor glow
was unchecked; that hid original `Emittance` and CryEngine `GlowAmount` too.
Now the switch solely controls loading/applying that monster-specific overlay.
Explicit material emission remains on, with original source-relative gain.
The renderer now binds source Emittance and the optional armor overlay as
**independent GPU textures**, so turning the overlay on does not suppress or
overwrite normally emitting areas. The shader computes source emission using
authored GlowAmount, adds optional armor glow independently at unity source
gain, then applies the user's shared viewport slider. Unchecking removes only
the armor channel (neutral black map), without blanking original emission.

**Model Options emissive-strength slider:** shown only when a loaded model has
a visible emitting material or explicit standalone emissive map; 0–4×,
default 1×, reset action. A live per-scene viewport-only uniform scales both
original emission and (when enabled) the optional armor overlay. Material
source gain is retained as its separate uniform component; a slider change
writes tiny GPU buffers without recooking DDS, rebuilding geometry, modifying
material assignments, or changing FBX exports. Switching source assets resets
preview gain to 1×. No slider is shown for non-emissive or unloaded models.

**Acceptance:** Confirm ordinary Goliath material emission still visible with
Full armor glow off; only the optional armor regions toggle; slider updates
instantaneously, respects 0× and reset, and doesn't brighten diffuse/normal
or alter original DDS. Verify both slider and option after CDF reloads,
damage-blend preview and on Windows GPU; check CI before merge/release.

## October 20 — Stage 2 full-armor glow and source-authored intensity

User-provided Goliath3 screenshot: **Full armor glow checked**, armor remains
dark. Investigation on public branch `6d80591` verified two separate defects:
the material parser discarded `GlowAmount` and other named shader parameters;
and armor_emission clipped each encoded channel difference to **80/255** before
the GPU's sRGB decode (approximately **0.08** linear intensity at the cap).
Original 2012 shader source was also reported to distinguish alpha-glow and
glow-pass; **there is no evidence that GlowAmount=3 means "100%"**.

Implemented in the ongoing PR: keep GlowAmount verbatim as an optional per-leaf
source scalar (root attribute, PublicParams, named ShaderParams); use it as a
relative WGPU fragment multiplier on each exact source subset. A positive
legacy Illum/Cloth/HumanSkin amount may use a diffuse-alpha emission mask only
where actual source alpha differs from opaque and no explicit emitting map
exists. An authored missing Emittance map is not replaced with a guessed alpha
mask. The separate `*_glow_diff` overlay now derives only positive *linear-light*
differences rather than capped sRGB differences; it does not replace an
already verified emitting map or override explicitly authored zero emission.
The monster checkbox disables preview glow on uncheck and rebuilds the
preview on recheck; FBX Rigged/Embedded options are not required for viewport
display. Source meshes, UV0, game assets and DDS bytes remain untouched.

**Unverified:** real Goliath3/Behemoth/Kraken/Wraith source-map completeness
and glow equivalence to game runtime; actual CryEngine alpha-glow and separate
glow-pass algorithms, tone mapping and bloom; Windows WGPU driver rendering.
The current target is LDR and may clip bright fragments. GPU intensity is
guarded at 100 to avoid absurd shader values without changing archived
GlowAmount. Verify both intensity and original tint per material in real
Stage 2 views, on/off interaction, authored zero/missing DDS diagnostics,
all material subsets/equipment, exact higher mip PAK sourcing, and Windows CI.
Do not claim in-game parity or merge/release until visual comparisons pass.

## October 20 — local supplied FBX investigation and shared decoder fix

This checkpoint is based on public branch `c207d2e`, preserving the newer remote
implementation. An older local investigation commit (`ed683b9`) was **not** pushed:
its broad changes overlap newer work and must not be applied over this branch.
All investigation and changes here are in the public repository. The original
supplied `scorpid3.fbx` and game assets were read only; no game assets enter Git.

**Confirmed remaining export bug:** PNG export and the desktop preview selected
Xbox 360 layout from original DDS header evidence, but `decode_dds_rgba` and
`decode_dds_rgba_with_parts` always selected `DdsLayout::Linear`. Unified FBX's
`decode_original_mip` calls those shared APIs, including its cross-PAK retry.
Thus even successful, fully linked embedded images contained corrupt pixels.
The shared APIs now use the same strict existing `FYRC`/flags/caps checks as PNG
export. Explicit-layout callers remain available. Numbered-fragment layout checks
read the .0 header. This preserves the current same-build high/low resolver and
per-material FBX verification instead of replacing them with older code.

**Evidence:** Blender 5.2.2 extraction of the supplied Scorpid3 FBX found five mesh
parts, fifteen packed images, and zero Actions. Its extracted pixels are visibly
striped/corrupted independently of lighting. A real export at `c207d2e` reproduced
that corruption despite having all five diffuse-textured parts, 124 bones and
17,228 weighted vertices. The initial WhipSpider sample's imported high-mip bytes
matched the original PAK; Xbox tiling + 16-bit word byte swapping yielded a coherent
atlas, with pixel agreement against a separate Python/Pillow reconstruction.
This establishes a decoder-path problem, not evidence of damaged source archives.

**Fixed final-file acceptance:** Regenerated `scorpid3-current-fixed.fbx` with
Blender 5.2.2, then imported that final published FBX in a separate Blender process
and extracted its packed images. All five parts have linked diffuse textures;
124 bones and 17,228 weighted vertices were retained. The body atlas (slot 10),
head diffuse and head normal were visually inspected and are coherent, with the
old striping gone. Both basher meshes remain included. The report still correctly
says `fully_textured: false`: complete shader/texture-role parity is not proven.
No CAF was supplied, so zero Actions is expected. Original supplied FBX untouched.
Before/after images and JSON reports are local `current-before-*`/`current-after-*`
artifacts; the fixed FBX is outside Git under `work/fbx-acceptance`.

**Tests:** 279 tests passed on the reconciled code (core 146, CLI 1, integration 8,
desktop 122, manifest 2); workspace all-targets Clippy with `-D warnings` passed.
The new regression checks that both shared decode APIs recover a console-endian
red tile, explicitly forced linear output differs, and a PC header remains linear.
Tests also retain the existing tiling-address regression. Desktop tests ran from
an asInvoker disposable EXE copy with `Start-Process -WindowStyle Hidden -Wait`
and captured exit code 0; production UAC policy was unchanged. Rename-dependent
filesystem tests ran outside the Windows sandbox using their temporary fixtures.

**Reproducible export probe:** `cargo run -p evolve-core --example
validate_unified_export -- <extracted-root> <cdf> <Blender.exe> <NEW absolute FBX>
[original CAF]`. Output must be outside the source assets and public repository.
Local acceptance artifacts are under the parent workspace's `work/fbx-acceptance`.
Keep generated FBX, extracted images, original PAKs and machine-specific logs out
of Git. The existing untracked Cargo.lock was not added.

**Model viewer animation finding:** The real 2012 Scorpid3 chrparams declares
`animations/Monsters/Scorpid2_3/Scorpid3.dba`; the matching 2012 Animations.pak import
contains Scorpid1/2/3 DBA databases and LMGs, with no loose Scorpid CAF files.
Automatic CAF idle lookup therefore cannot provide this character's animation.
Current `native_dba.rs` indexes metadata but does not decode playable key streams.
The desktop's converter-backed `DbaConverted` path clears the native CDF binding;
`MotionGlb` loads a separate GLB node-animation preview and clears skin preview.
It does not bind converted DBA tracks back to the original assembled character.
The supplied FBX's zero Actions are consistent with a static export, but do not
alone diagnose viewer playback. Native Stage 2 CAF remains a different path.

**Resume here:** decode/verify packed DBA tracks (or converter-track binding) and
map controllers to the actual 2012 skeleton before driving assembly deformation.
Check rest hierarchy, local bone palettes, duration, idle selection, rigid sockets
and additive convention against real clips. Preserve the current equipment chooser.
A custom all-six-L/R-arm gallery with verified textures, independent presentation
spacing and rigged FBX output still needs explicit acceptance; selecting one arm
per side is not that gallery. Full shader/normal/UV visual equivalence, tiny/padded
console textures, and interactive animation frame pacing remain unverified.

## October 20 — File → Export named FBX bundles, PNG dependencies and collaborator UV0 specifications

**Requested behavior (implemented in PR #6, pending final CI/real-asset acceptance):**

- `File → Export…` in the detached model viewer opens a floating, scrollable export dialog; the existing Ctrl/Shift selection is carried in, plus an indexed file filter and inclusion toggles. Users enter an **existing absolute parent directory** and a **new, custom export folder name**. A transaction stages all source conversions and only publishes the named folder when everything succeeds. Existing outputs are never overwritten and the source PAK/Asset Library is read-only. The Model menu and context-menu rigged FBX action now open the same dialog.
- Options cover exact included model / CDF assembly files, verified rigging on/off, Blender executable, optional matching original skeleton/CAF, binary-FBX embedded textures on/off, and optional source dependencies. Output shape: `<ChosenFolder>/<model>.fbx`, `<ChosenFolder>/textures/<original-virtual-subpath>/*.png`, and `<ChosenFolder>/dependencies/<original-virtual-subpath>/...`. The last two appear when dependencies are selected. Assets are not nested in an extra character folder.
- External texture dependencies are **PNG only**; no original DDS is copied to the export folder. Source `.dds.0` and `.dds` variants are decoded through `export_dds_png` with a strict `FYRC`/header-flag console-layout selection for verified 2012 Xbox tiled data; Stage 2 PC textures remain linear. Missing mip headers and unsupported formats fail with diagnostic errors, not guessed colors. Only authored, unambiguous CDF/MTL references and exact indexed same-stem materials are traversed. Imported cross-PAK material dependencies not present in the current index are not silently borrowed.
- The two supported Blender conversion scripts share `blender_uv0.py`, adapted from the private manager's PR #107 and the collaborator's [Models for painting setup](https://docs.google.com/document/d/1yeYjSwQ1-cy6eFsK4UECu3pq6qILUY96ImPfO2Z5bFo/edit). It uses source UV0 rather than an invented atlas, separates mixed original material groups, retains **one source-backed UV channel** per textured mesh, and corrects only continuity-proven near-equal corners within **1e-6 UV units**. Its repeat-tile shift affects whole connected islands only when original texture sampler evidence permits wrapping; mirrored seams, intentional UV boundaries, transforms, UDIM and unverified sampling remain intact. The collaborator's 0.001 cm weld note is spatial, not permission to globally merge UV coords. Source vertex positions, polygons, bone weights, rest hierarchies and imported CAF Actions are preserved.
- UV regression fixtures ported from the private PR cover exact/source-identical coordinates, repaired near-equal shared-edge cracks, mirrored-seam refusal, verified REPEAT whole-island tile shifts, and malformed UV rejection. CI's Linux job parses Blender scripts and runs this pure-Python suite before cargo format/lint/tests. The current code does **not** claim auto-repair of holes/capping missing polygons, manual-model duplicate skinned components, unknown shader transforms or universally correct real-game art; those need source-specific proof and scene review.

**Validation gate / checkpoint:** commit chain `cb94607` (export core), `e4a8859` (File popup), `1bda11f` (shared UV repair), `820b219` (portable UV import fix), `d70cca3`/`ca9b290` (rustfmt), `2ccc395` (console PNG decoder and Model-menu parity), all on `feature/blender-viewport-cdf-autoload`. The initial Actions run 39487702709 failed formatting and a moved-key ownership error; both were addressed in subsequent commits. **Do not claim final green until the CI run for latest HEAD completes** with Linux core and Windows desktop/portable success. Validate real Scorpid 2012 DDS/UV texture output and representative Kala, Wraith, Goliath and Behemoth rigged FBX with Blender 5.2 reimport before merging/releasing.

**Next operations:** 
1. Read current GitHub Actions for latest HEAD, apply all cargo fmt + Clippy/compiler/test corrections without disabling gates; rerun to green. 
2. Validate realistic CDF/MTL dependency closure including original `.skinm`/`.chrm` companions and high/low split DDS mip resolution across exact same-build imported libraries; enforce PNG at every external texture output.
3. Inspect exported Blender 5.2 per-material UV0 islands, sampler tiling, alpha/gloss/normal maps, original seam normals, weights, Actions and CDF equipment attachment placement; independently verify rigs and source pixel parity before declaring all models repaired.
4. Keep exported files isolated in a chosen folder, originals untouched; no release/tag/merge from CI alone.

## October 20 — Scorpid3 header-first CryTek rig directory fix (follow-up after lighting improvement)

**User retest symptom:** The Asset / Model Viewer screenshot still showed `Objects/characters/monsters/Scorpid_2/scorpid3.cdf: CDF part torso cannot be rigged: ... Historical bone/stream chunk table mismatch`, even after adding support for the **terminal 16-byte** directory. Lighting improvement from the separate toolbar slider was confirmed by the user; the persistent problem was export parsing, not the viewport or material light.

**Root cause identified in code, distinct from the previous terminal-16 change:** `legacy_rig.rs::chunks` required every chunk's `offset < table` and `upper <= table` for *all* layouts. The private manager's `source/legacy_crytek_mesh.py::parse_crytek_chunks` recognizes original 2012 0x745 character files whose **five-dword/20-byte directory begins at file offset 20, with compiled chunks after the directory**. The earlier Rust-only fixtures had modeled the five-dword directory at EOF, hiding the mismatch. This was exactly the repeated generic "chunk table mismatch" validation path.

**Implemented and verified in public Rust code:** [`cd826906`](https://github.com/corvodl/Evolve-Mod-Manager/commit/cd826906ab8d5e4293e143e4e9afcfe33772839a) supports both **header-first five-dword** and **older terminal five-/four-dword** chunk-directory layouts, selected from the actual directory span and strict bounded offsets. It never treats partial five-dword directories as terminal four-dword directories. It enforces unique record IDs and offsets, embedded chunk-kind/version/offset/ID equality, bounded non-overlapping ranges, no directory overlap and all pre-existing 584-byte bone, original rest/bind matrix, weight-sum and subset-palette invariants. Synthetic regression `original_2012_header_first_chunk_directory_preserves_bones_and_weights` checks source-identical joint IDs, influence counts and weights; negative tests reject corrupt source IDs, duplicate or directory-overlapping offsets and oversized chunk lengths. No fake bones, nearest-bone skin weights or speculative offsets are introduced.

**CI acceptance for exact source commit:** [Actions run 39500151316](https://github.com/corvodl/Evolve-Mod-Manager/actions/runs/39500151316) succeeded on `cd826906ab8d5e4293e143e4e9afcfe33772839a`: Linux core formatting, tests and Clippy; Windows desktop build, Clippy and workspace tests; Windows portable preview artifact `EvolveModManager-Rust-Preview-Windows` uploaded successfully. This artifact is **not** a stable release and does not prove real game-file rigging/Blender reimport.

**Still required for real-world acceptance:** Download the preview artifact from run 39500151316, reopen the same imported historical Objects PAK source and retry `scorpid3.cdf` as rigged FBX. Confirm all five original CDF parts and left/right equipment actually export, original UV/material assignments and PNG-only external dependencies remain intact, and Blender reimport has functional source-backed armature and weights. If a new validation error occurs, retain the full new Activity line including part name, operation and original asset paths; diagnose that exact failure without weakening constraints. The 2012 rig audit reported Scorpid3 as a five-part CDF with master skeleton compatibility, but those local observations are not equivalent to reimporting this specific Windows preview's FBX output. No release or merge until representative real-asset validation.

## Resume here first

1. Inspect PR #6's latest commit, GitHub Actions **core**, **desktop** and **Windows portable preview** conclusions. Do not assume an older green CI result covers the newest commit.
2. Run `cargo fmt --all -- --check`, `cargo test -p evolve-core --all-targets`, `cargo clippy -p evolve-core --all-targets -- -D warnings`, `cargo check --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace --all-targets`. Fix failing tests without disabling lint or source validation.
3. Resume **real Blender 5.2 character acceptance** for unified FBX, then native character CAF/DBA and offline installed-game workflows. Any real extracted assets remain out of Git and release packages.
4. Update this file after each verified advance with the exact commit/run, observation, blockers, and next operation. Keep PR open until representative real-asset acceptance and CI pass; no release just for green synthetic tests.

## October 20 continuation — 2012 equipment, verified rigging and Codex texture handoff

**Persisted in PR #6:** latest code HEAD before this documentation update was `17d1bd241ba2520d1c45a2f1fd7dacc650123f4a`. GitHub Actions [run 39469670052](https://github.com/corvodl/Evolve-Mod-Manager/actions/runs/39469670052) completed **successfully** for Linux core, Windows desktop, and Windows portable preview (not a release). Those results are for that exact code commit, **not** for an uncommitted Codex workspace or real extracted game assets.

- **New viewer sidebar Equipment tab:** `crates/evolve-desktop/src/asset_viewer.rs`, `main.rs` and `crates/evolve-core/src/character_equipment.rs` expose separate exact-path left/right choices for historical Scorpid arms. `read_static_cdf_assembly_with_equipment` validates each choice and uses the override for the preview without editing the original CDF. `export_unified_cdf_fbx_with_equipment` carries current choices to verified rigged FBX. Selections are scoped to the current CDF and reject unknown/cross-character variants. Only CDF-indexed alternative .chr families are currently offered; other equipment types need actual authored slot evidence.
- **Historical 2012 rigging:** `legacy_rig.rs` validates CryTek `0x745` original 584-byte bone records, parent hierarchy, inverse/rest matrices, stream 9 four-weight influences (exact 255 sum), and subset-local bone palettes. It remaps weighted bones to the master skeleton only if identity, ancestry and bind matrices agree; otherwise the original body-local bind pose wins. `rig.rs` now emits original-weight skinned GLB/FBX for these sources and refuses to mislabel historic DBA/CAF data as verified Stage 2 animation. Synthetic positive and rejection fixtures passed in the CI run above; Blender 5.2 reimport with real 2012 characters is **not yet verified**.
- **Scorpid default assembly:** the repository has a synthetic regression for five authored Scorpid parts (body/rig, head, petapalps, left Basher, right Basher) with unavailable bone sockets. This does **not** prove real Scorpid CDF/equipment preview is complete or fully deformable.

**Separate Codex session, stopped by usage limit (user-provided screenshot, not persisted or independently retested in PR #6):** The session reported that a previously corrupted 2012 console DDS sample decoded correctly after **Xbox 360 tiled block-address reconstruction plus 16-bit byte swapping**, and that the decoded bytes corresponded to the original PAK sample (extraction itself was not the corruption cause). It also reported discovering that **low/high mip fragments span separate imported PAKs**, while the decoder looked only beside the DDS header. Finally it reported getting Scorpid's two default Basher arms to load by tolerating empty socket declarations and mapping each arm's reduced bone table against the main skeleton. These are **reported observations**, not proof those final changes were saved, committed, or tested in this repository.

Current public Rust code check:
- `dds.rs` already has `xbox360_tiled_xy` / `untile_xbox360` and `decode_dds_rgba_with_layout(..., Xbox360Tiled)`, with a synthetic 128×128 block regression. The standard `decode_dds_rgba` still calls `Linear` unconditionally. **Missing:** verified header/platform-flag-triggered layout selection at all preview/export entrypoints. Do not untile ordinary linear DDS files.
- `load_dds` still enumerates numbered mip fragments via `fs::read_dir(parent)`. **Missing:** resolving each exact numbered virtual DDS fragment across imported archives of the *same registered build*, with duplicate/ambiguous and byte-size validation and no cross-build borrowing.
- Current CDF assembly and historical rig readers are more capable than the earlier iteration, but the **specific empty-socket and reduced-arm-table fixes reported by Codex are not independently linked to a Git commit**. Compare the Codex working tree/diff against `17d1bd2` before reimplementing or declaring these fixes done.

**Next worker tasks, in order:**
1. Recover the interrupted Codex workspace/diff if available; do not assume its uncommitted patches exist in Git. Reproduce the problem using the private sample outside the repository; record checksum, DDS header flags, source PAK IDs/build and original path privately (never commit copyrighted assets).
2. Implement strictly header-flag-controlled Xbox 360 untile + 16-bit swap in the common DDS decode call sites; test console vs standard linear DDS and unsupported layouts. Preserve raw extracted PAK bytes.
3. Resolve highest-quality mip fragments by exact case-insensitive virtual path from the same registered imported build, detect ambiguity, enforce expected mip byte lengths and include direct-vs-split DDS regressions.
4. Test **Scorpid 1/2/3** default CDF arm loads, all six independent left/right alternatives, socket declarations, reduced bone-table remapping, equipment switches and character reloads; check real GPU textured render, source bone IDs and CDF FBX reimport. Do not substitute guessed palette indices or fake limb weights.
5. Run formatter, core + Windows workspace tests, portable preview, and real 2012 sample smoke tests on the final commit. Update this section with exact new commit/run and blockers; **do not merge or release** from synthetic green CI alone.

## October 20 — uploaded 2012 dds_high extraction and image-viewer work

User supplied `files(3).zip`, a **ZIP of extracted dds_high files** (not the .pak container). Local inspection found 2,148 DDS-like texture fragments: **1,968 distinct numbered high-mip fragments without their .dds.0 header in this ZIP**, 178 .dds.0 headers (many cubemap environment probes), and two special headerless `.dds.0a` alpha auxiliaries. Nineteen Scorpid-related DDS fragments include body, head, pedipalps, poison, grabber, concussion and other equipment maps. No textures from the upload were added to Git. The highest numbered `.dds.N` fragments are the high-resolution candidates; **dds_low is needed to supply the matching format/dimensions/mip tail metadata** for the rest. Do not invent DDS formats for headerless blocks.

**Implemented in code after previously green run 39469670052 (newest run must be checked):**
- `asset_cache.rs` now recognizes the **actual** source header marker `FYRC` (not the previous mistaken `CRYF`) together with exact historical console format flags/caps. Xbox 360 compressed textures use the existing endian-swap/tiled-block decoder; ordinary Stage 2 PC DDS remains linear. Conversion never mutates the game file.
- `decoded_dds_indexed` accepts selected `.dds.N` texture assets and resolves their matching `.dds.0` plus other numbered fragments via exact local indexed path or `resolve_imported_reference`, scoped to the **same registered build**. Strict manifest-size and path checks, bounded copies into a temporary read-only preview set, and existing mip-count/byte-size checks prevent guessed or mixed-build texture pixels. Missing low DDS headers return actionable diagnostics. Source path, role, and submaterial channel are preserved in the 3D GPU CDF material resolver.
- The viewer's selected-asset preview now supports numbered `.dds.N` extensions. Standalone complete six-face cubemaps can be shown as **3×2 face atlases** (with strict face/mip size validation), rather than treated as regular flat 2D textures. Two historical `.dds.0a` files have 124-byte header-without-magic plus 8-bit alpha pixels, so they receive an explicit grayscale preview rather than being guessed as ordinary RGB.
- Added synthetic regressions for `FYRC` vs PC headers, complete six-face image atlases, imported dds_low + dds_high cross-PAK high-mip preview, and `.dds.0a` alpha auxiliary preview.

**NOT YET PROVEN:** all real 2012 textures rendering or applying correctly to Scorpid; specific console uncompressed/tiny tiled surfaces, uncommon DXGI formats and complete Stage 2 texture coverage; real multi-PAK high/low source file lengths. Uploading matching **dds_low** is the next required dataset step, because 1,968 of these high fragments are intrinsically headerless. The new feature code must pass Linux format/core tests, Windows workspace checks, and portable CI at its **final commit** before a tested release is claimed. Also run actual Scorpid 1/2/3 CDF GPU renders and six interchangeable arms per side with original material/normal/specular channels, and verify that no live source DDS bytes are modified.

## October 20 — uploaded dds_low archive, complete high/low inventory and library File menu

User supplied `files(4).zip` containing the extracted `dds_low` file tree (61 MiB compressed, 8,393 regular files, about 101 MiB uncompressed). Cross-checked against previously uploaded `files(3).zip` (`dds_high`, 2,148 regular files). The two ZIPs have **zero duplicated file paths** and **1,968 shared texture base names**, each with a `.dds.0` low header and an exact higher-resolution numbered fragment in the high ZIP. There are 178 extra high-only self-contained `.dds.0` textures, chiefly environment-probe derivatives. A bounded inspection of DDS dimensions, FourCC, mip counts, cubemap face count and exact expected part sizes found **2,146 complete texture sets** when both archives are combined, including Scorpid's `scorpid_body.dds` (512×512 DXT1: `.0` to `.4` in low, `.5` in high). This is structural completeness, **not** a claim of visually verified Xbox decoding.

Source format distribution includes BC1, BC2, BC3, BC5, CTX1, 8-bit alpha and 32-bit RGBA/cubemap; **seven previously unsupported low files** use six 8-bit luminance and one 24-bit RGB streams. The decoder now includes `DdsFormat::Luminance8` and `DdsFormat::Rgb24` plus synthetic grayscale/channel ordering tests. Historical auxiliary `.dds.0a` / `.dds.1a` etc. split streams now resolve by numbered, exact virtual path and verified same-build manifest before temporary reconstruction, including 124-byte header-without-magic `.0a`. No ZIP or extracted game source was added to the public repository.

**Requested Asset Library UI:** Asset Library → **File → Open Asset Library Folder in Explorer**, in `asset_library_page.rs`, handled by `Message::LibraryOpenFolder` in `main.rs`. It verifies the configured Asset Library root is an existing absolute directory, launches Windows Explorer for that directory, and shows actionable errors rather than opening the game or an unrelated selected archive. It does not change files, mount them or create missing directories. Added a directory validation test.

**Acceptance required before claiming all images work:** verify final PR #6 HEAD with formatting, Linux core tests, Windows desktop compile/Clippy/tests, and Windows portable preview; test real Scorpid body, arms, normals, cubemaps, and auxiliary images from the combined dds_low/dds_high archives (original assets remain local). Real-asset visual comparison against the original Xbox sample remains pending. An imported Asset Library *build* must include both the low and high manifests; ZIP uploads in the conversation are source research, not automatically installed game PAKs. A filename-only texture guess or cross-build mip merge is forbidden.

## Viewer regression work — indexed .skin and CDF assembly, authored GPU textures

User validation on 2026-10-20 identified: standalone `.skin` selections and full CDF assemblies were not viewable, associated equipment did not attach, and an 11-slot MTL showed 23 resolved DDS references but no per-face GPU textures. The screenshot was intentionally in **wireframe mode** to demonstrate source geometry; red wireframe by itself is not a texture failure.

Implemented on PR #6 after last fully green run, **pending the latest CI and real-game acceptance**:

- `auto_resources::exact_render_companion`: selecting a native `.skin`/ `.chr`/ `.cgf`/ `.cga` tries only its same-virtual-name compiled `.skinm`/ `.chrm`/ `.cgfm`/ `.cgam` via indexed assets or the manifest-verified imported PAK library. It does not select a similar filename or combine archives/builds without provenance.
- `Operation::InspectIndexedModel` and `State.render_source`: preview the exact render companion while retaining the user-selected skeletal binding in the explorer. CAF binding and GPU surface loading follow the render source, including one stored in another imported PAK. Missing compiled companions must produce actionable errors rather than silently loading an unrelated mesh.
- `read_static_cdf_assembly_with_imported`: allow a CDF to draw original `.skinm` geometry when the separate `.skin` metadata or compiled skeleton is missing. The report explicitly warns that **original skin weights, bone sockets and animation cannot be verified**; missing/ambiguous socket transforms are never invented. A genuine `CA_BONE` still requires the original verified bind-pose skeleton and matching `BoneName`.
- `assembly_materials::load_model` and main background job: after MTL discovery, upload original per-subset diffuse/normal/emissive/specular/gloss maps using strict `0x08001017` indices. With multiple MTL leaves but no original source subset mapping, refuse to paint all triangles with slot zero. Preserve NoDraw/hole face spans. The inspector reports **actual GPU-bound original subsets**, not only resolved MTL asset counts.
- Viewer layout: explorer defaults to the **left** for new settings, with a one-click left/right toggle. Shaded/wireframe switching is visible in the viewport toolbar. Existing saved explorer placement preferences are preserved.
- Synthetic tests cover precise `.skinm` companion choice, CDF static fallback when an original skeleton and separate `.skin` are absent, and authored rigid equipment socket transform placement.

Still required:
1. Latest GitHub Actions format/core/desktop/Windows portable validation, including no regressions in 2012 `CryTek 0x745` meshes.
2. **Real Stage 2 and 2012 asset tests:** Kala (torso/equipment DDS), Wraith stages, Griffin and Goliath/Behemoth skin rigs. Record the exact error and original CDF attachment names, matched render-companion virtual paths, number of GPU subset surfaces/textures, bone names and missing dependencies. Do not commit copyrighted extracted assets.
3. When a CDF lacks the true skeleton or rendered compiled equipment companion, the model must report incomplete assembly rather than falsely claim every attachment loaded. Implement original bone-paletted skinning and bone-attached animation only against real skeletons.
4. Current model viewer does **not** automatically infer which CDF owns an arbitrary selected standalone `.skin`; selecting the authored CDF is still the supported full-assembly route. Future discovery should match exact CDF Binding references, not filename similarity.
5. Source MTLs with no original mesh-subset table still cannot safely assign individual texture slots. No guessed face/material association is permitted.

## Follow-up implementation — DBA indexing, multi-material FBX, recovery phases

Work is on PR #6 branch `feature/blender-viewport-cdf-autoload`. **Run the latest core + desktop + portable checks after this commit; earlier green runs do not validate these changes.**

- **Native DBA source index** (`native_dba.rs`): `index_native_dba_bytes` now parses bounded Controller_905 named animation directory records, additive/cycle flags and original controller CRC/track-index references for both standard and in-place streaming ordering. Invalid index pairs, repeated CRCs, track offsets and truncated directory records fail. Synthetic fixtures cover both modes. **No compressed key samples are decoded; native DBA playback and UI integration remain unimplemented.** Compare real DBAs with an independent decoder before claiming format parity.
- **Rigged FBX per-material primitives** (`rig.rs`, `character_unified_fbx.rs`): original `0x08001017` index subset ranges are preserved as separate GLB primitives, sharing original UV0 and JOINTS/WEIGHTS accessors. Each original MTL leaf is matched by material ID, with independently resolved diffuse/normal/emissive/specular/gloss DDS maps embedded into its own glTF material. NoDraw leaves keep zero-alpha materials rather than shifting source subset IDs. Unknown subset 255 or missing/ambiguous exact material source fails instead of painting the entire character with one atlas. Synthetic unit test verifies material index ranges and attributes. **Blender 5.2 real-character reimport still requires material slot count/order, UV seam/alpha, skeleton and animation acceptance tests**, especially Kala equipment and Wraith stages. `fully_textured` remains false until validated.
- **Native offline recovery planning** (`native_offline_launch.rs`): `plan_native_offline_swap` now checks installed signed originals are byte-identical to the precise isolated original PAKs and hashes the planned replacements. `audit_native_offline_swap` read-only classifies `OriginalReady`, `InterruptedBeforeMount`, `OfflineReplacementMounted`, and `StrandedOriginalBackup` from exact hashes; it refuses third-party modification and unsafe/linked files. Synthetic tests create these four states without invoking the game. **Native file swapping, process launch, atomic journal persistence, power-loss-safe restore and offline DLL gating are NOT implemented or enabled.** Do not wire game-launch UI to this until independent crash recovery and offline-only validation pass.

Next implementation checkpoints:
1. Fix all latest GitHub Actions formatting/Clippy/compiler/test failures and run Windows portable CI at the final commit.
2. Exercise synthetic Blender multi-primitive round-trip; verify split meshes/material slots/UVs and source skin weights, then repeat with real Stage 2 and 2012-era assets.
3. Implement and validate actual Controller_905 compressed time/position/quaternion tracks and per-animation frame ranges, including negative-offset mode, against real DBA samples. Only then expose native playback.
4. Implement a durable native swap journal with fsync, atomic rename ordering, idempotent restore and explicit process/watchdog shutdown. Prove failure injection in isolated install clones before touching any installed game.

## October 20 continuation — original MTL slots, native CAF layers, offline launch gate

These commits build on the verified FBX publishing and viewport changes. All changes remain experimental until the newest CI, real Stage 2 assets and Windows launch recovery are independently checked.

**Source multi-material parity**
- `materials.rs` now preserves empty/self-closing `<Material .../>` leaves in their exact original depth-first order, including `Shader="NoDraw"`. The previous parser dropped those leaves, shifting GPU subset ID 1 onto material 0 and potentially putting equipment/glow textures on the wrong character part.
- `assembly_materials.rs` preserves exact source material slot ranges, and `textured.rs` does not draw explicit NoDraw subsets. A valid all-NoDraw table stays hidden instead of rendering the complete model through fallback.
- Synthetic cases exercise NoDraw/empty/body/emissive slots in their original positions. Still unverified: actual Kala multi-material mapping, texture alpha/opacity, LOD subset IDs, per-face UV assignments, and Blender multi-slot FBX export. No fabricated diffuse DDS fallbacks.

**Native CAF/DBA animation parity**
- `caf_layers.rs` now applies CryEngine-style additive **local delta** translations/quaternions in the mixer with an explicitly verified full compiled bind pose. Ordinary CAF layers still use controller-CRC-matched nonadditive interpolation. Weighted additive rotation is composed onto the actual base pose; unknown CRCs, missing/incomplete bind or invalid weights fail.
- The no-bind mixer still rejects additive clips. The source convention is from documented CryEngine animation metadata; synthetic pose tests are not proof all Stage 2 clips obey that exact convention. Native CAF assembly already accepts `mesh_at_layers`; actual GUI selection and layering controls need full live integration/verification.
- **Direct DBA playback is not implemented.** `native_dba.rs` now provides header and named-animation index validation; compressed keyframe sampling is still absent and `playable_animation_count` remains zero. DBA bundles have compressed/shared controller streams and metadata-only success is not playback. Next: use permitted real AnimationsDBA.pak samples to verify chunk formats and storage mode, parse per-animation directory/controller references, decode compressed times/positions/rotations, map original controller CRCs to the compiled skeleton, and compare samples/timing against an independent converter. Do not commit proprietary assets.

**Native offline-launch parity**
- `native_offline_launch.rs` introduces `inspect_native_offline_readiness`, a **read-only** preflight using the exact isolated stage, enabled validated profile, independently signed installed PAKs, safe game executable, distinct original/custom public keys, nonoverlapping roots and no stranded per-target backups. It returns `native_launch_armed=false` by design.
- No installed PAK is copied, swapped, injected or executed by this API. Native installed-game launch/swap, crash-safe backup journal, power-loss recovery, process-close verification, DLL offline gating and restore-watchdog handshake remain **blocking**. The private Python/Frida launcher is still a separate compatibility path and must not be silently treated as a native implementation.
- Next: design an explicit native transaction manifest recording original SHA256, target, temp/backup paths, verified custom digest, restore intent and crash recovery; test simulated mid-copy/mid-rename interruption, symlinks/case aliases, mismatched keys and disk failure in isolated temp game roots. Only after audited recovery and real offline safety should any GUI launch button invoke the native path.

**Verification:** check the latest commit/CI for format, core tests, desktop clippy/tests and portable Windows packaging before marking any of the above green. The previous Wraith Blender 5.2 report is not a checked-in fixture; live game acceptance remains outstanding.

## Exact exporter work left in progress

### What was wrong in the public branch

- `character_unified_fbx.rs` read and verified the **23-byte binary FBX header** from the temporary file, then copied without returning to byte zero. That silently produced a published file missing the complete header.
- The external Blender script's Python errors were not reliably reflected in the process exit status.
- The Blender round-trip's list of MESH objects counted Blender 5.2 **bone-display Icospheres** as character parts; such display objects may legitimately be separate meshes, but only if referenced as custom bone shapes should they be excluded.
- The previous screenshot from a separate working session **reported** that Wraith stage 3 exported with a rig and CAF Action, and that further UV/weight validation and other stages were underway. The screenshot is **not** a persisted test artifact for the current branch; do not claim this report re-tested until reproduced against extracted assets.

### Current code implementation (revalidate on latest CI)

- `crates/evolve-core/src/fbx.rs`: shared `publish_verified_binary_fbx()` checks file type, size and signature, **seeks to byte zero** before a full copy, checks copied byte count, syncs, and publishes atomically with no overwrite. Regression fixtures check exact byte equality and invalid signatures. Both single and unified exporters use this publisher.
- `crates/evolve-core/src/character_unified_fbx.rs`: calls the shared publisher; passes Blender's `--python-exit-code 1`, so script errors should abort the export.
- `crates/evolve-core/src/blender_character_fbx.py`: filters objects that are actually referenced as pose-bone custom display shapes rather than deleting anything named “Icosphere.” Reimport validates original bone hierarchy and source armature binding, verifies finite per-vertex weights referencing actual bones, checks original UV0 loop presence, compares the full sorted per-axis UV-loop coordinate distribution (tolerance 0.001) and overall UV bounds across reimport, and requires a resulting CAF Action when animation is explicitly enabled. Exception handling explicitly exits nonzero.
- No fallback bone assignment, fake UVs, source asset edits, release screenshots, or bundled proprietary game keys have been added.

### Repeatable real-asset acceptance checklist (not yet verified here)

- [ ] From a fresh, separate Windows test location, export a **single unified Wraith stage 3 FBX** with opt-in compatible CAF. Confirm Blender 5.2 exits 0; inspect file's first 23 bytes for `Kaydara FBX Binary 0x00 0x1A 0x00`, and confirm the file can be reopened.
- [ ] Reimport in a clean Blender 5.2 session: exactly the intended character rig, unchanged bone hierarchy, no bone-display Icosphere falsely classified as an exported character mesh, UV0 present and aligned, nonzero source bone weights, and an editable original CAF Action with real pose changes.
- [ ] Test Wraith stages 1–2 and compare to stage 3; test hunter **Kala** with equipment; then **Goliath/Behemoth** for compound monster rigs. Test both animation-disabled (smaller file) and opt-in animation cases.
- [ ] Exercise 2012 `CryTek 0x745` assets separately from `CrChF v7` Stage 2; keep the historical geometry, UV0 and bone compatibility independent. Current automated legacy fixture only establishes bounded mesh decoding.
- [ ] Check separate diffuse/normal/emissive maps, multi-slot materials, transparency/glow, original UV layout and source LOD. Do **not** mark `fully_textured` true until source-authored multi-slot material parity is verified.
- [ ] Force invalid CAF, missing compatible skeleton, bad UV/weight source, Blender script failure, timeout and existing destination: none should publish a partial FBX or change original files.
- [ ] Record **exact asset identifiers**, Blender version, mesh/bone/weighted vertex/Action counts, and source/output hash *locally*; no real assets or sensitive paths should be committed.

### Remaining gaps and likely next implementation

1. **FBX:** Verify actual Blender 5.2 round-trip with Wraith stages and Kala. Verify per-face UV seam corner assignment (the current complete UV-coordinate distribution catches value loss but cannot detect swapping UVs among faces), and ensure split meshes preserve skin weights and material slots. Extend the exporter beyond the present single-slot authored material limitation; preserve fully populated multi-part equipment and correct CA_BONE attachment placement. Stage character variants intentionally; confirm the whole character has one armature and correct action(s). Ensure Blender stdout/stderr diagnostic capture is bounded and does not leak private paths to public logs.
2. **Character viewport:** Finish real-stage tests of CDF per-part materials, 2012 legacy model loading, GPU lighting/grid and bone-mounted equipment motion. Current native CAF playback rejects unverified skin/bone references, by design. Fully animated layered CAF clips need coherent source bone mapping.
3. **CAF/DBA:** Native DBA packed bitstream decoding, catalog search, idle auto-selection for all valid characters, layered/additive CAF playback and exporting multiple verified FBX Actions are still incomplete.
4. **Offline mod launch:** Isolated signed-PAK staging and journaled rollback exist. Native installed-game mount/swap, process launch, verified offline safety, emergency recovery and production profiles are not parity-complete; no private launcher script/key should be included in the public repo.
5. **Secondary parity:** Full Level Editor scene dependencies, original GFX/USM editing, installer/updater upgrade acceptance, asset-library performance and other GUI controls remain release gates in PARITY.md.

## October 20 continuation — automatic Blender executable discovery for FBX

- Requested behavior: **no manual Blender path on normal Windows installations**; show the manual path only when no usable Blender install is found. FBX production still requires a separate Blender executable; no copy of Blender is shipped inside the manager.
- Desktop now checks the validated user-specified `blender.exe` first, then recognizes versioned `Blender Foundation/Blender X.Y/blender.exe` under Windows Program Files and per-user LocalAppData Programs; the newest discovered Blender **4.2+** takes priority when multiple versions exist. Steam's conventional `Steam/steamapps/common/Blender/blender.exe` is an optional fallback. Only canonical, regular, non-symlink exact executables qualify; no PATH commands, shell, downloads, or unrelated filename searching. If a saved custom path becomes stale, normal-path discovery recovers automatically.
- Resolution is performed at startup, when File → Export is opened, and when FBX export starts. Routed through bundled FBX, standalone rigged FBX, unified CDF, experimental flat CDF, and level FBX operations. PNG-only exports need **no Blender**. The File → Export dialog and Settings Integration tab only ask for a manual executable path when none was validated. Autodetected paths are **not persisted** as user preferences.
- Synthetic discovery tests cover multiple side-by-side versions (ignores unsupported 4.1), invalid/missing paths, and a valid manual override. Acceptance still requires Windows desktop CI + live Windows installation with Blender installed in Program Files; no assertion about a user's personal machine installation is made.

## Last known evidence, provenance and limits

- Before this continuation, PR #6 HEAD was `59a6b1bd69a510f1c5677022f7cb8f5e056bd6f1` and Actions run `39372627121` reported **success**. This was **before** the exporter changes above.
- A separate previous working session (screenshot, not checked-in test output) reported real Blender 5.2 Wraith stage 3 plus CAF Action, but work was interrupted at its usage limit; it named header publication, Blender exit propagation and display-Icosphere classification as blockers. Its local tests, temp files and screenshots are **not automatically available** on this branch.
- For the current changes, trust only CI at the latest PR head and new independently recorded local Blender tests. Every unresolved checkbox remains a blocker, not an implicit success.

## Operating constraints

Keep the public repository independent of the private manager's proprietary assets, credentials and personal install paths. Keep documentation focused on implementation status, evidence, and reproducible verification. Keep release assets limited to the requested portable application and installer; development artifacts are not a release. No merge/tag until required checks and real-game acceptance are complete.

## October 20 — Scorpid3 legacy rig parser, viewport illumination, home UI and Windows console

- Reported screenshot in read-only Asset / Model Viewer: `Objects/characters/monsters/Scorpid_2/scorpid3.cdf` export refused on the torso with `Historical bone/stream chunk table mismatch`. This was from `legacy_rig::chunks` enforcing only explicit 20-byte directory entries. The private 2012 historical mesh parser supports either a complete five-dword directory or a **terminal four-dword** directory whose sizes are derived from sorted, unique, in-range offsets; that conservative second layout is now supported in the Rust legacy rig reader as well. It still verifies every embedded 16-byte header, all chunk IDs, disjoint bounded ranges, bone rest/inverse, source weight sums, skin vertex counts and exact subset palettes. Added synthetic positive test for authentic four-dword layout and negative tests for duplicate offsets/forged embedded headers. **Not yet proof that Scorpid3's genuine original bone/weight data are valid**; rerun using the user's extracted CDF/body/master skeleton and capture any next exact error.
- The separate Asset / Model Viewer now has a sun icon and **live preview-light slider** next to the camera/zoom icons. Preview intensity range is 0.2×–4.0×; default 1.6×. WGSL and GPU vertex path share the expanded range. A matching range remains in viewer Preferences; Save preferences records the selected value. No texture bytes or material identities are changed.
- Home workspace tile icons now align vertically with label/description through an aligned row. Existing Settings SVG artwork is preserved.
- Desktop `main.rs` uses `#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]` to stop Windows showing a persistent **application console** alongside the GUI for release/portable binaries; existing crash diagnostics remain file-backed. If a separate external utility console persists, investigate its process tree before applying broad process-launch flags.
- Branch: `feature/blender-viewport-cdf-autoload`, PR #6. Commits: `e7a501a`, `8db61a0` (CryTek magic literal correction), `5f44a6c` (original Settings SVG preserved), `ae92dff` (rustfmt). Run final-head Linux/Windows CI, then test actual 2012 Scorpid3 and verify brightness controls and console behavior on an installed/portable Windows EXE. Do not merge or publish an unverified release.



## October 20 — follow-up: actual Scorpid3 archive compared against private 2012-to-FBX pipeline

**Observed failure and exact evidence.** The supplied original 2012 `characters/` ZIP was inspected locally (not added to Git). Unlike the previous synthetic four-dword directory case, `Scorpid3_Rig.chr`, `Scorpid3_Skeleton.chr`, `Scorpid3.chr` and CDF attachment files use five-dword header-first chunk directories. The first `0xCCCC0013`, version 0, chunk is the original NUL-terminated Perforce source filename, **not** an embedded `kind/version/current-offset/id` binary header. Rust's `legacy_rig::chunks` had required that embedded header for **every** directory entry and rejected the valid path record as `Historical bone/stream chunk table mismatch`. Adding support for terminal four-dword tables alone could not fix this.

**Private implementation comparison.** The private manager's `legacy_crytek_mesh.parse_crytek_chunks` validates all directory bounds, duplicate IDs and interval overlap; `legacy_crytek_rig._one_chunk` and `_legacy_streams` require embedded headers **only** for the actual consumed bone, subset and stream chunks. It reads original 584-byte bone records, source inverse/rest matrices, source weight stream 9 and each subset's 260-byte local bone palette, mapping active influences by verified bone identity, ancestor and rest pose. Only then does it create a skinned GLB and invoke Blender for FBX. The public Rust parser needs the same separation between harmless authored metadata and consumed rig data.

**Implemented in the public PR #6 feature branch.** The 2012 rig reader now recognizes bounded `0xCCCC0013` v0 source filename metadata without fabricating a binary header. It also recognizes the independently observed `0xCCCC000D` v0x826 small auxiliary record whose embedded source offset is stale, but whose kind/version/id match. Both exceptions are narrowly typed and bounded; source bone, stream and subset headers, unique chunk IDs, offsets, non-overlap, source bone matrices, exact weights and palettes continue to be validated. Regression cases cover both original layouts plus forged rig chunk IDs and unterminated path text. Source file contents remain unchanged.

**Local original-asset audit (outside CI).** Scanned 450 original `.chr` files: 444 contain raw source-path metadata inconsistent with the previous Rust universal-header assumption, and 21 also contain the stale auxiliary-offset record. For the actual `scorpid3.cdf`, the original torso binding is `Scorpid3_Rig.chr` (not `Scorpid3.chr`), plus head, petapalps and two bashers. Independent read-only checks of these five CDF parts found valid original four-weight sum 255, source subset palette coverage and active joint rest/ancestor agreement with `Scorpid3_Skeleton.chr` (124 source master bones). The separate `Scorpid3.chr` mesh has incompatible weighted rest transforms and must keep its own body bind pose, as the private exporter does. This is not a claim of successful Blender export.

**Verification still required.** Verify newest Linux/Windows CI after the final patch; exercise real Rust decoding against the exact locally extracted Scorpid3 files, then Blender 5.2 import/export/reimport with original UV0 loop mapping, active bone weights, CDF equipment, texture roles and preserved source hierarchy. Repeat on other 2012 fauna and monsters. If a new error remains, preserve the exact failing file/part and new diagnostic rather than broadening format detection indiscriminately. In addition, finish independent full export-dialog acceptance and UV reconstruction tests; UI checkboxes alone do not prove every legacy source exports correctly. Do not merge, tag or release from synthetic tests alone.


## October 20 — texture parity: private fixes validated first, native model/CDF port

### Private reference first (separate draft PR #111, unmerged)

On `corvodl/Modded-Evolve-Mod-Manager`, draft PR #111 on `fix/texture-source-build-and-hires-loading` fixes the original Python/Qt path before any Rust port. The private model-viewer CI succeeded at source commit `2d96aac19959cedf8c60528e819139649c7960f1` (run 39506164621). Its synthetic tests cover exactly authored DDS0-before-DDS loose-folder selection, actor-specific full virtual texture paths, `build_id`-scoped cross-PAK numbered mip chains, complete 6-face cubemap contact-sheet images, bounded `.dds.0a` 8-bit auxiliary alpha streams, and explicitly verified original FYRC Xbox CTX1 normal-map preview. A single CI-failing `KeyError` and an intermediate syntax error were fixed **before** the successful check. The private PR remains a draft and must not be merged based only on synthetic acceptance.

Independently audited the two supplied read-only uploaded archives `dds_high` and `dds_low` (no archive bytes added to Git): all 2,151 observed numbered DDS families have contiguous fragments across the two files; there are no duplicate exact virtual paths. The archives also contain genuine cubemap, CTX1 and `.dds.0a` auxiliary fixtures. The presence of a numbered set proves fragment completeness, **not** every source console surface tag is decoded correctly: unsupported formats are still diagnostics, never guessed colors.

### Public Rust port (PR #6; CI check on final HEAD required)

- `assembly_materials.rs`: model and CDF authored map loaders now use one exact `material_virtual_owner` and `scoped_texture_candidates` path. Relative map files search their original owning MTL directory, that character's `textures/` directory, then the appropriate Hunters/Monsters/Creatures Shared TS namespace, while honoring rooted MTL paths and a limited explicitly authored global texture path. The `.dds.0` high-resolution candidate precedes the lower `.dds`, and every candidate is verified by local inventory or the correct selected imported build before bytes are decoded. If none matches, the material slot retains an explicit missing-texture diagnostic rather than borrowing another actor's image.
- Added synthetic tests in `assembly_materials.rs` for an actual MTL-relative `.tif` finding the owned `.dds.0`, refusing unsafe/traversing paths, and never searching another actor's basename.
- `main.rs`: the read-only viewer auto-preview recognizes every bounded numbered `.dds.N` and `.dds.Na` (up to 19), not just `.dds.0`; existing `asset_cache.rs` already verifies the matching `.0` header, rebuilds same-build high mips and six-face cube previews, and supports wrapped 8-bit source auxiliary streams.
- `dds.rs`: Xbox CTX1 block decoder retains the exact source RG interpolation and now reconstructs the nonnegative tangent Z component for meaningful normal-map image and PNG previews, matching the private manager's helper. The native WGSL path already reconstructs tangent Z from XY when rendering.
- No private Python files, proprietary original DDS/PAK assets, credentials or signing keys were copied into this public repo. Only bounded Rust resolution/decoding changes and synthetic tests.

### Still blocking full private/public visual parity

1. Final-head Rust Linux core + desktop tests/Clippy and Windows portable build, then on-device interactive texture preview. The last known green public CI before these changes was run 39503974567 and does **not** prove the new changes.
2. Real original 2012 Scorpid1/2/3, Wraith (internal Teleporter) and Stage 2 Kala/Behemoth/Goliath previews. Confirm per-submaterial original slots, no NoDraw bleed, correct opacity/tint/gloss, highest correct mips, UV seams, true source file/provenance and no stall while uploading large GPU textures.
3. The Python/Qt manager's progressive per-frame GPU uploads, detail/normal/AO/reflection/environment slots, build-scoped identical-byte duplicate policy, flexible material overrides and normal conversion options are **not all mirrored** in the Rust viewer yet. Do not claim one-for-one shader/material parity merely because source maps can be found.
4. Static FBX and every multi-slot unified FBX texture round-trip; current public static export reports missing verified embedded material maps in some paths. Validate embedded vs external `<user folder>/textures/` dependency output and source-consistent image pixel hashes. Keep game originals read-only.
5. Avoid any release/merge until CI and real-source visual/Blender acceptance are recorded. If a specific Scorpid texture remains missing, record the original MTL path, material slot, virtual DDS reference and exact build-qualified resolver diagnostic, then fix the verified source rule rather than a global basename guess.


### Follow-up: one native texture resolver for GPU preview and CDF FBX

The original private texture resolution methodology was also carried across the **export** boundary. Previously the Rust viewer and the unified-character FBX exporter used separate DDS lookup heuristics: the viewer could find `characters/monsters/scorpid_2/textures/body.dds.0` but the FBX path looked only for the raw global `body.tif` or same-stem `Scorpid3_Rig.mtl`. The updated public core exports `scoped_texture_candidates` from `materials.rs`; both `assembly_materials.rs` and `character_unified_fbx.rs` use it for exactly scoped source texture candidates and high-mip priority. The FBX exporter also carries a CDF attachment's explicit `Material` override, and accepts an exact source-verified 2012 variant owner MTL such as `Scorpid3.mtl` for `Scorpid3_Rig.chr`. Explicit missing materials fail closed instead of borrowing a different actor's variant. Synthetic tests cover both single-slot and source-subset material PNG paths, preserving original source identities. Normal CTX1 RGB preview reconstructs source XY Z while WGSL retains original RG.

This repairs material lookup and embedded image preparation for supported local CDF sources only. A complete verified Scorpid3 Blender FBX visual round-trip, Stage 2 high-resolution native GPU on Windows, standalone static FBX embedded-texture semantics, arbitrary imported-library dependencies, all private detail/AO/environment shader channels and per-frame progressive GPU uploads remain separate acceptance gates. Green Linux/Windows tests on final code HEAD must be rechecked after these commits; synthetic data is not equivalent to real source acceptance.


### Developer offline live Float32 editing (PR #58, experimental)

The Developer menu now provides opt-in single-shot Float32 edits for a behavior-verified Goliath armor candidate or an explicitly read manual address. The user must first confirm an isolated offline match, separately enable live writing, inspect the current Float32, and enter a new finite value in the inclusive range 0–100000. Each write compares the four-byte baseline again immediately before writing with Win32 `WriteProcessMemory`, and readback must match to claim success. No auto-freezing, debugger attachment, pointer scanning, or repeated writes are performed. The 32-bit value encoding is independent of the 64-bit address width.

The manager resets write authorization and address state after developer lock, offline confirmation withdrawal or process restart/redetection. The offline guard fails closed when detecting remote established TCP connections or when its Windows query fails, but cannot establish that a game is offline over UDP or other transport. Manual addresses do not imply verified actor identity and can crash the game if selected incorrectly. Windows CI and actual isolated-offline-match behavior must be confirmed before release.
