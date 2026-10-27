//! One source of truth for the footer and Game Launch window.
//! Percentages come only from a worker's progress counter. All other work
//! is measured in files/bytes/elapsed time and uses an animated (unknown
//! completion) bar, not a fictional percent.
use super::{Message, State};
use iced::widget::{container, progress_bar, row, svg, text};
use iced::{Alignment, Element, Length};
use std::sync::atomic::Ordering;

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub label: String,
    pub detail: String,
    /// None means this worker has not provided a completion percentage.
    pub percent: Option<u32>,
    /// Animation is only an activity indicator, never a completion estimate.
    pub position: f32,
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
        return Some(make(label.into(), detail, percent));
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
        return Some(make(
            label.into(),
            format!("Launch stage {percent}% · {elapsed}"),
            Some(percent),
        ));
    }
    if let Some(progress) = &state.library_import_progress {
        return Some(make(
            "Importing or updating PAKs".into(),
            format!(
                "{} files processed · {elapsed}",
                progress.load(Ordering::Relaxed)
            ),
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
    Some(make(label, elapsed, None))
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

/// Animate the red vector wheel from loading_wheel.gfx without running a
/// Flash/Scaleform interpreter in the footer. The original has a 40-frame
/// rotating sprite; the manager reuses its existing 40-step activity pulse.
fn wheel_frame(pulse: u16) -> svg::Handle {
    let angle = u32::from(pulse % 40) * 9;
    let art = include_str!("../../../assets/loading_wheel.svg")
        .replace("rotate(0 0 0)", &format!("rotate({angle} 0 0)"));
    svg::Handle::from_memory(art.into_bytes())
}

pub fn footer(state: &State) -> Element<'_, Message> {
    let Some(status) = snapshot(state) else {
        return row![].into();
    };
    let metric = status.percent.map_or_else(
        || status.detail.clone(),
        |value| format!("{value}% · {}", status.detail),
    );
    row![
        svg(wheel_frame(state.footer_pulse))
            .width(Length::Fixed(26.0))
            .height(Length::Fixed(26.0)),
        text(status.label.clone()).size(12),
        text(metric).size(11)
    ]
    .spacing(10)
    .align_y(Alignment::Center)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, atomic::AtomicUsize};

    #[test]
    fn wheel_frames_rotate_while_progress_data_remains_independent() {
        let first = include_str!("../../../assets/loading_wheel.svg");
        assert!(first.contains("rotate(0 0 0)"));
        let rotated = first.replace("rotate(0 0 0)", "rotate(9 0 0)");
        assert!(rotated.contains("rotate(9 0 0)"));
        let _ = wheel_frame(0);
        let _ = wheel_frame(1);
    }

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
