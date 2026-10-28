//! A dedicated native Play result window; no need to discover launch errors
//! in the main window's scrolling footer or activity log.
use super::{Message, State, design, work_progress};
use iced::widget::{column, container, row, scrollable, space, text};
use iced::{Element, Fill, Length};
use std::sync::atomic::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Modded,
    Clean,
    Manager,
    FbxExport,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Preparing,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug)]
pub struct Notice {
    pub kind: Kind,
    pub phase: Phase,
    pub message: String,
}

impl Notice {
    pub fn preparing(kind: Kind) -> Self {
        Self {
            kind,
            phase: Phase::Preparing,
            message: match kind {
                Kind::Modded => "Building enabled mods and verifying the signed PAKs, hook and original-file recovery state. No in-game swap has happened yet.".into(),
                Kind::Clean => "Opening the regular client. The manager will not install mod files.".into(),
                Kind::Manager => "Checking the current manager task.".into(),
                Kind::FbxExport => "Exporting the selected asset to FBX.".into(),
            },
        }
    }

    pub fn finished(kind: Kind, phase: Phase, message: impl Into<String>) -> Self {
        Self {
            kind,
            phase,
            message: message.into(),
        }
    }

    pub fn title(&self) -> &'static str {
        match self.phase {
            Phase::Preparing if self.kind == Kind::FbxExport => "Preparing FBX export",
            Phase::Preparing => "Preparing game launch",
            Phase::Success if self.kind == Kind::Modded => "Modded game launch confirmed",
            Phase::Success if self.kind == Kind::FbxExport => "FBX export completed",
            Phase::Success => "Regular client opened",
            Phase::Warning if self.kind == Kind::FbxExport => "FBX export warning",
            Phase::Warning => "Mods were not installed",
            Phase::Error if self.kind == Kind::FbxExport => "FBX export failed",
            Phase::Error if self.kind == Kind::Manager => "Manager operation failed",
            Phase::Error => "Game launch needs attention",
        }
    }
}

pub fn view(state: &State) -> Element<'_, Message> {
    let Some(notice) = state.play_notice.as_ref() else {
        return container(text("No game launch to report."))
            .padding(24)
            .into();
    };
    let stage = if notice.kind == Kind::FbxExport {
        match notice.phase {
            Phase::Preparing => "EXPORTING — building the FBX file",
            Phase::Success => "COMPLETE — FBX export",
            Phase::Warning => "WARNING — FBX export",
            Phase::Error => "ERROR — FBX export validation failed",
        }
    } else if notice.kind == Kind::Manager {
        match notice.phase {
            Phase::Preparing => "PREPARING — manager task",
            Phase::Success => "COMPLETE — manager task",
            Phase::Warning => "WARNING — manager task",
            Phase::Error => "ERROR — manager task failed",
        }
    } else if notice.phase == Phase::Preparing && notice.kind == Kind::Modded {
        let progress = state
            .mod_launch_progress
            .as_ref()
            .map_or(0, |value| value.load(Ordering::Relaxed));
        if progress >= 95 {
            "READY TO PRESS PLAY IN CLIENT — hook attached. The actual PAK swap is not yet confirmed."
        } else if progress >= 85 {
            "Checking prepared copies, archive signatures and recovery safeguards…"
        } else {
            "PREPARING — rebuilding enabled mods and their isolated signing stage…"
        }
    } else {
        match notice.phase {
            Phase::Preparing => "Opening the regular client…",
            Phase::Success => "COMPLETE",
            Phase::Warning => "WARNING — Evolve may have opened without mods",
            Phase::Error => "ERROR — no verified modded launch",
        }
    };
    let explanation = match (notice.kind, notice.phase) {
        (Kind::Modded, Phase::Success) => {
            "The launcher confirmed the PAK swap, Evolve process release and helper completion. Automatic original-file restoration remains active."
        }
        (Kind::Modded, Phase::Error | Phase::Warning) => {
            "The manager did not confirm that enabled mods were installed. Close Evolve before retrying or restoring. Do not manually delete backup files or the swap journal."
        }
        (Kind::Clean, _) => "Regular-client mode does not apply the modded PAK swap.",
        (Kind::FbxExport, _) => {
            "The FBX export stopped before publishing a completed file. Review the Blender diagnostic above; original model, UVs and textures were not changed."
        }
        (Kind::Manager, _) => {
            "Review this error before repeating the operation. The manager's Activity window retains the detail."
        }
        _ => {
            "You may switch to the client to press Play. This window will show the verified launch outcome."
        }
    };
    // Show the exact same snapshot/counter as the footer: percentages only
    // for workers that actually publish them, an active sweep otherwise.
    let progress = if notice.phase == Phase::Preparing {
        work_progress::snapshot(state).unwrap_or_else(|| work_progress::Snapshot {
            label: "Opening game client".into(),
            detail: "Waiting for launch worker".into(),
            percent: None,
            position: 0.0,
        })
    } else {
        work_progress::Snapshot {
            label: match notice.phase {
                Phase::Success => "Launch completed".into(),
                Phase::Warning => "Game opened without verified mods".into(),
                Phase::Error if notice.kind == Kind::FbxExport => "FBX export stopped".into(),
                Phase::Error if notice.kind == Kind::Manager => "Manager task stopped".into(),
                Phase::Error => "Launch operation stopped".into(),
                Phase::Preparing => "Preparing game launch".into(),
            },
            detail: match notice.phase {
                Phase::Success => "Completed".into(),
                _ if notice.kind == Kind::FbxExport => "No completed FBX was published".into(),
                _ if notice.kind == Kind::Manager => "Manager task did not complete".into(),
                _ => "No completed modded swap can be assumed".into(),
            },
            percent: (notice.phase == Phase::Success).then_some(100),
            position: 0.0,
        }
    };
    let label = progress.label.clone();
    let meter = work_progress::bar(&progress, Length::Fill, 16.0);
    let detail = if let Some(percent) = progress.percent {
        format!("{percent}% · {}", progress.detail)
    } else {
        progress.detail.clone()
    };
    let content = column![
        text(notice.title()).size(23),
        text(stage).size(12).style(design::muted),
        column![
            text(label).size(13),
            meter,
            text(detail).size(12).style(design::muted),
        ]
        .spacing(7),
        scrollable(
            column![
                text(notice.message.as_str()).size(14),
                text(explanation).size(12).style(design::muted),
            ]
            .spacing(16)
        )
        .height(Length::Fill),
        row![
            space().width(Fill),
            design::button("Close").on_press(Message::DismissPlayNotice)
        ],
    ]
    .padding(23)
    .spacing(18)
    .width(Fill)
    .height(Fill);
    container(content).width(Fill).height(Fill).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_error_does_not_claim_game_launch_failure() {
        let export = Notice::finished(Kind::FbxExport, Phase::Error, "invalid tangent");
        assert_eq!(export.title(), "FBX export failed");
    }

    #[test]
    fn readiness_is_distinct_from_a_verified_launch() {
        let ready = Notice::preparing(Kind::Modded);
        assert_eq!(ready.phase, Phase::Preparing);
        assert_ne!(ready.title(), "Modded game launch confirmed");
        let failed = Notice::finished(Kind::Modded, Phase::Warning, "stale copy");
        assert_eq!(failed.title(), "Mods were not installed");
        assert!(failed.message.contains("stale copy"));
    }
}
