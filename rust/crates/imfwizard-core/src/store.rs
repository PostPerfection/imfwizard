//! Where imfwizard keeps its own files: the preferences and the GUI job queue
//! sit in one folder per platform.

use std::path::PathBuf;

const APP_DIRECTORY_NAME: &str = "imfwizard";

/// The folder imfwizard writes its state into: `%APPDATA%\imfwizard` on
/// Windows, `~/Library/Application Support/imfwizard` on macOS, and
/// `$XDG_CONFIG_HOME/imfwizard` or `~/.config/imfwizard` elsewhere.
pub fn data_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata).join(APP_DIRECTORY_NAME);
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join(APP_DIRECTORY_NAME);
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            return PathBuf::from(xdg).join(APP_DIRECTORY_NAME);
        }
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(".config").join(APP_DIRECTORY_NAME);
        }
    }

    PathBuf::from(".")
}
