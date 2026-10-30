# Model-viewer animation: verified scope and next binding step

The viewer's native `CrChF` CAF path can deform a matching compiled rig. Legacy
and Stage 2 clips with this header share that path; file headers, not build
labels, determine which decoder is safe. The FBX export dialog can opt in to a
selected CAF, and the rig exporter refuses incompatible controller IDs. No
animation is embedded by default.

The 2012 `CryTek` 0x745 / DBA 0x905 path is different. The separate GPLv2
converter can decode real tracks, but the Rust viewer currently reads the
converted GLB as a **track timeline only**. It does not yet bind its joints to
the selected model, deform the viewport, or export DBA Actions in FBX. A
playable clip list must not be mistaken for working on-model playback.

## Local 2012 acceptance probe (2026-10-29)

In an isolated copy of the user's extracted 2012 assets, pinned Markemp
Cryengine-Converter v2.0.0 converted
`Objects/characters/monsters/ParasiteHost/ParasiteHost3_Skeleton.chr` with its
original `.chrparams`, original body/CDF and declared
`Animations/monsters/parasitehost_3/ParasiteHost3.dba`. The resulting GLB
contained **one skin, 89 joints and 43 named animation clips**. This confirms
track decoding for this sample, **not** compatibility with the public viewer's
mesh. The copied game files, converted output and converter binary remain
outside Git; installed originals were not modified.

## Remaining acceptance gates

1. Generate the selected CDF's rigged GLB from the same extracted build. Match
   converted joints against it by unique name, parent and rest transform;
   reject missing, extra animated or ambiguous joints. For 2012, use the
   original master skeleton and its `.chrparams`, not the 77-bone body alone.
2. Sample a named, verified clip at two different timestamps and prove the
   displayed model's weighted vertices change while source UVs and topology
   remain unchanged. Test pause, scrub, loop and model switching. Never show
   track inspection as mesh playback.
3. Only after binding, append the selected clip(s) to the model's rigged GLB
   and confirm their Actions survive Blender's binary FBX export. Keep the
   animation option off by default; reject mismatched skeletons and names.
4. Repeat on owned Legacy and Stage 2 CAF samples, plus a 2012 ParasiteHost
   clip. Do not commit proprietary game assets as fixtures.
