//! Native import-source selection, invoked only off the Iced UI thread.
//! Windows dialogs open without a PowerShell console window. The untrusted
//! returned paths are always verified by evolve_core before any import.
use std::path::PathBuf;

fn selected_lines(stdout: &[u8]) -> Result<Option<Vec<PathBuf>>, String> {
    let text = String::from_utf8(stdout.to_vec())
        .map_err(|_| "The file picker returned a non-UTF8 filename".to_owned())?;
    let files: Vec<PathBuf> = text.lines().map(str::trim)
        .filter(|line| !line.is_empty()).map(PathBuf::from).collect();
    Ok((!files.is_empty()).then_some(files))
}

/// Choose multiple PAKs or a source directory. `Ok(None)` is cancellation.
pub(crate) fn choose_sources(folder: bool) -> Result<Option<Vec<PathBuf>>, String> {
    #[cfg(target_os = "windows")]
    {
        let script = if folder {
            r#"[Console]::OutputEncoding=New-Object System.Text.UTF8Encoding($false); Add-Type -AssemblyName System.Windows.Forms; $d=New-Object System.Windows.Forms.FolderBrowserDialog; $d.Description='Choose EvolveGame or a folder of PAKs (subfolders included)'; if($d.ShowDialog() -eq 'OK') { [Console]::WriteLine($d.SelectedPath) }"#
        } else {
            r#"[Console]::OutputEncoding=New-Object System.Text.UTF8Encoding($false); Add-Type -AssemblyName System.Windows.Forms; $d=New-Object System.Windows.Forms.OpenFileDialog; $d.Title='Select one or more Evolve PAK archives'; $d.Filter='Evolve PAK files (*.pak)|*.pak'; $d.Multiselect=$true; if($d.ShowDialog() -eq 'OK') { $d.FileNames | ForEach-Object { [Console]::WriteLine($_) } }"#
        };
        use std::os::windows::process::CommandExt;
        let mut command = std::process::Command::new("powershell.exe");
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let output = command.args(["-NoProfile", "-NonInteractive", "-STA", "-Command", script])
            .output().map_err(|e| format!("Cannot open Windows file picker: {e}"))?;
        if !output.status.success() {
            return Err("Windows archive picker failed. Select a valid local folder or PAK.".into());
        }
        selected_lines(&output.stdout)
    }
    #[cfg(target_os = "macos")]
    {
        let command = if folder {
            "POSIX path of (choose folder with prompt \"Select folder containing Evolve PAK files\")"
        } else {
            "POSIX path of (choose file with prompt \"Select an Evolve PAK\")"
        };
        let output = std::process::Command::new("osascript")
            .args(["-e", command]).output().map_err(|e| e.to_string())?;
        if !output.status.success() { return Ok(None); }
        selected_lines(&output.stdout)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let mut dialog = std::process::Command::new("zenity");
        dialog.arg("--file-selection");
        if folder {
            dialog.arg("--directory");
        } else {
            dialog.args(["--multiple", "--separator=\n", "--file-filter=PAK archives | *.pak *.PAK"]);
        }
        let output = dialog.output().map_err(|e| format!("Cannot open file picker: {e}"))?;
        if !output.status.success() { return Ok(None); }
        selected_lines(&output.stdout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_parser_preserves_multiple_paths_and_cancel() {
        assert!(selected_lines(b"").unwrap().is_none());
        assert_eq!(
            selected_lines(b"C:\\EvolveGame\\UI.pak\r\nD:\\mods\\shared.pak\n").unwrap().unwrap(),
            vec![PathBuf::from("C:\\EvolveGame\\UI.pak"), PathBuf::from("D:\\mods\\shared.pak")]
        );
    }
}
