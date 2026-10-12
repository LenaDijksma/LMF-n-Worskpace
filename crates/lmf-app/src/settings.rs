//! User settings, stored as a small `key=value` text file in the platform config folder.
//!
//! Stored: theme, font scale and the last opened folder. Nothing else, and nothing leaves the
//! machine. The file is read tolerantly: unknown keys and invalid values are ignored.

use std::fs;
use std::io;
use std::path::PathBuf;

use crate::theme::ThemeKind;

pub const MIN_SCALE: f32 = 0.8;
pub const MAX_SCALE: f32 = 2.0;
const SCALE_STEP: f32 = 0.1;

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub theme: ThemeKind,
    pub font_scale: f32,
    pub last_folder: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: ThemeKind::Light,
            font_scale: 1.0,
            last_folder: None,
        }
    }
}

/// Clamps to the allowed range and rounds to one decimal place.
pub fn clamp_scale(scale: f32) -> f32 {
    if !scale.is_finite() {
        return 1.0;
    }
    let clamped = scale.clamp(MIN_SCALE, MAX_SCALE);
    (clamped * 10.0).round() / 10.0
}

/// Moves the scale one step up (`+1`) or down (`-1`).
pub fn step_scale(scale: f32, direction: i32) -> f32 {
    clamp_scale(scale + SCALE_STEP * direction as f32)
}

impl Settings {
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "theme" => {
                    if let Some(t) = ThemeKind::from_id(value) {
                        s.theme = t;
                    }
                }
                "font_scale" => {
                    if let Ok(v) = value.parse::<f32>() {
                        s.font_scale = clamp_scale(v);
                    }
                }
                "last_folder" => {
                    if !value.is_empty() {
                        s.last_folder = Some(value.to_string());
                    }
                }
                _ => {}
            }
        }
        s
    }

    pub fn serialize(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("theme={}\n", self.theme.id()));
        out.push_str(&format!("font_scale={:.1}\n", clamp_scale(self.font_scale)));
        if let Some(folder) = &self.last_folder {
            // A newline would corrupt the line-based format.
            if !folder.contains('\n') && !folder.contains('\r') {
                out.push_str(&format!("last_folder={}\n", folder));
            }
        }
        out
    }

    /// Loads settings, falling back to defaults if the file is missing or unreadable.
    pub fn load() -> Settings {
        settings_path()
            .and_then(|p| fs::read_to_string(p).ok())
            .map(|t| Settings::parse(&t))
            .unwrap_or_default()
    }

    /// Best effort; callers may ignore the error (settings are a convenience).
    pub fn save(&self) -> io::Result<()> {
        let Some(path) = settings_path() else {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no config folder"));
        };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(path, self.serialize())
    }
}

fn config_dir() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join("Library").join("Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    }
}

fn settings_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("lovely-workspace").join("settings.txt"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_for_empty_or_garbage() {
        assert_eq!(Settings::parse(""), Settings::default());
        assert_eq!(Settings::parse("nonsense\n=\n===\n"), Settings::default());
    }

    #[test]
    fn round_trip() {
        let s = Settings {
            theme: ThemeKind::Dark,
            font_scale: 1.3,
            last_folder: Some("/home/me/My Notes".to_string()),
        };
        assert_eq!(Settings::parse(&s.serialize()), s);
    }

    #[test]
    fn invalid_values_are_ignored_and_scale_is_clamped() {
        let s = Settings::parse("theme=neon\nfont_scale=banana\nunknown=1\n");
        assert_eq!(s, Settings::default());
        assert_eq!(Settings::parse("font_scale=99").font_scale, MAX_SCALE);
        assert_eq!(Settings::parse("font_scale=0.01").font_scale, MIN_SCALE);
        assert_eq!(Settings::parse("font_scale=NaN").font_scale, 1.0);
    }

    #[test]
    fn scale_steps_stay_in_range() {
        let mut s = 1.0;
        for _ in 0..50 {
            s = step_scale(s, 1);
        }
        assert_eq!(s, MAX_SCALE);
        for _ in 0..50 {
            s = step_scale(s, -1);
        }
        assert_eq!(s, MIN_SCALE);
    }

    #[test]
    fn folders_with_newlines_are_not_written() {
        let s = Settings {
            last_folder: Some("a\nb".to_string()),
            ..Settings::default()
        };
        assert!(!s.serialize().contains("last_folder"));
    }
}
