//! Read-only original-map atmosphere and local light inputs.
//! SDK-derived *preview* only. No embedded proprietary shader or sky assets.
//! The source map, rather than a global art preset, owns every attachment.
use evolve_core::LevelScene;
use quick_xml::events::{BytesStart, Event};
use std::collections::HashMap;

const MAX_LIGHTS: usize = 2048;
const MAX_LAYER_BYTES: usize = 48 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub(super) struct LocalLight {
    pub position: [f32; 3],
    pub direction: [f32; 3],
    pub color: [f32; 3],
    pub radius: f32,
    pub intensity: f32,
    /// -1 means omni; otherwise cosine of authored projector half-angle.
    pub cone_cos: f32,
}

#[derive(Clone, Debug, Default)]
pub(super) struct SkySource {
    pub material: Option<String>,
    pub angle_degrees: f32,
    pub stretching: f32,
}

fn attr(e: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref().eq_ignore_ascii_case(name))
        .and_then(|a| a.unescape_value().ok().map(|s| s.into_owned()))
}
fn val(e: &BytesStart<'_>, names: &[&[u8]]) -> Option<f32> {
    names
        .iter()
        .find_map(|n| attr(e, n))
        .and_then(|s| s.parse::<f32>().ok())
        .filter(|n| n.is_finite())
}
fn rgb(raw: &str) -> Option<[f32; 3]> {
    let values = raw
        .split(',')
        .map(|s| s.trim().parse::<f32>().ok())
        .collect::<Vec<_>>();
    if values.len() != 3 {
        return None;
    }
    Some([values[0]?, values[1]?, values[2]?].map(|v| v.clamp(0.0, 16.0)))
}
fn rotate(q: [f32; 4], p: [f32; 3]) -> [f32; 3] {
    let [w, x, y, z] = q;
    let uv = [
        y * p[2] - z * p[1],
        z * p[0] - x * p[2],
        x * p[1] - y * p[0],
    ];
    let uuv = [
        y * uv[2] - z * uv[1],
        z * uv[0] - x * uv[2],
        x * uv[1] - y * uv[0],
    ];
    [
        p[0] + 2.0 * (w * uv[0] + uuv[0]),
        p[1] + 2.0 * (w * uv[1] + uuv[1]),
        p[2] + 2.0 * (w * uv[2] + uuv[2]),
    ]
}
fn unit(v: [f32; 3]) -> [f32; 3] {
    let norm = v.iter().map(|n| n * n).sum::<f32>().sqrt();
    if norm < 1e-5 {
        [1.0, 0.0, 0.0]
    } else {
        v.map(|n| n / norm)
    }
}

#[derive(Clone)]
struct PendingLight {
    id: String,
    depth: usize,
    radius: Option<f32>,
    intensity: Option<f32>,
    color: Option<[f32; 3]>,
    cone: Option<f32>,
    enabled: bool,
}
impl PendingLight {
    fn props(&mut self, e: &BytesStart<'_>) {
        if let Some(radius) = val(
            e,
            &[
                b"Radius",
                b"fRadius",
                b"LightRadius",
                b"fAttenuationBulbSize",
            ],
        ) && (0.1..=100000.0).contains(&radius)
        {
            self.radius = Some(radius);
        }
        if let Some(intensity) = val(
            e,
            &[
                b"DiffuseMultiplier",
                b"fDiffuseMultiplier",
                b"LightIntensity",
                b"Intensity",
            ],
        ) && (0.0..=1000000.0).contains(&intensity)
        {
            self.intensity = Some(intensity);
        }
        if let Some(color) = [
            b"DiffuseColor".as_slice(),
            b"clrDiffuse",
            b"Color",
            b"color",
        ]
        .iter()
        .find_map(|name| attr(e, name))
        .and_then(|s| rgb(&s))
        {
            self.color = Some(color);
        }
        if let Some(fov) = val(e, &[b"ProjectorFov", b"fProjectorFov", b"FOV", b"fFov"])
            && (1.0..179.0).contains(&fov)
        {
            self.cone = Some((0.5 * fov.to_radians()).cos());
        }
        if let Some(active) = attr(e, b"bActive").or_else(|| attr(e, b"Active"))
            && (active == "0" || active.eq_ignore_ascii_case("false"))
        {
            self.enabled = false;
        }
    }
}
/// Parse authored placement XML, then match exact GUIDs to the scene's
/// resolved world transforms; nesting/hierarchical placement is not guessed.
/// No global active light, generic object name or bare-geometry fallback.
pub(super) fn lights(
    sources: &[(String, Vec<u8>)],
    scene: Option<&LevelScene>,
) -> (Vec<LocalLight>, Vec<String>) {
    let Some(scene) = scene else {
        return (Vec::new(), Vec::new());
    };
    let objects: HashMap<_, _> = scene
        .objects
        .iter()
        .filter(|o| !o.hidden)
        .map(|o| (o.id.as_str(), o))
        .collect();
    let mut result = Vec::new();
    let mut warnings = Vec::new();
    for (name, raw) in sources {
        if raw.len() > MAX_LAYER_BYTES {
            continue;
        }
        let Ok(xml) = evolve_core::cryxml::xml_bytes(raw, MAX_LAYER_BYTES) else {
            continue;
        };
        let mut reader = quick_xml::Reader::from_reader(xml.as_ref());
        let mut depth = 0usize;
        let mut pending: Option<PendingLight> = None;
        loop {
            let event = match reader.read_event() {
                Ok(v) => v,
                Err(err) => {
                    warnings.push(format!("{name}: original light XML: {err}"));
                    break;
                }
            };
            let nested = matches!(&event, Event::Start(_));
            match event {
                Event::Start(e) | Event::Empty(e) => {
                    if nested {
                        depth += 1;
                    }
                    if depth > 128 {
                        warnings.push(format!("{name}: light XML nesting limit"));
                        break;
                    }
                    let tag = e.name();
                    let is_entity = tag.as_ref().eq_ignore_ascii_case(b"Object")
                        || tag.as_ref().eq_ignore_ascii_case(b"Entity");
                    if is_entity {
                        let class = attr(&e, b"EntityClass")
                            .or_else(|| attr(&e, b"Class"))
                            .unwrap_or_default()
                            .to_ascii_lowercase();
                        let kind = attr(&e, b"Type").unwrap_or_default().to_ascii_lowercase();
                        if (matches!(
                            class.as_str(),
                            "light" | "lightentity" | "pointlight" | "spotlight"
                        ) || (kind == "light" && class.is_empty()))
                            && let Some(id) = attr(&e, b"Id").or_else(|| attr(&e, b"EntityId"))
                        {
                            pending = Some(PendingLight {
                                id: id.to_ascii_lowercase(),
                                depth,
                                radius: None,
                                intensity: None,
                                color: None,
                                cone: None,
                                enabled: true,
                            });
                            if let Some(ref mut p) = pending {
                                p.props(&e);
                            }
                        }
                    } else if let Some(ref mut p) = pending
                        // Empty <Properties .../> is a direct child too:
                        // its tag does not increment the XML reader depth.
                        && depth + usize::from(!nested) > p.depth
                        && depth + usize::from(!nested) <= p.depth + 7
                    {
                        p.props(&e);
                    }
                    // Only a self-closing Light entity, not a self-closing
                    // <Properties/>, can finish the pending light.
                    if !nested && is_entity && pending.as_ref().is_some_and(|p| p.depth == depth) {
                        finalize(&mut pending, &objects, &mut result);
                    }
                }
                Event::End(_) => {
                    if pending.as_ref().is_some_and(|p| p.depth == depth) {
                        finalize(&mut pending, &objects, &mut result);
                    }
                    depth = depth.saturating_sub(1);
                }
                Event::Eof => break,
                Event::DocType(_) | Event::GeneralRef(_) => {
                    warnings.push(format!("{name}: unsafe light XML"));
                    break;
                }
                _ => {}
            }
            if result.len() > MAX_LIGHTS {
                warnings.push("Original map light inventory capped at 2048 entries".into());
                result.truncate(MAX_LIGHTS);
                break;
            }
        }
    }
    (result, warnings)
}
fn finalize(
    pending: &mut Option<PendingLight>,
    objects: &HashMap<&str, &evolve_core::PlacedObject>,
    output: &mut Vec<LocalLight>,
) {
    let Some(p) = pending.take() else {
        return;
    };
    if !p.enabled {
        return;
    }
    let Some(o) = objects.get(p.id.as_str()) else {
        return;
    };
    let (Some(radius), Some(intensity), Some(color)) = (p.radius, p.intensity, p.color) else {
        return;
    };
    if !o.world.position.iter().all(|v| v.is_finite()) {
        return;
    }
    output.push(LocalLight {
        position: o.world.position,
        direction: unit(rotate(o.world.quaternion, [1.0, 0.0, 0.0])),
        radius,
        intensity: intensity.clamp(0.0, 32.0),
        color,
        cone_cos: p.cone.unwrap_or(-1.0),
    });
}

/// CryEngine level placement XML uses EntityClass="ParticleEffect" on an
/// Object/Entity; the parser's scene.kind is merely "Object", not the class.
/// Resolve exact scene GUIDs for world transforms. An authored emitter is
/// source data, not proof its XML particle library/texture is present.
#[derive(Clone, Debug)]
pub(super) struct ParticlePlacement {
    pub effect: String,
    pub position: [f32; 3],
}
pub(super) fn particle_placements(
    sources: &[(String, Vec<u8>)],
    scene: Option<&LevelScene>,
) -> (Vec<ParticlePlacement>, Vec<String>) {
    let Some(scene) = scene else {
        return (Vec::new(), Vec::new());
    };
    let placements: HashMap<_, _> = scene
        .objects
        .iter()
        .filter(|obj| !obj.hidden)
        .map(|obj| (obj.id.as_str(), obj))
        .collect();
    let mut found = Vec::new();
    let mut warnings = Vec::new();
    for (name, bytes) in sources {
        if bytes.len() > MAX_LAYER_BYTES {
            continue;
        }
        let Ok(xml) = evolve_core::cryxml::xml_bytes(bytes, MAX_LAYER_BYTES) else {
            continue;
        };
        let mut xml = quick_xml::Reader::from_reader(xml.as_ref());
        let mut depth = 0usize;
        let mut pending: Option<(usize, String, Option<String>, bool)> = None;
        loop {
            let event = match xml.read_event() {
                Ok(event) => event,
                Err(error) => {
                    warnings.push(format!("{name}: particle placement XML: {error}"));
                    break;
                }
            };
            let nested = matches!(&event, Event::Start(_));
            match event {
                Event::Start(e) | Event::Empty(e) => {
                    if nested {
                        depth += 1;
                    }
                    if depth > 128 {
                        warnings.push(format!("{name}: particle XML depth limit"));
                        break;
                    }
                    let is_object = e.name().as_ref().eq_ignore_ascii_case(b"Object")
                        || e.name().as_ref().eq_ignore_ascii_case(b"Entity");
                    if is_object
                        && attr(&e, b"EntityClass")
                            .or_else(|| attr(&e, b"Class"))
                            .is_some_and(|s| s.eq_ignore_ascii_case("ParticleEffect"))
                    {
                        if let Some(id) = attr(&e, b"Id").or_else(|| attr(&e, b"EntityId")) {
                            let effect = attr(&e, b"ParticleEffect");
                            pending = Some((depth, id.to_ascii_lowercase(), effect, true));
                        }
                    } else if let Some((owner_depth, _, effect, active)) = pending.as_mut()
                        && depth + usize::from(!nested) > *owner_depth
                        && depth + usize::from(!nested) <= *owner_depth + 7
                    {
                        if let Some(source) = attr(&e, b"ParticleEffect") {
                            *effect = Some(source);
                        }
                        if let Some(flag) = attr(&e, b"Active").or_else(|| attr(&e, b"bActive"))
                            && (flag == "0" || flag.eq_ignore_ascii_case("false"))
                        {
                            *active = false;
                        }
                    }
                    if !nested && is_object && pending.as_ref().is_some_and(|p| p.0 == depth) {
                        complete_particle(&mut pending, &placements, &mut found);
                    }
                }
                Event::End(_) => {
                    if pending.as_ref().is_some_and(|p| p.0 == depth) {
                        complete_particle(&mut pending, &placements, &mut found);
                    }
                    depth = depth.saturating_sub(1);
                }
                Event::Eof => break,
                Event::DocType(_) | Event::GeneralRef(_) => {
                    warnings.push(format!("{name}: unsafe particle XML"));
                    break;
                }
                _ => {}
            }
            if found.len() >= 2048 {
                warnings.push("Original map particle emitter inventory capped at 2048".into());
                break;
            }
        }
    }
    (found, warnings)
}
fn complete_particle(
    pending: &mut Option<(usize, String, Option<String>, bool)>,
    placed: &HashMap<&str, &evolve_core::PlacedObject>,
    out: &mut Vec<ParticlePlacement>,
) {
    let Some((_, id, effect, active)) = pending.take() else {
        return;
    };
    if !active {
        return;
    }
    let Some(effect) = effect.filter(|s| !s.trim().is_empty()) else {
        return;
    };
    let Some(obj) = placed.get(id.as_str()) else {
        return;
    };
    if !obj.world.position.iter().all(|v| v.is_finite()) {
        return;
    }
    out.push(ParticlePlacement {
        effect,
        position: obj.world.position,
    });
}

/// Map-owned Environment.xml sky material reference. Only exact references
/// are acceptable for the static sky; no guess by basename or map name.
pub(super) fn sky_source(bytes: &[u8]) -> Result<SkySource, String> {
    let xml = evolve_core::cryxml::xml_bytes(bytes, 4 * 1024 * 1024).map_err(|e| e.to_string())?;
    let mut reader = quick_xml::Reader::from_reader(xml.as_ref());
    let mut found = SkySource::default();
    let mut seen = false;
    loop {
        match reader.read_event().map_err(|e| e.to_string())? {
            Event::Start(e) | Event::Empty(e)
                if e.name().as_ref().eq_ignore_ascii_case(b"SkyBox") =>
            {
                if seen {
                    return Err("Ambiguous duplicate authored SkyBox".into());
                }
                seen = true;
                let material = attr(&e, b"Material").or_else(|| attr(&e, b"MaterialName"));
                if let Some(path) = material {
                    let path = path.replace('\\', "/");
                    if !path.is_empty()
                        && !path.starts_with('/')
                        && !path.contains(':')
                        && !path.split('/').any(|part| matches!(part, "." | ".."))
                    {
                        found.material = Some(path);
                    } else {
                        return Err("Unsafe original sky material path".into());
                    }
                }
                found.angle_degrees = val(&e, &[b"Angle"]).unwrap_or(0.0).clamp(-360.0, 360.0);
                found.stretching = val(&e, &[b"Stretching"]).unwrap_or(1.0).clamp(0.1, 10.0);
            }
            Event::Eof => break,
            Event::DocType(_) | Event::GeneralRef(_) => {
                return Err("Unsafe authored environment XML".into());
            }
            _ => {}
        }
    }
    Ok(found)
}

/// Mapped original sky/TOD controls, sampled per-frame on slider changes.
/// This is physically motivated atmospheric shading, not proprietary Evolve
/// Rayleigh/Mie integration or its HDR adaptation history.
pub(super) fn atmosphere_uniform(
    source: Option<&super::level_environment::TimeOfDay>,
    hour: f32,
    sky: &SkySource,
    static_available: bool,
) -> [[f32; 4]; 10] {
    let color = |key: &str, default: [f32; 3]| {
        source
            .and_then(|tod| tod.color(key, hour))
            .unwrap_or(default)
    };
    let scalar = |key: &str, default: f32| {
        source
            .and_then(|tod| tod.scalar(key, hour))
            .unwrap_or(default)
    };
    let sky_sun = source
        .and_then(|tod| {
            tod.color("Sky light: Sun intensity", hour)
                .or_else(|| tod.color("Sky light sun intensity", hour))
                .or_else(|| tod.color("Sun color", hour))
        })
        .unwrap_or([1.0, 0.94, 0.85]);
    // Stage 2 TimeOfDay.xml names these "Sky light: ..." and "Night sky: ...".
    let authored = |names: [&str; 2], default: f32| {
        source
            .and_then(|tod| {
                tod.scalar(names[0], hour)
                    .or_else(|| tod.scalar(names[1], hour))
            })
            .unwrap_or(default)
    };
    let authored_color = |names: [&str; 2], default: [f32; 3]| {
        source
            .and_then(|tod| {
                tod.color(names[0], hour)
                    .or_else(|| tod.color(names[1], hour))
            })
            .unwrap_or(default)
    };
    let skybox_multiplier = scalar(
        "Skybox multiplier",
        if static_available { 1.0 } else { 0.0 },
    );
    let intensity = authored(
        [
            "Sky light: Sun intensity multiplier",
            "Sky light sun intensity multiplier",
        ],
        1.0,
    );
    let rayleigh = authored(
        ["Sky light: Rayleigh scattering", "Rayleigh scattering"],
        2.0,
    );
    let mie = authored(["Sky light: Mie scattering", "Mie scattering"], 4.8);
    let anisotropy = authored(
        ["Sky light: Sun anisotropy factor", "Sun anisotropy factor"],
        -0.995,
    );
    let waves = [
        authored(["Sky light: Wavelength (R)", "Wavelength R"], 650.),
        authored(["Sky light: Wavelength (G)", "Wavelength G"], 570.),
        authored(["Sky light: Wavelength (B)", "Wavelength B"], 475.),
    ];
    let night_zenith = authored_color(
        ["Night sky: Zenith color", "Night sky zenith color"],
        [0.01, 0.025, 0.065],
    );
    let night_horizon = authored_color(
        ["Night sky: Horizon color", "Night sky horizon color"],
        [0.025, 0.035, 0.065],
    );
    let scene_key = scalar("Scene key", 0.18).clamp(0.005, 4.0);
    let low = scalar("Min exposure", 0.125).clamp(0.01, 8.);
    let high = scalar("Max exposure", 8.).clamp(low, 64.);
    let exposure = (scene_key / 0.18).clamp(low, high);
    let white = scalar("Film curve whitepoint", 4.0).clamp(0.5, 32.);
    [
        [
            sky_sun[0],
            sky_sun[1],
            sky_sun[2],
            intensity.clamp(0., 100.),
        ],
        [
            rayleigh.clamp(0.01, 2000.),
            mie.clamp(0.01, 2000.),
            anisotropy.clamp(-0.999, 0.999),
            if static_available && skybox_multiplier >= 0.5 {
                1.0
            } else {
                0.0
            },
        ],
        [
            waves[0].clamp(380., 780.),
            waves[1].clamp(380., 780.),
            waves[2].clamp(380., 780.),
            sky.angle_degrees.to_radians(),
        ],
        [
            color("Sky color", [0.24, 0.43, 0.75])[0],
            color("Sky color", [0.24, 0.43, 0.75])[1],
            color("Sky color", [0.24, 0.43, 0.75])[2],
            0.0,
        ],
        [
            color("Fog color", [0.47, 0.53, 0.60])[0],
            color("Fog color", [0.47, 0.53, 0.60])[1],
            color("Fog color", [0.47, 0.53, 0.60])[2],
            sky.stretching,
        ],
        [
            night_zenith[0],
            night_zenith[1],
            night_zenith[2],
            authored(
                [
                    "Night sky: Zenith color multiplier",
                    "Night sky zenith color multiplier",
                ],
                1.,
            )
            .clamp(0., 10.),
        ],
        [
            night_horizon[0],
            night_horizon[1],
            night_horizon[2],
            authored(
                [
                    "Night sky: Horizon color multiplier",
                    "Night sky horizon color multiplier",
                ],
                1.,
            )
            .clamp(0., 10.),
        ],
        [
            exposure,
            white,
            scalar("Saturation", 1.).clamp(0., 2.),
            hour,
        ],
        {
            let radial = color("Fog color (radial)", [0.0; 3]);
            let scale = scalar("Fog color (radial) multiplier", 1.).max(0.);
            [
                radial[0] * scale,
                radial[1] * scale,
                radial[2] * scale,
                scalar("Fog radial size", 0.75).clamp(0., 0.999),
            ]
        },
        {
            let top = color("Fog color (top)", [0.0; 3]);
            let scale = scalar("Fog color (top) multiplier", 1.).max(0.);
            [
                top[0] * scale,
                top[1] * scale,
                top[2] * scale,
                scalar("Night sky: Zenith shift", 0.5).clamp(0.01, 1.),
            ]
        },
    ]
}

/// The sun the map records for its authored mission time. Other hours rotate
/// from this reference; the rotation axis is a preview choice, not engine data.
#[derive(Clone, Copy, Debug)]
pub(super) struct AuthoredSun {
    pub to_sun: [f32; 3],
    pub hour: f32,
}

fn xml_attribute<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    // The leading space keeps "x" from matching a longer name like "_x".
    let marker = format!(" {name}=\"");
    let start = text.find(&marker)? + marker.len();
    let length = text[start..].find('"')?;
    Some(&text[start..start + length])
}

fn parse_authored_sun(mission: &str) -> Option<AuthoredSun> {
    let vector = xml_attribute(mission, "SunVector")?
        .split(',')
        .map(|value| value.trim().parse::<f32>().ok())
        .collect::<Option<Vec<_>>>()?;
    let [x, y, z] = vector.as_slice() else {
        return None;
    };
    let length = (x * x + y * y + z * z).sqrt();
    if !length.is_finite() || length < 0.001 {
        return None;
    }
    let (hours, minutes) = xml_attribute(mission, "Time")?.split_once(':')?;
    let hour = hours.parse::<f32>().ok()? + minutes.parse::<f32>().ok()? / 60.0;
    // SunVector points along the light's travel; shading wants toward the sun.
    Some(AuthoredSun {
        to_sun: [-x / length, -y / length, -z / length],
        hour,
    })
}

/// Read the mission from the same installation's `levels/<map>/level.pak`.
pub(super) fn authored_sun(
    archive: &evolve_core::ImportedPak,
    map_name: &str,
) -> Result<Option<AuthoredSun>, String> {
    let root = std::path::Path::new(&archive.directory)
        .parent()
        .ok_or("Imported library unavailable")?;
    let game = std::path::Path::new(&archive.source)
        .parent()
        .ok_or("Imported archive has no source folder")?
        .to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches("//?/")
        .to_ascii_lowercase();
    let wanted = format!("{game}/levels/{}/level.pak", map_name.to_ascii_lowercase());
    let library = evolve_core::imported_library(root).map_err(|e| e.to_string())?;
    for candidate in &library.archives {
        let source = candidate.source.replace('\\', "/");
        if source.trim_start_matches("//?/").to_ascii_lowercase() != wanted {
            continue;
        }
        let mission = std::path::Path::new(&candidate.directory)
            .join("files")
            .join("mission_mission0.xml");
        let Ok(bytes) = std::fs::read(mission) else {
            continue;
        };
        return Ok(parse_authored_sun(&String::from_utf8_lossy(&bytes)));
    }
    Ok(None)
}

fn xml_tuple(text: &str, name: &str) -> Option<Vec<f32>> {
    xml_attribute(text, name)?
        .split(',')
        .map(|value| value.trim().parse::<f32>().ok())
        .collect()
}

/// Mean HDR color of the smallest stored mip on each of a BC6H cubemap's faces.
/// Each face's mip tail is stored largest first in the `.dds.0` head.
fn cubemap_face_means(bytes: &[u8]) -> Result<[[f32; 3]; 6], String> {
    const HEADER: usize = 148;
    if bytes.len() <= HEADER || &bytes[84..88] != b"DX10" {
        return Err("Probe cubemap is not a DX10 DDS".into());
    }
    let format = u32::from_le_bytes(bytes[128..132].try_into().unwrap());
    if format != 95 && format != 96 {
        return Err(format!("Unsupported probe cubemap format {format}"));
    }
    let payload = bytes.len() - HEADER;
    let face = payload / 6;
    if !payload.is_multiple_of(6) || !face.is_multiple_of(16) || face == 0 {
        return Err("Probe cubemap mip tail has an unexpected size".into());
    }
    let mut means = [[0.0_f32; 3]; 6];
    for (index, mean) in means.iter_mut().enumerate() {
        let start = HEADER + face * (index + 1) - 16;
        let mut pixels = [0.0_f32; 48];
        bcdec_rs::bc6h_float(&bytes[start..start + 16], &mut pixels, 12, format == 96);
        for pixel in pixels.as_chunks::<3>().0 {
            for (channel, value) in mean.iter_mut().zip(pixel) {
                *channel += value.max(0.0) / 16.0;
            }
        }
    }
    Ok(means)
}

/// The largest mip a BC6H cubemap head stores, as half-float RGBA per face.
#[derive(Clone, Debug)]
pub(super) struct ProbeCube {
    pub side: u32,
    pub faces: [Vec<u16>; 6],
    pub tint: [f32; 3],
}

fn decode_probe_cube(bytes: &[u8], tint: [f32; 3]) -> Result<ProbeCube, String> {
    const HEADER: usize = 148;
    let payload = bytes.len().saturating_sub(HEADER);
    let face_blocks = payload / 6 / 16;
    // A mip tail ending at one block holds 1 + 4 + 16 + ... blocks per face.
    let (mut top_blocks, mut total) = (1usize, 1usize);
    while total < face_blocks {
        top_blocks *= 4;
        total += top_blocks;
    }
    if total != face_blocks {
        return Err("Probe cubemap mip tail is not a whole chain".into());
    }
    let blocks_wide = top_blocks.isqrt();
    let side = blocks_wide * 4;
    let mut faces: [Vec<u16>; 6] = std::array::from_fn(|_| vec![0x3C00u16; side * side * 4]);
    for (index, face) in faces.iter_mut().enumerate() {
        let start = HEADER + index * face_blocks * 16;
        for block in 0..top_blocks {
            let source = &bytes[start + block * 16..start + (block + 1) * 16];
            let mut pixels = [0u16; 48];
            bcdec_rs::bc6h_half(source, &mut pixels, 12, false);
            let (block_x, block_y) = (block % blocks_wide, block / blocks_wide);
            for y in 0..4 {
                for x in 0..4 {
                    let to = ((block_y * 4 + y) * side + block_x * 4 + x) * 4;
                    face[to..to + 3].copy_from_slice(&pixels[(y * 4 + x) * 3..(y * 4 + x) * 3 + 3]);
                }
            }
        }
    }
    Ok(ProbeCube {
        side: side as u32,
        faces,
        tint,
    })
}

/// Ambient color per direction: +X, -X, +Y, -Y, +Z, -Z.
pub(super) type AmbientCube = [[f32; 4]; 6];

/// Baked ambient per direction from the widest EnvironmentLight in the layers.
pub(super) fn authored_ambient_cube(
    archive: &evolve_core::ImportedPak,
    layers: &[(String, Vec<u8>)],
) -> Result<Option<(AmbientCube, ProbeCube)>, String> {
    let mut best: Option<(f32, String, [f32; 3])> = None;
    for (_, bytes) in layers {
        let text = String::from_utf8_lossy(bytes);
        for (offset, _) in text.match_indices("EntityClass=\"EnvironmentLight\"") {
            let object = &text[offset..];
            let object = &object[..object.find("</Object>").unwrap_or(object.len())];
            let Some(cubemap) = xml_attribute(object, "texture_deferred_cubemap") else {
                continue;
            };
            let box_size = xml_attribute(object, "BoxSizeX")
                .and_then(|value| value.parse::<f32>().ok())
                .unwrap_or(0.0);
            if best.as_ref().is_some_and(|(size, _, _)| *size >= box_size) {
                continue;
            }
            let color = xml_tuple(object, "clrDiffuse").unwrap_or_default();
            let scale = xml_attribute(object, "fDiffuseMultiplier")
                .and_then(|value| value.parse::<f32>().ok())
                .unwrap_or(1.0);
            let tint = match color.as_slice() {
                [r, g, b] => [r * scale, g * scale, b * scale],
                _ => [scale; 3],
            };
            best = Some((box_size, cubemap.to_owned(), tint));
        }
    }
    let Some((_, cubemap, tint)) = best else {
        return Ok(None);
    };
    let root = std::path::Path::new(&archive.directory)
        .parent()
        .ok_or("Imported library unavailable")?;
    let (build, _guard) =
        evolve_core::begin_imported_lookup_session_for_source(root, &archive.source)
            .map_err(|e| e.to_string())?;
    let stem = cubemap
        .rsplit_once('.')
        .map_or(cubemap.as_str(), |(stem, _)| stem);
    let Some(path) = indexed_source(root, &build, &format!("{stem}.dds.0"))? else {
        return Ok(None);
    };
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let means = cubemap_face_means(&bytes)?;
    let cube = decode_probe_cube(&bytes, tint)?;
    Ok(Some((
        std::array::from_fn(|face| {
            [
                means[face][0] * tint[0],
                means[face][1] * tint[1],
                means[face][2] * tint[2],
                0.0,
            ]
        }),
        cube,
    )))
}

/// The map's authored color chart, read from the same installation.
pub(super) fn authored_color_chart(
    archive: &evolve_core::ImportedPak,
    environment: &[u8],
) -> Result<Option<image::RgbaImage>, String> {
    let text = String::from_utf8_lossy(environment);
    let Some(chart) = xml_attribute(&text, "Chart").filter(|chart| !chart.is_empty()) else {
        return Ok(None);
    };
    let root = std::path::Path::new(&archive.directory)
        .parent()
        .ok_or("Imported library unavailable")?;
    let (build, _guard) =
        evolve_core::begin_imported_lookup_session_for_source(root, &archive.source)
            .map_err(|e| e.to_string())?;
    let stem = chart.rsplit_once('.').map_or(chart, |(stem, _)| stem);
    let Some(path) = indexed_source(root, &build, &format!("{stem}.dds.0"))? else {
        return Ok(None);
    };
    let (_, image) = evolve_core::decode_dds_rgba(&path).map_err(|e| e.to_string())?;
    if image.dimensions() != (256, 16) {
        return Err(format!(
            "Unexpected color chart size {:?}",
            image.dimensions()
        ));
    }
    Ok(Some(image))
}

/// Verified authored static sky: CryEngine's Sky material packs four sides
/// in _12 and _34, and the zenith in _5. No sky image is generated when
/// one of the three original cooked textures is absent.
#[derive(Clone, Debug)]
pub(super) struct StaticSky {
    pub images: [std::sync::Arc<image::RgbaImage>; 3],
}
fn indexed_source(
    root: &std::path::Path,
    build: &str,
    virtual_name: &str,
) -> Result<Option<std::path::PathBuf>, String> {
    match evolve_core::resolve_imported_reference(root, virtual_name, Some(build))
        .map_err(|e| e.to_string())?
    {
        evolve_core::LibraryReferenceStatus::Found(item) => Ok(Some(item.absolute_path.into())),
        evolve_core::LibraryReferenceStatus::Missing => Ok(None),
        evolve_core::LibraryReferenceStatus::Ambiguous(_) => Err(format!(
            "Ambiguous sky asset {virtual_name}; no arbitrary source choice"
        )),
    }
}
fn sky_texture(
    root: &std::path::Path,
    build: &str,
    material: &str,
    authored: &str,
) -> Result<image::RgbaImage, String> {
    for cooked in evolve_core::scoped_texture_candidates(authored, material)? {
        let Some(source) = indexed_source(root, build, &cooked)? else {
            continue;
        };
        let (info, rgba) = if cooked.to_ascii_lowercase().ends_with(".dds.0") {
            let stem = cooked.strip_suffix(".0").ok_or("Bad cooked sky DDS path")?;
            let mut parts = Vec::new();
            for i in 1..20 {
                if let Some(path) = indexed_source(root, build, &format!("{stem}.{i}"))? {
                    parts.push(path);
                }
            }
            evolve_core::decode_dds_rgba_with_parts(&source, &parts).map_err(|e| e.to_string())?
        } else {
            evolve_core::decode_dds_rgba(&source).map_err(|e| e.to_string())?
        };
        if info.width > 4096 || info.height > 4096 || info.width < 2 || info.height < 2 {
            return Err("Authored sky texture dimensions unsupported".into());
        }
        return Ok(rgba);
    }
    Err(format!(
        "No same-install cooked texture for original sky {authored}"
    ))
}
pub(super) fn load_static_sky(
    archive: &evolve_core::ImportedPak,
    sky: &SkySource,
) -> Result<Option<StaticSky>, String> {
    let Some(material) = sky.material.as_deref() else {
        return Ok(None);
    };
    let root = std::path::Path::new(&archive.directory)
        .parent()
        .ok_or("Original imported sky library unavailable")?;
    let (build, _guard) =
        evolve_core::begin_imported_lookup_session_for_source(root, &archive.source)
            .map_err(|e| e.to_string())?;
    let material = if material.to_ascii_lowercase().ends_with(".mtl") {
        material.to_owned()
    } else {
        format!("{material}.mtl")
    };
    let path = indexed_source(root, &build, &material)?
        .ok_or_else(|| format!("Original SkyBox material missing: {material}"))?;
    let doc = evolve_core::read_material(&path).map_err(|e| e.to_string())?;
    if doc
        .slots
        .iter()
        .any(|slot| slot.shader.eq_ignore_ascii_case("SkyHDR"))
    {
        // SkyHDR is the dynamic atmospheric shader, not a static cube.
        return Ok(None);
    }
    let diffuse = doc
        .slots
        .iter()
        .filter(|slot| slot.shader.eq_ignore_ascii_case("Sky"))
        .flat_map(|slot| slot.maps.iter())
        .find(|texture| texture.role == "diffuse")
        .map(|texture| texture.path.as_str())
        .ok_or("SkyBox material has no authored Sky diffuse reference")?;
    let base = diffuse.rsplit_once('.').map(|(b, _)| b).unwrap_or(diffuse);
    let prefix = base
        .strip_suffix("_12")
        .or_else(|| base.strip_suffix("_34"))
        .or_else(|| base.strip_suffix("_5"))
        .ok_or("Original static Sky texture is not CryEngine _12/_34/_5 layout")?;
    let extension = diffuse
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .unwrap_or("tif");
    let a = sky_texture(root, &build, &material, &format!("{prefix}_12.{extension}"))?;
    let b = sky_texture(root, &build, &material, &format!("{prefix}_34.{extension}"))?;
    let top = sky_texture(root, &build, &material, &format!("{prefix}_5.{extension}"))?;
    Ok(Some(StaticSky {
        images: [
            std::sync::Arc::new(a),
            std::sync::Arc::new(b),
            std::sync::Arc::new(top),
        ],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn placed_particle_effect_uses_entity_class_and_original_guid() {
        let xml = br#"<Objects>
          <Object Id="fx-one" EntityClass="ParticleEffect" Pos="10,20,30">
            <Properties ParticleEffect="Smoke.Rising" Active="1"/>
          </Object>
          <Object Id="fx-two" EntityClass="ParticleEffect" Pos="99,1,1">
            <Properties ParticleEffect="Dust.Disabled" Active="0"/>
          </Object>
        </Objects>"#;
        let scene = evolve_core::parse_level_scene(&[("effects.lyr", xml.as_slice())]).unwrap();
        assert_eq!(scene.objects[0].kind, "Object");
        let (emissions, warnings) =
            particle_placements(&[("effects.lyr".into(), xml.to_vec())], Some(&scene));
        assert!(warnings.is_empty());
        assert_eq!(emissions.len(), 1);
        assert_eq!(emissions[0].effect, "Smoke.Rising");
        assert_eq!(emissions[0].position, [10.0, 20.0, 30.0]);
    }

    #[test]
    fn source_sky_material_is_not_inferred() {
        assert!(sky_source(br#"<Environment><SkyBox Material="materials/sky/clouds.mtl" Angle="60"/></Environment>"#)
            .unwrap().material.as_deref()==Some("materials/sky/clouds.mtl"));
        assert!(
            sky_source(br#"<Environment><SkyBox Material="../../other.mtl"/></Environment>"#)
                .is_err()
        );
        assert!(sky_source(b"<Environment/>").unwrap().material.is_none());
    }
    #[test]
    fn missing_source_light_parameters_do_not_create_a_light() {
        let scene = LevelScene {
            schema: 1,
            objects: Vec::new(),
            diagnostics: Vec::new(),
        };
        let (lights,_) = lights(&[("map.lyr".into(),br#"<Objects><Object Id="a" EntityClass="Light"><Properties Radius="30"/></Object></Objects>"#.to_vec())],Some(&scene));
        assert!(lights.is_empty());
    }
    #[test]
    fn authored_projector_light_uses_original_properties_and_world_position() {
        let xml = br#"<Objects><Object Id="light-a" EntityClass="Light" Pos="3,5,7"><Properties Radius="30" DiffuseColor="1,0.5,0.25" DiffuseMultiplier="2" ProjectorFov="60"/></Object></Objects>"#;
        let scene = evolve_core::parse_level_scene(&[("map.lyr", xml.as_slice())]).unwrap();
        let (found, warnings) = lights(&[("map.lyr".into(), xml.to_vec())], Some(&scene));
        assert!(warnings.is_empty());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].position, [3.0, 5.0, 7.0]);
        assert_eq!(found[0].color, [1.0, 0.5, 0.25]);
        assert_eq!(found[0].radius, 30.0);
        assert_eq!(found[0].intensity, 2.0);
        assert!((found[0].cone_cos - (30.0_f32.to_radians()).cos()).abs() < 0.0001);
    }

    #[test]
    fn exposure_is_finite_without_original_tod() {
        let p = atmosphere_uniform(None, 12.0, &SkySource::default(), false);
        assert!(p.iter().flatten().all(|v| v.is_finite()));
        assert_eq!(p[1][3], 0.);
    }
}
