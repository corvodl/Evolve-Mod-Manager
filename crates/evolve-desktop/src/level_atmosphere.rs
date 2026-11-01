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
    /// fAttenuationBulbSize, the distance inverse-square falloff starts from.
    pub bulb: f32,
    /// World axis across the projector, for its cookie.
    pub side: [f32; 3],
    /// Index into the map's cookie list, or -1 without one.
    pub cookie: i32,
    /// Tangent of half the projector field of view.
    pub projector_tan: f32,
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
    bulb: Option<f32>,
    cookie: Option<String>,
    fov_tan: Option<f32>,
    enabled: bool,
}
impl PendingLight {
    fn props(&mut self, e: &BytesStart<'_>) {
        if let Some(bulb) = val(e, &[b"fAttenuationBulbSize"]).filter(|bulb| *bulb > 0.0) {
            self.bulb = Some(bulb);
        }
        // Fake lights only feed lens flares and never light the scene.
        if attr(e, b"bFakeLight").is_some_and(|fake| fake == "1") {
            self.enabled = false;
        }
        if let Some(radius) = val(e, &[b"Radius", b"fRadius", b"LightRadius"])
            && (0.1..=100000.0).contains(&radius)
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
        if let Some(cookie) = attr(e, b"texture_Texture").filter(|file| !file.is_empty()) {
            self.cookie = Some(cookie);
        }
        if let Some(fov) = val(e, &[b"ProjectorFov", b"fProjectorFov", b"FOV", b"fFov"])
            && (1.0..179.0).contains(&fov)
        {
            self.fov_tan = Some((0.5 * fov.to_radians()).tan());
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
) -> (Vec<LocalLight>, Vec<String>, Vec<String>) {
    let Some(scene) = scene else {
        return (Vec::new(), Vec::new(), Vec::new());
    };
    let mut cookies = Vec::new();
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
                                bulb: None,
                                cookie: None,
                                fov_tan: None,
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
                        finalize(&mut pending, &objects, &mut result, &mut cookies);
                    }
                }
                Event::End(_) => {
                    if pending.as_ref().is_some_and(|p| p.depth == depth) {
                        finalize(&mut pending, &objects, &mut result, &mut cookies);
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
    (result, warnings, cookies)
}
fn finalize(
    pending: &mut Option<PendingLight>,
    objects: &HashMap<&str, &evolve_core::PlacedObject>,
    output: &mut Vec<LocalLight>,
    cookies: &mut Vec<String>,
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
    let cookie = p.cookie.filter(|_| p.cone.is_some()).map_or(-1, |file| {
        let at = cookies.iter().position(|known| *known == file);
        at.unwrap_or_else(|| {
            cookies.push(file);
            cookies.len() - 1
        }) as i32
    });
    output.push(LocalLight {
        side: unit(rotate(o.world.quaternion, [0.0, 1.0, 0.0])),
        cookie,
        projector_tan: p.fov_tan.unwrap_or(1.0),
        position: o.world.position,
        direction: unit(rotate(o.world.quaternion, [1.0, 0.0, 0.0])),
        radius,
        intensity: intensity.clamp(0.0, 1000.0),
        color,
        cone_cos: p.cone.unwrap_or(-1.0),
        bulb: p.bulb.unwrap_or(0.05),
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
    /// w, x, y, z.
    pub rotation: [f32; 4],
}

/// Particles sharing one sprite. Each instance is origin and phase, velocity
/// and life, birth and full size with gravity and alpha, tint and spin, tiles
/// across and down with framerate and first tile, then diffuse, emissive, the
/// radius it wraps around the camera (0 stays put), whether it adds light, its
/// X and Y scale with whether it stretches along its motion, and its fade.
#[derive(Clone, Debug)]
pub(super) struct ParticleBatch {
    pub sprite: std::sync::Arc<image::RgbaImage>,
    pub instances: Vec<[f32; 32]>,
}

// Across the whole map. Effects past this draw nothing instead of stalling.
const MAX_PARTICLES: usize = 60_000;
// The game keeps rain around the player. Placed sheets repeat within this reach.
const RAIN_WRAP_RADIUS: f32 = 70.0;

/// The named particle library in this installation's own libs.pak. Several
/// builds can hold the same virtual path, so this looks only in that one pak.
fn particle_library_path(
    root: &std::path::Path,
    source: &str,
    library: &str,
) -> Option<std::path::PathBuf> {
    let catalog = evolve_core::imported_library(root).ok()?;
    let pak = catalog.archives.iter().find(|pak| {
        pak.name.eq_ignore_ascii_case("libs.pak")
            && std::path::Path::new(&pak.source).parent() == std::path::Path::new(source).parent()
    })?;
    let files = std::path::Path::new(&pak.directory).join("files");
    let wanted = format!("{library}.xml");
    let matching = |parent: &std::path::Path, name: &str| {
        std::fs::read_dir(parent).ok()?.flatten().find_map(|entry| {
            entry
                .file_name()
                .to_str()?
                .eq_ignore_ascii_case(name)
                .then(|| entry.path())
        })
    };
    let directory = matching(&matching(&files, "libs")?, "particles")?;
    matching(&directory, &wanted)
}

/// Expand every placed effect into the looping particles its emitters keep alive.
pub(super) fn authored_particles(
    archive: &evolve_core::ImportedPak,
    placed: &[ParticlePlacement],
) -> Result<Vec<ParticleBatch>, String> {
    let root = std::path::Path::new(&archive.directory)
        .parent()
        .ok_or("Imported library unavailable")?;
    let (build, _guard) =
        evolve_core::begin_imported_lookup_session_for_source(root, &archive.source)
            .map_err(|e| e.to_string())?;
    let mut libraries = HashMap::<String, Vec<evolve_core::ParticleEmitter>>::new();
    let mut sprites = HashMap::<String, Option<std::sync::Arc<image::RgbaImage>>>::new();
    let mut batches = HashMap::<String, ParticleBatch>::new();
    let mut total = 0usize;
    // Camera-following rain overlaps every placement, so only a few are kept.
    let mut wrapped = HashMap::<String, u32>::new();
    let mut seed = 0x2545_F491_u32;
    let mut random = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32
    };
    for place in placed {
        let Some((library, effect)) = place.effect.split_once('.') else {
            continue;
        };
        let key = library.to_ascii_lowercase();
        if !libraries.contains_key(&key) {
            let emitters = particle_library_path(root, &archive.source, library)
                .and_then(|path| std::fs::read(path).ok())
                .and_then(|bytes| evolve_core::decode_particle_library(&bytes).ok())
                .unwrap_or_default();
            libraries.insert(key.clone(), emitters);
        }
        let child_prefix = format!("{}.", effect.to_ascii_lowercase());
        for emitter in &libraries[&key] {
            let name = emitter.effect.to_ascii_lowercase();
            if name != effect.to_ascii_lowercase() && !name.starts_with(&child_prefix) {
                continue;
            }
            let Some(texture) = emitter.texture.as_ref().filter(|_| {
                emitter.enabled && emitter.continuous && emitter.count > 0.0 && emitter.size > 0.0
            }) else {
                continue;
            };
            let sprite = sprites.entry(texture.clone()).or_insert_with(|| {
                let stem = texture
                    .rsplit_once('.')
                    .map_or(texture.as_str(), |(stem, _)| stem);
                let path = indexed_source(root, &build, &format!("{stem}.dds.0")).ok()??;
                evolve_core::decode_dds_rgba(&path)
                    .ok()
                    .map(|(_, image)| std::sync::Arc::new(image))
            });
            let Some(sprite) = sprite.clone() else {
                continue;
            };
            let is_rain = name.contains("rain") && emitter.gravity_scale > 0.0;
            if is_rain {
                let kept = wrapped.entry(name.clone()).or_insert(0);
                *kept += 1;
                if *kept > 3 {
                    continue;
                }
            }
            let batch = batches
                .entry(texture.clone())
                .or_insert_with(|| ParticleBatch {
                    sprite,
                    instances: Vec::new(),
                });
            let axis = rotate(place.rotation, [0.0, 0.0, 1.0]);
            let side = unit(rotate(place.rotation, [1.0, 0.0, 0.0]));
            let other = unit(rotate(place.rotation, [0.0, 1.0, 0.0]));
            for _ in 0..(emitter.count.ceil() as usize).min(256) {
                if total >= MAX_PARTICLES {
                    break;
                }
                total += 1;
                let tilt = emitter.spread.to_radians() * random();
                let turn = std::f32::consts::TAU * random();
                let direction: [f32; 3] = std::array::from_fn(|i| {
                    axis[i] * tilt.cos()
                        + (side[i] * turn.cos() + other[i] * turn.sin()) * tilt.sin()
                });
                let jitter = emitter
                    .random_offset
                    .map(|reach| (random() * 2.0 - 1.0) * reach);
                let origin: [f32; 3] = std::array::from_fn(|i| {
                    place.position[i]
                        + side[i] * jitter[0]
                        + other[i] * jitter[1]
                        + axis[i] * jitter[2]
                });
                let frames = emitter.tiling[0] * emitter.tiling[1];
                batch.instances.push([
                    origin[0],
                    origin[1],
                    origin[2],
                    random(),
                    direction[0] * emitter.speed,
                    direction[1] * emitter.speed,
                    direction[2] * emitter.speed,
                    emitter.lifetime * (0.85 + random() * 0.3),
                    emitter.size * emitter.size_birth,
                    emitter.size,
                    emitter.gravity_scale,
                    emitter.alpha,
                    emitter.color[0],
                    emitter.color[1],
                    emitter.color[2],
                    random() * std::f32::consts::TAU,
                    emitter.tiling[0],
                    emitter.tiling[1],
                    emitter.tiling[2],
                    (random() * frames).floor(),
                    emitter.diffuse_lighting,
                    emitter.emissive_lighting,
                    if is_rain { RAIN_WRAP_RADIUS } else { 0.0 },
                    if emitter.additive { 1.0 } else { 0.0 },
                    emitter.scale[0],
                    emitter.scale[1],
                    if emitter.oriented { 1.0 } else { 0.0 },
                    0.0,
                    emitter.fade[0],
                    emitter.fade[1],
                    emitter.fade[2],
                    emitter.fade[3],
                ]);
            }
        }
    }
    Ok(batches.into_values().collect())
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
                            // The weather system switches these on while it runs.
                            && attr(&e, b"bWeatherEmitter").as_deref() != Some("1")
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
        rotation: obj.world.quaternion,
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
            color("Sky color", [0.0, 0.0, 0.0])[0],
            color("Sky color", [0.0, 0.0, 0.0])[1],
            color("Sky color", [0.0, 0.0, 0.0])[2],
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

const DX10_HEADER: usize = 148;

fn bc6h_format(bytes: &[u8]) -> Result<bool, String> {
    if bytes.len() <= DX10_HEADER || &bytes[84..88] != b"DX10" {
        return Err("Probe cubemap is not a DX10 DDS".into());
    }
    match u32::from_le_bytes(bytes[128..132].try_into().unwrap()) {
        95 => Ok(false),
        96 => Ok(true),
        format => Err(format!("Unsupported probe cubemap format {format}")),
    }
}

/// Mean HDR color of every stored block on each cubemap face.
fn cubemap_face_means(bytes: &[u8]) -> Result<[[f32; 3]; 6], String> {
    let signed = bc6h_format(bytes)?;
    let payload = bytes.len() - DX10_HEADER;
    let face = payload / 6;
    if !payload.is_multiple_of(6) || !face.is_multiple_of(16) || face == 0 {
        return Err("Probe cubemap has an unexpected size".into());
    }
    let mut means = [[0.0_f32; 3]; 6];
    for (index, mean) in means.iter_mut().enumerate() {
        let start = DX10_HEADER + face * index;
        let blocks = face / 16;
        for block in 0..blocks {
            let mut pixels = [0.0_f32; 48];
            let at = start + block * 16;
            bcdec_rs::bc6h_float(&bytes[at..at + 16], &mut pixels, 12, signed);
            for pixel in pixels.as_chunks::<3>().0 {
                for (channel, value) in mean.iter_mut().zip(pixel) {
                    *channel += value.max(0.0) / (16.0 * blocks as f32);
                }
            }
        }
    }
    Ok(means)
}

/// One cubemap mip as half-float RGBA per face, largest level first.
#[derive(Clone, Debug)]
pub(super) struct ProbeLevel {
    pub side: u32,
    pub faces: [Vec<u16>; 6],
}

#[derive(Clone, Debug)]
pub(super) struct ProbeCube {
    pub levels: Vec<ProbeLevel>,
    pub tint: [f32; 3],
}

fn decode_probe_level(blocks: &[u8], blocks_wide: usize, signed: bool) -> Vec<u16> {
    let side = blocks_wide * 4;
    let mut face = vec![0x3C00u16; side * side * 4];
    for block in 0..blocks_wide * blocks_wide {
        let mut pixels = [0u16; 48];
        bcdec_rs::bc6h_half(
            &blocks[block * 16..(block + 1) * 16],
            &mut pixels,
            12,
            signed,
        );
        let (block_x, block_y) = (block % blocks_wide, block / blocks_wide);
        for y in 0..4 {
            for x in 0..4 {
                let to = ((block_y * 4 + y) * side + block_x * 4 + x) * 4;
                face[to..to + 3].copy_from_slice(&pixels[(y * 4 + x) * 3..(y * 4 + x) * 3 + 3]);
            }
        }
    }
    face
}

/// Every mip stored for a probe: numbered files hold one level across all six
/// faces, and the `.dds.0` head holds each face's remaining tail.
fn decode_probe_cube(
    head_path: &std::path::Path,
    head: &[u8],
    tint: [f32; 3],
    max_side: u32,
) -> Result<ProbeCube, String> {
    let signed = bc6h_format(head)?;
    let payload = head.len() - DX10_HEADER;
    let face_blocks = payload / 6 / 16;
    let (mut top_blocks, mut total) = (1usize, 1usize);
    while total < face_blocks {
        top_blocks *= 4;
        total += top_blocks;
    }
    if total != face_blocks {
        return Err("Probe cubemap mip tail is not a whole chain".into());
    }
    let mut levels = Vec::new();
    let name = head_path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".0"))
        .ok_or("Probe cubemap head is not a numbered DDS")?
        .to_owned();
    let mut blocks_wide = top_blocks.isqrt() * 2;
    let mut part = 1;
    let mut upper = Vec::new();
    loop {
        if (blocks_wide * 4) as u32 > max_side {
            break;
        }
        let path = head_path.with_file_name(format!("{name}.{part}"));
        let Ok(bytes) = std::fs::read(path) else {
            break;
        };
        if bytes.len() != 6 * blocks_wide * blocks_wide * 16 {
            break;
        }
        let per_face = bytes.len() / 6;
        upper.push(ProbeLevel {
            side: (blocks_wide * 4) as u32,
            faces: std::array::from_fn(|face| {
                decode_probe_level(
                    &bytes[face * per_face..(face + 1) * per_face],
                    blocks_wide,
                    signed,
                )
            }),
        });
        blocks_wide *= 2;
        part += 1;
    }
    upper.reverse();
    levels.extend(upper);
    let mut wide = top_blocks.isqrt();
    let mut offset = 0usize;
    while wide >= 1 {
        let size = wide * wide * 16;
        if (wide * 4) as u32 <= max_side {
            levels.push(ProbeLevel {
                side: (wide * 4) as u32,
                faces: std::array::from_fn(|face| {
                    let at = DX10_HEADER + face * face_blocks * 16 + offset;
                    decode_probe_level(&head[at..at + size], wide, signed)
                }),
            });
        }
        offset += size;
        wide /= 2;
    }
    Ok(ProbeCube { levels, tint })
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
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    // The `_diff` companion is the probe's baked irradiance.
    let diffuse = indexed_source(root, &build, &format!("{stem}_diff.dds.0"))?
        .and_then(|path| std::fs::read(path).ok());
    let means = cubemap_face_means(diffuse.as_deref().unwrap_or(&bytes))?;
    let cube = decode_probe_cube(&path, &bytes, tint, u32::MAX)?;
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

/// An authored EnvironmentLight box other than the map's global probe.
#[derive(Clone, Debug)]
pub(super) struct LocalProbe {
    pub center: [f32; 3],
    /// World axes of the box, each with its inverse half size.
    pub axes: [[f32; 4]; 3],
    pub half_diagonal: f32,
    pub tint: [f32; 3],
    pub falloff: f32,
    pub priority: f32,
    pub ambient: [[f32; 4]; 6],
    pub cube: ProbeCube,
}

/// The side of each local probe's largest uploaded mip.
pub(super) const LOCAL_PROBE_SIDE: u32 = 64;

/// Every EnvironmentLight except the widest, which is already the global probe.
pub(super) fn authored_local_probes(
    archive: &evolve_core::ImportedPak,
    layers: &[(String, Vec<u8>)],
) -> Result<Vec<LocalProbe>, String> {
    struct Pending {
        center: [f32; 3],
        rotation: [f32; 4],
        size: [f32; 3],
        tint: [f32; 3],
        falloff: f32,
        priority: f32,
        cubemap: String,
    }
    let mut pending = Vec::new();
    for (_, bytes) in layers {
        let text = String::from_utf8_lossy(bytes);
        for (offset, _) in text.match_indices("EntityClass=\"EnvironmentLight\"") {
            let start = text[..offset].rfind("<Object").unwrap_or(offset);
            let object = &text[start..];
            let object = &object[..object.find("</Object>").unwrap_or(object.len())];
            let (Some(cubemap), Some(position)) = (
                xml_attribute(object, "texture_deferred_cubemap"),
                xml_tuple(object, "Pos").filter(|pos| pos.len() == 3),
            ) else {
                continue;
            };
            if xml_attribute(object, "bActive") == Some("0") {
                continue;
            }
            let rotation = xml_tuple(object, "Rotate")
                .filter(|rotate| rotate.len() == 4)
                .map_or([1.0, 0.0, 0.0, 0.0], |rotate| {
                    [rotate[0], rotate[1], rotate[2], rotate[3]]
                });
            let scale = mtl_value(object, "fDiffuseMultiplier", 1.0);
            let color = xml_tuple(object, "clrDiffuse").unwrap_or_default();
            pending.push(Pending {
                center: [position[0], position[1], position[2]],
                rotation,
                size: [
                    mtl_value(object, "BoxSizeX", 0.0),
                    mtl_value(object, "BoxSizeY", 0.0),
                    mtl_value(object, "BoxSizeZ", 0.0),
                ],
                tint: match color.as_slice() {
                    [r, g, b] => [r * scale, g * scale, b * scale],
                    _ => [scale; 3],
                },
                falloff: mtl_value(object, "fAttenuationFalloffMax", 0.3),
                priority: mtl_value(object, "SortPriority", 0.0),
                cubemap: cubemap.to_owned(),
            });
        }
    }
    let widest =
        (0..pending.len()).max_by(|&a, &b| pending[a].size[0].total_cmp(&pending[b].size[0]));
    if let Some(widest) = widest {
        pending.swap_remove(widest);
    }
    let root = std::path::Path::new(&archive.directory)
        .parent()
        .ok_or("Imported library unavailable")?;
    let (build, _guard) =
        evolve_core::begin_imported_lookup_session_for_source(root, &archive.source)
            .map_err(|e| e.to_string())?;
    let mut located = Vec::new();
    for probe in pending {
        let stem = probe
            .cubemap
            .rsplit_once('.')
            .map_or(probe.cubemap.as_str(), |(stem, _)| stem)
            .to_owned();
        if let Some(path) = indexed_source(root, &build, &format!("{stem}.dds.0"))? {
            let diffuse = indexed_source(root, &build, &format!("{stem}_diff.dds.0"))?;
            located.push((probe, path, diffuse));
        }
    }
    let threads = std::thread::available_parallelism().map_or(4, |count| count.get());
    let chunk = located.len().div_ceil(threads).max(1);
    let decoded = std::thread::scope(|scope| {
        let workers = located
            .chunks(chunk)
            .map(|batch| {
                scope.spawn(move || {
                    batch
                        .iter()
                        .filter_map(|(probe, path, diffuse)| {
                            let bytes = std::fs::read(path).ok()?;
                            let irradiance = diffuse.as_ref().and_then(|d| std::fs::read(d).ok());
                            let means =
                                cubemap_face_means(irradiance.as_deref().unwrap_or(&bytes)).ok()?;
                            let cube =
                                decode_probe_cube(path, &bytes, probe.tint, LOCAL_PROBE_SIDE)
                                    .ok()?;
                            let half = probe.size.map(|size| (size * 0.5).max(0.01));
                            let axis = |unit: [f32; 3], half: f32| {
                                let world = rotate(probe.rotation, unit);
                                [world[0], world[1], world[2], 1.0 / half]
                            };
                            Some(LocalProbe {
                                center: probe.center,
                                axes: [
                                    axis([1.0, 0.0, 0.0], half[0]),
                                    axis([0.0, 1.0, 0.0], half[1]),
                                    axis([0.0, 0.0, 1.0], half[2]),
                                ],
                                half_diagonal: half.iter().map(|h| h * h).sum::<f32>().sqrt(),
                                tint: probe.tint,
                                falloff: probe.falloff.max(0.001),
                                priority: probe.priority,
                                ambient: std::array::from_fn(|face| {
                                    [
                                        means[face][0] * probe.tint[0],
                                        means[face][1] * probe.tint[1],
                                        means[face][2] * probe.tint[2],
                                        0.0,
                                    ]
                                }),
                                cube,
                            })
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap_or_default())
            .collect::<Vec<_>>()
    });
    Ok(decoded)
}

/// Each named projector texture, in list order; absent ones leave a gap.
pub(super) fn authored_light_cookies(
    archive: &evolve_core::ImportedPak,
    files: &[String],
) -> Result<Vec<Option<image::RgbaImage>>, String> {
    let root = std::path::Path::new(&archive.directory)
        .parent()
        .ok_or("Imported library unavailable")?;
    let (build, _guard) =
        evolve_core::begin_imported_lookup_session_for_source(root, &archive.source)
            .map_err(|e| e.to_string())?;
    Ok(files
        .iter()
        .map(|file| {
            let stem = file
                .rsplit_once('.')
                .map_or(file.as_str(), |(stem, _)| stem);
            let path = indexed_source(root, &build, &format!("{stem}.dds.0")).ok()??;
            evolve_core::decode_dds_rgba(&path).ok().map(|(_, image)| {
                image::imageops::resize(&image, 128, 128, image::imageops::FilterType::Triangle)
            })
        })
        .collect())
}

/// How much the map's Rain entity darkens wet surfaces, when it has one.
pub(super) fn authored_rain_darkening(layers: &[(String, Vec<u8>)]) -> f32 {
    layers
        .iter()
        .find_map(|(_, bytes)| {
            let text = String::from_utf8_lossy(bytes);
            let at = text.find("EntityClass=\"Rain\"")?;
            let object = &text[at..];
            let object = &object[..object.find("</Object>").unwrap_or(object.len())];
            xml_attribute(object, "fDiffuseDarkening")?
                .parse::<f32>()
                .ok()
        })
        .filter(|value| value.is_finite())
        .map_or(0.0, |value| value.clamp(0.0, 1.0))
}

/// One authored WaterVolume, as triangles at its world height.
#[derive(Clone, Debug)]
pub(super) struct WaterSurface {
    pub triangles: Vec<[f32; 3]>,
    pub tint: [f32; 3],
    pub decal_strength: f32,
    /// Tilling, DetailTilling, DecalTilling, NormalsScale.
    pub tiling: [f32; 4],
    /// DetailNormalsScale, SoftIntersectionFactor, FoamAmount, FoamSoftIntersectionFactor.
    pub surface: [f32; 4],
    /// GlossMapTilling, GlossMapScale, GlossMapBias, unused.
    pub gloss: [f32; 4],
    /// FogColor scaled by its multiplier, then FogDensity.
    pub fog: [f32; 4],
    pub specular: [f32; 3],
    pub foam_tilling: f32,
    pub decal_falloff: f32,
    pub reflection: f32,
    pub decal: Option<std::sync::Arc<image::RgbaImage>>,
    pub gloss_map: Option<std::sync::Arc<image::RgbaImage>>,
}

/// Ear clipping for the simple polygons authored as water outlines.
fn triangulate(points: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let cross = |a: [f32; 3], b: [f32; 3], c: [f32; 3]| {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    };
    let mut ring = points.to_vec();
    let area = (0..ring.len())
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f32>();
    if area < 0.0 {
        ring.reverse();
    }
    let mut triangles = Vec::with_capacity(points.len().saturating_sub(2) * 3);
    while ring.len() > 3 {
        let count = ring.len();
        let ear = (0..count).find(|&i| {
            let (a, b, c) = (
                ring[(i + count - 1) % count],
                ring[i],
                ring[(i + 1) % count],
            );
            cross(a, b, c) > 0.0
                && !ring.iter().any(|&p| {
                    p != a
                        && p != b
                        && p != c
                        && cross(a, b, p) >= 0.0
                        && cross(b, c, p) >= 0.0
                        && cross(c, a, p) >= 0.0
                })
        });
        let Some(index) = ear else {
            break;
        };
        triangles.extend([
            ring[(index + count - 1) % count],
            ring[index],
            ring[(index + 1) % count],
        ]);
        ring.remove(index);
    }
    if ring.len() == 3 {
        triangles.extend_from_slice(&ring);
    }
    triangles
}

fn mtl_value(text: &str, name: &str, default: f32) -> f32 {
    xml_attribute(text, name)
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| value.is_finite())
        .unwrap_or(default)
}

/// Every authored WaterVolume in the map's layers, with its material values.
pub(super) fn authored_water(
    archive: &evolve_core::ImportedPak,
    layers: &[(String, Vec<u8>)],
) -> Result<Vec<WaterSurface>, String> {
    let root = std::path::Path::new(&archive.directory)
        .parent()
        .ok_or("Imported library unavailable")?;
    let (build, _guard) =
        evolve_core::begin_imported_lookup_session_for_source(root, &archive.source)
            .map_err(|e| e.to_string())?;
    let mut surfaces = Vec::new();
    for (_, bytes) in layers {
        let text = String::from_utf8_lossy(bytes);
        for (offset, _) in text.match_indices(" Type=\"WaterVolume\"") {
            let start = text[..offset].rfind("<Object").unwrap_or(offset);
            let object = &text[start..];
            let object = &object[..object.find("</Object>").unwrap_or(object.len())];
            let Some(origin) = xml_tuple(object, "Pos").filter(|pos| pos.len() == 3) else {
                continue;
            };
            let points = object
                .match_indices("<Point ")
                .filter_map(|(at, _)| xml_tuple(&object[at..], "Pos"))
                .filter(|point| point.len() == 3)
                // The polygon is flat: use the volume's own height everywhere.
                .map(|point| [origin[0] + point[0], origin[1] + point[1], origin[2]])
                .collect::<Vec<_>>();
            if points.len() < 3 {
                continue;
            }
            let material = xml_attribute(object, "Material").unwrap_or_default();
            let mtl = indexed_source(root, &build, &format!("{material}.mtl"))
                .ok()
                .flatten()
                .and_then(|path| std::fs::read_to_string(path).ok())
                .unwrap_or_default();
            let tint = xml_tuple(&mtl, "Diffuse").unwrap_or_default();
            let material_map = |map: &str| {
                mtl.find(&format!("Map=\"{map}\""))
                    .and_then(|at| xml_attribute(&mtl[at..], "File"))
                    .and_then(|file| {
                        let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
                        let path =
                            indexed_source(root, &build, &format!("{stem}.dds.0")).ok()??;
                        evolve_core::decode_dds_rgba(&path).ok()
                    })
                    .map(|(_, image)| std::sync::Arc::new(image))
            };
            let decal = material_map("Decal").or_else(|| material_map("Custom"));
            let gloss_map = material_map("Specular");
            let fog_color = xml_tuple(object, "FogColor").unwrap_or_default();
            let fog_scale = mtl_value(object, "FogColorMultiplier", 1.0);
            let specular = xml_tuple(&mtl, "Specular").unwrap_or_default();
            surfaces.push(WaterSurface {
                triangles: triangulate(&points),
                tint: match tint.as_slice() {
                    [r, g, b] => [*r, *g, *b],
                    _ => [0.06, 0.07, 0.05],
                },
                decal_strength: mtl_value(&mtl, "DecalStrength", 0.5),
                tiling: [
                    mtl_value(&mtl, "Tilling", 0.1),
                    mtl_value(&mtl, "DetailTilling", 3.0),
                    mtl_value(&mtl, "DecalTilling", 0.05),
                    mtl_value(&mtl, "NormalsScale", 1.0),
                ],
                surface: [
                    mtl_value(&mtl, "DetailNormalsScale", 1.0),
                    mtl_value(&mtl, "SoftIntersectionFactor", 1.0),
                    mtl_value(&mtl, "FoamAmount", 0.0),
                    mtl_value(&mtl, "FoamSoftIntersectionFactor", 1.0),
                ],
                foam_tilling: mtl_value(&mtl, "FoamTilling", 1.0),
                decal_falloff: mtl_value(&mtl, "DecalFalloff", 1.0),
                reflection: mtl_value(&mtl, "RealtimeReflMul", 1.0),
                gloss: [
                    mtl_value(&mtl, "GlossMapTilling", 0.1),
                    mtl_value(&mtl, "GlossMapScale", 0.5),
                    mtl_value(&mtl, "GlossMapBias", 0.5),
                    0.0,
                ],
                fog: match fog_color.as_slice() {
                    [r, g, b] => [
                        r * fog_scale,
                        g * fog_scale,
                        b * fog_scale,
                        mtl_value(object, "FogDensity", 1.0),
                    ],
                    _ => [0.0, 0.0, 0.0, mtl_value(object, "FogDensity", 1.0)],
                },
                specular: match specular.as_slice() {
                    [r, g, b] => [*r, *g, *b],
                    _ => [0.1; 3],
                },
                decal,
                gloss_map,
            });
        }
    }
    Ok(surfaces)
}

/// One planet or moon from Environment.xml.
#[derive(Clone, Debug)]
pub(super) struct CelestialBody {
    /// Unit direction, then authored Size.
    pub direction_size: [f32; 4],
    pub rotation_degrees: f32,
    pub image: std::sync::Arc<image::RgbaImage>,
}

/// CelestialBody1..N. The size scale is calibrated
/// to in-game captures; only the authored numbers come from the map.
pub(super) fn authored_celestial_bodies(
    archive: &evolve_core::ImportedPak,
    environment: &[u8],
) -> Result<Vec<CelestialBody>, String> {
    let text = String::from_utf8_lossy(environment);
    let root = std::path::Path::new(&archive.directory)
        .parent()
        .ok_or("Imported library unavailable")?;
    let (build, _guard) =
        evolve_core::begin_imported_lookup_session_for_source(root, &archive.source)
            .map_err(|e| e.to_string())?;
    let mut bodies = Vec::new();
    for index in 1..=4 {
        let Some(at) = text.find(&format!("<CelestialBody{index} ")) else {
            continue;
        };
        let element = &text[at..];
        let element = &element[..element.find("/>").unwrap_or(element.len())];
        let Some(file) = xml_attribute(element, "Texture").filter(|file| !file.is_empty()) else {
            continue;
        };
        let number = |name: &str| mtl_value(element, name, 0.0);
        let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
        let Some(path) = indexed_source(root, &build, &format!("{stem}.dds.0"))? else {
            continue;
        };
        let (_, image) = evolve_core::decode_dds_rgba(&path).map_err(|e| e.to_string())?;
        // CryEngine's moon placement: latitude turns from -180 degrees, and
        // longitude tips down from straight up.
        let (azimuth, polar) = (
            (number("Latitude") - 180.0).to_radians(),
            (90.0 - number("Longitude")).to_radians(),
        );
        bodies.push(CelestialBody {
            direction_size: [
                polar.sin() * azimuth.cos(),
                polar.sin() * azimuth.sin(),
                polar.cos(),
                number("Size"),
            ],
            rotation_degrees: number("Rotation"),
            image: std::sync::Arc::new(image),
        });
    }
    Ok(bodies)
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
        let (lights,_,_) = lights(&[("map.lyr".into(),br#"<Objects><Object Id="a" EntityClass="Light"><Properties Radius="30"/></Object></Objects>"#.to_vec())],Some(&scene));
        assert!(lights.is_empty());
    }
    #[test]
    fn authored_projector_light_uses_original_properties_and_world_position() {
        let xml = br#"<Objects><Object Id="light-a" EntityClass="Light" Pos="3,5,7"><Properties Radius="30" DiffuseColor="1,0.5,0.25" DiffuseMultiplier="2" ProjectorFov="60"/></Object></Objects>"#;
        let scene = evolve_core::parse_level_scene(&[("map.lyr", xml.as_slice())]).unwrap();
        let (found, warnings, _) = lights(&[("map.lyr".into(), xml.to_vec())], Some(&scene));
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
