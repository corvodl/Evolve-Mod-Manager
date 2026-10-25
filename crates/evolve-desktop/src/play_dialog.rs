//! A dedicated native Play result window; no need to discover launch errors
//! in the main window's scrolling footer or activity log.
use super::{Message, State, design};
use iced::widget::{column, container, row, scrollable, space, text};
use iced::{Element, Fill, Length};
use std::sync::atomic::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Modded,
    Clean,
    Manager,
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
            Phase::Preparing => "Preparing game launch",
            Phase::Success if self.kind == Kind::Modded => "Modded game launch confirmed",
            Phase::Success => "Regular client opened",
            Phase::Warning => "Mods were not installed",
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
    let stage = if notice.phase == Phase::Preparing && notice.kind == Kind::Modded {
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
        (Kind::Manager, _) => "Review this error before repeating the operation. The manager's Activity window retains the detail.",
        _ => {
            "You may switch to the client to press Play. This window will show the verified launch outcome."
        }
    };
    let content = column![
        text(notice.title()).size(23),
        text(stage).size(12).style(design::muted),
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
    fn readiness_is_distinct_from_a_verified_launch() {
        let ready = Notice::preparing(Kind::Modded);
        assert_eq!(ready.phase, Phase::Preparing);
        assert_ne!(ready.title(), "Modded game launch confirmed");
        let failed = Notice::finished(Kind::Modded, Phase::Warning, "stale copy");
        assert_eq!(failed.title(), "Mods were not installed");
        assert!(failed.message.contains("stale copy"));
    }
}
