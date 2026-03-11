//! Platform capability detection.
//!
//! Instead of scattering `cfg!(target_os = ...)` checks throughout the UI code,
//! this module describes what the current platform supports in a single struct.
//! UI code queries capabilities rather than testing OS names.

use conch_core::config::WindowDecorations;

/// Describes UI capabilities of the current platform.
#[derive(Debug, Clone)]
pub struct PlatformCapabilities {
    /// Whether the platform supports `fullsize_content_view` (content extends behind title bar).
    pub fullsize_content_view: bool,
    /// Whether the platform supports transparent window backgrounds.
    pub transparent_windows: bool,
    /// Whether the platform supports completely hiding window decorations
    /// while remaining usable (buttonless mode needs a drag region fallback).
    pub buttonless_decorations: bool,
    /// Whether a native global menu bar is available (macOS).
    pub native_global_menu: bool,
}

impl PlatformCapabilities {
    /// Detect capabilities for the current platform.
    pub fn current() -> Self {
        Self {
            fullsize_content_view: cfg!(target_os = "macos"),
            transparent_windows: cfg!(any(target_os = "macos", target_os = "linux")),
            buttonless_decorations: cfg!(target_os = "macos"),
            native_global_menu: cfg!(target_os = "macos"),
        }
    }

    /// Validate and clamp a user-chosen decoration style to what the platform
    /// actually supports, falling back to `Full` for unsupported modes.
    pub fn effective_decorations(&self, requested: WindowDecorations) -> WindowDecorations {
        match requested {
            WindowDecorations::Buttonless if !self.buttonless_decorations => {
                log::warn!("Buttonless decorations not supported on this platform, using Full");
                WindowDecorations::Full
            }
            WindowDecorations::Transparent if !self.transparent_windows => {
                log::warn!("Transparent decorations not supported on this platform, using Full");
                WindowDecorations::Full
            }
            other => other,
        }
    }
}
