pub mod cosmic;

use cosmic_text::SwashContent;

pub use cosmic::CosmicTextEngine;

pub(crate) fn rgba_bitmap_data(content: SwashContent, data: &[u8]) -> Vec<u8> {
    match content {
        SwashContent::Mask => {
            let mut rgba = Vec::with_capacity(data.len() * 4);
            for alpha in data {
                rgba.extend_from_slice(&[*alpha, *alpha, *alpha, *alpha]);
            }
            rgba
        }
        SwashContent::Color | SwashContent::SubpixelMask => data.to_vec(),
    }
}
