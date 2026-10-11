use ::kRPC::RPCErrors;
use base64::{engine::general_purpose::STANDARD, Engine as _};

pub(crate) const MAX_IMAGE_DATA_URL_BYTES: usize = 512 * 1024;
pub(crate) const MAX_AVATAR_PX: u32 = 192;
pub(crate) const MAX_ZONE_ICON_PX: u32 = 512;

/// Avatars and icons are saved inline as small base64 PNG/JPEG/WebP data URLs, so any reader
/// that can read the owning record (including anonymous Zone visitors for the Zone icon) can load them.
pub(crate) fn validate_image_data_url(field: &str, value: &str, max_px: u32) -> Result<(), RPCErrors> {
    let invalid = |message: String| RPCErrors::ParseRequestError(format!("invalid {field}: {message}"));
    let (header, data) = value
        .strip_prefix("data:image/")
        .and_then(|rest| rest.split_once(";base64,"))
        .ok_or_else(|| invalid("expect a base64 image data URL".to_string()))?;
    if !matches!(header, "png" | "jpeg" | "webp") {
        return Err(invalid("image must be PNG, JPEG or WebP".to_string()));
    }
    if data.len() > MAX_IMAGE_DATA_URL_BYTES * 4 / 3 + 4 {
        return Err(invalid("image is too large".to_string()));
    }
    let bytes = STANDARD
        .decode(data)
        .map_err(|_| invalid("bad base64 payload".to_string()))?;
    let (width, height) = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| invalid("unknown image format".to_string()))?
        .into_dimensions()
        .map_err(|_| invalid("unreadable image".to_string()))?;
    if width > max_px || height > max_px {
        return Err(invalid(format!("image must be at most {max_px}px")));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn png_data_url(width: u32, height: u32) -> String {
    let mut bytes = Vec::new();
    image::RgbaImage::new(width, height)
        .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
        .unwrap();
    format!("data:image/png;base64,{}", STANDARD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_data_url_must_be_a_small_raster_image() {
        assert!(validate_image_data_url("avatar", &png_data_url(192, 192), MAX_AVATAR_PX).is_ok());
        assert!(validate_image_data_url("avatar", &png_data_url(193, 10), MAX_AVATAR_PX).is_err());
        assert!(validate_image_data_url("icon", &png_data_url(512, 256), MAX_ZONE_ICON_PX).is_ok());
        assert!(validate_image_data_url("avatar", "https://example.com/a.png", MAX_AVATAR_PX).is_err());
        assert!(validate_image_data_url("avatar", "data:image/png;base64,!!!", MAX_AVATAR_PX).is_err());
        let svg = format!("data:image/svg+xml;base64,{}", STANDARD.encode("<svg/>"));
        assert!(validate_image_data_url("icon", &svg, MAX_ZONE_ICON_PX).is_err());
    }
}
