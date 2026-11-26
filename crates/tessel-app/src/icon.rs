//! App window icon loading.
//!
//! Single responsibility: decode the embedded `assets/icons/tessel-icon.png`
//! into a `winit` window icon. Nothing else lives here.

use winit::window::Icon;

const APP_ICON_PNG: &[u8] = include_bytes!("../../../assets/icons/tessel-icon.png");

pub(crate) fn load_window_icon() -> Option<Icon> {
    let image = image::load_from_memory(APP_ICON_PNG)
        .map_err(|err| tracing::warn!("bundled app icon failed to decode: {err}"))
        .ok()?;
    let rgba = image.into_rgba8();
    let (width, height) = (rgba.width(), rgba.height());
    Icon::from_rgba(rgba.into_raw(), width, height)
        .map_err(|err| tracing::warn!("bundled app icon has unusable dimensions: {err}"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_icon_decodes() {
        assert!(load_window_icon().is_some());
    }
}
