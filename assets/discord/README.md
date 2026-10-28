# Evolve Discord Rich Presence — complete character portrait registry

The game-specific Discord application is **Evolve**, ID `1565093139616763987`.
The separate Evolve Mod Manager application remains `1563784115785048114` and keeps its own artwork.

## Portrait import

The supplied `CharacterImageHiRez.zip.001` contained **39** 1800×1500 DDS
portraits with transparent alpha. All 39 have been converted to centered,
transparent **1024×1024 PNGs**, keeping the original art and alpha. The
downloadable `Evolve_Discord_Character_Portraits_39.zip` bundle contains the
complete `portraits/` folder, upload instructions, and the same metadata as
[character_portraits.json](character_portraits.json).

**The PNG binaries are not bundled in the Rust repository.** They must be
uploaded to the **Evolve game application**, not the manager application:

1. Open the [Discord Developer Portal](https://discord.com/developers/applications/1565093139616763987).
2. Navigate to **Rich Presence → Art Assets** for the Evolve application.
3. Upload *all 39* PNGs in the bundle, including characters that have no
   resolved internal entity name yet.
4. Keep each lowercase filename stem as its asset key (without `.png`).
   For example: `markov`, `grounder`, `teleporter`, `rockybobcicle`,
   and `goliathmeteor`. Discord normalizes uploaded keys to lowercase.
5. Confirm that the keys shown in Discord match `asset_key` in
   [character_portraits.json](character_portraits.json).

The Discord application supports up to 300 uploaded Rich Presence art assets,
so the 39 portraits fit. The repository cannot verify which PNGs have been
uploaded or upload them without authorized access to the Developer Portal.
Until uploaded, text Rich Presence still works; missing images may not render.

## Adding internal names as they are discovered

The manifest lists **all 39** portrait files. **All 39 have exactly one
canonical playable name**. The `scrap.png` portrait belongs to **Jack**,
whose internal name is **`Scrap`**; the `tech.png` portrait belongs to
**Kala**, whose internal name is **`Tech`**. The earlier `TrackingGBot`
and `SupportGBot` entries are not assigned as extra Jack aliases.
**Necro Lazarus is unreleased and excluded** from the playable roster,
registry, and Rich Presence mappings.

A single exact, case-insensitive local player entity name activates a portrait.
Known co-op actors (such as `GoliathCoopBot`), summoned minions, decoys and
NPC copies are listed separately under `non_player_entities` and are **not**
mapped to player portraits. They are not additional alternate names for
playable characters.

When an entity name is confirmed through the validated local IEntity resolver,
edit only that character's `internal_names` list. For example, to support a
verified Val name (illustrative name only, **not** a discovered ID):

```json
{
  "display_name": "Val",
  "asset_key": "val",
  "internal_names": ["REPLACE_WITH_ACTUAL_VERIFIED_VAL_ENTITY_NAME"]
}
```

No new image conversion, Discord upload, or Rust code branch is needed.
Rebuild the manager so its compile-time registry includes the updated alias.
The lookup is case-insensitive but otherwise exact: unknown names will not
be guessed or misclassified. The `scrap` art key resolves Jack (`Scrap`),
and the `tech` art key resolves Kala (`Tech`).

The original naming distinction is important: the supplied **art** filename
is `rockybobcicle.dds` (Discord key `rockybobcicle`), while the corrected
**entity** name is `RockyBobcicleBot`. This displays **Glacial Behemoth**,
not "Bobcicle Behemoth". The roster's `KrakenElderBot` and
`GoliathMeteorBot` also replace older provisional suffix guesses.

The manager displays each portrait as Discord `assets.large_image` only
when the game is running and the user has enabled character visibility.
Minimal/privacy mode and unrecognized actors do not expose a character image.
Discord's registered application icon is not changed.

## Testing

Run `cargo fmt --all -- --check` and `cargo test -p evolve-mod-manager` on
Windows, then validate an actual Discord session after importing art assets.
Verify Markov, Hyde, Goliath, Wraith, and each monster variant; switch
characters and confirm `large_image` follows the local actor. If another
in-game RPC publisher runs, activity precedence is still best-effort.


## Canonical playable-character names

Each supported playable character has **one** entity name in the manifest.

| Category | Character displayed | Internal entity name | Discord art key |
|---|---|---|---|
| Assault | Markov | `AssaultEBot` | `markov` |
| Assault | Blitz Markov | `AssaultE_BlitzBot` | `markovblitz` |
| Assault | Hyde | `AssaultDBot` | `hyde` |
| Assault | Hellfire Hyde | `AssaultD_HellfireBot` | `hydehellfire` |
| Assault | Parnell | `AssaultBBot` | `parnell` |
| Assault | Parnell (Tank) | `MedicParnellTankBot` | `parnelltank` |
| Assault | Torvald | `AssaultFBot` | `torvald` |
| Assault | Lennox | `AssaultGBot` | `lennox` |
| Assault | Renegade Abe | `AssaultAbeRenegadeBot` | `aberenegade` |
| Trapper | Griffin | `TrackingABot` | `griffin` |
| Trapper | Electro Griffin | `TrackingA_ElectroBot` | `griffinelectro` |
| Trapper | Abe | `TrackingCBot` | `abe` |
| Trapper | Maggie | `TrackingEBot` | `maggie` |
| Trapper | Savage Maggie | `TrackingE_SavageBot` | `maggiesavage` |
| Trapper | Crow | `TrackingFBot` | `crow` |
| Trapper | Jack | `Scrap` | `scrap` |
| Medic | Lazarus | `MedicBBot` | `lazarus` |
| Medic | Caira | `MedicCBot` | `caira` |
| Medic | Lowgrav Caira | `MedicC_LowgravBot` | `cairalowgrav` |
| Medic | Val | `MedicEBot` | `val` |
| Medic | Rogue Val | `MedicE_RogueBot` | `valrogue` |
| Medic | Slim | `MedicFBot` | `slim` |
| Medic | Emet | `MedicGBot` | `emet` |
| Support | Bucket | `SupportABot` | `bucket` |
| Support | Captain Bucket | `SupportA_CaptainBot` | `bucketcaptain` |
| Support | Hank | `SupportCBot` | `hank` |
| Support | Sgt. Hank | `SupportC_SgtBot` | `hanksgt` |
| Support | Cabot | `SupportEBot` | `cabot` |
| Support | Battle Cabot | `SupportE_BattleBot` | `cabotbattle` |
| Support | Sunny | `SupportFBot` | `sunny` |
| Monsters | Goliath | `GoliathABot` | `goliath` |
| Monsters | Meteor Goliath | `GoliathMeteorBot` | `goliathmeteor` |
| Monsters | Kraken | `GrounderABot` | `grounder` |
| Monsters | Elder Kraken | `KrakenElderBot` | `krakenelder` |
| Monsters | Wraith | `TeleporterABot` | `teleporter` |
| Monsters | Behemoth | `RockyABot` | `rocky` |
| Monsters | Glacial Behemoth | `RockyBobcicleBot` | `rockybobcicle` |
| Monsters | Gorgon | `LurkerABot` | `lurker` |

`Tech` is retained as a PNG asset with no confirmed player entity name. The artwork is ready for a later verified mapping. This is not a separate identified playable character.

## Compact in-game Rich Presence text

The game activity intentionally uses Discord's **two native text fields**,
rather than embedding newline characters that Discord may not render:

- **Details:** `Playing as Goliath · Stage 1` (stage only when the
  detected local character is a monster and stage 1–3 is validated).
- **State:** `Health 10,400 · Armor 2,344` (armor only for monsters).

Hunters display `Playing as Markov` and `Health 1,600`.
The redundant general game-version label `Stage 2`, game-mode/level guesses
and mod-profile details are omitted **while playing**. Manager presence still
shows manager status; the minimal/privacy toggle continues to hide character
and stats. Numeric Float32 telemetry is rounded to readable whole values.
