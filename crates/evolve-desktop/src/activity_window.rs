//! Detached, non-modal activity monitor. The monitor shows the same live
//! status transitions as the desktop; closing it never cancels the worker.
use super::{Message, State};
use iced::widget::{button, column, container, row, scrollable, text};
use iced::{Element, Fill};

pub(super) fn view(state: &State) -> Element<'_, Message> {
    let active = if state.busy { "Working" } else { "Idle" };
    let mut lines = column![].spacing(4);
    for message in state.activity_log.iter().rev().take(250) {
        lines = lines.push(text(message).size(12));
    }
    container(column![
        row![
            text("MANAGER ACTIVITY").size(17),
            text(active).size(12),
            iced::widget::space().width(Fill),
            button("Clear History").on_press(Message::ClearActivity),
            button("Cancel Scan")
                .on_press_maybe(state.scan_cancel.is_some().then_some(Message::CancelScan))
        ].align_y(iced::Center).spacing(12),
        text("Live status for long-running operations. Closing this window does not cancel work.").size(12),
        container(text(&state.status).size(13)).padding(8),
        scrollable(lines).height(Fill)
    ].spacing(14).padding(16)).width(Fill).height(Fill).into()
}
