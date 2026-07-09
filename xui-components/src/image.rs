use thiserror::Error;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use xui::assets::AssetId;
use xui::assets::load_image_asset;
use xui::component;
use xui::prelude::*;
use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_image::image::Image;

#[derive(Debug, Error, Clone)]
pub enum ImageError {
    #[error("network error: {0}")]
    NetworkError(String),
    #[error("decode error: {0}")]
    DecodeError(String),
    #[error("io error: {0}")]
    IoError(String),
    #[error("invalid asset path: {0}")]
    InvalidAssetPath(String),
    #[error("asset not found: {0}")]
    AssetNotFound(String),
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum ImageSrc {
    Url(String),
    Local(String),
    AssetPath(String),
    AssetId(AssetId),
}

impl From<&str> for ImageSrc {
    fn from(value: &str) -> Self {
        if value.is_empty() {
            panic!();
        }

        if value.starts_with("http://") || value.starts_with("https://") {
            return ImageSrc::Url(value.to_string());
        }

        if let Some(path) = value
            .strip_prefix("assets://")
            .or_else(|| value.strip_prefix("asset://"))
            .or_else(|| value.strip_prefix("assets:"))
            .or_else(|| value.strip_prefix("asset:"))
        {
            return ImageSrc::AssetPath(path.to_string());
        }

        if let Some(path) = value.strip_prefix("file://") {
            return ImageSrc::Local(path.to_string());
        }

        ImageSrc::Local(value.to_string())
    }
}

impl From<&String> for ImageSrc {
    fn from(value: &String) -> Self {
        value.as_str().into()
    }
}

impl From<String> for ImageSrc {
    fn from(value: String) -> Self {
        value.as_str().into()
    }
}

impl From<AssetId> for ImageSrc {
    fn from(value: AssetId) -> Self {
        ImageSrc::AssetId(value)
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Default)]
pub enum ImageSize {
    #[default]
    Auto,
    Value(Sizing),
}

impl ImageSize {
    fn apply_width(self, style: Style) -> Style {
        match self {
            ImageSize::Auto => style,
            ImageSize::Value(width) => style.width(width),
        }
    }

    fn apply_height(self, style: Style) -> Style {
        match self {
            ImageSize::Auto => style,
            ImageSize::Value(height) => style.height(height),
        }
    }
}

impl From<Sizing> for ImageSize {
    fn from(value: Sizing) -> Self {
        ImageSize::Value(value)
    }
}

impl From<f32> for ImageSize {
    fn from(value: f32) -> Self {
        ImageSize::Value(value.into())
    }
}

impl From<u32> for ImageSize {
    fn from(value: u32) -> Self {
        ImageSize::Value(value.into())
    }
}

impl From<Option<Sizing>> for ImageSize {
    fn from(value: Option<Sizing>) -> Self {
        value.map(ImageSize::Value).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct ImageOpacity(u32);

impl ImageOpacity {
    pub fn new(value: f32) -> Self {
        Self(value.clamp(0.0, 1.0).to_bits())
    }

    pub fn get(self) -> f32 {
        f32::from_bits(self.0)
    }
}

impl Default for ImageOpacity {
    fn default() -> Self {
        Self::new(1.0)
    }
}

impl From<f32> for ImageOpacity {
    fn from(value: f32) -> Self {
        Self::new(value)
    }
}

impl From<u32> for ImageOpacity {
    fn from(value: u32) -> Self {
        Self::new(value as f32)
    }
}

#[component]
#[defaults(
    alt = None,
    width = ImageSize::Auto,
    height = ImageSize::Auto,
    opacity = ImageOpacity::default(),
    image_style = ImageStyle::default(),
    fit = None,
    alignment = None,
    repeat = None,
    sampling = None,
)]
pub fn image(
    src: &ImageSrc,
    alt: &Option<String>,
    width: &ImageSize,
    height: &ImageSize,
    opacity: &ImageOpacity,
    image_style: &ImageStyle,
    fit: &Option<ImageFit>,
    alignment: &Option<Alignment>,
    repeat: &Option<ImageRepeat>,
    sampling: &Option<Sampling>,
) {
    let image_key = image_key(src);
    let child = vec![];
    let resource_src = src.clone();
    let fetch_src = resource_src.clone();
    let image_src = cx.use_resource(
        resource_src,
        move |_| async move { get_image(&fetch_src).await },
    );
    let mut style = Style::new();

    style = width.apply_width(style);
    style = height.apply_height(style);
    let image_style = resolve_image_style(*image_style, *fit, *alignment, *repeat, *sampling);

    use AsyncValue::*;

    match image_src.get() {
        Pending => placeholder(style, None),
        Ready(image_data) => ImageWidget::new()
            .image_data(image_key, image_data)
            .style(style)
            .image_style(image_style)
            .opacity(opacity.get())
            .into_element_desc(child),

        Error(e) => {
            eprintln!("{}", e);
            placeholder(style, alt.as_deref())
        }
    }
}

fn resolve_image_style(
    mut image_style: ImageStyle,
    fit: Option<ImageFit>,
    alignment: Option<Alignment>,
    repeat: Option<ImageRepeat>,
    sampling: Option<Sampling>,
) -> ImageStyle {
    if let Some(fit) = fit {
        image_style.fit = fit;
    }
    if let Some(alignment) = alignment {
        image_style.alignment = alignment;
    }
    if let Some(repeat) = repeat {
        image_style.repeat = repeat;
    }
    if let Some(sampling) = sampling {
        image_style.sampling = sampling;
    }
    image_style
}

fn placeholder(style: Style, alt: Option<&str>) -> ElementDesc {
    match alt.filter(|text| !text.is_empty()) {
        Some(text) => TextWidget::new(text.to_string())
            .style(style)
            .into_element_desc(),
        None => ContainerWidget::new()
            .style(style)
            .into_element_desc(Vec::new()),
    }
}

async fn get_network_image(path: &str) -> Result<ImageData, ImageError> {
    let resp = reqwest::get(path)
        .await
        .map_err(|e| ImageError::NetworkError(e.to_string()))?
        .error_for_status()
        .map_err(|e| ImageError::NetworkError(e.to_string()))?;
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| ImageError::NetworkError(e.to_string()))?
        .to_vec();
    decode_image(&bytes).map_err(|e| ImageError::DecodeError(e.to_string()))
}

async fn get_local_image(path: &str) -> Result<ImageData, ImageError> {
    let mut file = File::open(path)
        .await
        .map_err(|e| ImageError::IoError(e.to_string()))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .await
        .map_err(|e| ImageError::IoError(e.to_string()))?;
    decode_image(&buf).map_err(|e| ImageError::DecodeError(e.to_string()))
}

async fn get_image(src: &ImageSrc) -> Result<ImageData, ImageError> {
    match src {
        ImageSrc::Url(url) => get_network_image(url).await,
        ImageSrc::Local(path) => get_local_image(path).await,
        ImageSrc::AssetPath(path) => get_asset_path_image(path),
        ImageSrc::AssetId(id) => get_asset_image(*id),
    }
}

fn get_asset_path_image(path: &str) -> Result<ImageData, ImageError> {
    let id =
        AssetId::from_path(path).map_err(|_| ImageError::InvalidAssetPath(path.to_string()))?;
    get_asset_image(id).map_err(|error| match error {
        ImageError::AssetNotFound(_) => ImageError::AssetNotFound(path.to_string()),
        other => other,
    })
}

fn get_asset_image(id: AssetId) -> Result<ImageData, ImageError> {
    load_image_asset(id).ok_or_else(|| ImageError::AssetNotFound(format!("{:?}", id)))
}

fn image_key(src: &ImageSrc) -> ImageKey {
    match src {
        ImageSrc::AssetId(id) => ImageKey::AssetId(*id.as_bytes()),
        ImageSrc::AssetPath(path) => ImageKey::AssetPath(path.into()),
        ImageSrc::Url(url) => ImageKey::Url(url.clone()),
        ImageSrc::Local(path) => ImageKey::AssetPath(path.into()),
    }
}

fn decode_image(bytes: impl AsRef<[u8]>) -> anyhow::Result<ImageData> {
    let cursor = ZCursor::new(bytes.as_ref());
    let mut image = Image::read(cursor, DecoderOptions::default())?;
    image.convert_color(ColorSpace::RGBA)?;
    let (width, height) = image.dimensions();
    let width = u32::try_from(width)?;
    let height = u32::try_from(height)?;
    let pixels = image
        .flatten_to_u8()
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("decoded image has no pixel buffer"))?;
    Ok(ImageData::rgba8(Size::new(width, height), pixels))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_src_from_str_uses_protocol() {
        assert_eq!(
            ImageSrc::from("https://example.com/demo.png"),
            ImageSrc::Url("https://example.com/demo.png".to_string())
        );
        assert_eq!(
            ImageSrc::from("http://example.com/demo.png"),
            ImageSrc::Url("http://example.com/demo.png".to_string())
        );
        assert_eq!(
            ImageSrc::from("assets://images/demo.png"),
            ImageSrc::AssetPath("images/demo.png".to_string())
        );
        assert_eq!(
            ImageSrc::from("asset://images/demo.png"),
            ImageSrc::AssetPath("images/demo.png".to_string())
        );
        assert_eq!(
            ImageSrc::from("assets:images/demo.png"),
            ImageSrc::AssetPath("images/demo.png".to_string())
        );
        assert_eq!(
            ImageSrc::from("file:///tmp/demo.png"),
            ImageSrc::Local("/tmp/demo.png".to_string())
        );
        assert_eq!(
            ImageSrc::from("images/demo.png"),
            ImageSrc::Local("images/demo.png".to_string())
        );
    }

    #[test]
    fn decode_image_returns_rgba_pixels() {
        let source = Image::from_u8(&[1, 2, 3, 255, 4, 5, 6, 255], 2, 1, ColorSpace::RGBA);
        let png = source
            .write_to_vec(zune_image::codecs::ImageFormat::PNG)
            .unwrap();

        let decoded = decode_image(png).unwrap();

        assert_eq!(decoded.size, Size::new(2, 1));
        assert_eq!(decoded.pixels.len(), 8);
        assert_eq!(decoded.pixels.as_ref(), &[1, 2, 3, 255, 4, 5, 6, 255]);
    }

    #[test]
    fn image_size_converts_common_values() {
        assert_eq!(
            ImageSize::from(12u32),
            ImageSize::Value(Sizing::from(12u32))
        );
        assert_eq!(ImageSize::from(12.0), ImageSize::Value(Sizing::from(12.0)));
        assert_eq!(
            ImageSize::from(Sizing::Fill),
            ImageSize::Value(Sizing::Fill)
        );
        assert_eq!(ImageSize::from(None::<Sizing>), ImageSize::Auto);
    }

    #[test]
    fn opacity_clamps_to_valid_range() {
        assert_eq!(ImageOpacity::from(-1.0).get(), 0.0);
        assert_eq!(ImageOpacity::from(0.5).get(), 0.5);
        assert_eq!(ImageOpacity::from(2.0).get(), 1.0);
    }

    #[test]
    fn image_style_defaults_to_widget_defaults() {
        assert_eq!(
            resolve_image_style(ImageStyle::default(), None, None, None, None),
            ImageStyle::default()
        );
    }

    #[test]
    fn image_style_shortcuts_override_base_style() {
        let style = resolve_image_style(
            ImageStyle {
                fit: ImageFit::Contain,
                alignment: Alignment::CENTER,
                repeat: ImageRepeat::NoRepeat,
                sampling: Sampling::Linear,
            },
            Some(ImageFit::Cover),
            Some(Alignment::START),
            Some(ImageRepeat::RepeatY),
            Some(Sampling::Nearest),
        );

        assert_eq!(
            style,
            ImageStyle {
                fit: ImageFit::Cover,
                alignment: Alignment::START,
                repeat: ImageRepeat::RepeatY,
                sampling: Sampling::Nearest,
            }
        );
    }

    #[test]
    fn invalid_asset_path_reports_path_error() {
        let error = get_asset_path_image("../bad.png").unwrap_err();

        assert!(matches!(error, ImageError::InvalidAssetPath(path) if path == "../bad.png"));
    }
}
