//! Fallback static file serving for themes.
//!
//! Provides static file serving with theme fallback support. When serving static
//! files, the active theme's directory is tried first; if the file is not found,
//! the default theme's directory is used as a fallback.

use tower_http::services::ServeDir;

use crate::config::ThemeConfig;

/// Create a static file service with theme fallback.
///
/// Returns a `ServeDir` service that:
/// 1. First tries to serve files from the active theme's static directory
/// 2. Falls back to the default theme's static directory if not found
///
/// If the active theme is "default", no fallback is needed and files are served
/// directly from the default theme's static directory.
pub fn create_static_service(theme: &ThemeConfig) -> ServeDir<ServeDir> {
    let default_static = theme.static_path("default");

    if theme.name == "default" {
        // No fallback needed - serve directly from default theme
        // We still wrap in ServeDir to maintain consistent return type
        ServeDir::new(&default_static).fallback(ServeDir::new(&default_static))
    } else {
        // Active theme with fallback to default
        let theme_static = theme.static_path(&theme.name);
        ServeDir::new(theme_static).fallback(ServeDir::new(default_static))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_static_service_default_theme() {
        let theme = ThemeConfig {
            name: "default".to_string(),
            themes_dir: "/etc/september/themes".to_string(),
        };
        // Just verify it doesn't panic - actual file serving tested in integration
        let _service = create_static_service(&theme);
    }

    #[test]
    fn test_create_static_service_custom_theme() {
        let theme = ThemeConfig {
            name: "dark".to_string(),
            themes_dir: "/etc/september/themes".to_string(),
        };
        let _service = create_static_service(&theme);
    }
}
