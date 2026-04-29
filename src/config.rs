//! TOML configuration loaded from `--config <PATH>` or
//! `~/.config/lazylog/config.toml` by default.
//!
//! ```toml
//! [theme]
//! scheme = "dark"      # or "light"
//!
//! [theme.level_colors]
//! ERROR = "#ff5555"
//! WARN  = "#ffaa00"
//! INFO  = "#55aa55"
//! ```

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub theme: ThemeConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeConfig {
    /// Map level name -> hex color string e.g. "ERROR" -> "#ff5555".
    #[serde(default)]
    pub level_colors: HashMap<String, String>,
    /// General UI color scheme: "dark" (default) or "light".
    #[serde(default = "default_scheme")]
    pub scheme: String,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            level_colors: HashMap::new(),
            scheme: default_scheme(),
        }
    }
}

fn default_scheme() -> String {
    "dark".to_string()
}

impl AppConfig {
    /// Load from `path`, otherwise the default `<config_dir>/lazylog/config.toml`.
    /// Missing or unreadable files yield `Self::default()` — never an error.
    pub fn load(path: Option<&std::path::Path>) -> Self {
        let resolved = path.map(|p| p.to_path_buf()).or_else(|| {
            dirs::config_dir().map(|d| d.join("lazylog").join("config.toml"))
        });
        let Some(path) = resolved else { return Self::default() };
        let Ok(text) = std::fs::read_to_string(&path) else { return Self::default() };
        toml::from_str(&text).unwrap_or_default()
    }

    /// Resolve a level name (e.g. "ERROR") to a `ratatui::style::Color` from
    /// the configured palette, falling back to `default_color` if unset or invalid.
    pub fn level_color(
        &self,
        name: &str,
        default_color: ratatui::style::Color,
    ) -> ratatui::style::Color {
        self.theme
            .level_colors
            .get(name)
            .and_then(|hex| parse_hex_color(hex))
            .unwrap_or(default_color)
    }
}

/// Parse "#rrggbb" or "rrggbb" into `Color::Rgb`.
fn parse_hex_color(s: &str) -> Option<ratatui::style::Color> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some(ratatui::style::Color::Rgb(r, g, b))
}
