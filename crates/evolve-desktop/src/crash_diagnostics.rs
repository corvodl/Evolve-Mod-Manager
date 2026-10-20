//! Local, bounded, opt-in-free crash diagnostics for the portable viewer.
//! Log messages contain a fixed event and an optional sanitized asset-relative
//! name, never game keys or archive contents. Native fail-fast can bypass a
//! Rust panic hook, but the preceding loading stage remains on disk.
use std::backtrace::Backtrace;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_LOG_BYTES: u64 = 1024 * 1024;
static LOG_LOCK: Mutex<()> = Mutex::new(());
static LOG_ENABLED: AtomicBool = AtomicBool::new(true);

pub fn enabled() -> bool {
    LOG_ENABLED.load(Ordering::Relaxed)
}

pub fn set_enabled(value: bool) {
    LOG_ENABLED.store(value, Ordering::Relaxed);
}

pub fn log_path() -> io::Result<PathBuf> {
    #[cfg(windows)]
    let root = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA is missing"))?;
    #[cfg(not(windows))]
    let root = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is missing"))?
        .join(".config");
    Ok(root
        .join("EvolveModManager")
        .join("Data")
        .join("Logs")
        .join("rust-preview.log"))
}

/// Escape line breaks, control bytes and exceptionally long values so a
/// source filename cannot forge separate log entries.
fn safe_detail(detail: &str) -> String {
    detail
        .chars()
        .take(8_192)
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

pub fn record(event: &str, detail: &str) {
    if !enabled() {
        return;
    }
    // A failing or poisoned log must never become another reason to crash.
    let Ok(_guard) = LOG_LOCK.try_lock() else {
        return;
    };
    let _ = write_event(event, detail);
}

fn write_event(event: &str, detail: &str) -> io::Result<()> {
    let path = log_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if fs::metadata(&path).is_ok_and(|meta| meta.len() >= MAX_LOG_BYTES) {
        // Keep only the previous bounded log, preserving one earlier crash.
        let rotated = path.with_extension("previous.log");
        let _ = fs::remove_file(&rotated);
        fs::rename(&path, rotated)?;
    }
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(
        file,
        "{secs} pid={} {} {}",
        std::process::id(),
        safe_detail(event),
        safe_detail(detail)
    )?;
    file.flush()
}

pub fn install(start_enabled: bool) {
    set_enabled(start_enabled);
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
            .unwrap_or("Non-string panic");
        let place = info.location().map_or_else(
            || "unknown".to_owned(),
            |l| format!("{}:{}", l.file(), l.line()),
        );
        if !enabled() {
            original(info);
            return;
        }
        record("rust_panic", &format!("{place}: {payload}"));
        let trace = Backtrace::force_capture();
        // Write a bounded number of trace characters; avoid allocating an
        // unbounded diagnostic when an allocator or GPU driver just failed.
        record(
            "rust_backtrace",
            &trace.to_string().chars().take(12_000).collect::<String>(),
        );
        original(info);
    }));
    record(
        "startup",
        concat!("evolve-mod-manager ", env!("CARGO_PKG_VERSION")),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_filename_is_not_the_install_or_game_directory() {
        let path = log_path().unwrap();
        assert!(path.ends_with("EvolveModManager/Data/Logs/rust-preview.log"));
    }

    #[test]
    fn asset_label_is_line_safe_and_limited() {
        assert_eq!(
            safe_detail("rocky3.cdf\nfabricated log"),
            "rocky3.cdf fabricated log"
        );
        assert_eq!(safe_detail(&"a".repeat(9_000)).chars().count(), 8_192);
    }
    #[test]
    fn disabling_logging_suppresses_events_and_can_be_reenabled() {
        let old = enabled();
        set_enabled(false);
        assert!(!enabled());
        // This should not touch the filesystem, even if the log path
        // is missing or the user's profile is read-only.
        record("suppressed", "test");
        set_enabled(true);
        assert!(enabled());
        set_enabled(old);
    }

}
