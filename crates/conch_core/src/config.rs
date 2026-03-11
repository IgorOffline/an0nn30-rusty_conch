//! Configuration and persistent state management.
//!
//! Split into two files:
//! - `config.toml` — terminal + appearance prefs (Alacritty-compatible + [conch.*] extensions)
//! - `state.toml` — ephemeral UI state (not user-edited)

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// UserConfig — ~/.config/conch/config.toml
// ---------------------------------------------------------------------------

/// User preferences (portable, version-controlled).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserConfig {
    #[serde(default)]
    pub window: WindowConfig,
    #[serde(default)]
    pub font: FontConfig,
    #[serde(default)]
    pub colors: ColorsConfig,
    #[serde(default)]
    pub terminal: TerminalConfig,
    #[serde(default)]
    pub conch: ConchConfig,
}

// ---------------------------------------------------------------------------
// Window config — [window] / [window.dimensions]
// ---------------------------------------------------------------------------

/// Window decoration style (mirrors Alacritty `window.decorations`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub enum WindowDecorations {
    #[default]
    Full,
    Transparent,
    Buttonless,
    None,
}

impl<'de> serde::Deserialize<'de> for WindowDecorations {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        match s.to_lowercase().as_str() {
            "full" => Ok(Self::Full),
            "transparent" => Ok(Self::Transparent),
            "buttonless" => Ok(Self::Buttonless),
            "none" => Ok(Self::None),
            _ => Err(serde::de::Error::unknown_variant(
                &s,
                &["Full", "Transparent", "Buttonless", "None"],
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowConfig {
    #[serde(default)]
    pub dimensions: WindowDimensions,
    #[serde(default)]
    pub decorations: WindowDecorations,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowDimensions {
    #[serde(default = "default_columns")]
    pub columns: u16,
    #[serde(default = "default_lines")]
    pub lines: u16,
}

fn default_columns() -> u16 { 150 }
fn default_lines() -> u16 { 50 }

impl Default for WindowDimensions {
    fn default() -> Self {
        Self {
            columns: default_columns(),
            lines: default_lines(),
        }
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            dimensions: WindowDimensions::default(),
            decorations: WindowDecorations::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Terminal config — [terminal] / [terminal.shell]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerminalConfig {
    #[serde(default)]
    pub shell: TerminalShell,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub cursor: CursorConfig,
    #[serde(default = "default_scroll_sensitivity")]
    pub scroll_sensitivity: f32,
}

fn default_scroll_sensitivity() -> f32 {
    0.15
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CursorConfig {
    #[serde(default)]
    pub style: CursorStyleConfig,
    #[serde(default)]
    pub vi_mode_style: Option<CursorStyleConfig>,
}

impl Default for CursorConfig {
    fn default() -> Self {
        Self {
            style: CursorStyleConfig::default(),
            vi_mode_style: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CursorStyleConfig {
    #[serde(default = "default_cursor_shape")]
    pub shape: String,
    #[serde(default = "default_true", deserialize_with = "deserialize_blinking")]
    pub blinking: bool,
}

fn default_cursor_shape() -> String {
    "Block".to_owned()
}

fn deserialize_blinking<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    use serde::de;

    struct BlinkingVisitor;

    impl<'de> de::Visitor<'de> for BlinkingVisitor {
        type Value = bool;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a boolean or one of \"Never\", \"Off\", \"On\", \"Always\"")
        }

        fn visit_bool<E: de::Error>(self, v: bool) -> Result<bool, E> {
            Ok(v)
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<bool, E> {
            match v.to_lowercase().as_str() {
                "always" | "on" => Ok(true),
                "never" | "off" => Ok(false),
                _ => Err(de::Error::unknown_variant(v, &["Never", "Off", "On", "Always"])),
            }
        }
    }

    deserializer.deserialize_any(BlinkingVisitor)
}

impl Default for CursorStyleConfig {
    fn default() -> Self {
        Self {
            shape: default_cursor_shape(),
            blinking: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerminalShell {
    #[serde(default)]
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub startup_command: String,
    #[serde(default)]
    pub use_tmux: bool,
}

impl Default for TerminalShell {
    fn default() -> Self {
        Self {
            program: String::new(),
            args: Vec::new(),
            startup_command: String::new(),
            use_tmux: false,
        }
    }
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            shell: TerminalShell::default(),
            env: std::collections::HashMap::new(),
            cursor: CursorConfig::default(),
            scroll_sensitivity: default_scroll_sensitivity(),
        }
    }
}

// ---------------------------------------------------------------------------
// Conch-specific config — [conch.keyboard] / [conch.ui]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConchConfig {
    #[serde(default)]
    pub keyboard: KeyboardConfig,
    #[serde(default)]
    pub ui: UiConfig,
}

impl Default for ConchConfig {
    fn default() -> Self {
        Self {
            keyboard: KeyboardConfig::default(),
            ui: UiConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default)]
    pub font_family: String,
    #[serde(default = "default_ui_size")]
    pub font_size: f32,
    #[serde(default)]
    pub native_menu_bar: bool,
}

fn default_true() -> bool {
    true
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            font_family: String::new(),
            font_size: default_ui_size(),
            native_menu_bar: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontConfig {
    #[serde(default)]
    pub normal: FontFamily,
    #[serde(default = "default_font_size")]
    pub size: f32,
    #[serde(default)]
    pub offset: FontOffset,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FontOffset {
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
}

impl Default for FontOffset {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontFamily {
    #[serde(default = "default_font_name")]
    pub family: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorsConfig {
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_appearance_mode")]
    pub appearance_mode: String,
}

/// Keyboard shortcuts configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyboardConfig {
    #[serde(default = "default_new_tab")]
    pub new_tab: String,
    #[serde(default = "default_close_tab")]
    pub close_tab: String,
    #[serde(default = "default_quit")]
    pub quit: String,
    #[serde(default = "default_new_window")]
    pub new_window: String,
    #[serde(default = "default_zen_mode")]
    pub zen_mode: String,
}

fn default_theme() -> String { "dracula".into() }
fn default_appearance_mode() -> String { "dark".into() }
fn default_font_size() -> f32 { 14.0 }
fn default_font_name() -> String { "JetBrains Mono".into() }
fn default_ui_size() -> f32 { 13.0 }
fn default_new_tab() -> String { "cmd+t".into() }
fn default_close_tab() -> String { "cmd+w".into() }
fn default_quit() -> String { "cmd+q".into() }
fn default_new_window() -> String { "cmd+shift+n".into() }
fn default_zen_mode() -> String { "cmd+shift+z".into() }

impl Default for FontFamily {
    fn default() -> Self { Self { family: default_font_name() } }
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            normal: FontFamily::default(),
            size: default_font_size(),
            offset: FontOffset::default(),
        }
    }
}

impl Default for ColorsConfig {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            appearance_mode: default_appearance_mode(),
        }
    }
}

impl Default for KeyboardConfig {
    fn default() -> Self {
        Self {
            new_tab: default_new_tab(),
            close_tab: default_close_tab(),
            quit: default_quit(),
            new_window: default_new_window(),
            zen_mode: default_zen_mode(),
        }
    }
}

impl Default for UserConfig {
    fn default() -> Self {
        Self {
            window: WindowConfig::default(),
            font: FontConfig::default(),
            colors: ColorsConfig::default(),
            terminal: TerminalConfig::default(),
            conch: ConchConfig::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// PersistentState — ~/.config/conch/state.toml
// ---------------------------------------------------------------------------

/// Machine-local UI state (not version-controlled).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentState {
    #[serde(default)]
    pub layout: LayoutConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutConfig {
    /// Persisted window width in logical points (0 = use config default).
    #[serde(default)]
    pub window_width: f32,
    /// Persisted window height in logical points (0 = use config default).
    #[serde(default)]
    pub window_height: f32,
    /// Persisted UI zoom factor (0 or 1.0 = default).
    #[serde(default = "default_zoom")]
    pub zoom_factor: f32,
}

fn default_zoom() -> f32 { 1.0 }

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            window_width: 0.0,
            window_height: 0.0,
            zoom_factor: 1.0,
        }
    }
}

impl Default for PersistentState {
    fn default() -> Self {
        Self {
            layout: LayoutConfig::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// Returns the config directory: `~/.config/conch/`.
pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("conch")
}

pub fn config_path() -> PathBuf { config_dir().join("config.toml") }
fn state_path() -> PathBuf { config_dir().join("state.toml") }

// ---------------------------------------------------------------------------
// Load / Save — UserConfig
// ---------------------------------------------------------------------------

pub fn load_user_config() -> Result<UserConfig> {
    let path = config_path();
    if !path.exists() {
        log::info!("No config.toml at {}, using defaults", path.display());
        return Ok(UserConfig::default());
    }
    let contents = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let config: UserConfig = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse {}", path.display()))?;
    Ok(config)
}

pub fn save_user_config(config: &UserConfig) -> Result<()> {
    let dir = config_dir();
    if !dir.exists() { fs::create_dir_all(&dir)?; }
    let contents = toml::to_string_pretty(config).context("Failed to serialize config")?;
    fs::write(config_path(), contents)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Load / Save — PersistentState
// ---------------------------------------------------------------------------

pub fn load_persistent_state() -> Result<PersistentState> {
    let path = state_path();
    if !path.exists() {
        log::info!("No state.toml at {}, using defaults", path.display());
        return Ok(PersistentState::default());
    }
    let contents = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let state: PersistentState = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse {}", path.display()))?;
    Ok(state)
}

pub fn save_persistent_state(state: &PersistentState) -> Result<()> {
    let dir = config_dir();
    if !dir.exists() { fs::create_dir_all(&dir)?; }
    let contents = toml::to_string_pretty(state).context("Failed to serialize state")?;
    fs::write(state_path(), contents)?;
    Ok(())
}
