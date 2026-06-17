use crate::ImageKey;
use image::ImageBuffer;

pub struct ImageProviderResult {
    pub image: Option<Vec<u8>>,
}

pub trait ImageProvider {
    async fn get_image(&self, key: &ImageKey) -> Option<ImageProviderResult>;
}
