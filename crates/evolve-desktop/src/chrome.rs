//! Window chrome: persistent banners, nav indicators, breadcrumbs, meters,
//! the confirmation modal, the side sheet, menubar helpers and the shared
//! shortcut table used by tooltips and menus.
//!
//! Banners and indicators are computed from `State` on every view rather
//! than queued, so they disappear the moment their condition clears.

use crate::anim::{Easing, Tween};
use crate::design::{self, Tone};
use crate::{Message, Page, State, icons};
use iced::widget::{
    button, column, container, mouse_area, row, scrollable, space, stack, svg, text,
};
use iced::{Alignment, Background, Border, Color, Element, Fill, Length, Theme};
use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};

/// Width of the side sheet.
pub const SHEET_WIDTH: f32 = 380.0;
const SHEET_SLIDE: Duration = Duration::from_millis(220);
/// Banners shown before the rest collapse into "+N".
const VISIBLE_BANNERS: usize = 2;

// ---------------------------------------------------------------------------
// Shortcuts
// ---------------------------------------------------------------------------

/// Every keyboard shortcut the manager handles, keyed by the action label
/// used on buttons, dock items and menu entries. Tooltips and menus read
/// from here so their hints match the real key handling in `main.rs`.
pub const SHORTCUTS: &[(&str, &str)] = &[
    ("Command palette", "Ctrl+K"),
    ("Close", "Esc"),
    ("Focus selection", "F"),
    ("Focus selected object", "F"),
    ("Focus selected placement", "F"),
    ("Duplicate", "Shift+D"),
    ("Copy", "Ctrl+C"),
    ("Paste", "Ctrl+V"),
    ("Delete", "Delete"),
    ("Move row up", "Alt+Up"),
    ("Move row down", "Alt+Down"),
    ("Exit fullscreen", "Esc"),
    // Level editor tools.
    ("Select", "1"),
    ("Sculpt", "2"),
    ("Paint", "3"),
    ("Vegetation", "4"),
    ("Move", "G"),
    ("Rotate", "R"),
    ("Scale", "S"),
    ("Constrain to axis", "X / Y / Z"),
    ("Confirm transform", "Enter"),
    ("Cancel transform", "Esc"),
    ("Undo", "Ctrl+Z"),
    ("Redo", "Ctrl+Y"),
    ("Save map", "Ctrl+S"),
    ("Erase vegetation", "Ctrl+LMB"),
    ("Raise", "Shift+1"),
    ("Flatten", "Shift+2"),
    ("Smooth", "Shift+3"),
    ("Create ground", "Shift+4"),
    ("Smaller brush", "["),
    ("Larger brush", "]"),
    ("Lower terrain", "Ctrl+LMB"),
    ("Brush stroke", "LMB"),
    ("Look around", "RMB"),
    ("Fly", "RMB+W A S D"),
    ("Descend / ascend", "RMB+Q / E"),
    ("Select object", "LMB"),
    // Developer tabs.
    ("Network & Process", "Ctrl+1"),
    ("Actors & Memory", "Ctrl+2"),
    ("AI Controls", "Ctrl+3"),
    ("Teleport & Fly", "Ctrl+4"),
    ("External Overlay", "Ctrl+5"),
    ("Keybinds", "Ctrl+6"),
];

/// Level editor shortcuts grouped by tool, for the cheat sheet. Every label
/// resolves through [`shortcut_for`].
pub const LEVEL_SHORTCUTS: &[(&str, &[&str])] = &[
    (
        "Tools",
        &[
            "Select",
            "Sculpt",
            "Paint",
            "Vegetation",
            "Command palette",
        ],
    ),
    (
        "Select and transform",
        &[
            "Select object",
            "Move",
            "Rotate",
            "Scale",
            "Constrain to axis",
            "Confirm transform",
            "Cancel transform",
            "Focus selection",
            "Delete",
            "Copy",
            "Paste",
            "Duplicate",
            "Undo",
            "Redo",
            "Save map",
        ],
    ),
    (
        "Sculpt and paint",
        &[
            "Raise",
            "Flatten",
            "Smooth",
            "Create ground",
            "Brush stroke",
            "Lower terrain",
            "Erase vegetation",
            "Smaller brush",
            "Larger brush",
        ],
    ),
    (
        "Camera",
        &["Look around", "Fly", "Descend / ascend", "Exit fullscreen"],
    ),
];

/// The shortcut for an action label, if it has one.
pub fn shortcut_for(label: &str) -> Option<&'static str> {
    SHORTCUTS
        .iter()
        .find(|(action, _)| action.eq_ignore_ascii_case(label))
        .map(|(_, keys)| *keys)
}

// ---------------------------------------------------------------------------
// State and messages
// ---------------------------------------------------------------------------

/// A confirmation that needs explaining before it runs.
#[derive(Debug, Clone)]
pub struct Modal {
    pub title: String,
    pub body: String,
    pub confirm: &'static str,
    pub tone: Tone,
    pub action: Box<Message>,
}

/// What the side sheet shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sheet {
    /// A Modding project, by index into `modding_projects`.
    Project(usize),
    /// An imported PAK, by index into the library catalog.
    Pak(usize),
    /// The model inspector, without detaching a window.
    Inspector,
}

#[derive(Debug, Clone)]
pub enum Msg {
    /// Hide a dismissible banner for the rest of the session.
    Dismiss(&'static str),
    /// Show or hide banners beyond the first two.
    ExpandBanners(bool),
    /// Ask before running an action.
    Confirm(Box<Modal>),
    ModalCancel,
    ModalConfirm,
    /// Enter pressed outside a text field: confirms an open modal.
    Enter,
    /// Clicks on the modal backdrop do nothing.
    Absorb,
    OpenSheet(Sheet),
    CloseSheet,
    PinSheet(bool),
    /// The "…" segment of a long breadcrumb.
    CrumbMenu(bool),
    /// Asset Library breadcrumb: back to the whole library.
    LibraryRoot,
    /// A menubar title was hovered while another menu was open.
    MenuHover(MenuBar, &'static str),
}

/// Which window's menubar a hover belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuBar {
    Level,
    Viewer,
}

#[derive(Debug, Default)]
pub struct ChromeState {
    dismissed: BTreeSet<&'static str>,
    pub banners_expanded: bool,
    pub modal: Option<Modal>,
    pub sheet: Option<Sheet>,
    sheet_reveal: Tween,
    pub sheet_pinned: bool,
    pub crumb_menu: bool,
    /// Cached swap journal check: `(swap folder, busy, live)`.
    swap: Option<(String, bool, bool)>,
}

impl ChromeState {
    pub fn animating(&self, now: Instant) -> bool {
        self.sheet_reveal.is_running(now)
    }

    /// Settles the sheet slide; a closed sheet is dropped once hidden.
    pub fn tick(&mut self, now: Instant) {
        if !self.sheet_reveal.tick(now) && self.sheet_reveal.to == 0.0 && !self.sheet_pinned {
            self.sheet = None;
        }
    }

    pub fn sheet_reveal(&self, now: Instant) -> f32 {
        self.sheet_reveal.value(now)
    }

    /// True while an unpinned sheet is open (Esc and outside clicks close it).
    pub fn sheet_floating(&self) -> bool {
        self.sheet.is_some() && !self.sheet_pinned && self.sheet_reveal.to > 0.0
    }

    pub fn swap_live(&self) -> bool {
        self.swap.as_ref().is_some_and(|(_, _, live)| *live)
    }
}

/// Re-reads the swap journal only when the folder changes or work starts
/// or finishes, never on redraw.
pub fn sync(state: &mut State) {
    // A closed sheet is dropped once its slide-out ends, even if no
    // animation frame arrives after the last step.
    state.chrome.tick(Instant::now());
    let swap = state.preferences.swap.trim();
    let busy = state.busy;
    let stale = state
        .chrome
        .swap
        .as_ref()
        .is_none_or(|(folder, was_busy, _)| folder != swap || *was_busy != busy);
    if stale {
        let live = swap_journal_live(Path::new(swap));
        state.chrome.swap = Some((swap.to_owned(), busy, live));
    }
}

/// A swap is live when its journal exists in any phase other than
/// "prepared" or "restored" (the same rule the client reconfiguration
/// check uses), or when the journal cannot be read.
pub fn swap_journal_live(swap: &Path) -> bool {
    if swap.as_os_str().is_empty() {
        return false;
    }
    let journal = swap.join("controlled_swap_state.json");
    let Ok(metadata) = std::fs::symlink_metadata(&journal) else {
        return false;
    };
    if !metadata.file_type().is_file() || metadata.len() > 4 * 1024 * 1024 {
        return true;
    }
    let Ok(data) = std::fs::read(&journal) else {
        return true;
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&data) else {
        return true;
    };
    !matches!(value["phase"].as_str(), Some("prepared" | "restored"))
}

pub fn update(state: &mut State, message: Msg) -> Option<Message> {
    let now = Instant::now();
    let reduce = state.preferences.reduce_motion;
    match message {
        Msg::Dismiss(key) => {
            state.chrome.dismissed.insert(key);
        }
        Msg::ExpandBanners(open) => state.chrome.banners_expanded = open,
        Msg::Confirm(modal) => state.chrome.modal = Some(*modal),
        Msg::ModalCancel => state.chrome.modal = None,
        Msg::ModalConfirm | Msg::Enter => {
            return state.chrome.modal.take().map(|modal| *modal.action);
        }
        Msg::Absorb => {}
        Msg::OpenSheet(sheet) => {
            state.chrome.sheet = Some(sheet);
            state
                .chrome
                .sheet_reveal
                .animate_to(1.0, now, SHEET_SLIDE, reduce);
            state.chrome.sheet_reveal = state.chrome.sheet_reveal.with_easing(Easing::EaseOut);
        }
        Msg::CloseSheet => {
            state.chrome.sheet_pinned = false;
            state
                .chrome
                .sheet_reveal
                .animate_to(0.0, now, SHEET_SLIDE, reduce);
            if reduce {
                state.chrome.sheet = None;
            }
        }
        Msg::PinSheet(pinned) => state.chrome.sheet_pinned = pinned,
        Msg::CrumbMenu(open) => state.chrome.crumb_menu = open,
        Msg::LibraryRoot => {
            state.library_selected = None;
            state.chrome.crumb_menu = false;
        }
        Msg::MenuHover(bar, menu) => match bar {
            MenuBar::Level if state.library_menu.is_some_and(|open| open != menu) => {
                state.library_menu = Some(menu);
            }
            MenuBar::Viewer if state.viewer_menu.is_some_and(|open| open != menu) => {
                state.viewer_menu = Some(menu);
            }
            _ => {}
        },
    }
    None
}

pub fn msg(message: Msg) -> Message {
    Message::Chrome(message)
}

/// Opens a confirmation modal that runs `action` on Confirm.
pub fn confirm(
    title: impl Into<String>,
    body: impl Into<String>,
    confirm: &'static str,
    tone: Tone,
    action: Message,
) -> Message {
    msg(Msg::Confirm(Box::new(Modal {
        title: title.into(),
        body: body.into(),
        confirm,
        tone,
        action: Box::new(action),
    })))
}

// ---------------------------------------------------------------------------
// Banners
// ---------------------------------------------------------------------------

pub struct Banner {
    pub key: &'static str,
    pub tone: Tone,
    /// Overrides the tone's icon.
    pub icon: Option<fn() -> svg::Handle>,
    pub message: String,
    pub action: Option<(&'static str, Message)>,
    pub dismissible: bool,
}

/// Long-lasting conditions, most urgent first.
pub fn active_banners(state: &State) -> Vec<Banner> {
    let mut banners = Vec::new();
    if state.chrome.swap_live() {
        let idle = !state.busy && state.active_play.is_none();
        banners.push(Banner {
            key: "modded",
            tone: Tone::Warning,
            icon: Some(icons::shield),
            message:
                "Game files are currently modded. Restore the originals before playing online."
                    .into(),
            action: idle.then(|| ("Restore originals…", restore_originals_confirm())),
            dismissible: false,
        });
    }
    if let Some(error) = &state.update_worker_failure {
        banners.push(Banner {
            key: "update-failed",
            tone: Tone::Danger,
            icon: None,
            message: format!("The previous update failed: {}", first_line(error)),
            action: (!state.busy).then_some(("Retry", Message::RetryUpdate)),
            dismissible: true,
        });
    }
    if state.preferences.play_through_test_client {
        banners.push(Banner {
            key: "test-stage",
            tone: Tone::Info,
            icon: Some(icons::flask),
            message: "Test Game stage active: both Play buttons use the Test Client and Test Game."
                .into(),
            action: Some((
                "Settings",
                Message::OpenSettingsTab(crate::settings_page::Tab::Storage),
            )),
            dismissible: true,
        });
    }
    if let Some(update) = &state.available_update {
        banners.push(Banner {
            key: "update-ready",
            tone: Tone::Info,
            icon: None,
            message: format!("Version {} is ready to download.", update.version),
            action: Some((
                "View update",
                Message::OpenSettingsTab(crate::settings_page::Tab::Updates),
            )),
            dismissible: true,
        });
    }
    banners.retain(|banner| !state.chrome.dismissed.contains(banner.key));
    banners
}

fn first_line(value: &str) -> &str {
    let line = value.lines().next().unwrap_or_default().trim();
    match line.char_indices().nth(120) {
        Some((cut, _)) => &line[..cut],
        None => line,
    }
}

/// The modal shown before restoring original game files.
pub fn restore_originals_confirm() -> Message {
    confirm(
        "Restore original game files?",
        "The manager puts back the original PAKs it swapped out and keeps your prepared mod files so the next modded launch can reuse them. Close the game first. Nothing in your Modding projects is changed.",
        "Restore originals",
        Tone::Warning,
        Message::StartOfflineLauncherRestore,
    )
}

fn tone_icon(tone: Tone) -> svg::Handle {
    match tone {
        Tone::Info => icons::info(),
        Tone::Success => icons::check(),
        Tone::Warning => icons::warning(),
        Tone::Danger => icons::error(),
    }
}

/// The banner strip at the top of the main window; empty when nothing is
/// active.
pub fn banner_strip(state: &State) -> Option<Element<'_, Message>> {
    let banners = active_banners(state);
    if banners.is_empty() {
        return None;
    }
    let hidden = banners.len().saturating_sub(VISIBLE_BANNERS);
    let shown = if state.chrome.banners_expanded {
        banners.len()
    } else {
        VISIBLE_BANNERS
    };
    let mut strip = column![].spacing(6);
    for (index, banner) in banners.into_iter().take(shown).enumerate() {
        let tone = banner.tone;
        let icon = banner.icon.map_or_else(|| tone_icon(tone), |icon| icon());
        let mut line = row![
            icons::glyph(icon, 16.0).style(move |theme: &Theme, _| svg::Style {
                color: Some(design::tone_color(theme, tone)),
            }),
            text(banner.message).size(design::LABEL).width(Fill),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        if let Some((label, action)) = banner.action {
            line = line.push(
                design::button(text(label).size(design::LABEL))
                    .padding([4, 10])
                    .style(design::secondary)
                    .on_press(action),
            );
        }
        if index == 0 && hidden > 0 {
            line = line.push(
                design::button(
                    text(if state.chrome.banners_expanded {
                        "Show less".to_owned()
                    } else {
                        format!("+{hidden}")
                    })
                    .size(design::LABEL),
                )
                .padding([4, 8])
                .style(design::text_button)
                .on_press(msg(Msg::ExpandBanners(!state.chrome.banners_expanded))),
            );
        }
        if banner.dismissible {
            line = line.push(design::tooltip(
                button(icons::glyph(icons::close(), 14.0))
                    .padding(4)
                    .style(design::text_button)
                    .on_press(msg(Msg::Dismiss(banner.key))),
                "Hide for this session",
                None,
                iced::widget::tooltip::Position::Left,
            ));
        }
        strip = strip.push(
            container(line)
                .padding([6, 12])
                .width(Fill)
                .style(move |theme| design::tinted(theme, tone)),
        );
    }
    Some(strip.into())
}

// ---------------------------------------------------------------------------
// Nav indicators
// ---------------------------------------------------------------------------

/// What a sidebar item should flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Indicator {
    Dot,
    Count(usize),
}

/// Announcements newer than the last time What's New was opened.
pub fn unread_news(state: &State) -> usize {
    let seen = state.preferences.news_seen_at.as_str();
    state
        .news
        .cache
        .announcements
        .iter()
        .filter(|post| post.timestamp.as_str() > seen)
        .count()
}

/// The newest announcement timestamp, to remember as seen.
pub fn newest_news(state: &State) -> Option<String> {
    state
        .news
        .cache
        .announcements
        .iter()
        .map(|post| post.timestamp.clone())
        .max()
}

pub fn nav_indicator(state: &State, page: Page) -> Option<Indicator> {
    match page {
        Page::Settings => (state.available_update.is_some() || !state.launch_updates.is_empty())
            .then_some(Indicator::Dot),
        Page::WhatsNew if state.page != Some(Page::WhatsNew) => {
            let unread = unread_news(state);
            (unread > 0).then_some(Indicator::Count(unread))
        }
        _ => None,
    }
}

/// Layers an indicator over the top-right corner of a nav icon. The icon
/// keeps its size, so the dot shows in the collapsed Legacy rail too.
pub fn badge_icon<'a>(
    icon: impl Into<Element<'a, Message>>,
    size: f32,
    indicator: Option<Indicator>,
    pulse: f32,
) -> Element<'a, Message> {
    let icon = container(icon).padding([3, 4]);
    let Some(indicator) = indicator else {
        return icon.into();
    };
    let dot = match indicator {
        Indicator::Dot => design::indicator_dot(None, pulse),
        Indicator::Count(count) => design::indicator_dot(Some(count), 1.0),
    };
    stack![
        icon,
        container(dot)
            .width(Length::Fixed(size + 8.0))
            .align_right(Length::Fixed(size + 8.0))
    ]
    .into()
}

// ---------------------------------------------------------------------------
// Breadcrumbs
// ---------------------------------------------------------------------------

/// One breadcrumb segment. `None` makes it plain text (the current level).
pub struct Crumb {
    pub label: String,
    pub message: Option<Message>,
}

/// Segments kept on each side of the "…" when a path is long.
const CRUMB_HEAD: usize = 1;
const CRUMB_TAIL: usize = 3;

/// Which segments to show, and the hidden middle range when collapsed.
pub fn crumb_layout(count: usize) -> (Vec<usize>, Option<std::ops::Range<usize>>) {
    if count <= CRUMB_HEAD + CRUMB_TAIL + 1 {
        return ((0..count).collect(), None);
    }
    let hidden = CRUMB_HEAD..count - CRUMB_TAIL;
    let shown = (0..CRUMB_HEAD).chain(count - CRUMB_TAIL..count).collect();
    (shown, Some(hidden))
}

fn crumb_button<'a>(crumb: Crumb, last: bool) -> Element<'a, Message> {
    let label = text(crumb.label).size(design::LABEL);
    if last || crumb.message.is_none() {
        return container(label).padding([3, 4]).into();
    }
    button(label)
        .padding([3, 4])
        .style(design::text_button)
        .on_press_maybe(crumb.message)
        .into()
}

/// `Library › Build › PAK › folder › file`. Long paths collapse their
/// middle into "…", which opens a list of the hidden levels.
pub fn breadcrumbs<'a>(crumbs: Vec<Crumb>, menu_open: bool) -> Element<'a, Message> {
    let count = crumbs.len();
    let (shown, hidden) = crumb_layout(count);
    let mut crumbs: Vec<Option<Crumb>> = crumbs.into_iter().map(Some).collect();
    let separator = || text("›").size(design::LABEL).style(design::muted);
    let mut bar = row![].spacing(2).align_y(Alignment::Center);
    let mut hidden_list = column![].spacing(2);
    for (position, index) in shown.iter().enumerate() {
        if position > 0 {
            bar = bar.push(separator());
        }
        if let Some(range) = &hidden
            && position == CRUMB_HEAD
        {
            bar = bar.push(
                button(text("…").size(design::LABEL))
                    .padding([3, 6])
                    .style(design::text_button)
                    .on_press(msg(Msg::CrumbMenu(!menu_open))),
            );
            bar = bar.push(separator());
            for hidden_index in range.clone() {
                if let Some(crumb) = crumbs[hidden_index].take() {
                    hidden_list = hidden_list.push(crumb_button(crumb, false));
                }
            }
        }
        if let Some(crumb) = crumbs[*index].take() {
            bar = bar.push(crumb_button(crumb, *index + 1 == count));
        }
    }
    if hidden.is_some() && menu_open {
        column![bar, container(hidden_list).padding(4).style(design::panel)]
            .spacing(4)
            .into()
    } else {
        bar.into()
    }
}

/// Splits an archive member path into folder crumbs that filter the
/// explorer to that folder (via `on_folder`), ending with the file name.
pub fn path_crumbs(
    root_label: String,
    root: Message,
    path: &str,
    on_folder: impl Fn(String) -> Message,
) -> Vec<Crumb> {
    let normalized = path.replace('\\', "/");
    let parts: Vec<&str> = normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    let mut crumbs = vec![Crumb {
        label: root_label,
        message: Some(root),
    }];
    let mut prefix = String::new();
    for (index, part) in parts.iter().enumerate() {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(part);
        let folder = index + 1 < parts.len();
        crumbs.push(Crumb {
            label: (*part).to_owned(),
            message: folder.then(|| on_folder(format!("{prefix}/"))),
        });
    }
    crumbs
}

// ---------------------------------------------------------------------------
// Meter
// ---------------------------------------------------------------------------

/// Threshold colour for a fill level: calm until 75 %, amber to 90 %,
/// then red.
pub fn meter_tone(level: f32) -> Tone {
    if level >= 0.9 {
        Tone::Danger
    } else if level >= 0.75 {
        Tone::Warning
    } else {
        Tone::Success
    }
}

/// A labelled level bar (disk, memory), not a progress bar.
pub fn meter<'a>(label: String, level: f32, value: String) -> Element<'a, Message> {
    let level = level.clamp(0.0, 1.0);
    let tone = meter_tone(level);
    let track = container(
        row![
            container(space().height(6))
                .width(Length::FillPortion((level * 1000.0).round().max(1.0) as u16))
                .style(move |theme: &Theme| container::Style {
                    background: Some(Background::Color(design::tone_color(theme, tone))),
                    border: Border {
                        radius: 3.0.into(),
                        ..Border::default()
                    },
                    ..container::Style::default()
                }),
            space()
                .width(Length::FillPortion(
                    ((1.0 - level) * 1000.0).round().max(1.0) as u16
                ))
                .height(6),
        ]
        .width(Fill),
    )
    .width(Fill)
    .style(|theme: &Theme| container::Style {
        background: Some(Background::Color(design::field(theme))),
        border: Border {
            radius: 3.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    });
    column![
        row![
            text(label).size(design::LABEL).width(Fill),
            text(value).size(design::CAPTION).style(design::muted),
        ]
        .align_y(Alignment::Center),
        track,
    ]
    .spacing(4)
    .into()
}

// ---------------------------------------------------------------------------
// Modal
// ---------------------------------------------------------------------------

fn scrim(alpha: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(Color::from_rgba(0.0, 0.0, 0.0, alpha))),
        ..container::Style::default()
    }
}

/// Lays the open modal over `base`: a dimmed backdrop that swallows clicks
/// and a centred card. Esc cancels and Enter confirms (see `main.rs`).
pub fn modal_overlay<'a>(base: Element<'a, Message>, state: &'a State) -> Element<'a, Message> {
    let Some(modal) = &state.chrome.modal else {
        return base;
    };
    let tone = modal.tone;
    let card = container(
        column![
            row![
                icons::glyph(tone_icon(tone), 20.0).style(move |theme: &Theme, _| svg::Style {
                    color: Some(design::tone_color(theme, tone)),
                }),
                design::heading(modal.title.as_str()),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
            design::body(modal.body.as_str()),
            row![
                space().width(Fill),
                row![
                    design::kbd("Esc"),
                    design::button("Cancel")
                        .style(design::secondary)
                        .on_press(msg(Msg::ModalCancel)),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
                row![
                    design::kbd("Enter"),
                    design::button(modal.confirm)
                        .style(design::primary)
                        .on_press(msg(Msg::ModalConfirm)),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            ]
            .spacing(14)
            .align_y(Alignment::Center),
        ]
        .spacing(14),
    )
    .padding(20)
    .max_width(460)
    .style(|theme: &Theme| container::Style {
        background: Some(Background::Color(design::surface(theme))),
        border: Border {
            color: design::border(theme),
            width: 1.0,
            radius: design::radius(theme, design::Radius::Large).into(),
        },
        shadow: iced::Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.45),
            offset: iced::Vector::new(0.0, 8.0),
            blur_radius: 28.0,
        },
        ..container::Style::default()
    });
    // `Idle` interaction stops hovers and scrolls reaching the page too.
    let backdrop = mouse_area(
        container(space())
            .width(Fill)
            .height(Fill)
            .style(scrim(0.5)),
    )
    .on_press(msg(Msg::Absorb))
    .interaction(iced::mouse::Interaction::Idle);
    stack![base, backdrop, container(card).center(Fill).padding(24)].into()
}

// ---------------------------------------------------------------------------
// Side sheet
// ---------------------------------------------------------------------------

fn sheet_contents(state: &State, sheet: Sheet) -> (String, Element<'_, Message>) {
    let field = |name: &'static str, value: String| -> Element<'_, Message> {
        column![
            text(name).size(design::CAPTION).style(design::muted),
            text(value).size(design::LABEL),
        ]
        .spacing(2)
        .into()
    };
    match sheet {
        Sheet::Project(index) => {
            let Some(project) = state
                .modding_projects
                .as_ref()
                .and_then(|projects| projects.get(index))
            else {
                return (
                    "Project".into(),
                    design::caption("This project is no longer listed.").into(),
                );
            };
            let mut modified = column![].spacing(2);
            for member in project.modified.iter().take(40) {
                modified = modified.push(design::mono(member.as_str()).size(design::CAPTION));
            }
            if project.modified.len() > 40 {
                modified = modified.push(design::caption(format!(
                    "+{} more",
                    project.modified.len() - 40
                )));
            }
            (
                project.label.clone(),
                column![
                    crate::ui_kit::badge(
                        if project.enabled {
                            crate::ui_kit::BadgeKind::Enabled
                        } else {
                            crate::ui_kit::BadgeKind::Neutral
                        },
                        if project.enabled {
                            "Enabled"
                        } else {
                            "Disabled"
                        },
                    ),
                    field("TARGET PAK", project.target.clone()),
                    field("SOURCE BUILD", project.source_build.clone()),
                    field("STATUS", project.status.clone()),
                    field("DETAILS", project.details.clone()),
                    field("WORKSPACE", project.workspace.display().to_string()),
                    text(format!("CHANGED FILES · {}", project.modified.len()))
                        .size(design::CAPTION)
                        .style(design::muted),
                    modified,
                ]
                .spacing(10)
                .into(),
            )
        }
        Sheet::Pak(index) => {
            let Some(archive) = state
                .library_catalog
                .as_ref()
                .and_then(|catalog| catalog.archives.get(index))
            else {
                return (
                    "PAK".into(),
                    design::caption("This PAK is no longer imported.").into(),
                );
            };
            (
                archive.name.clone(),
                column![
                    field("GROUP", archive.group.clone()),
                    field("DESCRIPTION", archive.description.clone()),
                    field("ENTRIES", archive.entries.to_string()),
                    field(
                        "EXTRACTED SIZE",
                        crate::ui_kit::format_bytes(archive.extracted_bytes as f64)
                    ),
                    field("BUILDS", archive.builds.join(", ")),
                    field("SOURCE", archive.source.clone()),
                    field("FOLDER", archive.directory.clone()),
                    field(
                        "WARNINGS",
                        if archive.warnings == 0 {
                            "None".into()
                        } else {
                            archive.warnings.to_string()
                        }
                    ),
                    design::button("Open map in Level Editor")
                        .style(design::secondary)
                        .on_press(Message::LibraryOpenMap(index)),
                ]
                .spacing(10)
                .into(),
            )
        }
        Sheet::Inspector => ("Inspector".into(), crate::viewer_inspector::view(state)),
    }
}

fn sheet_panel(state: &State, sheet: Sheet) -> Element<'_, Message> {
    let (title, body) = sheet_contents(state, sheet);
    let pinned = state.chrome.sheet_pinned;
    let header = row![
        design::heading(title).width(Fill),
        design::tooltip(
            button(icons::glyph(icons::pin(), 16.0))
                .padding(5)
                .style(move |theme, status| design::rail(theme, status, pinned))
                .on_press(msg(Msg::PinSheet(!pinned))),
            if pinned { "Unpin" } else { "Pin as a column" },
            None,
            iced::widget::tooltip::Position::Bottom,
        ),
        design::tooltip(
            button(icons::glyph(icons::close(), 16.0))
                .padding(5)
                .style(design::text_button)
                .on_press(msg(Msg::CloseSheet)),
            "Close",
            Some("Esc"),
            iced::widget::tooltip::Position::Bottom,
        ),
    ]
    .spacing(4)
    .align_y(Alignment::Center);
    container(column![header, scrollable(body).height(Fill)].spacing(12))
        .padding(16)
        .width(Length::Fixed(SHEET_WIDTH))
        .height(Fill)
        .style(|theme: &Theme| container::Style {
            background: Some(Background::Color(design::surface(theme))),
            border: Border {
                color: design::border(theme),
                width: 1.0,
                radius: 0.0.into(),
            },
            shadow: iced::Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.35),
                offset: iced::Vector::new(-6.0, 0.0),
                blur_radius: 24.0,
            },
            ..container::Style::default()
        })
        .into()
}

/// Shows the side sheet: a column beside `page` when pinned, otherwise a
/// panel sliding in over it. The page dims slightly but keeps scrolling;
/// clicking outside the sheet closes it.
pub fn sheet_overlay<'a>(page: Element<'a, Message>, state: &'a State) -> Element<'a, Message> {
    let Some(sheet) = state.chrome.sheet else {
        return page;
    };
    let panel = sheet_panel(state, sheet);
    if state.chrome.sheet_pinned {
        return row![page, panel].height(Fill).into();
    }
    let reveal = state.chrome.sheet_reveal(Instant::now());
    // Fully hidden: draw nothing, so no invisible backdrop eats clicks.
    if reveal <= 0.0 && !state.chrome.sheet_floating() {
        return page;
    }
    // The backdrop only handles clicks, so scroll events still reach the
    // page below it.
    let backdrop = mouse_area(
        container(space())
            .width(Fill)
            .height(Fill)
            .style(scrim(0.18 * reveal)),
    )
    .on_press(msg(Msg::CloseSheet));
    // A right-aligned, clipped slot that widens with `reveal`: the sheet's
    // left edge appears first, so it reads as sliding in from the right.
    let slot = container(panel)
        .width(Length::Fixed((SHEET_WIDTH * reveal).max(1.0)))
        .height(Fill)
        .clip(true);
    stack![
        page,
        backdrop,
        container(slot).width(Fill).height(Fill).align_right(Fill)
    ]
    .into()
}

// ---------------------------------------------------------------------------
// Menubar
// ---------------------------------------------------------------------------

/// A menubar title. Hovering it while another menu is open switches menus.
pub fn menu_title<'a>(
    bar: MenuBar,
    label: &'static str,
    id: &'static str,
    open: Option<&'static str>,
    on_press: Message,
    size: f32,
    padding: [u16; 2],
) -> Element<'a, Message> {
    let active = open == Some(id);
    let title = button(text(label).size(size))
        .padding(padding)
        .style(move |theme, status| {
            if active {
                design::rail(theme, status, true)
            } else {
                design::text_button(theme, status)
            }
        })
        .on_press(on_press);
    mouse_area(title)
        .on_enter(msg(Msg::MenuHover(bar, id)))
        .into()
}

/// A menu entry with its shortcut on the right. `None` greys it out.
pub fn menu_item<'a>(label: &'static str, action: Option<Message>) -> Element<'a, Message> {
    let enabled = action.is_some();
    let mut line = row![
        text(label)
            .size(design::LABEL)
            .width(Fill)
            .style(move |theme: &Theme| text::Style {
                color: (!enabled).then(|| design::with_alpha(theme.palette().text, 0.4)),
            })
    ]
    .spacing(12)
    .align_y(Alignment::Center);
    if let Some(shortcut) = shortcut_for(label) {
        line = line.push(design::key_hint(shortcut));
    }
    button(line)
        .padding([6, 12])
        .width(Fill)
        .style(design::text_button)
        .on_press_maybe(action.map(|action| Message::MenuCommand(Box::new(action))))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_are_found_case_insensitively() {
        assert_eq!(shortcut_for("command palette"), Some("Ctrl+K"));
        assert_eq!(shortcut_for("Focus selection"), Some("F"));
        assert_eq!(shortcut_for("Unknown action"), None);
    }

    #[test]
    fn every_listed_level_shortcut_resolves() {
        for (_, actions) in LEVEL_SHORTCUTS {
            for action in *actions {
                assert!(shortcut_for(action).is_some(), "{action}");
            }
        }
    }

    #[test]
    fn short_breadcrumbs_show_every_segment() {
        let (shown, hidden) = crumb_layout(5);
        assert_eq!(shown, vec![0, 1, 2, 3, 4]);
        assert!(hidden.is_none());
    }

    #[test]
    fn long_breadcrumbs_collapse_the_middle() {
        let (shown, hidden) = crumb_layout(8);
        assert_eq!(shown, vec![0, 5, 6, 7]);
        assert_eq!(hidden, Some(1..5));
    }

    #[test]
    fn path_crumbs_filter_folders_and_end_on_the_file() {
        let crumbs = path_crumbs(
            "objects.pak".into(),
            Message::ViewerSearchChanged(String::new()),
            "characters/hunters/emet.cdf",
            Message::ViewerSearchChanged,
        );
        let labels: Vec<_> = crumbs.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["objects.pak", "characters", "hunters", "emet.cdf"]);
        assert!(matches!(
            &crumbs[2].message,
            Some(Message::ViewerSearchChanged(prefix)) if prefix == "characters/hunters/"
        ));
        assert!(crumbs[3].message.is_none());
    }

    #[test]
    fn meter_colours_follow_thresholds() {
        assert_eq!(meter_tone(0.5), Tone::Success);
        assert_eq!(meter_tone(0.8), Tone::Warning);
        assert_eq!(meter_tone(0.95), Tone::Danger);
    }

    #[test]
    fn swap_journal_phases() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!swap_journal_live(dir.path()));
        let journal = dir.path().join("controlled_swap_state.json");
        std::fs::write(&journal, br#"{"phase":"restored","files":[]}"#).unwrap();
        assert!(!swap_journal_live(dir.path()));
        std::fs::write(&journal, br#"{"phase":"swapped","files":[]}"#).unwrap();
        assert!(swap_journal_live(dir.path()));
        std::fs::write(&journal, b"not json").unwrap();
        assert!(swap_journal_live(dir.path()));
        assert!(!swap_journal_live(Path::new("")));
    }

    #[test]
    fn modal_confirm_returns_the_action_once() {
        let mut chrome = ChromeState {
            modal: Some(Modal {
                title: "t".into(),
                body: "b".into(),
                confirm: "Go",
                tone: Tone::Info,
                action: Box::new(Message::RetryUpdate),
            }),
            ..ChromeState::default()
        };
        let action = chrome.modal.take().map(|modal| *modal.action);
        assert!(matches!(action, Some(Message::RetryUpdate)));
        assert!(chrome.modal.is_none());
    }

    #[test]
    fn long_update_errors_are_trimmed_to_one_line() {
        assert_eq!(first_line("a\nb"), "a");
        assert_eq!(first_line(&"x".repeat(300)).len(), 120);
    }
}
