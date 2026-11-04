# Legacy monster ability allocation prototype

Source-only patch tools for the **adapted Legacy PreRound movie** used by the
private Stage 2 character-select experiment. The game movies, ability XML,
asset index, signing keys, and signed PAKs are not part of this repository.
Do not apply these patches to stock Stage 2 `characterSelect.gfx` or to an
unmodified Legacy `preround.gfx`.

Starting from the previously adapted `m40` `scripts/frame_1/DoAction.as`:

1. Copy it to a new FFDec script-import tree and run
   `python tools/patch_preround_m41.py <copied-DoAction.as>`.
2. Supply an external `index.pkl` containing the extracted Stage 2 asset
   index (`lowercase member path -> (tag, directory, member path, ...)`), with
   the indexed ability XML in the corresponding `directory/files/member path`.
   Generate `monster_ability_info.as` with
   `python tools/generate_monster_abilities.py <output.as> <external-index.pkl>`.
3. Copy the **m41** ActionScript to a new import tree and run
   `python tools/patch_preround_m42.py <copied-DoAction.as> <output.as>`.
   Import that tree into the adapted base movie using FFDec. Keep the
   monster selection's existing `PreRound.xml` host binding.

The generator requires exactly four slots and three levels for each of nine
Stage 2 monsters. The menu clears the previous monster's data before requesting
the native abilities, uses those XML definitions when the callback returns
nothing, and offers a four-point initial budget only on that local-data path.
Spending the fourth point submits the four level-suffixed IDs through the
existing `AcceptAbilities` callback before advancing to perks. Undo reverses
one spend and refunds one point.

**Experimental:** the stage-context native `GetAbilityPoints` returned
`undefined` during the last in-game probe. A four-point UI budget is not proof
that the game accepted the loadout. In-game testing must confirm the AP screen,
undo/back/monster-switch behavior, and the actual monster abilities in a match
before this can be considered a gameplay fix. Never sign or launch while
`Evolve.exe` is running.
