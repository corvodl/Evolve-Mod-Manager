//! Reusable interface components rebuilt for iced from common web UI kit
//! patterns (callouts, empty states, steppers, hold-to-confirm, number
//! tickers, skeletons, timelines, status island, dock, save status).
//!
//! All motion goes through [`crate::anim`]: values are time-based, frames
//! are only requested while something is moving (see [`UiState::animating`]
//! and [`UiState::ambient`]), and `reduce_motion` makes every effect
//! instant or static.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use iced::widget::{column, container, mouse_area, row, space, svg, text};
use iced::{Alignment, Background, Border, Element, Fill, Length, Point, Theme};

use crate::anim::{Easing, Tween};
use crate::design::{self, Radius, Tone};
use crate::{Message, command_palette};

/// How long a hold-to-confirm button must be held.
pub const HOLD_DURATION: Duration = Duration::from_millis(900);
/// Second click window for the reduce-motion two-click confirm.
const ARM_WINDOW: Duration = Duration::from_secs(4);
const TICKER_DURATION: Duration = Duration::from_millis(400);
const ISLAND_DURATION: Duration = Duration::from_millis(180);
const FOLDER_DURATION: Duration = Duration::from_millis(150);
const SEGMENT_DURATION: Duration = Duration::from_millis(220);
const SWITCH_DURATION: Duration = Duration::from_millis(160);
const COPIED_FOR: Duration = Duration::from_millis(1500);
/// Fixed row height of reorder lists (drop index = cursor y / row).
pub const REORDER_ROW: f32 = 34.0;
const REORDER_SLIDE: Duration = Duration::from_millis(120);
const SCRAMBLE_TOTAL: Duration = Duration::from_millis(420);
const SCRAMBLE_STEP_MS: u128 = 30;
const SCRAMBLE_GLYPHS: &[char] = &[
    'A', 'E', 'H', 'K', 'M', 'R', 'V', 'X', 'Z', '0', '3', '7', '#', '%', '/', '<', '>', '=', '+',
];

/// Split-flap resolve: character `i` settles at a staggered time across
/// [`SCRAMBLE_TOTAL`]; before that it shows a glyph that changes every
/// 30 ms. Spaces never scramble, so word shapes stay readable.
pub fn scramble(value: &str, elapsed: Duration) -> String {
    let chars: Vec<char> = value.chars().collect();
    let count = chars.len().max(1) as u128;
    let total = SCRAMBLE_TOTAL.as_millis();
    let elapsed_ms = elapsed.as_millis();
    let step = elapsed_ms / SCRAMBLE_STEP_MS;
    chars
        .iter()
        .enumerate()
        .map(|(index, ch)| {
            let settle = total * (index as u128 + 1) / count;
            if ch.is_whitespace() || elapsed_ms >= settle {
                *ch
            } else {
                let seed = (index as u128 * 31 + step * 17) as usize;
                SCRAMBLE_GLYPHS[seed % SCRAMBLE_GLYPHS.len()]
            }
        })
        .collect()
}
const DISCLOSE_DURATION: Duration = Duration::from_millis(240);
/// Height cap the accordion grows through; content beyond it appears once
/// the animation settles (iced cannot measure content ahead of layout).
const DISCLOSE_MAX: f32 = 900.0;
/// Skeleton pulse period.
const PULSE_PERIOD: f32 = 1.4;

#[derive(Debug, Clone)]
pub enum UiMsg {
    Palette(command_palette::Msg),
    /// Mouse down on a hold button: `(key, action)`.
    HoldPress(String, Box<Message>),
    /// Mouse released or left the button before the hold completed.
    HoldRelease,
    /// Reduce-motion confirm: first click arms, second click fires.
    ArmConfirm(String, Box<Message>),
    IslandHover(bool),
    DockMove(&'static str, f32),
    DockExit(&'static str),
    /// Cursor position over a spotlight card, or `None` when it leaves.
    Spotlight(String, Option<Point>),
    /// Segmented control selection: `(control id, index, message to run)`.
    Segment(&'static str, usize, Box<Message>),
    /// Switch flipped: `(switch key, new value, message to run)`.
    Switch(String, bool, Box<Message>),
    /// Image comparison split set by clicking or dragging (0..1).
    CompareSet(f32),
    /// Arrow-key nudge of the comparison split.
    CompareNudge(f32),
    /// Reorder list: start dragging row `index` of `list`.
    ReorderGrab(&'static str, usize, fn(usize, usize) -> Message),
    /// Cursor height over the list while dragging.
    ReorderMove(&'static str, f32),
    ReorderDrop(&'static str),
    /// Alt+↑/↓ moves the last focused row.
    ReorderKey(i32),
    /// Accordion toggled: `(key, now open, message that flips the state)`.
    Disclose(String, bool, Box<Message>),
    /// Open (`Some`) or close a popover form.
    Popover(Option<&'static str>),
    /// Save a popover form: close it and run the message.
    PopoverSave(Box<Message>),
    /// A faded scroll list moved: `(list, relative offset, overflows)`.
    Scrolled(&'static str, f32, bool),
}

/// An in-progress reorder drag (or the keyboard-focused row).
#[derive(Debug, Clone, Copy)]
struct Reorder {
    list: &'static str,
    from: usize,
    target: usize,
    dragging: bool,
    make: fn(usize, usize) -> Message,
    /// Eased drop indicator position, in rows.
    indicator: Tween,
}

#[derive(Debug, Clone)]
struct HoldPress {
    key: String,
    started: Instant,
    action: Box<Message>,
}

/// An eased number display: tweens progress 0→1 and lerps `f64` endpoints
/// so large values (bytes) keep their precision.
#[derive(Debug, Clone, Copy)]
pub struct Ticker {
    from: f64,
    to: f64,
    progress: Tween,
}

impl Ticker {
    fn settled(value: f64) -> Self {
        Self {
            from: value,
            to: value,
            progress: Tween::settled(1.0),
        }
    }

    pub fn value(&self, now: Instant) -> f64 {
        let t = f64::from(self.progress.value(now));
        self.from + (self.to - self.from) * t
    }

    fn set(&mut self, value: f64, now: Instant, reduce_motion: bool, easing: Easing) {
        if value == self.to {
            return;
        }
        self.from = self.value(now);
        self.to = value;
        self.progress = Tween::settled(0.0).with_easing(easing);
        self.progress
            .animate_to(1.0, now, TICKER_DURATION, reduce_motion);
    }
}

#[derive(Debug, Default)]
pub struct UiState {
    pub palette: command_palette::PaletteState,
    hold: Option<HoldPress>,
    armed: Option<(String, Instant)>,
    island: Tween,
    pub island_hovered: bool,
    dock: BTreeMap<&'static str, f32>,
    pub spotlight: Option<(String, Point)>,
    tickers: BTreeMap<&'static str, Ticker>,
    /// Origin for continuous ambient effects (skeleton pulse, beam, embers).
    epoch: Option<Instant>,
    /// Last successful save, keyed by editor ("text", "level").
    saved_at: BTreeMap<&'static str, Instant>,
    saving: BTreeMap<&'static str, bool>,
    /// Folder chevron turn (0 closed → 1 open), keyed by folder path.
    folders: BTreeMap<String, Tween>,
    /// Segmented-control highlight position (segment index as f32).
    segments: BTreeMap<&'static str, Tween>,
    /// Switch knob position (0 off → 1 on).
    switches: BTreeMap<String, Tween>,
    /// Copy buttons show ✓ for a moment after copying.
    copied: Option<(String, Instant)>,
    /// Image comparison split (0 = all replacement, 1 = all original).
    pub compare_split: Option<f32>,
    /// Window currently hovered by dragged files.
    pub drop_target: Option<iced::window::Id>,
    reorder: Option<Reorder>,
    /// Accordion openness (0 closed → 1 open), keyed by section.
    disclosures: BTreeMap<String, Tween>,
    /// Storage ring sweep-in after an analysis completes.
    ring: Tween,
    /// Open popover form, by id.
    pub popover: Option<&'static str>,
    /// Last scroll position per faded list: `(relative y, overflows)`.
    scrolls: BTreeMap<&'static str, (f32, bool)>,
    /// When the Legacy title scramble started (once per page change).
    scramble_start: Option<Instant>,
}

impl UiState {
    /// Seconds since the ambient clock started; used by looping effects.
    pub fn clock(&self, now: Instant) -> f32 {
        self.epoch
            .map(|start| now.saturating_duration_since(start).as_secs_f32())
            .unwrap_or_default()
    }

    pub fn start_clock(&mut self) {
        self.epoch.get_or_insert_with(Instant::now);
    }

    /// Ease a folder chevron toward open or closed.
    pub fn turn_folder(&mut self, path: &str, open: bool, now: Instant, reduce: bool) {
        let tween = self
            .folders
            .entry(path.to_owned())
            .or_insert_with(|| Tween::settled(if open { 0.0 } else { 1.0 }));
        *tween = tween.with_easing(Easing::EaseOut);
        tween.animate_to(if open { 1.0 } else { 0.0 }, now, FOLDER_DURATION, reduce);
    }

    /// Chevron turn for a folder; unknown folders follow `open`.
    pub fn folder_turn(&self, path: &str, open: bool, now: Instant) -> f32 {
        self.folders
            .get(path)
            .map_or(if open { 1.0 } else { 0.0 }, |tween| tween.value(now))
    }

    /// Highlight position for a segmented control. A selection changed
    /// elsewhere (palette, keyboard) shows immediately.
    pub fn segment_position(&self, id: &str, selected: usize, now: Instant) -> f32 {
        match self.segments.get(id) {
            Some(tween) if tween.to == selected as f32 => tween.value(now),
            _ => selected as f32,
        }
    }

    pub fn switch_position(&self, key: &str, value: bool, now: Instant) -> f32 {
        let target = if value { 1.0 } else { 0.0 };
        match self.switches.get(key) {
            Some(tween) if tween.to == target => tween.value(now),
            _ => target,
        }
    }

    pub fn mark_copied(&mut self, id: String) {
        self.copied = Some((id, Instant::now()));
    }

    pub fn recently_copied(&self, id: &str, now: Instant) -> bool {
        self.copied.as_ref().is_some_and(|(copied, at)| {
            copied == id && now.saturating_duration_since(*at) < COPIED_FOR
        })
    }

    /// `(from, target, indicator rows)` while `list` is being dragged.
    pub fn reorder_state(&self, list: &str, now: Instant) -> Option<(usize, usize, f32)> {
        self.reorder
            .filter(|drag| drag.list == list && drag.dragging)
            .map(|drag| (drag.from, drag.target, drag.indicator.value(now)))
    }

    /// True while a tween-driven element needs ~60 fps frames.
    pub fn animating(&self, now: Instant) -> bool {
        if self.scrambling(now) {
            return true;
        }
        if self
            .reorder
            .is_some_and(|drag| drag.indicator.is_running(now))
            || self.disclosures.values().any(|tween| tween.is_running(now))
            || self.ring.is_running(now)
        {
            return true;
        }
        self.segments.values().any(|tween| tween.is_running(now))
            || self.switches.values().any(|tween| tween.is_running(now))
            // The ✓ needs a frame to revert after COPIED_FOR.
            || self.copied.as_ref().is_some_and(|(_, at)| now.saturating_duration_since(*at) < COPIED_FOR)
            || self.folders.values().any(|tween| tween.is_running(now))
            || self.hold.is_some()
            || self.island.is_running(now)
            || self
                .tickers
                .values()
                .any(|ticker| ticker.progress.is_running(now))
    }

    /// Hold progress 0..=1 for the button with `key`.
    pub fn hold_progress(&self, key: &str, now: Instant) -> f32 {
        self.hold
            .as_ref()
            .filter(|hold| hold.key == key)
            .map(|hold| {
                now.saturating_duration_since(hold.started).as_secs_f32()
                    / HOLD_DURATION.as_secs_f32()
            })
            .unwrap_or_default()
            .clamp(0.0, 1.0)
    }

    pub fn is_armed(&self, key: &str, now: Instant) -> bool {
        self.armed.as_ref().is_some_and(|(armed, at)| {
            armed == key && now.saturating_duration_since(*at) < ARM_WINDOW
        })
    }

    /// Called every animation frame: fires a completed hold and settles
    /// finished tweens. Returns the confirmed action, if any.
    pub fn frame(&mut self, now: Instant) -> Option<Message> {
        self.island.tick(now);
        // Settled chevrons need no entry: `folder_turn` falls back to state.
        self.folders.retain(|_, tween| tween.tick(now));
        self.switches.retain(|_, tween| tween.tick(now));
        self.disclosures.retain(|_, tween| tween.tick(now));
        for tween in self.segments.values_mut() {
            tween.tick(now);
        }
        if self
            .copied
            .as_ref()
            .is_some_and(|(_, at)| now.saturating_duration_since(*at) >= COPIED_FOR)
        {
            self.copied = None;
        }
        for ticker in self.tickers.values_mut() {
            ticker.progress.tick(now);
        }
        if self
            .armed
            .as_ref()
            .is_some_and(|(_, at)| now.saturating_duration_since(*at) >= ARM_WINDOW)
        {
            self.armed = None;
        }
        let done = self
            .hold
            .as_ref()
            .is_some_and(|hold| now.saturating_duration_since(hold.started) >= HOLD_DURATION);
        if done {
            return self.hold.take().map(|hold| *hold.action);
        }
        None
    }

    /// Start the split-flap title effect (Legacy only, skipped with
    /// reduce motion by the caller).
    pub fn start_scramble(&mut self, now: Instant) {
        self.scramble_start = Some(now);
    }

    pub fn scrambling(&self, now: Instant) -> bool {
        self.scramble_start
            .is_some_and(|start| now.saturating_duration_since(start) < SCRAMBLE_TOTAL)
    }

    /// `value` as currently resolved by the scramble.
    pub fn scrambled(&self, value: &str, now: Instant) -> String {
        match self.scramble_start {
            Some(start) if self.scrambling(now) => {
                scramble(value, now.saturating_duration_since(start))
            }
            _ => value.to_owned(),
        }
    }

    /// Sweep the storage ring in from empty.
    pub fn sweep_ring(&mut self, now: Instant, reduce: bool) {
        self.ring = Tween::settled(0.0).with_easing(Easing::EaseOut);
        self.ring
            .animate_to(1.0, now, Duration::from_millis(700), reduce);
    }

    /// Ring sweep 0..1 (1 when no sweep has run).
    pub fn ring_sweep(&self, now: Instant) -> f32 {
        if self.ring.start.is_none() && self.ring.to == 0.0 {
            1.0
        } else {
            self.ring.value(now)
        }
    }

    pub fn island_value(&self, now: Instant) -> f32 {
        self.island.value(now)
    }

    pub fn dock_cursor(&self, dock: &str) -> Option<f32> {
        self.dock.get(dock).copied()
    }

    pub fn ticker(&self, key: &str, now: Instant) -> Option<f64> {
        self.tickers.get(key).map(|ticker| ticker.value(now))
    }

    /// Retarget a number ticker. The first value is shown immediately.
    pub fn set_ticker(&mut self, key: &'static str, value: f64, now: Instant, reduce: bool) {
        // Live counters (worker progress) count steadily; totals settle.
        let easing = if key == "import_files" {
            Easing::Linear
        } else {
            Easing::EaseOut
        };
        self.tickers
            .entry(key)
            .and_modify(|ticker| ticker.set(value, now, reduce, easing))
            .or_insert_with(|| Ticker::settled(value));
    }

    pub fn clear_ticker(&mut self, key: &str) {
        self.tickers.remove(key);
    }

    pub fn mark_saving(&mut self, editor: &'static str) {
        self.saving.insert(editor, true);
    }

    pub fn mark_saved(&mut self, editor: &'static str, ok: bool) {
        self.saving.insert(editor, false);
        if ok {
            self.saved_at.insert(editor, Instant::now());
        }
    }

    pub fn save_info(&self, editor: &str) -> (bool, Option<Instant>) {
        (
            self.saving.get(editor).copied().unwrap_or(false),
            self.saved_at.get(editor).copied(),
        )
    }
}

/// Handle a [`UiMsg`]. Returns a follow-up message (a confirmed action or
/// a palette command) for the caller to dispatch.
pub fn update(ui: &mut UiState, message: UiMsg, reduce_motion: bool) -> Option<Message> {
    let now = Instant::now();
    match message {
        // The palette needs the whole `State` to build its command list, so
        // `Desktop::update` routes these to `command_palette::update`.
        UiMsg::Palette(_) => {}
        UiMsg::HoldPress(key, action) => {
            ui.hold = Some(HoldPress {
                key,
                started: now,
                action,
            });
        }
        UiMsg::HoldRelease => ui.hold = None,
        UiMsg::ArmConfirm(key, action) => {
            if ui.is_armed(&key, now) {
                ui.armed = None;
                return Some(*action);
            }
            ui.armed = Some((key, now));
        }
        UiMsg::IslandHover(hovered) => {
            ui.island_hovered = hovered;
            ui.island.animate_to(
                if hovered { 1.0 } else { 0.0 },
                now,
                ISLAND_DURATION,
                reduce_motion,
            );
        }
        UiMsg::DockMove(dock, x) => {
            ui.dock.insert(dock, x);
        }
        UiMsg::DockExit(dock) => {
            ui.dock.remove(dock);
        }
        UiMsg::Spotlight(key, point) => {
            ui.spotlight = point.map(|point| (key, point));
        }
        UiMsg::CompareSet(fraction) => ui.compare_split = Some(fraction.clamp(0.0, 1.0)),
        UiMsg::ReorderGrab(list, index, make) => {
            ui.reorder = Some(Reorder {
                list,
                from: index,
                target: index,
                dragging: true,
                make,
                indicator: Tween::settled(index as f32).with_easing(Easing::EaseOut),
            });
        }
        UiMsg::ReorderMove(list, y) => {
            if let Some(drag) = ui.reorder.as_mut().filter(|d| d.list == list && d.dragging) {
                let target = (y / REORDER_ROW).floor().max(0.0) as usize;
                if target != drag.target {
                    drag.target = target;
                    drag.indicator
                        .animate_to(target as f32, now, REORDER_SLIDE, reduce_motion);
                }
            }
        }
        UiMsg::ReorderDrop(list) => {
            if let Some(drag) = ui.reorder.as_mut().filter(|d| d.list == list && d.dragging) {
                drag.dragging = false;
                let (from, to) = (drag.from, drag.target);
                // Keep keyboard focus on the moved row.
                drag.from = to;
                if from != to {
                    return Some((drag.make)(from, to));
                }
            }
        }
        UiMsg::Disclose(key, open, message) => {
            let tween = ui
                .disclosures
                .entry(key)
                .or_insert_with(|| Tween::settled(if open { 0.0 } else { 1.0 }));
            *tween = tween.with_easing(Easing::EaseOut);
            tween.animate_to(
                if open { 1.0 } else { 0.0 },
                now,
                DISCLOSE_DURATION,
                reduce_motion,
            );
            return Some(*message);
        }
        UiMsg::Popover(id) => ui.popover = id,
        UiMsg::Scrolled(list, offset, overflows) => {
            ui.scrolls.insert(list, (offset, overflows));
        }
        UiMsg::PopoverSave(message) => {
            ui.popover = None;
            return Some(*message);
        }
        UiMsg::ReorderKey(delta) => {
            if let Some(drag) = ui.reorder.as_mut() {
                let from = drag.from;
                let to = (from as i64 + i64::from(delta)).max(0) as usize;
                if to != from {
                    drag.from = to;
                    return Some((drag.make)(from, to));
                }
            }
        }
        UiMsg::CompareNudge(delta) => {
            let split = ui.compare_split.unwrap_or(0.5);
            ui.compare_split = Some((split + delta).clamp(0.0, 1.0));
        }
        UiMsg::Segment(id, index, message) => {
            let tween = ui
                .segments
                .entry(id)
                .or_insert_with(|| Tween::settled(index as f32));
            // Width-relative travel: one segment step is one "unit".
            let distance = (tween.value(now) - index as f32).abs().max(1.0);
            *tween = tween.with_easing(Easing::EaseOut);
            tween.animate_to(
                index as f32,
                now,
                SEGMENT_DURATION.mul_f32(distance.min(3.0)),
                reduce_motion,
            );
            return Some(*message);
        }
        UiMsg::Switch(key, value, message) => {
            let tween = ui
                .switches
                .entry(key)
                .or_insert_with(|| Tween::settled(if value { 0.0 } else { 1.0 }));
            *tween = tween.with_easing(Easing::EaseInOut);
            tween.animate_to(
                if value { 1.0 } else { 0.0 },
                now,
                SWITCH_DURATION,
                reduce_motion,
            );
            return Some(*message);
        }
    }
    None
}

fn ui(message: UiMsg) -> Message {
    Message::Ui(message)
}

// ---------------------------------------------------------------------------
// Callout
// ---------------------------------------------------------------------------

fn tone_glyph(tone: Tone) -> &'static str {
    match tone {
        Tone::Info => "i",
        Tone::Success => "✓",
        Tone::Warning => "!",
        Tone::Danger => "!",
    }
}

/// Inline alert: tinted panel, coloured left bar and an icon badge.
pub fn callout<'a>(tone: Tone, body: impl text::IntoFragment<'a>) -> Element<'a, Message> {
    callout_with(tone, None, body)
}

fn callout_with<'a>(
    tone: Tone,
    title: Option<&'a str>,
    body: impl text::IntoFragment<'a>,
) -> Element<'a, Message> {
    let bar =
        container(space().width(3).height(Fill)).style(move |theme: &Theme| container::Style {
            background: Some(Background::Color(design::tone_color(theme, tone))),
            border: Border {
                radius: design::radius(theme, Radius::Small).into(),
                ..Border::default()
            },
            ..container::Style::default()
        });
    let badge =
        container(
            text(tone_glyph(tone))
                .size(11)
                .style(move |theme: &Theme| text::Style {
                    color: Some(design::tone_color(theme, tone)),
                }),
        )
        .center_x(18)
        .center_y(18)
        .style(move |theme: &Theme| container::Style {
            border: Border {
                color: design::tone_color(theme, tone),
                width: 1.0,
                radius: design::radius(theme, Radius::Pill).into(),
            },
            ..container::Style::default()
        });
    let mut copy = column![].spacing(3).width(Fill);
    if let Some(title) = title {
        copy = copy.push(text(title).size(13));
    }
    copy = copy.push(text(body).size(12));
    container(
        row![bar, badge, copy]
            .spacing(10)
            .align_y(Alignment::Start)
            .height(Length::Shrink),
    )
    .padding([9, 12])
    .width(Fill)
    .style(move |theme: &Theme| design::tinted(theme, tone))
    .into()
}

// ---------------------------------------------------------------------------
// Empty state
// ---------------------------------------------------------------------------

/// Centered illustration-free empty state with an optional fix-it action.
pub fn empty_state<'a>(
    icon: Option<svg::Handle>,
    title: &'a str,
    body: &'a str,
    action: Option<(&'a str, Message)>,
) -> Element<'a, Message> {
    let glyph: Element<'a, Message> = match icon {
        Some(handle) => svg(handle).width(28).height(28).style(design::icon).into(),
        None => text("○").size(24).into(),
    };
    let ring = container(glyph)
        .center_x(56)
        .center_y(56)
        .style(|theme: &Theme| container::Style {
            background: Some(Background::Color(design::field(theme))),
            border: Border {
                color: design::border(theme),
                width: 1.0,
                radius: design::radius(theme, Radius::Pill).into(),
            },
            ..container::Style::default()
        });
    let mut content = column![
        ring,
        text(title).size(16),
        text(body)
            .size(12)
            .style(design::muted)
            .align_x(iced::alignment::Horizontal::Center),
    ]
    .spacing(10)
    .align_x(Alignment::Center)
    .max_width(420);
    if let Some((label, message)) = action {
        content = content.push(
            design::button(label)
                .style(design::primary)
                .on_press(message),
        );
    }
    container(content)
        .center_x(Fill)
        .padding([28, 16])
        .width(Fill)
        .into()
}

// ---------------------------------------------------------------------------
// Save status pill
// ---------------------------------------------------------------------------

pub fn save_label(dirty: bool, saving: bool, saved_at: Option<Instant>, now: Instant) -> String {
    if saving {
        return "Saving…".into();
    }
    if dirty {
        return "Unsaved changes".into();
    }
    match saved_at {
        Some(at) => {
            let seconds = now.saturating_duration_since(at).as_secs();
            match seconds {
                0..=4 => "Saved just now".into(),
                5..=59 => format!("Saved {seconds} s ago"),
                60..=3599 => format!("Saved {} min ago", seconds / 60),
                _ => format!("Saved {} h ago", seconds / 3600),
            }
        }
        None => "No changes".into(),
    }
}

/// "Unsaved changes • Saving… • Saved 12 s ago" pill with a status dot.
pub fn save_status<'a>(
    dirty: bool,
    saving: bool,
    saved_at: Option<Instant>,
) -> Element<'a, Message> {
    let now = Instant::now();
    let tone = if saving {
        Tone::Info
    } else if dirty {
        Tone::Warning
    } else {
        Tone::Success
    };
    let dot = container(space().width(7).height(7)).style(move |theme: &Theme| container::Style {
        background: Some(Background::Color(design::tone_color(theme, tone))),
        border: Border {
            radius: design::radius(theme, Radius::Pill).into(),
            ..Border::default()
        },
        ..container::Style::default()
    });
    container(
        row![dot, text(save_label(dirty, saving, saved_at, now)).size(11)]
            .spacing(6)
            .align_y(Alignment::Center),
    )
    .padding([3, 10])
    .style(design::pill)
    .into()
}

// ---------------------------------------------------------------------------
// Skeleton
// ---------------------------------------------------------------------------

/// Pulse opacity for skeleton placeholders at clock `t` seconds.
pub fn pulse(t: f32, reduce_motion: bool) -> f32 {
    if reduce_motion {
        return 0.55;
    }
    let phase = (t / PULSE_PERIOD).fract();
    let wave = 0.5 - 0.5 * (phase * std::f32::consts::TAU).cos();
    0.35 + 0.4 * wave
}

/// A placeholder block whose opacity pulses while content loads.
pub fn skeleton<'a>(width: impl Into<Length>, height: f32, alpha: f32) -> Element<'a, Message> {
    container(space().height(height))
        .width(width)
        .height(height)
        .style(move |theme: &Theme| container::Style {
            background: Some(Background::Color(design::with_alpha(
                design::border(theme),
                alpha,
            ))),
            border: Border {
                radius: design::radius(theme, Radius::Small).into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

// ---------------------------------------------------------------------------
// Timeline
// ---------------------------------------------------------------------------

pub struct TimelineItem<'a> {
    pub title: String,
    pub meta: String,
    pub body: Option<Element<'a, Message>>,
    pub tone: Option<Tone>,
}

/// Vertical line with dots and content cards to one side.
pub fn timeline<'a>(items: Vec<TimelineItem<'a>>) -> Element<'a, Message> {
    let count = items.len();
    let mut list = column![].spacing(0);
    for (index, item) in items.into_iter().enumerate() {
        let tone = item.tone;
        let last = index + 1 == count;
        let dot = container(space().width(10).height(10)).style(move |theme: &Theme| {
            let color = tone.map_or(theme.palette().primary, |tone| {
                design::tone_color(theme, tone)
            });
            container::Style {
                background: Some(Background::Color(color)),
                border: Border {
                    color: design::surface(theme),
                    width: 2.0,
                    radius: design::radius(theme, Radius::Pill).into(),
                },
                ..container::Style::default()
            }
        });
        let line: Element<'a, Message> = if last {
            space().width(2).height(Fill).into()
        } else {
            container(space().width(2).height(Fill))
                .style(|theme: &Theme| container::Style {
                    background: Some(Background::Color(design::border(theme))),
                    ..container::Style::default()
                })
                .into()
        };
        let rail = column![dot, line]
            .align_x(Alignment::Center)
            .width(14)
            .height(Fill)
            .spacing(2);
        let mut card = column![
            row![
                text(item.title).size(13).width(Fill),
                text(item.meta).size(11).style(design::muted)
            ]
            .spacing(8)
        ]
        .spacing(6);
        if let Some(body) = item.body {
            card = card.push(body);
        }
        list = list.push(
            row![
                rail,
                container(card)
                    .padding(iced::Padding {
                        top: 0.0,
                        right: 0.0,
                        bottom: if last { 0.0 } else { 16.0 },
                        left: 0.0,
                    })
                    .width(Fill)
            ]
            .spacing(12)
            .height(Length::Shrink),
        );
    }
    list.into()
}

// ---------------------------------------------------------------------------
// Stepper / multi-step loader
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    Done,
    Current,
    Waiting,
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Steps {
    pub labels: Vec<&'static str>,
    pub current: usize,
    pub failed: bool,
}

impl Steps {
    pub fn new(labels: &[&'static str], current: usize) -> Self {
        Self {
            labels: labels.to_vec(),
            current: current.min(labels.len().saturating_sub(1)),
            failed: false,
        }
    }

    pub fn state(&self, index: usize) -> StepState {
        match index.cmp(&self.current) {
            std::cmp::Ordering::Less => StepState::Done,
            std::cmp::Ordering::Equal if self.failed => StepState::Failed,
            std::cmp::Ordering::Equal => StepState::Current,
            std::cmp::Ordering::Greater => StepState::Waiting,
        }
    }

    pub fn all_done(labels: &[&'static str]) -> Self {
        Self {
            labels: labels.to_vec(),
            current: labels.len(),
            failed: false,
        }
    }
}

/// ✓ done, loading wheel for the current step, ○ waiting, ✕ failed.
pub fn stepper<'a>(steps: &Steps, wheel_frame: u32) -> Element<'a, Message> {
    let mut list = column![].spacing(7);
    for (index, label) in steps.labels.iter().enumerate() {
        let status = steps.state(index);
        let marker: Element<'a, Message> = match status {
            StepState::Current => svg(crate::loading_wheel::handle(wheel_frame, 1.0))
                .width(18)
                .height(18)
                .into(),
            other => {
                let (glyph, tone) = match other {
                    StepState::Done => ("✓", Some(Tone::Success)),
                    StepState::Failed => ("✕", Some(Tone::Danger)),
                    _ => ("○", None),
                };
                container(
                    text(glyph)
                        .size(13)
                        .style(move |theme: &Theme| text::Style {
                            color: Some(tone.map_or(
                                theme.extended_palette().background.strongest.text,
                                |tone| design::tone_color(theme, tone),
                            )),
                        }),
                )
                .center_x(18)
                .center_y(18)
                .into()
            }
        };
        let label = text(*label)
            .size(12)
            .style(move |theme: &Theme| text::Style {
                color: Some(match status {
                    StepState::Waiting => theme.extended_palette().background.strongest.text,
                    StepState::Failed => theme.palette().danger,
                    _ => theme.palette().text,
                }),
            });
        list = list.push(row![marker, label].spacing(9).align_y(Alignment::Center));
    }
    list.into()
}

// ---------------------------------------------------------------------------
// Number ticker
// ---------------------------------------------------------------------------

pub fn format_bytes(bytes: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes.max(0.0);
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// The ticker's eased value, or `fallback` when it has not been set yet.
pub fn ticked(ui: &UiState, key: &str, fallback: f64) -> f64 {
    ui.ticker(key, Instant::now()).unwrap_or(fallback)
}

// ---------------------------------------------------------------------------
// Hold to confirm
// ---------------------------------------------------------------------------

/// A destructive action that fires after holding the button ~0.9 s. With
/// reduce motion on, it becomes a two-click confirm instead.
pub fn hold_button<'a>(
    ui_state: &UiState,
    key: &str,
    label: &'a str,
    action: Message,
    enabled: bool,
    reduce_motion: bool,
) -> Element<'a, Message> {
    let now = Instant::now();
    if reduce_motion {
        let armed = ui_state.is_armed(key, now);
        return design::button(text(if armed {
            "Click again to confirm"
        } else {
            label
        }))
        .style(if armed {
            design::primary
        } else {
            design::secondary
        })
        .on_press_maybe(enabled.then(|| ui_arm(key, action)))
        .into();
    }
    let progress = ui_state.hold_progress(key, now);
    let holding = progress > 0.0;
    let caption = if holding { "Keep holding…" } else { label };
    let fill_portion = ((progress * 1000.0) as u16).max(1);
    let rest_portion = (((1.0 - progress) * 1000.0) as u16).max(1);
    let fill = row![
        container(space().height(Fill))
            .width(Length::FillPortion(fill_portion))
            .style(move |theme: &Theme| container::Style {
                background: Some(Background::Color(design::with_alpha(
                    theme.palette().danger,
                    if holding { 0.55 } else { 0.0 },
                ))),
                ..container::Style::default()
            }),
        space().width(Length::FillPortion(rest_portion)),
    ]
    .height(Fill);
    let face = container(
        text(caption)
            .size(13)
            .style(move |theme: &Theme| text::Style {
                color: Some(if enabled {
                    theme.palette().text
                } else {
                    theme.extended_palette().background.strongest.text
                }),
            }),
    )
    .center_x(Fill)
    .center_y(Fill);
    let body = container(iced::widget::stack![fill, face])
        .width(Length::Shrink)
        .height(34)
        .padding([0, 0])
        .clip(true)
        .style(move |theme: &Theme| container::Style {
            background: Some(Background::Color(design::field(theme))),
            border: Border {
                color: if holding {
                    theme.palette().danger
                } else {
                    design::border(theme)
                },
                width: 1.0,
                radius: design::radius(theme, Radius::Medium).into(),
            },
            ..container::Style::default()
        });
    // Give the stack an intrinsic width from the label.
    let sized = container(body).width(Length::Fixed(
        (label.chars().count().max(14) as f32) * 7.4 + 28.0,
    ));
    if !enabled {
        return sized.into();
    }
    mouse_area(sized)
        .on_press(ui(UiMsg::HoldPress(key.to_owned(), Box::new(action))))
        .on_release(ui(UiMsg::HoldRelease))
        .on_exit(ui(UiMsg::HoldRelease))
        .interaction(iced::mouse::Interaction::Pointer)
        .into()
}

fn ui_arm(key: &str, action: Message) -> Message {
    ui(UiMsg::ArmConfirm(key.to_owned(), Box::new(action)))
}

// ---------------------------------------------------------------------------
// Bento grid
// ---------------------------------------------------------------------------

/// A titled dashboard tile. Combine in rows with `FillPortion` widths.
pub fn bento_tile<'a>(
    title: &'a str,
    content: impl Into<Element<'a, Message>>,
    portion: u16,
) -> Element<'a, Message> {
    container(column![text(title).size(11).style(design::muted), content.into()].spacing(8))
        .padding(14)
        .width(Length::FillPortion(portion))
        .height(Fill)
        .style(design::panel)
        .into()
}

// ---------------------------------------------------------------------------
// Status island
// ---------------------------------------------------------------------------

/// Footer pill showing the current operation and elapsed time. Hovering it
/// opens [`island_panel`], which the page lays over itself with `stack!`.
pub fn status_island<'a>(
    label: String,
    elapsed: Option<Duration>,
    wheel_frame: u32,
    working: bool,
) -> Element<'a, Message> {
    let mut head = row![].spacing(8).align_y(Alignment::Center);
    if working {
        head = head.push(
            svg(crate::loading_wheel::handle(wheel_frame, 1.0))
                .width(16)
                .height(16),
        );
    }
    head = head.push(text(label).size(11));
    if let Some(elapsed) = elapsed {
        let seconds = elapsed.as_secs();
        head = head.push(
            text(format!("{}:{:02}", seconds / 60, seconds % 60))
                .size(11)
                .style(design::muted),
        );
    }
    let island = container(head).padding([2, 12]).style(design::pill);
    mouse_area(island)
        .on_enter(ui(UiMsg::IslandHover(true)))
        .on_exit(ui(UiMsg::IslandHover(false)))
        .into()
}

/// The expanded island: recent activity entries that grow in on hover.
/// Returns `None` while closed so nothing is layered over the page.
pub fn island_panel<'a>(ui_state: &UiState, recent: Vec<String>) -> Option<Element<'a, Message>> {
    let open = ui_state.island_value(Instant::now());
    if open <= 0.02 || recent.is_empty() {
        return None;
    }
    let shown = ((recent.len() as f32) * open).ceil() as usize;
    let mut lines = column![text("RECENT ACTIVITY").size(10).style(design::muted)].spacing(4);
    for line in recent.into_iter().rev().take(shown.max(1)) {
        lines = lines.push(text(line).size(11).style(move |theme: &Theme| text::Style {
            color: Some(design::with_alpha(theme.palette().text, open)),
        }));
    }
    let panel = container(lines)
        .padding([10, 14])
        .width(Length::Fixed(260.0 + 160.0 * open))
        .style(move |theme: &Theme| {
            let mut style = design::panel(theme);
            style.shadow = iced::Shadow {
                color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.35 * open),
                offset: iced::Vector::new(0.0, 6.0),
                blur_radius: 18.0,
            };
            style
        });
    Some(
        mouse_area(panel)
            .on_enter(ui(UiMsg::IslandHover(true)))
            .on_exit(ui(UiMsg::IslandHover(false)))
            .into(),
    )
}

// ---------------------------------------------------------------------------
// Segmented control
// ---------------------------------------------------------------------------

/// Equal-width segments with a highlight that slides to the selection.
/// The highlight is laid out with `FillPortion`s, so it needs no pixel
/// measurement: left gap = position, highlight = 1, right gap = rest.
pub fn segmented<'a>(
    ui_state: &UiState,
    id: &'static str,
    options: Vec<(&'a str, Message)>,
    selected: usize,
    enabled: bool,
) -> Element<'a, Message> {
    const UNIT: f32 = 1000.0;
    let count = options.len().max(1);
    let position = ui_state
        .segment_position(id, selected, Instant::now())
        .clamp(0.0, (count - 1) as f32);
    let left = (position * UNIT) as u16;
    let right = (((count - 1) as f32 - position) * UNIT) as u16;
    let mut track = row![].height(Fill);
    if left > 0 {
        track = track.push(space().width(Length::FillPortion(left)));
    }
    track = track.push(
        container(space())
            .width(Length::FillPortion(UNIT as u16))
            .height(Fill)
            .style(|theme: &Theme| container::Style {
                background: Some(Background::Color(theme.palette().primary)),
                border: Border {
                    radius: design::radius(theme, Radius::Medium).into(),
                    ..Border::default()
                },
                ..container::Style::default()
            }),
    );
    if right > 0 {
        track = track.push(space().width(Length::FillPortion(right)));
    }
    let mut buttons = row![].height(Fill);
    for (index, (label, message)) in options.into_iter().enumerate() {
        let active = index == selected;
        buttons = buttons.push(
            iced::widget::button(
                container(
                    text(label)
                        .size(12)
                        .style(move |theme: &Theme| text::Style {
                            color: Some(if active {
                                theme.extended_palette().primary.base.text
                            } else {
                                theme.palette().text
                            }),
                        }),
                )
                .center_x(Fill)
                .center_y(Fill),
            )
            .width(Length::FillPortion(1))
            .height(Fill)
            .padding(0)
            .style(|_, _| iced::widget::button::Style::default())
            .on_press_maybe(enabled.then(|| ui(UiMsg::Segment(id, index, Box::new(message))))),
        );
    }
    container(iced::widget::stack![track, buttons])
        .padding(3)
        .height(34)
        .width(Length::Fixed(110.0 * count as f32))
        .style(|theme: &Theme| container::Style {
            background: Some(Background::Color(design::field(theme))),
            border: Border {
                color: design::border(theme),
                width: 1.0,
                radius: design::radius(theme, Radius::Medium).into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// One tab of a [`segmented_tabs`] row.
pub struct TabItem<'a> {
    pub icon: svg::Handle,
    pub label: &'a str,
    /// A status dot after the label.
    pub dot: Option<Tone>,
    pub message: Message,
}

/// Segmented tabs with an icon and optional status dot per tab, sharing
/// [`segmented`]'s sliding highlight.
pub fn segmented_tabs<'a>(
    ui_state: &UiState,
    id: &'static str,
    items: Vec<TabItem<'a>>,
    selected: usize,
    width: f32,
) -> Element<'a, Message> {
    const UNIT: f32 = 1000.0;
    let count = items.len().max(1);
    let position = ui_state
        .segment_position(id, selected, Instant::now())
        .clamp(0.0, (count - 1) as f32);
    let left = (position * UNIT) as u16;
    let right = (((count - 1) as f32 - position) * UNIT) as u16;
    let mut track = row![].height(Fill);
    if left > 0 {
        track = track.push(space().width(Length::FillPortion(left)));
    }
    track = track.push(
        container(space())
            .width(Length::FillPortion(UNIT as u16))
            .height(Fill)
            .style(|theme: &Theme| container::Style {
                background: Some(Background::Color(theme.palette().primary)),
                border: Border {
                    radius: design::radius(theme, Radius::Medium).into(),
                    ..Border::default()
                },
                ..container::Style::default()
            }),
    );
    if right > 0 {
        track = track.push(space().width(Length::FillPortion(right)));
    }
    let mut buttons = row![].height(Fill);
    for (index, item) in items.into_iter().enumerate() {
        let active = index == selected;
        let ink = move |theme: &Theme| {
            if active {
                theme.extended_palette().primary.base.text
            } else {
                theme.palette().text
            }
        };
        let mut content = row![
            svg(item.icon)
                .width(15)
                .height(15)
                .style(move |theme: &Theme, _| svg::Style {
                    color: Some(ink(theme)),
                }),
            text(item.label)
                .size(12)
                .style(move |theme: &Theme| text::Style {
                    color: Some(ink(theme)),
                }),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        if let Some(tone) = item.dot {
            content = content.push(
                container(space().width(7).height(7)).style(move |theme: &Theme| {
                    container::Style {
                        background: Some(Background::Color(design::tone_color(theme, tone))),
                        border: Border {
                            radius: 4.0.into(),
                            color: theme.palette().background,
                            width: 1.0,
                        },
                        ..container::Style::default()
                    }
                }),
            );
        }
        buttons = buttons.push(
            iced::widget::button(container(content).center_x(Fill).center_y(Fill))
                .width(Length::FillPortion(1))
                .height(Fill)
                .padding(0)
                .style(|_, _| iced::widget::button::Style::default())
                .on_press(ui(UiMsg::Segment(id, index, Box::new(item.message)))),
        );
    }
    container(iced::widget::stack![track, buttons])
        .padding(3)
        .height(36)
        .width(Length::Fixed(width * count as f32))
        .style(|theme: &Theme| container::Style {
            background: Some(Background::Color(design::field(theme))),
            border: Border {
                color: design::border(theme),
                width: 1.0,
                radius: design::radius(theme, Radius::Medium).into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// One button of an [`icon_segmented`] group.
pub struct IconToggle {
    pub icon: svg::Handle,
    pub label: &'static str,
    pub message: Message,
}

/// Icon-only segmented toggles (tool palettes). The selected slot gets the
/// same sliding highlight as [`segmented`]; each icon has a tooltip with its
/// name and shortcut from `chrome::SHORTCUTS`.
pub fn icon_segmented<'a>(
    ui_state: &UiState,
    id: &'static str,
    items: Vec<IconToggle>,
    selected: usize,
) -> Element<'a, Message> {
    const SLOT: f32 = 32.0;
    const UNIT: f32 = 1000.0;
    let count = items.len().max(1);
    let position = ui_state
        .segment_position(id, selected, Instant::now())
        .clamp(0.0, (count - 1) as f32);
    let left = (position * UNIT) as u16;
    let right = (((count - 1) as f32 - position) * UNIT) as u16;
    let mut track = row![].height(Fill);
    if left > 0 {
        track = track.push(space().width(Length::FillPortion(left)));
    }
    track = track.push(
        container(space())
            .width(Length::FillPortion(UNIT as u16))
            .height(Fill)
            .style(|theme: &Theme| container::Style {
                background: Some(Background::Color(theme.palette().primary)),
                border: Border {
                    radius: design::radius(theme, Radius::Small).into(),
                    ..Border::default()
                },
                ..container::Style::default()
            }),
    );
    if right > 0 {
        track = track.push(space().width(Length::FillPortion(right)));
    }
    let mut buttons = row![].height(Fill);
    for (index, item) in items.into_iter().enumerate() {
        let active = index == selected;
        let icon = svg(item.icon)
            .width(18)
            .height(18)
            .style(move |theme: &Theme, _| svg::Style {
                color: Some(if active {
                    theme.extended_palette().primary.base.text
                } else {
                    theme.palette().text
                }),
            });
        let button = iced::widget::button(container(icon).center_x(Fill).center_y(Fill))
            .width(SLOT)
            .height(Fill)
            .padding(0)
            .style(|_, _| iced::widget::button::Style::default())
            .on_press(ui(UiMsg::Segment(id, index, Box::new(item.message))));
        let label = match crate::chrome::shortcut_for(item.label) {
            Some(keys) => format!("{} · {keys}", item.label),
            None => item.label.to_owned(),
        };
        buttons = buttons.push(design::tooltip(
            button,
            label,
            None,
            iced::widget::tooltip::Position::Bottom,
        ));
    }
    container(iced::widget::stack![track, buttons])
        .padding(2)
        .height(34)
        .width(Length::Fixed(SLOT * count as f32 + 4.0))
        .style(|theme: &Theme| container::Style {
            background: Some(Background::Color(design::field(theme))),
            border: Border {
                color: design::border(theme),
                width: 1.0,
                radius: design::radius(theme, Radius::Medium).into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// A square icon button with a tooltip (name and shortcut). `None`
/// disables it; `reason` then explains why in the tooltip.
pub fn icon_button<'a>(
    icon: svg::Handle,
    label: &'static str,
    message: Option<Message>,
    reason: Option<&'static str>,
) -> Element<'a, Message> {
    let enabled = message.is_some();
    let glyph = svg(icon)
        .width(17)
        .height(17)
        .style(move |theme: &Theme, _| svg::Style {
            color: Some(if enabled {
                theme.palette().text
            } else {
                design::with_alpha(theme.palette().text, 0.35)
            }),
        });
    let button = iced::widget::button(container(glyph).center_x(30).center_y(30))
        .padding(0)
        .style(design::secondary)
        .on_press_maybe(message);
    let tip = match (enabled, reason) {
        (false, Some(reason)) => format!("{label} · {reason}"),
        _ => label.to_owned(),
    };
    design::tooltip(
        button,
        tip,
        crate::chrome::shortcut_for(label),
        iced::widget::tooltip::Position::Bottom,
    )
}

/// A labelled button with a leading icon.
pub fn icon_label_button<'a>(
    icon: svg::Handle,
    label: &'a str,
) -> iced::widget::Button<'a, Message> {
    iced::widget::button(
        row![
            svg(icon).width(16).height(16).style(design::icon),
            text(label).size(design::BODY),
        ]
        .spacing(7)
        .align_y(Alignment::Center),
    )
    .padding([7, 12])
}

/// Wraps a possibly-disabled control in a tooltip saying why it's disabled.
pub fn why_disabled<'a>(
    control: impl Into<Element<'a, Message>>,
    disabled: bool,
    reason: &'a str,
) -> Element<'a, Message> {
    if disabled {
        design::tooltip(control, reason, None, iced::widget::tooltip::Position::Top)
    } else {
        control.into()
    }
}

// ---------------------------------------------------------------------------
// Switch
// ---------------------------------------------------------------------------

/// Labelled on/off switch. The whole row is clickable; the 36×20 knob
/// slides across with a short animation.
pub fn switch<'a>(
    ui_state: &UiState,
    key: &str,
    label: impl text::IntoFragment<'a>,
    value: bool,
    message: Message,
) -> Element<'a, Message> {
    let position = ui_state.switch_position(key, value, Instant::now());
    let knob = iced::widget::canvas(crate::fx::SwitchKnob { position })
        .width(36)
        .height(20);
    iced::widget::button(
        row![knob, text(label).size(13)]
            .spacing(10)
            .align_y(Alignment::Center),
    )
    .padding([4, 6])
    .style(design::text_button)
    .on_press(ui(UiMsg::Switch(key.to_owned(), !value, Box::new(message))))
    .into()
}

// ---------------------------------------------------------------------------
// Image comparison slider
// ---------------------------------------------------------------------------

/// Original (left of the handle) over the replacement (right), split by a
/// handle you drag. Drawn on a canvas so the original is clipped, not
/// squeezed. Left/Right arrow keys nudge the split.
pub fn compare_slider<'a>(
    ui_state: &UiState,
    original: iced::widget::image::Handle,
    replacement: iced::widget::image::Handle,
) -> Element<'a, Message> {
    iced::widget::canvas(crate::fx::ImageCompare {
        original,
        replacement,
        split: ui_state.compare_split.unwrap_or(0.5),
    })
    .width(Fill)
    .height(Fill)
    .into()
}

// ---------------------------------------------------------------------------
// Drop zone
// ---------------------------------------------------------------------------

/// Layer the drop feedback over `base`: dashed border plus a label saying
/// what the drop will do (or why it is refused).
pub fn drop_zone<'a>(
    base: Element<'a, Message>,
    label: String,
    refused: bool,
) -> Element<'a, Message> {
    let tone = if refused { Tone::Danger } else { Tone::Info };
    let message = container(
        row![
            text(if refused { "⊘" } else { "⤓" }).size(20),
            text(label).size(14),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .padding([12, 18])
    .style(move |theme: &Theme| {
        let mut style = design::tinted(theme, tone);
        style.background = Some(Background::Color(design::surface(theme)));
        style
    });
    iced::widget::stack![
        base,
        iced::widget::canvas(crate::fx::DropZone { refused })
            .width(Fill)
            .height(Fill),
        container(message).center(Fill),
    ]
    .into()
}

// ---------------------------------------------------------------------------
// Sortable table header
// ---------------------------------------------------------------------------

/// Current sort: `(column id, ascending)`.
pub type Sort = Option<(&'static str, bool)>;

/// Flip direction on the active column, otherwise sort ascending by it.
pub fn next_sort(current: Sort, column: &'static str) -> Sort {
    match current {
        Some((active, ascending)) if active == column => Some((column, !ascending)),
        _ => Some((column, true)),
    }
}

/// A clickable column header with a ▲/▼ arrow on the active column.
pub fn sort_header<'a>(
    label: &'a str,
    column: &'static str,
    sort: Sort,
    message: Message,
    width: Length,
) -> Element<'a, Message> {
    let arrow = match sort {
        Some((active, true)) if active == column => " ▲",
        Some((active, false)) if active == column => " ▼",
        _ => "",
    };
    iced::widget::button(
        text(format!("{label}{arrow}"))
            .size(10)
            .style(design::muted),
    )
    .padding([2, 0])
    .width(width)
    .style(|theme, status| {
        let mut style = design::text_button(theme, status);
        style.background = None;
        style
    })
    .on_press(message)
    .into()
}

// ---------------------------------------------------------------------------
// Filter chips
// ---------------------------------------------------------------------------

/// Removable filter chips followed by a combo box to add more. Backspace
/// in the empty input removes the last chip (handled by the caller's
/// `on_input`, which receives "" twice in a row).
pub fn chips<'a>(
    chips: &'a [String],
    options: &'a iced::widget::combo_box::State<String>,
    query: &'a str,
    on_pick: fn(String) -> Message,
    on_input: fn(String) -> Message,
    on_remove: fn(usize) -> Message,
) -> Element<'a, Message> {
    let mut line = row![].spacing(6).align_y(Alignment::Center);
    for (index, chip) in chips.iter().enumerate() {
        line = line.push(
            container(
                row![
                    text(chip.as_str()).size(11),
                    iced::widget::button(text("✕").size(9))
                        .padding([0, 3])
                        .style(design::text_button)
                        .on_press(on_remove(index)),
                ]
                .spacing(4)
                .align_y(Alignment::Center),
            )
            .padding([2, 4])
            .style(|theme: &Theme| {
                let mut style = design::pill(theme);
                style.background = Some(Background::Color(design::selection(theme)));
                style
            }),
        );
    }
    let picker = iced::widget::combo_box(options, "Add filter…", None, on_pick)
        .on_input(on_input)
        .size(12)
        .width(Length::Fixed(220.0));
    let _ = query;
    line.push(picker).wrap().into()
}

// ---------------------------------------------------------------------------
// Drag-to-reorder list
// ---------------------------------------------------------------------------

/// Rows you can drag (grip ⠿) or move with ↑/↓ buttons; Alt+↑/↓ moves the
/// last touched row. While dragging, rows preview the new order and an
/// accent line eases to the drop slot. `make(from, to)` builds the message
/// that persists the move.
pub fn reorder_list<'a>(
    ui_state: &UiState,
    list: &'static str,
    rows: Vec<Element<'a, Message>>,
    make: fn(usize, usize) -> Message,
) -> Element<'a, Message> {
    let count = rows.len();
    let drag = ui_state.reorder_state(list, Instant::now());
    let mut slots: Vec<Option<Element<'a, Message>>> = rows.into_iter().map(Some).collect();
    // Preview order: the dragged row sits at its target slot.
    let mut order: Vec<usize> = (0..count).collect();
    if let Some((from, target, _)) = drag {
        let target = target.min(count.saturating_sub(1));
        let moved = order.remove(from.min(count.saturating_sub(1)));
        order.insert(target, moved);
    }
    let mut column = column![].spacing(0);
    for (position, index) in order.into_iter().enumerate() {
        let content = slots[index].take().expect("each row used once");
        let dragged = drag.is_some_and(|(from, _, _)| from == index);
        let grip = mouse_area(
            container(text("⠿").size(14).style(design::muted))
                .center_y(Fill)
                .padding([0, 6]),
        )
        .on_press(ui(UiMsg::ReorderGrab(list, index, make)))
        .interaction(iced::mouse::Interaction::Grab);
        let arrows = row![
            design::button(text("↑").size(11))
                .padding([1, 6])
                .style(design::text_button)
                .on_press_maybe((index > 0).then(|| make(index, index - 1))),
            design::button(text("↓").size(11))
                .padding([1, 6])
                .style(design::text_button)
                .on_press_maybe((index + 1 < count).then(|| make(index, index + 1))),
        ];
        let line = container(
            row![grip, container(content).width(Fill), arrows]
                .spacing(6)
                .align_y(Alignment::Center)
                .height(Fill),
        )
        .height(REORDER_ROW)
        .style(move |theme: &Theme| container::Style {
            background: Some(Background::Color(if dragged {
                design::selection(theme)
            } else if position % 2 == 0 {
                design::surface(theme)
            } else {
                design::field(theme)
            })),
            border: Border {
                color: if dragged {
                    theme.palette().primary
                } else {
                    iced::Color::TRANSPARENT
                },
                width: if dragged { 1.0 } else { 0.0 },
                radius: design::radius(theme, Radius::Small).into(),
            },
            ..container::Style::default()
        });
        column = column.push(line);
    }
    let mut layers = iced::widget::stack![column];
    if let Some((_, _, indicator)) = drag {
        layers = layers.push(column![
            space().height(Length::Fixed((indicator * REORDER_ROW - 1.0).max(0.0))),
            container(space().height(2).width(Fill)).style(|theme: &Theme| container::Style {
                background: Some(Background::Color(theme.palette().primary)),
                ..container::Style::default()
            }),
        ]);
    }
    mouse_area(layers)
        .on_move(move |point| ui(UiMsg::ReorderMove(list, point.y)))
        .on_release(ui(UiMsg::ReorderDrop(list)))
        .into()
}

// ---------------------------------------------------------------------------
// Accordion / disclosure
// ---------------------------------------------------------------------------

/// A header with a rotating chevron; the body grows open inside a clipped
/// container and shrinks closed before it is removed.
pub fn disclosure<'a>(
    ui_state: &UiState,
    key: &str,
    title: impl Into<Element<'a, Message>>,
    open: bool,
    toggle: Message,
    body: impl FnOnce() -> Element<'a, Message>,
) -> Element<'a, Message> {
    let now = Instant::now();
    let tween = ui_state.disclosures.get(key);
    let value = match tween {
        Some(tween) if tween.to == if open { 1.0 } else { 0.0 } => tween.value(now),
        _ => {
            if open {
                1.0
            } else {
                0.0
            }
        }
    };
    let animating = tween.is_some_and(|tween| tween.is_running(now));
    let header = iced::widget::button(
        row![chevron(value), title.into()]
            .spacing(10)
            .align_y(Alignment::Center),
    )
    .width(Fill)
    .style(design::secondary)
    .on_press(ui(UiMsg::Disclose(key.to_owned(), !open, Box::new(toggle))));
    let mut column = column![header].spacing(11);
    if value > 0.001 {
        let content = container(body()).width(Fill);
        column = column.push(if animating {
            content.max_height(DISCLOSE_MAX * value).clip(true)
        } else {
            content
        });
    }
    column.into()
}

// ---------------------------------------------------------------------------
// Popover form
// ---------------------------------------------------------------------------

pub fn popover_input_id(id: &str) -> iced::widget::Id {
    iced::widget::Id::from(format!("popover-{id}"))
}

/// A trigger button that morphs into a small inline form (text field +
/// Save/Cancel). Enter saves, Esc or Cancel closes.
#[allow(clippy::too_many_arguments)]
pub fn popover_form<'a>(
    ui_state: &UiState,
    id: &'static str,
    trigger: &'a str,
    placeholder: &'a str,
    value: &'a str,
    on_input: fn(String) -> Message,
    save: Option<Message>,
) -> Element<'a, Message> {
    if ui_state.popover != Some(id) {
        return design::button(trigger)
            .style(design::secondary)
            .on_press(ui(UiMsg::Popover(Some(id))))
            .into();
    }
    let save_message = save.map(|message| ui(UiMsg::PopoverSave(Box::new(message))));
    let mut input = design::text_input(placeholder, value)
        .id(popover_input_id(id))
        .on_input(on_input)
        .width(Length::Fixed(240.0));
    if let Some(message) = save_message.clone() {
        input = input.on_submit(message);
    }
    container(
        row![
            input,
            design::button("Save")
                .style(design::primary)
                .on_press_maybe(save_message),
            design::button("Cancel")
                .style(design::text_button)
                .on_press(ui(UiMsg::Popover(None))),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding(6)
    .style(|theme: &Theme| {
        let mut style = design::panel(theme);
        style.border.color = theme.palette().primary;
        style.shadow = iced::Shadow {
            color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.3),
            offset: iced::Vector::new(0.0, 4.0),
            blur_radius: 12.0,
        };
        style
    })
    .into()
}

// ---------------------------------------------------------------------------
// Edge fades
// ---------------------------------------------------------------------------

/// Gradient strip from the surface colour to transparent. `top` = opaque
/// at the top edge. (iced's angle 0 runs bottom → top.)
fn edge_fade<'a>(top: bool) -> Element<'a, Message> {
    container(space().height(22).width(Fill))
        .width(Fill)
        .style(move |theme: &Theme| {
            let solid = design::surface(theme);
            let clear = design::with_alpha(solid, 0.0);
            let gradient =
                iced::gradient::Linear::new(if top { 0.0 } else { std::f32::consts::PI })
                    .add_stop(0.0, clear)
                    .add_stop(1.0, solid);
            container::Style {
                background: Some(Background::Gradient(gradient.into())),
                ..container::Style::default()
            }
        })
        .into()
}

/// A vertical scrollable with soft fades over its edges: the top fade only
/// once scrolled down, the bottom fade until the end is reached. `long`
/// shows the bottom fade before the first scroll event for lists that are
/// known to overflow.
pub fn faded_scroll<'a>(
    ui_state: &UiState,
    list: &'static str,
    content: impl Into<Element<'a, Message>>,
    long: bool,
) -> Element<'a, Message> {
    let (offset, overflows) = ui_state.scrolls.get(list).copied().unwrap_or((0.0, long));
    let scroll = iced::widget::scrollable(content)
        .height(Fill)
        .width(Fill)
        .on_scroll(move |viewport| {
            let overflows = viewport.content_bounds().height > viewport.bounds().height + 1.0;
            ui(UiMsg::Scrolled(
                list,
                viewport.relative_offset().y,
                overflows,
            ))
        });
    let mut layers = iced::widget::stack![scroll];
    if overflows && offset > 0.01 {
        layers = layers.push(column![edge_fade(true)]);
    }
    if overflows && offset < 0.99 {
        layers = layers.push(column![space().height(Fill), edge_fade(false)]);
    }
    layers.into()
}

// ---------------------------------------------------------------------------
// Copy button
// ---------------------------------------------------------------------------

/// Copies `value` to the clipboard; shows ✓ "Copied" for 1.5 s.
pub fn copy_button<'a>(ui_state: &UiState, id: &str, value: String) -> Element<'a, Message> {
    let copied = ui_state.recently_copied(id, Instant::now());
    iced::widget::tooltip(
        design::button(text(if copied { "✓" } else { "⧉" }).size(12))
            .padding([2, 7])
            .style(design::text_button)
            .on_press(Message::CopyToClipboard(id.to_owned(), value)),
        container(text(if copied { "Copied" } else { "Copy" }).size(11))
            .padding([3, 7])
            .style(design::pill),
        iced::widget::tooltip::Position::Top,
    )
    .into()
}

// ---------------------------------------------------------------------------
// Badge
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadgeKind {
    Enabled,
    Modified,
    Signed,
    Conflict,
    ReadOnly,
    Nightly,
    Neutral,
}

impl BadgeKind {
    fn color(self, theme: &Theme) -> iced::Color {
        let palette = theme.palette();
        match self {
            BadgeKind::Enabled | BadgeKind::Signed => palette.success,
            BadgeKind::Conflict => palette.danger,
            BadgeKind::Modified | BadgeKind::Nightly => palette.warning,
            BadgeKind::ReadOnly | BadgeKind::Neutral => {
                theme.extended_palette().background.strongest.text
            }
        }
    }
}

/// Small pill with a faint fill and coloured text, following the theme.
pub fn badge<'a>(kind: BadgeKind, label: impl text::IntoFragment<'a>) -> Element<'a, Message> {
    container(
        text(label)
            .size(10)
            .style(move |theme: &Theme| text::Style {
                color: Some(kind.color(theme)),
            }),
    )
    .padding([2, 8])
    .style(move |theme: &Theme| {
        let color = kind.color(theme);
        container::Style {
            background: Some(Background::Color(design::with_alpha(color, 0.14))),
            border: Border {
                color: design::with_alpha(color, 0.45),
                width: 1.0,
                radius: design::radius(theme, Radius::Pill).into(),
            },
            ..container::Style::default()
        }
    })
    .into()
}

// ---------------------------------------------------------------------------
// File tree
// ---------------------------------------------------------------------------

/// Faint vertical guides, one per indent level, for tree rows.
pub fn indent_guides<'a>(depth: usize) -> Element<'a, Message> {
    let mut guides = row![].spacing(0);
    for _ in 0..depth.min(20) {
        guides = guides.push(
            container(
                container(space().width(1).height(18)).style(|theme: &Theme| container::Style {
                    background: Some(Background::Color(design::with_alpha(
                        design::border(theme),
                        0.8,
                    ))),
                    ..container::Style::default()
                }),
            )
            .width(14)
            .center_x(14),
        );
    }
    guides.into()
}

/// A chevron rotated from ▸ (0) to ▾ (1).
pub fn chevron<'a>(turn: f32) -> Element<'a, Message> {
    svg(crate::icons::chevron())
        .width(14)
        .height(14)
        .style(design::icon)
        .rotation(iced::Radians(
            turn.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2,
        ))
        .into()
}

// ---------------------------------------------------------------------------
// Dock
// ---------------------------------------------------------------------------

/// Icon scale for an item centred at `center` with the cursor at `cursor`.
pub fn dock_scale(cursor: Option<f32>, center: f32, reduce_motion: bool) -> f32 {
    let Some(cursor) = cursor.filter(|_| !reduce_motion) else {
        return 1.0;
    };
    let distance = (cursor - center).abs();
    let reach = 90.0;
    if distance >= reach {
        1.0
    } else {
        let t = 1.0 - distance / reach;
        1.0 + 0.45 * Easing::EaseInOut.apply(t)
    }
}

pub struct DockItem {
    pub icon: svg::Handle,
    pub label: &'static str,
    pub message: Option<Message>,
    pub active: bool,
}

/// A toolbar whose icons grow as the cursor approaches them.
pub fn dock<'a>(
    ui_state: &UiState,
    id: &'static str,
    items: Vec<DockItem>,
    reduce_motion: bool,
) -> Element<'a, Message> {
    const BASE: f32 = 20.0;
    const SLOT: f32 = 30.0;
    let cursor = ui_state.dock_cursor(id);
    let mut bar = row![].spacing(0).align_y(Alignment::End);
    for (index, item) in items.into_iter().enumerate() {
        let center = SLOT * index as f32 + SLOT / 2.0 + 5.0;
        let scale = dock_scale(cursor, center, reduce_motion);
        let size = BASE * scale;
        let active = item.active;
        let icon = svg(item.icon).width(size).height(size).style(design::icon);
        let button = iced::widget::button(icon)
            .padding(4)
            .style(move |theme, status| design::rail(theme, status, active))
            .on_press_maybe(item.message);
        bar = bar.push(
            container(design::tooltip(
                button,
                item.label,
                crate::chrome::shortcut_for(item.label),
                iced::widget::tooltip::Position::Top,
            ))
            .width(SLOT)
            .align_x(Alignment::Center)
            .align_bottom(BASE * 1.45 + 10.0),
        );
    }
    let shell = container(bar).padding([3, 5]).style(design::pill);
    mouse_area(shell)
        .on_move(move |point| ui(UiMsg::DockMove(id, point.x)))
        .on_exit(ui(UiMsg::DockExit(id)))
        .into()
}

// ---------------------------------------------------------------------------
// Spotlight card wrapper
// ---------------------------------------------------------------------------

/// Wrap a card so a soft accent glow follows the cursor across it.
pub fn spotlight<'a>(
    ui_state: &UiState,
    key: String,
    card: impl Into<Element<'a, Message>>,
    reduce_motion: bool,
) -> Element<'a, Message> {
    let cursor = ui_state
        .spotlight
        .as_ref()
        .filter(|(active, _)| *active == key)
        .map(|(_, point)| *point);
    let card = card.into();
    let layered: Element<'a, Message> = match cursor {
        // The glow is drawn above the card (cards are opaque) and keeps the
        // default mouse interaction, so clicks and hovers still reach it.
        Some(point) if !reduce_motion => iced::widget::stack![
            card,
            iced::widget::canvas(crate::fx::Spotlight { cursor: point })
                .width(Fill)
                .height(Fill),
        ]
        .into(),
        _ => card,
    };
    let enter_key = key.clone();
    mouse_area(layered)
        .on_move(move |point| ui(UiMsg::Spotlight(enter_key.clone(), Some(point))))
        .on_exit(ui(UiMsg::Spotlight(key, None)))
        .into()
}

// ---------------------------------------------------------------------------
// Border beam
// ---------------------------------------------------------------------------

/// While `active`, a bright segment travels around `content`'s border. The
/// content keeps all of its own interaction; the beam layer never captures.
pub fn beam<'a>(
    ui_state: &UiState,
    content: impl Into<Element<'a, Message>>,
    active: bool,
    reduce_motion: bool,
) -> Element<'a, Message> {
    let content = content.into();
    if !active || reduce_motion {
        return content;
    }
    iced::widget::stack![
        content,
        iced::widget::canvas(crate::fx::BorderBeam {
            time: ui_state.clock(Instant::now()),
            period: 2.4,
        })
        .width(Fill)
        .height(Fill),
    ]
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_fires_once_after_duration_and_release_cancels() {
        let mut state = UiState::default();
        update(
            &mut state,
            UiMsg::HoldPress("remove".into(), Box::new(Message::ClearActivity)),
            false,
        );
        let started = state.hold.as_ref().unwrap().started;
        assert!(state.frame(started + HOLD_DURATION / 2).is_none());
        assert!(state.hold_progress("remove", started + HOLD_DURATION / 2) > 0.4);
        assert!(matches!(
            state.frame(started + HOLD_DURATION),
            Some(Message::ClearActivity)
        ));
        assert!(state.frame(started + HOLD_DURATION * 2).is_none());

        update(
            &mut state,
            UiMsg::HoldPress("remove".into(), Box::new(Message::ClearActivity)),
            false,
        );
        update(&mut state, UiMsg::HoldRelease, false);
        assert!(state.frame(Instant::now() + HOLD_DURATION * 2).is_none());
    }

    #[test]
    fn reduce_motion_confirm_needs_two_clicks() {
        let mut state = UiState::default();
        let first = update(
            &mut state,
            UiMsg::ArmConfirm("x".into(), Box::new(Message::ClearActivity)),
            true,
        );
        assert!(first.is_none());
        assert!(state.is_armed("x", Instant::now()));
        let second = update(
            &mut state,
            UiMsg::ArmConfirm("x".into(), Box::new(Message::ClearActivity)),
            true,
        );
        assert!(matches!(second, Some(Message::ClearActivity)));
    }

    #[test]
    fn ticker_eases_between_values_and_first_value_is_immediate() {
        let mut state = UiState::default();
        let t0 = Instant::now();
        state.set_ticker("mods", 3.0, t0, false);
        assert_eq!(state.ticker("mods", t0), Some(3.0));
        state.set_ticker("mods", 7.0, t0, false);
        let mid = state.ticker("mods", t0 + TICKER_DURATION / 2).unwrap();
        assert!(mid > 3.0 && mid < 7.0);
        assert!(state.animating(t0 + TICKER_DURATION / 2));
        assert_eq!(state.ticker("mods", t0 + TICKER_DURATION), Some(7.0));
        state.set_ticker("mods", 1.0, t0, true);
        assert_eq!(state.ticker("mods", t0), Some(1.0));
    }

    #[test]
    fn save_labels_cover_each_state() {
        let now = Instant::now();
        assert_eq!(save_label(false, true, None, now), "Saving…");
        assert_eq!(save_label(true, false, None, now), "Unsaved changes");
        assert_eq!(save_label(false, false, Some(now), now), "Saved just now");
        assert_eq!(
            save_label(false, false, Some(now), now + Duration::from_secs(12)),
            "Saved 12 s ago"
        );
        assert_eq!(save_label(false, false, None, now), "No changes");
    }

    #[test]
    fn steps_mark_done_current_waiting_and_failed() {
        let mut steps = Steps::new(&["a", "b", "c"], 1);
        assert_eq!(steps.state(0), StepState::Done);
        assert_eq!(steps.state(1), StepState::Current);
        assert_eq!(steps.state(2), StepState::Waiting);
        steps.failed = true;
        assert_eq!(steps.state(1), StepState::Failed);
        assert_eq!(Steps::all_done(&["a"]).state(0), StepState::Done);
    }

    #[test]
    fn dock_scale_peaks_under_cursor_and_respects_reduce_motion() {
        assert!(dock_scale(Some(50.0), 50.0, false) > 1.4);
        assert_eq!(dock_scale(Some(500.0), 50.0, false), 1.0);
        assert_eq!(dock_scale(Some(50.0), 50.0, true), 1.0);
        assert_eq!(dock_scale(None, 50.0, false), 1.0);
    }

    #[test]
    fn pulse_stays_in_range_and_is_static_with_reduce_motion() {
        for step in 0..50 {
            let value = pulse(step as f32 * 0.1, false);
            assert!((0.35..=0.76).contains(&value));
        }
        assert_eq!(pulse(0.3, true), pulse(0.9, true));
    }

    #[test]
    fn scramble_resolves_left_to_right_and_finishes_exact() {
        let title = "Asset Library";
        let early = scramble(title, Duration::from_millis(5));
        assert_eq!(early.chars().count(), title.chars().count());
        assert_ne!(early, title);
        assert_eq!(early.chars().nth(5), Some(' '));
        assert_eq!(scramble(title, SCRAMBLE_TOTAL), title);
        let mid = scramble(title, SCRAMBLE_TOTAL / 2);
        assert!(mid.starts_with("Ass"));
    }

    #[test]
    fn bytes_format_with_units() {
        assert_eq!(format_bytes(512.0), "512 B");
        assert_eq!(format_bytes(1536.0), "1.5 KiB");
        assert_eq!(format_bytes(3.0 * 1024.0 * 1024.0 * 1024.0), "3.0 GiB");
    }
}
