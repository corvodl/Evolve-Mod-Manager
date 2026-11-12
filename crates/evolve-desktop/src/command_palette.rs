//! Ctrl+K command palette: jump to pages and settings tabs, open mod
//! projects or library archives, and run common actions from anywhere.
//!
//! Layered over the main window with `stack!` (like the toasts). Keyboard
//! handling reuses the app's `event::listen_with`: Ctrl+K toggles, the
//! arrow keys move, Enter runs and Esc closes.

use iced::widget::{Id, column, container, mouse_area, row, scrollable, space, text};
use iced::{Alignment, Background, Border, Color, Element, Fill, Length, Task, Theme};

use crate::design::{self, Radius};
use crate::settings_page::Tab;
use crate::ui_kit::UiMsg;
use crate::{Message, Page, State};

const MAX_ROWS: usize = 12;
const MAX_PROJECTS: usize = 400;
const MAX_ARCHIVES: usize = 2000;

pub fn input_id() -> Id {
    Id::new("command-palette-input")
}

#[derive(Debug, Clone)]
pub enum Msg {
    Toggle,
    Close,
    Query(String),
    Move(i32),
    Submit,
    Run(usize),
    Hover(usize),
}

#[derive(Debug, Default)]
pub struct PaletteState {
    pub open: bool,
    pub query: String,
    pub selected: usize,
}

#[derive(Debug, Clone)]
pub struct Command {
    pub group: &'static str,
    pub label: String,
    pub hint: String,
    pub message: Message,
}

fn command(
    group: &'static str,
    label: impl Into<String>,
    hint: impl Into<String>,
    message: Message,
) -> Command {
    Command {
        group,
        label: label.into(),
        hint: hint.into(),
        message,
    }
}

/// Every command available in the current state.
pub fn commands(state: &State) -> Vec<Command> {
    let mut list = Vec::new();
    for page in Page::ALL.into_iter().chain([Page::Settings]) {
        list.push(command(
            "Go to",
            page.title(),
            "Page",
            Message::Navigate(page),
        ));
    }
    for tab in Tab::ALL {
        list.push(command(
            "Settings",
            format!("Settings › {}", tab.title()),
            "Settings tab",
            Message::OpenSettingsTab(tab),
        ));
    }
    let actions: [(&str, &str, Message); 14] = [
        (
            "Play with mods",
            "Build enabled mods and launch",
            Message::PlayWithMods,
        ),
        (
            "Play without mods",
            "Regular client",
            Message::PlayWithoutMods,
        ),
        ("Build selected mod", "Modding", Message::ModdingBuild),
        ("Check for updates", "Updates", Message::CheckUpdates),
        ("Open Asset Viewer", "Window", Message::OpenViewer),
        (
            "Open Level Editor",
            "Window",
            Message::OpenLevelEditorWindow,
        ),
        ("Show activity", "Window", Message::ToggleActivity),
        (
            "Show inspector in side panel",
            "Window",
            Message::Chrome(crate::chrome::Msg::OpenSheet(
                crate::chrome::Sheet::Inspector,
            )),
        ),
        (
            "Restore original game files…",
            "Recovery",
            crate::chrome::restore_originals_confirm(),
        ),
        (
            "Analyze storage",
            "Settings › Storage",
            Message::SettingsAnalyzeStorage,
        ),
        ("Refresh news", "What's New", Message::RefreshNews),
        ("Refresh mod projects", "Modding", Message::ModdingRefresh),
        (
            "Toggle light / dark",
            "Appearance",
            Message::PreferenceAppearance,
        ),
        (
            "Toggle reduce motion",
            "Appearance",
            Message::PreferenceReduceMotion,
        ),
    ];
    for (label, hint, message) in actions {
        list.push(command("Actions", label, hint, message));
    }
    // Everything in the Asset Library menus, while it's available.
    for (menu, label, action, _, _) in crate::asset_library_page::menu_commands(state) {
        if let Some(message) = action {
            list.push(command("Asset Library", label, format!("Asset Library › {menu}"), message));
        }
    }
    // Everything in the Modding menus, while it's available.
    for (menu, label, action) in crate::modding_page::menu_commands(state) {
        if let Some(message) = action {
            list.push(command(
                "Modding",
                label,
                format!("Modding › {menu}"),
                message,
            ));
        }
    }
    if let Some(projects) = state.modding_projects.as_ref() {
        for (index, project) in projects.iter().take(MAX_PROJECTS).enumerate() {
            list.push(command(
                "Projects",
                project.label.clone(),
                if project.enabled {
                    "Mod project · enabled"
                } else {
                    "Mod project"
                },
                Message::Sequence(vec![
                    Message::Navigate(Page::Files),
                    Message::ModdingSelectProject(index),
                ]),
            ));
        }
    }
    if let Some(catalog) = state.library_catalog.as_ref() {
        for (index, archive) in catalog.archives.iter().take(MAX_ARCHIVES).enumerate() {
            list.push(command(
                "Library",
                archive.name.clone(),
                archive.group.clone(),
                Message::Sequence(vec![
                    Message::Navigate(Page::AssetLibrary),
                    Message::LibrarySelect(index),
                ]),
            ));
        }
    }
    list
}

/// Subsequence fuzzy score: higher is better, `None` means no match.
/// Rewards consecutive characters, word starts and an early first hit.
pub fn fuzzy_score(query: &str, candidate: &str) -> Option<i32> {
    let query: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    if query.is_empty() {
        return Some(0);
    }
    let chars: Vec<char> = candidate.chars().collect();
    let lower: Vec<char> = chars
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    let mut score = 0;
    let mut qi = 0;
    let mut previous: Option<usize> = None;
    let mut first = None;
    for (index, ch) in lower.iter().enumerate() {
        if qi < query.len() && *ch == query[qi] {
            score += 1;
            if previous.is_some_and(|p| p + 1 == index) {
                score += 5;
            }
            let word_start = index == 0
                || !chars[index - 1].is_alphanumeric()
                || (chars[index].is_uppercase() && chars[index - 1].is_lowercase());
            if word_start {
                score += 8;
            }
            first.get_or_insert(index);
            previous = Some(index);
            qi += 1;
        }
    }
    if qi < query.len() {
        return None;
    }
    score -= first.unwrap_or(0).min(20) as i32;
    // Prefer shorter labels for equal matches.
    score -= (chars.len() / 16) as i32;
    Some(score)
}

/// Commands matching the query, best first, capped at [`MAX_ROWS`].
pub fn results(state: &State, query: &str) -> Vec<Command> {
    let mut scored: Vec<(i32, usize, Command)> = commands(state)
        .into_iter()
        .enumerate()
        .filter_map(|(order, cmd)| {
            let best = fuzzy_score(query, &cmd.label)
                .into_iter()
                .chain(fuzzy_score(query, &format!("{} {}", cmd.group, cmd.label)).map(|s| s - 4))
                .max()?;
            Some((best, order, cmd))
        })
        .collect();
    if !query.trim().is_empty() {
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    }
    scored
        .into_iter()
        .take(MAX_ROWS)
        .map(|(_, _, cmd)| cmd)
        .collect()
}

/// Returns a task (e.g. focusing the input) and a command to run.
pub fn update(state: &mut State, message: Msg) -> (Task<Message>, Option<Message>) {
    let palette = &mut state.ui.palette;
    match message {
        Msg::Toggle => {
            palette.open = !palette.open;
            palette.query.clear();
            palette.selected = 0;
            if palette.open {
                return (iced::widget::operation::focus(input_id()), None);
            }
        }
        Msg::Close => palette.open = false,
        Msg::Query(query) => {
            palette.query = query;
            palette.selected = 0;
        }
        Msg::Move(delta) if palette.open => {
            let query = palette.query.clone();
            let count = results(state, &query).len();
            let palette = &mut state.ui.palette;
            if count > 0 {
                let next = (palette.selected as i32 + delta).rem_euclid(count as i32);
                palette.selected = next as usize;
            }
        }
        Msg::Move(_) => {}
        Msg::Hover(index) => palette.selected = index,
        Msg::Submit => {
            let index = palette.selected;
            return update(state, Msg::Run(index));
        }
        Msg::Run(index) if state.ui.palette.open => {
            let query = state.ui.palette.query.clone();
            let chosen = results(state, &query).into_iter().nth(index);
            state.ui.palette.open = false;
            return (Task::none(), chosen.map(|cmd| cmd.message));
        }
        Msg::Run(_) => {}
    }
    (Task::none(), None)
}

fn palette(message: Msg) -> Message {
    Message::Ui(UiMsg::Palette(message))
}

/// Layer the palette over `base` when open.
pub fn overlay<'a>(base: Element<'a, Message>, state: &'a State) -> Element<'a, Message> {
    let palette_state = &state.ui.palette;
    if !palette_state.open {
        return base;
    }
    let found = results(state, &palette_state.query);
    let input = design::text_input(
        "Search pages, settings, projects, library and actions…",
        &palette_state.query,
    )
    .id(input_id())
    .on_input(|value| palette(Msg::Query(value)))
    .on_submit(palette(Msg::Submit))
    .size(15)
    .padding([11, 14]);
    let mut rows = column![].spacing(2);
    if found.is_empty() {
        rows = rows.push(
            container(text("No matching commands").size(12).style(design::muted)).padding([10, 12]),
        );
    }
    let mut last_group = "";
    for (index, cmd) in found.into_iter().enumerate() {
        if cmd.group != last_group {
            last_group = cmd.group;
            rows = rows.push(
                container(text(cmd.group).size(10).style(design::muted)).padding(iced::Padding {
                    top: 8.0,
                    right: 12.0,
                    bottom: 2.0,
                    left: 12.0,
                }),
            );
        }
        let selected = index == palette_state.selected;
        let line = row![
            text(cmd.label).size(13).width(Fill),
            text(cmd.hint).size(11).style(design::muted),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        let item = iced::widget::button(line)
            .width(Fill)
            .padding([7, 12])
            .style(move |theme: &Theme, status| {
                let mut style = design::text_button(theme, status);
                if selected {
                    style.background = Some(Background::Color(design::selection(theme)));
                }
                style
            })
            .on_press(palette(Msg::Run(index)));
        rows = rows.push(mouse_area(item).on_enter(palette(Msg::Hover(index))));
    }
    let footer = row![
        text("↑↓ move").size(10).style(design::muted),
        text("Enter run").size(10).style(design::muted),
        text("Esc close").size(10).style(design::muted),
        space().width(Fill),
        text("Ctrl+K").size(10).style(design::muted),
    ]
    .spacing(14);
    let card =
        container(column![input, scrollable(rows).height(Length::Shrink), footer].spacing(10))
            .padding(12)
            .width(Length::Fixed(600.0))
            .max_height(520)
            .style(|theme: &Theme| container::Style {
                background: Some(Background::Color(design::surface(theme))),
                border: Border {
                    color: design::border(theme),
                    width: 1.0,
                    radius: design::radius(theme, Radius::Large).into(),
                },
                shadow: iced::Shadow {
                    color: Color::from_rgba(0.0, 0.0, 0.0, 0.45),
                    offset: iced::Vector::new(0.0, 10.0),
                    blur_radius: 30.0,
                },
                ..container::Style::default()
            });
    // Clicks on the card are captured by its widgets; clicks on the dimmed
    // backdrop close the palette.
    let backdrop =
        mouse_area(
            container(space())
                .width(Fill)
                .height(Fill)
                .style(|_| container::Style {
                    background: Some(Background::Color(Color::from_rgba(0.0, 0.0, 0.0, 0.42))),
                    ..container::Style::default()
                }),
        )
        .on_press(palette(Msg::Close));
    let layer = container(mouse_area(card).on_press(palette(Msg::Hover(palette_state.selected))))
        .width(Fill)
        .center_x(Fill)
        .padding(iced::Padding {
            top: 90.0,
            right: 16.0,
            bottom: 16.0,
            left: 16.0,
        });
    iced::widget::stack![base, backdrop, layer].into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_prefers_word_starts_and_rejects_non_matches() {
        assert!(fuzzy_score("xyz", "Settings").is_none());
        let word = fuzzy_score("cfu", "Check for updates").unwrap();
        let scattered = fuzzy_score("cfu", "Ice Alpine fullscreen").unwrap_or(i32::MIN);
        assert!(word > scattered);
        assert!(fuzzy_score("set", "Settings").unwrap() > fuzzy_score("set", "Reset").unwrap());
        assert_eq!(fuzzy_score("", "anything"), Some(0));
    }

    #[test]
    fn results_are_capped_and_ranked() {
        let state = State::default();
        let all = results(&state, "");
        assert!(all.len() <= MAX_ROWS);
        let top = results(&state, "updates");
        assert!(top[0].label.contains("pdates"), "{:?}", top[0].label);
        let storage = results(&state, "storage");
        assert!(storage.iter().any(|cmd| cmd.label == "Settings › Storage"));
    }

    #[test]
    fn toggle_opens_and_run_closes_with_the_chosen_message() {
        let mut state = State::default();
        let _ = update(&mut state, Msg::Toggle);
        assert!(state.ui.palette.open);
        let _ = update(&mut state, Msg::Query("check for up".into()));
        let (_, chosen) = update(&mut state, Msg::Submit);
        assert!(matches!(chosen, Some(Message::CheckUpdates)));
        assert!(!state.ui.palette.open);
        // Moving while closed is ignored.
        let _ = update(&mut state, Msg::Move(3));
        assert_eq!(state.ui.palette.selected, 0);
    }
}
