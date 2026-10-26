# Evolve FMOD audio tools

The Asset Library recognizes legacy FMOD Designer `.fdp` projects, `.fev` event definitions and `.fsb` FSB5 sound banks. The manager inspects source headers and project metadata without executing sound code or modifying protected archives. The supplied `sounds_Monsters.zip` contained 34 FSB5 banks, 12 FEV files and 12 FDP projects, but *not* the original WAV sources.

## Optional tools

**Settings → Integrations → Audio Tools** supports vgmstream r2117 and separately installed FMOD Designer. The Windows setup offers a separate, optional vgmstream component and the manager can download the SHA-256 pinned upstream vgmstream Windows x64 release on demand. It includes the decoder and applicable upstream copyright notices. The portable build does not include it.

The legacy `fmod_designer.exe` is **not** bundled or downloaded. Point Integrations at a legitimately installed copy; modern FMOD Studio does not directly build these legacy FDP projects.

## Native Rebuild Sound Bank (experimental)

For Evolve's existing **FMOD Vorbis FSB5** sample slots, the Rust manager can rebuild a full multi-sample bank without running FMOD Designer. The supplied monster archive has **33 Vorbis banks and one IMA ADPCM bank**; the latter is not supported by this native path.

Install both **vgmstream** and the MIT-licensed **oggvorbis2fsb5** helper via Settings → Integrations → Audio Tools (or configure their installed executables). For replacement **WAV** files, also configure FFmpeg, which converts WAV to Ogg Vorbis at the exact channel count and sample rate of the selected original sound. For **OGG** input, FFmpeg is not necessary.

In an *editable Modding project*, right-click the existing .fsb, enter the **1-based sample index**, and choose **Rebuild Sound Bank · Replace WAV/OGG**. The audio pipeline is:

1. WAV to Ogg Vorbis (optional configured FFmpeg)
2. Ogg Vorbis to single-subsound FSB5 (open-source remuxer)
3. Native Rust FSB5 multi-sample rebuild: preserve original bank sample order, names, unaffected samples and unknown extra chunks; copy the new Vorbis setup chunk and encoded sample bytes; update sample count and aligned data offsets
4. Decode the rebuilt selected subsound with vgmstream. Abort rather than stage if its Vorbis setup or packet format cannot be decoded.
5. Stage the modified bank as an *existing manifest-listed file* through the manager's normal EditorBackups/PAK workflow.

The manager rejects mismatched channels/rates, missing Vorbis metadata, malformed donor/header/offsets and unsupported codecs. Original Asset Library PAKs are never changed during editing.

**Compatibility is not yet verified in a running Evolve match**. FMOD Vorbis is not generic Ogg Vorbis; a codebook/setup emitted by an independent encoder may be incompatible with this game's specific FMOD runtime even if the FSB header and standalone decoder pass. The current feature is a guarded experimental replacement builder; it cannot yet add new FMOD events or compile FEV event structures.

## Using it

1. Open an existing extracted FMOD asset in the **Asset Library → File Explorer**. Inspect the FSB5 bank to see its encoded format and sample count; inspect an FDP project to see waveform references. Right-click a selected asset to access its **Audio Tools** controls.
2. On an FSB, enter a **one-based sample number**, then **Preview sound** or **Export selected WAV**. The decoder converts the chosen subsong into bounded, disposable scratch space. The Windows preview uses a hidden PowerShell-hosted .NET WAV player; it does not alter the bank or game files.
3. To change a sound, open an editable **Modding project** containing the original FDP. Choose its one-based **waveform index**, click **Replace waveform WAV**, and select a valid PCM RIFF WAV file. The manager copies the new audio to a separate `FMOD_Sources` authoring directory, rewrites only that waveform path to a relative reference, and imports a staged FDP copy through the normal Modding backup workflow. The waveform index is a **Designer project source ordering**, not the FSB bank sample index.
4. Click **Open FMOD Designer** on the editable FDP; configure the original project and use **Project → Build** to compile new FEV/FSB outputs. Do not rename original event identifiers, wave bank names, paths or event groups. Designer needs every referenced source sound; the uploaded FDPs still refer to source WAVs that are not supplied in the ZIP. Other missing sound sources must be supplied or reconstructed before a complete rebuild can succeed.
5. Return to the project, select an existing FSB or FEV member and click **Import rebuilt FSB/FEV**. The manager validates the file/header; FSB sample count and codec must match the original, and the new bank must keep the same original filename. The staged replacement is imported through the Modding backup workflow. Repeat for both FSB and FEV if both changed. Rebuild/sign the mod PAK using the existing manager workflow.

## Important restrictions

- The native bank reassembler handles existing **Vorbis FSB5** sample replacements through open-source encoders/remuxers. **FEV** event recompilation and **IMA ADPCM** bank compilation still require separate tooling. For advanced project authoring, FMOD Designer can still compile the FSB/FEV project externally. The manager handles project waveform substitution, decoding, inspection, and guarded import.
- Decoding a bank does not re-create a complete FMOD project or preserve missing WAV sources automatically; project source paths need to be satisfied for the authoring tool to rebuild a whole bank.
- FEV preflight validates only its RIFF/FEV container, not event GUID/ID compatibility. FSB preflight checks sample count and encoding, not the game's FMOD runtime version. Testing the result in an **offline** Evolve match is required.
- Only files already present in an editable project manifest can be safely staged as replacement members. New file insertion, signature verification and sound-event discovery are separate game compatibility issues.
- The original PAK and read-only Asset Library remain untouched. The edited project uses EditorBackups for source-file changes.

## Validation

Rust unit tests cover malformed FSB5 headers, FEV and FDP inspection, missing tool paths and rejected non-FSB audio. GitHub CI exercises the code and smoke-tests optional installer packaging. A passing CI build does not prove in-game sound playback, modified audio output, or Evolve runtime compatibility.
