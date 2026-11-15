# Repository instructions

Evolve Mod Manager is a Windows modding and asset-inspection app built with Rust,
Iced, and wgpu. The Cargo workspace lives in `crates/` (`evolve-core`,
`evolve-desktop`, `evolve-teleport-overlay`, `evolve-privileged-worker`).

## Repository topology

- Source of truth: the **private** `corvodl/EMM-SRC` repository. Branch from
  `main` and open pull requests against `EMM-SRC` only.
- Public artifact repository: `corvodl/Evolve-Mod-Manager`. Its `main` contains
  only README and LICENSE and is not a source mirror. Release objects carry the
  installer and portable ZIP.
- Never push source, branches, or pull requests to the public repository, and
  never create a public tag from a source commit. Never add the public
  repository as a checkout or clone in a workflow. Its only writes are the
  release objects and the two installer/ZIP assets created by the release
  workflows with the dedicated token described below.
- Do not merge, tag, publish, or release without explicit approval.

## Public release credentials

Both release workflows require the Actions secret `PUBLIC_RELEASE_TOKEN` in
`corvodl/EMM-SRC` and fail closed when it is absent. The source `GITHUB_TOKEN`
is read-only and must never be used to publish.

Set it up once:

1. GitHub → Settings → Developer settings → Personal access tokens →
   Fine-grained tokens → Generate new token.
2. Resource owner `corvodl`; Repository access: **Only select repositories** →
   `corvodl/Evolve-Mod-Manager`. Do not grant access to `EMM-SRC` or any other
   private repository.
3. Repository permissions: **Contents: Read and write**. Metadata: Read-only is
   added automatically.
4. Generate the token and copy it immediately. In `corvodl/EMM-SRC` →
   Settings → Secrets and variables → Actions → New repository secret, create
   `PUBLIC_RELEASE_TOKEN`.
5. To rotate, create the replacement token, update the secret, then revoke the
   old token. Never print, commit, or reuse the token elsewhere.

The public repository must stay public with `main` as its default branch.
Workflows reject a private target or a different default branch.

## Release rules

- `.github/workflows/nightly.yml` publishes Nightly prereleases from `main` via
  the manual trigger or the `.github/nightly-request.txt` request file.
- `.github/workflows/release.yml` publishes stable, beta, or rc builds after the
  `PARITY_VERIFIED_AND_TESTED` acknowledgment and every `docs/PARITY.md`
  checklist row passes.
- Keep the existing manual triggers, concurrency, version checks, and approval
  gates. Do not add schedules or ordinary-branch publishing.
- Release assets are exactly the compiled Windows installer and portable ZIP.
  Never upload README, LICENSE, manager source archives, symbols, game assets,
  keys, or credentials. The portable ZIP may include the GPLv2
  Cryengine-Converter and its corresponding source under `third_party/` for
  license compliance; that is expected, not manager source. GitHub's
  auto-generated source archives are the only other download links and contain
  only the public README and LICENSE.
- Both workflows extract the embedded manifest from
  `target/release/evolve-mod-manager.exe` with `mt.exe` before packaging and
  fail unless it declares `requireAdministrator`. Do not bypass that guard or
  build a release with `EVOLVE_TEST_NO_MANIFEST`.
- Publish with `repository: corvodl/Evolve-Mod-Manager`,
  `token: ${{ secrets.PUBLIC_RELEASE_TOKEN }}`, and
  `target_commitish: main` on `softprops/action-gh-release@v3`. Never target
  this private repository.

## Elevation and privileged actions

Read `docs/ON_DEMAND_ELEVATION.md` before touching UAC, protected paths,
firewall rules, installs, updates, mod staging, or signing.

- The GUI currently embeds `requireAdministrator` and must keep that manifest
  until the audited broker, authenticated worker channel, recovery path, and
  acceptance tests are complete. Do not change it to `asInvoker` first.
- Desktop tests use `scripts/test-desktop-standard-user.ps1`, which scopes
  `EVOLVE_TEST_NO_MANIFEST` to a separate target directory. Interactive
  layout/UI reviews may use a separate locally scoped `asInvoker` build. Neither
  is a release; never use that flag for release builds, and the packaging guard
  rejects an `asInvoker` binary.
- Once the broker lands, protected operations must go through a short-lived
  worker launched only after an explicit user action.
- The worker accepts a closed set of operations. Never add a generic command,
  script, or arbitrary file-copy operation.
- Validate game roots, paths, links/reparse points, phases, and hashes inside
  the worker before the first write. Do not trust caller-supplied paths or
  journals.
- A future broker must not automatically elevate a portable/user-writable copy;
  never launch an arbitrary sibling executable with the worker token.
- Change manifests only together with `tests/windows_admin_manifest.rs`, the
  manifest assertion in `.github/workflows/rust-ci.yml`, and the packaging
  guard. Never ship an `asInvoker` build as a release.

## Desktop UI

Read `.claude/skills/ui-behaviors/SKILL.md` before adding or changing a page,
dialog, popover, menu, list, or shortcut.

- Reuse `ui_kit.rs`, `design.rs`, `forms.rs`, `chrome.rs`, and `toast.rs`.
  Do not hand-roll a second button, dialog, popover, field, or toast.
- One Esc closes the topmost layer in the `Message::ViewportEscape` chain.
  Enter confirms the open modal when focus is not in a field. Tab stays inside
  the current window or dialog via `focus_scope`.
- Route global shortcuts through `keybinds.rs` and register new commands in the
  command palette.
- Name every control; use `ui_kit::why_disabled` for unavailable actions. Style
  from `design` tokens and widget state, check contrast in light and dark
  themes, and honour `preferences.reduce_motion`.
- Review at 1440×900 and the 880×620 minimum, in both themes, with the keyboard
  only.

## Verification

Run from the repository root:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

Additional checks:

- Workflow edits: validate YAML syntax with `actionlint` or `yamllint` when
  available, confirm no public checkout/push, confirm the `mt.exe`
  `requireAdministrator` guard still runs before packaging, and confirm the
  publish step uses `PUBLIC_RELEASE_TOKEN`, `target_commitish: main`, and
  `softprops/action-gh-release@v3`.
- UI changes: `cargo check -p evolve-mod-manager --tests`, then walk the change
  with Tab, Enter, and Esc. State what could not be checked interactively.
- Security-sensitive changes: add negative tests for foreign paths, unknown
  operations, forged hashes/phases, and replaced helpers before any privileged
  write. Do not disable source validation or lint to make a check pass.
- Do not claim a release is verified from synthetic tests alone; real-game
  acceptance is separate.

## Privacy

Keep proprietary game assets, extracted archives, signing keys and certificates,
DPAPI identity data, relay tokens, personal paths, and configuration out of the
repository and release assets. Stage only redistributable third-party binaries
with their licenses and corresponding source where required.
