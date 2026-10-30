//! Source-backed, read-only TimeOfDay curve preview and original FX inventory.
//! The preview approximates the SDK's Bezier spline; the concrete engine TOD
//! interpolator is not public here. No native shader/particle code is executed.
use quick_xml::{
    Reader,
    events::{BytesStart, Event},
};
use std::collections::BTreeMap;
const MAX_XML: usize = 4 * 1024 * 1024;
#[derive(Clone, Debug)]
struct Key {
    time: f32,
    value: [f32; 3],
    flags: u32,
}
// ISplines.h: the low three bits select the incoming tangent; bits 3-5
// select the outgoing tangent. Higher bits include editor state, not slopes.
const TANGENT_STEP: u32 = 3;
const TANGENT_LINEAR: u32 = 4;
const TANGENT_ZERO: u32 = 2;
fn incoming(key: &Key) -> u32 {
    key.flags & 7
}
fn outgoing(key: &Key) -> u32 {
    (key.flags >> 3) & 7
}
#[derive(Clone, Debug)]
struct Curve {
    keys: Vec<Key>,
    color: bool,
}
impl Curve {
    fn sample(&self, time: f32) -> [f32; 3] {
        let t = (time / 24.0).clamp(0.0, 1.0);
        if t <= self.keys[0].time {
            return self.keys[0].value;
        }
        for (index, pair) in self.keys.windows(2).enumerate() {
            if t < pair[1].time {
                let [from, to] = pair else { unreachable!() };
                // BezierSpline in ISplines.h steps *to* on an outgoing step,
                // or holds *from* on an incoming step. The concrete TOD
                // interpolator is not exposed by the FreeSDK headers.
                if outgoing(from) == TANGENT_STEP {
                    return to.value;
                }
                if incoming(to) == TANGENT_STEP {
                    return from.value;
                }
                let u = (t - from.time) / (to.time - from.time);
                // The serialized TOD keys contain no explicit custom tangent
                // vectors. Do not invent slopes for unsupported tangent modes.
                if !matches!(outgoing(from), 0 | TANGENT_ZERO | TANGENT_LINEAR)
                    || !matches!(incoming(to), 0 | TANGENT_ZERO | TANGENT_LINEAR)
                {
                    return std::array::from_fn(|channel| {
                        from.value[channel] + (to.value[channel] - from.value[channel]) * u
                    });
                }
                let prev = index.checked_sub(1).map(|i| &self.keys[i]);
                let next = self.keys.get(index + 2);
                return std::array::from_fn(|channel| {
                    let a = from.value[channel];
                    let b = to.value[channel];
                    let out = match outgoing(from) {
                        TANGENT_ZERO => 0.0,
                        TANGENT_LINEAR => (b - a) / 3.0,
                        _ => match prev {
                            Some(prev) => {
                                let span = to.time - prev.time;
                                (to.value[channel] - prev.value[channel]) * (to.time - from.time)
                                    / (3.0 * span)
                            }
                            None => (b - a) / 3.0,
                        },
                    };
                    let in_slope = match incoming(to) {
                        TANGENT_ZERO => 0.0,
                        TANGENT_LINEAR => (b - a) / 3.0,
                        _ => match next {
                            Some(next) => {
                                let span = next.time - from.time;
                                (next.value[channel] - a) * (to.time - from.time) / (3.0 * span)
                            }
                            None => (b - a) / 3.0,
                        },
                    };
                    let one_minus_u = 1.0 - u;
                    one_minus_u.powi(3) * a
                        + 3.0 * one_minus_u.powi(2) * u * (a + out)
                        + 3.0 * one_minus_u * u * u * (b - in_slope)
                        + u.powi(3) * b
                });
            }
        }
        self.keys.last().unwrap().value
    }
}
#[derive(Clone, Debug)]
pub(super) struct TimeOfDay {
    pub initial_hour: f32,
    /// Source's playback bounds and speed, for inspection only. The map
    /// preview deliberately keeps its manual 0–24 hour slider independent.
    pub playback_window: Option<(f32, f32, f32)>,
    pub invalid_playback_window: bool,
    curves: BTreeMap<String, Curve>,
    pub keyed_variables: usize,
    /// Out-of-day keys in the source XML skipped without modifying original data.
    pub ignored_out_of_range_keys: usize,
}
fn attribute(e: &BytesStart<'_>, key: &[u8]) -> Result<Option<String>, String> {
    for a in e.attributes() {
        let a = a.map_err(|e| e.to_string())?;
        if a.key.as_ref().eq_ignore_ascii_case(key) {
            return a
                .unescape_value()
                .map(|v| Some(v.into_owned()))
                .map_err(|e| e.to_string());
        }
    }
    Ok(None)
}
fn value(source: &str) -> Option<([f32; 3], bool)> {
    let trimmed = source.trim().trim_start_matches('(').trim_end_matches(')');
    let parts: Vec<_> = trimmed.split([':', ',']).collect();
    if parts.len() != 1 && parts.len() != 3 {
        return None;
    }
    let mut rgb = [0.0; 3];
    for (i, p) in parts.iter().enumerate() {
        rgb[i] = p.trim().parse::<f32>().ok()?;
        if !rgb[i].is_finite() || rgb[i].abs() > 1e7 {
            return None;
        }
    }
    if parts.len() == 1 {
        rgb = [rgb[0]; 3];
    }
    Some((rgb, parts.len() == 3))
}
fn keys(source: &str) -> Result<(Vec<Key>, usize), String> {
    let mut parsed = Vec::new();
    let mut ignored_out_of_range = 0;
    for (index, raw) in source
        .split(',')
        .filter(|entry| !entry.trim().is_empty())
        .enumerate()
    {
        // Count source entries, not only accepted entries: invalid keys cannot
        // evade the per-spline memory and parsing budget.
        if index >= 512 {
            return Err("Too many original spline keys".into());
        }
        let (time, remaining) = raw
            .split_once(':')
            .ok_or("Malformed CryEngine spline key")?;
        let time = time
            .trim()
            .parse::<f32>()
            .map_err(|_| "Invalid spline time")?;
        if !time.is_finite() {
            return Err("Non-finite original spline time".into());
        }
        // CryEngine 3.5.8 Forest TimeOfDay.xml contains a small number of
        // serialized editor outliers (e.g. 1.78957e+008 and 1.7895). They
        // are not valid normalized clock positions. Dropping only those
        // entries preserves the source daytime curve; clamping them to 1.0
        // would incorrectly overwrite legitimate midnight keyframes.
        if !(0.0..=1.0).contains(&time) {
            ignored_out_of_range += 1;
            continue;
        }
        let (sample, flag_text) = if let Some(tuple) = remaining.strip_prefix('(') {
            let (tuple, tail) = tuple.split_once(')').ok_or("Unterminated color spline")?;
            (
                value(tuple).ok_or("Invalid original color spline")?.0,
                tail.strip_prefix(':').ok_or("Missing spline key flags")?,
            )
        } else {
            let (sample, flag_text) = remaining
                .split_once(':')
                .ok_or("Missing spline key flags")?;
            (
                value(sample).ok_or("Invalid original scalar spline")?.0,
                flag_text,
            )
        };
        let flags = flag_text
            .parse::<u32>()
            .map_err(|_| "Invalid original spline key flags")?;
        parsed.push(Key {
            time,
            value: sample,
            flags,
        });
    }
    parsed.sort_by(|a, b| a.time.total_cmp(&b.time));
    parsed.dedup_by(|a, b| a.time == b.time);
    if parsed.is_empty() {
        return Err("Empty original spline".into());
    }
    Ok((parsed, ignored_out_of_range))
}
impl TimeOfDay {
    pub fn parse(source: &[u8]) -> Result<Self, String> {
        if source.len() > MAX_XML {
            return Err("TimeOfDay.xml exceeds 4 MiB".into());
        }
        // Original level environments can be authored XML or compiled
        // CryXmlB. Reuse the bounded core decoder before touching attributes;
        // never guess the header or mutate an imported source file.
        let decoded =
            evolve_core::cryxml::xml_bytes(source, MAX_XML).map_err(|error| error.to_string())?;
        let mut reader = Reader::from_reader(decoded.as_ref());
        let mut curves = BTreeMap::new();
        let mut pending: Option<(String, Curve)> = None;
        let mut initial_hour = 12.0;
        let mut playback_window = None;
        let mut invalid_playback_window = false;
        let mut ignored_out_of_range_keys = 0;
        let mut valid_root = false;
        loop {
            match reader.read_event().map_err(|e| e.to_string())? {
                Event::Start(e) | Event::Empty(e)
                    if e.name().as_ref().eq_ignore_ascii_case(b"TimeOfDay") =>
                {
                    valid_root = true;
                    if let Some(raw) = attribute(&e, b"Time")? {
                        initial_hour = raw
                            .parse::<f32>()
                            .map_err(|_| "Bad original current time")?;
                        if !initial_hour.is_finite() || !(0.0..=24.0).contains(&initial_hour) {
                            return Err("Original current hour invalid".into());
                        }
                    }
                    let start = attribute(&e, b"TimeStart")?;
                    let end = attribute(&e, b"TimeEnd")?;
                    let speed = attribute(&e, b"TimeAnimSpeed")?;
                    if start.is_some() || end.is_some() || speed.is_some() {
                        let parsed = start
                            .and_then(|v| v.parse::<f32>().ok())
                            .zip(end.and_then(|v| v.parse::<f32>().ok()))
                            .zip(speed.and_then(|v| v.parse::<f32>().ok()));
                        if let Some(((start, end), speed)) =
                            parsed.filter(|((start, end), speed)| {
                                start.is_finite()
                                    && end.is_finite()
                                    && speed.is_finite()
                                    && (0.0..=24.0).contains(start)
                                    && (0.0..=24.0).contains(end)
                                    && start <= end
                            })
                        {
                            playback_window = Some((start, end, speed));
                        } else {
                            invalid_playback_window = true;
                        }
                    }
                }
                Event::Start(e) if e.name().as_ref().eq_ignore_ascii_case(b"Variable") => {
                    let name = attribute(&e, b"Name")?
                        .unwrap_or_default()
                        .to_ascii_lowercase();
                    if name.len() > 128 {
                        return Err("Original variable name too long".into());
                    }
                    let original = if let Some(c) = attribute(&e, b"Color")? {
                        value(&c).map(|(rgb, _)| (rgb, true))
                    } else {
                        attribute(&e, b"Value")?.and_then(|v| value(&v))
                    };
                    pending = original.and_then(|(v, color)| {
                        (!name.is_empty()).then_some((
                            name,
                            Curve {
                                keys: vec![Key {
                                    time: 0.0,
                                    value: v,
                                    flags: 0,
                                }],
                                color,
                            },
                        ))
                    });
                }
                Event::Start(e) | Event::Empty(e)
                    if e.name().as_ref().eq_ignore_ascii_case(b"Spline") =>
                {
                    if let Some((_, c)) = &mut pending
                        && let Some(raw) = attribute(&e, b"Keys")?
                    {
                        let (parsed, ignored) = keys(&raw)?;
                        ignored_out_of_range_keys += ignored;
                        c.keys = parsed;
                    }
                }
                Event::End(e) if e.name().as_ref().eq_ignore_ascii_case(b"Variable") => {
                    if let Some((name, c)) = pending.take() {
                        if curves.len() >= 256 {
                            return Err("Too many original TOD variables".into());
                        }
                        if curves.insert(name, c).is_some() {
                            return Err("Duplicate original TOD variable".into());
                        }
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }
        if !valid_root || curves.is_empty() {
            return Err("No supported original TimeOfDay variables".into());
        }
        let keyed_variables = curves.values().filter(|v| v.keys.len() > 1).count();
        Ok(Self {
            initial_hour,
            playback_window,
            invalid_playback_window,
            curves,
            keyed_variables,
            ignored_out_of_range_keys,
        })
    }
    pub fn color(&self, name: &str, hour: f32) -> Option<[f32; 3]> {
        self.curves
            .get(&name.to_ascii_lowercase())
            .filter(|c| c.color)
            .map(|c| c.sample(hour))
    }
    pub fn scalar(&self, name: &str, hour: f32) -> Option<f32> {
        self.curves
            .get(&name.to_ascii_lowercase())
            .filter(|c| !c.color)
            .map(|c| c.sample(hour)[0])
    }
    pub fn count(&self) -> usize {
        self.curves.len()
    }
    pub fn playback_window_description(&self) -> String {
        if self.invalid_playback_window {
            return "authored playback window malformed (manual 0–24h preview retained)".into();
        }
        match self.playback_window {
            Some((start, end, speed)) => format!(
                "authored playback {:.2}–{:.2}h at speed {:.3} (manual preview)",
                start, end, speed
            ),
            None => "no authored playback window (manual 0–24h preview)".into(),
        }
    }
}
/// Bounded, read-only fog parameters from an exact imported map owner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct FogProfile {
    pub view_distance: f32,
    pub ldr_density_multiplier: f32,
}

impl FogProfile {
    pub fn parse(source: &[u8]) -> Result<Self, String> {
        if source.len() > MAX_XML {
            return Err("Environment.xml exceeds 4 MiB".into());
        }
        let decoded =
            evolve_core::cryxml::xml_bytes(source, MAX_XML).map_err(|error| error.to_string())?;
        let mut reader = Reader::from_reader(decoded.as_ref());
        let mut parsed = None;
        loop {
            match reader.read_event().map_err(|error| error.to_string())? {
                Event::Start(element) | Event::Empty(element)
                    if element.name().as_ref().eq_ignore_ascii_case(b"Fog") =>
                {
                    if parsed.is_some() {
                        return Err("Multiple original Fog definitions".into());
                    }
                    let distance = attribute(&element, b"ViewDistance")?
                        .ok_or("Original Fog has no ViewDistance")?
                        .parse::<f32>()
                        .map_err(|_| "Invalid original Fog ViewDistance")?;
                    let multiplier = attribute(&element, b"LDRGlobalDensMult")?
                        .ok_or("Original Fog has no LDRGlobalDensMult")?
                        .parse::<f32>()
                        .map_err(|_| "Invalid original Fog LDRGlobalDensMult")?;
                    if !distance.is_finite()
                        || !(1.0..=100_000.0).contains(&distance)
                        || !multiplier.is_finite()
                        || !(0.0..=32.0).contains(&multiplier)
                    {
                        return Err("Original Fog parameters exceed preview bounds".into());
                    }
                    parsed = Some(Self {
                        view_distance: distance,
                        ldr_density_multiplier: multiplier,
                    });
                }
                Event::Eof => break,
                _ => {}
            }
        }
        parsed.ok_or_else(|| "Environment.xml has no supported Fog definition".into())
    }

    /// Deliberately non-native distance-only fog preview, never the original
    /// CryEngine height/volumetric fog or scattering equation. Zeroed when the
    /// map has no verified TimeOfDay Fog color curve.
    pub fn preview_uniform(
        self,
        tod: Option<&TimeOfDay>,
        hour: f32,
    ) -> [[f32; 4]; 2] {
        let disabled = [[0.0; 4]; 2];
        let Some(rgb) = tod.and_then(|profile| profile.color("Fog color", hour)) else {
            return disabled;
        };
        if self.ldr_density_multiplier <= 0.0 || !rgb.iter().all(|v| v.is_finite()) {
            return disabled;
        }
        [
            [
                rgb[0].clamp(0.0, 1.0),
                rgb[1].clamp(0.0, 1.0),
                rgb[2].clamp(0.0, 1.0),
                0.0,
            ],
            [
                1.0,
                self.view_distance.recip(),
                // Preview-only heuristic, NOT a recovered engine equation.
                (0.35 * self.ldr_density_multiplier).clamp(0.0, 8.0),
                0.0,
            ],
        ]
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct ResourceSummary {
    pub particle_libraries: usize,
    pub particle_effects: Vec<String>,
    pub material_files: usize,
    pub shader_sources: usize,
    pub authored_shaders: Vec<String>,
    pub emitters: usize,
}
impl ResourceSummary {
    pub fn description(&self) -> String {
        format!(
            "Original source inventory: {} particle XML libraries ({} named effects), {} MTL materials ({} shader assignments), {} CFX shader sources, {} particle placements. Native CryEngine effects/shaders are not executed.",
            self.particle_libraries,
            self.particle_effects.len(),
            self.material_files,
            self.authored_shaders.len(),
            self.shader_sources,
            self.emitters
        )
    }
}
pub(super) fn named_items(source: &[u8], tag: &[u8], key: &[u8]) -> Vec<String> {
    if source.len() > MAX_XML {
        return Vec::new();
    }
    let Ok(decoded) = evolve_core::cryxml::xml_bytes(source, MAX_XML) else {
        return Vec::new();
    };
    let mut reader = Reader::from_reader(decoded.as_ref());
    let mut result = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e) | Event::Empty(e))
                if e.name().as_ref().eq_ignore_ascii_case(tag) =>
            {
                if let Ok(Some(item)) = attribute(&e, key)
                    && item.len() <= 160
                    && !item.is_empty()
                    && !result.contains(&item)
                {
                    result.push(item);
                }
                if result.len() >= 512 {
                    break;
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fog_schema_is_bounded_and_missing_data_is_not_invented() {
        let source = br#"<Environment><Fog ViewDistance="4500" LDRGlobalDensMult="3"/></Environment>"#;
        let fog = FogProfile::parse(source).unwrap();
        assert_eq!(fog.view_distance, 4500.0);
        assert_eq!(fog.ldr_density_multiplier, 3.0);
        assert_eq!(fog.preview_uniform(None, 12.0), [[0.0; 4]; 2]);
        let tod = TimeOfDay::parse(br#"<TimeOfDay><Variable Name="Fog color" Color="0.3,0.5,0.7"/></TimeOfDay>"#).unwrap();
        let uniform = fog.preview_uniform(Some(&tod), 12.0);
        assert_eq!(uniform[0], [0.3, 0.5, 0.7, 0.0]);
        assert_eq!(uniform[1][0], 1.0);
        assert!((uniform[1][1] - 1.0/4500.0).abs() < 1e-8);
        for bad in [
            b"<Environment/>".as_slice(),
            b"<Environment><Fog ViewDistance=\"NaN\" LDRGlobalDensMult=\"3\"/></Environment>",
            b"<Environment><Fog ViewDistance=\"4500\" LDRGlobalDensMult=\"-2\"/></Environment>",
            b"<Environment><Fog ViewDistance=\"4500\"/></Environment>",
            b"<Environment><Fog ViewDistance=\"4500\" LDRGlobalDensMult=\"3\"/><Fog ViewDistance=\"4000\" LDRGlobalDensMult=\"2\"/></Environment>",
        ] {
            assert!(FogProfile::parse(bad).is_err());
        }
    }

    #[test]
    fn binary_cryxmlb_environment_fog_matches_plain_xml() {
        let original = br#"<Environment><Fog ViewDistance="2400" LDRGlobalDensMult="1"/></Environment>"#;
        let compiled = evolve_core::cryxml::encode_xml(original, MAX_XML).unwrap();
        assert_eq!(FogProfile::parse(&compiled).unwrap(), FogProfile::parse(original).unwrap());
    }

    #[test]
    fn source_spline_keys_match_cryengine_xml_layout() {
        let xml = br#"<TimeOfDay Time="13.5"><Variable Name="Sun color" Color="1,1,1"><Spline Keys="0:(0.2:0.3:0.4):36,0.5:(1:0.8:0.6):36,1:(0.2:0.3:0.4):36,"/></Variable><Variable Name="Sun intensity" Value="1000"><Spline Keys="0:1000:36,0.5:120000:36,1:1000:36,"/></Variable></TimeOfDay>"#;
        let parsed = TimeOfDay::parse(xml).unwrap();
        assert_eq!(parsed.initial_hour, 13.5);
        assert_eq!(parsed.keyed_variables, 2);
        assert_eq!(parsed.scalar("Sun intensity", 12.0), Some(120000.0));
        assert!((parsed.color("Sun color", 6.0).unwrap()[0] - 0.6).abs() < 0.001);
    }
    #[test]
    fn sdk_forest_style_outliers_do_not_discard_valid_curves() {
        // Derived from the key serialization pattern, not redistributed SDK assets.
        let xml = br#"<TimeOfDay Time="9.75"><Variable Name="Sun color multiplier" Value="5"><Spline Keys="0:0.1:36,0.5:9.3:0,1:0.04:65536,1.78957e+008:0.1:0,"/></Variable><Variable Name="Sky color" Color="0.2,0.3,0.4"><Spline Keys="-1:(1:1:1):0,0:(0.1:0.2:0.3):36,0.5:(0.8:0.6:0.4):0,1:(0.2:0.3:0.4):0,1.7895:(0.9:0.9:0.9):0,"/></Variable></TimeOfDay>"#;
        let parsed = TimeOfDay::parse(xml).unwrap();
        assert_eq!(parsed.ignored_out_of_range_keys, 3);
        assert_eq!(parsed.keyed_variables, 2);
        assert_eq!(parsed.scalar("Sun color multiplier", 24.0), Some(0.04));
        assert_eq!(parsed.color("Sky color", 24.0), Some([0.2, 0.3, 0.4]));
        assert_eq!(parsed.scalar("Sun color multiplier", 12.0), Some(9.3));
    }
    #[test]
    fn source_playback_window_is_metadata_not_a_slider_clamp() {
        let variable = r#"<Variable Name="Sun intensity" Value="1"><Spline Keys="0:1:36,1:3:36,"/></Variable>"#;
        let make = |attrs: &str| {
            TimeOfDay::parse(format!("<TimeOfDay {attrs}>{variable}</TimeOfDay>").as_bytes())
                .unwrap()
        };
        let source = make(r#"Time="12" TimeStart="9.75" TimeEnd="23.833334" TimeAnimSpeed="0""#);
        assert_eq!(source.playback_window, Some((9.75, 23.833334, 0.0)));
        assert_eq!(source.scalar("Sun intensity", 0.0), Some(1.0));
        assert_eq!(source.scalar("Sun intensity", 24.0), Some(3.0));
        let missing = make(r#"Time="12""#);
        assert_eq!(missing.playback_window, None);
        assert!(!missing.invalid_playback_window);
        let malformed = make(r#"Time="12" TimeStart="18" TimeEnd="6" TimeAnimSpeed="NaN""#);
        assert!(malformed.invalid_playback_window);
        assert_eq!(malformed.playback_window, None);
        assert_eq!(malformed.scalar("Sun intensity", 12.0), Some(2.0));
        assert!(make(r#"Time="12" TimeStart="4""#).invalid_playback_window);
    }
    #[test]
    fn binary_cryxmlb_time_of_day_matches_text_source() {
        let original = br#"<TimeOfDay Time="14"><Variable Name="Sun intensity" Value="123"><Spline Keys="0:0:36,0.5:456:36,1:0:36,"/></Variable></TimeOfDay>"#;
        let compiled = evolve_core::cryxml::encode_xml(original, MAX_XML).unwrap();
        let from_binary = TimeOfDay::parse(&compiled).unwrap();
        let from_text = TimeOfDay::parse(original).unwrap();
        assert_eq!(from_binary.count(), from_text.count());
        assert_eq!(from_binary.keyed_variables, from_text.keyed_variables);
        assert_eq!(from_binary.scalar("Sun intensity", 12.0), Some(456.0));
        let particle_xml = br#"<Particles><Item Name="dust"/><Item Name="sparks"/></Particles>"#;
        let compiled_particles = evolve_core::cryxml::encode_xml(particle_xml, MAX_XML).unwrap();
        assert_eq!(
            named_items(&compiled_particles, b"Item", b"Name"),
            named_items(particle_xml, b"Item", b"Name")
        );
    }
    #[test]
    fn outlier_only_or_unbounded_splines_fail_closed() {
        assert!(keys("1.7895:5:0,").is_err());
        assert!(keys("NaN:5:0,").is_err());
        assert!(keys("0:1:garbage,").is_err());
        assert!(keys("0:(1:2:3):,").is_err());
        assert!(keys(&"0:1:0,".repeat(513)).is_err());
    }
    #[test]
    fn tangent_flags_change_preview_without_interpreting_editor_bits() {
        fn curve(source: &str) -> Curve {
            Curve {
                keys: keys(source).unwrap().0,
                color: false,
            }
        }
        let linear = curve("0:0:36,0.5:10:36,1:0:36,");
        assert!((linear.sample(6.0)[0] - 5.0).abs() < 0.0001);
        let zero = curve("0:0:18,0.5:10:18,1:0:18,");
        assert!((zero.sample(6.0)[0] - 5.0).abs() < 0.0001);
        assert!(zero.sample(3.0)[0] < linear.sample(3.0)[0]);
        assert_eq!(curve("0:0:24,0.5:10:3,").sample(6.0)[0], 10.0);
        assert_eq!(curve("0:0:0,0.5:10:3,").sample(6.0)[0], 0.0);
        assert_eq!(curve("0:0:0,0.5:10:3,").sample(12.0)[0], 10.0);
        assert_eq!(curve("0:0:0,0.5:10:35,1:20:36,").sample(6.0)[0], 0.0);
        assert_eq!(curve("0:0:28,0.5:10:36,").sample(6.0)[0], 10.0);
        assert_eq!(curve("0:0:8,0.5:10:1,").sample(6.0)[0], 5.0);
        let editor = curve("0:0:65572,0.5:10:458752,1:0:36,");
        assert_eq!(outgoing(&editor.keys[0]), TANGENT_LINEAR);
        assert_eq!(incoming(&editor.keys[1]), 0);
        assert!(editor.sample(6.0)[0].is_finite());
        let smooth = curve("0:0:0,0.25:2:0,0.5:10:0,1:0:0,");
        assert!((smooth.sample(9.0)[0] - 6.0).abs() > 0.01);
    }
    #[test]
    fn invalid_sources_never_become_fake_lighting() {
        assert!(TimeOfDay::parse(b"<Other/>").is_err());
        assert!(TimeOfDay::parse(br#"<TimeOfDay><Variable Name="Sun intensity" Value="1"><Spline Keys="3:20:36,"/></Variable></TimeOfDay>"#).is_err());
        assert_eq!(
            named_items(
                br#"<Particles><Item Name="smoke"/><Item Name="sparks"/></Particles>"#,
                b"Item",
                b"Name"
            )
            .len(),
            2
        );
    }
}
