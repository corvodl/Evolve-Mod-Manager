//! Blender-style transform gizmos for the Level Editor viewport: axis arrows
//! to move, rings to rotate and square-tipped handles to scale, each in
//! world X (red), Y (green) and Z (blue), plus a white handle for a free
//! move, view-axis rotation or uniform scale. The same shapes are drawn and
//! hit-tested, so what is seen is what can be grabbed.
use super::level_transform::Mode;
use super::textured::Primitive;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Handle {
    Axis(usize),
    Free,
}

/// A handle's outline (screen-pixel segments) and solid parts (triangles).
struct Shape {
    handle: Handle,
    lines: Vec<[[f32; 2]; 2]>,
    fills: Vec<[[f32; 2]; 3]>,
}

/// How far from a handle a click still grabs it, in pixels.
const GRAB: f32 = 9.0;

fn axis_color(handle: Handle) -> [f32; 4] {
    match handle {
        Handle::Axis(0) => [0.96, 0.24, 0.30, 1.0],
        Handle::Axis(1) => [0.48, 0.86, 0.18, 1.0],
        Handle::Axis(_) => [0.24, 0.55, 1.0, 1.0],
        Handle::Free => [0.92, 0.92, 0.92, 1.0],
    }
}

fn unit(axis: usize) -> [f32; 3] {
    let mut v = [0.0; 3];
    v[axis] = 1.0;
    v
}

fn offset(p: [f32; 3], direction: [f32; 3], amount: f32) -> [f32; 3] {
    std::array::from_fn(|i| p[i] + direction[i] * amount)
}

fn normalize2(v: [f32; 2]) -> Option<[f32; 2]> {
    let length = (v[0] * v[0] + v[1] * v[1]).sqrt();
    (length > 1e-3).then(|| [v[0] / length, v[1] / length])
}

/// A screen-space circle around `center`, as segments.
fn circle(center: [f32; 2], radius: f32, segments: usize) -> Vec<[[f32; 2]; 2]> {
    (0..segments)
        .map(|i| {
            let at = |k: usize| {
                let angle = k as f32 / segments as f32 * std::f32::consts::TAU;
                [
                    center[0] + radius * angle.cos(),
                    center[1] + radius * angle.sin(),
                ]
            };
            [at(i), at(i + 1)]
        })
        .collect()
}

/// A filled screen-space square centred on `center`.
fn square(center: [f32; 2], half: f32) -> Vec<[[f32; 2]; 3]> {
    let [x, y] = center;
    vec![
        [
            [x - half, y - half],
            [x + half, y - half],
            [x + half, y + half],
        ],
        [
            [x - half, y - half],
            [x + half, y + half],
            [x - half, y + half],
        ],
    ]
}

/// The gizmo for `mode` at `pivot`, its arms `size` world units long.
/// `project` maps a world point to screen pixels, or None behind the camera.
fn shapes(
    mode: Mode,
    pivot: [f32; 3],
    size: f32,
    project: &dyn Fn([f32; 3]) -> Option<[f32; 2]>,
) -> Vec<Shape> {
    let Some(center) = project(pivot) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match mode {
        Mode::Move | Mode::Scale => {
            for axis in 0..3 {
                let Some(tip) = project(offset(pivot, unit(axis), size)) else {
                    continue;
                };
                let Some(direction) = normalize2([tip[0] - center[0], tip[1] - center[1]]) else {
                    continue;
                };
                let side = [-direction[1], direction[0]];
                let fills = if mode == Mode::Move {
                    // An arrowhead past the tip.
                    vec![[
                        [tip[0] + direction[0] * 14.0, tip[1] + direction[1] * 14.0],
                        [tip[0] + side[0] * 6.0, tip[1] + side[1] * 6.0],
                        [tip[0] - side[0] * 6.0, tip[1] - side[1] * 6.0],
                    ]]
                } else {
                    square(
                        [tip[0] + direction[0] * 6.0, tip[1] + direction[1] * 6.0],
                        6.0,
                    )
                };
                out.push(Shape {
                    handle: Handle::Axis(axis),
                    lines: vec![[center, tip]],
                    fills,
                });
            }
            // A small ring at the centre: move freely, or scale uniformly.
            out.push(Shape {
                handle: Handle::Free,
                lines: circle(center, 9.0, 20),
                fills: Vec::new(),
            });
        }
        Mode::Rotate => {
            let mut widest: f32 = 0.0;
            for axis in 0..3 {
                let (u, v) = (unit((axis + 1) % 3), unit((axis + 2) % 3));
                let points = (0..=64)
                    .map(|k| {
                        let angle = k as f32 / 64.0 * std::f32::consts::TAU;
                        let p: [f32; 3] = std::array::from_fn(|i| {
                            pivot[i] + size * (angle.cos() * u[i] + angle.sin() * v[i])
                        });
                        project(p)
                    })
                    .collect::<Vec<_>>();
                let lines = points
                    .windows(2)
                    .filter_map(|pair| Some([pair[0]?, pair[1]?]))
                    .collect::<Vec<_>>();
                for [a, _] in &lines {
                    widest = widest
                        .max(((a[0] - center[0]).powi(2) + (a[1] - center[1]).powi(2)).sqrt());
                }
                out.push(Shape {
                    handle: Handle::Axis(axis),
                    lines,
                    fills: Vec::new(),
                });
            }
            // The outer ring turns about the view axis.
            if widest > 0.0 {
                out.push(Shape {
                    handle: Handle::Free,
                    lines: circle(center, widest * 1.18, 64),
                    fills: Vec::new(),
                });
            }
        }
    }
    out
}

fn segment_distance(p: [f32; 2], [a, b]: [[f32; 2]; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let length = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if length < 1e-6 {
        0.0
    } else {
        (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / length).clamp(0.0, 1.0)
    };
    let closest = [a[0] + ab[0] * t, a[1] + ab[1] * t];
    ((p[0] - closest[0]).powi(2) + (p[1] - closest[1]).powi(2)).sqrt()
}

fn inside_triangle(p: [f32; 2], [a, b, c]: [[f32; 2]; 3]) -> bool {
    let side =
        |u: [f32; 2], v: [f32; 2]| (v[0] - u[0]) * (p[1] - u[1]) - (v[1] - u[1]) * (p[0] - u[0]);
    let (d1, d2, d3) = (side(a, b), side(b, c), side(c, a));
    let negative = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let positive = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(negative && positive)
}

/// The handle under the cursor (screen pixels), if any.
pub(super) fn hit(
    mode: Mode,
    pivot: [f32; 3],
    size: f32,
    project: &dyn Fn([f32; 3]) -> Option<[f32; 2]>,
    cursor: [f32; 2],
) -> Option<Handle> {
    let mut best: Option<(f32, Handle)> = None;
    for shape in shapes(mode, pivot, size, project) {
        let mut distance = shape
            .lines
            .iter()
            .map(|segment| segment_distance(cursor, *segment))
            .fold(f32::INFINITY, f32::min);
        if shape.fills.iter().any(|t| inside_triangle(cursor, *t)) {
            distance = 0.0;
        }
        // The centre handle of move and scale is a disc, not just its ring.
        if shape.handle == Handle::Free
            && mode != Mode::Rotate
            && let Some(center) = project(pivot)
            && ((cursor[0] - center[0]).powi(2) + (cursor[1] - center[1]).powi(2)).sqrt() <= 9.0
        {
            distance = 0.0;
        }
        if distance <= GRAB && best.is_none_or(|(d, _)| distance < d) {
            best = Some((distance, shape.handle));
        }
    }
    best.map(|(_, handle)| handle)
}

/// Draw the gizmo into the viewport overlay. `focus` is brightened (the
/// hovered handle, or the one being dragged); with `others_dim`, the rest
/// fade, as while a transform is in progress.
#[allow(clippy::too_many_arguments)] // The gizmo, its view and its highlight.
pub(super) fn draw(
    primitive: &mut Primitive,
    mode: Mode,
    pivot: [f32; 3],
    size: f32,
    project: &dyn Fn([f32; 3]) -> Option<[f32; 2]>,
    width: f32,
    height: f32,
    focus: Option<Handle>,
    others_dim: bool,
) {
    let ndc = |p: [f32; 2]| [p[0] / width * 2.0 - 1.0, 1.0 - p[1] / height * 2.0];
    for shape in shapes(mode, pivot, size, project) {
        let focused = focus == Some(shape.handle);
        let mut color = axis_color(shape.handle);
        if focused {
            color = [
                color[0] * 0.5 + 0.5,
                color[1] * 0.5 + 0.5,
                color[2] * 0.5 + 0.5,
                1.0,
            ];
        } else if others_dim {
            color[3] = 0.3;
        }
        let thickness = if focused { 4.0 } else { 2.5 };
        for [a, b] in &shape.lines {
            let Some(direction) = normalize2([b[0] - a[0], b[1] - a[1]]) else {
                continue;
            };
            let side = [
                -direction[1] * thickness * 0.5,
                direction[0] * thickness * 0.5,
            ];
            let corners = [
                [a[0] + side[0], a[1] + side[1]],
                [b[0] + side[0], b[1] + side[1]],
                [b[0] - side[0], b[1] - side[1]],
                [a[0] - side[0], a[1] - side[1]],
            ]
            .map(ndc);
            primitive.add_overlay_triangle([corners[0], corners[1], corners[2]], color);
            primitive.add_overlay_triangle([corners[0], corners[2], corners[3]], color);
        }
        for triangle in &shape.fills {
            primitive.add_overlay_triangle(triangle.map(ndc), color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An orthographic view looking down -Y: X right, Z up, 10 px per unit,
    /// centred at (200, 200).
    fn project(p: [f32; 3]) -> Option<[f32; 2]> {
        Some([200.0 + p[0] * 10.0, 200.0 - p[2] * 10.0])
    }

    #[test]
    fn handles_are_grabbed_where_they_are_drawn() {
        let pivot = [0.0; 3];
        // The X arrow runs right from the centre; Z runs up.
        assert_eq!(
            hit(Mode::Move, pivot, 10.0, &project, [260.0, 202.0]),
            Some(Handle::Axis(0))
        );
        assert_eq!(
            hit(Mode::Move, pivot, 10.0, &project, [201.0, 150.0]),
            Some(Handle::Axis(2))
        );
        assert_eq!(
            hit(Mode::Move, pivot, 10.0, &project, [203.0, 198.0]),
            Some(Handle::Free)
        );
        assert_eq!(hit(Mode::Move, pivot, 10.0, &project, [260.0, 260.0]), None);
        // The scale square at the tip of X.
        assert_eq!(
            hit(Mode::Scale, pivot, 10.0, &project, [306.0, 203.0]),
            Some(Handle::Axis(0))
        );
        // The Y ring (in the XZ plane, a circle here) and the outer view ring.
        assert_eq!(
            hit(Mode::Rotate, pivot, 10.0, &project, [270.7, 129.3]),
            Some(Handle::Axis(1))
        );
        assert_eq!(
            hit(Mode::Rotate, pivot, 10.0, &project, [200.0, 82.0]),
            Some(Handle::Free)
        );
    }
}
