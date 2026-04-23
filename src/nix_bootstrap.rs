//! Nix detection, installation, and configuration checking.
//!
//! This module provides the user-facing Nix bootstrap experience. When a
//! user runs a command that requires Nix (e.g., `envo install`), this module
//! detects whether Nix is available and, if not, offers to install it
//! interactively using Determinate Systems' installer.
//!
//! # Design decisions
//!
//! - **Interactive vs non-interactive**: In a TTY, we prompt the user. In
//!   CI or piped environments, we fail with a clear error — never prompt.
//! - **No persistent state**: We always re-detect Nix at runtime. The user
//!   might install Nix between envo invocations.
//! - **Fallback paths**: If `nix` isn't in PATH, we check common install
//!   locations before giving up.

use std::io::{self, BufRead, Write};
use std::process::Command;
use thiserror::Error;

/// Errors that can occur during Nix bootstrap.
#[derive(Debug, Error)]
pub enum NixBootstrapError {
    #[error(
        "Nix is not installed. Install it with:\n  \
         curl --proto '=https' --tlsv1.2 -sSf -L \
         https://install.determinate.systems/nix | sh -s -- install"
    )]
    NixNotInstalled,

    #[error("Nix installation failed. Please install manually: https://install.determinate.systems/nix")]
    NixInstallFailed,

    #[error("IO error: {0}")]
    Io(#[from] io::Error),
}

/// The status of Nix on the current system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NixStatus {
    /// Nix is installed and accessible.
    Available {
        /// The Nix version string (e.g., "2.33.3").
        version: String,

        /// Whether this is Determinate Nix (vs. standard Nix).
        is_determinate: bool,
    },

    /// Nix is not installed or not accessible.
    NotInstalled,
}

/// Detect whether Nix is installed and accessible.
///
/// Checks `nix --version` in PATH first, then falls back to common
/// install locations. Returns `NixStatus::Available` with version info
/// if found, or `NixStatus::NotInstalled` if not.
pub fn detect_nix() -> NixStatus {
    // Try nix in PATH first
    if let Some(status) = try_nix_at_path("nix") {
        return status;
    }

    // Check common install locations where Nix might be installed
    // but not yet in the current shell's PATH
    let fallback_paths = [
        "/nix/var/nix/profiles/default/bin/nix",
        "/run/current-system/sw/bin/nix", // NixOS
    ];

    // Also check $HOME/.nix-profile/bin/nix
    if let Ok(home) = std::env::var("HOME") {
        let home_nix = format!("{home}/.nix-profile/bin/nix");
        if let Some(status) = try_nix_at_path(&home_nix) {
            return status;
        }
    }

    for path in &fallback_paths {
        if let Some(status) = try_nix_at_path(path) {
            return status;
        }
    }

    NixStatus::NotInstalled
}

/// Try to run `nix --version` at a specific path and parse the output.
fn try_nix_at_path(nix_path: &str) -> Option<NixStatus> {
    let output = Command::new(nix_path)
        .arg("--version")
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let version_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if version_str.is_empty() {
        return None;
    }

    let (version, is_determinate) = parse_nix_version(&version_str);

    Some(NixStatus::Available {
        version,
        is_determinate,
    })
}

/// Parse a nix version string into a clean version number and determinate flag.
///
/// Examples:
/// - `"nix (Determinate Nix 3.18.0) 2.33.3"` → `("2.33.3", true)`
/// - `"nix (Nix) 2.18.1"` → `("2.18.1", false)`
/// - `"nix 2.18.1"` → `("2.18.1", false)`
pub fn parse_nix_version(version_str: &str) -> (String, bool) {
    let is_determinate = version_str.contains("Determinate");

    // The actual nix version is typically the last whitespace-separated token
    let version = version_str
        .split_whitespace()
        .last()
        .unwrap_or(version_str)
        .to_string();

    (version, is_determinate)
}

/// Format the Nix status for display in `envo version`.
///
/// Returns a human-readable string like:
/// - `"Determinate Nix 3.18.0 (nix 2.33.3)"`
/// - `"nix 2.18.1"`
/// - `"not installed"`
pub fn format_nix_status(status: &NixStatus) -> String {
    match status {
        NixStatus::Available {
            version,
            is_determinate,
        } => {
            if *is_determinate {
                format!("Determinate Nix (nix {version})")
            } else {
                format!("nix {version}")
            }
        }
        NixStatus::NotInstalled => "not installed".to_string(),
    }
}

/// Format the Nix status as a JSON-compatible serde_json::Value.
pub fn nix_status_to_json(status: &NixStatus) -> serde_json::Value {
    match status {
        NixStatus::Available {
            version,
            is_determinate,
        } => serde_json::json!({
            "installed": true,
            "version": version,
            "determinate": is_determinate,
        }),
        NixStatus::NotInstalled => serde_json::json!({
            "installed": false,
        }),
    }
}

/// Ensure Nix is available, prompting for installation if needed.
///
/// This is the user-facing function that commands like `envo install` call.
/// It detects Nix, and if not found:
/// - In interactive mode (TTY): prompts to install via Determinate's installer
/// - In non-interactive mode (CI/piped): returns an error with install instructions
///
/// Returns the `NixStatus` on success.
pub fn ensure_nix() -> Result<NixStatus, NixBootstrapError> {
    let status = detect_nix();

    match status {
        NixStatus::Available { .. } => Ok(status),
        NixStatus::NotInstalled => {
            if is_interactive() {
                prompt_nix_install()
            } else {
                // Non-interactive (CI, piped input) — never prompt
                Err(NixBootstrapError::NixNotInstalled)
            }
        }
    }
}

/// Check if the Nix installation supports flakes and the nix-command interface.
///
/// Runs a trivial `nix eval --expr "1"` to see if the nix-command
/// experimental feature is enabled. Returns `Ok(())` if it works,
/// or `Err(warning_message)` if flakes need to be enabled.
///
/// Note: This returns a String warning, not an error — the caller should
/// print it to stderr but continue.
pub fn check_flake_support() -> Result<(), String> {
    let output = Command::new("nix")
        .args(["eval", "--expr", "1"])
        .output();

    match output {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if stderr.contains("experimental feature")
                || stderr.contains("nix-command")
            {
                Err(
                    "Warning: Nix experimental features (nix-command, flakes) are not enabled.\n  \
                     Add 'experimental-features = nix-command flakes' to ~/.config/nix/nix.conf\n  \
                     Or install Determinate Nix, which enables these by default:\n  \
                     curl --proto '=https' --tlsv1.2 -sSf -L \
                     https://install.determinate.systems/nix | sh -s -- install"
                        .to_string(),
                )
            } else {
                // Some other nix error — don't warn about flakes
                Ok(())
            }
        }
        Err(_) => {
            // Can't run nix at all — detect_nix should have caught this
            Ok(())
        }
    }
}

/// Check if stdin is connected to a TTY (interactive terminal).
///
/// Returns false in CI, when input is piped, or when running non-interactively.
fn is_interactive() -> bool {
    atty_check()
}

/// Platform-specific TTY detection.
#[cfg(unix)]
fn atty_check() -> bool {
    // Use libc::isatty on Unix
    unsafe { libc::isatty(libc::STDIN_FILENO) != 0 }
}

#[cfg(not(unix))]
fn atty_check() -> bool {
    // Conservative default: assume non-interactive on non-Unix
    false
}

/// Prompt the user to install Nix interactively.
fn prompt_nix_install() -> Result<NixStatus, NixBootstrapError> {
    eprintln!();
    eprintln!("ℹ envo requires Nix to install and manage packages.");
    eprintln!();
    eprint!("Would you like to install Nix now? (recommended) [Y/n] ");
    io::stderr().flush()?;

    let mut input = String::new();
    io::stdin().lock().read_line(&mut input)?;
    let input = input.trim().to_lowercase();

    if input.is_empty() || input == "y" || input == "yes" {
        eprintln!();
        eprintln!("ℹ Installing Nix via Determinate Systems installer...");
        eprintln!();

        // Run the Determinate installer interactively — inherit stdin/stdout/stderr
        // so the user can see progress and respond to any installer prompts
        let status = Command::new("sh")
            .args([
                "-c",
                "curl --proto '=https' --tlsv1.2 -sSf -L https://install.determinate.systems/nix | sh -s -- install",
            ])
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status()
            .map_err(|_| NixBootstrapError::NixInstallFailed)?;

        if !status.success() {
            return Err(NixBootstrapError::NixInstallFailed);
        }

        // Re-detect after installation
        // The installer modifies PATH in shell config but not in the current process,
        // so we need to check the fallback paths
        let new_status = detect_nix();

        match new_status {
            NixStatus::Available { .. } => {
                eprintln!();
                eprintln!("✓ Nix installed successfully!");
                Ok(new_status)
            }
            NixStatus::NotInstalled => {
                // Nix was installed but isn't in our PATH yet — this is normal
                // The user needs to restart their shell
                eprintln!();
                eprintln!("✓ Nix installed. Please restart your shell and try again.");
                eprintln!("  Or run: . /nix/var/nix/profiles/default/etc/profile.d/nix-daemon.sh");
                Err(NixBootstrapError::NixInstallFailed)
            }
        }
    } else {
        eprintln!();
        eprintln!("To install Nix manually, run:");
        eprintln!("  curl --proto '=https' --tlsv1.2 -sSf -L https://install.determinate.systems/nix | sh -s -- install");
        Err(NixBootstrapError::NixNotInstalled)
    }
}

// ── Unit tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_determinate_nix_version() {
        let (version, is_det) =
            parse_nix_version("nix (Determinate Nix 3.18.0) 2.33.3");
        assert_eq!(version, "2.33.3");
        assert!(is_det);
    }

    #[test]
    fn test_parse_standard_nix_version() {
        let (version, is_det) = parse_nix_version("nix (Nix) 2.18.1");
        assert_eq!(version, "2.18.1");
        assert!(!is_det);
    }

    #[test]
    fn test_parse_bare_nix_version() {
        let (version, is_det) = parse_nix_version("nix 2.18.1");
        assert_eq!(version, "2.18.1");
        assert!(!is_det);
    }

    #[test]
    fn test_parse_empty_version() {
        let (version, is_det) = parse_nix_version("");
        assert_eq!(version, "");
        assert!(!is_det);
    }

    #[test]
    fn test_parse_unexpected_format() {
        // Should not panic on unexpected input
        let (version, is_det) =
            parse_nix_version("something completely different 1.2.3");
        assert_eq!(version, "1.2.3");
        assert!(!is_det);
    }

    #[test]
    fn test_format_nix_status_available_determinate() {
        let status = NixStatus::Available {
            version: "2.33.3".to_string(),
            is_determinate: true,
        };
        assert_eq!(format_nix_status(&status), "Determinate Nix (nix 2.33.3)");
    }

    #[test]
    fn test_format_nix_status_available_standard() {
        let status = NixStatus::Available {
            version: "2.18.1".to_string(),
            is_determinate: false,
        };
        assert_eq!(format_nix_status(&status), "nix 2.18.1");
    }

    #[test]
    fn test_format_nix_status_not_installed() {
        assert_eq!(
            format_nix_status(&NixStatus::NotInstalled),
            "not installed"
        );
    }

    #[test]
    fn test_nix_status_to_json_available() {
        let status = NixStatus::Available {
            version: "2.33.3".to_string(),
            is_determinate: true,
        };
        let json = nix_status_to_json(&status);
        assert_eq!(json["installed"], true);
        assert_eq!(json["version"], "2.33.3");
        assert_eq!(json["determinate"], true);
    }

    #[test]
    fn test_nix_status_to_json_not_installed() {
        let json = nix_status_to_json(&NixStatus::NotInstalled);
        assert_eq!(json["installed"], false);
        assert!(json.get("version").is_none());
    }

    #[test]
    fn test_detect_nix_on_system_with_nix() {
        // This test verifies detect_nix doesn't crash.
        // On CI with Nix: should return Available
        // On CI without Nix: should return NotInstalled
        let status = detect_nix();
        match &status {
            NixStatus::Available { version, .. } => {
                assert!(!version.is_empty());
            }
            NixStatus::NotInstalled => {
                // Also valid — depends on the test environment
            }
        }
    }
}
