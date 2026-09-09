//! Chrome profile discovery and binary location.
//!
//! Read-only with respect to the user's data: listing parses the `Local
//! State` manifest, and personal launches point Chrome at the real
//! directory without copying or modifying anything outside it.

use std::io;
use std::path::PathBuf;

/// The Chrome user-data directory holding every profile. Override with
/// `CHROME_USER_DATA_DIR` (used by tests to point at fixtures).
pub fn user_data_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("CHROME_USER_DATA_DIR")
        && !dir.is_empty()
    {
        return Some(PathBuf::from(dir));
    }
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(|base| PathBuf::from(base).join("Google/Chrome/User Data"))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(|base| PathBuf::from(base).join("Library/Application Support/Google/Chrome"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::env::var_os("HOME").map(|base| PathBuf::from(base).join(".config/google-chrome"))
    }
    #[cfg(not(any(windows, unix)))]
    {
        None
    }
}

/// Chrome executable. `CHROME_BINARY` wins; otherwise well-known install
/// paths are probed. Deliberately no Edge fallback: personal browsing
/// stays in the browser the user chose.
pub fn chrome_binary() -> Option<PathBuf> {
    if let Ok(binary) = std::env::var("CHROME_BINARY")
        && !binary.is_empty()
    {
        return Some(PathBuf::from(binary));
    }
    #[cfg(windows)]
    {
        for candidate in [
            "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
            "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
        ] {
            if std::path::Path::new(candidate).exists() {
                return Some(PathBuf::from(candidate));
            }
        }
        None
    }
    #[cfg(target_os = "macos")]
    {
        let candidate = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
        if std::path::Path::new(candidate).exists() {
            return Some(PathBuf::from(candidate));
        }
        None
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        for candidate in [
            "/usr/bin/google-chrome",
            "/usr/bin/chromium",
            "/snap/bin/chromium",
        ] {
            if std::path::Path::new(candidate).exists() {
                return Some(PathBuf::from(candidate));
            }
        }
        None
    }
    #[cfg(not(any(windows, unix)))]
    {
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileInfo {
    /// Human name shown in the Chrome profile picker.
    pub name: String,
    /// Directory name inside the user-data dir (`Default`, `Profile 1`, …).
    pub directory: String,
}

/// Read-only listing of the user's profiles from the `Local State`
/// manifest. Returns an error (never guesses) when the directory or the
/// manifest is missing or malformed.
pub fn list_profiles() -> io::Result<Vec<ProfileInfo>> {
    let data_dir =
        user_data_dir().ok_or_else(|| io::Error::other("Chrome user-data directory unknown"))?;
    let manifest = std::fs::read_to_string(data_dir.join("Local State"))
        .map_err(|e| io::Error::other(format!("cannot read Chrome Local State: {e}")))?;
    let value: serde_json::Value = serde_json::from_str(&manifest)
        .map_err(|e| io::Error::other(format!("bad Local State JSON: {e}")))?;
    let cache = value
        .get("profile")
        .and_then(|profile| profile.get("info_cache"))
        .and_then(|cache| cache.as_object())
        .ok_or_else(|| io::Error::other("Local State has no profile cache"))?;
    let mut profiles = Vec::new();
    for (directory, info) in cache {
        let name = info
            .get("name")
            .and_then(|name| name.as_str())
            .unwrap_or(directory)
            .to_string();
        profiles.push(ProfileInfo {
            name,
            directory: directory.clone(),
        });
    }
    if profiles.is_empty() {
        return Err(io::Error::other("no Chrome profiles found"));
    }
    Ok(profiles)
}
