use gpui_kit::{AssetSource, Result, SharedString};
use std::borrow::Cow;

/// Embedded asset key for the full-bleed high-DPI app icon rendered inside the GPUI UI.
pub const APP_ICON_ASSET: &str = "app-icon.png";

static APP_ICON_PNG: &[u8] = include_bytes!("../../../assets/icons/128x128@2x.png");

#[cfg(target_os = "macos")]
static MACOS_DOCK_ICON_PNG: &[u8] = include_bytes!("../../../assets/icons/icon-macos.png");

/// Composite asset source serving the custom application icon alongside all built-in
/// `gpui-kit` Lucide icons.
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == APP_ICON_ASSET {
            return Ok(Some(Cow::Borrowed(APP_ICON_PNG)));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut items = gpui_kit::assets::Assets.list(path)?;
        if APP_ICON_ASSET.starts_with(path) {
            items.push(SharedString::from(APP_ICON_ASSET));
        }
        Ok(items)
    }
}

/// Apply the custom macOS Dock / Application Switcher icon at runtime so launching via
/// `cargo run` or a standalone binary shows the real icon even outside an `.app` bundle.
#[cfg(target_os = "macos")]
pub fn install_platform_app_icon() {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(MACOS_DOCK_ICON_PNG);
    if let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) {
        let app = NSApplication::sharedApplication(mtm);
        // SAFETY: `image` is a valid, retained NSImage initialized on the main thread.
        unsafe {
            app.setApplicationIconImage(Some(&image));
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn install_platform_app_icon() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_app_icon_is_valid_png() {
        let bytes = AppAssets
            .load(APP_ICON_ASSET)
            .expect("load app icon asset")
            .expect("app icon bytes present");
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(bytes.len() > 1000);

        let listed = AppAssets.list("app-").expect("list assets");
        assert!(listed.iter().any(|p| p.as_ref() == APP_ICON_ASSET));
    }

    #[test]
    fn delegates_standard_lucide_icons() {
        let icon = AppAssets
            .load("icons/settings.svg")
            .expect("load bundled lucide icon")
            .expect("settings icon bytes present");
        assert!(!icon.is_empty());
    }
}
