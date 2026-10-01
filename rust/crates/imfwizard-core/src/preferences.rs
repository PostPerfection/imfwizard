use postkit::preferences::PrefsMigration;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

pub const CURRENT_PREFERENCES_VERSION: u32 = 2;
const FIRST_MIGRATION_VERSION: u32 = 2;
pub const AUTOMATIC_ENCODE_THREADS: u32 = 0;
const SNAKE_CASE_RENAMES: [(&str, &str); 14] = [
    ("default_app_profile", "profile"),
    ("creator_name", "creator"),
    ("default_language", "language"),
    ("preferred_encoder", "preferredEncoder"),
    ("default_bandwidth_mbps", "bandwidth"),
    ("default_colour_space", "colourspace"),
    ("default_hdr_mode", "hdr"),
    ("default_channel_config", "channelConfig"),
    ("loudness_target_lufs", "loudnessTargetLufs"),
    ("signing_certificate_path", "signingCert"),
    ("signing_key_path", "signingKey"),
    ("default_output_dir", "outputDir"),
    ("naming_template", "namingTemplate"),
    ("show_advanced_options", "showAdvancedOptions"),
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preferences {
    pub version: u32,
    pub profile: String,
    pub creator: String,
    pub language: String,
    pub preferred_encoder: String,
    pub bandwidth: u32,
    pub colourspace: String,
    pub hdr: String,
    pub channel_config: String,
    pub loudness_target_lufs: f64,
    pub signing_cert: String,
    pub signing_key: String,
    pub output_dir: String,
    pub naming_template: String,
    pub theme: String,
    pub show_advanced_options: bool,
    pub show_hints_before_build: bool,
    pub verify_after_build: bool,
    pub gpu: bool,
    pub gpu_license: String,
    pub gpu_registration_url: String,
    pub encode_threads: u32,
    pub detect_picture_findings: bool,
    #[serde(flatten)]
    pub additional: BTreeMap<String, serde_json::Value>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: CURRENT_PREFERENCES_VERSION,
            profile: "App2e".to_string(),
            creator: String::new(),
            language: "en".to_string(),
            preferred_encoder: "grok".to_string(),
            bandwidth: 250,
            colourspace: "Rec.709".to_string(),
            hdr: "SDR".to_string(),
            channel_config: "5.1".to_string(),
            loudness_target_lufs: -24.0,
            signing_cert: String::new(),
            signing_key: String::new(),
            output_dir: String::new(),
            naming_template: String::new(),
            theme: "dark".to_string(),
            show_advanced_options: false,
            show_hints_before_build: true,
            verify_after_build: true,
            gpu: false,
            gpu_license: String::new(),
            gpu_registration_url: String::new(),
            encode_threads: AUTOMATIC_ENCODE_THREADS,
            detect_picture_findings: false,
            additional: BTreeMap::new(),
        }
    }
}

pub fn preferences_path() -> PathBuf {
    postkit::preferences::config_dir("imfwizard").join("preferences.json")
}

pub fn load_preferences() -> io::Result<Preferences> {
    Ok(load_preferences_if_present()?.unwrap_or_default())
}

pub fn load_preferences_if_present() -> io::Result<Option<Preferences>> {
    load_preferences_from(&preferences_path())
}

pub fn preference_migrations() -> Vec<PrefsMigration> {
    vec![PrefsMigration {
        version: FIRST_MIGRATION_VERSION,
        description: "rename snake_case keys".to_string(),
        apply: Box::new(migrate_to_version_two),
    }]
}

fn migrate_to_version_two(json: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<Value>(json) else {
        return json.to_string();
    };
    let Some(object) = value.as_object_mut() else {
        return json.to_string();
    };
    for (snake_case_name, camel_case_name) in SNAKE_CASE_RENAMES {
        if let Some(stored) = object.remove(snake_case_name) {
            object.entry(camel_case_name).or_insert(stored);
        }
    }
    value.to_string()
}

fn newer_file_error(stored_version: u32) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "preferences file version {stored_version} is newer than supported version {CURRENT_PREFERENCES_VERSION}"
        ),
    )
}

pub fn load_preferences_from(path: &Path) -> io::Result<Option<Preferences>> {
    let Some(contents) = postkit::preferences::read_preferences_file(path)? else {
        return Ok(None);
    };
    let stored_version = postkit::preferences::prefs_version(&contents);
    if stored_version > CURRENT_PREFERENCES_VERSION {
        return Err(newer_file_error(stored_version));
    }
    // migration hides the parse error of invalid json
    serde_json::from_str::<Value>(&contents)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let migrated = postkit::preferences::migrate_preferences(&contents, &preference_migrations());
    let preferences: Preferences = serde_json::from_str(&migrated)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

    if stored_version < CURRENT_PREFERENCES_VERSION {
        save_preferences_to(&preferences, path)?;
    }

    Ok(Some(preferences))
}

pub fn save_preferences(preferences: &Preferences) -> io::Result<()> {
    save_preferences_to(preferences, &preferences_path())
}

pub fn save_preferences_to(preferences: &Preferences, path: &Path) -> io::Result<()> {
    if preferences.version > CURRENT_PREFERENCES_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "preferences version {} is newer than supported version {}",
                preferences.version, CURRENT_PREFERENCES_VERSION
            ),
        ));
    }

    if let Some(existing) = postkit::preferences::read_preferences_file(path)? {
        let existing_version = postkit::preferences::prefs_version(&existing);
        if existing_version > CURRENT_PREFERENCES_VERSION {
            return Err(newer_file_error(existing_version));
        }
    }

    let mut current = preferences.clone();
    current.version = CURRENT_PREFERENCES_VERSION;
    let contents = serde_json::to_string_pretty(&current).map_err(io::Error::other)?;
    postkit::preferences::write_preferences_file(path, &contents)
}

pub fn reset_preferences() -> io::Result<Preferences> {
    reset_preferences_to(&preferences_path())
}

pub fn reset_preferences_to(path: &Path) -> io::Result<Preferences> {
    let preferences = Preferences::default();
    let contents = serde_json::to_string_pretty(&preferences).map_err(io::Error::other)?;
    postkit::preferences::write_preferences_file(path, &contents)?;
    Ok(preferences)
}

pub fn set_preference(name: &str, value: &str) -> Result<Preferences, String> {
    let preferences = load_preferences().map_err(|error| error.to_string())?;
    let preferences = postkit::preferences::set_json_preference(&preferences, name, value)?;
    save_preferences(&preferences).map_err(|error| error.to_string())?;
    Ok(preferences)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn defaults_match_the_settings_form() {
        let preferences = Preferences::default();

        assert_eq!(preferences.version, CURRENT_PREFERENCES_VERSION);
        assert_eq!(preferences.profile, "App2e");
        assert_eq!(preferences.bandwidth, 250);
        assert_eq!(preferences.colourspace, "Rec.709");
        assert_eq!(preferences.hdr, "SDR");
        assert!(preferences.show_hints_before_build);
        assert!(preferences.verify_after_build);
        assert!(!preferences.gpu);
    }

    #[test]
    fn version_one_file_adds_auth_defaults_and_updates_version() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("preferences.json");
        let contents =
            r#"{"version":1,"creator_name":"Studio","default_bandwidth_mbps":180,"gpu_device":-1}"#;
        postkit::preferences::write_preferences_file(&path, contents).unwrap();

        let preferences = load_preferences_from(&path).unwrap().unwrap();

        assert_eq!(preferences.creator, "Studio");
        assert_eq!(preferences.bandwidth, 180);
        assert_eq!(preferences.gpu_license, "");
        assert_eq!(preferences.additional["gpu_device"], -1);
        assert_eq!(preferences.version, CURRENT_PREFERENCES_VERSION);
        let saved = postkit::preferences::read_preferences_file(&path)
            .unwrap()
            .unwrap();
        assert_eq!(
            postkit::preferences::prefs_version(&saved),
            CURRENT_PREFERENCES_VERSION
        );
        assert!(saved.contains("gpuRegistrationUrl"));
        assert!(saved.contains("gpu_device"));
    }

    #[test]
    fn a_file_without_encode_threads_encodes_on_automatic_threads() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("preferences.json");
        let contents = format!(r#"{{"version":{CURRENT_PREFERENCES_VERSION},"gpu":true}}"#);
        postkit::preferences::write_preferences_file(&path, &contents).unwrap();

        let preferences = load_preferences_from(&path).unwrap().unwrap();

        assert_eq!(preferences.encode_threads, AUTOMATIC_ENCODE_THREADS);
        assert!(!preferences.additional.contains_key("encodeThreads"));
    }

    #[test]
    fn a_file_without_the_findings_setting_leaves_the_picture_unscanned() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("preferences.json");
        let contents = format!(r#"{{"version":{CURRENT_PREFERENCES_VERSION},"gpu":true}}"#);
        postkit::preferences::write_preferences_file(&path, &contents).unwrap();

        let preferences = load_preferences_from(&path).unwrap().unwrap();

        assert!(!preferences.detect_picture_findings);
    }

    #[test]
    fn saved_preferences_name_the_encode_threads() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("preferences.json");

        save_preferences_to(&Preferences::default(), &path).unwrap();

        let saved = postkit::preferences::read_preferences_file(&path)
            .unwrap()
            .unwrap();
        assert!(saved.contains(r#""encodeThreads": 0"#));
    }

    #[test]
    fn newer_file_is_refused_until_reset() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("preferences.json");
        let contents = r#"{"version":99,"creator":"Future","futureField":true}"#;
        postkit::preferences::write_preferences_file(&path, contents).unwrap();

        let error = load_preferences_from(&path).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("version 99"));
        assert!(save_preferences_to(&Preferences::default(), &path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), contents.as_bytes());

        reset_preferences_to(&path).unwrap();

        let reset = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            postkit::preferences::prefs_version(&reset),
            CURRENT_PREFERENCES_VERSION
        );
    }

    #[test]
    fn the_migration_steps_run_from_the_first_to_the_current_version() {
        let versions: Vec<u32> = preference_migrations()
            .iter()
            .map(|migration| migration.version)
            .collect();

        let expected: Vec<u32> = (FIRST_MIGRATION_VERSION..=CURRENT_PREFERENCES_VERSION).collect();
        assert_eq!(versions, expected);
    }

    const EVERY_SNAKE_CASE_KEY: &str = r#""default_app_profile":"App2","creator_name":"Studio","default_language":"fr","preferred_encoder":"openjpeg","default_bandwidth_mbps":180,"default_colour_space":"Rec.2020","default_hdr_mode":"PQ","default_channel_config":"7.1","loudness_target_lufs":-23.0,"signing_certificate_path":"/keys/cert.pem","signing_key_path":"/keys/key.pem","default_output_dir":"/out","naming_template":"{title}","show_advanced_options":true"#;

    fn assert_every_snake_case_key_renamed(preferences: &Preferences, path: &Path) {
        assert_eq!(preferences.profile, "App2");
        assert_eq!(preferences.creator, "Studio");
        assert_eq!(preferences.language, "fr");
        assert_eq!(preferences.preferred_encoder, "openjpeg");
        assert_eq!(preferences.bandwidth, 180);
        assert_eq!(preferences.colourspace, "Rec.2020");
        assert_eq!(preferences.hdr, "PQ");
        assert_eq!(preferences.channel_config, "7.1");
        assert_eq!(preferences.loudness_target_lufs, -23.0);
        assert_eq!(preferences.signing_cert, "/keys/cert.pem");
        assert_eq!(preferences.signing_key, "/keys/key.pem");
        assert_eq!(preferences.output_dir, "/out");
        assert_eq!(preferences.naming_template, "{title}");
        assert!(preferences.show_advanced_options);
        assert!(preferences.additional.is_empty());

        let saved = postkit::preferences::read_preferences_file(path)
            .unwrap()
            .unwrap();
        let saved: serde_json::Map<String, Value> = serde_json::from_str(&saved).unwrap();
        assert_eq!(saved["version"], CURRENT_PREFERENCES_VERSION);
        for (snake_case_name, camel_case_name) in SNAKE_CASE_RENAMES {
            assert!(!saved.contains_key(snake_case_name), "{snake_case_name}");
            assert!(saved.contains_key(camel_case_name), "{camel_case_name}");
        }
        assert_eq!(saved["creator"], "Studio");
        assert_eq!(saved["signingCert"], "/keys/cert.pem");
    }

    #[test]
    fn version_one_snake_case_keys_are_renamed() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("preferences.json");
        let contents = format!(r#"{{"version":1,{EVERY_SNAKE_CASE_KEY}}}"#);
        postkit::preferences::write_preferences_file(&path, &contents).unwrap();

        let preferences = load_preferences_from(&path).unwrap().unwrap();

        assert_every_snake_case_key_renamed(&preferences, &path);
    }

    #[test]
    fn unversioned_snake_case_keys_are_renamed() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("preferences.json");
        let contents = format!("{{{EVERY_SNAKE_CASE_KEY}}}");
        postkit::preferences::write_preferences_file(&path, &contents).unwrap();

        let preferences = load_preferences_from(&path).unwrap().unwrap();

        assert_every_snake_case_key_renamed(&preferences, &path);
    }

    #[test]
    fn camel_case_key_wins_over_its_snake_case_name() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("preferences.json");
        let contents = r#"{"version":1,"creator_name":"Old","creator":"New"}"#;
        postkit::preferences::write_preferences_file(&path, contents).unwrap();

        let preferences = load_preferences_from(&path).unwrap().unwrap();

        assert_eq!(preferences.creator, "New");
        assert!(!preferences.additional.contains_key("creator_name"));
    }

    #[test]
    fn invalid_json_returns_an_error() {
        let directory = TempDir::new().unwrap();
        let path = directory.path().join("preferences.json");
        postkit::preferences::write_preferences_file(&path, "invalid").unwrap();

        assert!(load_preferences_from(&path).is_err());
    }
}
