use gpui_kit::Global;
use nle_core::NleFormat;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Default policy applied to detected effects when opening a project.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefaultEffectPolicy {
    #[default]
    Recommended,
    MarkerOnly,
    SkipAll,
}

impl DefaultEffectPolicy {
    pub const ALL: &'static [(Self, &'static str)] = &[
        (
            Self::Recommended,
            "Recommended (Auto-Convert + Suggested Equivalents)",
        ),
        (
            Self::MarkerOnly,
            "Drop Effects + Place Review Markers",
        ),
        (Self::SkipAll, "Skip All Non-Native Effects"),
    ];
}

/// Persistent user settings for Aagedal NLE Project Converter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub default_target_format: NleFormat,
    pub default_effect_policy: DefaultEffectPolicy,
    pub auto_reveal_output: bool,
    pub show_shortcut_hints: bool,
    pub remember_last_directory: bool,
    pub last_directory: Option<String>,

    // Default conversion options applied to new sessions / projects
    pub default_include_bins: bool,
    pub default_include_unused_media: bool,
    pub default_include_markers: bool,
    pub default_include_clip_colors: bool,
    pub default_include_lumetri: bool,
    pub default_inject_fx_markers: bool,
    pub default_flatten_nested: bool,

    // Default media path relinker rules
    pub auto_fill_relink: bool,
    pub default_relink_from: String,
    pub default_relink_to: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            default_target_format: NleFormat::DaVinciResolve,
            default_effect_policy: DefaultEffectPolicy::Recommended,
            auto_reveal_output: false,
            show_shortcut_hints: true,
            remember_last_directory: true,
            last_directory: None,
            default_include_bins: true,
            default_include_unused_media: true,
            default_include_markers: true,
            default_include_clip_colors: true,
            default_include_lumetri: true,
            default_inject_fx_markers: true,
            default_flatten_nested: false,
            auto_fill_relink: true,
            default_relink_from: String::new(),
            default_relink_to: String::new(),
        }
    }
}

impl AppSettings {
    pub fn settings_path() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("aagedal-nle-converter").join("settings.json"))
    }

    pub fn load() -> Self {
        Self::settings_path()
            .and_then(|p| Self::load_from_path(&p).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<PathBuf, String> {
        let Some(path) = Self::settings_path() else {
            return Err("Could not determine system configuration directory.".to_string());
        };
        self.save_to_path(&path)?;
        Ok(path)
    }

    pub fn load_from_path(path: &Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str::<Self>(&raw).map_err(|e| e.to_string())
    }

    pub fn save_to_path(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| e.to_string())
    }

    pub fn last_dir_path(&self) -> Option<PathBuf> {
        if !self.remember_last_directory {
            return None;
        }
        self.last_directory
            .as_deref()
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
    }
}

/// GPUI Global wrapper around `AppSettings`.
pub struct AppSettingsGlobal(pub AppSettings);

impl Global for AppSettingsGlobal {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_roundtrip_on_disk() {
        let dir = std::env::temp_dir().join("aagedal_nle_settings_test");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("nested").join("settings.json");

        let mut s = AppSettings::default();
        s.default_target_format = NleFormat::PremierePro;
        s.default_effect_policy = DefaultEffectPolicy::MarkerOnly;
        s.auto_reveal_output = true;
        s.show_shortcut_hints = false;
        s.default_flatten_nested = true;
        s.default_relink_from = "D:\\Media".to_string();
        s.default_relink_to = "/Volumes/RAID/Media".to_string();
        s.last_directory = Some("/tmp".to_string());

        s.save_to_path(&path).expect("save settings");
        let loaded = AppSettings::load_from_path(&path).expect("load settings");
        assert_eq!(loaded, s);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn partial_json_falls_back_to_defaults() {
        let partial = r#"{"auto_reveal_output": true}"#;
        let parsed: AppSettings = serde_json::from_str(partial).expect("parse partial json");
        assert!(parsed.auto_reveal_output);
        assert_eq!(parsed.default_target_format, NleFormat::DaVinciResolve);
        assert!(parsed.default_include_bins);
        assert!(parsed.show_shortcut_hints);
    }
}
