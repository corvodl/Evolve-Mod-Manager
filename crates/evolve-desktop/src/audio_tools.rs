//! Optional, local FMOD audio helpers. No game files are modified here.
//! Decoder: pinned vgmstream-cli. Rebuilds: user-installed legacy FMOD Designer.
use evolve_core::fmod::{FmodInfo, inspect_fmod_file};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MAX_PREVIEW_WAV: u64 = 128 * 1024 * 1024;
const MAX_REBUILD_BANK: u64 = 1024 * 1024 * 1024;

fn regular_executable(path: &str, name: &str) -> Option<PathBuf> {
    let file = Path::new(path.trim());
    if !file.is_absolute() || !file.file_name()?.eq_ignore_ascii_case(name) {
        return None;
    }
    let info = fs::symlink_metadata(file).ok()?;
    if !info.file_type().is_file() || info.len() == 0 {
        return None;
    }
    fs::canonicalize(file).ok()
}

pub fn valid_decoder(path: &str) -> Option<PathBuf> {
    regular_executable(path, "vgmstream-cli.exe")
}

pub fn valid_designer(path: &str) -> Option<PathBuf> {
    regular_executable(path, "fmod_designer.exe")
}

pub fn default_decoder(root: &Path) -> Option<PathBuf> {
    let file = root.join("third_party/vgmstream/vgmstream-cli.exe");
    valid_decoder(file.to_str()?)
}

pub fn default_designer() -> Option<PathBuf> {
    // User-installed legacy Designer; never download or bundle proprietary software.
    for variable in ["ProgramFiles(x86)", "ProgramFiles"] {
        let Some(root) = std::env::var_os(variable) else { continue; };
        let candidate = PathBuf::from(root)
            .join("FMOD SoundSystem/FMOD Designer/fmod_designer.exe");
        if let Some(path) = valid_designer(candidate.to_str()?) {
            return Some(path);
        }
    }
    None
}

fn run_limited(command: &mut Command, timeout: Duration) -> Result<(), String> {
    command.stdout(Stdio::null()).stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let mut child = command.spawn().map_err(|e| format!("Cannot launch external audio tool: {e}"))?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            if status.success() { return Ok(()); }
            return Err(format!("Audio tool exited with status {status}; verify codecs and dependencies."));
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Audio tool timed out and was stopped.".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn verified_bank(source: &Path, index: u32) -> Result<(), String> {
    let FmodInfo::Bank { samples, .. } = inspect_fmod_file(source).map_err(|e| e.to_string())? else {
        return Err("Select an FSB5 sound bank.".into());
    };
    if index == 0 || index > samples {
        return Err(format!("Sample number must be between 1 and {samples}."));
    }
    Ok(())
}

pub fn decode_to_wav(source: &Path, decoder: &str, index: u32, output: &Path) -> Result<(), String> {
    verified_bank(source, index)?;
    let exe = valid_decoder(decoder).ok_or("Install vgmstream in Settings → Integrations → Audio Tools.")?;
    if output.exists() {
        return Err("The output WAV already exists; choose a different destination.".into());
    }
    let mut command = Command::new(exe);
    command.arg("-i").arg("-s").arg(index.to_string())
        .arg("-o").arg(output).arg(source);
    run_limited(&mut command, Duration::from_secs(90))?;
    let metadata = fs::symlink_metadata(output).map_err(|e| e.to_string())?;
    if !metadata.file_type().is_file() || metadata.len() < 44 || metadata.len() > MAX_PREVIEW_WAV {
        let _ = fs::remove_file(output);
        return Err("Decoder output is missing or exceeds the preview size limit.".into());
    }
    let bytes = fs::read(output).map_err(|e| e.to_string())?;
    if !bytes.starts_with(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        let _ = fs::remove_file(output);
        return Err("Decoder did not produce a RIFF WAVE file.".into());
    }
    Ok(())
}

pub fn export_sample(source: &Path, decoder: &str, index: u32, folder: &Path) -> Result<PathBuf, String> {
    let meta = fs::symlink_metadata(folder).map_err(|e| e.to_string())?;
    if !meta.file_type().is_dir() { return Err("Select an existing output folder.".into()); }
    let name = source.file_stem().and_then(|n| n.to_str())
        .ok_or("Invalid source bank name")?;
    let dest = folder.join(format!("{name}_sample_{index:04}.wav"));
    decode_to_wav(source, decoder, index, &dest)?;
    Ok(dest)
}

/// Decode a selected subsong into disposable storage and play it, on Windows,
/// without opening a command prompt or modifying the FMOD bank.
pub fn preview_sample(source: &Path, decoder: &str, index: u32) -> Result<String, String> {
    let scratch = tempfile::tempdir().map_err(|e| e.to_string())?;
    let wav = scratch.path().join("preview.wav");
    decode_to_wav(source, decoder, index, &wav)?;
    #[cfg(target_os = "windows")]
    {
        // PowerShell is a Windows-provided launcher for managed SoundPlayer,
        // not a shell-string interpolation of the untrusted source path.
        let script = scratch.path().join("play.ps1");
        fs::write(&script, "param([string]$File)\n$player = New-Object System.Media.SoundPlayer($File)\n$player.Load()\n$player.PlaySync()\n")
            .map_err(|e| e.to_string())?;
        let mut command = Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&script).arg(&wav);
        run_limited(&mut command, Duration::from_secs(120))?;
        Ok(format!("Played sample {index} from {}.", source.display()))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = wav;
        Err("Audio playback is currently supported by the Windows manager.".into())
    }
}

/// Open a *local, user-installed* FMOD Designer project editor explicitly.
pub fn open_designer(project: &Path, configured: &str) -> Result<String, String> {
    if !project.extension().is_some_and(|x| x.eq_ignore_ascii_case("fdp")) {
        return Err("Select a .fdp FMOD Designer project.".into());
    }
    inspect_fmod_file(project).map_err(|e| e.to_string())?;
    let editor = valid_designer(configured).ok_or("Configure a licensed FMOD Designer executable in Settings → Integrations.")?;
    Command::new(editor).arg(project).spawn().map_err(|e| e.to_string())?;
    Ok("Opened the project in FMOD Designer. Rebuild FEV/FSB there, then import the matching bank into your Modding project.".into())
}

/// Reject a rebuilt bank with a different sample index or codec layout.
/// This is a structural preflight, not proof the Evolve runtime will load it.
pub fn verify_rebuilt(original: &Path, replacement: &Path) -> Result<String, String> {
    let old_name = original.file_name().ok_or("Original filename missing")?;
    let new_name = replacement.file_name().ok_or("Replacement filename missing")?;
    if !old_name.eq_ignore_ascii_case(new_name) {
        return Err("Rebuilt asset must have the exact original filename.".into());
    }
    let new_meta = fs::symlink_metadata(replacement).map_err(|e| e.to_string())?;
    if !new_meta.file_type().is_file() || new_meta.len() > MAX_REBUILD_BANK {
        return Err("Rebuilt asset is not a regular file of an acceptable size.".into());
    }
    let a = inspect_fmod_file(original).map_err(|e| e.to_string())?;
    let b = inspect_fmod_file(replacement).map_err(|e| e.to_string())?;
    match (a, b) {
        (FmodInfo::Bank { samples: s1, codec: c1, .. }, FmodInfo::Bank { samples: s2, codec: c2, .. })
            if s1 == s2 && c1 == c2 =>
            Ok(format!("FSB5 preflight passed: {s2} samples, {c2}. Gameplay compatibility still requires offline testing.")),
        (FmodInfo::Events { .. }, FmodInfo::Events { .. }) =>
            Ok("FEV header verified. Event IDs and relationships have NOT been validated; test offline.".into()),
        (FmodInfo::Bank { .. }, FmodInfo::Bank { .. }) =>
            Err("Sample count or codec differs from original FSB5 bank; cannot safely stage.".into()),
        _ => Err("Replacement is not the same kind of FMOD asset.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_missing_or_untrusted_tool_paths() {
        assert!(valid_decoder("").is_none());
        assert!(valid_designer("fmod_designer.exe").is_none());
    }
    #[test]
    fn requires_valid_sound_bank_before_decoding() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("test.fsb");
        fs::write(&f, b"not a valid FSB").unwrap();
        assert!(decode_to_wav(&f, "", 1, &t.path().join("output.wav")).is_err());
    }
}
