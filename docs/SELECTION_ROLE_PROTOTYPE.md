# Stage 2 selection: isolated role interaction prototype

The role prototype edits **the stock Stage 2 `Libs/UI/roleSelect.gfx` root
ActionScript**, not Legacy's incompatible `team_selection.gfx`. It retains the
native `SelectClass` call, lock/claim updates, observer option, controller
navigation, and back action. The first screen says **SELECT YOUR ROLE** and
shows Hunter and Monster (plus Observer when enabled); Hunter opens a second
step with Assault, Tracking, Medic, and Support. The Hunter card temporarily
reuses the Stage 2 Assault icon. Selecting it alone does not call `SelectClass`.
This is an **interaction prototype**, not visual parity with the supplied
Legacy screenshots, and it has not been observed in-game.

To build from *your own authenticated stock movie*:

1. Use `evolve-cli signed extract-member <signed-ui-assets.pak> <verified-original-public.der> ui_assets.pak Libs/UI/roleSelect.gfx <scratch>/roleSelect.gfx`.
   This new command verifies the signed PAK and member CRC and publishes only a
   new scratch output file, without expanding the whole archive. If your
   manager stage already contains changes to this member, extract and patch
   **that staged member** instead, using the stage's custom public key.
2. Use `python scripts/build-stage2-role-prototype.py --stock <scratch>/roleSelect.gfx --ffdec <ffdec-cli.exe> --output <new-temp-folder>/roleSelect.gfx`.
   The builder accepts output only under the system temporary directory,
   refuses an existing file, modifies only the root script, re-exports the
   result, and does not install it.
3. Use the manager's isolated signing keys and `evolve-cli signed rebuild` to
   replace exactly `Libs/UI/roleSelect.gfx` in the **manager-staged**
   `Game/UI_Assets.pak`; the signed basename is `ui_assets.pak`. Authenticate
   the result and compare untouched member hashes. The output PAK should be
   kept outside the stage, with the physical filename `UI_Assets.pak` when
   registering it in a manual profile.
4. A manual profile can register this PAK alongside the separate UI_Data
   selection probe and plan/apply both in **one reversible isolated stage
   batch**. The manager's Verify selected action now summarizes added, changed,
   removed and case-renamed signed members per batch. The advanced offline
   launcher (not the main enabled-project Play button) still authenticates
   every live original PAK before making any temporary swap. A profile plan is
   **not** evidence of game-side acceptance.

Do not copy either output PAK into the installed `Game/` directory. The
configured live installation currently has original-key authentication
failures; resolve them through trusted repair/provenance before an offline
launch. Do not bypass this check or overwrite existing mods to force a test.

**Still missing:** Legacy artwork/composition of the role screen, functional
3D CharacterStage/model port over stock Stage 2, moving-background alignment,
full monster and hunter model rollout, and in-game verification of ability,
perk, purchase, skin, readiness and callback paths. The UI_Data probe alone
disables perks; testing it alongside this role prototype is not a test of
those missing features.

The user-owned Legacy installation does contain a signed
`Level_UI_Menu_Levels_data.pak` (25 authenticated members) and a signed
`Levels/UI_Menu_Levels/CharacterStage/level.pak` (24 authenticated members).
Stock Stage 2 has neither exact archive target. These are useful source assets
for porting the stage, **not** ready replacement files for the existing
manager's authenticated-target profile. Do not create a new live root PAK or
assume that adding the level members to `UI_Data.pak` makes the map mount; load
precedence and game-side discovery require a reversible offline test first.
