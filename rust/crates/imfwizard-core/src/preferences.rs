//! User preferences management with platform-specific paths and schema migration.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const CURRENT_PREFS_VERSION: u32 = 1;

/// User preferences for IMF Wizard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preferences {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default = "default_app_profile")]
    pub default_app_profile: String,
    #[serde(default)]
    pub creator_name: String,
    #[serde(default = "default_language")]
    pub default_language: String,
    #[serde(default = "default_encoder")]
    pub preferred_encoder: String,
    #[serde(default = "default_bandwidth")]
    pub default_bandwidth_mbps: u32,
    #[serde(default = "default_colour_space")]
    pub default_colour_space: String,
    #[serde(default = "default_gpu_device")]
    pub gpu_device: i32,
    #[serde(default = "default_hdr_mode")]
    pub default_hdr_mode: String,
    #[serde(default = "default_channel_config")]
    pub default_channel_config: String,
    #[serde(default = "default_loudness")]
    pub loudness_target_lufs: f64,
    #[serde(default)]
    pub signing_certificate_path: String,
    #[serde(default)]
    pub signing_key_path: String,
    #[serde(default)]
    pub default_output_dir: String,
    #[serde(default)]
    pub naming_template: String,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default)]
    pub show_advanced_options: bool,
}

fn default_version() -> u32 {
    CURRENT_PREFS_VERSION
}
fn default_app_profile() -> String {
    "App2e".to_string()
}
fn default_language() -> String {
    "en".to_string()
}
fn default_encoder() -> String {
    "grok".to_string()
}
fn default_bandwidth() -> u32 {
    250
}
fn default_colour_space() -> String {
    "Rec.709".to_string()
}
fn default_gpu_device() -> i32 {
    -1
}
fn default_hdr_mode() -> String {
    "SDR".to_string()
}
fn default_channel_config() -> String {
    "5.1".to_string()
}
fn default_loudness() -> f64 {
    -24.0
}
fn default_theme() -> String {
    "dark".to_string()
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: CURRENT_PREFS_VERSION,
            default_app_profile: default_app_profile(),
            creator_name: String::new(),
            default_language: default_language(),
            preferred_encoder: default_encoder(),
            default_bandwidth_mbps: default_bandwidth(),
            default_colour_space: default_colour_space(),
            gpu_device: default_gpu_device(),
            default_hdr_mode: default_hdr_mode(),
            default_channel_config: default_channel_config(),
            loudness_target_lufs: default_loudness(),
            signing_certificate_path: String::new(),
            signing_key_path: String::new(),
            default_output_dir: String::new(),
            naming_template: String::new(),
            theme: default_theme(),
            show_advanced_options: false,
        }
    }
}

/// Get the platform-specific preferences file path.
pub fn preferences_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata)
                .join("imfwizard")
                .join("preferences.json");
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("imfwizard")
                .join("preferences.json");
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            return PathBuf::from(xdg)
                .join("imfwizard")
                .join("preferences.json");
        }
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join(".config")
                .join("imfwizard")
                .join("preferences.json");
        }
    }

    PathBuf::from("preferences.json")
}

/// Load preferences from disk. Returns defaults if file doesn't exist.
pub fn load_preferences() -> Preferences {
    load_preferences_from(&preferences_path())
}

/// Load preferences from a specific path.
pub fn load_preferences_from(path: &PathBuf) -> Preferences {
    let Ok(content) = fs::read_to_string(path) else {
        return Preferences::default();
    };

    serde_json::from_str(&content).unwrap_or_default()
}

/// Save preferences to disk.
pub fn save_preferences(prefs: &Preferences) -> std::io::Result<()> {
    save_preferences_to(prefs, &preferences_path())
}

/// Save preferences to a specific path.
pub fn save_preferences_to(prefs: &Preferences, path: &PathBuf) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(prefs)
        .map_err(std::io::Error::other)?;
    fs::write(path, json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_preferences() {
        let prefs = Preferences::default();
        assert_eq!(prefs.version, 1);
        assert_eq!(prefs.default_app_profile, "App2e");
        assert_eq!(prefs.preferred_encoder, "grok");
        assert_eq!(prefs.default_bandwidth_mbps, 250);
        assert_eq!(prefs.default_colour_space, "Rec.709");
        assert_eq!(prefs.gpu_device, -1);
        assert_eq!(prefs.default_hdr_mode, "SDR");
        assert_eq!(prefs.default_channel_config, "5.1");
        assert!((prefs.loudness_target_lufs - (-24.0)).abs() < 0.01);
        assert_eq!(prefs.theme, "dark");
        assert!(!prefs.show_advanced_options);
    }

    #[test]
    fn test_save_and_load() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("prefs.json");

        let prefs = Preferences {
            creator_name: "Test Creator".to_string(),
            default_bandwidth_mbps: 500,
            theme: "light".to_string(),
            ..Default::default()
        };

        save_preferences_to(&prefs, &path).unwrap();

        let loaded = load_preferences_from(&path);
        assert_eq!(loaded.creator_name, "Test Creator");
        assert_eq!(loaded.default_bandwidth_mbps, 500);
        assert_eq!(loaded.theme, "light");
    }

    #[test]
    fn test_load_missing_file() {
        let path = PathBuf::from("/nonexistent/prefs.json");
        let prefs = load_preferences_from(&path);
        assert_eq!(prefs.default_app_profile, "App2e");
    }

    #[test]
    fn test_load_partial_json() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("prefs.json");

        // Write minimal JSON — serde should fill in defaults
        fs::write(&path, r#"{"creator_name": "Partial", "theme": "blue"}"#).unwrap();

        let prefs = load_preferences_from(&path);
        assert_eq!(prefs.creator_name, "Partial");
        assert_eq!(prefs.theme, "blue");
        assert_eq!(prefs.default_bandwidth_mbps, 250); // default
        assert_eq!(prefs.preferred_encoder, "grok"); // default
    }

    #[test]
    fn test_load_invalid_json() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("prefs.json");
        fs::write(&path, "not json at all").unwrap();

        let prefs = load_preferences_from(&path);
        // Should return defaults
        assert_eq!(prefs.default_app_profile, "App2e");
    }

    #[test]
    fn test_roundtrip_all_fields() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("prefs.json");

        let prefs = Preferences {
            version: 1,
            default_app_profile: "App4".to_string(),
            creator_name: "Studio X".to_string(),
            default_language: "fr".to_string(),
            preferred_encoder: "kakadu".to_string(),
            default_bandwidth_mbps: 100,
            default_colour_space: "P3-D65".to_string(),
            gpu_device: 2,
            default_hdr_mode: "PQ".to_string(),
            default_channel_config: "7.1".to_string(),
            loudness_target_lufs: -23.0,
            signing_certificate_path: "/certs/cert.pem".to_string(),
            signing_key_path: "/certs/key.pem".to_string(),
            default_output_dir: "/output".to_string(),
            naming_template: "{title}_{version}".to_string(),
            theme: "system".to_string(),
            show_advanced_options: true,
        };

        save_preferences_to(&prefs, &path).unwrap();
        let loaded = load_preferences_from(&path);

        assert_eq!(loaded.default_app_profile, "App4");
        assert_eq!(loaded.creator_name, "Studio X");
        assert_eq!(loaded.default_language, "fr");
        assert_eq!(loaded.preferred_encoder, "kakadu");
        assert_eq!(loaded.default_bandwidth_mbps, 100);
        assert_eq!(loaded.default_colour_space, "P3-D65");
        assert_eq!(loaded.gpu_device, 2);
        assert_eq!(loaded.default_hdr_mode, "PQ");
        assert_eq!(loaded.default_channel_config, "7.1");
        assert!((loaded.loudness_target_lufs - (-23.0)).abs() < 0.01);
        assert_eq!(loaded.signing_certificate_path, "/certs/cert.pem");
        assert_eq!(loaded.signing_key_path, "/certs/key.pem");
        assert_eq!(loaded.default_output_dir, "/output");
        assert_eq!(loaded.naming_template, "{title}_{version}");
        assert_eq!(loaded.theme, "system");
        assert!(loaded.show_advanced_options);
    }

    #[test]
    fn test_preferences_path_not_empty() {
        let path = preferences_path();
        assert!(!path.as_os_str().is_empty());
        assert!(path.to_string_lossy().contains("imfwizard"));
    }
}
