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


/// Show authored source-wave paths and their FMOD Designer ordering, not FSB sample indices.
pub fn project_waveforms(project: &Path, limit: usize) -> Result<(usize, Vec<String>), String> {
    if !project.extension().is_some_and(|e| e.eq_ignore_ascii_case("fdp")) {
        return Err("Select an .fdp project.".into());
    }
    let meta = fs::symlink_metadata(project).map_err(|e| e.to_string())?;
    if !meta.file_type().is_file() || meta.len() > 32 * 1024 * 1024 {
        return Err("Invalid or oversized FMOD project.".into());
    }
    let xml = fs::read_to_string(project).map_err(|e| e.to_string())?;
    let mut count = 0usize;
    let mut paths = Vec::new();
    let mut rest = xml.as_str();
    while let Some(start) = rest.find("<waveform>") {
        rest = &rest[start + "<waveform>".len()..];
        let end = rest.find("</waveform>").ok_or("Unclosed waveform element")?;
        let body = &rest[..end];
        let a = body.find("<filename>").ok_or("Waveform is missing its filename")? + "<filename>".len();
        let b = body[a..].find("</filename>").ok_or("Waveform has an incomplete filename")? + a;
        count += 1;
        if count <= limit { paths.push(body[a..b].to_owned()); }
        rest = &rest[end + "</waveform>".len()..];
        if count > 100_000 { return Err("Too many source-wave entries.".into()); }
    }
    Ok((count, paths))
}

fn xml_escape(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Create a new FMOD authoring scratch project with only the chosen waveform
/// path changed. The source and original PAK are never modified. User should
/// rebuild through legacy Designer and import a matching FSB/FEV afterward.
pub fn prepare_waveform_override(
    project: &Path,
    wav: &Path,
    one_based: usize,
    workspace: &Path,
) -> Result<PathBuf, String> {
    if one_based == 0 { return Err("Waveform numbers start at 1.".into()); }
    let original = fs::canonicalize(project).map_err(|e| e.to_string())?;
    let workspace = fs::canonicalize(workspace).map_err(|e| e.to_string())?;
    if !original.starts_with(workspace.join("files")) ||
       !original.extension().is_some_and(|e| e.eq_ignore_ascii_case("fdp")) {
        return Err("Only an editable project's FDP can be modified.".into());
    }
    let wav_meta = fs::symlink_metadata(wav).map_err(|e| e.to_string())?;
    if !wav_meta.file_type().is_file() || wav_meta.len() < 44 || wav_meta.len() > 64 * 1024 * 1024 {
        return Err("Choose a regular WAV file smaller than 64 MiB.".into());
    }
    let mut header = [0u8; 12];
    use std::io::Read;
    fs::File::open(wav).map_err(|e| e.to_string())?
        .read_exact(&mut header).map_err(|e| e.to_string())?;
    if &header[..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return Err("The selected replacement is not a RIFF WAV.".into());
    }
    let xml = fs::read_to_string(&original).map_err(|e| e.to_string())?;
    if xml.len() > 32 * 1024 * 1024 { return Err("FMOD project exceeds size budget.".into()); }
    let (_, _) = project_waveforms(&original, 0)?;
    let mut location = 0usize;
    let mut segment = None;
    for _ in 0..one_based {
        let Some(a) = xml[location..].find("<waveform>") else {
            return Err("The requested waveform index does not exist.".into());
        };
        let start = location + a + "<waveform>".len();
        let Some(b) = xml[start..].find("</waveform>") else {
            return Err("Unclosed waveform element.".into());
        };
        let end = start + b;
        segment = Some((start, end));
        location = end + "</waveform>".len();
    }
    let (start, end) = segment.ok_or("Waveform not found")?;
    let wave = &xml[start..end];
    let a = wave.find("<filename>").ok_or("Selected waveform has no filename")? + "<filename>".len();
    let b = a + wave[a..].find("</filename>").ok_or("Missing filename closing tag")?;
    // Preserve project source wave replacement separately from the PAK's files folder.
    let output_root = workspace.join("FMOD_Sources");
    fs::create_dir_all(&output_root).map_err(|e| e.to_string())?;
    let temporary = tempfile::Builder::new().prefix("wave-").tempdir_in(&output_root)
        .map_err(|e| e.to_string())?;
    let audio_copy = temporary.path().join("replacement.wav");
    fs::copy(wav, &audio_copy).map_err(|e| e.to_string())?;
    // Match the paths FMOD Designer consumes; local absolute paths are scoped
    // to the user's machine and are never installed into the game.
    let resolved = fs::canonicalize(&audio_copy).map_err(|e| e.to_string())?;
    let mut authored = String::with_capacity(xml.len() + 128);
    authored.push_str(&xml[..start+a]);
    authored.push_str(&xml_escape(&resolved.to_string_lossy().replace('\\', "/")));
    authored.push_str(&xml[start+b..]);
    let name = original.file_name().ok_or("Project name missing")?;
    // The managed project importer intentionally rejects replacement files
    // located *inside* the project directory. Keep the authoring WAV in
    // FMOD_Sources, but stage the edited FDP in independent OS scratch space.
    let edit_scratch = tempfile::Builder::new().prefix("evolve-fmod-fdp-").tempdir()
        .map_err(|e| e.to_string())?;
    let edit = edit_scratch.path().join(name);
    fs::write(&edit, authored).map_err(|e| e.to_string())?;
    inspect_fmod_file(&edit).map_err(|e| e.to_string())?;
    let _wave_files = temporary.keep();
    let saved = edit_scratch.keep();
    Ok(saved.join(name))
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
