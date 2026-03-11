use serde::Deserialize;
use std::path::PathBuf;

const DEFAULT_CONFIG_PATH: &str = "/etc/barrgreet/config.toml";

pub const DEFAULT_CONFIG: &str = include_str!("../config.toml.example");

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    #[serde(default)]
    pub layout: LayoutConfig,
    #[serde(default)]
    pub style: StyleConfig,
    #[serde(default)]
    pub general: GeneralConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            layout: LayoutConfig::default(),
            style: StyleConfig::default(),
            general: GeneralConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct LayoutConfig {
    #[serde(default = "default_position")]
    pub position: Position,
    #[serde(default)]
    pub margin_top: u16,
    #[serde(default)]
    pub margin_bottom: u16,
    #[serde(default)]
    pub margin_left: u16,
    #[serde(default)]
    pub margin_right: u16,
    #[serde(default = "default_card_width")]
    pub card_width: u16,
    #[serde(default = "default_card_padding")]
    pub card_padding: u16,
    #[serde(default = "default_card_border_radius")]
    pub card_border_radius: f32,
    #[serde(default = "default_spacing")]
    pub spacing: u16,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            position: default_position(),
            margin_top: 0,
            margin_bottom: 0,
            margin_left: 0,
            margin_right: 0,
            card_width: default_card_width(),
            card_padding: default_card_padding(),
            card_border_radius: default_card_border_radius(),
            spacing: default_spacing(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Position {
    Center,
    Top,
    TopLeft,
    TopRight,
    Bottom,
    BottomLeft,
    BottomRight,
    Left,
    Right,
}

impl Position {
    pub fn horizontal(&self) -> iced::alignment::Horizontal {
        match self {
            Position::TopLeft | Position::Left | Position::BottomLeft => {
                iced::alignment::Horizontal::Left
            }
            Position::TopRight | Position::Right | Position::BottomRight => {
                iced::alignment::Horizontal::Right
            }
            _ => iced::alignment::Horizontal::Center,
        }
    }

    pub fn vertical(&self) -> iced::alignment::Vertical {
        match self {
            Position::Top | Position::TopLeft | Position::TopRight => {
                iced::alignment::Vertical::Top
            }
            Position::Bottom | Position::BottomLeft | Position::BottomRight => {
                iced::alignment::Vertical::Bottom
            }
            _ => iced::alignment::Vertical::Center,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct StyleConfig {
    #[serde(default = "default_background_color")]
    pub background_color: String,
    #[serde(default = "default_background_opacity")]
    pub background_opacity: f32,
    #[serde(default = "default_border_color")]
    pub border_color: String,
    #[serde(default = "default_border_opacity")]
    pub border_opacity: f32,
    #[serde(default = "default_border_width")]
    pub border_width: f32,
    #[serde(default = "default_text_color")]
    pub text_color: String,
    #[serde(default = "default_error_color")]
    pub error_color: String,
    #[serde(default = "default_button_color")]
    pub button_color: String,
    #[serde(default = "default_button_opacity")]
    pub button_opacity: f32,
    #[serde(default = "default_destructive_color")]
    pub destructive_color: String,
    #[serde(default = "default_destructive_opacity")]
    pub destructive_opacity: f32,
}

impl Default for StyleConfig {
    fn default() -> Self {
        Self {
            background_color: default_background_color(),
            background_opacity: default_background_opacity(),
            border_color: default_border_color(),
            border_opacity: default_border_opacity(),
            border_width: default_border_width(),
            text_color: default_text_color(),
            error_color: default_error_color(),
            button_color: default_button_color(),
            button_opacity: default_button_opacity(),
            destructive_color: default_destructive_color(),
            destructive_opacity: default_destructive_opacity(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct GeneralConfig {
    #[serde(default = "default_welcome_text")]
    pub welcome_text: String,
    #[serde(default = "default_session_dirs")]
    pub session_dirs: Vec<String>,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            welcome_text: default_welcome_text(),
            session_dirs: default_session_dirs(),
        }
    }
}

// Default value functions
fn default_position() -> Position { Position::Center }
fn default_card_width() -> u16 { 450 }
fn default_card_padding() -> u16 { 32 }
fn default_card_border_radius() -> f32 { 20.0 }
fn default_spacing() -> u16 { 16 }

fn default_background_color() -> String { "#14141E".into() }
fn default_background_opacity() -> f32 { 0.55 }
fn default_border_color() -> String { "#FFFFFF".into() }
fn default_border_opacity() -> f32 { 0.15 }
fn default_border_width() -> f32 { 1.0 }
fn default_text_color() -> String { "#FFFFFF".into() }
fn default_error_color() -> String { "#FF8888".into() }
fn default_button_color() -> String { "#648CF0".into() }
fn default_button_opacity() -> f32 { 0.8 }
fn default_destructive_color() -> String { "#C83C3C".into() }
fn default_destructive_opacity() -> f32 { 0.5 }

fn default_welcome_text() -> String { "Welcome".into() }
fn default_session_dirs() -> Vec<String> {
    vec![
        "/usr/share/wayland-sessions".into(),
        "/usr/share/xsessions".into(),
        "/run/current-system/sw/share/wayland-sessions".into(),
        "/run/current-system/sw/share/xsessions".into(),
        "/usr/local/share/wayland-sessions".into(),
        "/usr/local/share/xsessions".into(),
    ]
}

/// Parse a hex color string like "#FF8800" into (r, g, b).
pub fn parse_hex_color(s: &str) -> (u8, u8, u8) {
    let s = s.strip_prefix('#').unwrap_or(s);
    let r = u8::from_str_radix(&s[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&s[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&s[4..6], 16).unwrap_or(0);
    (r, g, b)
}

/// Convert a hex color string + opacity to an iced Color.
pub fn hex_to_color(hex: &str, opacity: f32) -> iced::Color {
    let (r, g, b) = parse_hex_color(hex);
    iced::Color::from_rgba8(r, g, b, opacity)
}

impl Config {
    pub fn load() -> Self {
        let args: Vec<String> = std::env::args().collect();
        let path = args
            .iter()
            .position(|a| a == "-c")
            .and_then(|i| args.get(i + 1).cloned())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_PATH));

        match std::fs::read_to_string(&path) {
            Ok(contents) => match toml::from_str(&contents) {
                Ok(config) => {
                    eprintln!("[barrgreet] loaded config from {}", path.display());
                    config
                }
                Err(e) => {
                    eprintln!(
                        "[barrgreet] WARNING: failed to parse {}: {e} — using defaults",
                        path.display()
                    );
                    Config::default()
                }
            },
            Err(_) if path == PathBuf::from(DEFAULT_CONFIG_PATH) => {
                eprintln!("[barrgreet] no config at {DEFAULT_CONFIG_PATH} — using defaults");
                Config::default()
            }
            Err(e) => {
                eprintln!(
                    "[barrgreet] WARNING: could not read {}: {e} — using defaults",
                    path.display()
                );
                Config::default()
            }
        }
    }
}
