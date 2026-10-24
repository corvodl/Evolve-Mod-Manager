//! Guarded, offline preparation of the user's existing Stage 2 client hook.
//! Does NOT inject, start or modify the installed client. No DLLs or RSA keys
//! are embedded; all sources must be supplied by the user and validated.
use crate::signed_pak::{SignedPakError, inspect_signed_pak};
use rsa::{
    RsaPublicKey,
    pkcs1::{DecodeRsaPublicKey, EncodeRsaPublicKey},
    traits::PublicKeyParts,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;

const HOOK_SIZE: usize = 4096;
const KEY_OFFSET: usize = 0x870;
const PUBLIC_BYTES: usize = 140;

#[derive(Debug, thiserror::Error)]
pub enum ClientHookError {
    #[error("Unexpected hook size or public RSA-1024 PKCS#1 format")]
    Format,
    #[error("Original hook's embedded RSA key did not match the supplied original public key")]
    OriginalMismatch,
    #[error("Custom hook differs outside the permitted embedded RSA public-key bytes")]
    OtherChanges,
    #[error(
        "Rebuilt hook does not match the signing setup's verified SHA-256; the installed hook or signing baseline changed"
    )]
    BaselineMismatch,
    #[error("A hook output already exists or would overwrite a source")]
    UnsafeOutput,
    #[error("Signed PAK and matching client key validation failed: {0}")]
    Pak(#[from] SignedPakError),
    #[error("Client hook file I/O: {0}")]
    Io(#[from] io::Error),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HookPairReport {
    pub schema: u32,
    pub source_sha256: String,
    pub custom_sha256: String,
    pub original_public_sha256: String,
    pub custom_public_sha256: String,
    pub only_public_key_bytes_changed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PakHookPairReport {
    pub schema: u32,
    pub archive_basename: String,
    pub original_pak_sha256: String,
    pub custom_pak_sha256: String,
    pub signed_original_entries: usize,
    pub signed_replacement_entries: usize,
    pub archive_member_paths_match: bool,
    pub hook: HookPairReport,
    pub runtime_mount_verified: bool,
}

fn canonical_public(path: &Path) -> Result<Vec<u8>, ClientHookError> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() || meta.len() != PUBLIC_BYTES as u64 {
        return Err(ClientHookError::Format);
    }
    let bytes = fs::read(path)?;
    let key = RsaPublicKey::from_pkcs1_der(&bytes).map_err(|_| ClientHookError::Format)?;
    if key.size() != 128
        || key.e() != &rsa::BigUint::from(65537u32)
        || key
            .to_pkcs1_der()
            .map_err(|_| ClientHookError::Format)?
            .as_bytes()
            != bytes
    {
        return Err(ClientHookError::Format);
    }
    Ok(bytes)
}
fn hook_bytes(path: &Path) -> Result<Vec<u8>, ClientHookError> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() || meta.len() != HOOK_SIZE as u64 {
        return Err(ClientHookError::Format);
    }
    Ok(fs::read(path)?)
}
fn archive_fingerprint(path: &Path) -> Result<String, ClientHookError> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() || meta.len() > 16 * 1024 * 1024 * 1024 {
        return Err(ClientHookError::Format);
    }
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut chunk = [0u8; 128 * 1024];
    let mut total = 0u64;
    loop {
        let n = file.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        total = total.checked_add(n as u64).ok_or(ClientHookError::Format)?;
        if total > meta.len() {
            return Err(ClientHookError::Format);
        }
        hasher.update(&chunk[..n]);
    }
    if total != meta.len() {
        return Err(ClientHookError::Format);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
fn fingerprint(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn compare(
    original: &[u8],
    custom: &[u8],
    old: &[u8],
    new: &[u8],
) -> Result<HookPairReport, ClientHookError> {
    if original.len() != HOOK_SIZE
        || custom.len() != HOOK_SIZE
        || old.len() != PUBLIC_BYTES
        || new.len() != PUBLIC_BYTES
        || old == new
    {
        return Err(ClientHookError::Format);
    }
    if original[KEY_OFFSET..KEY_OFFSET + PUBLIC_BYTES] != *old {
        return Err(ClientHookError::OriginalMismatch);
    }
    if custom[KEY_OFFSET..KEY_OFFSET + PUBLIC_BYTES] != *new
        || original[..KEY_OFFSET] != custom[..KEY_OFFSET]
        || original[KEY_OFFSET + PUBLIC_BYTES..] != custom[KEY_OFFSET + PUBLIC_BYTES..]
    {
        return Err(ClientHookError::OtherChanges);
    }
    Ok(HookPairReport {
        schema: 1,
        source_sha256: fingerprint(original),
        custom_sha256: fingerprint(custom),
        original_public_sha256: fingerprint(old),
        custom_public_sha256: fingerprint(new),
        only_public_key_bytes_changed: true,
    })
}

/// Prepare a NEW locally staged copy of a verified original inject.dll.
/// The game file remains untouched. Requires a known original key at 0x870;
/// guessing the hook location or swapping an arbitrary DLL is prohibited.
pub fn prepare_client_hook(
    original_hook: &Path,
    original_public: &Path,
    custom_public: &Path,
    destination: &Path,
) -> Result<HookPairReport, ClientHookError> {
    let old = canonical_public(original_public)?;
    let new = canonical_public(custom_public)?;
    if old == new {
        return Err(ClientHookError::Format);
    }
    let original = hook_bytes(original_hook)?;
    if original[KEY_OFFSET..KEY_OFFSET + PUBLIC_BYTES] != *old {
        return Err(ClientHookError::OriginalMismatch);
    }
    let parent = destination.parent().ok_or(ClientHookError::UnsafeOutput)?;
    // The offline launcher consumes stage/inject-custom.dll; export tools
    // also support a separately staged copy named inject.dll. Both require
    // a NEW path and an exact unchanged original hook except its RSA region.
    let accepted = matches!(
        destination.file_name().and_then(|n| n.to_str()),
        Some("inject.dll" | "inject-custom.dll")
    );
    if !accepted
        || !fs::symlink_metadata(parent)?.file_type().is_dir()
        || fs::symlink_metadata(destination).is_ok()
    {
        return Err(ClientHookError::UnsafeOutput);
    }
    let mut patched = original.clone();
    patched[KEY_OFFSET..KEY_OFFSET + PUBLIC_BYTES].copy_from_slice(&new);
    let report = compare(&original, &patched, &old, &new)?;
    let mut staged = tempfile::Builder::new()
        .prefix(".evolve-hook-")
        .tempfile_in(parent)?;
    staged.write_all(&patched)?;
    staged.as_file_mut().sync_all()?;
    // Re-read the original before publication. A simultaneous modification
    // invalidates the prepared candidate instead of distributing an outdated hook.
    if hook_bytes(original_hook)? != original {
        return Err(ClientHookError::OtherChanges);
    }
    staged.persist_noclobber(destination).map_err(|e| {
        if e.error.kind() == io::ErrorKind::AlreadyExists {
            ClientHookError::UnsafeOutput
        } else {
            ClientHookError::Io(e.error)
        }
    })?;
    Ok(report)
}

/// Recover only a corrupt staged hook, using the SHA-256 recorded when the
/// signing stage was first generated. The installed game's hook is read-only.
/// A changed original can never silently become the new signing baseline.
pub fn repair_client_hook_from_verified_baseline(
    original_hook: &Path,
    staged_hook: &Path,
    original_public: &Path,
    custom_public: &Path,
    expected_sha256: &str,
) -> Result<bool, ClientHookError> {
    if expected_sha256.len() != 64
        || !expected_sha256.bytes().all(|b| b.is_ascii_hexdigit())
        || staged_hook.file_name().and_then(|name| name.to_str()) != Some("inject-custom.dll")
        || staged_hook == original_hook
    {
        return Err(ClientHookError::Format);
    }
    let source_before = hook_bytes(original_hook)?;
    let damaged = hook_bytes(staged_hook)?;
    if fingerprint(&damaged).eq_ignore_ascii_case(expected_sha256) {
        // inspect_client_hook_pair still checks the live original separately.
        return Ok(false);
    }
    let parent = staged_hook.parent().ok_or(ClientHookError::UnsafeOutput)?;
    let candidate_dir = tempfile::Builder::new()
        .prefix(".evolve-hook-repair-")
        .tempdir_in(parent)?;
    let candidate = candidate_dir.path().join("inject-custom.dll");
    prepare_client_hook(original_hook, original_public, custom_public, &candidate)?;
    if !fingerprint(&hook_bytes(&candidate)?).eq_ignore_ascii_case(expected_sha256) {
        return Err(ClientHookError::BaselineMismatch);
    }
    // If either input changed during preparation, do not replace any file.
    if hook_bytes(original_hook)? != source_before || hook_bytes(staged_hook)? != damaged {
        return Err(ClientHookError::OtherChanges);
    }
    fs::rename(&candidate, staged_hook)?;
    if !fingerprint(&hook_bytes(staged_hook)?).eq_ignore_ascii_case(expected_sha256) {
        return Err(ClientHookError::BaselineMismatch);
    }
    Ok(true)
}

/// Confirm an existing custom hook has no changes beyond the exact public
/// key bytes. This check alone cannot prove the hook is loaded by Evolve.
pub fn inspect_client_hook_pair(
    original_hook: &Path,
    custom_hook: &Path,
    original_public: &Path,
    custom_public: &Path,
) -> Result<HookPairReport, ClientHookError> {
    let old = canonical_public(original_public)?;
    let new = canonical_public(custom_public)?;
    compare(
        &hook_bytes(original_hook)?,
        &hook_bytes(custom_hook)?,
        &old,
        &new,
    )
}

/// A required *preflight* before any future controlled game PAK swapping.
/// It verifies both signatures, identical member paths, and that the custom
/// shim and replacement archive share one exact user-provided RSA public key.
/// It never claims that the actual game process mounted the replacement.
pub fn inspect_pak_hook_pair(
    original_pak: &Path,
    custom_pak: &Path,
    original_hook: &Path,
    custom_hook: &Path,
    original_public: &Path,
    custom_public: &Path,
    basename: &str,
) -> Result<PakHookPairReport, ClientHookError> {
    inspect_pak_hook_pair_with_original_name(
        original_pak,
        custom_pak,
        original_hook,
        custom_hook,
        original_public,
        custom_public,
        (basename, basename),
    )
}

/// Verify each CryPAK against the exact basename it was signed with.
/// Retail source signatures may use ASCII lowercase while their installed
/// file names use different case. Custom staged copies preserve the
/// authenticated signing name, regardless of the unchanged on-disk basename.
pub fn inspect_pak_hook_pair_with_original_name(
    original_pak: &Path,
    custom_pak: &Path,
    original_hook: &Path,
    custom_hook: &Path,
    original_public: &Path,
    custom_public: &Path,
    signed_names: (&str, &str),
) -> Result<PakHookPairReport, ClientHookError> {
    let (original_signed_name, basename) = signed_names;
    if original_signed_name != basename && original_signed_name != basename.to_ascii_lowercase() {
        return Err(ClientHookError::Format);
    }
    let hook =
        inspect_client_hook_pair(original_hook, custom_hook, original_public, custom_public)?;
    let old = inspect_signed_pak(original_pak, original_public, original_signed_name)?;
    let new = inspect_signed_pak(custom_pak, custom_public, original_signed_name)?;
    let original_paths: std::collections::HashSet<_> = old
        .entries
        .iter()
        .map(|e| e.path.to_ascii_lowercase())
        .collect();
    let replacement_paths: std::collections::HashSet<_> = new
        .entries
        .iter()
        .map(|e| e.path.to_ascii_lowercase())
        .collect();
    if original_paths != replacement_paths || old.entries.len() != new.entries.len() {
        return Err(ClientHookError::OtherChanges);
    }
    Ok(PakHookPairReport {
        schema: 1,
        archive_basename: basename.to_owned(),
        original_pak_sha256: archive_fingerprint(original_pak)?,
        custom_pak_sha256: archive_fingerprint(custom_pak)?,
        signed_original_entries: old.entries.len(),
        signed_replacement_entries: new.entries.len(),
        archive_member_paths_match: true,
        hook,
        runtime_mount_verified: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::{RsaPrivateKey, pkcs1::EncodeRsaPublicKey};
    fn setup() -> (
        tempfile::TempDir,
        std::path::PathBuf,
        std::path::PathBuf,
        std::path::PathBuf,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = rand::rngs::OsRng;
        let a = RsaPrivateKey::new(&mut rng, 1024).unwrap();
        let b = RsaPrivateKey::new(&mut rng, 1024).unwrap();
        let old = dir.path().join("old.der");
        let new = dir.path().join("new.der");
        fs::write(
            &old,
            RsaPublicKey::from(&a).to_pkcs1_der().unwrap().as_bytes(),
        )
        .unwrap();
        fs::write(
            &new,
            RsaPublicKey::from(&b).to_pkcs1_der().unwrap().as_bytes(),
        )
        .unwrap();
        let shim = dir.path().join("original.dll");
        let mut bytes = vec![0x47u8; HOOK_SIZE];
        bytes[KEY_OFFSET..KEY_OFFSET + PUBLIC_BYTES].copy_from_slice(&fs::read(&old).unwrap());
        fs::write(&shim, bytes).unwrap();
        (dir, shim, old, new)
    }
    #[test]
    fn private_launcher_stage_hook_name_is_supported_without_overwriting_original() {
        let (dir, original, old, new) = setup();
        let custom = dir.path().join("inject-custom.dll");
        let original_bytes = fs::read(&original).unwrap();
        let verified = prepare_client_hook(&original, &old, &new, &custom).unwrap();
        assert!(verified.only_public_key_bytes_changed);
        assert!(inspect_client_hook_pair(&original, &custom, &old, &new).is_ok());
        assert_eq!(fs::read(&original).unwrap(), original_bytes);
        assert!(prepare_client_hook(&original, &old, &new, &custom).is_err());
        let wrong = dir.path().join("inject-custom.dll.bak");
        assert!(prepare_client_hook(&original, &old, &new, &wrong).is_err());
    }

    #[test]
    fn creates_only_an_exact_custom_key_hook_without_touching_original() {
        let (dir, original, old, new) = setup();
        let output_dir = dir.path().join("staged");
        fs::create_dir(&output_dir).unwrap();
        let custom = output_dir.join("inject.dll");
        let before = fs::read(&original).unwrap();
        let report = prepare_client_hook(&original, &old, &new, &custom).unwrap();
        assert!(report.only_public_key_bytes_changed);
        assert_eq!(fs::read(&original).unwrap(), before);
        let check = inspect_client_hook_pair(&original, &custom, &old, &new).unwrap();
        assert_eq!(check.custom_sha256, report.custom_sha256);
        assert!(matches!(
            prepare_client_hook(&original, &old, &new, &custom),
            Err(ClientHookError::UnsafeOutput)
        ));
        assert_eq!(fs::read(&original).unwrap(), before);
    }
    #[test]
    fn repairs_only_staged_hook_bytes_matching_the_saved_setup_digest() {
        let (dir, original, old, new) = setup();
        let custom = dir.path().join("inject-custom.dll");
        prepare_client_hook(&original, &old, &new, &custom).unwrap();
        let source_before = fs::read(&original).unwrap();
        let expected = fingerprint(&fs::read(&custom).unwrap());
        let mut corrupted = fs::read(&custom).unwrap();
        corrupted[4] ^= 1;
        fs::write(&custom, &corrupted).unwrap();
        assert!(
            repair_client_hook_from_verified_baseline(&original, &custom, &old, &new, &expected)
                .unwrap()
        );
        assert_eq!(fingerprint(&fs::read(&custom).unwrap()), expected);
        assert_eq!(fs::read(&original).unwrap(), source_before);
        assert!(
            !repair_client_hook_from_verified_baseline(&original, &custom, &old, &new, &expected)
                .unwrap()
        );
        assert!(inspect_client_hook_pair(&original, &custom, &old, &new).is_ok());
    }

    #[test]
    fn never_repairs_from_an_original_hook_changed_after_setup() {
        let (dir, original, old, new) = setup();
        let custom = dir.path().join("inject-custom.dll");
        prepare_client_hook(&original, &old, &new, &custom).unwrap();
        let expected = fingerprint(&fs::read(&custom).unwrap());
        let mut changed_source = fs::read(&original).unwrap();
        changed_source[1] ^= 1;
        fs::write(&original, changed_source).unwrap();
        let mut corrupted = fs::read(&custom).unwrap();
        corrupted[4] ^= 1;
        fs::write(&custom, &corrupted).unwrap();
        assert!(matches!(
            repair_client_hook_from_verified_baseline(&original, &custom, &old, &new, &expected),
            Err(ClientHookError::BaselineMismatch)
        ));
        assert_eq!(fs::read(&custom).unwrap(), corrupted);
        // A valid staged SHA alone is never proof the installed original is safe.
        let mut valid = corrupted;
        valid[4] ^= 1;
        fs::write(&custom, valid).unwrap();
        assert!(
            !repair_client_hook_from_verified_baseline(&original, &custom, &old, &new, &expected)
                .unwrap()
        );
        assert!(inspect_client_hook_pair(&original, &custom, &old, &new).is_err());
    }

    #[test]
    fn rejects_wrong_original_key_modified_hook_and_unsafe_paths() {
        let (dir, original, old, new) = setup();
        let output_dir = dir.path().join("staged");
        fs::create_dir(&output_dir).unwrap();
        let custom = output_dir.join("inject.dll");
        assert!(matches!(
            prepare_client_hook(&original, &new, &old, &custom),
            Err(ClientHookError::OriginalMismatch)
        ));
        prepare_client_hook(&original, &old, &new, &custom).unwrap();
        let mut data = fs::read(&custom).unwrap();
        data[15] ^= 1;
        fs::write(&custom, data).unwrap();
        assert!(matches!(
            inspect_client_hook_pair(&original, &custom, &old, &new),
            Err(ClientHookError::OtherChanges)
        ));
        assert!(prepare_client_hook(&original, &old, &new, &original).is_err());
        assert!(
            inspect_client_hook_pair(&original, &original, &old, &old).is_err(),
            "an unmodified hook with the original key cannot masquerade as a custom-key swap"
        );
    }
}
