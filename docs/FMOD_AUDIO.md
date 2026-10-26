# FMOD audio assets (Evolve Stage 2)

The Asset Library recognizes three legacy FMOD file types: `.fdp` (FMOD Designer projects), `.fev` (event definitions), and `.fsb` (sound banks). The native `evolve_core::fmod::inspect_fmod_file` reader validates FSB5 and FEV headers and summarizes bounded FDP XML projects without executing third-party code. It does **not** decode audio, edit banks, or alter PAK files.

## Source files and recommended toolchain

The test source `sounds_Monsters.zip` contains 34 FSB5 sound banks, 12 FEV files, and 12 FDP projects. Its FDP waveform entries reference original WAV files that are not included. The FSB5 banks use primarily FMOD Vorbis (codec 15), with one IMA ADPCM (codec 7).

For listen/export tasks, use a separately installed vgmstream build supporting the specific bank codecs. For replacement builds, use a **compatible legacy FMOD Designer version**, if lawfully available, with replacement source audio. Newer FMOD Studio is not a drop-in reader/compiler for legacy FDP/FEV projects. Do not bundle proprietary FMOD editor binaries without redistribution permission.

A sound replacement workflow must work against **copies** in Modding projects: map the source waveform to its existing event, use an isolated build directory, preserve the event name/ID, bank name and encoding constraints, verify rebuilt bank/event headers, then stage an existing PAK-member replacement with backups. Never imply in-game sound playback is verified until tested in Evolve offline.

## Tool integration

The existing Settings → Integrations pattern (configurable executable path, auto-detection, verification, hidden background commands, optional installer components) can support FMOD Designer and vgmstream. A manual FMOD path is the licensing-safe default; bundling vgmstream requires including its applicable notices and validated upstream release hashes. A dedicated **Audio Tools** section and bank/sample browser remain follow-up UI work. The initial classification and inspection support must not be described as editing support.

## Compatibility checkpoints

1. Verify that an extracted FSB5 sample can be decoded and played.
2. Build a replacement sound bank matching Evolve's FMOD runtime version and event references.
3. Confirm that the Mod Manager safely replaces the corresponding member within the original PAK without adding unrecognized paths.
4. Test the replacement in an offline game before claiming sound editing or game compatibility.
