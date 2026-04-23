//! Lockfile types, serialization, and staleness detection.
//!
//! The lockfile (`manifest.lock`) records the resolved Nix store paths for
//! every package in the manifest, per target system. It is a JSON file that
//! can be committed to version control to ensure reproducible environments.
//!
//! # Staleness
//!
//! The lockfile stores a SHA256 hash of the manifest content. When the manifest
//! changes (packages added, removed, or modified), the lockfile becomes stale
//! and needs to be re-resolved.

pub mod resolver;

use crate::manifest::{Manifest, ENVO_DIR};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Current lockfile schema version.
pub const LOCKFILE_VERSION: u32 = 1;

/// Lockfile filename within the envo directory.
pub const LOCKFILE_FILENAME: &str = "manifest.lock";

/// Errors that can occur during lockfile operations.
#[derive(Debug, Error)]
pub enum LockfileError {
    #[error("lockfile parse error: {0}")]
    Parse(String),

    #[error("lockfile IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("lockfile version mismatch: expected {expected}, got {got}")]
    VersionMismatch { expected: u32, got: u32 },

    #[error("resolution failed: {0}")]
    Resolution(String),

    #[error("no lockfile found (run `envo install` first)")]
    NotFound,
}

/// The top-level lockfile structure.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Lockfile {
    /// Schema version for forward compatibility.
    pub version: u32,

    /// The nixpkgs git revision used for resolution.
    pub nixpkgs_revision: String,

    /// SHA256 hash of the manifest.toml content at resolution time.
    /// Used for staleness detection.
    pub manifest_hash: String,

    /// Resolved packages, keyed by package name.
    pub packages: HashMap<String, ResolvedPackage>,
}

/// A resolved package with per-system store paths.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedPackage {
    /// Resolutions keyed by system string (e.g., "x86_64-linux").
    pub systems: HashMap<String, PackageResolution>,
}

/// Resolution details for a single package on a single system.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PackageResolution {
    /// The Nix store path for this package.
    pub store_path: String,

    /// The nixpkgs attribute path that was resolved.
    pub resolved_attr: String,
}

impl Lockfile {
    // ── Constructors ──────────────────────────────────────────────────

    /// Parse a lockfile from a JSON string.
    pub fn from_str(json: &str) -> Result<Self, LockfileError> {
        let lockfile: Self = serde_json::from_str(json)
            .map_err(|e| LockfileError::Parse(e.to_string()))?;

        if lockfile.version != LOCKFILE_VERSION {
            return Err(LockfileError::VersionMismatch {
                expected: LOCKFILE_VERSION,
                got: lockfile.version,
            });
        }

        Ok(lockfile)
    }

    /// Load a lockfile from `{dir}/.envo/manifest.lock`.
    pub fn load(dir: Option<&Path>) -> Result<Self, LockfileError> {
        let path = lockfile_path(dir);
        if !path.exists() {
            return Err(LockfileError::NotFound);
        }
        let contents = std::fs::read_to_string(&path)?;
        Self::from_str(&contents)
    }

    // ── Accessors (interface contract for downstream modules) ─────────

    /// Get the store path for a package on a specific system.
    /// Returns None if the package or system is not in the lockfile.
    pub fn get_store_path(&self, package_name: &str, system: &str) -> Option<&str> {
        self.packages
            .get(package_name)
            .and_then(|pkg| pkg.systems.get(system))
            .map(|res| res.store_path.as_str())
    }

    /// Iterate over all resolved packages as (name, system, store_path) tuples.
    pub fn all_packages(&self) -> impl Iterator<Item = (&str, &str, &str)> {
        self.packages.iter().flat_map(|(name, pkg)| {
            pkg.systems
                .iter()
                .map(move |(sys, res)| (name.as_str(), sys.as_str(), res.store_path.as_str()))
        })
    }

    /// Check if the lockfile is stale relative to the given manifest.
    ///
    /// The lockfile is stale if the manifest's content hash differs from
    /// the hash stored in the lockfile.
    pub fn is_stale(&self, manifest: &Manifest) -> bool {
        match resolver::compute_manifest_hash(manifest) {
            Ok(current_hash) => self.manifest_hash != current_hash,
            // If we can't compute the hash, treat as stale to be safe.
            Err(_) => true,
        }
    }

    /// The nixpkgs git revision used for this resolution.
    pub fn nixpkgs_revision(&self) -> &str {
        &self.nixpkgs_revision
    }

    /// List all package names in the lockfile.
    pub fn package_names(&self) -> Vec<&str> {
        self.packages.keys().map(|s| s.as_str()).collect()
    }

    // ── Serialization ─────────────────────────────────────────────────

    /// Serialize the lockfile to a pretty-printed JSON string.
    pub fn to_json_string(&self) -> Result<String, LockfileError> {
        serde_json::to_string_pretty(self)
            .map_err(|e| LockfileError::Parse(e.to_string()))
    }

    /// Write the lockfile to `{dir}/.envo/manifest.lock`.
    pub fn save(&self, dir: &Path) -> Result<(), LockfileError> {
        let path = dir.join(ENVO_DIR).join(LOCKFILE_FILENAME);
        let json = self.to_json_string()?;
        std::fs::write(path, json)?;
        Ok(())
    }
}

/// Get the path to the lockfile for a given project directory.
pub fn lockfile_path(dir: Option<&Path>) -> PathBuf {
    let base = dir.unwrap_or_else(|| Path::new("."));
    base.join(ENVO_DIR).join(LOCKFILE_FILENAME)
}

// ── Unit tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;

    fn sample_lockfile() -> Lockfile {
        let mut systems = HashMap::new();
        systems.insert(
            "x86_64-linux".to_string(),
            PackageResolution {
                store_path: "/nix/store/abc123-ripgrep-14.1.0".to_string(),
                resolved_attr: "ripgrep".to_string(),
            },
        );
        systems.insert(
            "aarch64-linux".to_string(),
            PackageResolution {
                store_path: "/nix/store/def456-ripgrep-14.1.0".to_string(),
                resolved_attr: "ripgrep".to_string(),
            },
        );

        let mut packages = HashMap::new();
        packages.insert(
            "ripgrep".to_string(),
            ResolvedPackage { systems },
        );

        Lockfile {
            version: LOCKFILE_VERSION,
            nixpkgs_revision: "abc123def456".to_string(),
            manifest_hash: "deadbeef".to_string(),
            packages,
        }
    }

    #[test]
    fn test_lockfile_json_round_trip() {
        let original = sample_lockfile();
        let json = original.to_json_string().unwrap();
        let parsed = Lockfile::from_str(&json).unwrap();
        assert_eq!(original, parsed);
    }

    #[test]
    fn test_lockfile_json_structure() {
        let lf = sample_lockfile();
        let json = lf.to_json_string().unwrap();

        // Verify the JSON has the expected structure
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["version"], 1);
        assert_eq!(value["nixpkgs_revision"], "abc123def456");
        assert!(value["packages"]["ripgrep"]["systems"]["x86_64-linux"]["store_path"]
            .as_str()
            .unwrap()
            .starts_with("/nix/store/"));
    }

    #[test]
    fn test_get_store_path() {
        let lf = sample_lockfile();
        assert_eq!(
            lf.get_store_path("ripgrep", "x86_64-linux"),
            Some("/nix/store/abc123-ripgrep-14.1.0")
        );
        assert_eq!(
            lf.get_store_path("ripgrep", "aarch64-linux"),
            Some("/nix/store/def456-ripgrep-14.1.0")
        );
        assert_eq!(lf.get_store_path("ripgrep", "x86_64-darwin"), None);
        assert_eq!(lf.get_store_path("nonexistent", "x86_64-linux"), None);
    }

    #[test]
    fn test_all_packages_iterator() {
        let lf = sample_lockfile();
        let all: Vec<_> = lf.all_packages().collect();
        assert_eq!(all.len(), 2); // ripgrep on 2 systems
        assert!(all.iter().all(|(name, _, _)| *name == "ripgrep"));
    }

    #[test]
    fn test_staleness_detection_not_stale() {
        let manifest_str = "[project]\nname = \"test\"\n\n[packages]\nripgrep = \"*\"\n";
        let manifest = Manifest::from_str(manifest_str).unwrap();
        let hash = resolver::compute_manifest_hash(&manifest).unwrap();

        let lf = Lockfile {
            version: LOCKFILE_VERSION,
            nixpkgs_revision: "abc".to_string(),
            manifest_hash: hash,
            packages: HashMap::new(),
        };

        assert!(!lf.is_stale(&manifest));
    }

    #[test]
    fn test_staleness_detection_stale() {
        let manifest = Manifest::from_str("[project]\nname = \"test\"\n").unwrap();

        let lf = Lockfile {
            version: LOCKFILE_VERSION,
            nixpkgs_revision: "abc".to_string(),
            manifest_hash: "wrong-hash".to_string(),
            packages: HashMap::new(),
        };

        assert!(lf.is_stale(&manifest));
    }

    #[test]
    fn test_version_mismatch_rejected() {
        let json = r#"{"version": 99, "nixpkgs_revision": "", "manifest_hash": "", "packages": {}}"#;
        let result = Lockfile::from_str(json);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("version mismatch"), "unexpected error: {err}");
    }

    #[test]
    fn test_save_and_reload() {
        let tmp = tempfile::tempdir().unwrap();
        let envo_dir = tmp.path().join(ENVO_DIR);
        std::fs::create_dir_all(&envo_dir).unwrap();

        let original = sample_lockfile();
        original.save(tmp.path()).unwrap();

        let reloaded = Lockfile::load(Some(tmp.path())).unwrap();
        assert_eq!(original, reloaded);
    }

    #[test]
    fn test_load_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let result = Lockfile::load(Some(tmp.path()));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("no lockfile found"), "unexpected error: {err}");
    }

    #[test]
    fn test_package_names() {
        let lf = sample_lockfile();
        let names = lf.package_names();
        assert_eq!(names, vec!["ripgrep"]);
    }

    #[test]
    fn test_nixpkgs_revision() {
        let lf = sample_lockfile();
        assert_eq!(lf.nixpkgs_revision(), "abc123def456");
    }
}
