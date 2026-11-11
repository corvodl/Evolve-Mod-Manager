//! Interactive terrain sculpting and painting for an editable Modding project.
//!
//! The project's Heightmap.dat or terrain.dat is loaded at full resolution
//! into memory. Brush strokes change that copy and stream the affected rows of
//! the ground mesh to the GPU, so the viewport updates as the mouse moves.
//! Nothing touches the project file until Save writes the edited rectangle as
//! one [`TerrainPatch`] through the verified project save.
use evolve_core::{TerrainInfo, TerrainPatch};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// The editor's active tool, shown in the tool bar of an editable project.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum LevelTool {
    #[default]
    Select,
    Sculpt,
    Paint,
}

/// What a left-button sculpt stroke does; holding Ctrl always lowers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum SculptMode {
    #[default]
    Raise,
    Flatten,
    Smooth,
    /// Turn terrain holes under the brush into ground at the stroke's
    /// starting height (terrain.dat maps, which have holes).
    Create,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct BrushSettings {
    pub mode: SculptMode,
    /// World metres.
    pub radius: f32,
    /// 0..1: how much one dab moves the surface.
    pub strength: f32,
    /// 0..1: the share of the radius at full strength before it fades out.
    pub hardness: f32,
    /// Paint: a LayerId (Heightmap.dat) or SurfaceTypeID (terrain.dat).
    pub surface: u8,
}

impl Default for BrushSettings {
    fn default() -> Self {
        Self {
            mode: SculptMode::Raise,
            radius: 8.0,
            strength: 0.3,
            hardness: 0.4,
            surface: 0,
        }
    }
}

/// Floats per ground vertex: position, UV, normal, layer paint, vertex color.
/// Must match `world_instances::Vertex`.
pub(super) const VERTEX_FLOATS: usize = 12;

/// Consecutive ground-mesh vertices to overwrite on the GPU.
pub(super) struct GroundPatch {
    pub first_vertex: u32,
    pub data: Vec<f32>,
}

/// Pending ground-mesh updates for the renderer.
#[derive(Default)]
pub(super) struct GroundUpdates {
    pub rows: Vec<GroundPatch>,
    /// A whole new index list, after holes were filled or restored.
    pub indices: Option<Vec<u32>>,
}

pub(super) type GroundPatches = Arc<Mutex<GroundUpdates>>;

/// A rectangle of samples (x0, y0, x1, y1 inclusive) and its old contents.
struct Snapshot {
    rect: [u32; 4],
    heights: Vec<f32>,
    surfaces: Option<Vec<u8>>,
    holes: Option<Vec<bool>>,
}

struct Stroke {
    lower: bool,
    paint: bool,
    flatten_to: f32,
    heights: Vec<f32>,
    surfaces: Option<Vec<u8>>,
    holes: Option<Vec<bool>>,
    touched: Option<[u32; 4]>,
}

pub(super) struct TerrainSession {
    pub source: PathBuf,
    pub compiled: bool,
    width: u32,
    height: u32,
    unit: f32,
    /// Heightmap.dat stores heights in 0..max_height.
    max_height: f32,
    heights: Vec<f32>,
    /// Current LayerIds or SurfaceTypeIDs, when the file has them.
    surfaces: Option<Vec<u8>>,
    saved_heights: Vec<f32>,
    saved_surfaces: Option<Vec<u8>>,
    /// terrain.dat: per-sample hole flags as the ground mesh was built.
    holes: Option<Vec<bool>>,
    saved_holes: Option<Vec<bool>>,
    dirty: Option<[u32; 4]>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    stroke: Option<Stroke>,
    /// The sample columns and rows the on-screen ground mesh keeps.
    xs: Vec<u32>,
    ys: Vec<u32>,
    patches: GroundPatches,
}

/// Where a ray first passes below a height field, refined by bisection.
pub(super) fn march(
    height_at: impl Fn(f32, f32) -> Option<f32>,
    origin: [f32; 3],
    direction: [f32; 3],
    step: f32,
    far: f32,
) -> Option<[f32; 3]> {
    let step = step.max(0.25);
    let at = |t: f32| std::array::from_fn::<f32, 3, _>(|i| origin[i] + direction[i] * t);
    let above = |p: [f32; 3]| height_at(p[0], p[1]).map(|h| p[2] > h);
    let mut previous = 0.0;
    let mut t = 0.0;
    while t < far {
        t += step;
        if above(at(t)) == Some(false) {
            let (mut lo, mut hi) = (previous, t);
            for _ in 0..12 {
                let mid = (lo + hi) * 0.5;
                if above(at(mid)) == Some(false) {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            return Some(at(hi));
        }
        previous = t;
    }
    None
}

/// Ground height at world XY from the on-screen terrain mesh (a grid of
/// `render_positions` columns and rows), bilinear within a cell.
pub(super) fn mesh_height(
    mesh: &evolve_core::RenderMesh,
    info: &TerrainInfo,
    x: f32,
    y: f32,
) -> Option<f32> {
    let xs = evolve_core::render_positions(info.width, info.stride);
    let ys = evolve_core::render_positions(info.height, info.stride);
    if xs.len() < 2 || ys.len() < 2 || mesh.positions.len() != xs.len() * ys.len() {
        return None;
    }
    let cell = |samples: &[u32], value: f32| -> Option<(usize, f32)> {
        let world = |i: usize| samples[i] as f32 * info.unit;
        if value < world(0) || value > world(samples.len() - 1) {
            return None;
        }
        let i = samples
            .partition_point(|s| *s as f32 * info.unit <= value)
            .clamp(1, samples.len() - 1)
            - 1;
        Some((i, (value - world(i)) / (world(i + 1) - world(i)).max(1e-6)))
    };
    let (i, tx) = cell(&xs, x)?;
    let (j, ty) = cell(&ys, y)?;
    let z = |i: usize, j: usize| mesh.positions[i * ys.len() + j][2];
    Some(
        (z(i, j) * (1.0 - tx) + z(i + 1, j) * tx) * (1.0 - ty)
            + (z(i, j + 1) * (1.0 - tx) + z(i + 1, j + 1) * tx) * ty,
    )
}

fn grow(rect: Option<[u32; 4]>, other: [u32; 4]) -> [u32; 4] {
    rect.map_or(other, |r| {
        [
            r[0].min(other[0]),
            r[1].min(other[1]),
            r[2].max(other[2]),
            r[3].max(other[3]),
        ]
    })
}

impl TerrainSession {
    /// Load the project's terrain file. `render` is the ground mesh's grid,
    /// which must cover the same samples.
    pub fn load(
        source: PathBuf,
        compiled: bool,
        render: &TerrainInfo,
        patches: GroundPatches,
    ) -> Result<Self, String> {
        let bytes = std::fs::read(&source).map_err(|e| e.to_string())?;
        let mut holes = None;
        let (width, height, unit, max_height, heights, surfaces) = if compiled {
            let terrain =
                evolve_core::decode_compiled_terrain(&bytes).map_err(|e| e.to_string())?;
            let side = terrain.units + 1;
            holes = Some(terrain.holes);
            (
                side,
                side,
                terrain.unit_size,
                f32::INFINITY,
                terrain.heights,
                Some(terrain.surfaces),
            )
        } else {
            let (info, heights, paint) =
                evolve_core::decode_terrain_samples(&bytes).map_err(|e| e.to_string())?;
            (
                info.width,
                info.height,
                info.unit,
                info.max_height,
                heights,
                paint,
            )
        };
        if render.width != width || render.height != height {
            return Err(format!(
                "The loaded ground mesh ({}×{}) does not match the terrain file ({width}×{height}); reopen the map.",
                render.width, render.height
            ));
        }
        Ok(Self {
            source,
            compiled,
            width,
            height,
            unit,
            max_height,
            saved_heights: heights.clone(),
            saved_surfaces: surfaces.clone(),
            saved_holes: holes.clone(),
            holes,
            heights,
            surfaces,
            dirty: None,
            undo: Vec::new(),
            redo: Vec::new(),
            stroke: None,
            xs: evolve_core::render_positions(width, render.stride),
            ys: evolve_core::render_positions(height, render.stride),
            patches,
        })
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty.is_some()
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn can_paint(&self) -> bool {
        self.surfaces.is_some()
    }
    /// Only terrain.dat has holes to turn into ground.
    pub fn can_create(&self) -> bool {
        self.holes.is_some()
    }

    fn index(&self, x: u32, y: u32) -> usize {
        x as usize * self.height as usize + y as usize
    }

    /// Bilinear height at world XY, or None off the map.
    pub fn height_at(&self, x: f32, y: f32) -> Option<f32> {
        let (fx, fy) = (x / self.unit, y / self.unit);
        if !(0.0..=(self.width - 1) as f32).contains(&fx)
            || !(0.0..=(self.height - 1) as f32).contains(&fy)
        {
            return None;
        }
        let (x0, y0) = (fx.floor() as u32, fy.floor() as u32);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let h = |x, y| self.heights[self.index(x, y)];
        Some(
            (h(x0, y0) * (1.0 - tx) + h(x1, y0) * tx) * (1.0 - ty)
                + (h(x0, y1) * (1.0 - tx) + h(x1, y1) * tx) * ty,
        )
    }

    /// Where a view ray first meets the terrain.
    pub fn hit(&self, origin: [f32; 3], direction: [f32; 3]) -> Option<[f32; 3]> {
        let far = (self.width.max(self.height) as f32 * self.unit * 1.5).min(20_000.0);
        march(
            |x, y| self.height_at(x, y),
            origin,
            direction,
            self.unit * 0.5,
            far,
        )
    }

    /// The brush outline on the ground, slightly lifted so it stays visible.
    pub fn ring(&self, center: [f32; 3], radius: f32) -> Vec<[f32; 3]> {
        (0..=48)
            .filter_map(|i| {
                let angle = i as f32 / 48.0 * std::f32::consts::TAU;
                let (x, y) = (
                    center[0] + radius * angle.cos(),
                    center[1] + radius * angle.sin(),
                );
                Some([x, y, self.height_at(x, y)? + 0.15])
            })
            .collect()
    }

    pub fn begin_stroke(&mut self, at: [f32; 3], lower: bool, paint: bool) {
        self.stroke = Some(Stroke {
            lower,
            paint: paint && self.surfaces.is_some(),
            flatten_to: self.height_at(at[0], at[1]).unwrap_or(at[2]),
            heights: self.heights.clone(),
            surfaces: self.surfaces.clone(),
            holes: self.holes.clone(),
            touched: None,
        });
    }

    /// Apply one brush dab at a ground point. Returns whether anything changed.
    pub fn dab(&mut self, at: [f32; 3], settings: &BrushSettings) -> bool {
        let Some(stroke) = self.stroke.as_ref() else {
            return false;
        };
        let (lower, paint, flatten_to) = (stroke.lower, stroke.paint, stroke.flatten_to);
        let radius = (settings.radius / self.unit).max(0.5);
        let (cx, cy) = (at[0] / self.unit, at[1] / self.unit);
        let x0 = (cx - radius).floor().max(0.0) as u32;
        let y0 = (cy - radius).floor().max(0.0) as u32;
        let x1 = ((cx + radius).ceil() as u32).min(self.width - 1);
        let y1 = ((cy + radius).ceil() as u32).min(self.height - 1);
        if x0 > x1 || y0 > y1 || cx < -radius || cy < -radius {
            return false;
        }
        let hardness = settings.hardness.clamp(0.0, 0.99);
        let strength = settings.strength.clamp(0.0, 1.0);
        let weight = |x: u32, y: u32| {
            let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt() / radius;
            if d > 1.0 {
                0.0
            } else if d <= hardness {
                1.0
            } else {
                let t = 1.0 - (d - hardness) / (1.0 - hardness);
                t * t * (3.0 - 2.0 * t)
            }
        };
        // Smooth reads the heights from before this dab.
        let before = self.heights.clone();
        let mut changed = false;
        let mut filled = false;
        for x in x0..=x1 {
            for y in y0..=y1 {
                let w = weight(x, y);
                if w <= 0.0 {
                    continue;
                }
                let index = self.index(x, y);
                if paint {
                    if w >= 0.5
                        && let Some(surfaces) = self.surfaces.as_mut()
                        && !(self.compiled && surfaces[index] == evolve_core::HOLE_ID)
                        && surfaces[index] != settings.surface
                    {
                        surfaces[index] = settings.surface;
                        changed = true;
                    }
                    continue;
                }
                if settings.mode == SculptMode::Create && !lower {
                    if w >= 0.5 && self.holes.as_ref().is_some_and(|holes| holes[index]) {
                        let surface = self
                            .nearest_ground_surface(x, y)
                            .unwrap_or(settings.surface);
                        if let Some(surfaces) = self.surfaces.as_mut() {
                            surfaces[index] = surface;
                        }
                        if let Some(holes) = self.holes.as_mut() {
                            holes[index] = false;
                        }
                        self.heights[index] = flatten_to;
                        filled = true;
                        changed = true;
                    }
                    continue;
                }
                let current = self.heights[index];
                let target = if lower {
                    current - strength * w * settings.radius * 0.05
                } else {
                    match settings.mode {
                        // Handled above: it fills holes rather than reshaping.
                        SculptMode::Create => current,
                        SculptMode::Raise => current + strength * w * settings.radius * 0.05,
                        SculptMode::Flatten => current + (flatten_to - current) * strength * w,
                        SculptMode::Smooth => {
                            let mut sum = 0.0;
                            let mut count = 0.0;
                            for nx in x.saturating_sub(1)..=(x + 1).min(self.width - 1) {
                                for ny in y.saturating_sub(1)..=(y + 1).min(self.height - 1) {
                                    sum += before[self.index(nx, ny)];
                                    count += 1.0;
                                }
                            }
                            current + (sum / count - current) * strength * w
                        }
                    }
                };
                let target = target.clamp(0.0, self.max_height);
                if (target - current).abs() > 1e-5 {
                    self.heights[index] = target;
                    changed = true;
                }
            }
        }
        if changed {
            let rect = [x0, y0, x1, y1];
            if let Some(stroke) = self.stroke.as_mut() {
                stroke.touched = Some(grow(stroke.touched, rect));
            }
            self.dirty = Some(grow(self.dirty, rect));
            self.publish(rect);
            if filled {
                self.publish_indices();
            }
        }
        changed
    }

    /// The surface of the closest sample that is ground, for new terrain.
    fn nearest_ground_surface(&self, x: u32, y: u32) -> Option<u8> {
        let (surfaces, holes) = (self.surfaces.as_ref()?, self.holes.as_ref()?);
        for ring in 1..=32u32 {
            for nx in x.saturating_sub(ring)..=(x + ring).min(self.width - 1) {
                for ny in y.saturating_sub(ring)..=(y + ring).min(self.height - 1) {
                    if nx.abs_diff(x) != ring && ny.abs_diff(y) != ring {
                        continue;
                    }
                    let index = self.index(nx, ny);
                    if !holes[index] && surfaces[index] != evolve_core::HOLE_ID {
                        return Some(surfaces[index]);
                    }
                }
            }
        }
        None
    }

    /// Rebuild the ground mesh's triangles, leaving out the quads whose
    /// corner sample is a hole, exactly as the terrain mesh was built.
    fn publish_indices(&self) {
        let Some(holes) = &self.holes else {
            return;
        };
        let rows = self.ys.len();
        let mut indices = Vec::with_capacity((self.xs.len() - 1) * (rows - 1) * 6);
        for (column, &x) in self.xs.iter().enumerate().take(self.xs.len() - 1) {
            for (row, &y) in self.ys.iter().enumerate().take(rows - 1) {
                if holes[self.index(x, y)] {
                    continue;
                }
                let a = (column * rows + row) as u32;
                let b = a + rows as u32;
                let c = a + 1;
                indices.extend_from_slice(&[a, b, c, b, b + 1, c]);
            }
        }
        if let Ok(mut queue) = self.patches.lock() {
            queue.indices = Some(indices);
        }
    }

    /// Finish a stroke as one undo step.
    pub fn end_stroke(&mut self) {
        let Some(stroke) = self.stroke.take() else {
            return;
        };
        let Some(rect) = stroke.touched else {
            return;
        };
        let snapshot = self.snapshot_of(
            rect,
            &stroke.heights,
            stroke.surfaces.as_deref(),
            stroke.holes.as_deref(),
        );
        self.undo.push(snapshot);
        self.redo.clear();
        while self.undo.len() > 64 {
            self.undo.remove(0);
        }
    }

    fn snapshot_of(
        &self,
        rect: [u32; 4],
        heights: &[f32],
        surfaces: Option<&[u8]>,
        holes: Option<&[bool]>,
    ) -> Snapshot {
        let mut snapshot = Snapshot {
            rect,
            heights: Vec::new(),
            surfaces: surfaces.map(|_| Vec::new()),
            holes: holes.map(|_| Vec::new()),
        };
        for x in rect[0]..=rect[2] {
            for y in rect[1]..=rect[3] {
                let index = self.index(x, y);
                snapshot.heights.push(heights[index]);
                if let (Some(out), Some(source)) = (snapshot.surfaces.as_mut(), surfaces) {
                    out.push(source[index]);
                }
                if let (Some(out), Some(source)) = (snapshot.holes.as_mut(), holes) {
                    out.push(source[index]);
                }
            }
        }
        snapshot
    }

    /// Swap a snapshot's region with the current contents; returns the swap.
    fn restore(&mut self, snapshot: Snapshot) -> Snapshot {
        let current = self.snapshot_of(
            snapshot.rect,
            &self.heights,
            self.surfaces.as_deref(),
            self.holes.as_deref(),
        );
        let rect = snapshot.rect;
        let mut at = 0;
        for x in rect[0]..=rect[2] {
            for y in rect[1]..=rect[3] {
                let index = self.index(x, y);
                self.heights[index] = snapshot.heights[at];
                if let (Some(out), Some(source)) =
                    (self.surfaces.as_mut(), snapshot.surfaces.as_ref())
                {
                    out[index] = source[at];
                }
                if let (Some(out), Some(source)) = (self.holes.as_mut(), snapshot.holes.as_ref()) {
                    out[index] = source[at];
                }
                at += 1;
            }
        }
        self.dirty = Some(grow(self.dirty, rect));
        self.publish(rect);
        self.publish_indices();
        current
    }

    pub fn undo(&mut self) -> bool {
        let Some(snapshot) = self.undo.pop() else {
            return false;
        };
        let redo = self.restore(snapshot);
        self.redo.push(redo);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(snapshot) = self.redo.pop() else {
            return false;
        };
        let undo = self.restore(snapshot);
        self.undo.push(undo);
        true
    }

    /// Put every edited sample back as last loaded or saved.
    pub fn discard(&mut self) {
        let Some(rect) = self.dirty.take() else {
            return;
        };
        self.heights.clone_from(&self.saved_heights);
        self.surfaces.clone_from(&self.saved_surfaces);
        self.holes.clone_from(&self.saved_holes);
        self.undo.clear();
        self.redo.clear();
        self.publish(rect);
        self.publish_indices();
    }

    /// The edited rectangle as one patch for the terrain file writer.
    pub fn patch(&self) -> Option<TerrainPatch> {
        let rect = self.dirty?;
        let mut patch = TerrainPatch {
            x: rect[0],
            y: rect[1],
            width: rect[2] - rect[0] + 1,
            height: rect[3] - rect[1] + 1,
            heights: Vec::new(),
            surfaces: Vec::new(),
        };
        let mut painted = false;
        let mut surfaces = Vec::new();
        for x in rect[0]..=rect[2] {
            for y in rect[1]..=rect[3] {
                let index = self.index(x, y);
                patch.heights.push(self.heights[index]);
                let surface = match (&self.surfaces, &self.saved_surfaces) {
                    (Some(now), Some(saved)) if now[index] != saved[index] => Some(now[index]),
                    _ => None,
                };
                painted |= surface.is_some();
                surfaces.push(surface);
            }
        }
        if painted {
            patch.surfaces = surfaces;
        }
        Some(patch)
    }

    /// Send the ground-mesh rows around `rect` (one sample wider, for their
    /// normals) to the renderer.
    fn publish(&self, rect: [u32; 4]) {
        let (x0, y0) = (rect[0].saturating_sub(1), rect[1].saturating_sub(1));
        let (x1, y1) = (rect[2] + 1, rect[3] + 1);
        let rows = self.ys.len() as u32;
        let first_row = self.ys.partition_point(|y| *y < y0);
        let last_row = self.ys.partition_point(|y| *y <= y1);
        if first_row >= last_row {
            return;
        }
        let h = |x: u32, y: u32| self.heights[self.index(x, y)];
        let mut out = Vec::new();
        for (column, &x) in self.xs.iter().enumerate() {
            if x < x0 || x > x1 {
                continue;
            }
            let mut data = Vec::with_capacity((last_row - first_row) * VERTEX_FLOATS);
            for &y in &self.ys[first_row..last_row] {
                let xm = x.saturating_sub(1);
                let xp = (x + 1).min(self.width - 1);
                let ym = y.saturating_sub(1);
                let yp = (y + 1).min(self.height - 1);
                let dx = (h(xp, y) - h(xm, y)) / ((xp - xm) as f32 * self.unit);
                let dy = (h(x, yp) - h(x, ym)) / ((yp - ym) as f32 * self.unit);
                let size = (dx * dx + dy * dy + 1.0).sqrt();
                data.extend_from_slice(&[
                    x as f32 * self.unit,
                    y as f32 * self.unit,
                    h(x, y),
                    x as f32 / (self.width - 1) as f32,
                    y as f32 / (self.height - 1) as f32,
                    -dx / size,
                    -dy / size,
                    1.0 / size,
                    1.0,
                    1.0,
                    1.0,
                    1.0,
                ]);
            }
            out.push(GroundPatch {
                first_vertex: column as u32 * rows + first_row as u32,
                data,
            });
        }
        if let Ok(mut queue) = self.patches.lock() {
            queue.rows.extend(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(width: u32, height: u32, z: f32) -> TerrainSession {
        TerrainSession {
            source: PathBuf::new(),
            compiled: false,
            width,
            height,
            unit: 1.0,
            max_height: 1000.0,
            heights: vec![z; (width * height) as usize],
            surfaces: Some(vec![0; (width * height) as usize]),
            saved_heights: vec![z; (width * height) as usize],
            saved_surfaces: Some(vec![0; (width * height) as usize]),
            holes: None,
            saved_holes: None,
            dirty: None,
            undo: Vec::new(),
            redo: Vec::new(),
            stroke: None,
            xs: evolve_core::render_positions(width, 1),
            ys: evolve_core::render_positions(height, 1),
            patches: GroundPatches::default(),
        }
    }

    #[test]
    fn rays_hit_the_ground_and_dabs_raise_lower_and_undo() {
        let mut session = flat(33, 33, 10.0);
        let hit = session.hit([16.0, 16.0, 50.0], [0.0, 0.0, -1.0]).unwrap();
        assert!((hit[2] - 10.0).abs() < 0.01);
        let settings = BrushSettings::default();
        session.begin_stroke(hit, false, false);
        assert!(session.dab(hit, &settings));
        session.end_stroke();
        let raised = session.height_at(16.0, 16.0).unwrap();
        assert!(raised > 10.0);
        assert!((session.height_at(1.0, 1.0).unwrap() - 10.0).abs() < 1e-6);
        session.begin_stroke(hit, true, false);
        session.dab(hit, &settings);
        session.end_stroke();
        assert!(session.height_at(16.0, 16.0).unwrap() < raised);
        assert!(session.undo());
        assert!((session.height_at(16.0, 16.0).unwrap() - raised).abs() < 1e-6);
        assert!(session.redo());
        let patch = session.patch().unwrap();
        assert_eq!(patch.heights.len(), (patch.width * patch.height) as usize);
        assert!(patch.surfaces.is_empty());
        assert!(!session.patches.lock().unwrap().rows.is_empty());
    }

    /// Brush a real Heightmap.dat and terrain.dat, save the patch with the
    /// project writers, and check the saved file and the live GPU rows both
    /// match the mesh the editor builds from that file.
    #[test]
    #[ignore = "reads the local asset library"]
    fn real_terrain_strokes_save_and_match_the_ground_mesh() {
        let library = PathBuf::from(std::env::var("EVOLVE_BENCH_LIBRARY").unwrap());
        let find = |archive: &str, name: &str| {
            let mut stack = std::fs::read_dir(&library)
                .unwrap()
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with(archive))
                })
                .collect::<Vec<_>>();
            while let Some(path) = stack.pop() {
                if path.is_dir() {
                    stack.extend(
                        std::fs::read_dir(&path)
                            .unwrap()
                            .flatten()
                            .map(|e| e.path()),
                    );
                } else if path
                    .to_string_lossy()
                    .replace('\\', "/")
                    .to_ascii_lowercase()
                    .ends_with(name)
                {
                    return path;
                }
            }
            panic!("{name} not found in {archive}");
        };
        let cases = [
            (find("levels_tutorial.pak", "heightmap.dat"), false),
            (
                find("level.pak__2b2b6d03b7833f0140d6", "terrain/terrain.dat"),
                true,
            ),
        ];
        for (path, compiled) in cases {
            let bytes = std::fs::read(&path).unwrap();
            let mesh = |bytes: &[u8]| -> evolve_core::TerrainMesh {
                if compiled {
                    evolve_core::decode_compiled_terrain(bytes)
                        .unwrap()
                        .mesh()
                        .unwrap()
                } else {
                    evolve_core::decode_terrain_bytes(bytes, evolve_core::MAX_RENDER_GRID).unwrap()
                }
            };
            let before = mesh(&bytes);
            let patches = GroundPatches::default();
            let mut session =
                TerrainSession::load(path.clone(), compiled, &before.info, patches.clone())
                    .unwrap();
            let center = [
                before.info.width as f32 * before.info.unit * 0.5,
                before.info.height as f32 * before.info.unit * 0.5,
            ];
            let ground = session.height_at(center[0], center[1]).unwrap();
            let hit = session
                .hit([center[0], center[1], ground + 200.0], [0.0, 0.0, -1.0])
                .unwrap();
            assert!((hit[2] - ground).abs() < 0.05);
            let settings = BrushSettings {
                radius: 12.0,
                strength: 0.8,
                ..BrushSettings::default()
            };
            session.begin_stroke(hit, false, false);
            for step in 0..10 {
                let at = [hit[0] + step as f32, hit[1], hit[2]];
                assert!(session.dab(at, &settings));
            }
            session.end_stroke();
            let patch = session.patch().unwrap();
            let saved = if compiled {
                evolve_core::patch_compiled_terrain_bytes(&bytes, &patch).unwrap()
            } else {
                evolve_core::patch_terrain_bytes(&bytes, &patch).unwrap()
            };
            let after = mesh(&saved);
            assert_eq!(after.vertices.len(), before.vertices.len());
            // Every live row the brush streamed equals the reloaded mesh.
            let tolerance = if compiled { 0.03 } else { 0.01 };
            let mut checked = 0;
            // Later rows overwrite earlier ones on the GPU, as here.
            let mut last = std::collections::BTreeMap::new();
            for row in patches.lock().unwrap().rows.iter() {
                for (k, vertex) in row.data.chunks(VERTEX_FLOATS).enumerate() {
                    last.insert(row.first_vertex as usize + k, vertex.to_vec());
                }
            }
            for (index, vertex) in last {
                {
                    let want = after.vertices[index];
                    for axis in 0..3 {
                        assert!(
                            (vertex[axis] - want[axis]).abs() < tolerance,
                            "{}: vertex {index} axis {axis}: {} vs {}",
                            path.display(),
                            vertex[axis],
                            want[axis]
                        );
                    }
                    let normal = after.normals[index];
                    assert!((vertex[5] - normal[0]).abs() < 0.05);
                    assert!((vertex[7] - normal[2]).abs() < 0.05);
                    assert_eq!([vertex[3], vertex[4]], after.uv[index]);
                    checked += 1;
                }
            }
            assert!(checked > 100, "{checked} vertices checked");
            let raised = after
                .vertices
                .iter()
                .zip(&before.vertices)
                .filter(|(a, b)| a[2] > b[2] + 0.05)
                .count();
            println!(
                "{}: {raised} vertices raised, {checked} live vertices match",
                path.display()
            );
            assert!(raised > 50);
            if !compiled {
                continue;
            }
            // Create: drag from ground into a hole next to it.
            let patches = GroundPatches::default();
            let mut session =
                TerrainSession::load(path.clone(), true, &before.info, patches.clone()).unwrap();
            let holes = session.holes.clone().unwrap();
            let side = session.height;
            let unit = session.unit;
            let edge = (2..side - 2)
                .flat_map(|x| (2..side - 2).map(move |y| (x, y)))
                .find(|&(x, y)| {
                    holes[(x * side + y) as usize]
                        && !holes[(x * side + y - 1) as usize]
                        && holes[(x * side + y + 1) as usize]
                })
                .expect("a hole beside ground");
            let start = [edge.0 as f32 * unit, (edge.1 - 1) as f32 * unit, 0.0];
            let start = [
                start[0],
                start[1],
                session.height_at(start[0], start[1]).unwrap(),
            ];
            let settings = BrushSettings {
                mode: SculptMode::Create,
                radius: unit * 3.0,
                ..BrushSettings::default()
            };
            session.begin_stroke(start, false, false);
            let target = [edge.0 as f32 * unit, (edge.1 + 1) as f32 * unit, start[2]];
            assert!(session.dab(target, &settings));
            session.end_stroke();
            let patch = session.patch().unwrap();
            assert!(patch.surfaces.iter().any(Option::is_some));
            let saved = evolve_core::patch_compiled_terrain_bytes(&bytes, &patch).unwrap();
            let decoded = evolve_core::decode_compiled_terrain(&saved).unwrap();
            let filled = before_holes(&bytes) - decoded.holes.iter().filter(|h| **h).count();
            assert!(filled > 0, "no holes filled");
            let after = mesh(&saved);
            let live = patches
                .lock()
                .unwrap()
                .indices
                .clone()
                .expect("indices rebuilt");
            assert_eq!(live, after.indices, "live triangles equal the saved mesh");
            println!("{}: Create filled {filled} hole samples", path.display());
        }
    }

    fn before_holes(bytes: &[u8]) -> usize {
        evolve_core::decode_compiled_terrain(bytes)
            .unwrap()
            .holes
            .iter()
            .filter(|h| **h)
            .count()
    }

    #[test]
    fn mesh_heights_and_rays_follow_a_strided_ground_grid() {
        // A 5×5-sample, unit-2 map drawn with stride 2: columns 0, 2, 4.
        let info = TerrainInfo {
            width: 5,
            height: 5,
            unit: 2.0,
            max_height: 100.0,
            decoded_samples: 25,
            stride: 2,
        };
        let mut positions = Vec::new();
        for x in [0u32, 2, 4] {
            for y in [0u32, 2, 4] {
                positions.push([x as f32 * 2.0, y as f32 * 2.0, (x + y) as f32]);
            }
        }
        let mesh = evolve_core::RenderMesh {
            info: evolve_core::ModelInfo {
                format: String::new(),
                version: 1,
                chunk_count: 0,
                vertex_count: 9,
                triangle_count: 0,
                original_uv0: false,
                original_normals: false,
            },
            positions,
            indices: Vec::new(),
            uv0: None,
            normals: None,
            vertex_alpha: None,
            vertex_color: None,
        };
        // z = (x + y) / 2 in world metres.
        assert!((mesh_height(&mesh, &info, 3.0, 5.0).unwrap() - 4.0).abs() < 1e-5);
        assert!(mesh_height(&mesh, &info, 9.0, 1.0).is_none());
        let hit = march(
            |x, y| mesh_height(&mesh, &info, x, y),
            [4.0, 4.0, 50.0],
            [0.0, 0.0, -1.0],
            1.0,
            100.0,
        )
        .unwrap();
        assert!((hit[2] - 4.0).abs() < 0.01);
    }

    #[test]
    fn flatten_smooth_and_paint() {
        let mut session = flat(33, 33, 10.0);
        let center = session.index(16, 16);
        session.heights[center] = 20.0;
        let settings = BrushSettings {
            mode: SculptMode::Smooth,
            strength: 1.0,
            ..BrushSettings::default()
        };
        session.begin_stroke([16.0, 16.0, 20.0], false, false);
        session.dab([16.0, 16.0, 20.0], &settings);
        session.end_stroke();
        assert!(session.heights[session.index(16, 16)] < 20.0);
        let flatten = BrushSettings {
            mode: SculptMode::Flatten,
            strength: 1.0,
            hardness: 0.99,
            ..BrushSettings::default()
        };
        session.begin_stroke([10.0, 10.0, 10.0], false, false);
        session.dab([16.0, 16.0, 10.0], &flatten);
        session.end_stroke();
        assert!((session.heights[session.index(16, 16)] - 10.0).abs() < 1e-4);
        let paint = BrushSettings {
            surface: 3,
            ..BrushSettings::default()
        };
        session.begin_stroke([16.0, 16.0, 10.0], false, true);
        session.dab([16.0, 16.0, 10.0], &paint);
        session.end_stroke();
        let patch = session.patch().unwrap();
        assert!(patch.surfaces.contains(&Some(3)));
    }
}
