# Tutorial diffuse-material and terrain-color investigation

## Findings from current merged code

**DDS provenance (confirmed bug):** The source-verified imported resolver,
`resolve_verified_duplicate_textures`, already collapses byte-identical copies
of the same cooked `.dds` or `.dds.0` virtual member **inside a selected
build**. It previously excluded all numbered higher DDS fragments
(`.dds.1`–`.dds.19`) and historical alpha sidecars (`.dds.0a`, etc.).
The material loader's `decoded_dds_indexed` explicitly resolves each such
fragment. When a fragment exists in more than one same-game PAK, it could
therefore reject an identical verified copy as ambiguous and leave the
material's original texture maps unresolved. The observed Tutorial
`carcass_large_internals_ddna.dds.1` is a normal-map fragment; resolving it
may restore shading, but it does not by itself repair a missing diffuse.
This PR extends the *same* strict path, build,
manifest and SHA-256 matching to those fragment suffixes; different bytes,
different virtual paths, unsupported suffixes, and unselected builds still
fail closed. No heuristic nearest-name lookup is used.

**Missing source data (unverified):** The Tutorial Activity snapshot reported
29 model/material pairs without verified diffuse and 9 unresolved geometry
dependencies. These are *not* necessarily all the same fault. GPU residency,
frustum culling and mip uploads cannot create a DDS or CGF that is not in the
selected imported Game installation. Some material failures may come from an
unavailable MTL, a compiled mesh material-ID mismatch, missing diffuse
reference, missing DDS parts, or unsupported DDS decode.

Before this PR, the per-pair `level_missing_verified_diffuse` crash record
reported only the object, material and source archive. It now includes the
best retained original resolver/decoder diagnostic where available:
`MTL lookup`, `Missing exact cooked DDS`, `Ambiguous imported DDS fragment`,
or material section mismatch. Mixed-quality models where *some* visible
material sections already have a diffuse while others do not now also
emit `level_partially_missing_diffuse` with the number of absent sections.
This permits an evidence-based fix of each remaining unresolved pair
using the actual original game files.

## Terrain color is a distinct issue

PR #163 transposed the imported RGB24 tile atlas to follow the existing
CRY terrain coordinate mapping. Its tests use synthesized tile layouts;
they do **not** validate against a matched native Tutorial capture.
The decoded original Tutorial terrain source in the Activity log is a
2×2 RGB24 grid (2048×2048 output), and its reported average original
RGB is approximately (88,96,97). The actual GPU output additionally uses:
- Source sRGB sampling and independently computed terrain normals
- An approximate TimeOfDay sun/ambient model in `level_lighting.rs`
- Non-native distance/height fog in `world_instances.wgsl`
- A baked color atlas without full original terrain paint layers,
  material-specific reflectance, native engine HDR/exposure or shader effects

A visually gray/brown ground is therefore **not enough to prove RGB24
bytes were loaded incorrectly**. Compare the original tile bytes and
atlas diagnostic corner samples, an unlit source-image view, and a
matched Evolve scene (same build, location, time of day, and weather)
before applying any color multiplier, gamma change, or guessed texture
substitution. This PR deliberately does not modify the original source
RGB or change the shader's terrain color balance.

## Acceptance checks

1. Import *all* Tutorial and corresponding texture PAKs from the **same
   actual Game directory**. Open Tutorial and save the Activity and
   `rust-preview`/crash diagnostics.
2. Check whether `.dds.1` and later fragments formerly reported ambiguous
   are now resolved **only when byte-identical**. Confirm conflicting
   duplicates still report ambiguity rather than selecting by file order.
3. Count `level_missing_verified_diffuse` records and inspect each new
   `reason=` field. Do not assume all 29 cases are repaired by this PR.
4. Compare terrain atlas corner and average RGB logs to original imported
   tile bytes, then inspect the actual ground after the PR #163 orientation
   fix. Check source texture alignment separately from lighting/fog.
5. Check Stage 2 and Legacy models with legitimate split DDS alpha channels.
   Do not mix archives from different builds to fill missing mips.
6. Run the Windows workspace unit/Clippy CI. A green build is *not*
   proof of 1:1 terrain texture or fog parity.

Only the new per-fragment duplicate resolution and diagnostics are implemented
here. A separate Evolve-specific native lighting/terrain validation pass
is required for correct terrain color.
