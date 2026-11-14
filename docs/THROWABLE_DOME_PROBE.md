# Stage 2 throwable dome — experimental PAK probe

This is an **untested gameplay experiment**, not a verified restoration of the
legacy dome. It replaces just
`libs/Items/Mercs/Shared/MobileArenaTriggerWeapon.xml` in `libs.pak`.
All Stage 2 hunter actor loadouts already reference that item. The override
makes it selectable and substitutes the shipped `Throw` fire mode targeting the
shipped `MobileArenaAmmo` (`MobileArenaProjectile`). The shipped
`MobileArenaSharedWorld` ammo reference is retained. The normal native
`MobileArenaTrigger` fire mode is removed from this test item.

**Unknown until tested:** whether F fires or equips this changed item, whether
the game can throw a `MobileArenaProjectile` via the generic `Throw` fire mode,
whether animations work for non-Trappers, and whether the native shared cooldown
and single-active-dome rule still apply when bypassing `MobileArenaTrigger`.
The existing arena ammo has static physics; it may not fly or deploy at all.
Do not use this PAK in public matchmaking or assume it meets the one-dome rule.

The override builder is `scripts/experiments/build_throwable_dome_probe.py`.
It requires a **verified, decoded XML** copy of the exact target archive's
original weapon definition. The original Stage 2 member is CryXmlB, so extract
and decode it with `evolve-cli signed extract-member` and `evolve-cli cryxml
decode` first. Place the decoded XML at the same member path under the supplied
`--extracted-libs` directory. The script refuses unknown versions and existing
output files. Encode its XML output with `evolve-cli cryxml encode` into the same
archive-relative path under an isolated overlay directory before running
`evolve-cli signed overlay` with **locally generated** signing keys. Never put
keys or original game assets in the repository.

For local testing, use the corresponding `inject-custom.dll` prepared with
`evolve-cli hook prepare` against the **same custom public key**. The new PAK
will not authenticate under the stock key or another custom hook. Verify with
`evolve-cli hook pak-preflight`. Use the manager's isolated, reversible signed
PAK/profile workflow; never overwrite the installed `Game/libs.pak` by hand.
The test PAK and hook must be installed together through a matching test
profile. This probe does **not** change the user's existing stage or game.

Private-match test checklist: try F far from the monster, select the dome via
normal weapon controls if F does nothing, throw toward open ground and a wall,
check deploy/cooldown/takedown, then have two hunters throw simultaneously.
Check server and client logs for item/fire-mode/ammo failures. Restore the
unmodified profile after testing. Failure at any gate means PAK-only porting
needs more research, not that the throwable dome works.
