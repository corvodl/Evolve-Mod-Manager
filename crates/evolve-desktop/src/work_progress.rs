//! One source of truth for the footer and Game Launch window.
//! Percentages come only from a worker's progress counter. All other work
//! is measured in files/bytes/elapsed time and uses an animated (unknown
//! completion) bar, not a fictional percent.
use super::{Message, State};
use iced::widget::{container, progress_bar};
use iced::{Element, Length};
use std::sync::atomic::Ordering;

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub label: String,
    pub detail: String,
    /// None means this worker has not provided a completion percentage.
    pub percent: Option<u32>,
    /// Animation is only an activity indicator, never a completion estimate.
    pub position: f32,
    /// Named phases for workers that publish their current step.
    pub steps: Option<crate::ui_kit::Steps>,
}

/// Project copy phases, matching `ProjectCopyProgress::phase`.
pub const COPY_STEPS: [&str; 4] = [
    "Checking imported assets",
    "Copying and verifying assets",
    "Saving project",
    "Opening project",
];

/// Play-with-mods phases, keyed from the launch worker's percentage.
pub const LAUNCH_STEPS: [&str; 6] = [
    "Build enabled mods",
    "Prepare signed PAKs",
    "Verify staged archives",
    "Prepare controlled launch",
    "Launcher ready — press Play",
    "Confirm PAK swap and game release",
];

pub fn launch_step(percent: u32) -> usize {
    match percent {
        0..=24 => 0,
        25..=69 => 1,
        70..=84 => 2,
        85..=94 => 3,
        95..=97 => 4,
        _ => 5,
    }
}

fn elapsed(state: &State) -> String {
    let seconds = state
        .activity_started
        .map(|start| start.elapsed().as_secs())
        .unwrap_or_default();
    format!("{seconds}s elapsed")
}

pub fn snapshot(state: &State) -> Option<Snapshot> {
    if !super::footer_is_working(state) {
        return None;
    }
    let elapsed = elapsed(state);
    let pulse = f32::from(state.footer_pulse % 40);
    let position = if pulse < 20.0 {
        pulse * 5.0
    } else {
        (40.0 - pulse) * 5.0
    };
    let make = |label: String, detail: String, percent| Snapshot {
        label,
        detail,
        percent,
        position,
        steps: None,
    };
    if let Some(progress) = &state.project_copy_progress {
        let phase = progress.phase.load(Ordering::Relaxed);
        let done = progress.files_done.load(Ordering::Relaxed);
        let total = progress.files_total.load(Ordering::Relaxed);
        let bytes = progress.bytes_done.load(Ordering::Relaxed);
        let size = progress.bytes_total.load(Ordering::Relaxed);
        let label = match phase {
            0 => "Checking imported assets",
            1 => "Copying and verifying assets",
            2 => "Saving project",
            _ => "Opening project",
        };
        let percent = if size > 0 && phase == 1 {
            Some(((bytes.min(size) as f64 / size as f64) * 95.0) as u32)
        } else {
            None
        };
        let detail = if size > 0 && phase == 1 {
            format!(
                "{done}/{total} files · {} / {} MiB · {elapsed}",
                bytes / 1048576,
                size / 1048576
            )
        } else {
            format!("{done}/{total} files · {elapsed}")
        };
        let mut snapshot = make(label.into(), detail, percent);
        snapshot.steps = Some(crate::ui_kit::Steps::new(&COPY_STEPS, phase.min(3)));
        return Some(snapshot);
    }
    if let Some(progress) = &state.mod_launch_progress {
        let percent = progress.load(Ordering::Relaxed).min(99) as u32;
        let label = match percent {
            0..=24 => "Building enabled mods",
            25..=69 => "Preparing signed PAKs",
            70..=84 => "Verifying staged archives",
            85..=94 => "Preparing controlled game launch",
            95..=97 => "Launcher ready — press Play",
            _ => "Confirming PAK swap and game release",
        };
        let mut snapshot = make(
            label.into(),
            format!("Launch stage {percent}% · {elapsed}"),
            Some(percent),
        );
        snapshot.steps = Some(crate::ui_kit::Steps::new(
            &LAUNCH_STEPS,
            launch_step(percent),
        ));
        return Some(snapshot);
    }
    if let Some(progress) = &state.library_import_progress {
        // The ticker eases between the worker's counter updates.
        let raw = progress.load(Ordering::Relaxed) as f64;
        let files = crate::ui_kit::ticked(&state.ui, "import_files", raw).round() as u64;
        return Some(make(
            "Importing or updating PAKs".into(),
            format!("{files} files processed · {elapsed}"),
            None,
        ));
    }
    if let Some(progress) = &state.scan_indexed {
        return Some(make(
            "Indexing game assets".into(),
            format!(
                "{} entries indexed · {elapsed}",
                progress.load(Ordering::Relaxed)
            ),
            None,
        ));
    }
    if let Some(progress) = &state.viewer_load_progress {
        let percent = progress.load(Ordering::Relaxed).min(99) as u32;
        return Some(make(
            "Loading character model".into(),
            format!("Model stages {percent}% · {elapsed}"),
            Some(percent),
        ));
    }
    let label = if state.gfx_busy || state.gfx_install_busy {
        "Editing or previewing GFX".to_owned()
    } else if state.storage_analyzing {
        "Analyzing game storage".to_owned()
    } else if state.library_import_scanning {
        "Discovering game PAKs".to_owned()
    } else if state.library_startup_loading {
        "Loading Asset Library".to_owned()
    } else if state.runtime_checking || state.developer_checking {
        "Checking runtime diagnostics".to_owned()
    } else if state.busy {
        state
            .busy_operation
            .as_deref()
            .unwrap_or("Processing selected assets")
            .to_owned()
    } else {
        "Processing game assets".to_owned()
    };
    let mut snapshot = make(label, elapsed, None);
    if state.busy_operation.as_deref().is_some_and(|op| {
        op == "Creating isolated offline setup" || op == "Creating isolated Test Game mod setup"
    }) {
        snapshot.steps = Some(crate::ui_kit::Steps::new(
            &crate::initial_setup::SETUP_STEPS,
            crate::initial_setup::current_step(),
        ));
    }
    Some(snapshot)
}

pub fn bar<'a>(snapshot: &Snapshot, width: Length, height: f32) -> Element<'a, Message> {
    let value = snapshot
        .percent
        .map_or(snapshot.position, |value| value as f32);
    container(
        progress_bar(0.0..=100.0, value)
            .length(width)
            .girth(Length::Fixed(height)),
    )
    .width(width)
    .height(Length::Fixed(height))
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, atomic::AtomicUsize};

    #[test]
    fn unmeasured_work_reports_a_named_task_and_elapsed_time_not_percentage() {
        let state = State {
            busy: true,
            busy_operation: Some("Inspecting signed PAK".into()),
            ..State::default()
        };
        let measured = snapshot(&state).unwrap();
        assert_eq!(measured.label, "Inspecting signed PAK");
        assert!(measured.detail.contains("elapsed"));
        assert_eq!(measured.percent, None);
    }

    #[test]
    fn launch_progress_is_shared_and_never_claims_swap_at_hook_ready() {
        let state = State {
            busy: true,
            mod_launch_progress: Some(Arc::new(AtomicUsize::new(95))),
            ..State::default()
        };
        let measured = snapshot(&state).unwrap();
        assert!(measured.label.contains("press Play"));
        assert_eq!(measured.percent, Some(95));
        let steps = measured.steps.unwrap();
        assert_eq!(steps.current, 4);
        assert_eq!(steps.state(3), crate::ui_kit::StepState::Done);
    }

    #[test]
    fn file_counters_are_quantified_without_a_fake_percent() {
        let state = State {
            busy: true,
            scan_indexed: Some(Arc::new(AtomicUsize::new(123))),
            ..State::default()
        };
        let measured = snapshot(&state).unwrap();
        assert!(measured.detail.contains("123 entries"));
        assert_eq!(measured.percent, None);
    }
}
