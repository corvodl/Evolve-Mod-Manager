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

    /// Use the map's recorded sun at its authored hour. Maps with no authored
    /// sky ambient variable get a neutral level tinted by the sky light color.
    pub fn with_authored_sun(
        mut self,
        sun: Option<super::level_atmosphere::AuthoredSun>,
        hour: f32,
        source: &super::level_environment::TimeOfDay,
    ) -> Self {
        if let Some(sun) = sun {
            let [x, y, z] = sun.to_sun;
            // Turn about the horizontal axis across the sun's heading.
            let flat = (x * x + y * y).sqrt().max(0.001);
            let axis = [-y / flat, x / flat, 0.0];
            let (sin, cos) = ((hour - sun.hour) * 15.0).to_radians().sin_cos();
            let along = axis[0] * x + axis[1] * y;
            let cross = [axis[1] * z, -axis[0] * z, axis[0] * y - axis[1] * x];
            self.sun[0] = x * cos + cross[0] * sin + axis[0] * along * (1.0 - cos);
            self.sun[1] = y * cos + cross[1] * sin + axis[1] * along * (1.0 - cos);
            self.sun[2] = z * cos + cross[2] * sin;
            // The mission SunVector specifies direction, not brightness.
            // Keep the light strength sampled from the map's TimeOfDay
            // "Sun intensity" curve by from_original; a made-up elevation
            // multiplier here discarded that authored intensity entirely.
        }
        if source.color("Sky color", hour).is_none()
            && source.color("Sky light color", hour).is_none()
        {
            let tint = source
                .color("Sky light: Sun intensity", hour)
                .unwrap_or([1.0; 3]);
            for (channel, tinted) in self.ambient[..3].iter_mut().zip(tint) {
                *channel = 0.11 + 0.09 * tinted.clamp(0.0, 1.0);
            }
        }
        self
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
    fn mission_sun_direction_does_not_override_original_sun_intensity() {
        // The source TimeOfDay curve is authoritative for intensity. The
        // mission's SunVector is used only for direction; neither the level's
        // authored hour nor a scrub to another hour may replace brightness
        // with a fixed elevation heuristic.
        let tod = super::super::level_environment::TimeOfDay::parse(
            br#"<TimeOfDay Time="12">
                <Variable Name="Sun intensity" Value="120000">
                    <Spline Keys="0:0:36,0.5:120000:36,1:0:36,"/>
                </Variable>
            </TimeOfDay>"#,
        )
        .unwrap();
        let source_sun = super::super::level_atmosphere::AuthoredSun {
            to_sun: [0.0, 0.6, 0.8],
            hour: 12.0,
        };
        for hour in [9.0, 12.0, 15.0] {
            let authored = Lighting::from_original(hour, &tod);
            let positioned = authored.with_authored_sun(Some(source_sun), hour, &tod);
            assert_eq!(
                positioned.sun[3], authored.sun[3],
                "SunVector must not overwrite authored intensity at hour {hour}"
            );
        }
        let midnight = Lighting::from_original(0.0, &tod)
            .with_authored_sun(Some(source_sun), 0.0, &tod);
        assert_eq!(midnight.sun[3], 0.0);
        let noon = Lighting::from_original(12.0, &tod)
            .with_authored_sun(Some(source_sun), 12.0, &tod);
        assert!(noon.sun[3] > midnight.sun[3]);
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
