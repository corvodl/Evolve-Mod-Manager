//! Verified, read-only CryEngine time-of-day curves and original FX inventory.
//! The preview does not execute CryEngine native shaders or particle simulation.
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
        for pair in self.keys.windows(2) {
            if t <= pair[1].time {
                let f = (t - pair[0].time) / (pair[1].time - pair[0].time).max(0.00001);
                return std::array::from_fn(|i| {
                    pair[0].value[i] + (pair[1].value[i] - pair[0].value[i]) * f
                });
            }
        }
        self.keys.last().unwrap().value
    }
}
#[derive(Clone, Debug)]
pub(super) struct TimeOfDay {
    pub initial_hour: f32,
    curves: BTreeMap<String, Curve>,
    pub keyed_variables: usize,
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
fn keys(source: &str) -> Result<Vec<Key>, String> {
    let mut parsed = Vec::new();
    for raw in source.split(',').filter(|entry| !entry.trim().is_empty()) {
        if parsed.len() >= 512 {
            return Err("Too many original spline keys".into());
        }
        let (time, remaining) = raw
            .split_once(':')
            .ok_or("Malformed CryEngine spline key")?;
        let time = time
            .trim()
            .parse::<f32>()
            .map_err(|_| "Invalid spline time")?;
        if !time.is_finite() || !(-0.01..=1.01).contains(&time) {
            return Err("Spline time outside original day interval".into());
        }
        let sample = if let Some(tuple) = remaining.strip_prefix('(') {
            let (tuple, _) = tuple.split_once(')').ok_or("Unterminated color spline")?;
            value(tuple).ok_or("Invalid original color spline")?.0
        } else {
            value(remaining.split(':').next().unwrap_or(""))
                .ok_or("Invalid original scalar spline")?
                .0
        };
        parsed.push(Key {
            time: time.clamp(0.0, 1.0),
            value: sample,
        });
    }
    parsed.sort_by(|a, b| a.time.total_cmp(&b.time));
    parsed.dedup_by(|a, b| a.time == b.time);
    if parsed.is_empty() {
        return Err("Empty original spline".into());
    }
    Ok(parsed)
}
impl TimeOfDay {
    pub fn parse(source: &[u8]) -> Result<Self, String> {
        if source.len() > MAX_XML {
            return Err("TimeOfDay.xml exceeds 4 MiB".into());
        }
        let mut reader = Reader::from_reader(source);
        let mut curves = BTreeMap::new();
        let mut pending: Option<(String, Curve)> = None;
        let mut initial_hour = 12.0;
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
                                }],
                                color,
                            },
                        ))
                    });
                }
                Event::Start(e) | Event::Empty(e)
                    if e.name().as_ref().eq_ignore_ascii_case(b"Spline") =>
                {
                    if let Some((_, c)) = &mut pending {
                        if let Some(raw) = attribute(&e, b"Keys")? {
                            c.keys = keys(&raw)?;
                        }
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
            curves,
            keyed_variables,
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
    let mut reader = Reader::from_reader(source);
    let mut result = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e) | Event::Empty(e))
                if e.name().as_ref().eq_ignore_ascii_case(tag) =>
            {
                if let Ok(Some(item)) = attribute(&e, key) {
                    if item.len() <= 160 && !item.is_empty() && !result.contains(&item) {
                        result.push(item);
                    }
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
    fn source_spline_keys_match_cryengine_xml_layout() {
        let xml = br#"<TimeOfDay Time="13.5"><Variable Name="Sun color" Color="1,1,1"><Spline Keys="0:(0.2:0.3:0.4):36,0.5:(1:0.8:0.6):36,1:(0.2:0.3:0.4):36,"/></Variable><Variable Name="Sun intensity" Value="1000"><Spline Keys="0:1000:36,0.5:120000:36,1:1000:36,"/></Variable></TimeOfDay>"#;
        let parsed = TimeOfDay::parse(xml).unwrap();
        assert_eq!(parsed.initial_hour, 13.5);
        assert_eq!(parsed.keyed_variables, 2);
        assert_eq!(parsed.scalar("Sun intensity", 12.0), Some(120000.0));
        assert!((parsed.color("Sun color", 6.0).unwrap()[0] - 0.6).abs() < 0.001);
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
