# Model-viewer animation parity: verified and outstanding paths

## Reference behavior
The private Qt manager has a separate converted-GLB skinned-mesh preview, CAF playback
on source-authorized character rigs, explicit layered overlays, a neutral idle
selected **paused**, clip scrubbing/looping/speed controls and export of real
animations beside character GLBs. Its converted GLB animation preview uses
its OWN mesh, joints, inverse bind matrices and weighted vertices. It must not
be confused with binding a converted track to an unrelated CDF mesh.

## Implemented in PR #153
- Native `CrChF` CAF playback deforms exact matching compiled
  Stage 2/Legacy source rigs, with source-weighted CDF equipment parts.
- Neutral idle auto-discovery loads frame zero **paused**; manual Play is required.
  Native scrub and optional CAF layers remain active and source-verified.
- Measured playback elapsed time avoids large jumps on focus/resume.
- Converted GLB clips with **one internally consistent skinned mesh** now
  deform their own rig and vertex geometry at clip selection, scrub and Play.
  The read-only preview parses explicit weights, inverse binds, node parent
  hierarchy, glTF transforms and original triangle indices; each frame
  preserves UV0 and index topology and uses glTF Y-up -> viewer Z-up.
  Malformed rigs/missing or duplicate joints fail closed into track-only
  inspection instead of masquerading as moving source-model geometry.
- Conversion remains optional; no converter code or game assets are committed.
- Rigged FBX export can explicitly include a selected *verified native CAF*;
  mismatched native controllers are rejected by the exporter.

## Known gaps before 1:1 parity
1. Independently converted 2012 `CryTek` 0x745 / DBA 0x905 motion must be
   validated against the exact selected CDF/master skeleton (unique joint names,
   parent chains and rest transforms). The internal GLB's animation is NOT proof
   that its tracks can animate a separately loaded source mesh.
2. Real-map/model acceptance: 2012 ParasiteHost3 (89 converted joints, 43 clip
   names), 2012 Scorpid, Legacy, Stage 2 hunters and monsters. Compare two
   clip timestamps, Play/Pause, scrub, loop and actual weighted vertex changes
   while preserving source UVs, rig, materials and surface ordering.
3. Converted glTF layered/additive channels currently have track data but no
   verified layered viewport composition. Native CAF layers are implemented.
4. Attach exact matched converted clips to an independently verified source
   model/character GLB, then verify the Blender FBX Action round-trip and
   animation-folder export parity. Exported DBA clips remain unverified.
5. GPU joint-palette acceleration, material/submesh parity for converted GLBs,
   and UI integration of blended clip timelines need additional validation.

## Historical private reference
A 2012 ParasiteHost3 source copy with original CHR/chrparams/DBA converted into
one skin with 89 joints and 43 named clips using pinned GPLv2 converter. This
establishes converted-track availability, not public CDF identity or a verified
Blender FBX export. No original game assets are included in this repository.
