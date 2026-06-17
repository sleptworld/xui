pub mod atlas;
pub mod cosmic;
pub mod render;

use cosmic_text::SwashContent;

pub use cosmic::CosmicTextEngine;
use xui_interface::widget::PType;

pub(super) fn rgba_bitmap_data(content: SwashContent, data: &[u8]) -> (Vec<u8>, PType) {
    match content {
        SwashContent::Mask => {
            let mut rgba = Vec::with_capacity(data.len() * 4);
            for alpha in data {
                rgba.extend_from_slice(&[*alpha, *alpha, *alpha, *alpha]);
            }
            (rgba, PType::Mask)
        }
        SwashContent::SubpixelMask => (data.to_vec(), PType::SubPixelMask),
        SwashContent::Color => (data.to_vec(), PType::Color),
    }
}
