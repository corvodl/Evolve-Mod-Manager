//! Offline prototype for a native kit-video compositor.
//!
//! This DLL currently exports an ABI marker and a testable compositing rule.
//! It does NOT hook D3D11, discover GFx video textures, read game memory or
//! draw into Evolve. Loading it proves only the guarded second-DLL path.

/// Bump if the launcher/native integration contract changes.
#[allow(unsafe_code)] // A stable DLL export needs an unmangled symbol; no unsafe operations.
#[unsafe(no_mangle)]
pub extern "system" fn EvolveKitCompositorApiVersion() -> u32 {
    1
}

/// Smooth left-to-right coverage, with no quantized or dithered mask.
pub fn video_coverage(x: f32, start: f32, end: f32) -> f32 {
    if !x.is_finite() || !start.is_finite() || !end.is_finite() || end <= start {
        return 0.0;
    }
    let t = ((x - start) / (end - start)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Reference blend for *linear-light* RGB samples, before UI text is drawn.
/// Actual native integration must supply both textures from the same frame.
pub fn composite(scene: [f32; 3], video: [f32; 3], coverage: f32) -> [f32; 3] {
    let weight = if coverage.is_finite() {
        coverage.clamp(0.0, 1.0)
    } else {
        0.0
    };
    std::array::from_fn(|i| scene[i] * (1.0 - weight) + video[i] * weight)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feather_is_continuous_and_clamped() {
        assert_eq!(video_coverage(-1.0, 0.0, 1.0), 0.0);
        assert_eq!(video_coverage(0.0, 0.0, 1.0), 0.0);
        assert_eq!(video_coverage(0.5, 0.0, 1.0), 0.5);
        assert_eq!(video_coverage(1.0, 0.0, 1.0), 1.0);
        assert_eq!(video_coverage(2.0, 0.0, 1.0), 1.0);
        assert_eq!(video_coverage(0.5, 1.0, 0.0), 0.0);
        assert_eq!(video_coverage(f32::NAN, 0.0, 1.0), 0.0);
        let mut previous = 0.0;
        for i in 0..=1000 {
            let current = video_coverage(i as f32 / 1000.0, 0.0, 1.0);
            assert!(current >= previous);
            previous = current;
        }
    }

    #[test]
    fn blend_preserves_both_endpoints_and_live_scene() {
        let scene = [0.2, 0.3, 0.4];
        let video = [0.8, 0.7, 0.6];
        assert_eq!(composite(scene, video, 0.0), scene);
        assert_eq!(composite(scene, video, 1.0), video);
        assert_eq!(composite(scene, video, 0.5), [0.5, 0.5, 0.5]);
        assert_eq!(composite(scene, video, f32::NAN), scene);
    }
}
