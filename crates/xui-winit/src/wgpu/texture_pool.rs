use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::{Arc, Mutex, Weak},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TexturePoolOptions {
    pub soft_budget_bytes: u64,
    pub hard_budget_bytes: u64,
    pub dimension_bucket: u32,
}

impl Default for TexturePoolOptions {
    fn default() -> Self {
        Self {
            soft_budget_bytes: 192 * 1024 * 1024,
            hard_budget_bytes: 256 * 1024 * 1024,
            dimension_bucket: 64,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureRequest {
    pub extent: wgpu::Extent3d,
    pub format: wgpu::TextureFormat,
    pub sample_count: u32,
    pub usage: wgpu::TextureUsages,
    pub label: &'static str,
}

impl TextureRequest {
    /// A linear scene-color texture suitable for render-graph intermediates.
    pub fn scene(extent: (u32, u32), label: &'static str) -> Self {
        Self {
            extent: wgpu::Extent3d {
                width: extent.0,
                height: extent.1,
                depth_or_array_layers: 1,
            },
            format: super::SCENE_FORMAT,
            sample_count: 1,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            label,
        }
    }

    /// A persistent, fixed-allocation scene tile. Edge tiles keep the same
    /// allocation extent and constrain their valid region with viewport/scissor.
    pub fn tile(tile_size: u32, label: &'static str) -> Self {
        Self::scene((tile_size.max(1), tile_size.max(1)), label)
    }

    /// The one multisampled tile target shared serially by all surface jobs.
    pub fn tile_msaa(tile_size: u32, label: &'static str) -> Self {
        Self {
            extent: wgpu::Extent3d {
                width: tile_size.max(1),
                height: tile_size.max(1),
                depth_or_array_layers: 1,
            },
            format: super::SCENE_FORMAT,
            sample_count: super::SCENE_SAMPLE_COUNT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            label,
        }
    }

    /// A cross-frame scene snapshot. Its usage flags support both rendering
    /// and GPU copies; holding the resulting lease pins it for the transition.
    pub fn transition(extent: (u32, u32), label: &'static str) -> Self {
        Self::scene(extent, label)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TexturePoolStats {
    pub resident_bytes: u64,
    pub active_bytes: u64,
    pub free_bytes: u64,
    pub active_textures: usize,
    pub free_textures: usize,
    pub allocations: u64,
    pub reuses: u64,
    pub evictions: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TexturePoolError {
    #[error("texture pool cannot allocate resources for a different wgpu device")]
    ForeignDevice,
    #[error("texture sample count must be greater than zero")]
    InvalidSampleCount,
    #[error("texture usage must not be empty")]
    EmptyUsage,
    #[error(
        "texture extent {width}x{height}x{layers} exceeds device limits (2D dimension {max_dimension_2d}, array layers {max_array_layers})"
    )]
    DeviceLimit {
        width: u32,
        height: u32,
        layers: u32,
        max_dimension_2d: u32,
        max_array_layers: u32,
    },
    #[error(
        "cannot allocate {requested_bytes} texture bytes with {resident_bytes} bytes resident under the {hard_budget_bytes}-byte hard budget"
    )]
    HardBudgetExceeded {
        requested_bytes: u64,
        resident_bytes: u64,
        hard_budget_bytes: u64,
    },
    #[error("texture dimensions overflow while applying the allocation bucket")]
    DimensionOverflow,
    #[error("cannot estimate storage for texture format {0:?}")]
    UnsupportedFormat(wgpu::TextureFormat),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TextureClass {
    format: wgpu::TextureFormat,
    sample_count: u32,
    usage: wgpu::TextureUsages,
    depth_or_array_layers: u32,
}

struct PoolTexture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    class: TextureClass,
    extent: wgpu::Extent3d,
    bytes: u64,
    last_used: u64,
}

#[derive(Default)]
struct TexturePoolState {
    free: Vec<PoolTexture>,
    resident_bytes: u64,
    active_bytes: u64,
    active_textures: usize,
    clock: u64,
    allocations: u64,
    reuses: u64,
    evictions: u64,
}

struct TexturePoolShared {
    device_identity: u64,
    options: TexturePoolOptions,
    state: Mutex<TexturePoolState>,
}

#[derive(Clone)]
pub struct TexturePool {
    shared: Arc<TexturePoolShared>,
}

impl TexturePool {
    pub fn new(device: &wgpu::Device, mut options: TexturePoolOptions) -> Self {
        options.dimension_bucket = options.dimension_bucket.max(1);
        options.hard_budget_bytes = options
            .hard_budget_bytes
            .max(options.soft_budget_bytes)
            .max(1);
        Self {
            shared: Arc::new(TexturePoolShared {
                device_identity: device_identity(device),
                options,
                state: Mutex::new(TexturePoolState::default()),
            }),
        }
    }

    pub fn options(&self) -> TexturePoolOptions {
        self.shared.options
    }

    pub fn acquire(
        &self,
        device: &wgpu::Device,
        request: TextureRequest,
    ) -> Result<TextureLease, TexturePoolError> {
        if device_identity(device) != self.shared.device_identity {
            return Err(TexturePoolError::ForeignDevice);
        }
        if request.sample_count == 0 {
            return Err(TexturePoolError::InvalidSampleCount);
        }
        if request.usage.is_empty() {
            return Err(TexturePoolError::EmptyUsage);
        }
        let requested_extent = wgpu::Extent3d {
            width: request.extent.width.max(1),
            height: request.extent.height.max(1),
            depth_or_array_layers: request.extent.depth_or_array_layers.max(1),
        };
        let limits = device.limits();
        let extent = bucket_extent(
            request.extent,
            self.shared.options.dimension_bucket,
            limits.max_texture_dimension_2d,
            limits.max_texture_array_layers,
        )?;
        let class = TextureClass {
            format: request.format,
            sample_count: request.sample_count,
            usage: request.usage,
            depth_or_array_layers: extent.depth_or_array_layers,
        };
        let bytes = estimate_texture_bytes(request.format, extent, class.sample_count)?;
        if bytes > self.shared.options.hard_budget_bytes {
            return Err(TexturePoolError::HardBudgetExceeded {
                requested_bytes: bytes,
                resident_bytes: self.stats().resident_bytes,
                hard_budget_bytes: self.shared.options.hard_budget_bytes,
            });
        }

        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.clock = state.clock.wrapping_add(1).max(1);
        let now = state.clock;
        if let Some(index) = best_fit(&state.free, class, extent) {
            let mut texture = state.free.swap_remove(index);
            texture.last_used = now;
            state.active_bytes = state.active_bytes.saturating_add(texture.bytes);
            state.active_textures += 1;
            state.reuses += 1;
            return Ok(TextureLease {
                texture: Some(texture),
                pool: Arc::downgrade(&self.shared),
                requested_extent,
            });
        }

        evict_until_fits(&mut state, bytes, self.shared.options.hard_budget_bytes);
        if state.resident_bytes.saturating_add(bytes) > self.shared.options.hard_budget_bytes {
            return Err(TexturePoolError::HardBudgetExceeded {
                requested_bytes: bytes,
                resident_bytes: state.resident_bytes,
                hard_budget_bytes: self.shared.options.hard_budget_bytes,
            });
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(request.label),
            size: extent,
            mip_level_count: 1,
            sample_count: class.sample_count,
            dimension: wgpu::TextureDimension::D2,
            format: request.format,
            usage: request.usage,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        state.resident_bytes = state.resident_bytes.saturating_add(bytes);
        state.active_bytes = state.active_bytes.saturating_add(bytes);
        state.active_textures += 1;
        state.allocations += 1;
        Ok(TextureLease {
            texture: Some(PoolTexture {
                texture,
                view,
                class,
                extent,
                bytes,
                last_used: now,
            }),
            pool: Arc::downgrade(&self.shared),
            requested_extent,
        })
    }

    pub fn trim(&self) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        trim_free_to_budget(&mut state, self.shared.options.soft_budget_bytes);
    }

    pub fn clear_free(&self) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        while !state.free.is_empty() {
            evict_oldest(&mut state);
        }
    }

    pub fn stats(&self) -> TexturePoolStats {
        let state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        TexturePoolStats {
            resident_bytes: state.resident_bytes,
            active_bytes: state.active_bytes,
            free_bytes: state.resident_bytes.saturating_sub(state.active_bytes),
            active_textures: state.active_textures,
            free_textures: state.free.len(),
            allocations: state.allocations,
            reuses: state.reuses,
            evictions: state.evictions,
        }
    }
}

/// Exclusive physical texture ownership. Keeping this value across frames pins
/// the allocation, which is the intended ownership model for transition snapshots.
#[must_use = "dropping the lease immediately returns its texture to the pool"]
pub struct TextureLease {
    texture: Option<PoolTexture>,
    pool: Weak<TexturePoolShared>,
    requested_extent: wgpu::Extent3d,
}

impl TextureLease {
    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture.as_ref().expect("live texture lease").texture
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.texture.as_ref().expect("live texture lease").view
    }

    pub fn allocation_extent(&self) -> wgpu::Extent3d {
        self.texture.as_ref().expect("live texture lease").extent
    }

    pub fn requested_extent(&self) -> wgpu::Extent3d {
        self.requested_extent
    }

    pub fn bytes(&self) -> u64 {
        self.texture.as_ref().expect("live texture lease").bytes
    }
}

impl Drop for TextureLease {
    fn drop(&mut self) {
        let Some(mut texture) = self.texture.take() else {
            return;
        };
        let Some(pool) = self.pool.upgrade() else {
            return;
        };
        let mut state = pool.state.lock().unwrap_or_else(|error| error.into_inner());
        state.clock = state.clock.wrapping_add(1).max(1);
        texture.last_used = state.clock;
        state.active_bytes = state.active_bytes.saturating_sub(texture.bytes);
        state.active_textures = state.active_textures.saturating_sub(1);
        state.free.push(texture);
        trim_free_to_budget(&mut state, pool.options.soft_budget_bytes);
    }
}

fn bucket_extent(
    mut extent: wgpu::Extent3d,
    bucket: u32,
    max_dimension_2d: u32,
    max_array_layers: u32,
) -> Result<wgpu::Extent3d, TexturePoolError> {
    extent.width = bucket_dimension(extent.width.max(1), bucket, max_dimension_2d)?;
    extent.height = bucket_dimension(extent.height.max(1), bucket, max_dimension_2d)?;
    extent.depth_or_array_layers = extent.depth_or_array_layers.max(1);
    if extent.width > max_dimension_2d
        || extent.height > max_dimension_2d
        || extent.depth_or_array_layers > max_array_layers
    {
        return Err(TexturePoolError::DeviceLimit {
            width: extent.width,
            height: extent.height,
            layers: extent.depth_or_array_layers,
            max_dimension_2d,
            max_array_layers,
        });
    }
    Ok(extent)
}

fn bucket_dimension(value: u32, bucket: u32, limit: u32) -> Result<u32, TexturePoolError> {
    if value > limit {
        return Ok(value);
    }
    let rounded = value
        .checked_add(bucket - 1)
        .ok_or(TexturePoolError::DimensionOverflow)?
        / bucket
        * bucket;
    Ok(if rounded > limit { value } else { rounded })
}

fn estimate_texture_bytes(
    format: wgpu::TextureFormat,
    extent: wgpu::Extent3d,
    sample_count: u32,
) -> Result<u64, TexturePoolError> {
    let block_size = format
        .block_copy_size(None)
        .or_else(|| format.target_pixel_byte_cost())
        .ok_or(TexturePoolError::UnsupportedFormat(format))? as u64;
    let (block_width, block_height) = format.block_dimensions();
    let width_blocks = extent.width.div_ceil(block_width) as u64;
    let height_blocks = extent.height.div_ceil(block_height) as u64;
    Ok(width_blocks
        .saturating_mul(height_blocks)
        .saturating_mul(extent.depth_or_array_layers as u64)
        .saturating_mul(block_size)
        .saturating_mul(sample_count.max(1) as u64))
}

fn best_fit(free: &[PoolTexture], class: TextureClass, extent: wgpu::Extent3d) -> Option<usize> {
    free.iter()
        .enumerate()
        .filter(|(_, texture)| {
            compatible_class(texture.class, class)
                && texture.extent.width >= extent.width
                && texture.extent.height >= extent.height
        })
        .min_by_key(|(_, texture)| texture.extent.width as u64 * texture.extent.height as u64)
        .map(|(index, _)| index)
}

fn compatible_class(actual: TextureClass, requested: TextureClass) -> bool {
    actual.format == requested.format
        && actual.sample_count == requested.sample_count
        && actual.depth_or_array_layers == requested.depth_or_array_layers
        && actual.usage.contains(requested.usage)
}

fn device_identity(device: &wgpu::Device) -> u64 {
    let mut hasher = DefaultHasher::new();
    device.hash(&mut hasher);
    hasher.finish()
}

fn evict_until_fits(state: &mut TexturePoolState, bytes: u64, hard_budget: u64) {
    while state.resident_bytes.saturating_add(bytes) > hard_budget && !state.free.is_empty() {
        evict_oldest(state);
    }
}

fn trim_free_to_budget(state: &mut TexturePoolState, soft_budget: u64) {
    while state.resident_bytes > soft_budget && !state.free.is_empty() {
        evict_oldest(state);
    }
}

fn evict_oldest(state: &mut TexturePoolState) {
    let Some((index, _)) = state
        .free
        .iter()
        .enumerate()
        .min_by_key(|(_, texture)| texture.last_used)
    else {
        return;
    };
    let texture = state.free.swap_remove(index);
    state.resident_bytes = state.resident_bytes.saturating_sub(texture.bytes);
    state.evictions += 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimensions_are_bucketed_without_crossing_device_limit() {
        assert_eq!(bucket_dimension(1, 64, 1024), Ok(64));
        assert_eq!(bucket_dimension(65, 64, 1024), Ok(128));
        assert_eq!(bucket_dimension(1023, 64, 1024), Ok(1024));
        assert_eq!(bucket_dimension(1025, 64, 1024), Ok(1025));
    }

    #[test]
    fn rgba16_float_accounting_includes_samples_and_layers() {
        assert_eq!(
            estimate_texture_bytes(
                wgpu::TextureFormat::Rgba16Float,
                wgpu::Extent3d {
                    width: 10,
                    height: 20,
                    depth_or_array_layers: 2,
                },
                4,
            ),
            Ok(10 * 20 * 2 * 4 * 8)
        );
    }

    #[test]
    fn usage_supersets_are_reusable_but_other_classes_are_not() {
        let actual = TextureClass {
            format: wgpu::TextureFormat::Rgba16Float,
            sample_count: 1,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_DST,
            depth_or_array_layers: 1,
        };
        let requested = TextureClass {
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            ..actual
        };
        assert!(compatible_class(actual, requested));
        assert!(!compatible_class(requested, actual));
        assert!(!compatible_class(
            actual,
            TextureClass {
                sample_count: 4,
                ..requested
            }
        ));
    }

    #[test]
    fn array_layers_use_their_own_device_limit() {
        assert!(matches!(
            bucket_extent(
                wgpu::Extent3d {
                    width: 32,
                    height: 32,
                    depth_or_array_layers: 9,
                },
                64,
                4096,
                8,
            ),
            Err(TexturePoolError::DeviceLimit { .. })
        ));
    }
}
