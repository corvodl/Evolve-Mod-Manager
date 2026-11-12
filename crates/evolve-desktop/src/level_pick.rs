//! Source-geometry 3D picking for the Level Editor. The renderer remains the
//! existing textured wgpu pipeline; this adds accurate triangle selection in
//! screen space rather than picking the nearest 2D overview dot.
use super::level_gizmo::{self, Handle};
use super::level_transform::Mode;
use super::{Message, level_scene_renderer::PickRange, textured};
use evolve_core::RenderMesh;
use iced::{Event, Rectangle, mouse, widget::shader};
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct LevelViewport {
    pub inner: textured::Scene,
    pub picks: Arc<Vec<PickRange>>,
    /// The transform gizmo to show for the selection, and where.
    pub gizmo: Option<(Mode, [f32; 3])>,
    pub editable: bool,
    /// A terrain sculpt or paint tool is active: left-drag brushes the
    /// ground; right and middle drag and the keyboard still move the camera.
    pub brush: bool,
    /// Ctrl is held: a sculpt stroke lowers instead.
    pub brush_invert: bool,
    pub brush_ring: Option<Arc<Vec<[f32; 3]>>>,
    /// A G/R/S transform in progress, and its axis: the mouse drives it,
    /// LMB applies and RMB cancels.
    pub transforming: Option<(Mode, Option<usize>)>,
    /// The selected area's polygon vertices (world), each a draggable handle.
    pub area_vertices: Option<Arc<Vec<[f32; 3]>>>,
}

#[derive(Default)]
pub(super) struct ViewState {
    navigation: super::view_navigation::Navigation,
    /// A gizmo handle is being dragged: releasing LMB applies it.
    gizmo_drag: bool,
    /// The polygon vertex being dragged.
    vertex_drag: Option<usize>,
    /// The mouse button of a brush stroke in progress.
    brushing: Option<mouse::Button>,
}

impl LevelViewport {
    /// A world point in viewport pixels, or None behind the camera.
    fn screen(&self, bounds: Rectangle) -> Option<impl Fn([f32; 3]) -> Option<[f32; 2]>> {
        let camera = self.inner.level_camera?;
        let (yaw, pitch, zoom) = (self.inner.yaw, self.inner.pitch, self.inner.zoom);
        let (width, height) = (bounds.width.max(1.0), bounds.height.max(1.0));
        Some(move |point: [f32; 3]| {
            let ndc = camera.project(point, yaw, pitch, width / height, zoom);
            (ndc[0] != 3.0 || ndc[1] != 3.0)
                .then(|| [(ndc[0] + 1.0) * width * 0.5, (1.0 - ndc[1]) * height * 0.5])
        })
    }

    /// World length of the gizmo arms: about 90 pixels at the pivot's depth.
    fn gizmo_size(&self, pivot: [f32; 3], bounds: Rectangle) -> Option<f32> {
        let camera = self.inner.level_camera?;
        let (forward, _, _) = super::level_camera::basis(self.inner.yaw, self.inner.pitch);
        let depth = (0..3)
            .map(|i| (pivot[i] - camera.position[i]) * forward[i])
            .sum::<f32>();
        if depth <= 0.1 {
            return None;
        }
        let tangent = (camera.fov_degrees.clamp(25.0, 110.0).to_radians() * 0.5).tan()
            / self.inner.zoom.clamp(0.2, 80.0);
        Some(depth * tangent * 90.0 / (bounds.height.max(1.0) * 0.5))
    }

    /// The gizmo handle under a viewport position.
    fn gizmo_handle(&self, bounds: Rectangle, position: iced::Point) -> Option<(Mode, Handle)> {
        let (mode, pivot) = self.gizmo?;
        let size = self.gizmo_size(pivot, bounds)?;
        let screen = self.screen(bounds)?;
        level_gizmo::hit(mode, pivot, size, &screen, [position.x, position.y])
            .map(|handle| (mode, handle))
    }

    /// The world ray under a viewport position, in flight-camera mode.
    fn ray(&self, bounds: Rectangle, position: iced::Point) -> Option<([f32; 3], [f32; 3])> {
        let camera = self.inner.level_camera?;
        let point = [
            2.0 * position.x / bounds.width.max(1.0) - 1.0,
            1.0 - 2.0 * position.y / bounds.height.max(1.0),
        ];
        let direction = super::level_camera::view_ray(
            camera,
            self.inner.yaw,
            self.inner.pitch,
            bounds.width / bounds.height.max(1.0),
            self.inner.zoom,
            point,
        )?;
        Some((camera.position, direction))
    }

    /// Brush input. None leaves the event to camera navigation.
    fn brush_event(
        &self,
        state: &mut ViewState,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<shader::Action<Message>> {
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let (origin, direction) = self.ray(bounds, cursor.position_in(bounds)?)?;
                state.brushing = Some(mouse::Button::Left);
                Some(
                    shader::Action::publish(Message::LevelBrushPress {
                        lower: self.brush_invert,
                        origin,
                        direction,
                    })
                    .and_capture(),
                )
            }
            Event::Mouse(mouse::Event::ButtonReleased(button))
                if state.brushing == Some(*button) =>
            {
                state.brushing = None;
                Some(shader::Action::publish(Message::LevelBrushRelease).and_capture())
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if state.brushing.is_some() => {
                let (origin, direction) = self.ray(bounds, cursor.position_in(bounds)?)?;
                Some(
                    shader::Action::publish(Message::LevelBrushDrag(origin, direction))
                        .and_capture(),
                )
            }
            _ => None,
        }
    }
}

struct Projection {
    center: [f32; 3],
    extent: f32,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    camera: Option<super::level_camera::Camera>,
    pan_x: f32,
    pan_y: f32,
    width: f32,
    height: f32,
}
impl Projection {
    fn new(scene: &textured::Scene, bounds: Rectangle) -> Option<Self> {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for p in &scene.mesh.positions {
            for i in 0..3 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        }
        if !min.iter().chain(&max).all(|v| v.is_finite()) {
            return None;
        }
        let center = std::array::from_fn(|i| (min[i] + max[i]) * 0.5);
        let extent = (0..3).map(|i| max[i] - min[i]).fold(1e-6f32, f32::max);
        Some(Self {
            center,
            extent,
            yaw: scene.yaw.to_radians(),
            pitch: scene.pitch.to_radians(),
            zoom: scene.zoom.clamp(0.2, 80.0),
            camera: scene.level_camera,
            pan_x: scene.pan_x,
            pan_y: scene.pan_y,
            width: bounds.width.max(1.0),
            height: bounds.height.max(1.0),
        })
    }
    fn project(&self, raw: [f32; 3]) -> [f32; 3] {
        if let Some(camera) = self.camera {
            let ndc = camera.project(
                raw,
                self.yaw.to_degrees(),
                self.pitch.to_degrees(),
                self.width / self.height,
                self.zoom,
            );
            return [
                (ndc[0] + 1.0) * self.width * 0.5,
                (1.0 - ndc[1]) * self.height * 0.5,
                ndc[2],
            ];
        }
        let x = (raw[0] - self.center[0]) / self.extent;
        let y = (raw[1] - self.center[1]) / self.extent;
        let z = (raw[2] - self.center[2]) / self.extent;
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let xr = x * cy - y * sy;
        let back = x * sy + y * cy;
        let yr = z * cp - back * sp;
        let depth = z * sp + back * cp;
        let perspective = (2.5 - depth).max(0.12);
        let scale = self.zoom * 1.6;
        let aspect = (self.width / self.height).max(0.01);
        let ndc_x = xr * scale / perspective / aspect + self.pan_x;
        let ndc_y = yr * scale / perspective + self.pan_y;
        [
            (ndc_x + 1.0) * self.width * 0.5,
            (1.0 - ndc_y) * self.height * 0.5,
            (2.5 - depth) / 5.0,
        ]
    }
}

fn hit_triangle(p: [f32; 2], a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> Option<f32> {
    let cross = |u: [f32; 2], v: [f32; 2]| u[0] * v[1] - u[1] * v[0];
    let ab = [b[0] - a[0], b[1] - a[1]];
    let ac = [c[0] - a[0], c[1] - a[1]];
    let ap = [p[0] - a[0], p[1] - a[1]];
    let area = cross(ab, ac);
    if area.abs() < 1e-7 {
        return None;
    }
    let u = cross(ap, ac) / area;
    let v = cross(ab, ap) / area;
    if u < -1e-4 || v < -1e-4 || u + v > 1.0001 {
        return None;
    }
    let d = a[2] * (1.0 - u - v) + b[2] * u + c[2] * v;
    d.is_finite().then_some(d)
}

/// Click-only pixel triangle test. Index ranges are derived at assembly time
/// from the original object's exact mesh instances. Geometry, not the nearest
/// pivot or arbitrary object name, decides selection.
fn pick(
    mesh: &RenderMesh,
    ranges: &[PickRange],
    projector: &Projection,
    click: [f32; 2],
) -> Option<usize> {
    let mut closest = (f32::INFINITY, None);
    // This bounded routine is for selection, not frame-by-frame rendering.
    // The source mesh stays immutable. Under heavy maps a future spatial BVH
    // will replace this budgeted exact subset test.
    let mut tested = 0usize;
    for range in ranges {
        let end = range
            .first_index
            .saturating_add(range.index_count)
            .min(mesh.indices.len());
        for tri in mesh.indices[range.first_index.min(end)..end]
            .as_chunks::<3>()
            .0
        {
            if tested >= 300_000 {
                return closest.1;
            }
            tested += 1;
            let Some(a) = mesh.positions.get(tri[0] as usize) else {
                continue;
            };
            let Some(b) = mesh.positions.get(tri[1] as usize) else {
                continue;
            };
            let Some(c) = mesh.positions.get(tri[2] as usize) else {
                continue;
            };
            let a = projector.project(*a);
            let b = projector.project(*b);
            let c = projector.project(*c);
            if let Some(distance) = hit_triangle(click, a, b, c)
                && distance >= 0.0
                && distance < closest.0
            {
                closest = (distance, Some(range.object_index));
            }
        }
    }
    closest.1
}

impl shader::Program<Message> for LevelViewport {
    type State = ViewState;
    type Primitive = textured::Primitive;
    fn update(
        &self,
        state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<shader::Action<Message>> {
        // Initialise the existing orbit/pan gesture for subsequent mouse moves
        // even when an object is selected by this click.
        let location = cursor.position_in(bounds);
        if self.transforming.is_some() {
            return match event {
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                    Some(shader::Action::publish(Message::LevelTransformConfirm).and_capture())
                }
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                    if state.gizmo_drag =>
                {
                    state.gizmo_drag = false;
                    Some(shader::Action::publish(Message::LevelTransformConfirm).and_capture())
                }
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                    state.gizmo_drag = false;
                    Some(shader::Action::publish(Message::LevelTransformCancel).and_capture())
                }
                Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                    let ray = location.and_then(|position| self.ray(bounds, position));
                    Some(shader::Action::publish(Message::LevelPointerRay(ray)).and_capture())
                }
                _ => None,
            };
        }
        if self.brush {
            if let Some(action) = self.brush_event(state, event, bounds, cursor) {
                return Some(action);
            }
            let navigation = state
                .navigation
                .handle(event, bounds, cursor, self.inner.shift);
            if navigation.is_none()
                && let Event::Mouse(mouse::Event::CursorMoved { .. }) = event
            {
                let ray = location.and_then(|position| self.ray(bounds, position));
                return Some(shader::Action::publish(Message::LevelBrushHover(ray)));
            }
            return navigation
                .map(|gesture| match gesture {
                    super::view_navigation::Gesture::Zoom(delta) => {
                        Message::LevelCameraZoomWheel(delta)
                    }
                    super::view_navigation::Gesture::Pan(dx, dy) => Message::ViewerOrbitDelta(
                        dx * bounds.width * 0.175,
                        -dy * bounds.height * 0.175,
                    ),
                    other => super::view_navigation::message(other),
                })
                .map(|message| shader::Action::publish(message).and_capture());
        }
        // Dragging an area's polygon vertex across the ground.
        if let Some(vertex) = state.vertex_drag {
            match event {
                Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                    let ray = location.and_then(|position| self.ray(bounds, position));
                    return Some(
                        shader::Action::publish(Message::LevelVertexDrag(vertex, ray))
                            .and_capture(),
                    );
                }
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                    state.vertex_drag = None;
                    return Some(
                        shader::Action::publish(Message::LevelVertexRelease).and_capture(),
                    );
                }
                _ => {}
            }
        }
        if self.editable
            && matches!(
                event,
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            )
            && let Some(position) = location
            && let Some(vertices) = &self.area_vertices
            && let Some(screen) = self.screen(bounds)
            && let Some(vertex) =
                level_gizmo::hit_vertex(vertices, &screen, [position.x, position.y])
        {
            state.vertex_drag = Some(vertex);
            return Some(
                shader::Action::publish(Message::LevelVertexDrag(
                    vertex,
                    self.ray(bounds, position),
                ))
                .and_capture(),
            );
        }
        if matches!(
            event,
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
        ) {
            state.gizmo_drag = false;
        }
        // Grabbing a gizmo handle starts that transform; releasing applies it.
        if self.editable
            && matches!(
                event,
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            )
            && let Some(position) = location
            && let Some((mode, handle)) = self.gizmo_handle(bounds, position)
        {
            state.gizmo_drag = true;
            let axis = match handle {
                Handle::Axis(axis) => Some(axis),
                Handle::Free => None,
            };
            return Some(
                shader::Action::publish(Message::LevelGizmoGrab(
                    mode,
                    axis,
                    self.ray(bounds, position),
                ))
                .and_capture(),
            );
        }
        let navigation = state
            .navigation
            .handle(event, bounds, cursor, self.inner.shift);
        if matches!(
            event,
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
        ) && !self.inner.shift
            && let Some(position) = cursor.position_in(bounds)
        {
            let triangles = Projection::new(&self.inner, bounds).and_then(|proj| {
                pick(
                    &self.inner.mesh,
                    &self.picks,
                    &proj,
                    [position.x, position.y],
                )
            });
            let instance = self.inner.world.as_ref().and_then(|world| {
                self.inner.level_camera.and_then(|camera| {
                    let point = [
                        2.0 * position.x / bounds.width.max(1.0) - 1.0,
                        1.0 - 2.0 * position.y / bounds.height.max(1.0),
                    ];
                    world.pick(
                        camera,
                        self.inner.yaw,
                        self.inner.pitch,
                        bounds.width / bounds.height.max(1.0),
                        self.inner.zoom,
                        point,
                    )
                })
            });
            if let Some(index) = triangles.or(instance) {
                return Some(shader::Action::publish(Message::LevelSelect(index)).and_capture());
            }
        }
        navigation
            .map(|gesture| match gesture {
                super::view_navigation::Gesture::Zoom(delta) => {
                    Message::LevelCameraZoomWheel(delta)
                }
                super::view_navigation::Gesture::Pan(dx, dy) => {
                    // In flight mode RMB-drag is mouse-look rather than
                    // shifting the entire rendered map off the screen.
                    if self.inner.level_camera.is_some() {
                        Message::ViewerOrbitDelta(
                            dx * bounds.width * 0.175,
                            -dy * bounds.height * 0.175,
                        )
                    } else {
                        Message::LevelCameraPanDelta(dx, dy)
                    }
                }
                other => super::view_navigation::message(other),
            })
            .map(|message| shader::Action::publish(message).and_capture())
            .or_else(|| {
                // Track the ray under the mouse so Ctrl+V can paste there.
                let Event::Mouse(mouse::Event::CursorMoved { .. }) = event else {
                    return None;
                };
                let ray = location.and_then(|position| self.ray(bounds, position));
                Some(shader::Action::publish(Message::LevelPointerRay(ray)))
            })
    }
    fn draw(
        &self,
        state: &Self::State,
        cursor: mouse::Cursor,
        bounds: Rectangle,
    ) -> Self::Primitive {
        let mut primitive = <textured::Scene as shader::Program<Message>>::draw(
            &self.inner,
            &state.navigation,
            cursor,
            bounds,
        );
        if self.brush
            && let Some(ring) = &self.brush_ring
            && let Some(proj) = Projection::new(&self.inner, bounds)
        {
            let ndc = |p: [f32; 3]| {
                let p = proj.project(p);
                [
                    p[0] * 2.0 / proj.width - 1.0,
                    1.0 - p[1] * 2.0 / proj.height,
                    p[2].clamp(0.0, 1.0),
                ]
            };
            for pair in ring.windows(2) {
                primitive.add_level_axis(ndc(pair[0]), ndc(pair[1]), [1.0, 0.78, 0.25, 1.0]);
            }
        }
        if let Some(vertices) = &self.area_vertices
            && let Some(screen) = self.screen(bounds)
        {
            let focus = state.vertex_drag.or_else(|| {
                let position = cursor.position_in(bounds)?;
                level_gizmo::hit_vertex(vertices, &screen, [position.x, position.y])
            });
            level_gizmo::draw_vertices(
                &mut primitive,
                vertices,
                &screen,
                bounds.width.max(1.0),
                bounds.height.max(1.0),
                focus,
            );
        }
        if let Some((mode, pivot)) = self.gizmo
            && let Some(size) = self.gizmo_size(pivot, bounds)
            && let Some(screen) = self.screen(bounds)
        {
            let (mode, focus, dim) = match self.transforming {
                // While transforming, the gizmo shows the operation and its axis.
                Some((mode, axis)) => (mode, Some(axis.map_or(Handle::Free, Handle::Axis)), true),
                None => (
                    mode,
                    cursor
                        .position_in(bounds)
                        .and_then(|position| self.gizmo_handle(bounds, position))
                        .map(|(_, handle)| handle),
                    false,
                ),
            };
            level_gizmo::draw(
                &mut primitive,
                mode,
                pivot,
                size,
                &screen,
                bounds.width.max(1.0),
                bounds.height.max(1.0),
                focus,
                dim,
            );
        }
        primitive
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn triangle_hit_uses_barycentric_depth() {
        let a = [0.0, 0.0, 0.4];
        let b = [100.0, 0.0, 0.4];
        let c = [0.0, 100.0, 0.4];
        assert!(hit_triangle([25.0, 25.0], a, b, c).is_some());
        assert!(hit_triangle([90.0, 90.0], a, b, c).is_none());
    }
}
