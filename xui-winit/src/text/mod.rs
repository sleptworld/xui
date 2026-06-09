pub mod atlas;
pub mod cosmic;
pub mod render;

use cosmic_text::SwashContent;

pub use cosmic::CosmicTextEngine;

pub(crate) fn rgba_bitmap_data(content: SwashContent, data: &[u8]) -> (Vec<u8>, u32) {
    match content {
        SwashContent::Mask => {
            let mut rgba = Vec::with_capacity(data.len() * 4);
            for alpha in data {
                rgba.extend_from_slice(&[*alpha, *alpha, *alpha, *alpha]);
            }
            (rgba, 0)
        }
        SwashContent::SubpixelMask => (data.to_vec(), 1),
        SwashContent::Color => (data.to_vec(), 2),
    }
}
