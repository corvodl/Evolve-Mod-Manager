# Native kit-video compositor: offline foundation only

The Stage 2 Direct Launch broker already loads `Stage2Trace.dll` into a
suspended `Evolve.exe`. This branch adds an **off-by-default** second load of
`LegacyKitCompositor.dll` from `<prepared stage>/native/`, gated by the exact
SHA-256 passed as `--compositor-sha256`. It is restricted to the modded Direct
Launch worker; clean launches and the normal client launcher cannot opt in.
Validation occurs before services/swaps and again before injection. An injection
failure terminates the still-suspended child. Existing restore/watchdog logic
continues to own game-file restoration. No installed PAK is changed by this
probe. The probe is not bundled or automatically installed.

`crates/evolve-kit-compositor` supplies an x64 `cdylib` ABI marker and tested
smoothstep/linear-light blend reference. Cargo names the output
`evolve_kit_compositor.dll`; a future, explicitly authorized offline test
would copy/rename that output to `<prepared stage>/native/LegacyKitCompositor.dll`,
compute its digest, then opt in via the Direct Launch worker argument. Merely
loading that DLL does **not** exercise the exported marker or prove a surface
exists. Do not load the present prototype expecting any visual change.

## Graphics discovery gate before a real compositor

1. Record the game's actual graphics API and swap-chain format from an
   offline, consented test session. D3D11 must **not** be assumed from DLL
   presence alone. Avoid `DllMain` hooks, device creation, or readback on a
   render thread without a bounded teardown/failure strategy.
2. Identify a scene-color surface **before** GFx/UI composition and a decoded
   kit-video surface (or shader-readable copy) for the *same frame*. Verify
   each independently across two character selections and a return from
   Perks/AP. Capture only metadata such as dimensions, format, timing and
   ownership first; do not dump frames or game memory by default.
3. Verify compositing order against character, background, ability text and
   video rectangle. If the scene is only available before the character's
   stage pass, or the kit surface is inaccessible, stop rather than cover the
   UI or claim a seamless blend.
4. Only after both inputs are proven: render an opt-in fullscreen/pass-local
   blend in linear light using `video_coverage`, preserve alpha and color-space
   semantics, and restore original render state. Provide an immediate bypass
   for device loss, resize, video end and hook failure.

This branch does **not** implement steps 1–4. A successful DLL load is not
evidence of an in-game blend. Do not sign, inspect the installed PAK, or run
an injection while `Evolve.exe` is active.
