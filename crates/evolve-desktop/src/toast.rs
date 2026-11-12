//! Transient completion toasts for long-running worker operations.
//!
//! The footer status line keeps the full, persistent message; a toast is only
//! a short-lived "this finished / this failed" cue that is visible from any
//! page or window. Toasts never block input outside their own cards.

use std::time::{Duration, Instant};

use iced::widget::{column, container, mouse_area, row, space, stack, text};
use iced::{Background, Border, Color, Element, Fill, Theme};

use crate::{Message, design};

const MAX_VISIBLE: usize = 4;
const SUCCESS_LIFETIME: Duration = Duration::from_secs(6);
const ERROR_LIFETIME: Duration = Duration::from_secs(12);
const UPDATE_LIFETIME: Duration = Duration::from_secs(20);
const FADE_IN: Duration = Duration::from_millis(180);
const FADE_OUT: Duration = Duration::from_millis(320);
/// Operations that ran at least this long announce success even when they
/// are not one of the explicitly announced user actions.
const SLOW_OPERATION: Duration = Duration::from_secs(10);
const DETAIL_LIMIT: usize = 180;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Success,
    Error,
    /// A newer release is available; clicking opens Settings > Updates.
    Update,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub id: u64,
    pub kind: Kind,
    pub title: String,
    pub detail: String,
    /// Sent when the card body is clicked (the toast is dismissed first).
    action: Option<Message>,
    created: Instant,
    lifetime: Duration,
}

impl Toast {
    fn age(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.created)
    }

    fn expired(&self, now: Instant) -> bool {
        self.age(now) >= self.lifetime
    }

    /// 0..=1 opacity for the fade in/out envelope.
    fn opacity(&self, now: Instant, reduce_motion: bool) -> f32 {
        if reduce_motion {
            return 1.0;
        }
        let age = self.age(now);
        let remaining = self.lifetime.saturating_sub(age);
        let fade_in = (age.as_secs_f32() / FADE_IN.as_secs_f32()).min(1.0);
        let fade_out = (remaining.as_secs_f32() / FADE_OUT.as_secs_f32()).min(1.0);
        fade_in.min(fade_out).clamp(0.0, 1.0)
    }

    /// Fraction of the lifetime still remaining, for the countdown bar.
    fn remaining_fraction(&self, now: Instant) -> f32 {
        let lifetime = self.lifetime.as_secs_f32().max(f32::EPSILON);
        (1.0 - self.age(now).as_secs_f32() / lifetime).clamp(0.0, 1.0)
    }
}

/// An operation that was launched and should report when it finishes.
#[derive(Debug, Clone)]
pub struct Pending {
    label: &'static str,
    announce_success: bool,
    started: Instant,
}

#[derive(Debug, Default)]
pub struct Toasts {
    items: Vec<Toast>,
    next_id: u64,
    /// The operation currently running, if it was started through `launch`.
    pub pending: Option<Pending>,
    /// Last frame clock; drives fades without reading the clock in `view`.
    now: Option<Instant>,
}

impl Toasts {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn begin(&mut self, label: &'static str, announce_success: bool) {
        self.pending = Some(Pending {
            label,
            announce_success,
            started: Instant::now(),
        });
    }

    /// Report a finished operation. `status` is the footer message the
    /// completion handler just wrote, used as the toast's detail line.
    pub fn complete(&mut self, pending: Option<Pending>, ok: bool, status: &str) {
        let Some(pending) = pending else {
            return;
        };
        if ok {
            if pending.announce_success || pending.started.elapsed() >= SLOW_OPERATION {
                self.push(Kind::Success, format!("{} finished", pending.label), status);
            }
        } else {
            self.push(Kind::Error, format!("{} failed", pending.label), status);
        }
    }

    pub fn push(&mut self, kind: Kind, title: impl Into<String>, detail: &str) {
        self.push_with_action(kind, title, detail, None);
    }

    /// "A Beta update is available!" — clicking it runs `action`.
    pub fn push_update(&mut self, channel: &str, version: &str, action: Message) {
        self.push_with_action(
            Kind::Update,
            format!("A {channel} update is available!"),
            &format!("• {version} is available to download."),
            Some(action),
        );
    }

    fn push_with_action(
        &mut self,
        kind: Kind,
        title: impl Into<String>,
        detail: &str,
        action: Option<Message>,
    ) {
        let now = Instant::now();
        self.next_id += 1;
        self.items.push(Toast {
            id: self.next_id,
            kind,
            title: title.into(),
            detail: compact(detail),
            action,
            created: now,
            lifetime: match kind {
                Kind::Success => SUCCESS_LIFETIME,
                Kind::Error => ERROR_LIFETIME,
                Kind::Update => UPDATE_LIFETIME,
            },
        });
        // Oldest toasts make way for new ones.
        let overflow = self.items.len().saturating_sub(MAX_VISIBLE);
        self.items.drain(..overflow);
        self.now = Some(now);
    }

    pub fn dismiss(&mut self, id: u64) {
        self.items.retain(|toast| toast.id != id);
    }

    /// Dismiss a clicked toast and return its action, if it has one.
    pub fn take_action(&mut self, id: u64) -> Option<Message> {
        let index = self.items.iter().position(|toast| toast.id == id)?;
        self.items.remove(index).action
    }

    pub fn tick(&mut self, now: Instant) {
        self.items.retain(|toast| !toast.expired(now));
        self.now = Some(now);
    }

    pub fn tick_interval(reduce_motion: bool) -> Duration {
        if reduce_motion {
            Duration::from_millis(500)
        } else {
            Duration::from_millis(33)
        }
    }
}

fn compact(detail: &str) -> String {
    let single_line = detail.split_whitespace().collect::<Vec<_>>().join(" ");
    if single_line.chars().count() <= DETAIL_LIMIT {
        single_line
    } else {
        let mut cut: String = single_line.chars().take(DETAIL_LIMIT).collect();
        cut.push('…');
        cut
    }
}

fn faded(color: Color, opacity: f32) -> Color {
    Color {
        a: color.a * opacity,
        ..color
    }
}

fn accent(theme: &Theme, kind: Kind) -> Color {
    let palette = theme.palette();
    match kind {
        Kind::Success => palette.success,
        Kind::Error => palette.danger,
        Kind::Update => palette.primary,
    }
}

fn card<'a>(toast: &'a Toast, now: Instant, reduce_motion: bool) -> Element<'a, Message> {
    let opacity = toast.opacity(now, reduce_motion);
    let kind = toast.kind;
    let remaining = toast.remaining_fraction(now);

    let marker = container(space().width(8).height(8)).style(move |theme: &Theme| {
        container::Style {
            background: Some(Background::Color(faded(accent(theme, kind), opacity))),
            border: Border {
                radius: if design::is_legacy(theme) { 0.0 } else { 4.0 }.into(),
                ..Border::default()
            },
            ..container::Style::default()
        }
    });
    let title = text(&toast.title)
        .size(14)
        .style(move |theme: &Theme| text::Style {
            color: Some(faded(theme.palette().text, opacity)),
        });
    let close = design::button(text("✕").size(11))
        .padding([2, 6])
        .style(move |theme, status| {
            let mut style = design::text_button(theme, status);
            style.text_color = faded(style.text_color, opacity);
            style
        })
        .on_press(Message::DismissToast(toast.id));
    let header = row![marker, container(title).width(Fill), close]
        .spacing(10)
        .align_y(iced::Center);

    let mut body = column![header].spacing(6);
    if !toast.detail.is_empty() {
        body = body.push(
            text(&toast.detail)
                .size(12)
                .style(move |theme: &Theme| text::Style {
                    color: Some(faded(
                        theme.extended_palette().background.strongest.text,
                        opacity,
                    )),
                }),
        );
    }

    // Countdown bar: a thin accent strip that shrinks as the toast ages.
    let bar = row![
        container(space().height(2))
            .width(iced::Length::FillPortion(
                ((remaining * 1000.0) as u16).max(1)
            ))
            .style(move |theme: &Theme| container::Style {
                background: Some(Background::Color(faded(accent(theme, kind), opacity * 0.8))),
                ..container::Style::default()
            }),
        space()
            .height(2)
            .width(iced::Length::FillPortion(
                (((1.0 - remaining) * 1000.0) as u16).max(1)
            )),
    ];
    body = body.push(bar);

    let card = container(body)
        .padding([10, 12])
        .width(340)
        .style(move |theme: &Theme| {
            let legacy = design::is_legacy(theme);
            container::Style {
                background: Some(Background::Color(faded(design::surface(theme), opacity))),
                border: Border {
                    color: faded(accent(theme, kind), opacity * 0.7),
                    width: 1.0,
                    radius: if legacy { 1.0 } else { 10.0 }.into(),
                },
                shadow: iced::Shadow {
                    color: faded(Color::from_rgba(0.0, 0.0, 0.0, 0.35), opacity),
                    offset: iced::Vector::new(0.0, 4.0),
                    blur_radius: 14.0,
                },
                ..container::Style::default()
            }
        });
    if toast.action.is_some() {
        // The close button captures its own press, so only clicks on the
        // card body reach this handler.
        mouse_area(card)
            .on_press(Message::ToastClicked(toast.id))
            .interaction(iced::mouse::Interaction::Pointer)
            .into()
    } else {
        card.into()
    }
}

/// Layer the toast stack over `base`, anchored to the bottom-right corner
/// above the footer. Returns `base` unchanged when there is nothing to show.
pub fn overlay<'a>(
    base: Element<'a, Message>,
    toasts: &'a Toasts,
    reduce_motion: bool,
) -> Element<'a, Message> {
    if toasts.items.is_empty() {
        return base;
    }
    let now = toasts.now.unwrap_or_else(Instant::now);
    let cards = toasts
        .items
        .iter()
        .fold(column![].spacing(8), |cards, toast| {
            cards.push(card(toast, now, reduce_motion))
        });
    let layer = container(cards)
        .width(Fill)
        .height(Fill)
        .align_right(Fill)
        .align_bottom(Fill)
        .padding(iced::Padding {
            top: 24.0,
            right: 24.0,
            bottom: 56.0,
            left: 24.0,
        });
    stack![base, layer].into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(announce: bool) -> Option<Pending> {
        Some(Pending {
            label: "Building mod PAK",
            announce_success: announce,
            started: Instant::now(),
        })
    }

    #[test]
    fn announced_success_and_any_failure_produce_toasts() {
        let mut toasts = Toasts::default();
        toasts.complete(pending(true), true, "Built 3 files.");
        toasts.complete(pending(false), false, "Disk full");
        assert_eq!(toasts.items.len(), 2);
        assert_eq!(toasts.items[0].kind, Kind::Success);
        assert_eq!(toasts.items[0].title, "Building mod PAK finished");
        assert_eq!(toasts.items[1].kind, Kind::Error);
        assert_eq!(toasts.items[1].detail, "Disk full");
    }

    #[test]
    fn quick_unannounced_success_and_unlaunched_results_stay_quiet() {
        let mut toasts = Toasts::default();
        toasts.complete(pending(false), true, "Loaded texture.");
        toasts.complete(None, false, "error without a launched operation");
        assert!(toasts.is_empty());
    }

    #[test]
    fn queue_is_capped_and_expires() {
        let mut toasts = Toasts::default();
        for index in 0..6 {
            toasts.push(Kind::Success, format!("t{index}"), "");
        }
        assert_eq!(toasts.items.len(), MAX_VISIBLE);
        assert_eq!(toasts.items[0].title, "t2");
        toasts.tick(Instant::now() + ERROR_LIFETIME + Duration::from_millis(1));
        assert!(toasts.is_empty());
    }

    #[test]
    fn dismiss_removes_only_that_toast() {
        let mut toasts = Toasts::default();
        toasts.push(Kind::Success, "a", "");
        toasts.push(Kind::Error, "b", "");
        let first = toasts.items[0].id;
        toasts.dismiss(first);
        assert_eq!(toasts.items.len(), 1);
        assert_eq!(toasts.items[0].title, "b");
    }

    #[test]
    fn update_toast_uses_requested_wording_and_returns_its_action() {
        let mut toasts = Toasts::default();
        toasts.push_update("Beta", "0.5.0-beta.2", Message::OpenUpdatesSettings);
        let toast = &toasts.items[0];
        assert_eq!(toast.kind, Kind::Update);
        assert_eq!(toast.title, "A Beta update is available!");
        assert_eq!(toast.detail, "• 0.5.0-beta.2 is available to download.");
        let id = toast.id;
        assert!(matches!(
            toasts.take_action(id),
            Some(Message::OpenUpdatesSettings)
        ));
        assert!(toasts.is_empty());
        assert!(toasts.take_action(id).is_none());
    }

    #[test]
    fn verbose_detail_is_one_short_line() {
        let detail = compact(&"line one\n\nline two ".repeat(50));
        assert!(!detail.contains('\n'));
        assert!(detail.chars().count() <= DETAIL_LIMIT + 1);
    }
}
