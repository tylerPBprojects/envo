//! Schema types for the envo manifest (manifest.toml).
//!
//! The manifest is a TOML file that declares packages, environment variables,
//! hooks, services, and options for an envo environment. The schema is designed
//! to feel like pyproject.toml or Cargo.toml — familiar and minimal.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level manifest structure, mirroring the TOML layout.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManifestToml {
    /// Project metadata (required).
    pub project: ProjectConfig,

    /// Package declarations. Keys are package names, values are either a version
    /// string (shorthand) or a full PackageSpec table.
    #[serde(default)]
    pub packages: HashMap<String, PackageEntry>,

    /// Environment variables to set on activation.
    #[serde(default)]
    pub vars: HashMap<String, String>,

    /// Hook scripts that run during environment lifecycle events.
    #[serde(default)]
    pub hook: Option<HookConfig>,

    /// Long-running services managed by the environment.
    #[serde(default)]
    pub services: HashMap<String, ServiceConfig>,

    /// Global options controlling resolution and behavior.
    #[serde(default)]
    pub options: ManifestOptions,
}

/// Project metadata section.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectConfig {
    /// Project name (required). Used as the environment identifier.
    pub name: String,

    /// Optional human-readable description.
    #[serde(default)]
    pub description: Option<String>,

    /// Optional project version.
    #[serde(default)]
    pub version: Option<String>,
}

/// A package entry in the manifest. Supports two forms:
///
/// Shorthand: `ripgrep = "14.1"`
/// Full:      `ripgrep = { version = "14.1", systems = ["x86_64-linux"] }`
///
/// We use an untagged enum so serde handles both transparently.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum PackageEntry {
    /// Shorthand: just a version string (or "*" for latest).
    Short(String),

    /// Full specification with optional fields.
    Full(PackageSpec),
}

/// Full package specification with all optional fields.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PackageSpec {
    /// Version constraint. None or "*" means latest.
    #[serde(default)]
    pub version: Option<String>,

    /// Restrict this package to specific systems (e.g., ["x86_64-linux"]).
    /// None means all systems the project targets.
    #[serde(default)]
    pub systems: Option<Vec<String>>,

    /// Override the nixpkgs attribute path. By default, the package name
    /// is used as the attribute path. This allows e.g. `python = { pkg-path = "python3" }`.
    #[serde(default, rename = "pkg-path")]
    pub pkg_path: Option<String>,

    /// Package priority for PATH ordering. Lower numbers = higher priority.
    /// Used to resolve conflicts when multiple packages provide the same binary.
    #[serde(default)]
    pub priority: Option<i32>,
}

/// Hook configuration for environment lifecycle events.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HookConfig {
    /// Bash script executed when the environment is activated.
    /// Runs in bash regardless of the user's shell.
    #[serde(default, rename = "on-activate")]
    pub on_activate: Option<String>,
}

/// Service configuration for long-running processes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceConfig {
    /// Command to start the service.
    pub command: String,

    /// Optional command to gracefully shut down the service.
    #[serde(default)]
    pub shutdown: Option<String>,
}

/// Global options controlling environment behavior.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManifestOptions {
    /// Nixpkgs flake reference for package resolution.
    /// Defaults to "nixpkgs" (the flake registry entry for nixpkgs-unstable).
    #[serde(default = "default_nixpkgs_channel", rename = "nixpkgs-channel")]
    pub nixpkgs_channel: String,

    /// Whether to allow packages with unfree licenses (e.g., CUDA).
    #[serde(default, rename = "allow-unfree")]
    pub allow_unfree: bool,

    /// Target systems for resolution. Defaults to the current system.
    /// Example: ["x86_64-linux", "aarch64-darwin"]
    #[serde(default)]
    pub systems: Vec<String>,
}

impl Default for ManifestOptions {
    fn default() -> Self {
        Self {
            nixpkgs_channel: default_nixpkgs_channel(),
            allow_unfree: false,
            systems: Vec::new(),
        }
    }
}

fn default_nixpkgs_channel() -> String {
    "nixpkgs".to_string()
}

impl PackageEntry {
    /// Normalize any PackageEntry into a full PackageSpec.
    /// Shorthand `"14.1"` becomes `PackageSpec { version: Some("14.1"), .. }`.
    /// Shorthand `"*"` becomes `PackageSpec { version: None, .. }` (resolve to latest).
    pub fn to_spec(&self) -> PackageSpec {
        match self {
            PackageEntry::Short(version) => {
                let version = if version == "*" {
                    None
                } else {
                    Some(version.clone())
                };
                PackageSpec {
                    version,
                    ..Default::default()
                }
            }
            PackageEntry::Full(spec) => spec.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_package_entry_short_version() {
        let entry = PackageEntry::Short("14.1".to_string());
        let spec = entry.to_spec();
        assert_eq!(spec.version, Some("14.1".to_string()));
        assert_eq!(spec.systems, None);
        assert_eq!(spec.pkg_path, None);
    }

    #[test]
    fn test_package_entry_short_wildcard() {
        let entry = PackageEntry::Short("*".to_string());
        let spec = entry.to_spec();
        assert_eq!(spec.version, None);
    }

    #[test]
    fn test_package_entry_full() {
        let entry = PackageEntry::Full(PackageSpec {
            version: Some("3.12".to_string()),
            systems: Some(vec!["x86_64-linux".to_string()]),
            pkg_path: Some("python3".to_string()),
            priority: Some(5),
        });
        let spec = entry.to_spec();
        assert_eq!(spec.version, Some("3.12".to_string()));
        assert_eq!(spec.pkg_path, Some("python3".to_string()));
    }

    #[test]
    fn test_default_options() {
        let opts = ManifestOptions::default();
        assert_eq!(opts.nixpkgs_channel, "nixpkgs");
        assert!(!opts.allow_unfree);
        assert!(opts.systems.is_empty());
    }
}
