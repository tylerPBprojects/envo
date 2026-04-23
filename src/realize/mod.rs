//! Lazy realization engine for envo environments.
//!
//! This module generates executable shim scripts in `.envo/bin/` that proxy
//! calls to real Nix-packaged binaries. Shims enable the core "lazy fetch"
//! behavior: packages are not downloaded until a user actually invokes a tool.
//!
//! # Two-phase binary discovery
//!
//! 1. **Before realization**: We don't know what binaries a package provides
//!    (the store path doesn't exist locally yet). We generate a "meta-shim"
//!    using the package name. When the user runs it, the package is fetched
//!    and a `.needs-rescan` marker is created.
//!
//! 2. **After realization**: We scan the realized store path's `bin/` directory,
//!    generate individual shims for each binary found, and remove the meta-shim.
//!    The mapping is cached in `.envo/bin-map.json`.

pub mod shim;

use crate::lockfile::Lockfile;
use crate::manifest::ENVO_DIR;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Subdirectory within `.envo/` where shims are placed.
const BIN_DIR: &str = "bin";

/// File tracking the shim-to-package mapping.
const BIN_MAP_FILENAME: &str = "bin-map.json";

/// Marker file indicating that binary discovery needs to run.
const NEEDS_RESCAN_MARKER: &str = ".needs-rescan";

/// Errors that can occur during realization.
#[derive(Debug, Error)]
pub enum RealizeError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("no resolution found for package '{name}' on system '{system}'")]
    NoResolution { name: String, system: String },

    #[error("bin-map parse error: {0}")]
    BinMapParse(String),
}

/// Tracks which shims were generated and their target store paths.
///
/// This is persisted as `.envo/bin-map.json` and used to:
/// - Detect which shims need regeneration when the lockfile changes
/// - Avoid re-scanning binaries for already-discovered packages
/// - Clean up stale shims when packages are removed
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShimManifest {
    /// Maps shim binary names to their package and store path info.
    /// Key: binary name (e.g., "rg"), Value: shim metadata.
    pub shims: HashMap<String, ShimEntry>,

    /// Packages whose binaries have been discovered (store path realized
    /// and bin/ directory scanned).
    pub discovered_packages: HashMap<String, Vec<String>>,
}

/// Metadata for a single generated shim.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShimEntry {
    /// The package name this shim belongs to.
    pub package_name: String,

    /// The Nix store path the shim targets.
    pub store_path: String,

    /// Whether this is a meta-shim (pre-discovery) or a real binary shim.
    pub is_meta: bool,
}

impl ShimManifest {
    /// Create an empty shim manifest.
    pub fn new() -> Self {
        Self {
            shims: HashMap::new(),
            discovered_packages: HashMap::new(),
        }
    }

    /// Load a shim manifest from `.envo/bin-map.json`.
    pub fn load(project_dir: &Path) -> Result<Self, RealizeError> {
        let path = project_dir.join(ENVO_DIR).join(BIN_MAP_FILENAME);
        if !path.exists() {
            return Ok(Self::new());
        }
        let contents = std::fs::read_to_string(&path)?;
        let manifest: Self = serde_json::from_str(&contents)
            .map_err(|e| RealizeError::BinMapParse(e.to_string()))?;
        Ok(manifest)
    }

    /// Save the shim manifest to `.envo/bin-map.json`.
    pub fn save(&self, project_dir: &Path) -> Result<(), RealizeError> {
        let path = project_dir.join(ENVO_DIR).join(BIN_MAP_FILENAME);
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| RealizeError::BinMapParse(e.to_string()))?;
        std::fs::write(path, json)?;
        Ok(())
    }
}

impl Default for ShimManifest {
    fn default() -> Self {
        Self::new()
    }
}

/// The main realization engine.
///
/// Generates shim scripts in `.envo/bin/` based on a resolved lockfile.
pub struct Realizer {
    /// The project root directory (contains `.envo/`).
    project_dir: PathBuf,
}

impl Realizer {
    /// Create a new Realizer for the given project directory.
    pub fn new(project_dir: &Path) -> Self {
        Self {
            project_dir: project_dir.to_path_buf(),
        }
    }

    /// The path to the shim bin directory (`.envo/bin/`).
    pub fn shim_bin_dir(&self) -> PathBuf {
        self.project_dir.join(ENVO_DIR).join(BIN_DIR)
    }

    /// Generate shims for all packages in the lockfile on the given system.
    ///
    /// This is the primary entry point. It:
    /// 1. Creates the `.envo/bin/` directory if needed
    /// 2. Loads the existing bin-map (if any)
    /// 3. For each package in the lockfile, generates shims
    /// 4. Removes shims for packages no longer in the lockfile
    /// 5. Runs binary discovery for any realized packages
    /// 6. Saves the updated bin-map
    pub fn generate_shims(
        &self,
        lockfile: &Lockfile,
        system: &str,
    ) -> Result<ShimManifest, RealizeError> {
        let bin_dir = self.shim_bin_dir();
        std::fs::create_dir_all(&bin_dir)?;

        let mut manifest = ShimManifest::load(&self.project_dir)?;
        let nixpkgs_rev = lockfile.nixpkgs_revision();

        // Track which packages are still in the lockfile
        let mut current_packages: HashMap<String, String> = HashMap::new();

        // Generate shims for each package
        for (pkg_name, resolved) in &lockfile.packages {
            let resolution = resolved.systems.get(system).ok_or_else(|| {
                RealizeError::NoResolution {
                    name: pkg_name.clone(),
                    system: system.to_string(),
                }
            })?;

            let store_path = &resolution.store_path;
            let resolved_attr = &resolution.resolved_attr;
            current_packages.insert(pkg_name.clone(), store_path.clone());

            let store_path_obj = Path::new(store_path);

            // Check if the store path is already realized
            if store_path_obj.exists() {
                // Phase 2: store path exists — discover and generate per-binary shims
                self.generate_discovered_shims(
                    &bin_dir,
                    &mut manifest,
                    pkg_name,
                    store_path,
                    nixpkgs_rev,
                    resolved_attr,
                )?;
            } else {
                // Phase 1: store path not yet realized — generate meta-shim
                // But only if we haven't already discovered this package's binaries
                // (which would mean the store path was previously realized)
                if !manifest.discovered_packages.contains_key(pkg_name) {
                    self.generate_meta_shim(
                        &bin_dir,
                        &mut manifest,
                        pkg_name,
                        store_path,
                        nixpkgs_rev,
                        resolved_attr,
                    )?;
                }
            }
        }

        // Remove shims for packages no longer in the lockfile
        let stale_shims: Vec<String> = manifest
            .shims
            .iter()
            .filter(|(_, entry)| !current_packages.contains_key(&entry.package_name))
            .map(|(name, _)| name.clone())
            .collect();

        for shim_name in &stale_shims {
            let shim_path = bin_dir.join(shim_name);
            if shim_path.exists() {
                std::fs::remove_file(&shim_path)?;
            }
            manifest.shims.remove(shim_name);
        }

        // Remove discovered_packages entries for removed packages
        manifest
            .discovered_packages
            .retain(|name, _| current_packages.contains_key(name));

        // Check for rescan marker
        let rescan_marker = self.project_dir.join(ENVO_DIR).join(NEEDS_RESCAN_MARKER);
        if rescan_marker.exists() {
            // A shim was run and triggered realization — rescan all packages
            for (pkg_name, store_path) in &current_packages {
                let store_path_obj = Path::new(store_path.as_str());
                if store_path_obj.exists()
                    && !manifest.discovered_packages.contains_key(pkg_name)
                {
                    if let Some(resolution) = lockfile
                        .packages
                        .get(pkg_name)
                        .and_then(|r| r.systems.get(system))
                    {
                        self.generate_discovered_shims(
                            &bin_dir,
                            &mut manifest,
                            pkg_name,
                            store_path,
                            nixpkgs_rev,
                            &resolution.resolved_attr,
                        )?;
                    }
                }
            }
            let _ = std::fs::remove_file(&rescan_marker);
        }

        // Save the updated bin-map
        manifest.save(&self.project_dir)?;

        Ok(manifest)
    }

    /// Generate a meta-shim for a package (phase 1: before realization).
    fn generate_meta_shim(
        &self,
        bin_dir: &Path,
        manifest: &mut ShimManifest,
        package_name: &str,
        store_path: &str,
        nixpkgs_rev: &str,
        resolved_attr: &str,
    ) -> Result<(), RealizeError> {
        let script = shim::generate_meta_shim_script(
            store_path,
            package_name,
            nixpkgs_rev,
            resolved_attr,
        );

        let shim_path = bin_dir.join(package_name);
        write_executable(&shim_path, &script)?;

        manifest.shims.insert(
            package_name.to_string(),
            ShimEntry {
                package_name: package_name.to_string(),
                store_path: store_path.to_string(),
                is_meta: true,
            },
        );

        Ok(())
    }

    /// Generate per-binary shims for a discovered package (phase 2: after realization).
    fn generate_discovered_shims(
        &self,
        bin_dir: &Path,
        manifest: &mut ShimManifest,
        package_name: &str,
        store_path: &str,
        nixpkgs_rev: &str,
        resolved_attr: &str,
    ) -> Result<(), RealizeError> {
        let binaries = shim::discover_binaries(Path::new(store_path));

        // Remove the meta-shim if it exists
        let meta_shim_path = bin_dir.join(package_name);
        if meta_shim_path.exists() {
            if let Some(entry) = manifest.shims.get(package_name) {
                if entry.is_meta {
                    std::fs::remove_file(&meta_shim_path)?;
                    manifest.shims.remove(package_name);
                }
            }
        }

        // Generate a shim for each binary
        for binary in &binaries {
            let script = shim::generate_shim_script(
                store_path,
                binary,
                nixpkgs_rev,
                resolved_attr,
            );

            let shim_path = bin_dir.join(binary);
            write_executable(&shim_path, &script)?;

            manifest.shims.insert(
                binary.clone(),
                ShimEntry {
                    package_name: package_name.to_string(),
                    store_path: store_path.to_string(),
                    is_meta: false,
                },
            );
        }

        // Record that this package's binaries have been discovered
        manifest
            .discovered_packages
            .insert(package_name.to_string(), binaries);

        Ok(())
    }
}

/// Write content to a file and make it executable (chmod +x).
fn write_executable(path: &Path, content: &str) -> Result<(), RealizeError> {
    std::fs::write(path, content)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(path, perms)?;
    }

    Ok(())
}

// ── Unit tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockfile::{Lockfile, PackageResolution, ResolvedPackage, LOCKFILE_VERSION};

    fn make_test_lockfile(packages: Vec<(&str, &str, &str)>) -> Lockfile {
        let mut pkg_map = HashMap::new();
        for (name, store_path, attr) in packages {
            let mut systems = HashMap::new();
            systems.insert(
                "x86_64-linux".to_string(),
                PackageResolution {
                    store_path: store_path.to_string(),
                    resolved_attr: attr.to_string(),
                },
            );
            pkg_map.insert(name.to_string(), ResolvedPackage { systems });
        }

        Lockfile {
            version: LOCKFILE_VERSION,
            nixpkgs_revision: "test-revision".to_string(),
            manifest_hash: "test-hash".to_string(),
            packages: pkg_map,
        }
    }

    #[test]
    fn test_generate_meta_shims_for_unrealized_packages() {
        let tmp = tempfile::tempdir().unwrap();
        let envo_dir = tmp.path().join(ENVO_DIR);
        std::fs::create_dir_all(&envo_dir).unwrap();

        let lockfile = make_test_lockfile(vec![
            ("ripgrep", "/nix/store/fake-ripgrep", "ripgrep"),
            ("jq", "/nix/store/fake-jq", "jq"),
        ]);

        let realizer = Realizer::new(tmp.path());
        let manifest = realizer
            .generate_shims(&lockfile, "x86_64-linux")
            .unwrap();

        // Both should be meta-shims since store paths don't exist
        assert_eq!(manifest.shims.len(), 2);
        assert!(manifest.shims["ripgrep"].is_meta);
        assert!(manifest.shims["jq"].is_meta);

        // Shim files should exist and be executable
        let rg_shim = realizer.shim_bin_dir().join("ripgrep");
        assert!(rg_shim.exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&rg_shim).unwrap().permissions().mode();
            assert!(mode & 0o111 != 0, "shim should be executable");
        }
    }

    #[test]
    fn test_generate_discovered_shims_for_realized_packages() {
        let tmp = tempfile::tempdir().unwrap();
        let envo_dir = tmp.path().join(ENVO_DIR);
        std::fs::create_dir_all(&envo_dir).unwrap();

        // Create a fake "realized" store path with binaries
        let fake_store = tmp.path().join("fake-store");
        let fake_bin = fake_store.join("bin");
        std::fs::create_dir_all(&fake_bin).unwrap();
        std::fs::write(fake_bin.join("rg"), "fake-binary").unwrap();
        std::fs::write(fake_bin.join("ripgrep"), "fake-binary").unwrap();

        let store_path = fake_store.to_str().unwrap();
        let lockfile = make_test_lockfile(vec![(
            "ripgrep",
            store_path,
            "ripgrep",
        )]);

        let realizer = Realizer::new(tmp.path());
        let manifest = realizer
            .generate_shims(&lockfile, "x86_64-linux")
            .unwrap();

        // Should have per-binary shims, not meta-shims
        assert_eq!(manifest.shims.len(), 2);
        assert!(!manifest.shims["rg"].is_meta);
        assert!(!manifest.shims["ripgrep"].is_meta);

        // Should have recorded discovered binaries
        assert_eq!(
            manifest.discovered_packages["ripgrep"],
            vec!["rg", "ripgrep"]
        );

        // Verify shim content
        let rg_shim_path = realizer.shim_bin_dir().join("rg");
        let rg_content = std::fs::read_to_string(&rg_shim_path).unwrap();
        assert!(rg_content.contains("BINARY=\"rg\""));
        assert!(rg_content.contains(store_path));
    }

    #[test]
    fn test_stale_shims_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let envo_dir = tmp.path().join(ENVO_DIR);
        std::fs::create_dir_all(&envo_dir).unwrap();

        // First pass: generate shims for ripgrep and jq
        let lockfile1 = make_test_lockfile(vec![
            ("ripgrep", "/nix/store/fake-ripgrep", "ripgrep"),
            ("jq", "/nix/store/fake-jq", "jq"),
        ]);

        let realizer = Realizer::new(tmp.path());
        realizer
            .generate_shims(&lockfile1, "x86_64-linux")
            .unwrap();

        assert!(realizer.shim_bin_dir().join("ripgrep").exists());
        assert!(realizer.shim_bin_dir().join("jq").exists());

        // Second pass: only ripgrep (jq removed)
        let lockfile2 = make_test_lockfile(vec![
            ("ripgrep", "/nix/store/fake-ripgrep", "ripgrep"),
        ]);

        let manifest = realizer
            .generate_shims(&lockfile2, "x86_64-linux")
            .unwrap();

        assert!(realizer.shim_bin_dir().join("ripgrep").exists());
        assert!(!realizer.shim_bin_dir().join("jq").exists());
        assert_eq!(manifest.shims.len(), 1);
        assert!(!manifest.shims.contains_key("jq"));
    }

    #[test]
    fn test_bin_map_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let envo_dir = tmp.path().join(ENVO_DIR);
        std::fs::create_dir_all(&envo_dir).unwrap();

        let mut manifest = ShimManifest::new();
        manifest.shims.insert(
            "rg".to_string(),
            ShimEntry {
                package_name: "ripgrep".to_string(),
                store_path: "/nix/store/abc-ripgrep".to_string(),
                is_meta: false,
            },
        );
        manifest
            .discovered_packages
            .insert("ripgrep".to_string(), vec!["rg".to_string()]);

        manifest.save(tmp.path()).unwrap();
        let reloaded = ShimManifest::load(tmp.path()).unwrap();
        assert_eq!(manifest, reloaded);
    }

    #[test]
    fn test_shim_bin_dir_path() {
        let realizer = Realizer::new(Path::new("/home/user/project"));
        assert_eq!(
            realizer.shim_bin_dir(),
            PathBuf::from("/home/user/project/.envo/bin")
        );
    }

    #[test]
    fn test_missing_system_in_lockfile() {
        let tmp = tempfile::tempdir().unwrap();
        let envo_dir = tmp.path().join(ENVO_DIR);
        std::fs::create_dir_all(&envo_dir).unwrap();

        let lockfile = make_test_lockfile(vec![
            ("ripgrep", "/nix/store/fake-ripgrep", "ripgrep"),
        ]);

        let realizer = Realizer::new(tmp.path());
        let result = realizer.generate_shims(&lockfile, "aarch64-darwin");

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("no resolution found"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_regeneration_updates_store_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let envo_dir = tmp.path().join(ENVO_DIR);
        std::fs::create_dir_all(&envo_dir).unwrap();

        // First pass: old store path
        let lockfile1 = make_test_lockfile(vec![
            ("ripgrep", "/nix/store/old-ripgrep", "ripgrep"),
        ]);

        let realizer = Realizer::new(tmp.path());
        realizer
            .generate_shims(&lockfile1, "x86_64-linux")
            .unwrap();

        let shim_content1 =
            std::fs::read_to_string(realizer.shim_bin_dir().join("ripgrep")).unwrap();
        assert!(shim_content1.contains("/nix/store/old-ripgrep"));

        // Second pass: new store path (package updated)
        let lockfile2 = make_test_lockfile(vec![
            ("ripgrep", "/nix/store/new-ripgrep", "ripgrep"),
        ]);

        let manifest = realizer
            .generate_shims(&lockfile2, "x86_64-linux")
            .unwrap();

        let shim_content2 =
            std::fs::read_to_string(realizer.shim_bin_dir().join("ripgrep")).unwrap();
        assert!(shim_content2.contains("/nix/store/new-ripgrep"));
        assert!(!shim_content2.contains("/nix/store/old-ripgrep"));
        assert_eq!(
            manifest.shims["ripgrep"].store_path,
            "/nix/store/new-ripgrep"
        );
    }
}
