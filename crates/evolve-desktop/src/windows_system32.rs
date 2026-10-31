//! Resolve trusted Windows system tools without using the caller-controlled
//! WINDIR/SystemRoot environment variables. This module never starts a process.
use std::path::PathBuf;

#[cfg(windows)]
#[allow(unsafe_code)] // Narrow Win32 API call to discover the real System32 path.
pub fn executable(name: &str) -> Result<PathBuf, String> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    if !matches!(name, "netsh.exe" | "tasklist.exe") {
        return Err("System tool is not on the approved diagnostic list".into());
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
    }

    let mut buffer = [0_u16; 1024];
    // GetSystemDirectoryW returns the number of UTF-16 code units written,
    // excluding NUL, or the required buffer length if the buffer is too small.
    let count = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
    if count == 0 || count as usize >= buffer.len() {
        return Err("Cannot resolve the Windows System32 directory".into());
    }
    let directory = PathBuf::from(OsString::from_wide(&buffer[..count as usize]));
    let path = directory.join(name);
    let meta = std::fs::symlink_metadata(&path)
        .map_err(|error| format!("Cannot find trusted Windows system tool: {error}"))?;
    if !meta.file_type().is_file() {
        return Err("Windows system tool is not a regular file".into());
    }
    Ok(path)
}

#[cfg(not(windows))]
pub fn executable(_name: &str) -> Result<PathBuf, String> {
    Err("Windows system tools require Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_rejects_unapproved_system_tools() {
        assert!(executable("cmd.exe").is_err());
        assert!(executable("powershell.exe").is_err());
        assert!(executable("..\\netsh.exe").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn approved_tools_resolve_inside_windows_system32() {
        for tool in ["netsh.exe", "tasklist.exe"] {
            let path = executable(tool).expect("Windows system tool");
            assert!(path.is_absolute());
            assert_eq!(path.file_name().and_then(|name| name.to_str()), Some(tool));
            assert!(path.parent().and_then(|p| p.file_name()).is_some_and(
                |folder| folder.to_string_lossy().eq_ignore_ascii_case("System32")
            ));
        }
    }
}
