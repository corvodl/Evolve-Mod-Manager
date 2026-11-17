//! Mip levels for level-world textures, computed by a background worker pool
//! ahead of the render thread. The pixels are exactly the ones the synchronous
//! upload path produces (same filter, same source for each level); only where
//! and when they are computed changes. Resizing thousands of decoded DDS maps
//! on the render thread froze every window for seconds while a map streamed.
use crate::level_camera;
use crate::world_instances::{
    ModelGroup, World, eager_diffuse_mips, group_distance_sq, is_ground, next_diffuse_mip,
};
use std::collections::{HashMap, HashSet};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    mpsc,
};
use std::time::Duration;

/// Prepared levels waiting in memory for their upload. Workers pause above
/// this, so a whole map's mip chains never sit in RAM at once.
const HELD_BUDGET_BYTES: u64 = 768 * 1024 * 1024;

/// Threads for map decoding and mip preparation: every core but two, so the
/// interface and the render thread stay responsive.
pub(super) fn worker_threads() -> usize {
    std::thread::available_parallelism()
        .map_or(1, |count| count.get().saturating_sub(2))
        .clamp(1, 16)
}

/// `f` over `items` on `worker_threads()` scoped threads, in item order.
pub(super) fn parallel_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let mut done = std::thread::scope(|scope| {
        let workers = (0..worker_threads().min(items.len()))
            .map(|_| {
                scope.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(index) else { break };
                        out.push((index, f(item)));
                    }
                    out
                })
            })
            .collect::<Vec<_>>();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap_or_default())
            .collect::<Vec<_>>()
    });
    done.sort_by_key(|(index, _)| *index);
    done.into_iter().map(|(_, value)| value).collect()
}

/// Chained levels (as `upload_rgba_mips` makes them) for each distinct image,
/// built on all cores and keyed by address. For textures uploaded all at once
/// when a world arrives, where a serial chain stalled the first frame.
pub(super) fn chained_levels<'a>(
    images: impl IntoIterator<Item = &'a image::RgbaImage>,
) -> HashMap<usize, Vec<image::RgbaImage>> {
    let mut seen = HashSet::new();
    let distinct = images
        .into_iter()
        .filter(|image| seen.insert(std::ptr::from_ref(*image) as usize))
        .collect::<Vec<_>>();
    let levels = parallel_map(&distinct, |image| compute(image, MipKind::Chained));
    distinct
        .iter()
        .map(|image| std::ptr::from_ref(*image) as usize)
        .zip(levels)
        .collect()
}

/// How a texture's smaller levels are derived; each matches one upload path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum MipKind {
    /// Each level resized from the one above it (`upload_rgba_mips`).
    Chained,
    /// Each level resized from the original (streamed full-size diffuse).
    FromSource,
}

/// Levels 1.. of a source image; level 0 is the source itself.
pub(super) type MipLevels = Arc<Vec<image::RgbaImage>>;
type Key = (usize, MipKind);

pub(super) fn level_count(width: u32, height: u32) -> u32 {
    width.max(height).max(1).ilog2() + 1
}

/// The level below `previous`, as the synchronous chained upload makes it.
pub(super) fn chained_next(previous: &image::RgbaImage) -> image::RgbaImage {
    image::imageops::resize(
        previous,
        (previous.width() / 2).max(1),
        (previous.height() / 2).max(1),
        image::imageops::FilterType::Triangle,
    )
}

fn compute(source: &image::RgbaImage, kind: MipKind) -> Vec<image::RgbaImage> {
    let count = level_count(source.width(), source.height());
    match kind {
        MipKind::Chained => {
            let mut levels: Vec<image::RgbaImage> = Vec::with_capacity(count as usize - 1);
            for _ in 1..count {
                let next = chained_next(levels.last().unwrap_or(source));
                levels.push(next);
            }
            levels
        }
        MipKind::FromSource => (1..count)
            .map(|level| next_diffuse_mip(source, level))
            .collect(),
    }
}

/// One texture a world group samples, as `Renderer::group` resolves it.
pub(super) struct Source<'a> {
    pub image: &'a Arc<image::RgbaImage>,
    /// Texture-cache role and ground flag, as `texture_key` takes them.
    pub role: u8,
    pub ground: bool,
    pub kind: MipKind,
}

/// Every texture `Renderer::group` would upload for `group`, with the mip
/// path each one takes at full quality.
pub(super) fn group_sources<'a>(world: &'a World, group: &'a ModelGroup) -> Vec<Source<'a>> {
    let ground = is_ground(group);
    let mut sources = Vec::new();
    for surface in &group.surfaces {
        if !surface.visible
            || surface.index_count == 0
            || surface.first_index as usize + surface.index_count as usize
                > group.mesh.indices.len()
        {
            continue;
        }
        if let Some(image) = &surface.diffuse {
            sources.push(Source {
                image,
                role: 0,
                ground,
                kind: if eager_diffuse_mips(image.width(), image.height()) {
                    MipKind::Chained
                } else {
                    MipKind::FromSource
                },
            });
        }
        let detail = if ground && world.terrain_gpu.is_none() {
            world.terrain_detail.as_ref()
        } else {
            surface.detail_map.as_ref()
        };
        for (role, image) in [
            (1, surface.opacity.as_ref()),
            (2, surface.normal.as_ref()),
            (3, surface.specular.as_ref()),
            (4, surface.gloss.as_ref()),
            (
                5,
                surface.blend_normal.as_ref().or(surface.emissive.as_ref()),
            ),
            (6, detail),
            (7, surface.blend_layer.as_ref()),
            (8, surface.blend_mask.as_ref()),
        ] {
            if let Some(image) = image {
                sources.push(Source {
                    image,
                    role,
                    ground: false,
                    kind: MipKind::Chained,
                });
            }
        }
    }
    sources
}

/// Background preparation for one world at full texture quality. Dropping it
/// stops its workers.
pub(super) struct MipPrep {
    /// Only read through `&mut self` (`get_mut`), so it never locks; the
    /// mutex just makes the renderer `Sync` as iced's pipeline requires.
    results: Mutex<mpsc::Receiver<(Key, MipLevels, u64)>>,
    ready: HashMap<Key, (MipLevels, u64)>,
    held: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
    running: bool,
    device_limit: u32,
}

impl Drop for MipPrep {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl MipPrep {
    /// Start preparing every texture of `world`, ground first and then by
    /// distance from `camera`, the order the renderer streams groups in.
    pub(super) fn start(world: Arc<World>, camera: Option<[f32; 3]>, device_limit: u32) -> Self {
        let (sender, results) = mpsc::channel();
        let held = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (worker_held, worker_cancel) = (held.clone(), cancel.clone());
        let spawned = std::thread::Builder::new()
            .name("level-mip-prep".into())
            .spawn(move || {
                let mut order = (0..world.groups.len()).collect::<Vec<_>>();
                if let Some(position) = camera {
                    let camera = level_camera::Camera {
                        position,
                        ..Default::default()
                    };
                    order.sort_by(|&a, &b| {
                        let (a, b) = (&world.groups[a], &world.groups[b]);
                        is_ground(b).cmp(&is_ground(a)).then(
                            group_distance_sq(a, camera).total_cmp(&group_distance_sq(b, camera)),
                        )
                    });
                }
                let mut seen = HashSet::new();
                let jobs = order
                    .into_iter()
                    .filter(|&index| !world.groups[index].instances.is_empty())
                    .flat_map(|index| group_sources(&world, &world.groups[index]))
                    .filter(|source| fits(source.image, device_limit))
                    .filter(|source| seen.insert((Arc::as_ptr(source.image) as usize, source.kind)))
                    .map(|source| (source.image.clone(), source.kind))
                    .collect::<Vec<_>>();
                let next = AtomicUsize::new(0);
                std::thread::scope(|scope| {
                    for _ in 0..worker_threads() {
                        let sender = sender.clone();
                        let (jobs, next) = (&jobs, &next);
                        let (held, cancel) = (&worker_held, &worker_cancel);
                        scope.spawn(move || {
                            while let Some((image, kind)) =
                                jobs.get(next.fetch_add(1, Ordering::Relaxed))
                            {
                                while held.load(Ordering::Relaxed) > HELD_BUDGET_BYTES
                                    && !cancel.load(Ordering::Relaxed)
                                {
                                    std::thread::sleep(Duration::from_millis(4));
                                }
                                if cancel.load(Ordering::Relaxed) {
                                    return;
                                }
                                let levels = compute(image, *kind);
                                let bytes =
                                    levels.iter().map(|level| level.as_raw().len() as u64).sum();
                                held.fetch_add(bytes, Ordering::Relaxed);
                                let key = (Arc::as_ptr(image) as usize, *kind);
                                if sender.send((key, Arc::new(levels), bytes)).is_err() {
                                    return;
                                }
                            }
                        });
                    }
                });
            });
        Self {
            results: Mutex::new(results),
            ready: HashMap::new(),
            held,
            cancel,
            // Without a worker every group takes the synchronous path.
            running: spawned.is_ok(),
            device_limit,
        }
    }

    /// Collect what the workers finished since the last redraw.
    pub(super) fn receive(&mut self) {
        let Ok(results) = self.results.get_mut() else {
            self.running = false;
            return;
        };
        loop {
            match results.try_recv() {
                Ok((key, levels, bytes)) => {
                    self.ready.insert(key, (levels, bytes));
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.running = false;
                    break;
                }
            }
        }
    }

    /// Prepared levels for exactly this source image.
    pub(super) fn levels(
        &self,
        image: &Arc<image::RgbaImage>,
        kind: MipKind,
    ) -> Option<&MipLevels> {
        self.ready
            .get(&(Arc::as_ptr(image) as usize, kind))
            .map(|(levels, _)| levels)
    }

    /// Whether `group` should wait for the workers rather than resize on the
    /// render thread. Never waits once the workers stopped or paused at the
    /// memory budget, so a group is always built eventually.
    pub(super) fn waiting_for(
        &self,
        world: &World,
        group: &ModelGroup,
        mut resident: impl FnMut(&Source) -> bool,
    ) -> bool {
        self.running
            && self.held.load(Ordering::Relaxed) <= HELD_BUDGET_BYTES
            && group_sources(world, group).iter().any(|source| {
                fits(source.image, self.device_limit)
                    && self.levels(source.image, source.kind).is_none()
                    && !resident(source)
            })
    }

    /// Drop the levels `group` used (or no longer needs) once it is built.
    pub(super) fn release(&mut self, world: &World, group: &ModelGroup) {
        for source in group_sources(world, group) {
            if let Some((_, bytes)) = self
                .ready
                .remove(&(Arc::as_ptr(source.image) as usize, source.kind))
            {
                self.held.fetch_sub(bytes, Ordering::Relaxed);
            }
        }
    }
}

/// At full quality a texture uploads at its own size unless the device cannot
/// hold it; only those exact-size uploads use prepared levels.
fn fits(image: &image::RgbaImage, device_limit: u32) -> bool {
    image.width() <= device_limit && image.height() <= device_limit
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(width: u32, height: u32) -> image::RgbaImage {
        image::RgbaImage::from_fn(width, height, |x, y| {
            image::Rgba([(x * 7) as u8, (y * 13) as u8, (x ^ y) as u8, 255])
        })
    }

    #[test]
    fn prepared_levels_match_the_synchronous_upload_paths() {
        let source = gradient(64, 16);
        let chained = compute(&source, MipKind::Chained);
        assert_eq!(chained.len(), 6);
        let mut previous = source.clone();
        for level in &chained {
            let expected = chained_next(&previous);
            assert_eq!(level.as_raw(), expected.as_raw());
            previous = expected;
        }
        assert_eq!(chained.last().unwrap().dimensions(), (1, 1));
        let direct = compute(&source, MipKind::FromSource);
        for (index, level) in direct.iter().enumerate() {
            assert_eq!(
                level.as_raw(),
                next_diffuse_mip(&source, index as u32 + 1).as_raw()
            );
        }
    }

    #[test]
    fn parallel_map_keeps_item_order() {
        let items = (0..1000).collect::<Vec<u32>>();
        let doubled = parallel_map(&items, |value| value * 2);
        assert_eq!(
            doubled,
            items.iter().map(|value| value * 2).collect::<Vec<_>>()
        );
        assert!(parallel_map(&[] as &[u32], |value| *value).is_empty());
    }

    #[test]
    fn a_one_pixel_texture_has_no_smaller_levels() {
        assert!(compute(&gradient(1, 1), MipKind::Chained).is_empty());
        assert!(compute(&gradient(1, 1), MipKind::FromSource).is_empty());
    }
}
