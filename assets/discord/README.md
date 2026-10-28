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

The manifest lists **every** portrait. `internal_names: []` means artwork is
ready but the game identity is not verified — it does not mean the portrait is
unavailable.

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
be guessed or misclassified. `Scrap` and `Tech` remain unclassified until
their game identities and team roles are verified.

The original naming distinction is important: the supplied **art** filename
is `rockybobcicle.dds` (Discord key `rockybobcicle`), while the current
provisional **entity** alias is `RockybobsicleABot`. Both are mapped through
the same manifest record; neither spelling needs to be changed.

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
