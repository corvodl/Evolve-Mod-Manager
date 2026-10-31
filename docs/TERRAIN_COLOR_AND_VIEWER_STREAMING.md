# Terrain RGB24 orientation and Model Viewer upload scheduling

## Terrain color: verified inputs, not guessed recoloring

The source-backed world terrain mesh builds UV0 as `[world X, world Y]`.
The existing CRY sector decoder's `stamp()` explicitly transposes source
terrain pixels to reconcile Cry3DEngine's `U=world Y, V=world X` convention.
The separate imported `tileX_Y.raw` RGB24 decoder did **not** transpose either
its tiles or their interior pixels. It therefore painted spatially mismatched
regions on Tutorial's heightmap. Both atlas decoders now use the same
coordinate convention, with distinct synthetic-color tests for tile positions
and nonuniform tile interiors.

This is a **source-coordinate consistency fix**, not a verified solution for
every visibly wrong terrain color. The original RGB bytes are unchanged; the
atlas is only assembled differently in memory. Terrain gamma, authored
material-layer blending, runtime exposure/HDR, fog, time-of-day, and potential
missing source layers require matched Evolve captures before claiming parity.
The Level Editor's raw tile RGB sampling diagnostics remain available.

### Real-map checks

1. Open Tutorial and compare original atlas quadrant samples and the visible
   cliff/ground boundaries against identical camera coordinates before and
   after this change.
2. Compare the unmodified RGB24 tiles with an in-game Evolve capture at a
   matched TimeOfDay. If colors remain off, check engine layer contribution,
   sRGB/linear handling, lighting and terrain UVs separately instead of
   applying arbitrary color/tint multipliers.
3. Test an Ice_Crashsite 4×4 atlas and an original CRY-sector map to make
   sure only the RGB24 reconstruction changed.
4. Do not move, rewrite, or rename any proprietary game files or imported PAKs.

## Model Viewer: bounded GPU work

The Model Viewer previously uploaded all visible CDF material sections and
all seven optional texture roles in one graphics prepare pass, potentially
blocking the first viewport render for a large character. It now processes
one material section per GPU redraw; already uploaded textures and bind
groups are retained and reused by source Arc/role identity. A 65 ms GUI
streaming tick runs only while queued sections remain, and the viewer
displays the remaining count.

Ordinary camera orbit, zoom and material ticks no longer rewrite an
unchanged mesh's entire vertex stream to the GPU. A model change, GPU
buffer reallocation, or quality change still uploads/rebuilds the
necessary objects. Full quality retains the source texture dimensions
subject to device limits, without quietly substituting lower-quality
files; Balanced retains its existing reduced budget.

### Known limitations

- This is **per-material section streaming**, not yet independent per-mip
  streaming of each individual 4K/8K Model Viewer texture. A section
  containing very large maps can still take time to upload in one pass.
- CPU-side PAK discovery, DDS decoding and assembled CDF dependency resolution
  happen before this GPU staging and are not accelerated by this change.
- The renderer must keep the previously uploaded sections visible while new
  ones arrive; incomplete source material tables fail closed.
- Real Windows profiling is needed to separate load time spent in PAK
  discovery, decode, GPU upload, and UI redraw. Compare 2012 and Stage 2
  single-mesh and full-character assemblies, not only Rust CI.

## Acceptance

- Cargo formatting, strict Clippy, Linux/Windows workspace tests
- Tutorial / Ice_Crashsite baked color visual comparisons
- Model Viewer camera stays responsive while many parts upload, and
  source texture identities persist across draws
- No missing facial parts or crash when switching models, quality, and
  toggling wireframe during an unfinished upload
