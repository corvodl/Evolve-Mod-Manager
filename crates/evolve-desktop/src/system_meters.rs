//! Disk and memory levels for the System Health meters. Kept apart from
//! `system_health`, which the privileged worker also compiles.

use std::path::Path;

/// One measured level: `used` of `total` bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meter {
    pub label: String,
    pub detail: String,
    pub used: u64,
    pub total: u64,
}

impl Meter {
    pub fn level(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.used as f64 / self.total as f64) as f32
        }
    }
}

/// The drive (path prefix) a path lives on, for de-duplicating meters.
fn drive_of(path: &Path) -> Option<std::path::PathBuf> {
    path.ancestors()
        .last()
        .map(Path::to_path_buf)
        .filter(|root| !root.as_os_str().is_empty())
}

/// Free space on the drives holding each labelled path (each drive once),
/// then physical memory. GPU memory isn't readable without a GPU API, so it
/// is left out.
pub fn meters(paths: &[(&str, &str)]) -> Vec<Meter> {
    let mut meters = Vec::new();
    let mut seen = Vec::new();
    for (label, path) in paths {
        let path = Path::new(path.trim());
        if path.as_os_str().is_empty() || !path.is_absolute() {
            continue;
        }
        let Some(existing) = path.ancestors().find(|candidate| candidate.exists()) else {
            continue;
        };
        let drive = drive_of(existing);
        if seen.contains(&drive) {
            continue;
        }
        seen.push(drive.clone());
        let (Ok(total), Ok(free)) = (fs2::total_space(existing), fs2::available_space(existing))
        else {
            continue;
        };
        let name = drive
            .map(|root| root.display().to_string())
            .unwrap_or_default();
        meters.push(Meter {
            label: format!("{label} ({})", name.trim_end_matches(['\\', '/'])),
            detail: format!("{} free of {}", gib(free), gib(total)),
            used: total.saturating_sub(free),
            total,
        });
    }
    let mut system = sysinfo::System::new();
    system.refresh_memory();
    let total = system.total_memory();
    if total > 0 {
        let used = system.used_memory().min(total);
        meters.push(Meter {
            label: "Memory (RAM)".into(),
            detail: format!("{} in use of {}", gib(used), gib(total)),
            used,
            total,
        });
    }
    meters
}

fn gib(bytes: u64) -> String {
    format!("{:.1} GiB", bytes as f64 / 1_073_741_824.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn meter_level_handles_zero_total() {
        let meter = Meter {
            label: "x".into(),
            detail: String::new(),
            used: 5,
            total: 0,
        };
        assert_eq!(meter.level(), 0.0);
    }

    #[test]
    fn meters_measure_each_drive_once_plus_memory() {
        let temp = tempfile::tempdir().unwrap();
        let a = temp.path().join("a");
        let b = temp.path().join("missing").join("b");
        fs::create_dir(&a).unwrap();
        let found = meters(&[
            ("Game drive", a.to_str().unwrap()),
            ("Stage drive", b.to_str().unwrap()),
            ("Relative", "not/absolute"),
        ]);
        let disks: Vec<_> = found
            .iter()
            .filter(|m| m.label.starts_with("Game"))
            .collect();
        assert_eq!(disks.len(), 1);
        assert!(disks[0].total >= disks[0].used);
        assert!(!found.iter().any(|m| m.label.starts_with("Stage")));
        assert!(!found.iter().any(|m| m.label.starts_with("Relative")));
    }
}
