//! Shared ground-plane geometry for the textured wgpu viewport and the
//! historical CryTek wireframe fallback. It is purely decorative: it never
//! changes CryEngine mesh vertices, UVs, materials or rigging.
#[derive(Clone, Copy, Debug)]
pub struct GridLine {
    pub start: [f32; 3],
    pub end: [f32; 3],
    pub color: [f32; 4],
}

/// A finite, fixed-cost XY plane just below the lowest authored Z vertex.
/// Legacy 2012 CryTek and Stage 2 CrChF meshes use this same geometry-only
/// overlay, so no format conversion or game-specific metadata is required.
pub fn floor_lines(vertices: &[[f32; 3]]) -> Vec<GridLine> {
    if vertices.is_empty() {
        return Vec::new();
    }
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for point in vertices {
        if point.iter().any(|coordinate| !coordinate.is_finite()) {
            return Vec::new();
        }
        for axis in 0..3 {
            lo[axis] = lo[axis].min(point[axis]);
            hi[axis] = hi[axis].max(point[axis]);
        }
    }
    let extent = (0..3).map(|axis| hi[axis] - lo[axis]).fold(1e-6f32, f32::max);
    if !extent.is_finite() || extent <= 1e-6 {
        return Vec::new();
    }
    let middle = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5];
    let height = lo[2] - extent * 0.0125;
    let span = extent * 1.15;
    let step = span / 20.0;
    let mut result = Vec::with_capacity(82);
    for i in -20i32..=20 {
        let position = i as f32 * step;
        let normal = i == 0;
        let major = i % 5 == 0;
        let gray = if major { [0.52, 0.56, 0.61, 0.37] } else { [0.42, 0.46, 0.51, 0.20] };
        // X=red, Y=green just like the Blender grid. The axis lines
        // intersect under the model rather than necessarily at world (0,0).
        result.push(GridLine {
            start: [middle[0] - span, middle[1] + position, height],
            end: [middle[0] + span, middle[1] + position, height],
            color: if normal { [0.79, 0.30, 0.34, 0.68] } else { gray },
        });
        result.push(GridLine {
            start: [middle[0] + position, middle[1] - span, height],
            end: [middle[0] + position, middle[1] + span, height],
            color: if normal { [0.35, 0.73, 0.46, 0.68] } else { gray },
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_is_below_lowest_foot_and_does_not_modify_source_mesh() {
        let original = vec![[100., -25., -3.], [106., -20., 10.], [103., -22., 4.]];
        let saved = original.clone();
        let grid = floor_lines(&original);
        assert_eq!(grid.len(), 82);
        assert!(grid.iter().all(|line| line.start[2] < -3. && line.end[2] < -3.));
        assert!(grid.iter().all(|line| line.start.iter().chain(line.end.iter()).all(|n| n.is_finite())));
        assert_eq!(original, saved);
        assert_eq!(grid[40].color, [0.79, 0.30, 0.34, 0.68]);
        assert_eq!(grid[41].color, [0.35, 0.73, 0.46, 0.68]);
    }

    #[test]
    fn invalid_geometry_has_no_dangerous_infinite_grid() {
        assert!(floor_lines(&[]).is_empty());
        assert!(floor_lines(&[[f32::NAN, 0., 0.]]).is_empty());
        assert!(floor_lines(&[[0., 0., 0.], [0., 0., 0.]]).is_empty());
    }
}
