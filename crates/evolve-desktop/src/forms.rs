//! Form controls: validated path fields, number fields and search fields.
//!
//! Path checks touch the disk only when a value changes (see
//! [`PathChecks::sync`]); views read the cached result.

use crate::settings::Preferences;
use crate::{Message, design, icons};
use iced::keyboard::Modifiers;
use iced::widget::{button, column, container, mouse_area, row, text};
use iced::{Alignment, Background, Border, Color, Element, Fill, Theme};
use std::collections::BTreeMap;
use std::path::Path;

// ---------------------------------------------------------------------------
// Path fields
// ---------------------------------------------------------------------------

/// Every path setting the manager checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PathKind {
    GameInstall,
    ClientExe,
    TestGame,
    TestClient,
    Blender,
    Ffdec,
    Ruffle,
    RuffleWeb,
    Ffmpeg,
    Vgmstream,
    OggRemux,
    FmodDesigner,
    Projects,
    AssetLibrary,
}

impl PathKind {
    pub const ALL: [Self; 14] = [
        Self::GameInstall,
        Self::ClientExe,
        Self::TestGame,
        Self::TestClient,
        Self::Blender,
        Self::Ffdec,
        Self::Ruffle,
        Self::RuffleWeb,
        Self::Ffmpeg,
        Self::Vgmstream,
        Self::OggRemux,
        Self::FmodDesigner,
        Self::Projects,
        Self::AssetLibrary,
    ];

    pub fn value(self, prefs: &Preferences) -> &str {
        match self {
            Self::GameInstall => &prefs.game_installation,
            Self::ClientExe => &prefs.client_executable,
            Self::TestGame => &prefs.test_game_installation,
            Self::TestClient => &prefs.test_client_executable,
            Self::Blender => &prefs.blender_executable,
            Self::Ffdec => &prefs.ffdec_executable,
            Self::Ruffle => &prefs.ruffle_executable,
            Self::RuffleWeb => &prefs.ruffle_web_runtime,
            Self::Ffmpeg => &prefs.gfx_ffmpeg_executable,
            Self::Vgmstream => &prefs.vgmstream_executable,
            Self::OggRemux => &prefs.oggvorbis_remux_executable,
            Self::FmodDesigner => &prefs.fmod_designer_executable,
            Self::Projects => &prefs.projects,
            Self::AssetLibrary => &prefs.asset_library_root,
        }
    }

    /// Browse picks a folder rather than a file.
    pub fn folder(self) -> bool {
        matches!(
            self,
            Self::GameInstall | Self::TestGame | Self::Projects | Self::AssetLibrary
        )
    }

    /// The message that stores a newly chosen value.
    pub fn set(self, value: String) -> Message {
        match self {
            Self::GameInstall => Message::PreferenceGameInstall(value),
            Self::ClientExe => Message::PreferenceClientExecutable(value),
            Self::TestGame => Message::PreferenceTestGameInstall(value),
            Self::TestClient => Message::PreferenceTestClientExecutable(value),
            Self::Blender => Message::PreferenceBlender(value),
            Self::Ffdec => Message::GfxConfigChanged(value),
            Self::Ruffle => Message::GfxRuffleConfigChanged(value),
            Self::RuffleWeb => Message::GfxRuffleWebConfigChanged(value),
            Self::Ffmpeg => Message::GfxFfmpegConfigChanged(value),
            Self::Vgmstream => Message::AudioDecoderPathChanged(value),
            Self::OggRemux => Message::AudioRemuxPathChanged(value),
            Self::FmodDesigner => Message::AudioDesignerPathChanged(value),
            Self::Projects => Message::PreferenceProjects(value),
            Self::AssetLibrary => Message::PreferenceAssets(value),
        }
    }
}

/// The result shown under a path field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathCheck {
    pub ok: bool,
    pub message: String,
}

impl PathCheck {
    fn found(message: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: message.into(),
        }
    }

    fn problem(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: message.into(),
        }
    }
}

/// Explains why `value` isn't the expected file.
fn file_problem(value: &str, expected: &str) -> PathCheck {
    let path = Path::new(value.trim());
    if !path.is_absolute() {
        return PathCheck::problem(format!("Use a full path ending in {expected}"));
    }
    match std::fs::metadata(path) {
        Err(_) => PathCheck::problem("File not found"),
        Ok(metadata) if metadata.is_dir() => {
            PathCheck::problem(format!("This is a folder; select {expected} inside it"))
        }
        Ok(metadata) if metadata.len() == 0 => PathCheck::problem("File is empty"),
        Ok(_) => PathCheck::problem(format!("Expected {expected}")),
    }
}

fn tool(found: Option<std::path::PathBuf>, value: &str, expected: &str) -> PathCheck {
    match found {
        Some(path) => PathCheck::found(format!(
            "Found {}",
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| expected.to_owned())
        )),
        None => file_problem(value, expected),
    }
}

fn folder(value: &str) -> PathCheck {
    let path = Path::new(value.trim());
    if !path.is_absolute() {
        PathCheck::problem("Use a full folder path")
    } else if path.is_dir() {
        PathCheck::found("Folder found")
    } else if path.exists() {
        PathCheck::problem("This is a file, not a folder")
    } else {
        PathCheck::problem("Folder not found")
    }
}

fn ffmpeg(value: &str) -> Option<std::path::PathBuf> {
    let path = Path::new(value.trim());
    let named = path
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("ffmpeg.exe"));
    (named && path.is_absolute())
        .then(|| std::fs::metadata(path).ok())
        .flatten()
        .filter(|metadata| metadata.is_file() && metadata.len() > 0)
        .map(|_| path.to_path_buf())
}

/// A replacement file for a signed member: it must exist.
pub fn check_existing_file(value: &str) -> Option<PathCheck> {
    if value.trim().is_empty() {
        return None;
    }
    let path = Path::new(value.trim());
    Some(match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => PathCheck::found("File found"),
        Ok(_) => PathCheck::problem("This is a folder, not a file"),
        Err(_) if !path.is_absolute() => PathCheck::problem("Use a full path"),
        Err(_) => PathCheck::problem("File not found"),
    })
}

/// A rebuild output: a new file whose name is exactly the original PAK's.
pub fn check_rebuild_output(value: &str, original: Option<&str>) -> Option<PathCheck> {
    if value.trim().is_empty() {
        return None;
    }
    let path = Path::new(value.trim());
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    Some(if !path.is_absolute() {
        PathCheck::problem("Use a full path")
    } else if path.exists() {
        PathCheck::problem("Choose a new path; this file already exists")
    } else if path.parent().is_none_or(|parent| !parent.is_dir()) {
        PathCheck::problem("Folder not found")
    } else {
        match original {
            Some(original) if !name.eq_ignore_ascii_case(original) => {
                PathCheck::problem(format!("Must end in the original name: {original}"))
            }
            Some(_) => PathCheck::found("New file with the original PAK name"),
            None if name.to_ascii_lowercase().ends_with(".pak") => {
                PathCheck::found("New .pak file (select a staged PAK to check its name)")
            }
            None => PathCheck::problem("Must end in the original .pak name"),
        }
    })
}

/// Checks one path. `None` for an empty value: nothing to report yet.
pub fn check(kind: PathKind, value: &str) -> Option<PathCheck> {
    if value.trim().is_empty() {
        return None;
    }
    use crate::{audio_tools, blender_install, gfx_bundled_tools, play_setup};
    Some(match kind {
        PathKind::GameInstall => match play_setup::game_root(value) {
            Ok(_) => PathCheck::found("Found Evolve.exe and inject.dll"),
            Err(error) => PathCheck::problem(error),
        },
        PathKind::ClientExe => match play_setup::client_exe(value) {
            Ok(_) => PathCheck::found("Found ModdedEvolveLauncher.exe"),
            Err(_) => file_problem(value, "ModdedEvolveLauncher.exe"),
        },
        PathKind::TestGame => match play_setup::test_game_root(value) {
            Ok(_) => PathCheck::found("Found bin64_SteamRetail/Evolve.exe"),
            Err(error) => PathCheck::problem(error),
        },
        PathKind::TestClient => match play_setup::test_client_exe(value) {
            Ok(path) => tool(Some(path), value, "the Test Client .exe"),
            Err(_) => file_problem(value, "the Test Client .exe"),
        },
        PathKind::Blender => tool(blender_install::valid_blender(value), value, "blender.exe"),
        PathKind::Ffdec => tool(
            gfx_bundled_tools::valid_ffdec(value),
            value,
            "ffdec-cli.exe or ffdec.jar",
        ),
        PathKind::Ruffle => tool(gfx_bundled_tools::valid_ruffle(value), value, "ruffle.exe"),
        PathKind::RuffleWeb => match gfx_bundled_tools::valid_ruffle_web(value) {
            Some(_) => PathCheck::found("Found ruffle.js with its WebAssembly runtime"),
            None => file_problem(value, "ruffle.js (next to its .wasm file)"),
        },
        PathKind::Ffmpeg => tool(ffmpeg(value), value, "ffmpeg.exe"),
        PathKind::Vgmstream => tool(
            audio_tools::valid_decoder(value),
            value,
            "vgmstream-cli.exe",
        ),
        PathKind::OggRemux => tool(
            audio_tools::valid_remuxer(value),
            value,
            "oggvorbis2fsb5.exe",
        ),
        PathKind::FmodDesigner => tool(
            audio_tools::valid_designer(value),
            value,
            "fmod_designer.exe",
        ),
        PathKind::Projects | PathKind::AssetLibrary => folder(value),
    })
}

/// Cached path checks, refreshed only when a value changes or work
/// finishes (an install or setup can make a path valid).
#[derive(Debug, Default)]
pub struct PathChecks {
    entries: BTreeMap<PathKind, (String, Option<PathCheck>)>,
    busy: bool,
    /// Checks for paths that aren't preferences, keyed by field.
    extra: BTreeMap<&'static str, (String, Option<PathCheck>)>,
}

impl PathChecks {
    /// Re-checks changed values. Returns how many paths were checked.
    pub fn sync(&mut self, prefs: &Preferences, busy: bool) -> usize {
        let finished_work = self.busy && !busy;
        self.busy = busy;
        let mut checked = 0;
        for kind in PathKind::ALL {
            let value = kind.value(prefs);
            let stale = finished_work
                || self
                    .entries
                    .get(&kind)
                    .is_none_or(|(previous, _)| previous != value);
            if stale {
                self.entries
                    .insert(kind, (value.to_owned(), check(kind, value)));
                checked += 1;
            }
        }
        checked
    }

    pub fn get(&self, kind: PathKind) -> Option<&PathCheck> {
        self.entries
            .get(&kind)
            .and_then(|(_, check)| check.as_ref())
    }

    /// Re-checks `field` only when `key` (its value plus anything the check
    /// depends on) changed.
    pub fn sync_extra(
        &mut self,
        field: &'static str,
        key: String,
        check: impl FnOnce() -> Option<PathCheck>,
    ) {
        if self.extra.get(field).is_none_or(|(old, _)| *old != key) {
            self.extra.insert(field, (key, check()));
        }
    }

    pub fn extra(&self, field: &str) -> Option<&PathCheck> {
        self.extra.get(field).and_then(|(_, check)| check.as_ref())
    }
}

/// A labelled path input with a Browse… button and a ✓/✕ line under it.
/// The border turns red while the path is wrong.
pub fn path_field<'a>(
    label: &'a str,
    placeholder: &'a str,
    kind: PathKind,
    prefs: &'a Preferences,
    checks: &'a PathChecks,
    browse: Message,
) -> Element<'a, Message> {
    path_input(
        label,
        placeholder,
        kind.value(prefs),
        checks.get(kind),
        move |value| kind.set(value),
        browse,
    )
}

/// [`path_field`] for a value that isn't a saved preference.
pub fn path_input<'a>(
    label: &'a str,
    placeholder: &'a str,
    value: &'a str,
    result: Option<&'a PathCheck>,
    on_input: impl Fn(String) -> Message + 'a,
    browse: Message,
) -> Element<'a, Message> {
    let bad = result.is_some_and(|check| !check.ok);
    let input = design::text_input(placeholder, value)
        .on_input(on_input)
        .width(Fill)
        .style(move |theme: &Theme, status| {
            let mut style = iced::widget::text_input::default(theme, status);
            style.background = Background::Color(design::field(theme));
            style.border.radius = design::radius(theme, design::Radius::Medium).into();
            style.placeholder = theme.extended_palette().background.strongest.text;
            if bad {
                style.border.color = theme.palette().danger;
                style.border.width = 1.5;
            }
            style
        });
    let mut field = column![
        design::label(label),
        row![
            input,
            design::button("Browse…")
                .on_press(browse)
                .style(design::secondary),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    ]
    .spacing(5);
    if let Some(result) = result {
        let ok = result.ok;
        let tint = move |theme: &Theme| {
            if ok {
                theme.palette().success
            } else {
                theme.palette().danger
            }
        };
        field = field.push(
            row![
                icons::glyph(if ok { icons::check() } else { icons::error() }, 13.0).style(
                    move |theme: &Theme, _| iced::widget::svg::Style {
                        color: Some(tint(theme)),
                    }
                ),
                text(result.message.as_str())
                    .size(design::CAPTION)
                    .style(move |theme: &Theme| text::Style {
                        color: Some(tint(theme)),
                    }),
            ]
            .spacing(5)
            .align_y(Alignment::Center),
        );
    }
    field.into()
}

// ---------------------------------------------------------------------------
// Number fields
// ---------------------------------------------------------------------------

/// Bounds and stepping for a number field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NumberSpec {
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub decimals: usize,
}

impl NumberSpec {
    pub const fn new(min: f64, max: f64, step: f64, decimals: usize) -> Self {
        Self {
            min,
            max,
            step,
            decimals,
        }
    }

    pub fn clamp(&self, value: f64) -> f64 {
        value.clamp(self.min, self.max)
    }
}

/// Last value that parsed, per field, so an unparsable entry can revert.
#[derive(Debug, Default)]
pub struct FormState {
    last_good: BTreeMap<String, f64>,
    drafts: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub enum FormMsg {
    /// A number field changed: remember `value` if it parsed, then run the
    /// field's own message.
    Edit(String, Option<f64>, Box<Message>),
    /// A bound number field's text changed: keep the draft so partial
    /// entries like `1.` survive, and run the message if it parsed.
    Draft(String, String, Option<Box<Message>>),
}

pub fn update(forms: &mut FormState, message: FormMsg) -> Option<Message> {
    match message {
        FormMsg::Edit(id, value, next) => {
            if let Some(value) = value {
                forms.last_good.insert(id, value);
            }
            Some(*next)
        }
        FormMsg::Draft(id, text, next) => {
            if let Some(value) = parse_finite(&text) {
                forms.last_good.insert(id.clone(), value);
            }
            forms.drafts.insert(id, text);
            next.map(|next| *next)
        }
    }
}

/// The text a bound field shows: the draft while it still describes the
/// current value (or is mid-entry), otherwise the value itself.
pub fn bound_text(forms: &FormState, id: &str, value: f64, decimals: usize) -> String {
    let tolerance = 10f64.powi(-(decimals as i32)) * 0.5 + 1e-9;
    match forms.drafts.get(id) {
        Some(draft)
            if parse_finite(draft).is_none_or(|parsed| (parsed - value).abs() < tolerance) =>
        {
            draft.clone()
        }
        _ => format_number(value, decimals),
    }
}

/// A number field bound to a numeric state value (a slider's companion).
/// Typing keeps a draft; each in-range entry runs `on_value`.
pub fn bound_number<'a, F>(
    forms: &FormState,
    modifiers: Modifiers,
    id: &'static str,
    value: f64,
    spec: NumberSpec,
    on_value: F,
) -> Element<'a, Message>
where
    F: Fn(f64) -> Message + Clone + 'a,
{
    let text = bound_text(forms, id, value, spec.decimals);
    number_field(forms, modifiers, id, &text, spec, move |raw: String| {
        let parsed = parse_finite(&raw)
            .filter(|value| (spec.min..=spec.max).contains(value))
            .map(|value| Box::new(on_value(value)));
        Message::Form(FormMsg::Draft(id.to_owned(), raw, parsed))
    })
}

/// True for text that is a number or on its way to one (`-`, `1.`, `.5`).
/// Anything else is refused as it's typed.
pub fn is_partial_number(raw: &str) -> bool {
    let body = raw.strip_prefix('-').unwrap_or(raw);
    let mut dots = 0;
    body.chars().all(|ch| {
        if ch == '.' {
            dots += 1;
            dots == 1
        } else {
            ch.is_ascii_digit()
        }
    })
}

pub fn parse_finite(raw: &str) -> Option<f64> {
    raw.trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

/// Formats with at most `decimals` places, without trailing zeros.
pub fn format_number(value: f64, decimals: usize) -> String {
    let text = format!("{value:.decimals$}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    };
    if text == "-0" { "0".into() } else { text }
}

/// The value kept when editing finishes: the typed number clamped, or the
/// last good value if the text doesn't parse.
pub fn committed(spec: NumberSpec, raw: &str, last_good: Option<f64>) -> f64 {
    spec.clamp(
        parse_finite(raw)
            .or(last_good)
            .unwrap_or_else(|| 0.0_f64.clamp(spec.min, spec.max)),
    )
}

/// Wheel and button step size: Shift ×10, Ctrl ×0.1.
pub fn step_multiplier(modifiers: Modifiers) -> f64 {
    if modifiers.shift() {
        10.0
    } else if modifiers.control() {
        0.1
    } else {
        1.0
    }
}

pub fn stepped(
    spec: NumberSpec,
    raw: &str,
    last_good: Option<f64>,
    direction: f64,
    modifiers: Modifiers,
) -> f64 {
    let base = committed(spec, raw, last_good);
    let next = base + direction * spec.step * step_multiplier(modifiers);
    let scale = 10f64.powi(spec.decimals.max(1) as i32 + 1);
    spec.clamp((next * scale).round() / scale)
}

fn record(id: &str, value: Option<f64>, next: Message) -> Message {
    Message::Form(FormMsg::Edit(id.to_owned(), value, Box::new(next)))
}

fn step_button<'a>(
    icon: iced::widget::svg::Handle,
    message: Option<Message>,
) -> Element<'a, Message> {
    button(icons::glyph(icon, 12.0))
        .padding([6, 6])
        .style(design::secondary)
        .on_press_maybe(message)
        .into()
}

/// A number input with −/+ buttons. Only numbers can be typed; the wheel
/// steps (Shift ×10, Ctrl ×0.1); Enter clamps to the range and restores
/// the last good value if the text doesn't parse.
pub fn number_field<'a, F>(
    forms: &FormState,
    modifiers: Modifiers,
    id: impl Into<String>,
    value: &str,
    spec: NumberSpec,
    on_change: F,
) -> Element<'a, Message>
where
    F: Fn(String) -> Message + Clone + 'a,
{
    number_field_submit(forms, modifiers, id, value, spec, on_change, None)
}

/// [`number_field`] whose Enter also runs `submit` after committing.
pub fn number_field_submit<'a, F>(
    forms: &FormState,
    modifiers: Modifiers,
    id: impl Into<String>,
    value: &str,
    spec: NumberSpec,
    on_change: F,
    submit: Option<Message>,
) -> Element<'a, Message>
where
    F: Fn(String) -> Message + Clone + 'a,
{
    // `text_input` copies its value, so the text needn't outlive the view.
    let id = id.into();
    let last = forms
        .last_good
        .get(&id)
        .copied()
        .or_else(|| parse_finite(value));
    let current = committed(spec, value, last);
    let make = |next: f64| {
        let text = format_number(next, spec.decimals);
        record(&id, Some(next), on_change(text))
    };
    let down = stepped(spec, value, last, -1.0, modifiers);
    let up = stepped(spec, value, last, 1.0, modifiers);
    let down_msg = (current > spec.min).then(|| make(down));
    let up_msg = (current < spec.max).then(|| make(up));
    let commit_msg = match submit {
        Some(submit) => Message::Sequence(vec![make(committed(spec, value, last)), submit]),
        None => make(committed(spec, value, last)),
    };
    let input = {
        let on_change = on_change.clone();
        let id = id.clone();
        let previous = value.to_owned();
        design::text_input("", value)
            .on_input(move |raw: String| {
                if is_partial_number(&raw) {
                    record(&id, parse_finite(&raw), on_change(raw))
                } else {
                    on_change(previous.clone())
                }
            })
            .on_submit(commit_msg)
            .width(Fill)
    };
    let wheel = {
        let up_msg = up_msg.clone();
        let down_msg = down_msg.clone();
        mouse_area(input).on_scroll(move |delta| {
            let y = match delta {
                iced::mouse::ScrollDelta::Lines { y, .. }
                | iced::mouse::ScrollDelta::Pixels { y, .. } => y,
            };
            let chosen = if y > 0.0 { &up_msg } else { &down_msg };
            chosen
                .clone()
                .unwrap_or(Message::Chrome(crate::chrome::Msg::Absorb))
        })
    };
    row![
        step_button(icons::minus(), down_msg),
        wheel,
        step_button(icons::plus(), up_msg),
    ]
    .spacing(3)
    .align_y(Alignment::Center)
    .into()
}

/// One number field per component of a comma-separated vector
/// (`"1, 2, 3"`); each edit rewrites the whole vector.
#[allow(clippy::too_many_arguments)]
pub fn vector_fields<'a, F>(
    forms: &FormState,
    modifiers: Modifiers,
    id: &str,
    value: &str,
    labels: &'static [&'static str],
    spec: NumberSpec,
    on_change: F,
    submit: Option<Message>,
) -> Element<'a, Message>
where
    F: Fn(String) -> Message + Clone + 'a,
{
    let mut parts: Vec<String> = value
        .split(',')
        .map(|part| part.trim().to_owned())
        .collect();
    parts.resize(labels.len(), String::new());
    let mut fields = row![].spacing(8);
    for (index, label) in labels.iter().enumerate() {
        let parts_for_edit = parts.clone();
        let on_change = on_change.clone();
        let component = parts[index].clone();
        let field = number_field_submit(
            forms,
            modifiers,
            format!("{id}.{index}"),
            &component,
            spec,
            move |raw: String| {
                let mut next = parts_for_edit.clone();
                next[index] = raw;
                on_change(next.join(", "))
            },
            submit.clone(),
        );
        fields = fields.push(
            column![
                text(*label).size(design::CAPTION).style(design::muted),
                field
            ]
            .spacing(3)
            .width(Fill),
        );
    }
    fields.into()
}

// ---------------------------------------------------------------------------
// Search field
// ---------------------------------------------------------------------------

/// A search box with a magnifier, a match count and a ✕ that clears it.
/// Esc also clears it (handled in `main.rs`).
pub fn search_field<'a, F>(
    placeholder: &'a str,
    value: &'a str,
    matches: Option<usize>,
    on_change: F,
) -> Element<'a, Message>
where
    F: Fn(String) -> Message + Clone + 'a,
{
    let clear = on_change("".into());
    let input = iced::widget::text_input(placeholder, value)
        .on_input(on_change)
        .padding([7, 4])
        .width(Fill)
        .style(|theme: &Theme, status| {
            let mut style = iced::widget::text_input::default(theme, status);
            style.background = Background::Color(Color::TRANSPARENT);
            style.border = Border::default();
            style.placeholder = theme.extended_palette().background.strongest.text;
            style
        });
    let mut bar = row![icons::glyph(icons::search(), 15.0), input]
        .spacing(4)
        .align_y(Alignment::Center);
    if !value.is_empty() {
        if let Some(count) = matches {
            bar = bar.push(
                text(match count {
                    1 => "1 match".to_owned(),
                    count => format!("{count} matches"),
                })
                .size(design::CAPTION)
                .style(design::muted),
            );
        }
        bar = bar.push(design::tooltip(
            button(icons::glyph(icons::close(), 13.0))
                .padding(4)
                .style(design::text_button)
                .on_press(clear),
            "Clear search",
            Some("Esc"),
            iced::widget::tooltip::Position::Bottom,
        ));
    }
    container(bar)
        .padding([0, 10])
        .width(Fill)
        .style(|theme: &Theme| container::Style {
            background: Some(Background::Color(design::field(theme))),
            border: Border {
                color: design::border(theme),
                width: 1.0,
                radius: design::radius(theme, design::Radius::Medium).into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// Byte ranges of case-insensitive matches of `query` in `content`.
pub fn match_ranges(content: &str, query: &str) -> Vec<std::ops::Range<usize>> {
    let query = query.trim();
    if query.is_empty() || !query.is_ascii() {
        return Vec::new();
    }
    let haystack = content.to_ascii_lowercase();
    let needle = query.to_ascii_lowercase();
    let mut ranges = Vec::new();
    let mut from = 0;
    while let Some(found) = haystack[from..].find(&needle) {
        let start = from + found;
        let end = start + needle.len();
        ranges.push(start..end);
        from = end;
    }
    ranges
}

/// Row text with the matching pieces on a faint accent background.
pub fn highlight<'a>(content: &str, query: &str, size: f32, accent: Color) -> Element<'a, Message> {
    let ranges = match_ranges(content, query);
    if ranges.is_empty() {
        return text(content.to_owned()).size(size).into();
    }
    let mut spans: Vec<iced::widget::text::Span<'a, ()>> = Vec::new();
    let mut at = 0;
    for range in ranges {
        if range.start > at {
            spans.push(iced::widget::span(content[at..range.start].to_owned()));
        }
        spans.push(
            iced::widget::span(content[range.clone()].to_owned())
                .background(design::with_alpha(accent, 0.28))
                .border(Border {
                    radius: 2.0.into(),
                    ..Border::default()
                }),
        );
        at = range.end;
    }
    if at < content.len() {
        spans.push(iced::widget::span(content[at..].to_owned()));
    }
    iced::widget::rich_text(spans).size(size).into()
}

/// The accent used for search highlights, from the saved appearance.
pub fn highlight_color(prefs: &Preferences) -> Color {
    design::theme(prefs).palette().primary
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: NumberSpec = NumberSpec::new(-10.0, 10.0, 0.5, 2);

    #[test]
    fn partial_numbers_are_accepted_and_letters_refused() {
        for ok in ["", "-", ".", "-.", "1.", "-1.25", "12"] {
            assert!(is_partial_number(ok), "{ok}");
        }
        for bad in ["a", "1a", "1..2", "--1", "1-", "1,2", "1e5"] {
            assert!(!is_partial_number(bad), "{bad}");
        }
    }

    #[test]
    fn committing_clamps_and_reverts_unparsable_text() {
        assert_eq!(committed(SPEC, "50", None), 10.0);
        assert_eq!(committed(SPEC, "-", Some(3.0)), 3.0);
        assert_eq!(committed(SPEC, "", None), 0.0);
        assert_eq!(committed(SPEC, "2.5", Some(1.0)), 2.5);
    }

    #[test]
    fn steps_follow_modifiers_and_bounds() {
        let none = Modifiers::empty();
        assert_eq!(stepped(SPEC, "1", None, 1.0, none), 1.5);
        assert_eq!(stepped(SPEC, "1", None, 1.0, Modifiers::SHIFT), 6.0);
        assert_eq!(stepped(SPEC, "1", None, -1.0, Modifiers::CTRL), 0.95);
        assert_eq!(stepped(SPEC, "9.8", None, 1.0, none), 10.0);
    }

    #[test]
    fn numbers_format_without_trailing_zeros() {
        assert_eq!(format_number(1.5, 3), "1.5");
        assert_eq!(format_number(2.0, 3), "2");
        assert_eq!(format_number(-0.0001, 2), "0");
        assert_eq!(format_number(7.0, 0), "7");
    }

    #[test]
    fn bound_fields_keep_partial_drafts_until_the_value_moves() {
        let mut forms = FormState::default();
        let none = update(&mut forms, FormMsg::Draft("r".into(), "1.".into(), None));
        assert!(none.is_none());
        assert_eq!(bound_text(&forms, "r", 1.0, 1), "1.");
        // The value changed elsewhere (a slider): the field follows it.
        assert_eq!(bound_text(&forms, "r", 7.5, 1), "7.5");
        assert_eq!(bound_text(&forms, "unknown", 2.0, 1), "2");
    }

    #[test]
    fn match_ranges_are_case_insensitive_and_disjoint() {
        assert_eq!(match_ranges("Emet/emet.CDF", "emet"), vec![0..4, 5..9]);
        assert!(match_ranges("abc", "").is_empty());
        assert!(match_ranges("abc", "x").is_empty());
    }

    #[test]
    fn path_checks_report_missing_and_found_folders() {
        let dir = tempfile::tempdir().unwrap();
        let found = check(PathKind::Projects, dir.path().to_str().unwrap()).unwrap();
        assert!(found.ok);
        let missing = check(
            PathKind::Projects,
            dir.path().join("missing").to_str().unwrap(),
        )
        .unwrap();
        assert_eq!(missing.message, "Folder not found");
        assert!(check(PathKind::Projects, "  ").is_none());
        let relative = check(PathKind::Blender, "blender.exe").unwrap();
        assert!(!relative.ok);
    }

    #[test]
    fn rebuild_output_must_be_new_and_keep_the_original_name() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("objects.pak");
        let wrong = dir.path().join("other.pak");
        assert!(check_rebuild_output(good.to_str().unwrap(), Some("Objects.pak")).unwrap().ok);
        assert!(!check_rebuild_output(wrong.to_str().unwrap(), Some("objects.pak")).unwrap().ok);
        std::fs::write(&good, b"x").unwrap();
        assert!(!check_rebuild_output(good.to_str().unwrap(), Some("objects.pak")).unwrap().ok);
        assert!(check_rebuild_output("", None).is_none());
        assert!(check_existing_file(good.to_str().unwrap()).unwrap().ok);
    }

    #[test]
    fn path_checks_only_rerun_on_change_or_finished_work() {
        let mut prefs = Preferences::default();
        let mut checks = PathChecks::default();
        assert_eq!(checks.sync(&prefs, false), PathKind::ALL.len());
        assert_eq!(checks.sync(&prefs, false), 0);
        prefs.projects = "C:/elsewhere".into();
        assert_eq!(checks.sync(&prefs, false), 1);
        assert_eq!(checks.sync(&prefs, true), 0);
        assert_eq!(checks.sync(&prefs, false), PathKind::ALL.len());
    }
}
