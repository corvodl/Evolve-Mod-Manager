//! Blender-style modal Move (G), Rotate (R) and Scale (S) of the selected
//! placements. The mouse drives the change live; X, Y or Z constrains it to
//! that world axis (again to release); typed digits give an exact amount;
//! LMB or Enter applies and RMB or Escape cancels. Pure math: the editor
//! feeds it pointer rays and draws its results.
use evolve_core::Transform;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Move,
    Rotate,
    Scale,
}

/// One placement being transformed.
#[derive(Clone, Debug)]
pub(super) struct Target {
    pub index: usize,
    /// World transform when the operation started.
    pub world: Transform,
    /// The parent's world transform, for placements nested in another.
    pub parent: Option<Transform>,
}

type Ray = ([f32; 3], [f32; 3]);

#[derive(Clone, Debug)]
pub(super) struct ModalTransform {
    pub mode: Mode,
    pub axis: Option<usize>,
    pub typed: String,
    pub targets: Vec<Target>,
    pub pivot: [f32; 3],
    /// The camera's view direction when the operation started.
    forward: [f32; 3],
    start: Option<Ray>,
    current: Option<Ray>,
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    a.map(|v| v * s)
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}
fn unit(axis: usize) -> [f32; 3] {
    let mut v = [0.0; 3];
    v[axis] = 1.0;
    v
}
/// w,x,y,z quaternion product, as the scene composes rotations.
fn mult(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [w, x, y, z] = a;
    let [p, q, r, s] = b;
    [
        w * p - x * q - y * r - z * s,
        w * q + x * p + y * s - z * r,
        w * r - x * s + y * p + z * q,
        w * s + x * r - y * q + z * p,
    ]
}
fn conjugate(q: [f32; 4]) -> [f32; 4] {
    [q[0], -q[1], -q[2], -q[3]]
}
fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let applied = mult(mult(q, [0.0, v[0], v[1], v[2]]), conjugate(q));
    [applied[1], applied[2], applied[3]]
}
fn normalized(q: [f32; 4]) -> [f32; 4] {
    let norm = q.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm < 1e-8 {
        [1.0, 0.0, 0.0, 0.0]
    } else {
        q.map(|v| v / norm)
    }
}
fn axis_angle(axis: [f32; 3], radians: f32) -> [f32; 4] {
    let axis = scale(axis, 1.0 / length(axis).max(1e-8));
    let (s, c) = (radians * 0.5).sin_cos();
    [c, axis[0] * s, axis[1] * s, axis[2] * s]
}

/// Where a ray meets the plane through `point` with normal `normal`.
fn plane_hit(ray: Ray, point: [f32; 3], normal: [f32; 3]) -> Option<[f32; 3]> {
    let (origin, direction) = ray;
    let facing = dot(direction, normal);
    if facing.abs() < 1e-4 {
        return None;
    }
    let t = dot(sub(point, origin), normal) / facing;
    (t > 0.0 && t.is_finite()).then(|| add(origin, scale(direction, t)))
}

/// How far along the line `point + s·axis` the ray passes closest.
fn closest_on_axis(ray: Ray, point: [f32; 3], axis: [f32; 3]) -> Option<f32> {
    let (origin, direction) = ray;
    let b = dot(axis, direction);
    let denominator = 1.0 - b * b;
    if denominator < 1e-5 {
        return None;
    }
    let w = sub(point, origin);
    let s = (b * dot(direction, w) - dot(axis, w)) / denominator;
    s.is_finite().then_some(s)
}

/// A world transform as the local transform of a placement under `parent`:
/// the inverse of the scene's parent-then-child composition.
pub(super) fn local_under(parent: Option<&Transform>, world: Transform) -> Transform {
    let Some(parent) = parent else {
        return world;
    };
    let inverse = conjugate(normalized(parent.quaternion));
    let offset = rotate(inverse, sub(world.position, parent.position));
    let divide = |a: f32, b: f32| if b.abs() > 1e-8 { a / b } else { a };
    Transform {
        position: std::array::from_fn(|i| divide(offset[i], parent.scale[i])),
        quaternion: normalized(mult(inverse, world.quaternion)),
        scale: std::array::from_fn(|i| divide(world.scale[i], parent.scale[i])),
    }
}

impl ModalTransform {
    pub fn new(mode: Mode, targets: Vec<Target>, forward: [f32; 3], ray: Option<Ray>) -> Self {
        let count = targets.len().max(1) as f32;
        let pivot = targets
            .iter()
            .fold([0.0; 3], |sum, target| add(sum, target.world.position))
            .map(|v| v / count);
        Self {
            mode,
            axis: None,
            typed: String::new(),
            targets,
            pivot,
            forward: scale(forward, 1.0 / length(forward).max(1e-8)),
            start: ray,
            current: ray,
        }
    }

    /// X, Y or Z: constrain to that world axis; the same key again releases it.
    pub fn toggle_axis(&mut self, axis: usize) {
        self.axis = (self.axis != Some(axis)).then_some(axis);
    }

    /// A typed digit, '.', or '-' (which flips the sign). False for other keys.
    pub fn type_char(&mut self, key: char) -> bool {
        match key {
            '-' => {
                if let Some(rest) = self.typed.strip_prefix('-') {
                    self.typed = rest.to_owned();
                } else {
                    self.typed.insert(0, '-');
                }
                true
            }
            '.' if !self.typed.contains('.') => {
                self.typed.push('.');
                true
            }
            '0'..='9' if self.typed.len() < 16 => {
                self.typed.push(key);
                true
            }
            _ => false,
        }
    }

    pub fn backspace(&mut self) {
        self.typed.pop();
    }

    pub fn pointer(&mut self, ray: Option<Ray>) {
        if ray.is_some() && self.start.is_none() {
            self.start = ray;
        }
        if ray.is_some() {
            self.current = ray;
        }
    }

    fn typed_value(&self) -> Option<f32> {
        self.typed
            .parse::<f32>()
            .ok()
            .filter(|value| value.is_finite())
    }

    /// The mouse's move since the start, in world metres.
    fn dragged_offset(&self) -> [f32; 3] {
        let (Some(start), Some(current)) = (self.start, self.current) else {
            return [0.0; 3];
        };
        match self.axis {
            Some(axis) => {
                // As Blender does: where the pointer meets the plane holding
                // the axis that faces the camera most, projected onto the axis.
                let along = unit(axis);
                let facing = sub(self.forward, scale(along, dot(self.forward, along)));
                let distance = |ray: Ray| {
                    if length(facing) > 1e-3 {
                        plane_hit(ray, self.pivot, facing)
                            .map(|hit| dot(sub(hit, self.pivot), along))
                    } else {
                        // Looking straight down the axis.
                        closest_on_axis(ray, self.pivot, along)
                    }
                };
                match (distance(start), distance(current)) {
                    (Some(a), Some(b)) => scale(along, b - a),
                    _ => [0.0; 3],
                }
            }
            None => match (
                plane_hit(start, self.pivot, self.forward),
                plane_hit(current, self.pivot, self.forward),
            ) {
                (Some(a), Some(b)) => sub(b, a),
                _ => [0.0; 3],
            },
        }
    }

    /// The pointer's turn about the pivot, counter-clockwise on screen.
    fn dragged_angle(&self) -> f32 {
        let (Some(start), Some(current)) = (self.start, self.current) else {
            return 0.0;
        };
        let (Some(a), Some(b)) = (
            plane_hit(start, self.pivot, self.forward),
            plane_hit(current, self.pivot, self.forward),
        ) else {
            return 0.0;
        };
        let (a, b) = (sub(a, self.pivot), sub(b, self.pivot));
        if length(a) < 1e-4 || length(b) < 1e-4 {
            return 0.0;
        }
        dot(scale(self.forward, -1.0), cross(a, b)).atan2(dot(a, b))
    }

    /// The pointer's distance from the pivot, relative to where it started.
    fn dragged_factor(&self) -> f32 {
        let (Some(start), Some(current)) = (self.start, self.current) else {
            return 1.0;
        };
        match (
            plane_hit(start, self.pivot, self.forward),
            plane_hit(current, self.pivot, self.forward),
        ) {
            (Some(a), Some(b)) => {
                let from = length(sub(a, self.pivot));
                if from < 1e-3 {
                    1.0
                } else {
                    length(sub(b, self.pivot)) / from
                }
            }
            _ => 1.0,
        }
    }

    /// The rotation axis and angle in radians.
    fn rotation(&self) -> ([f32; 3], f32) {
        let toward_viewer = scale(self.forward, -1.0);
        let axis = self.axis.map_or(toward_viewer, unit);
        let angle = match self.typed_value() {
            Some(degrees) => degrees.to_radians(),
            None => {
                // A screen turn is a turn about an axis facing the viewer;
                // one facing away turns the other way.
                let facing = dot(axis, toward_viewer);
                self.dragged_angle() * if facing < 0.0 { -1.0 } else { 1.0 }
            }
        };
        (axis, angle)
    }

    /// Each target's world transform now.
    pub fn results(&self) -> Vec<Transform> {
        match self.mode {
            Mode::Move => {
                let offset = match self.typed_value() {
                    Some(value) => scale(unit(self.axis.unwrap_or(0)), value),
                    None => self.dragged_offset(),
                };
                self.targets
                    .iter()
                    .map(|target| Transform {
                        position: add(target.world.position, offset),
                        ..target.world
                    })
                    .collect()
            }
            Mode::Rotate => {
                let (axis, angle) = self.rotation();
                let turn = axis_angle(axis, angle);
                self.targets
                    .iter()
                    .map(|target| Transform {
                        position: add(
                            self.pivot,
                            rotate(turn, sub(target.world.position, self.pivot)),
                        ),
                        quaternion: normalized(mult(turn, target.world.quaternion)),
                        scale: target.world.scale,
                    })
                    .collect()
            }
            Mode::Scale => {
                let factor = self
                    .typed_value()
                    .unwrap_or_else(|| self.dragged_factor())
                    .clamp(-1000.0, 1000.0);
                let factor = if factor.abs() < 1e-3 { 1e-3 } else { factor };
                self.targets
                    .iter()
                    .map(|target| {
                        let world = target.world;
                        let mut size = world.scale;
                        let mut position = sub(world.position, self.pivot);
                        match self.axis {
                            Some(axis) => {
                                // Placements scale along their own axes: the
                                // one closest to the chosen world axis.
                                let along = (0..3)
                                    .max_by(|&a, &b| {
                                        let lean = |k: usize| {
                                            rotate(world.quaternion, unit(k))[axis].abs()
                                        };
                                        lean(a).total_cmp(&lean(b))
                                    })
                                    .unwrap_or(axis);
                                size[along] *= factor;
                                position[axis] *= factor;
                            }
                            None => {
                                size = size.map(|v| v * factor);
                                position = scale(position, factor);
                            }
                        }
                        Transform {
                            position: add(self.pivot, position),
                            quaternion: world.quaternion,
                            scale: size,
                        }
                    })
                    .collect()
            }
        }
    }

    /// Each target's new local transform, as its layer stores it.
    pub fn local_results(&self) -> Vec<Transform> {
        self.targets
            .iter()
            .zip(self.results())
            .map(|(target, world)| local_under(target.parent.as_ref(), world))
            .collect()
    }

    /// The status line: the operation, its axis and the current amount.
    pub fn describe(&self) -> String {
        let axis = self.axis.map_or(String::new(), |axis| {
            format!(" along {}", ["X", "Y", "Z"][axis])
        });
        let typed = if self.typed.is_empty() {
            String::new()
        } else {
            format!(" [{}]", self.typed)
        };
        let amount = match self.mode {
            Mode::Move => {
                let first = self.results();
                let moved = first
                    .first()
                    .zip(self.targets.first())
                    .map_or([0.0; 3], |(now, target)| {
                        sub(now.position, target.world.position)
                    });
                format!(
                    "Move{axis}: {:.2}, {:.2}, {:.2} m",
                    moved[0], moved[1], moved[2]
                )
            }
            Mode::Rotate => format!("Rotate{axis}: {:.1}°", self.rotation().1.to_degrees()),
            Mode::Scale => format!(
                "Scale{axis}: {:.3}",
                self.typed_value().unwrap_or_else(|| self.dragged_factor())
            ),
        };
        format!("{amount}{typed} · X/Y/Z axis · type a value · LMB/Enter apply · RMB/Esc cancel")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(position: [f32; 3]) -> Target {
        Target {
            index: 0,
            world: Transform {
                position,
                ..Transform::default()
            },
            parent: None,
        }
    }

    /// A camera at (0, -10, 0) looking along +Y; rays toward points on the
    /// plane y = 0.
    fn ray_to(point: [f32; 3]) -> Ray {
        let origin = [0.0, -10.0, 0.0];
        let direction = sub(point, origin);
        (origin, scale(direction, 1.0 / length(direction)))
    }

    #[test]
    fn move_follows_the_pointer_and_its_axis() {
        let mut op = ModalTransform::new(
            Mode::Move,
            vec![target([0.0; 3])],
            [0.0, 1.0, 0.0],
            Some(ray_to([0.0; 3])),
        );
        op.pointer(Some(ray_to([2.0, 0.0, 1.0])));
        let moved = op.results()[0].position;
        assert!((moved[0] - 2.0).abs() < 1e-4 && (moved[2] - 1.0).abs() < 1e-4);
        op.toggle_axis(0);
        let along_x = op.results()[0].position;
        assert!((along_x[0] - 2.0).abs() < 1e-3 && along_x[2].abs() < 1e-4);
        // A typed value is exact.
        for key in "-3.5".chars() {
            assert!(op.type_char(key));
        }
        assert_eq!(op.results()[0].position, [-3.5, 0.0, 0.0]);
        op.toggle_axis(0);
        assert_eq!(op.axis, None);
    }

    #[test]
    fn rotate_turns_about_the_pivot() {
        let mut op = ModalTransform::new(
            Mode::Rotate,
            vec![target([1.0, 0.0, 0.0]), target([-1.0, 0.0, 0.0])],
            [0.0, 1.0, 0.0],
            Some(ray_to([1.0, 0.0, 0.0])),
        );
        assert_eq!(op.pivot, [0.0; 3]);
        op.toggle_axis(2);
        for key in "90".chars() {
            op.type_char(key);
        }
        let turned = op.results();
        assert!(
            (turned[0].position[1] - 1.0).abs() < 1e-5,
            "{:?}",
            turned[0]
        );
        assert!((turned[1].position[1] + 1.0).abs() < 1e-5);
        // A quarter turn about Z.
        assert!((turned[0].quaternion[3] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);
    }

    #[test]
    fn scale_grows_with_the_pointer_distance() {
        let mut op = ModalTransform::new(
            Mode::Scale,
            vec![target([0.0; 3])],
            [0.0, 1.0, 0.0],
            Some(ray_to([1.0, 0.0, 0.0])),
        );
        op.pointer(Some(ray_to([2.0, 0.0, 0.0])));
        let grown = op.results()[0].scale;
        assert!(grown.iter().all(|s| (s - 2.0).abs() < 1e-3), "{grown:?}");
        op.toggle_axis(2);
        assert_eq!(op.results()[0].scale[0], 1.0);
        assert!((op.results()[0].scale[2] - 2.0).abs() < 1e-3);
    }

    #[test]
    fn local_transform_undoes_the_parent() {
        let parent = Transform {
            position: [10.0, 0.0, 0.0],
            quaternion: axis_angle([0.0, 0.0, 1.0], std::f32::consts::FRAC_PI_2),
            scale: [2.0; 3],
        };
        let world = Transform {
            position: [10.0, 4.0, 0.0],
            ..Transform::default()
        };
        let local = local_under(Some(&parent), world);
        // The parent turned a quarter and doubled: (2, 0, 0) lands at (10, 4, 0).
        assert!((local.position[0] - 2.0).abs() < 1e-5, "{local:?}");
        assert!(local.position[1].abs() < 1e-5);
        assert!((local.scale[0] - 0.5).abs() < 1e-6);
    }
}
