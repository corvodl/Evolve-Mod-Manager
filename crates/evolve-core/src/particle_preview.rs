//! Bounded, source-backed CryEngine particle emitter inspection and
//! deterministic *preview* simulation. Original Evolve Particles libraries
//! are CryXmlB; no proprietary data is embedded or distributed.
//! Unsupported curves, per-particle physics and spawning geometry are
//! explicitly not inferred from the effect name.
use quick_xml::events::{BytesStart, Event};
use serde::{Deserialize, Serialize};

const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_EMITTERS: usize = 16384;
const MAX_DEPTH: usize = 64;
pub const MAX_LIVE_PARTICLES: usize = 32768;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ParticleEmitter {
    /// The source ParticleLibrary Name, e.g. map_ice_crashsite.
    pub library: String,
    /// The exact hierarchical authored effect path, including child names.
    pub effect: String,
    pub texture: Option<String>,
    pub material: Option<String>,
    /// Original Count parameter, as a nonnegative scalar; this does not
    /// imply an exact engine-specific spawn-rate formula.
    pub count: f32,
    pub lifetime: f32,
    pub speed: f32,
    pub size: f32,
    pub alpha: f32,
    pub color: [f32; 3],
    pub continuous: bool,
    pub enabled: bool,
    pub gravity_scale: f32,
    /// Size at birth as a fraction of Size, from its ParticleAge curve.
    #[serde(default)]
    pub size_birth: f32,
    /// Cone half angle around the emitter axis, in degrees.
    #[serde(default)]
    pub spread: f32,
    #[serde(default)]
    pub random_offset: [f32; 3],
    /// TilesX, TilesY, AnimFramerate.
    #[serde(default)]
    pub tiling: [f32; 3],
    #[serde(default)]
    pub diffuse_lighting: f32,
    #[serde(default)]
    pub emissive_lighting: f32,
    #[serde(default)]
    pub additive: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParticlePreview {
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub age: f32,
    pub lifetime: f32,
    pub size: f32,
    pub alpha: f32,
    pub color: [f32; 3],
    pub emitter_index: usize,
}

#[derive(Clone, Debug)]
pub struct PreviewEmitter {
    pub definition: ParticleEmitter,
    pub origin: [f32; 3],
    pub direction: [f32; 3],
    spawn_accumulator: f32,
    seeded: u64,
    initialized: bool,
}

#[derive(Default)]
pub struct ParticlePreviewSimulation {
    pub emitters: Vec<PreviewEmitter>,
    pub live: Vec<ParticlePreview>,
    pub elapsed: f32,
}

fn attr(tag: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    tag.attributes()
        .flatten()
        .find(|a| a.key.as_ref().eq_ignore_ascii_case(name))
        .and_then(|a| a.unescape_value().ok().map(|v| v.into_owned()))
}

fn source_scalar(raw: Option<String>, fallback: f32, max: f32) -> f32 {
    // CryEngine parameter envelopes begin with a scalar, then optionally
    // add ,Random= and ,ParticleAge= curves. Preserve the scalar baseline;
    // do not treat nested curve coordinates as numeric spawn settings.
    raw.and_then(|s| {
        s.split([',', ';'])
            .next()
            .and_then(|p| p.trim().parse::<f32>().ok())
    })
    .filter(|n| n.is_finite())
    .map(|n| n.clamp(-max, max))
    .unwrap_or(fallback)
}
fn source_color(raw: Option<String>) -> [f32; 3] {
    let Some(text) = raw else {
        return [1.0; 3];
    };
    let value = text.split(",ParticleAge=").next().unwrap_or(&text);
    let extract = |name: &str| -> Option<f32> {
        value.split([',', ')']).find_map(|piece| {
            let token = piece.trim().trim_start_matches('(');
            token
                .strip_prefix(name)?
                .parse::<f32>()
                .ok()
                .filter(|v| v.is_finite())
                .map(|v| v.clamp(0.0, 16.0))
        })
    };
    [
        extract("x=").unwrap_or(1.0),
        extract("y=").unwrap_or(1.0),
        extract("z=").unwrap_or(1.0),
    ]
}
fn defaults(library: &str, effect: String) -> ParticleEmitter {
    ParticleEmitter {
        library: library.into(),
        effect,
        texture: None,
        material: None,
        count: 0.0,
        lifetime: 1.0,
        speed: 0.0,
        size: 1.0,
        alpha: 1.0,
        color: [1.0; 3],
        continuous: false,
        enabled: true,
        gravity_scale: 0.0,
        size_birth: 1.0,
        spread: 0.0,
        random_offset: [0.0; 3],
        tiling: [1.0, 1.0, 0.0],
        diffuse_lighting: 1.0,
        emissive_lighting: 0.0,
        additive: false,
    }
}
/// The number after `name=` in a parameter's comma separated text.
fn named_value(text: &str, name: &str) -> Option<f32> {
    let key = format!("{name}=");
    text.split([',', '(', ';', ')'])
        .find_map(|piece| piece.trim().strip_prefix(key.as_str())?.parse::<f32>().ok())
        .filter(|value| value.is_finite())
}
fn source_parameters(tag: &BytesStart<'_>, emitter: &mut ParticleEmitter) {
    if let Some(value) = attr(tag, b"Enabled") {
        emitter.enabled = !value.eq_ignore_ascii_case("false") && value != "0";
    }
    if let Some(value) = attr(tag, b"Continuous") {
        emitter.continuous = value.eq_ignore_ascii_case("true") || value == "1";
    }
    emitter.count = source_scalar(attr(tag, b"Count"), emitter.count, 2048.0).max(0.0);
    emitter.lifetime =
        source_scalar(attr(tag, b"ParticleLifeTime"), emitter.lifetime, 3600.0).max(0.015);
    emitter.speed = source_scalar(attr(tag, b"Speed"), emitter.speed, 10000.0);
    emitter.size = source_scalar(attr(tag, b"Size"), emitter.size, 10000.0).max(0.0);
    emitter.alpha = source_scalar(attr(tag, b"Alpha"), emitter.alpha, 100.0).max(0.0);
    emitter.gravity_scale =
        source_scalar(attr(tag, b"GravityScale"), emitter.gravity_scale, 1000.0);
    emitter.color = source_color(attr(tag, b"Color"));
    emitter.texture = attr(tag, b"Texture").filter(|s| !s.is_empty());
    emitter.material = attr(tag, b"Material").filter(|s| !s.is_empty());
    if let Some(curve) = attr(tag, b"Size").as_deref().and_then(|size| {
        let (_, curve) = size.split_once("ParticleAge=")?;
        named_value(curve, "v")
    }) {
        emitter.size_birth = curve.clamp(0.0, 16.0);
    }
    emitter.spread = source_scalar(attr(tag, b"FocusAngle"), emitter.spread, 180.0).abs();
    if let Some(offset) = attr(tag, b"RandomOffset") {
        emitter.random_offset =
            ["x", "y", "z"].map(|axis| named_value(&offset, axis).unwrap_or(0.0).abs().min(1000.0));
    }
    if let Some(tiling) = attr(tag, b"TextureTiling") {
        emitter.tiling = [
            named_value(&tiling, "TilesX")
                .unwrap_or(1.0)
                .clamp(1.0, 64.0),
            named_value(&tiling, "TilesY")
                .unwrap_or(1.0)
                .clamp(1.0, 64.0),
            named_value(&tiling, "AnimFramerate")
                .unwrap_or(0.0)
                .clamp(0.0, 240.0),
        ];
    }
    if let Some(blend) = attr(tag, b"BlendType") {
        emitter.additive = blend.eq_ignore_ascii_case("Additive");
    }
    emitter.diffuse_lighting = source_scalar(
        attr(tag, b"DiffuseLighting"),
        emitter.diffuse_lighting,
        100.0,
    )
    .max(0.0);
    emitter.emissive_lighting = source_scalar(
        attr(tag, b"EmissiveLighting"),
        emitter.emissive_lighting,
        100.0,
    )
    .max(0.0);
}

/// Decode original CryXmlB (or safe plain XML) into source-authored emitter
/// definitions. The top-level particle name already contains original dots:
/// never drop those names or replace them with a guessed asset by basename.
pub fn decode_particle_library(bytes: &[u8]) -> Result<Vec<ParticleEmitter>, String> {
    let xml = crate::cryxml::xml_bytes(bytes, MAX_BYTES)
        .map_err(|e| format!("CryXmlB particle library: {e}"))?;
    let mut reader = quick_xml::Reader::from_reader(xml.as_ref());
    let mut library = None;
    let mut depth = 0usize;
    let mut stack: Vec<(usize, String)> = Vec::new();
    let mut emitters = Vec::new();
    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("Particle XML: {e}"))?;
        let start = matches!(&event, Event::Start(_));
        match event {
            Event::Start(tag) | Event::Empty(tag) => {
                if start {
                    depth += 1;
                }
                if depth > MAX_DEPTH {
                    return Err("Particle library depth limit exceeded".into());
                }
                let name = tag.name();
                if name.as_ref().eq_ignore_ascii_case(b"ParticleLibrary") {
                    library = attr(&tag, b"Name").filter(|s| !s.is_empty());
                } else if name.as_ref().eq_ignore_ascii_case(b"Particles") {
                    if emitters.len() >= MAX_EMITTERS {
                        return Err("Particle library emitter count limit exceeded".into());
                    }
                    let Some(label) = attr(&tag, b"Name") else {
                        return Err("Particle emitter missing source Name".into());
                    };
                    let prefix = stack.last().map(|(_, p)| p.as_str()).unwrap_or("");
                    let effect = if prefix.is_empty() {
                        label
                    } else {
                        format!("{prefix}.{label}")
                    };
                    emitters.push(defaults(library.as_deref().unwrap_or(""), effect.clone()));
                    if start {
                        stack.push((depth, effect));
                    }
                } else if name.as_ref().eq_ignore_ascii_case(b"Params")
                    && let Some((_, current)) = stack.last()
                    && let Some(emitter) = emitters.iter_mut().rev().find(|e| e.effect == *current)
                {
                    source_parameters(&tag, emitter);
                }
            }
            Event::End(tag) => {
                if tag.name().as_ref().eq_ignore_ascii_case(b"Particles")
                    && stack.last().is_some_and(|(opened, _)| *opened == depth)
                {
                    stack.pop();
                }
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            Event::DocType(_) | Event::GeneralRef(_) => {
                return Err("Unsafe external particle XML document".into());
            }
            _ => {}
        }
    }
    if library.is_none() {
        return Err("Missing ParticleLibrary Name".into());
    }
    for emitter in &mut emitters {
        emitter.library = library.clone().unwrap_or_default();
    }
    Ok(emitters)
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len < 0.00001 || !len.is_finite() {
        [0.0, 0.0, 1.0]
    } else {
        [v[0] / len, v[1] / len, v[2] / len]
    }
}
impl ParticlePreviewSimulation {
    pub fn add_emitter(
        &mut self,
        definition: ParticleEmitter,
        origin: [f32; 3],
        direction: [f32; 3],
    ) -> Result<(), String> {
        if self.emitters.len() >= 1024 {
            return Err("Preview emitter instance limit exceeded".into());
        }
        if !origin.iter().chain(direction.iter()).all(|x| x.is_finite()) {
            return Err("Nonfinite particle transform".into());
        }
        let mut seed = 0xcbf29ce484222325_u64;
        for byte in definition.effect.bytes() {
            seed = (seed ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }
        self.emitters.push(PreviewEmitter {
            definition,
            origin,
            direction: unit(direction),
            spawn_accumulator: 0.0,
            seeded: seed,
            initialized: false,
        });
        Ok(())
    }
    /// Fixed substeps prevent giant frame-time spikes from generating an
    /// unbounded particle cloud. Preview limits are independent of GPU budget.
    pub fn step(&mut self, delta_seconds: f32) {
        if !delta_seconds.is_finite() || delta_seconds <= 0.0 {
            return;
        }
        let count = (delta_seconds.min(0.25) / 0.016).ceil().max(1.0) as usize;
        let dt = delta_seconds.min(0.25) / count as f32;
        for _ in 0..count {
            self.step_once(dt);
        }
    }
    fn step_once(&mut self, dt: f32) {
        self.elapsed += dt;
        for p in &mut self.live {
            p.age += dt;
            p.velocity[2] -= 9.81 * self.emitters[p.emitter_index].definition.gravity_scale * dt;
            for index in 0..3 {
                p.position[index] += p.velocity[index] * dt;
            }
        }
        self.live.retain(|p| p.age < p.lifetime);
        for (index, emitter) in self.emitters.iter_mut().enumerate() {
            let d = &emitter.definition;
            if !d.enabled || d.count <= 0.0 || (d.texture.is_none() && d.material.is_none()) {
                continue;
            }
            // Count's exact native meaning varies with the authored
            // Continuous/PulsePeriod fields. This is an explicit preview
            // interpretation: burst count or count per lifetime, not parity.
            let to_spawn = if !emitter.initialized {
                emitter.initialized = true;
                if d.continuous {
                    0
                } else {
                    d.count.round() as usize
                }
            } else if !d.continuous {
                0
            } else {
                emitter.spawn_accumulator += dt * d.count / d.lifetime;
                let spawn = emitter.spawn_accumulator.floor() as usize;
                emitter.spawn_accumulator -= spawn as f32;
                spawn
            };
            for _ in 0..to_spawn.min(MAX_LIVE_PARTICLES.saturating_sub(self.live.len())) {
                // Stable jitter avoids identical stacked billboards. This
                // does not try to reproduce proprietary random distributions.
                emitter.seeded ^= emitter.seeded << 13;
                emitter.seeded ^= emitter.seeded >> 7;
                emitter.seeded ^= emitter.seeded << 17;
                let jitter = (emitter.seeded as u32 as f32 / u32::MAX as f32 - 0.5) * 0.02;
                let mut v = emitter.direction;
                v[0] += jitter;
                v = unit(v);
                self.live.push(ParticlePreview {
                    position: emitter.origin,
                    velocity: [v[0] * d.speed, v[1] * d.speed, v[2] * d.speed],
                    age: 0.0,
                    lifetime: d.lifetime,
                    size: d.size,
                    alpha: d.alpha,
                    color: d.color,
                    emitter_index: index,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cryengine_nested_effects_keep_authored_paths_and_source_values() {
        let xml = br#"<ParticleLibrary Name="map_ice_crashsite" ParticleVersion="24">
            <Particles Name="Rescue_Ship.RS_Lift_Jet"><Params Enabled="false"/>
              <Childs><Particles Name="Jet_Base"><Params Count="25" Continuous="true"
               ParticleLifeTime="0.4" Texture="textures/particle/smoke/smoke_cloud02_seq4.tif"
               Alpha="0.1,Random=0.5" Size="4,Random=0.15" Speed="-30"
               Color="(x=0.4,y=0.65,z=0.965)"/></Particles></Childs>
            </Particles></ParticleLibrary>"#;
        let emitters = decode_particle_library(xml).unwrap();
        assert_eq!(emitters.len(), 2);
        let jet = &emitters[1];
        assert_eq!(jet.effect, "Rescue_Ship.RS_Lift_Jet.Jet_Base");
        assert_eq!(jet.library, "map_ice_crashsite");
        assert_eq!(jet.count, 25.0);
        assert_eq!(jet.lifetime, 0.4);
        assert_eq!(jet.speed, -30.0);
        assert_eq!(jet.size, 4.0);
        assert_eq!(jet.color, [0.4, 0.65, 0.965]);
        assert!(jet.continuous && jet.enabled);
        assert!(!emitters[0].enabled);
    }
    #[test]
    fn deterministic_burst_and_bounded_step_expire_particles() {
        let emitter = defaults("test", "spark".into());
        let mut effect = ParticlePreviewSimulation::default();
        let mut d = emitter;
        d.count = 3.0;
        d.lifetime = 0.12;
        d.texture = Some("textures/particle/spark.tif".into());
        d.speed = 3.0;
        effect.add_emitter(d, [0.0; 3], [0.0, 0.0, 1.0]).unwrap();
        effect.step(0.016);
        assert_eq!(effect.live.len(), 3);
        effect.step(0.25);
        assert!(effect.live.is_empty());
        effect.step(0.016);
        assert!(effect.live.is_empty());
    }
    #[test]
    fn refuses_unsafe_documents_and_unbounded_counts() {
        assert!(decode_particle_library(b"<!DOCTYPE x><ParticleLibrary Name='x'/>").is_err());
        let xml = br#"<ParticleLibrary Name="x"><Particles Name="bad"><Params
         Count="999999999" Continuous="true" ParticleLifeTime="0.1"
         Texture="textures/x.tif"/></Particles></ParticleLibrary>"#;
        let parsed = decode_particle_library(xml).unwrap();
        assert_eq!(parsed[0].count, 2048.0);
    }
}
