//! Self-update logic for the envo CLI.
//!
//! Checks for new versions via the GitHub releases API, downloads
//! the correct binary for the current platform, and atomically replaces
//! the running binary. Uses `curl` via `Command::new` — no HTTP library
//! dependencies.

use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

/// GitHub repository owner for release downloads.
const GITHUB_OWNER: &str = "tylerPBprojects";

/// GitHub repository name for release downloads.
const GITHUB_REPO: &str = "envo";

/// The current version of envo, set at compile time from Cargo.toml.
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Errors that can occur during self-update operations.
#[derive(Debug, Error)]
pub enum SelfUpdateError {
    #[error("could not check for updates — check your connection or run `gh auth login`")]
    NetworkError,

    #[error("could not parse version from GitHub response")]
    ParseError,

    #[error("could not write to {path} — check file permissions")]
    PermissionError { path: String },

    #[error("could not determine install location")]
    InstallDirNotFound,

    #[error("unsupported platform: {os}-{arch}")]
    UnsupportedPlatform { os: String, arch: String },

    #[error("download failed: {message}")]
    DownloadFailed { message: String },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result of comparing the current version to the latest available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionStatus {
    /// Current version matches the latest.
    UpToDate,

    /// A newer version is available.
    UpdateAvailable { latest: String },
}

/// Check the latest version available on GitHub releases.
///
/// Shells out to `curl` to fetch the GitHub releases API endpoint
/// and parses the `tag_name` field from the JSON response.
pub fn check_latest_version() -> Result<String, SelfUpdateError> {
    let url = format!(
        "https://api.github.com/repos/{GITHUB_OWNER}/{GITHUB_REPO}/releases/latest"
    );

    let mut cmd = Command::new("curl");
    cmd.args([
        "-sSf",
        "-H", "Accept: application/vnd.github+json",
        "-H", "User-Agent: envo-self-update",
    ]);
    if let Some(token) = get_github_token() {
        cmd.arg("-H").arg(format!("Authorization: Bearer {token}"));
    }
    cmd.arg(&url);

    let output = cmd
        .output()
        .map_err(|_| SelfUpdateError::NetworkError)?;

    if !output.status.success() {
        return Err(SelfUpdateError::NetworkError);
    }

    let body = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|_| SelfUpdateError::ParseError)?;

    let tag = json
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or(SelfUpdateError::ParseError)?;

    // Strip leading 'v' if present (e.g., "v0.2.0" → "0.2.0")
    let version = tag.strip_prefix('v').unwrap_or(tag);

    Ok(version.to_string())
}

/// Compare the current version to the latest available version.
///
/// Uses simple string comparison of semver components. This is sufficient
/// for V1 — we don't need full semver range matching.
pub fn compare_versions(current: &str, latest: &str) -> VersionStatus {
    let current_parts = parse_semver(current);
    let latest_parts = parse_semver(latest);

    match (current_parts, latest_parts) {
        (Some(cur), Some(lat)) => {
            if lat > cur {
                VersionStatus::UpdateAvailable {
                    latest: latest.to_string(),
                }
            } else {
                VersionStatus::UpToDate
            }
        }
        // If we can't parse either version, fall back to string comparison
        _ => {
            if latest != current {
                VersionStatus::UpdateAvailable {
                    latest: latest.to_string(),
                }
            } else {
                VersionStatus::UpToDate
            }
        }
    }
}

/// Parse a semver string into (major, minor, patch) tuple.
fn parse_semver(version: &str) -> Option<(u32, u32, u32)> {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let major = parts[0].parse().ok()?;
    let minor = parts[1].parse().ok()?;
    // Patch might have a pre-release suffix (e.g., "0-beta"); take only digits
    let patch_str = parts[2].split('-').next().unwrap_or(parts[2]);
    let patch = patch_str.parse().ok()?;
    Some((major, minor, patch))
}

/// Download a new version and atomically replace the current binary.
///
/// Downloads to `{install_dir}/envo.new`, sets executable permissions,
/// then renames over the existing binary. The rename is atomic on the
/// same filesystem, so the binary is never in a partially-written state.
pub fn download_and_replace(version: &str) -> Result<(), SelfUpdateError> {
    let install_dir = get_install_dir()?;
    let current_binary = install_dir.join("envo");
    let temp_binary = install_dir.join("envo.new");
    let platform = get_current_platform()?;

    let binary_name = format!("envo-{version}-{platform}");
    let url = format!(
        "https://github.com/{GITHUB_OWNER}/{GITHUB_REPO}/releases/download/v{version}/{binary_name}"
    );

    // Download to temp file
    let mut cmd = Command::new("curl");
    cmd.args(["-sSfL", "-o"]).arg(temp_binary.as_os_str());
    if let Some(token) = get_github_token() {
        cmd.arg("-H").arg(format!("Authorization: Bearer {token}"));
    }
    cmd.arg(&url);

    let status = cmd
        .status()
        .map_err(|_| SelfUpdateError::NetworkError)?;

    if !status.success() {
        // Clean up temp file on failure
        let _ = std::fs::remove_file(&temp_binary);
        return Err(SelfUpdateError::DownloadFailed {
            message: format!("curl returned exit code {}", status.code().unwrap_or(-1)),
        });
    }

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp_binary, std::fs::Permissions::from_mode(0o755))
            .map_err(|_| SelfUpdateError::PermissionError {
                path: temp_binary.display().to_string(),
            })?;
    }

    // Atomic rename
    std::fs::rename(&temp_binary, &current_binary).map_err(|_| {
        // Clean up temp file if rename fails
        let _ = std::fs::remove_file(&temp_binary);
        SelfUpdateError::PermissionError {
            path: current_binary.display().to_string(),
        }
    })?;

    Ok(())
}

/// Get the directory where envo is installed.
///
/// Tries to determine this from the current executable path. Falls back
/// to `~/.envo/bin/` if the executable path can't be determined.
pub fn get_install_dir() -> Result<PathBuf, SelfUpdateError> {
    // Try to get the path of the currently running binary
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            return Ok(parent.to_path_buf());
        }
    }

    // Fall back to ~/.envo/bin/
    if let Some(home) = home_dir() {
        let default_dir = home.join(".envo").join("bin");
        if default_dir.exists() {
            return Ok(default_dir);
        }
    }

    Err(SelfUpdateError::InstallDirNotFound)
}

/// Get the current platform string matching the install.sh naming convention.
///
/// Returns strings like `"linux-x86_64"`, `"linux-aarch64"`, `"darwin-aarch64"`.
pub fn get_current_platform() -> Result<String, SelfUpdateError> {
    let arch = std::env::consts::ARCH;
    let os = std::env::consts::OS;

    let platform_os = match os {
        "linux" => "linux",
        "macos" => "darwin",
        _ => {
            return Err(SelfUpdateError::UnsupportedPlatform {
                os: os.to_string(),
                arch: arch.to_string(),
            });
        }
    };

    let platform_arch = match arch {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        _ => {
            return Err(SelfUpdateError::UnsupportedPlatform {
                os: os.to_string(),
                arch: arch.to_string(),
            });
        }
    };

    Ok(format!("{platform_os}-{platform_arch}"))
}

/// Try to find a GitHub token for authenticating API and download requests.
///
/// Checks GITHUB_TOKEN env var first, then falls back to the `gh` CLI.
/// Required for private repositories.
fn get_github_token() -> Option<String> {
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        if !token.is_empty() {
            return Some(token);
        }
    }

    let output = Command::new("gh")
        .args(["auth", "token"])
        .output()
        .ok()?;

    if output.status.success() {
        let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !token.is_empty() {
            return Some(token);
        }
    }

    None
}

/// Get the user's home directory.
fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
}

/// Get the install path of the current envo binary for display.
pub fn get_install_path() -> String {
    std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

/// Detect the current system string (same as lockfile/resolver but accessible here).
pub fn get_current_system() -> String {
    let arch = std::env::consts::ARCH;
    let os = std::env::consts::OS;

    let nix_arch = match arch {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        _ => arch,
    };

    let nix_os = match os {
        "linux" => "linux",
        "macos" => "darwin",
        _ => os,
    };

    format!("{nix_arch}-{nix_os}")
}

/// Get the Nix version string, if Nix is installed.
pub fn get_nix_version() -> Option<String> {
    let output = Command::new("nix")
        .arg("--version")
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

// ── Unit tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compare_versions_up_to_date() {
        assert_eq!(
            compare_versions("0.1.0", "0.1.0"),
            VersionStatus::UpToDate
        );
    }

    #[test]
    fn test_compare_versions_update_available_patch() {
        assert_eq!(
            compare_versions("0.1.0", "0.1.1"),
            VersionStatus::UpdateAvailable {
                latest: "0.1.1".to_string()
            }
        );
    }

    #[test]
    fn test_compare_versions_update_available_minor() {
        assert_eq!(
            compare_versions("0.1.0", "0.2.0"),
            VersionStatus::UpdateAvailable {
                latest: "0.2.0".to_string()
            }
        );
    }

    #[test]
    fn test_compare_versions_update_available_major() {
        assert_eq!(
            compare_versions("0.1.0", "1.0.0"),
            VersionStatus::UpdateAvailable {
                latest: "1.0.0".to_string()
            }
        );
    }

    #[test]
    fn test_compare_versions_current_is_newer() {
        // Edge case: running a pre-release or dev build newer than latest release
        assert_eq!(
            compare_versions("0.2.0", "0.1.0"),
            VersionStatus::UpToDate
        );
    }

    #[test]
    fn test_parse_semver_valid() {
        assert_eq!(parse_semver("0.1.0"), Some((0, 1, 0)));
        assert_eq!(parse_semver("1.23.456"), Some((1, 23, 456)));
    }

    #[test]
    fn test_parse_semver_with_prerelease() {
        // Should parse the numeric parts, ignoring pre-release suffix
        assert_eq!(parse_semver("0.1.0-beta"), Some((0, 1, 0)));
    }

    #[test]
    fn test_parse_semver_invalid() {
        assert_eq!(parse_semver("not-a-version"), None);
        assert_eq!(parse_semver("1.2"), None);
        assert_eq!(parse_semver(""), None);
    }

    #[test]
    fn test_get_current_platform() {
        let platform = get_current_platform().unwrap();
        assert!(
            platform.contains('-'),
            "platform should be os-arch: {platform}"
        );
        // Should be one of our supported platforms
        let valid = ["linux-x86_64", "linux-aarch64", "darwin-aarch64"];
        assert!(
            valid.contains(&platform.as_str()),
            "unexpected platform: {platform}"
        );
    }

    #[test]
    fn test_get_install_dir() {
        // Should succeed — we're running from cargo test, which has a known exe path
        let dir = get_install_dir();
        assert!(dir.is_ok(), "get_install_dir failed: {:?}", dir.err());
    }

    #[test]
    fn test_get_install_path() {
        let path = get_install_path();
        assert_ne!(path, "unknown");
        assert!(!path.is_empty());
    }

    #[test]
    fn test_get_current_system() {
        let sys = get_current_system();
        assert!(sys.contains('-'));
    }
}
