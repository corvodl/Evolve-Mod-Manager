//! Approximate time-of-day lighting for read-only map inspection.
//! Original CryEngine TimeOfDay.xml curves and light entities are not yet
//! decoded here. These controls are a predictable preview, never an export.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Lighting {
    pub sun: [f32; 4],       // xyz = direction toward the light, w = strength
    pub ambient: [f32; 4],   // rgb = ambient light, a = unused
    pub sun_color: [f32; 4], // rgb = directional color
}

impl Lighting {
    pub fn at(hour: f32) -> Self {
        let hour = if hour.is_finite() {
            hour.clamp(0.0, 24.0)
        } else {
            12.0
        };
        let cycle = hour / 24.0 * std::f32::consts::TAU;
        let elevation = ((hour - 6.0) / 12.0 * std::f32::consts::PI).sin();
        // Scattered twilight remains visible when the sun is at the horizon,
        // so Dawn and Dusk previews are not identical to midnight.
        let lit = ((elevation + 0.10) / 1.10).clamp(0.0, 1.0).powf(0.65);
        let horizon = (1.0 - elevation.abs()).clamp(0.0, 1.0).powf(5.0) * lit.sqrt();
        let horizontal = (1.0 - elevation * elevation).max(0.0).sqrt();
        let dir = [
            cycle.cos() * horizontal,
            cycle.sin() * horizontal,
            elevation,
        ];
        let length = (dir.iter().map(|x| x * x).sum::<f32>()).sqrt().max(0.001);
        let night = [0.055, 0.08, 0.14];
        let day = [0.34, 0.36, 0.40];
        let ambient: [f32; 3] = std::array::from_fn(|i| night[i] * (1.0 - lit) + day[i] * lit);
        Self {
            sun: [
                dir[0] / length,
                dir[1] / length,
                dir[2] / length,
                lit * 1.25,
            ],
            ambient: [ambient[0], ambient[1], ambient[2], 0.0],
            sun_color: [
                1.0,
                (0.97 - horizon * 0.27).max(0.0),
                (0.92 - horizon * 0.51).max(0.0),
                0.0,
            ],
        }
    }

    /// Original authored light properties mapped onto the existing wgpu
    /// preview. Tangents use an approximate SDK-backed curve; original sun
    /// position and HDR are not simulated.
    pub fn from_original(hour: f32, source: &super::level_environment::TimeOfDay) -> Self {
        let mut lit = Self::at(hour);
        if let Some(rgb) = source.color("Sun color", hour) {
            lit.sun_color = [rgb[0].max(0.0), rgb[1].max(0.0), rgb[2].max(0.0), 0.0];
        }
        if let Some(intensity) = source.scalar("Sun intensity", hour) {
            let scaled = if intensity > 1000.0 {
                intensity / 100_000.0
            } else {
                intensity
            };
            lit.sun[3] = scaled.clamp(0.0, 4.0);
        }
        if let Some(multiplier) = source.scalar("Sun color multiplier", hour) {
            for channel in &mut lit.sun_color[..3] {
                *channel = (*channel * multiplier.clamp(0.0, 10.0)).min(8.0);
            }
        }
        for name in ["Sky light color", "Sky color"] {
            if let Some(rgb) = source.color(name, hour) {
                lit.ambient[..3].copy_from_slice(&rgb.map(|v| v.clamp(0.0, 2.0)));
                break;
            }
        }
        lit
    }

    pub fn label(hour: f32) -> String {
        let minutes = ((hour.clamp(0.0, 24.0) * 60.0).round() as u32) % 1440;
        format!("{:02}:{:02}", minutes / 60, minutes % 60)
    }

    pub fn uniforms(self) -> [[f32; 4]; 3] {
        [self.sun, self.ambient, self.sun_color]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn daylight_noon_is_stronger_than_night() {
        let noon = Lighting::at(12.0);
        let night = Lighting::at(0.0);
        assert!(noon.sun[3] > 1.0);
        assert_eq!(night.sun[3], 0.0);
        assert!(Lighting::at(6.0).sun[3] > night.sun[3]);
        assert!(Lighting::at(18.0).sun[3] > night.sun[3]);
        assert!(noon.ambient[0] > night.ambient[0]);
        assert!(Lighting::at(6.5).sun_color[2] < noon.sun_color[2]);
    }
    #[test]
    fn time_scrubbing_is_bounded_and_deterministic() {
        assert_eq!(Lighting::at(-10.0), Lighting::at(0.0));
        assert_eq!(Lighting::at(200.0), Lighting::at(24.0));
        assert_eq!(Lighting::at(f32::NAN), Lighting::at(12.0));
        assert_eq!(Lighting::label(13.5), "13:30");
        assert!(
            Lighting::at(15.5)
                .uniforms()
                .iter()
                .flatten()
                .all(|x| x.is_finite())
        );
    }
}
