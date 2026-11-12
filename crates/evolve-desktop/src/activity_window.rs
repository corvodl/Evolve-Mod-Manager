//! Detached, non-modal activity monitor. The monitor shows the same live
//! status transitions as the desktop; closing it never cancels the worker.
use super::{Message, State, work_progress};
use iced::widget::{button, column, container, row, scrollable, text};
use iced::{Element, Fill, Length};

pub(super) fn view(state: &State) -> Element<'_, Message> {
    let progress = work_progress::snapshot(state);
    let active = progress
        .as_ref()
        .map_or_else(|| "Idle".to_owned(), |snapshot| snapshot.label.clone());
    let meter: Element<'_, Message> = if let Some(snapshot) = &progress {
        column![
            work_progress::bar(snapshot, Length::Fill, 12.0),
            text(snapshot.detail.clone()).size(12),
        ]
        .spacing(6)
        .push(
            snapshot
                .steps
                .as_ref()
                .map(|steps| crate::ui_kit::stepper(steps, state.loading_wheel_frame)),
        )
        .into()
    } else {
        column![].into()
    };
    // The log reads as a timeline, newest first; failures get a red dot.
    let items = state
        .activity_log
        .iter()
        .rev()
        .take(250)
        .map(|message| {
            let lower = message.to_ascii_lowercase();
            let tone = if lower.contains("error") || lower.contains("failed") || lower.contains("refused") {
                Some(crate::design::Tone::Danger)
            } else if lower.contains("warning") {
                Some(crate::design::Tone::Warning)
            } else {
                None
            };
            crate::ui_kit::TimelineItem {
                title: message.clone(),
                meta: String::new(),
                body: None,
                tone,
            }
        })
        .collect();
    let lines = crate::ui_kit::timeline(items);
    container(
        column![
            row![
                text("MANAGER ACTIVITY").size(17),
                text(active).size(12),
                iced::widget::space().width(Fill),
                button("Clear History").on_press(Message::ClearActivity),
                button("Cancel Scan")
                    .on_press_maybe(state.scan_cancel.is_some().then_some(Message::CancelScan))
            ]
            .align_y(iced::Center)
            .spacing(12),
            text(
                "Live status for long-running operations. Closing this window does not cancel work."
            )
            .size(12),
            meter,
            container(text(&state.status).size(13)).padding(8),
            scrollable(lines).height(Fill)
        ]
        .spacing(14)
        .padding(16),
    )
    .width(Fill)
    .height(Fill)
    .into()
}
