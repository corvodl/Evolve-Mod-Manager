//! Optional local integration for the developer-supplied Stage 2 encoder.
//! No developer executable, DLL, signing key or key source is shipped in Rust.
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const CRYPTO_SHA: &str = "841105bb6703206cae4d5c73d8d0ced5c50240d9595090a2a09dc2963fd22f78";
const EXE_SHA: &str = "c37e21134d7442c3fa195f9b09d8b951af0ee03b2c7617fe86ba5a7e7cbd4b9e";
const INJECT_SHA: &str = "62da468d2a8d98edfc5f9e9e1f6d7f74ca2a9d00712bfa8f163d674ec15ded91";
const PUBLIC_SHA: &str = "cec8d89e39db3f10d5f6cff984f713fe59cf84914f2df1294a27daeb21c42c18";
const DBGHELP_SHA: &str = "937f09c6eba49294f2d5495e7b4065aeae66047d65e7cc92bab92c8a702f74aa";

#[derive(Clone, Debug)]
pub struct OfficialTools {
    pub exe: PathBuf,
    pub public_key: PathBuf,
    pub inject: PathBuf,
    pub dbghelp: PathBuf,
}

fn sha(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("Cannot inspect official tool: {e}"))?;
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

fn checked(path: &Path, expected: &str) -> Result<PathBuf, String> {
    let meta = fs::symlink_metadata(path)
        .map_err(|_| format!("Official tool component missing: {}", path.display()))?;
    if !meta.file_type().is_file() || sha(path)? != expected {
        return Err(format!(
            "Official tool component differs from supplied 2025 release: {}",
            path.display()
        ));
    }
    fs::canonicalize(path).map_err(|e| e.to_string())
}

/// Optional local integration. Place files under storage/official-stage2-tools/
/// or explicitly set EVOLVE_STAGE2_TOOLS_DIR; never include them in releases.
pub fn discover(storage: &Path) -> Result<Option<OfficialTools>, String> {
    let selected = std::env::var_os("EVOLVE_STAGE2_TOOLS_DIR").map(PathBuf::from);
    let base = selected
        .clone()
        .unwrap_or_else(|| storage.join("official-stage2-tools"));
    if selected.is_none() && !base.exists() {
        return Ok(None);
    }
    let folder = if base.join("evolve_tool.exe").is_file() {
        base
    } else {
        base.join("evolve_tool_stage2")
    };
    if !fs::symlink_metadata(&folder).is_ok_and(|m| m.file_type().is_dir()) {
        return Err("Official Stage 2 tool folder missing or linked".into());
    }
    let exe = checked(&folder.join("evolve_tool.exe"), EXE_SHA)?;
    // The supplied executable requires crypto.dll beside it.
    let _crypto = checked(&folder.join("crypto.dll"), CRYPTO_SHA)?;
    let inject = checked(&folder.join("inject/inject.dll"), INJECT_SHA)?;
    let dbghelp = checked(&folder.join("inject/dbghelp.dll"), DBGHELP_SHA)?;
    let public_key = checked(&folder.join("inject/RSAKeyData.bin"), PUBLIC_SHA)?;
    let key = fs::read(&public_key).map_err(|e| e.to_string())?;
    let shim = fs::read(&inject).map_err(|e| e.to_string())?;
    if key.len() != 140 || shim.len() != 4096 || shim[0x870..0x870 + 140] != key {
        return Err("Official tool's public key does not match its inject.dll".into());
    }
    Ok(Some(OfficialTools {
        exe,
        public_key,
        inject,
        dbghelp,
    }))
}

/// Official stages require their original local tool installation; never
/// silently fall back to a different keypair.
pub fn for_stage(stage: &Path) -> Result<Option<OfficialTools>, String> {
    let status = stage.join("stage_status.json");
    let doc: serde_json::Value =
        serde_json::from_slice(&fs::read(&status).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    if doc["signer"].as_str() != Some("official-2025") {
        return Ok(None);
    }
    let storage = stage
        .parent()
        .and_then(Path::parent)
        .ok_or("Official signing stage has no storage root")?;
    let tools = discover(storage)?.ok_or(
        "Official Stage 2 tools missing. Restore the local tools folder or set EVOLVE_STAGE2_TOOLS_DIR."
    )?;
    if !tools.matches_stage(stage)? {
        return Err("Official signing stage and developer RSA verification key disagree".into());
    }
    Ok(Some(tools))
}

impl OfficialTools {
    pub fn matches_stage(&self, stage: &Path) -> Result<bool, String> {
        let expected = fs::read(&self.public_key).map_err(|e| e.to_string())?;
        let actual = fs::read(stage.join("mykeys/public_key.bin")).map_err(|e| e.to_string())?;
        Ok(actual == expected)
    }

    fn invoke(&self, action: &str, a: &Path, b: Option<&Path>) -> Result<(), String> {
        #[cfg(not(windows))]
        {
            let _ = (action, a, b);
            return Err("The developer encoder only runs on Windows".into());
        }
        #[cfg(windows)]
        {
            // This old program sometimes returns success even after an operation
            // fails; callers MUST verify the resulting signed archive themselves.
            use std::os::windows::process::CommandExt;
            let mut command = Command::new(&self.exe);
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
            command.arg(action).arg(a);
            if let Some(b) = b {
                command.arg(b);
            }
            let output = command
                .output()
                .map_err(|e| format!("Official Stage 2 encoder failed to start: {e}"))?;
            if !output.status.success() {
                return Err(format!(
                    "Official Stage 2 operation {action} failed. Original files were not replaced."
                ));
            }
            Ok(())
        }
    }

    /// Run the developer's one-time archive key migration against ISOLATED copies.
    /// The caller must independently authenticate each converted PAK afterwards.
    pub fn rekey_stage(&self, staged_paks: &Path) -> Result<(), String> {
        self.invoke("r", staged_paks, None)
    }

    /// Re-sign ONE already verified stock PAK for an isolated editing stage.
    /// Never modify the original source or invoke 'r' against a live game.
    pub fn rekey_copy(
        &self,
        source: &Path,
        signed_name: &str,
        output: &Path,
    ) -> Result<(), String> {
        use evolve_core::signed_pak::inspect_signed_pak;
        if output.exists() {
            return Err("Official target already exists".into());
        }
        let parent = output
            .parent()
            .ok_or("Official target needs parent directory")?;
        let scratch = tempfile::Builder::new()
            .prefix(".official-rekey-")
            .tempdir_in(parent)
            .map_err(|e| e.to_string())?;
        let name = source.file_name().ok_or("Source PAK has no name")?;
        let staged = scratch.path().join(name);
        fs::copy(source, &staged).map_err(|e| e.to_string())?;
        if inspect_signed_pak(&staged, &self.public_key, signed_name).is_err() {
            self.rekey_stage(scratch.path())?;
        }
        inspect_signed_pak(&staged, &self.public_key, signed_name)
            .map_err(|e| format!("Official single-PAK conversion did not authenticate: {e}"))?;
        fs::rename(staged, output).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Rebuild just one edited PAK with the original developer encoder.
    /// The encoder takes a full directory, so decode + overlay is required.
    /// Output is only published after signature/member verification.
    pub fn build_changed_pak(
        &self,
        source: &Path,
        signed_name: &str,
        replacements: &[(String, PathBuf)],
        output: &Path,
    ) -> Result<(), String> {
        use evolve_core::signed_pak::{inspect_signed_pak, open_verified_signed_pak};
        if replacements.is_empty() {
            return Err("No changed files to encode".into());
        }
        if output.exists() {
            return Err("Output PAK already exists".into());
        }
        let public = &self.public_key;
        let source_index = open_verified_signed_pak(source, public, signed_name).map_err(|e| {
            format!("Official build source is not signed with the official key: {e}")
        })?;
        let entries = &source_index.info.entries;
        if entries.is_empty() || entries.len() > 65_535 {
            return Err("Official encoder requires 1–65535 verified archive members".into());
        }
        let mut originals = HashMap::new();
        for entry in entries {
            if !safe_member(&entry.path)
                || originals
                    .insert(entry.path.clone(), (entry.crc32, entry.size))
                    .is_some()
            {
                return Err("Unsafe or duplicate source PAK member".into());
            }
        }
        let mut changed = HashSet::new();
        for (member, file) in replacements {
            if !safe_member(member)
                || !changed.insert(member.clone())
                || !originals.contains_key(member)
            {
                return Err("Changed member is unsafe, duplicate or absent from source PAK".into());
            }
            if !fs::symlink_metadata(file).is_ok_and(|m| m.file_type().is_file()) {
                return Err("Replacement is not a regular file".into());
            }
        }
        let parent = output.parent().ok_or("PAK output directory missing")?;
        let scratch = tempfile::Builder::new()
            .prefix(".official-stage2-")
            .tempdir_in(parent)
            .map_err(|e| e.to_string())?;
        let unpack = scratch.path().join("unpacked");
        fs::create_dir(&unpack).map_err(|e| e.to_string())?;
        self.invoke("d", source, Some(&unpack))?;

        // Source directory traversal was bounded and authenticated by Rust.
        // Reject unexpected files from the legacy decoder before overlaying.
        let decoded = walk_files(&unpack)?;
        if decoded.len() != originals.len()
            || decoded.iter().any(|name| !originals.contains_key(name))
        {
            return Err("Official decoder member inventory differs from signed original".into());
        }
        for (member, source_file) in replacements {
            let dest = unpack.join(Path::new(member));
            if !dest.is_file()
                || fs::symlink_metadata(&dest).is_ok_and(|m| !m.file_type().is_file())
            {
                return Err(format!("Decoded target member missing: {member}"));
            }
            fs::copy(source_file, &dest)
                .map_err(|e| format!("Cannot overlay changed member {member}: {e}"))?;
        }
        let signed = scratch.path().join(signed_name);
        self.invoke("e", &unpack, Some(&signed))?;
        let result = inspect_signed_pak(&signed, public, signed_name)
            .map_err(|e| format!("Official encoder output failed RSA verification: {e}"))?;
        if result.entries.len() != entries.len() {
            return Err("Official encoder lost original PAK members".into());
        }
        let rebuilt: HashMap<_, _> = result
            .entries
            .iter()
            .map(|entry| (entry.path.as_str(), (entry.crc32, entry.size)))
            .collect();
        for (name, metadata) in &originals {
            let updated = rebuilt
                .get(name.as_str())
                .ok_or_else(|| format!("Official PAK missing original member: {name}"))?;
            if !changed.contains(name) && metadata != updated {
                return Err(format!("Official encoder changed untouched member: {name}"));
            }
        }
        // Confirm the signed directory describes the exact replacement bytes,
        // not merely a PAK that passes its outer RSA signature.
        let rebuilt_index = open_verified_signed_pak(&signed, public, signed_name)
            .map_err(|e| format!("Cannot independently open signed official output: {e}"))?;
        for (member, edited_path) in replacements {
            let entry = rebuilt_index
                .info
                .entries
                .iter()
                .find(|entry| entry.path == *member)
                .ok_or("Official encoder omitted a replaced member")?;
            let (size, crc) = file_crc(edited_path)?;
            if entry.size != size || entry.crc32 != crc {
                return Err(format!(
                    "Official encoder payload CRC/size mismatch for {member}"
                ));
            }
            if size <= 64 * 1024 * 1024 {
                let decrypted = rebuilt_index
                    .read_member(&signed, member, size)
                    .map_err(|e| format!("Official edited member failed decryption: {e}"))?;
                if decrypted != fs::read(edited_path).map_err(|e| e.to_string())? {
                    return Err(format!("Official edited member content differs: {member}"));
                }
            }
        }
        fs::rename(&signed, output)
            .map_err(|e| format!("Cannot publish verified official PAK: {e}"))?;
        Ok(())
    }
}

fn file_crc(path: &Path) -> Result<(u64, u32), String> {
    use std::io::Read;
    let mut input = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = crc32fast::Hasher::new();
    let mut length = 0u64;
    let mut buffer = [0u8; 256 * 1024];
    loop {
        let count = input.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        length = length
            .checked_add(count as u64)
            .ok_or("Member size overflow")?;
        hash.update(&buffer[..count]);
    }
    Ok((length, hash.finalize()))
}

fn safe_member(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 1024
        && !name.contains('\\')
        && !name.contains(':')
        && !name.starts_with('/')
        && !name
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
}

fn walk_files(root: &Path) -> Result<Vec<String>, String> {
    let mut stack = vec![root.to_path_buf()];
    let mut found = Vec::new();
    while let Some(folder) = stack.pop() {
        if found.len() > 65_535 {
            return Err("Decoded PAK has too many members".into());
        }
        for item in fs::read_dir(&folder).map_err(|e| e.to_string())? {
            let item = item.map_err(|e| e.to_string())?;
            let ty = item.file_type().map_err(|e| e.to_string())?;
            if ty.is_symlink() {
                return Err("Official decoder produced a linked member".into());
            }
            if ty.is_dir() {
                stack.push(item.path());
            } else if ty.is_file() {
                let member = item
                    .path()
                    .strip_prefix(root)
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .replace('\\', "/");
                if !safe_member(&member) {
                    return Err("Unsafe official decoded member".into());
                }
                found.push(member);
            } else {
                return Err("Unsupported decoded filesystem entry".into());
            }
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_member_paths_never_leave_the_stage() {
        for path in [
            "../a",
            "/absolute",
            "a/../b",
            "a\\b",
            "C:/drive",
            "a//b",
            "",
        ] {
            assert!(!safe_member(path), "{path}");
        }
        assert!(safe_member("Libs/UI/main_menu.gfx"));
    }
    #[test]
    fn missing_optional_tools_use_existing_builder() {
        let dir = tempfile::tempdir().unwrap();
        if std::env::var_os("EVOLVE_STAGE2_TOOLS_DIR").is_none() {
            assert!(discover(dir.path()).unwrap().is_none());
        }
    }
}
