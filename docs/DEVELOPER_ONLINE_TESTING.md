# Developer online mod testing

The public manager defaults to its recoverable offline modded-launch flow. The password-protected **Developer** tab has a session-only checkbox:

**Allow Play with Mods online — any mod type (Developer testing)**

With the checkbox ON, the Home page says **Play with Mods Online · Dev Test**. That launch includes **every enabled Mod Project**, regardless of whether its changed members are cosmetic, gameplay-affecting, mixed, or unreviewed. The manager does not classify or reject project contents for this developer launch. If there are overlapping PAK targets, an invalid signing workspace, unfinished GFX edits, or unsafe/unresolved original backups, the existing local build/swap integrity checks still apply; this switch is **not** a bypass for corrupt game installation or abandoned recovery transactions.

The manager uses the *normal installed Modded Evolve client* and the established reversible local PAK swap. It does not add an `-offline` flag. It also does not patch remote authentication, the multiplayer server, or any anti-cheat logic. **Online connectivity is not guaranteed** merely because the local manager permits the attempt. Test only where the server's operator permits modified clients and gameplay assets.

When the checkbox is OFF, Play with Mods is the usual offline workflow. Developer must be unlocked to change the checkbox; it defaults to OFF whenever the manager starts, is not saved to settings, and resets to OFF whenever Developer is locked. Turning it on disarms the separate, explicitly offline Float32 memory-edit workflow. The clean/unmodded client button remains unchanged.

This switch only changes the manager's launch intent/content restrictions. The modded launcher still needs a separately verified end-to-end online test. It should not be shown as a guaranteed "connected" status until the client confirms an online session.
