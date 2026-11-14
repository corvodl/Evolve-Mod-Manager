//! Rebindable app-wide shortcuts, set in Settings › Shortcuts.
//!
//! `event::listen_with` takes a plain `fn`, so the current bindings reach
//! the key handler through `Subscription::with` instead of a capture.

use crate::settings::Preferences;
use crate::{Message, command_palette, ui_kit};
use iced::keyboard::{Key, Modifiers, key::Named};
use iced::{Subscription, window};
use std::fmt;
use std::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    CommandPalette,
    ToggleSidebar,
}

impl Action {
    pub const ALL: [Self; 2] = [Self::CommandPalette, Self::ToggleSidebar];

    fn id(self) -> &'static str {
        match self {
            Self::CommandPalette => "command_palette",
            Self::ToggleSidebar => "toggle_sidebar",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::CommandPalette => "Command palette",
            Self::ToggleSidebar => "Open or collapse the sidebar",
        }
    }

    /// Labels that buttons and menus look up through `chrome::shortcut_for`.
    pub fn labels(self) -> &'static [&'static str] {
        match self {
            Self::CommandPalette => &["Command palette"],
            Self::ToggleSidebar => &["Expand sidebar", "Collapse sidebar"],
        }
    }

    pub fn default_chord(self) -> Chord {
        let ctrl = |key: &str| Chord {
            ctrl: true,
            alt: false,
            shift: false,
            key: key.into(),
        };
        match self {
            Self::CommandPalette => ctrl("Space"),
            Self::ToggleSidebar => ctrl("B"),
        }
    }

    fn message(self) -> Message {
        match self {
            Self::CommandPalette => {
                Message::Ui(ui_kit::UiMsg::Palette(command_palette::Msg::Toggle))
            }
            Self::ToggleSidebar => Message::SidebarToggle,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Chord {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// "Space", "Up", "F5", or an upper-case character.
    pub key: String,
}

impl Chord {
    pub fn parse(text: &str) -> Option<Self> {
        let mut chord = Self {
            ctrl: false,
            alt: false,
            shift: false,
            key: String::new(),
        };
        for part in text.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => chord.ctrl = true,
                "alt" => chord.alt = true,
                "shift" => chord.shift = true,
                "" => return None,
                _ if chord.key.is_empty() => chord.key = normalize_key(part),
                _ => return None,
            }
        }
        (!chord.key.is_empty()).then_some(chord)
    }

    pub fn from_key(key: &Key, modifiers: Modifiers) -> Option<Self> {
        let key = match key {
            Key::Character(value) => value.to_uppercase(),
            Key::Named(named) => named_key(*named)?.to_owned(),
            Key::Unidentified => return None,
        };
        Some(Self {
            ctrl: modifiers.control(),
            alt: modifiers.alt(),
            shift: modifiers.shift(),
            key,
        })
    }

    /// Plain keys and Shift+key would fire while typing in a field.
    fn is_bindable(&self) -> bool {
        self.ctrl || self.alt || is_function_key(&self.key)
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (on, name) in [
            (self.ctrl, "Ctrl+"),
            (self.alt, "Alt+"),
            (self.shift, "Shift+"),
        ] {
            if on {
                f.write_str(name)?;
            }
        }
        f.write_str(&self.key)
    }
}

fn normalize_key(part: &str) -> String {
    if part.chars().count() == 1 {
        part.to_uppercase()
    } else {
        let mut chars = part.chars();
        chars
            .next()
            .map(|first| {
                first
                    .to_uppercase()
                    .chain(chars.flat_map(char::to_lowercase))
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn is_function_key(key: &str) -> bool {
    key.strip_prefix('F')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=12).contains(&n))
}

/// Modifier-only presses return `None` so recording waits for the real key.
fn named_key(named: Named) -> Option<&'static str> {
    Some(match named {
        Named::Space => "Space",
        Named::Enter => "Enter",
        Named::Tab => "Tab",
        Named::Escape => "Esc",
        Named::Backspace => "Backspace",
        Named::Delete => "Delete",
        Named::Insert => "Insert",
        Named::Home => "Home",
        Named::End => "End",
        Named::PageUp => "Pageup",
        Named::PageDown => "Pagedown",
        Named::ArrowUp => "Up",
        Named::ArrowDown => "Down",
        Named::ArrowLeft => "Left",
        Named::ArrowRight => "Right",
        Named::F1 => "F1",
        Named::F2 => "F2",
        Named::F3 => "F3",
        Named::F4 => "F4",
        Named::F5 => "F5",
        Named::F6 => "F6",
        Named::F7 => "F7",
        Named::F8 => "F8",
        Named::F9 => "F9",
        Named::F10 => "F10",
        Named::F11 => "F11",
        Named::F12 => "F12",
        _ => return None,
    })
}

pub fn chord(prefs: &Preferences, action: Action) -> Chord {
    prefs
        .keybinds
        .get(action.id())
        .and_then(|text| Chord::parse(text))
        .filter(Chord::is_bindable)
        .unwrap_or_else(|| action.default_chord())
}

pub fn is_custom(prefs: &Preferences, action: Action) -> bool {
    chord(prefs, action) != action.default_chord()
}

/// Why `chord` can't be used for `action`, if it can't.
pub fn conflict(prefs: &Preferences, action: Action, chord: &Chord) -> Option<String> {
    if !chord.is_bindable() {
        return Some(
            "Use Ctrl or Alt with the key (or F1–F12), so typing in a field isn't affected.".into(),
        );
    }
    if chord.key == "Esc" || chord.key == "Tab" {
        return Some(format!(
            "{chord} is reserved for closing and moving between fields."
        ));
    }
    if let Some(other) = Action::ALL
        .into_iter()
        .find(|other| *other != action && self::chord(prefs, *other) == *chord)
    {
        return Some(format!(
            "{chord} is already used for {}.",
            other.title().to_lowercase()
        ));
    }
    let shown = chord.to_string();
    crate::chrome::SHORTCUTS
        .iter()
        .filter(|(label, _)| !Action::ALL.iter().any(|a| a.labels().contains(label)))
        .find(|(_, keys)| keys.eq_ignore_ascii_case(&shown))
        .map(|(label, _)| format!("{shown} is already used for {}.", label.to_lowercase()))
}

pub fn set(prefs: &mut Preferences, action: Action, chord: &Chord) {
    if *chord == action.default_chord() {
        prefs.keybinds.remove(action.id());
    } else {
        prefs.keybinds.insert(action.id().into(), chord.to_string());
    }
    publish(prefs);
}

pub fn reset(prefs: &mut Preferences, action: Option<Action>) {
    match action {
        Some(action) => {
            prefs.keybinds.remove(action.id());
        }
        None => prefs.keybinds.clear(),
    }
    publish(prefs);
}

/// Hints shown on buttons and menus are `&'static str` everywhere, so each
/// rebinding leaks its few-byte label once.
static PUBLISHED: RwLock<Vec<(&'static str, &'static str)>> = RwLock::new(Vec::new());

pub fn publish(prefs: &Preferences) {
    let mut labels = Vec::new();
    for action in Action::ALL {
        let shown: &'static str = Box::leak(chord(prefs, action).to_string().into_boxed_str());
        labels.extend(action.labels().iter().map(|label| (*label, shown)));
    }
    if let Ok(mut published) = PUBLISHED.write() {
        *published = labels;
    }
}

/// The current shortcut for a rebindable action label.
pub fn shortcut_for(label: &str) -> Option<&'static str> {
    let published = PUBLISHED.read().ok()?;
    if published.is_empty() {
        drop(published);
        return Action::ALL
            .into_iter()
            .find(|action| {
                action
                    .labels()
                    .iter()
                    .any(|l| l.eq_ignore_ascii_case(label))
            })
            .map(|action| default_label(action));
    }
    published
        .iter()
        .find(|(action, _)| action.eq_ignore_ascii_case(label))
        .map(|(_, keys)| *keys)
}

fn default_label(action: Action) -> &'static str {
    match action {
        Action::CommandPalette => "Ctrl+Space",
        Action::ToggleSidebar => "Ctrl+B",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Bindings {
    chords: Vec<(Action, Chord)>,
    recording: bool,
}

pub fn subscription(prefs: &Preferences, recording: bool) -> Subscription<Message> {
    let bindings = Bindings {
        chords: Action::ALL
            .into_iter()
            .map(|a| (a, chord(prefs, a)))
            .collect(),
        recording,
    };
    iced::event::listen_with(|event, _status, id| match event {
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) => {
            Some((id, key, modifiers))
        }
        _ => None,
    })
    .with(bindings)
    .filter_map(resolve)
}

/// Bound chords fire even while a text field has focus.
fn resolve(
    (bindings, (_id, key, modifiers)): (Bindings, (window::Id, Key, Modifiers)),
) -> Option<Message> {
    let chord = Chord::from_key(&key, modifiers)?;
    if bindings.recording {
        return (chord.key != "Esc").then_some(Message::KeybindCaptured(chord));
    }
    bindings
        .chords
        .iter()
        .find(|(_, bound)| *bound == chord)
        .map(|(action, _)| action.message())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_round_trip_and_normalize() {
        let chord = Chord::parse("ctrl+space").unwrap();
        assert_eq!(chord.to_string(), "Ctrl+Space");
        assert_eq!(
            Chord::parse("Ctrl+Shift+k").unwrap().to_string(),
            "Ctrl+Shift+K"
        );
        assert_eq!(Chord::parse("Ctrl+"), None);
        assert_eq!(Chord::parse("Ctrl+A+B"), None);
    }

    #[test]
    fn defaults_and_plain_keys() {
        let prefs = Preferences::default();
        assert_eq!(
            chord(&prefs, Action::CommandPalette).to_string(),
            "Ctrl+Space"
        );
        assert_eq!(chord(&prefs, Action::ToggleSidebar).to_string(), "Ctrl+B");
        let plain = Chord::parse("K").unwrap();
        assert!(conflict(&prefs, Action::CommandPalette, &plain).is_some());
        assert!(conflict(&prefs, Action::CommandPalette, &Chord::parse("F2").unwrap()).is_none());
    }

    #[test]
    fn conflicts_with_other_bindings_and_fixed_shortcuts() {
        let prefs = Preferences::default();
        let sidebar = Chord::parse("Ctrl+B").unwrap();
        assert!(conflict(&prefs, Action::CommandPalette, &sidebar).is_some());
        let undo = Chord::parse("Ctrl+Z").unwrap();
        assert!(conflict(&prefs, Action::CommandPalette, &undo).is_some());
        let free = Chord::parse("Ctrl+K").unwrap();
        assert!(conflict(&prefs, Action::CommandPalette, &free).is_none());
    }
}
