//! Bounded, read-only CryEngine `.animevents` timeline parsing.
//!
//! The authored schema is
//! `<anim_event_list><animation name="…caf"><event …/></animation></anim_event_list>`.
//! Times follow `CAnimEventData` (`CryCharAnimationParams.h:245-324`): normalized
//! `[0, 1]`, and a missing `endTime` means a point event at `time`. Clamping is
//! reported as a diagnostic instead of being applied silently. Nothing here
//! executes or resolves the referenced sound, particle, ragdoll, force-feedback
//! or flow-graph event, and no source file is written.
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

/// Maximum accepted `.animevents` document size. The shipped 3.5.8 human file
/// is under 1 MiB, so this leaves headroom without allowing unbounded input.
pub const MAX_ANIMATION_EVENT_BYTES: usize = 8 * 1024 * 1024;
/// Maximum number of `<animation>` timelines kept from one document.
pub const MAX_ANIMATION_TIMELINES: usize = 32_768;
/// Maximum number of `<event>` elements accepted from one document.
pub const MAX_ANIMATION_EVENTS: usize = 100_000;
/// Maximum number of events attached to one `<animation>` timeline.
pub const MAX_EVENTS_PER_ANIMATION: usize = 4_096;
/// Maximum size of one authored event string or attribute value.
pub const MAX_EVENT_STRING_BYTES: usize = 1_024;
/// Maximum size of a normalized CAF path used as the index key.
pub const MAX_CAF_PATH_BYTES: usize = 512;
/// Diagnostics are bounded so a hostile document cannot exhaust memory.
pub const MAX_ANIMATION_EVENT_DIAGNOSTICS: usize = 4_096;

const MAX_DEPTH: usize = 8;
const MAX_VECTOR_COMPONENT: f32 = 1_000_000.0;
const EVENT_ATTRIBUTES: [&str; 8] = [
    "name",
    "time",
    "endTime",
    "parameter",
    "bone",
    "model",
    "offset",
    "dir",
];

#[derive(Debug, thiserror::Error)]
pub enum AnimationEventError {
    #[error("Animation event file exceeds the {limit}-byte limit")]
    FileTooLarge { limit: usize },
    #[error("Animation event file uses unsafe XML entities")]
    UnsafeXml,
    #[error("Malformed animation event XML: {0}")]
    Xml(String),
    #[error("Animation event file exceeds {limit} animation timelines")]
    TooManyTimelines { limit: usize },
    #[error("Animation event file exceeds {limit} events")]
    TooManyEvents { limit: usize },
    #[error("Animation {caf_path} exceeds {limit} events")]
    TooManyEventsForAnimation { caf_path: String, limit: usize },
    #[error("Animation event field {field} is {bytes} bytes, exceeding the {limit}-byte limit")]
    StringTooLong {
        field: String,
        bytes: usize,
        limit: usize,
    },
    #[error("Invalid game asset path {path:?}: {reason}")]
    UnsafePath { path: String, reason: String },
    #[error("I/O failure: {0}")]
    Io(#[from] io::Error),
}

/// One authored animation event, with the engine's normalized timing contract.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnimationEvent {
    /// Authored event name, preserved with its original casing.
    pub name: String,
    /// Normalized start time in `[0, 1]`.
    pub normalized_time: f32,
    /// Normalized end time in `[normalized_time, 1]`; equal to the start for a
    /// point event or when the sidecar omits `endTime`.
    pub normalized_end_time: f32,
    /// Authored custom parameter, whose meaning depends on the event name.
    pub parameter: String,
    /// Authored bone path, empty when the event is not attached to a bone.
    pub bone: String,
    /// Authored model path, empty when no model is referenced.
    pub model: String,
    /// Authored local offset, preserved in source units.
    pub offset: [f32; 3],
    /// Authored direction, preserved in source units.
    pub dir: [f32; 3],
}

impl AnimationEvent {
    /// Whether the authored event spans a time range rather than a point.
    pub fn is_range(&self) -> bool {
        self.normalized_end_time > self.normalized_time
    }

    /// Marker position in seconds for a clip of the given duration.
    pub fn start_seconds(&self, clip_duration_seconds: f32) -> f32 {
        self.normalized_time * clip_duration_seconds.max(0.0)
    }

    /// Range end in seconds for a clip of the given duration.
    pub fn end_seconds(&self, clip_duration_seconds: f32) -> f32 {
        self.normalized_end_time * clip_duration_seconds.max(0.0)
    }
}

/// All authored events for one CAF path, in source order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AnimationEventTimeline {
    /// Full path as authored, preserved for display and diagnostics.
    pub source_path: String,
    /// Full normalized CAF path used as the database key (lowercase, `/`).
    pub caf_path: String,
    /// Authored events, including duplicate `(name, time)` pairs.
    pub events: Vec<AnimationEvent>,
}

/// A non-fatal authored-data problem. The affected value is clamped or
/// defaulted, never silently repaired.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnimationEventDiagnostic {
    ClampedTime {
        caf_path: String,
        /// 0-based event position within its `<animation>` element.
        event_index: usize,
        original: f32,
        clamped: f32,
    },
    ClampedEndTime {
        caf_path: String,
        event_index: usize,
        original: f32,
        clamped: f32,
    },
    EndBeforeStart {
        caf_path: String,
        event_index: usize,
        start: f32,
        end: f32,
    },
    InvalidNumber {
        caf_path: String,
        event_index: usize,
        attribute: String,
        value: String,
    },
    InvalidVector {
        caf_path: String,
        event_index: usize,
        attribute: String,
        value: String,
    },
    MissingTime {
        caf_path: String,
        event_index: usize,
    },
    MissingEventName {
        caf_path: String,
        event_index: usize,
    },
    MissingCafPath {
        animation_index: usize,
    },
    UnknownAttribute {
        caf_path: String,
        event_index: usize,
        attribute: String,
    },
    UnsafeCafPath {
        path: String,
        reason: String,
    },
}

impl fmt::Display for AnimationEventDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClampedTime {
                caf_path,
                event_index,
                original,
                clamped,
            } => write!(
                f,
                "{caf_path} event {event_index}: time {original} clamped to {clamped}"
            ),
            Self::ClampedEndTime {
                caf_path,
                event_index,
                original,
                clamped,
            } => write!(
                f,
                "{caf_path} event {event_index}: endTime {original} clamped to {clamped}"
            ),
            Self::EndBeforeStart {
                caf_path,
                event_index,
                start,
                end,
            } => write!(
                f,
                "{caf_path} event {event_index}: endTime {end} is before time {start}; using {start}"
            ),
            Self::InvalidNumber {
                caf_path,
                event_index,
                attribute,
                value,
            } => write!(
                f,
                "{caf_path} event {event_index}: {attribute} value {value:?} is not a finite number; using the default"
            ),
            Self::InvalidVector {
                caf_path,
                event_index,
                attribute,
                value,
            } => write!(
                f,
                "{caf_path} event {event_index}: {attribute} value {value:?} is not three finite components; using zero"
            ),
            Self::MissingTime {
                caf_path,
                event_index,
            } => write!(
                f,
                "{caf_path} event {event_index}: time is missing; using 0"
            ),
            Self::MissingEventName {
                caf_path,
                event_index,
            } => write!(
                f,
                "{caf_path} event {event_index}: name is missing; event ignored"
            ),
            Self::MissingCafPath { animation_index } => {
                write!(
                    f,
                    "animation {animation_index}: name is missing; events ignored"
                )
            }
            Self::UnknownAttribute {
                caf_path,
                event_index,
                attribute,
            } => write!(
                f,
                "{caf_path} event {event_index}: unknown attribute {attribute:?} preserved as diagnostic only"
            ),
            Self::UnsafeCafPath { path, reason } => {
                write!(f, "animation path {path:?} rejected: {reason}")
            }
        }
    }
}

/// Parsed timelines and non-fatal diagnostics for one `.animevents` document.
#[derive(Clone, Debug, Default)]
pub struct AnimationEventDatabase {
    timelines: Vec<AnimationEventTimeline>,
    by_caf_path: BTreeMap<String, Vec<usize>>,
    diagnostics: Vec<AnimationEventDiagnostic>,
    diagnostics_truncated: bool,
}

impl AnimationEventDatabase {
    /// Timelines in authored source order. Duplicate animation entries remain
    /// separate rather than being merged.
    pub fn timelines(&self) -> &[AnimationEventTimeline] {
        &self.timelines
    }

    pub fn diagnostics(&self) -> &[AnimationEventDiagnostic] {
        &self.diagnostics
    }

    pub fn diagnostics_truncated(&self) -> bool {
        self.diagnostics_truncated
    }

    pub fn len(&self) -> usize {
        self.timelines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.timelines.is_empty()
    }

    pub fn event_count(&self) -> usize {
        self.timelines
            .iter()
            .map(|timeline| timeline.events.len())
            .sum()
    }

    /// Look up the first timeline for a full CAF path. The query is normalized
    /// exactly like the authored names, so a basename-only query never matches.
    pub fn timeline(&self, caf_path: &str) -> Option<&AnimationEventTimeline> {
        self.timelines_for(caf_path).next()
    }

    /// Every timeline for a full CAF path, preserving duplicate entries and
    /// source order.
    pub fn timelines_for<'a>(
        &'a self,
        caf_path: &str,
    ) -> impl Iterator<Item = &'a AnimationEventTimeline> + 'a {
        let ids = normalize_animation_path(caf_path)
            .ok()
            .and_then(|key| self.by_caf_path.get(&key))
            .map_or(&[][..], Vec::as_slice);
        ids.iter().filter_map(|&id| self.timelines.get(id))
    }
}

/// Normalize an authored CAF path for case-insensitive, full-path matching.
///
/// Backslashes become forward slashes, `.` segments and duplicate separators
/// are removed, and the result is ASCII-lowercased. Empty, absolute, drive,
/// colon and `..` traversal paths are rejected so sidecar names cannot escape
/// the asset root.
pub fn normalize_animation_path(path: &str) -> Result<String, AnimationEventError> {
    let trimmed = path.trim();
    let unsafe_path = |reason: &str| AnimationEventError::UnsafePath {
        path: path.to_string(),
        reason: reason.to_string(),
    };
    if trimmed.is_empty() {
        return Err(unsafe_path("empty path"));
    }
    if trimmed.len() > MAX_CAF_PATH_BYTES {
        return Err(unsafe_path("path exceeds the byte limit"));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(unsafe_path("path contains a control character"));
    }
    let unified = trimmed.replace('\\', "/");
    if unified.starts_with('/') {
        return Err(unsafe_path("absolute paths are not allowed"));
    }
    if unified.as_bytes().get(1) == Some(&b':') {
        return Err(unsafe_path("drive prefixes are not allowed"));
    }
    let mut segments = Vec::new();
    for segment in unified.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            return Err(unsafe_path("parent traversal is not allowed"));
        }
        if segment.contains(':') {
            return Err(unsafe_path("colons are not allowed in path segments"));
        }
        segments.push(segment);
    }
    if segments.is_empty() {
        return Err(unsafe_path("empty path"));
    }
    let normalized = segments.join("/").to_ascii_lowercase();
    if normalized.len() > MAX_CAF_PATH_BYTES {
        return Err(unsafe_path("path exceeds the byte limit"));
    }
    Ok(normalized)
}

/// Parse one `.animevents` document read-only. Any boundedness violation or
/// malformed XML fails closed with an error; the caller gets no partial data.
pub fn parse_animation_events(bytes: &[u8]) -> Result<AnimationEventDatabase, AnimationEventError> {
    if bytes.len() > MAX_ANIMATION_EVENT_BYTES {
        return Err(AnimationEventError::FileTooLarge {
            limit: MAX_ANIMATION_EVENT_BYTES,
        });
    }
    if bytes.is_empty() {
        return Err(AnimationEventError::Xml("empty document".into()));
    }
    let decoded = crate::cryxml::xml_bytes(bytes, MAX_ANIMATION_EVENT_BYTES)
        .map_err(|error| AnimationEventError::Xml(error.to_string()))?;
    let bytes = decoded.as_ref();
    if bytes
        .windows(9)
        .any(|window| window.eq_ignore_ascii_case(b"<!DOCTYPE"))
        || bytes
            .windows(8)
            .any(|window| window.eq_ignore_ascii_case(b"<!ENTITY"))
    {
        return Err(AnimationEventError::UnsafeXml);
    }
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);
    let mut state = ParseState::default();
    let mut stack: Vec<Vec<u8>> = Vec::new();
    loop {
        match reader
            .read_event()
            .map_err(|error| AnimationEventError::Xml(error.to_string()))?
        {
            Event::Start(element) => {
                if stack.len() >= MAX_DEPTH {
                    return Err(AnimationEventError::UnsafeXml);
                }
                state.element(&element, stack.len())?;
                stack.push(element.name().as_ref().to_vec());
            }
            Event::Empty(element) => {
                let depth = stack.len();
                state.element(&element, depth)?;
                if depth == 1 && element.name().as_ref().eq_ignore_ascii_case(b"animation") {
                    state.current = None;
                }
            }
            Event::End(element) => {
                let Some(previous) = stack.pop() else {
                    return Err(AnimationEventError::Xml(
                        "unexpected closing element".into(),
                    ));
                };
                if !previous.eq_ignore_ascii_case(element.name().as_ref()) {
                    return Err(AnimationEventError::Xml(
                        "mismatched closing element".into(),
                    ));
                }
                if stack.len() == 1 && previous.eq_ignore_ascii_case(b"animation") {
                    state.current = None;
                }
            }
            Event::DocType(_) | Event::GeneralRef(_) => {
                return Err(AnimationEventError::UnsafeXml);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !state.root_seen {
        return Err(AnimationEventError::Xml(
            "missing anim_event_list root element".into(),
        ));
    }
    if !stack.is_empty() {
        return Err(AnimationEventError::Xml(
            "unclosed animation event elements".into(),
        ));
    }
    Ok(AnimationEventDatabase {
        timelines: state.timelines,
        by_caf_path: state.by_caf_path,
        diagnostics: state.diagnostics,
        diagnostics_truncated: state.diagnostics_truncated,
    })
}

/// Read and parse one `.animevents` file without writing it back.
pub fn read_animation_events(
    path: impl AsRef<Path>,
) -> Result<AnimationEventDatabase, AnimationEventError> {
    let path = path.as_ref();
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(AnimationEventError::Xml(
            "not a regular animation event file".into(),
        ));
    }
    let length = metadata.len();
    if length > MAX_ANIMATION_EVENT_BYTES as u64 {
        return Err(AnimationEventError::FileTooLarge {
            limit: MAX_ANIMATION_EVENT_BYTES,
        });
    }
    parse_animation_events(&fs::read(path)?)
}

#[derive(Default)]
struct ParseState {
    timelines: Vec<AnimationEventTimeline>,
    by_caf_path: BTreeMap<String, Vec<usize>>,
    diagnostics: Vec<AnimationEventDiagnostic>,
    diagnostics_truncated: bool,
    current: Option<usize>,
    total_events: usize,
    animation_ordinal: usize,
    event_ordinal: usize,
    root_seen: bool,
}

impl ParseState {
    fn element(
        &mut self,
        element: &BytesStart<'_>,
        depth: usize,
    ) -> Result<(), AnimationEventError> {
        let name = element.name();
        let tag = name.as_ref();
        if depth == 0 {
            if self.root_seen {
                return Err(AnimationEventError::Xml("multiple root elements".into()));
            }
            if !tag.eq_ignore_ascii_case(b"anim_event_list") {
                return Err(AnimationEventError::Xml(
                    "unexpected animation event root element".into(),
                ));
            }
            self.root_seen = true;
        } else if depth == 1 && tag.eq_ignore_ascii_case(b"animation") {
            self.animation(element)?;
        } else if depth == 2 && tag.eq_ignore_ascii_case(b"event") {
            self.event(element)?;
        }
        Ok(())
    }

    fn animation(&mut self, element: &BytesStart<'_>) -> Result<(), AnimationEventError> {
        let animation_index = self.animation_ordinal;
        self.animation_ordinal += 1;
        self.current = None;
        self.event_ordinal = 0;
        let attributes = read_attributes(element)?;
        let Some(source_path) = attribute(&attributes, "name") else {
            self.push_diagnostic(AnimationEventDiagnostic::MissingCafPath { animation_index });
            return Ok(());
        };
        match normalize_animation_path(source_path) {
            Ok(caf_path) => {
                if self.timelines.len() >= MAX_ANIMATION_TIMELINES {
                    return Err(AnimationEventError::TooManyTimelines {
                        limit: MAX_ANIMATION_TIMELINES,
                    });
                }
                let id = self.timelines.len();
                self.timelines.push(AnimationEventTimeline {
                    source_path: source_path.trim().to_string(),
                    caf_path: caf_path.clone(),
                    events: Vec::new(),
                });
                self.by_caf_path.entry(caf_path).or_default().push(id);
                self.current = Some(id);
            }
            Err(AnimationEventError::UnsafePath { path, reason }) => {
                self.push_diagnostic(AnimationEventDiagnostic::UnsafeCafPath { path, reason });
            }
            Err(error) => return Err(error),
        }
        Ok(())
    }

    fn event(&mut self, element: &BytesStart<'_>) -> Result<(), AnimationEventError> {
        let Some(id) = self.current else {
            return Ok(());
        };
        self.total_events += 1;
        if self.total_events > MAX_ANIMATION_EVENTS {
            return Err(AnimationEventError::TooManyEvents {
                limit: MAX_ANIMATION_EVENTS,
            });
        }
        let event_index = self.event_ordinal;
        self.event_ordinal += 1;
        let caf_path = self.timelines[id].caf_path.clone();
        let attributes = read_attributes(element)?;
        let mut diagnostics = Vec::new();

        for (key, _) in &attributes {
            if !EVENT_ATTRIBUTES
                .iter()
                .any(|known| key.eq_ignore_ascii_case(known))
            {
                diagnostics.push(AnimationEventDiagnostic::UnknownAttribute {
                    caf_path: caf_path.clone(),
                    event_index,
                    attribute: key.clone(),
                });
            }
        }

        let name = attribute(&attributes, "name").unwrap_or_default();
        if name.trim().is_empty() {
            diagnostics.push(AnimationEventDiagnostic::MissingEventName {
                caf_path: caf_path.clone(),
                event_index,
            });
            for diagnostic in diagnostics {
                self.push_diagnostic(diagnostic);
            }
            return Ok(());
        }

        let normalized_time = normalized_time(
            attribute(&attributes, "time"),
            &caf_path,
            event_index,
            &mut diagnostics,
        );
        let normalized_end_time = normalized_end_time(
            attribute(&attributes, "endTime"),
            normalized_time,
            &caf_path,
            event_index,
            &mut diagnostics,
        );
        let offset = parse_vector(
            attribute(&attributes, "offset"),
            "offset",
            &caf_path,
            event_index,
            &mut diagnostics,
        );
        let dir = parse_vector(
            attribute(&attributes, "dir"),
            "dir",
            &caf_path,
            event_index,
            &mut diagnostics,
        );

        if self.timelines[id].events.len() >= MAX_EVENTS_PER_ANIMATION {
            return Err(AnimationEventError::TooManyEventsForAnimation {
                caf_path,
                limit: MAX_EVENTS_PER_ANIMATION,
            });
        }
        self.timelines[id].events.push(AnimationEvent {
            name: name.to_string(),
            normalized_time,
            normalized_end_time,
            parameter: attribute(&attributes, "parameter")
                .unwrap_or_default()
                .to_string(),
            bone: attribute(&attributes, "bone")
                .unwrap_or_default()
                .to_string(),
            model: attribute(&attributes, "model")
                .unwrap_or_default()
                .to_string(),
            offset,
            dir,
        });
        for diagnostic in diagnostics {
            self.push_diagnostic(diagnostic);
        }
        Ok(())
    }

    fn push_diagnostic(&mut self, diagnostic: AnimationEventDiagnostic) {
        if self.diagnostics.len() < MAX_ANIMATION_EVENT_DIAGNOSTICS {
            self.diagnostics.push(diagnostic);
        } else {
            self.diagnostics_truncated = true;
        }
    }
}

fn read_attributes(element: &BytesStart<'_>) -> Result<Vec<(String, String)>, AnimationEventError> {
    let mut attributes = Vec::new();
    for attribute in element.attributes() {
        if attributes.len() >= 32 {
            return Err(AnimationEventError::Xml("too many event attributes".into()));
        }
        let attribute = attribute.map_err(|error| AnimationEventError::Xml(error.to_string()))?;
        let key = String::from_utf8_lossy(attribute.key.as_ref()).into_owned();
        let value = attribute
            .unescape_value()
            .map_err(|error| AnimationEventError::Xml(error.to_string()))?
            .into_owned();
        if value.len() > MAX_EVENT_STRING_BYTES {
            return Err(AnimationEventError::StringTooLong {
                field: key,
                bytes: value.len(),
                limit: MAX_EVENT_STRING_BYTES,
            });
        }
        attributes.push((key, value));
    }
    Ok(attributes)
}

fn attribute<'a>(attributes: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find_map(|(key, value)| key.eq_ignore_ascii_case(name).then_some(value.as_str()))
}

fn normalized_time(
    raw: Option<&str>,
    caf_path: &str,
    event_index: usize,
    diagnostics: &mut Vec<AnimationEventDiagnostic>,
) -> f32 {
    let Some(raw) = raw else {
        diagnostics.push(AnimationEventDiagnostic::MissingTime {
            caf_path: caf_path.to_string(),
            event_index,
        });
        return 0.0;
    };
    let Ok(value) = raw.trim().parse::<f32>() else {
        diagnostics.push(AnimationEventDiagnostic::InvalidNumber {
            caf_path: caf_path.to_string(),
            event_index,
            attribute: "time".into(),
            value: raw.into(),
        });
        return 0.0;
    };
    if value.is_nan() {
        diagnostics.push(AnimationEventDiagnostic::InvalidNumber {
            caf_path: caf_path.to_string(),
            event_index,
            attribute: "time".into(),
            value: raw.into(),
        });
        return 0.0;
    }
    if value < 0.0 {
        diagnostics.push(AnimationEventDiagnostic::ClampedTime {
            caf_path: caf_path.to_string(),
            event_index,
            original: value,
            clamped: 0.0,
        });
        return 0.0;
    }
    if value > 1.0 {
        diagnostics.push(AnimationEventDiagnostic::ClampedTime {
            caf_path: caf_path.to_string(),
            event_index,
            original: value,
            clamped: 1.0,
        });
        return 1.0;
    }
    value
}

fn normalized_end_time(
    raw: Option<&str>,
    start: f32,
    caf_path: &str,
    event_index: usize,
    diagnostics: &mut Vec<AnimationEventDiagnostic>,
) -> f32 {
    let Some(raw) = raw else {
        return start;
    };
    let Ok(value) = raw.trim().parse::<f32>() else {
        diagnostics.push(AnimationEventDiagnostic::InvalidNumber {
            caf_path: caf_path.to_string(),
            event_index,
            attribute: "endTime".into(),
            value: raw.into(),
        });
        return start;
    };
    if value.is_nan() {
        diagnostics.push(AnimationEventDiagnostic::InvalidNumber {
            caf_path: caf_path.to_string(),
            event_index,
            attribute: "endTime".into(),
            value: raw.into(),
        });
        return start;
    }
    if value < start {
        diagnostics.push(AnimationEventDiagnostic::EndBeforeStart {
            caf_path: caf_path.to_string(),
            event_index,
            start,
            end: value,
        });
        return start;
    }
    if value > 1.0 {
        diagnostics.push(AnimationEventDiagnostic::ClampedEndTime {
            caf_path: caf_path.to_string(),
            event_index,
            original: value,
            clamped: 1.0,
        });
        return 1.0;
    }
    value
}

fn parse_vector(
    raw: Option<&str>,
    attribute: &str,
    caf_path: &str,
    event_index: usize,
    diagnostics: &mut Vec<AnimationEventDiagnostic>,
) -> [f32; 3] {
    let Some(raw) = raw else {
        return [0.0; 3];
    };
    let parts = raw.split(',').map(str::trim).collect::<Vec<_>>();
    let invalid = |diagnostics: &mut Vec<AnimationEventDiagnostic>| {
        diagnostics.push(AnimationEventDiagnostic::InvalidVector {
            caf_path: caf_path.to_string(),
            event_index,
            attribute: attribute.to_string(),
            value: raw.to_string(),
        });
    };
    if parts.len() != 3 {
        invalid(diagnostics);
        return [0.0; 3];
    }
    let mut result = [0.0; 3];
    for (i, part) in parts.iter().enumerate() {
        match part.parse::<f32>() {
            Ok(value) if value.is_finite() && value.abs() <= MAX_VECTOR_COMPONENT => {
                result[i] = value;
            }
            _ => {
                invalid(diagnostics);
                return [0.0; 3];
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const RIFLE: &[u8] = br#"<anim_event_list>
 <animation name="animations/weapons/us/scar/stand_tac_reloadempty_scar_weapon_1p_01.caf">
  <event name="magazine" time="0.50839999" parameter="Crysis2_weapon_scarab.mag_eject" bone="magazine" offset="0,0,0" dir="0,0,0" model=""/>
 </animation>
</anim_event_list>"#;

    #[test]
    fn rifle_style_without_end_time_defaults_end_to_start() {
        let database = parse_animation_events(RIFLE).unwrap();
        assert_eq!(database.len(), 1);
        assert!(database.diagnostics().is_empty());
        let timeline = database
            .timeline("animations/weapons/us/scar/stand_tac_reloadempty_scar_weapon_1p_01.caf")
            .unwrap();
        assert_eq!(timeline.events.len(), 1);
        let event = &timeline.events[0];
        assert!((event.normalized_time - 0.508_399_99).abs() < 1e-6);
        assert_eq!(event.normalized_end_time, event.normalized_time);
        assert!(!event.is_range());
        assert_eq!(event.name, "magazine");
        assert_eq!(event.parameter, "Crysis2_weapon_scarab.mag_eject");
        assert_eq!(event.bone, "magazine");
        assert_eq!(event.model, "");
        assert_eq!(event.offset, [0.0, 0.0, 0.0]);
        assert_eq!(event.dir, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn explicit_end_time_is_preserved_and_markers_scale_with_duration() {
        let database = parse_animation_events(
            br#"<anim_event_list>
 <animation name="objects/chars/hunter/reload.caf">
  <event name="sound" time="0.5" endTime="0.75" parameter="reload" bone="weapon" offset="1,2,3" dir="0,1,0" model="objects/weapons/rifle.cgf"/>
 </animation>
</anim_event_list>"#,
        )
        .unwrap();
        let event = &database
            .timeline("objects/chars/hunter/reload.caf")
            .unwrap()
            .events[0];
        assert_eq!(event.normalized_end_time, 0.75);
        assert!(event.is_range());
        assert_eq!(event.offset, [1.0, 2.0, 3.0]);
        assert_eq!(event.dir, [0.0, 1.0, 0.0]);
        assert_eq!(event.model, "objects/weapons/rifle.cgf");
        assert!((event.start_seconds(2.0) - 1.0).abs() < 1e-4);
        assert!((event.end_seconds(2.0) - 1.5).abs() < 1e-4);
        assert!(database.diagnostics().is_empty());
    }

    #[test]
    fn times_outside_the_normalized_range_are_clamped_with_diagnostics() {
        let database = parse_animation_events(
            br#"<anim_event_list>
 <animation name="objects/chars/hunter/timing.caf">
  <event name="early" time="-0.5" endTime="0.5"/>
  <event name="late" time="1.5"/>
  <event name="inverted" time="0.8" endTime="0.2"/>
  <event name="high_end" time="0.2" endTime="1.5"/>
 </animation>
</anim_event_list>"#,
        )
        .unwrap();
        let events = &database
            .timeline("objects/chars/hunter/timing.caf")
            .unwrap()
            .events;
        assert_eq!(events[0].normalized_time, 0.0);
        assert_eq!(events[0].normalized_end_time, 0.5);
        assert_eq!(events[1].normalized_time, 1.0);
        assert_eq!(events[1].normalized_end_time, 1.0);
        assert_eq!(events[2].normalized_time, 0.8);
        assert_eq!(events[2].normalized_end_time, 0.8);
        assert_eq!(events[3].normalized_end_time, 1.0);

        let diagnostics = database.diagnostics();
        assert_eq!(diagnostics.len(), 4);
        assert!(matches!(
            diagnostics[0],
            AnimationEventDiagnostic::ClampedTime {
                event_index: 0,
                original,
                clamped: 0.0,
                ..
            } if original == -0.5
        ));
        assert!(matches!(
            diagnostics[1],
            AnimationEventDiagnostic::ClampedTime {
                event_index: 1,
                original,
                clamped: 1.0,
                ..
            } if original == 1.5
        ));
        assert!(matches!(
            diagnostics[2],
            AnimationEventDiagnostic::EndBeforeStart {
                event_index: 2,
                start: 0.8,
                end: 0.2,
                ..
            }
        ));
        assert!(matches!(
            diagnostics[3],
            AnimationEventDiagnostic::ClampedEndTime {
                event_index: 3,
                original,
                clamped: 1.0,
                ..
            } if original == 1.5
        ));
    }

    #[test]
    fn full_path_matching_is_case_insensitive_and_slash_normalized() {
        let database = parse_animation_events(
            br#"<anim_event_list>
 <animation name="Objects/Chars/Hunter/Hunter_Idle.caf"><event name="foley" time="0.1"/></animation>
</anim_event_list>"#,
        )
        .unwrap();
        assert_eq!(database.len(), 1);
        assert!(
            database
                .timeline("objects\\chars\\hunter\\hunter_idle.CAF")
                .is_some()
        );
        assert_eq!(
            database
                .timeline("Objects//Chars/./Hunter/Hunter_Idle.caf")
                .unwrap()
                .caf_path,
            "objects/chars/hunter/hunter_idle.caf"
        );
        assert_eq!(
            database
                .timeline("Objects/chars/hunter/Hunter_Idle.caf")
                .unwrap()
                .source_path,
            "Objects/Chars/Hunter/Hunter_Idle.caf"
        );
        assert!(database.timeline("hunter_idle.caf").is_none());
        assert!(
            database
                .timeline("objects/chars/hunter/other.caf")
                .is_none()
        );
    }

    #[test]
    fn duplicate_events_keep_source_order_and_all_markers() {
        let database = parse_animation_events(
            br#"<anim_event_list>
 <animation name="objects/chars/hunter/footsteps.caf">
  <event name="footstep" time="0.25" parameter="left" bone="foot_l"/>
  <event name="foley" time="0.5"/>
  <event name="footstep" time="0.25" parameter="right" bone="foot_r"/>
 </animation>
</anim_event_list>"#,
        )
        .unwrap();
        let events = &database
            .timeline("objects/chars/hunter/footsteps.caf")
            .unwrap()
            .events;
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].parameter, "left");
        assert_eq!(events[1].name, "foley");
        assert_eq!(events[2].parameter, "right");
        assert_eq!(events[0].normalized_time, events[2].normalized_time);
    }

    #[test]
    fn duplicate_animation_entries_are_indexed_and_kept_separate() {
        let database = parse_animation_events(
            br#"<anim_event_list>
 <animation name="objects/chars/hunter/a.caf"><event name="first" time="0.1"/></animation>
 <animation name="OBJECTS/CHARS/HUNTER/A.CAF"><event name="second" time="0.2"/></animation>
</anim_event_list>"#,
        )
        .unwrap();
        assert_eq!(database.len(), 2);
        assert_eq!(
            database.timelines_for("objects/chars/hunter/a.caf").count(),
            2
        );
        let first = database.timeline("objects/chars/hunter/a.caf").unwrap();
        assert_eq!(first.events[0].name, "first");
        assert_eq!(first.source_path, "objects/chars/hunter/a.caf");
        let all = database
            .timelines_for("objects/chars/hunter/a.caf")
            .collect::<Vec<_>>();
        assert_eq!(all[1].events[0].name, "second");
        assert_eq!(all[1].source_path, "OBJECTS/CHARS/HUNTER/A.CAF");
    }

    #[test]
    fn traversal_absolute_and_drive_paths_are_rejected() {
        for path in [
            "../escape.caf",
            "objects/../../escape.caf",
            "/objects/chars/hunter/idle.caf",
            "\\\\server\\share\\idle.caf",
            "C:\\game\\objects\\idle.caf",
            "objects/chars/hunter/idle.caf:stream",
        ] {
            assert!(
                normalize_animation_path(path).is_err(),
                "{path} must be rejected"
            );
        }
        let database = parse_animation_events(
            br#"<anim_event_list>
 <animation name="../escape.caf"><event name="sound" time="0.1"/></animation>
 <animation name="/objects/chars/hunter/idle.caf"><event name="sound" time="0.1"/></animation>
 <animation name="objects//chars/./hunter/idle.caf"><event name="sound" time="0.1"/></animation>
</anim_event_list>"#,
        )
        .unwrap();
        assert_eq!(database.len(), 1);
        assert_eq!(
            database.timelines()[0].caf_path,
            "objects/chars/hunter/idle.caf"
        );
        let rejected = database
            .diagnostics()
            .iter()
            .filter(|diagnostic| {
                matches!(diagnostic, AnimationEventDiagnostic::UnsafeCafPath { .. })
            })
            .count();
        assert_eq!(rejected, 2);
        assert!(database.timeline("escape.caf").is_none());
    }

    #[test]
    fn missing_caf_path_is_reported_with_its_source_position() {
        let database = parse_animation_events(
            br#"<anim_event_list>
 <animation><event name="sound" time="0.1"/></animation>
 <animation name="objects/chars/hunter/valid.caf"/>
</anim_event_list>"#,
        )
        .unwrap();
        assert_eq!(database.len(), 1);
        assert!(matches!(
            database.diagnostics()[0],
            AnimationEventDiagnostic::MissingCafPath { animation_index: 0 }
        ));
        assert!(
            database
                .timeline("objects/chars/hunter/valid.caf")
                .is_some()
        );
    }

    #[test]
    fn missing_and_invalid_event_values_are_diagnosed_without_panicking() {
        let database = parse_animation_events(
            br#"<anim_event_list>
 <animation name="objects/chars/hunter/invalid.caf">
  <event name="bad_time" time="abc" endTime="def" offset="1,2" dir="1,2,3,4"/>
  <event time="0.4"/>
  <event name="missing_time"/>
 </animation>
</anim_event_list>"#,
        )
        .unwrap();
        let timeline = database
            .timeline("objects/chars/hunter/invalid.caf")
            .unwrap();
        assert_eq!(timeline.events.len(), 2);
        assert_eq!(timeline.events[0].name, "bad_time");
        assert_eq!(timeline.events[0].normalized_time, 0.0);
        assert_eq!(timeline.events[0].normalized_end_time, 0.0);
        assert_eq!(timeline.events[0].offset, [0.0, 0.0, 0.0]);
        assert_eq!(timeline.events[0].dir, [0.0, 0.0, 0.0]);
        assert_eq!(timeline.events[1].name, "missing_time");
        assert_eq!(timeline.events[1].normalized_time, 0.0);

        let diagnostics = database.diagnostics();
        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            AnimationEventDiagnostic::InvalidNumber { event_index: 0, attribute, .. } if attribute == "time"
        )));
        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            AnimationEventDiagnostic::InvalidNumber { event_index: 0, attribute, .. } if attribute == "endTime"
        )));
        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            AnimationEventDiagnostic::InvalidVector { event_index: 0, attribute, .. } if attribute == "offset"
        )));
        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            AnimationEventDiagnostic::InvalidVector { event_index: 0, attribute, .. } if attribute == "dir"
        )));
        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            AnimationEventDiagnostic::MissingEventName { event_index: 1, .. }
        )));
        assert!(diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            AnimationEventDiagnostic::MissingTime { event_index: 2, .. }
        )));
    }

    #[test]
    fn unknown_attributes_are_reported_without_dropping_the_event() {
        let database = parse_animation_events(
            br#"<anim_event_list>
 <animation name="objects/chars/hunter/schema.caf">
  <event name="sound" time="0.3" future_attribute="value"/>
 </animation>
</anim_event_list>"#,
        )
        .unwrap();
        assert_eq!(
            database
                .timeline("objects/chars/hunter/schema.caf")
                .unwrap()
                .events
                .len(),
            1
        );
        assert!(database.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic,
            AnimationEventDiagnostic::UnknownAttribute { attribute, .. } if attribute == "future_attribute"
        )));
    }

    #[test]
    fn oversized_file_fails_closed() {
        let oversized = vec![b' '; MAX_ANIMATION_EVENT_BYTES + 1];
        assert!(matches!(
            parse_animation_events(&oversized),
            Err(AnimationEventError::FileTooLarge { .. })
        ));
    }

    #[test]
    fn oversized_event_string_fails_closed() {
        let long_name = "x".repeat(MAX_EVENT_STRING_BYTES + 1);
        let xml = format!(
            r#"<anim_event_list><animation name="objects/chars/hunter/long.caf"><event name="{long_name}" time="0.1"/></animation></anim_event_list>"#
        );
        assert!(matches!(
            parse_animation_events(xml.as_bytes()),
            Err(AnimationEventError::StringTooLong { .. })
        ));
    }

    #[test]
    fn malformed_documents_fail_closed() {
        assert!(matches!(
            parse_animation_events(b"<anim_event_list><animation name=\"a.caf\">"),
            Err(AnimationEventError::Xml(_))
        ));
        assert!(matches!(
            parse_animation_events(b"<wrong_root/>"),
            Err(AnimationEventError::Xml(_))
        ));
        assert!(matches!(
            parse_animation_events(
                b"<anim_event_list><animation name=\"a.caf\"></anim_event_list>"
            ),
            Err(AnimationEventError::Xml(_))
        ));
        assert!(matches!(
            parse_animation_events(b""),
            Err(AnimationEventError::Xml(_))
        ));
        assert!(matches!(
            parse_animation_events(b"<!DOCTYPE x [<!ENTITY y \"z\">]><anim_event_list/>"),
            Err(AnimationEventError::UnsafeXml)
        ));
    }

    #[test]
    fn empty_documents_and_empty_timelines_are_not_errors() {
        let empty = parse_animation_events(b"<anim_event_list/>").unwrap();
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);
        assert_eq!(empty.event_count(), 0);

        let database = parse_animation_events(
            br#"<anim_event_list>
 <animation name="objects/chars/hunter/no_events.caf"/>
 <animation name="objects/chars/hunter/events.caf"><event name="sound" time="0.1"/></animation>
</anim_event_list>"#,
        )
        .unwrap();
        assert_eq!(database.len(), 2);
        assert_eq!(database.event_count(), 1);
        assert!(
            database
                .timeline("objects/chars/hunter/no_events.caf")
                .unwrap()
                .events
                .is_empty()
        );
    }
    #[test]
    fn compiled_cryxmlb_event_list_matches_text() {
        let text = br#"<anim_event_list><animation name="characters/hunter/idle.caf"><event name="footstep" time="0.3"/></animation></anim_event_list>"#;
        let compiled = crate::cryxml::encode_xml(text, MAX_ANIMATION_EVENT_BYTES).unwrap();
        let from_text = parse_animation_events(text).unwrap();
        let from_binary = parse_animation_events(&compiled).unwrap();
        assert_eq!(from_binary.timelines(), from_text.timelines());
    }
    #[test]
    #[ignore = "requires EVOLVE_ANIMEVENTS_SOURCE pointing to a locally owned .animevents file"]
    fn user_owned_animevents_parse_without_writing_source() {
        let source = std::env::var("EVOLVE_ANIMEVENTS_SOURCE").unwrap();
        let before = std::fs::read(&source).unwrap();
        let parsed = read_animation_events(&source).unwrap();
        eprintln!(
            "{} timelines, {} events, {} diagnostics",
            parsed.len(),
            parsed.event_count(),
            parsed.diagnostics().len()
        );
        assert!(!parsed.is_empty());
        assert!(parsed.event_count() > 0);
        assert_eq!(before, std::fs::read(source).unwrap());
    }
}
