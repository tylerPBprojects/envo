//! Manifest parsing, validation, and initialization for envo environments.
//!
//! This module provides the primary interface for working with envo manifests.
//! A manifest is a TOML file (`.envo/manifest.toml`) that declares the packages,
//! environment variables, hooks, services, and options for a development environment.
//!
//! # Examples
//!
//! ```rust
//! use envo::manifest::Manifest;
//!
//! let toml_str = r#"
//! [project]
//! name = "my-app"
//!
//! [packages]
//! ripgrep = "*"
//! python = { version = "3.12", pkg-path = "python3" }
//!
//! [vars]
//! EDITOR = "vim"
//! "#;
//!
//! let manifest = Manifest::from_str(toml_str).unwrap();
//! assert_eq!(manifest.project_name(), "my-app");
//! ```

pub mod schema;

use schema::{
    HookConfig, ManifestOptions, ManifestToml, PackageEntry, PackageSpec, ServiceConfig,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// The directory name where envo stores its state.
pub const ENVO_DIR: &str = ".envo";

/// The manifest filename within the envo directory.
pub const MANIFEST_FILENAME: &str = "manifest.toml";

/// Errors that can occur during manifest operations.
#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("manifest parse error: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("manifest serialization error: {0}")]
    Serialize(#[from] toml::ser::Error),

    #[error("validation error: {0}")]
    Validation(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("envo environment already exists at {0}")]
    AlreadyExists(PathBuf),

    #[error("no envo environment found (run `envo init` first)")]
    NotFound,
}

/// A parsed and validated envo manifest.
///
/// This is the primary interface that downstream modules (lockfile, realize,
/// activate) consume. It wraps the raw TOML types and provides accessor
/// methods that return normalized data structures.
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    raw: ManifestToml,
}

impl Manifest {
    // ── Constructors ──────────────────────────────────────────────────

    /// Parse a manifest from a TOML string and validate it.
    pub fn from_str(toml_str: &str) -> Result<Self, ManifestError> {
        let raw: ManifestToml = toml::from_str(toml_str)?;
        let manifest = Self { raw };
        manifest.validate()?;
        Ok(manifest)
    }

    /// Load a manifest from a file path.
    pub fn from_file(path: &Path) -> Result<Self, ManifestError> {
        let contents = std::fs::read_to_string(path)?;
        Self::from_str(&contents)
    }

    /// Load the manifest from the `.envo/manifest.toml` in the given directory.
    /// If `dir` is None, uses the current directory.
    pub fn load(dir: Option<&Path>) -> Result<Self, ManifestError> {
        let manifest_path = manifest_path(dir);
        if !manifest_path.exists() {
            return Err(ManifestError::NotFound);
        }
        Self::from_file(&manifest_path)
    }

    /// Initialize a new envo environment in the given directory.
    /// Creates `.envo/manifest.toml` with sensible defaults.
    ///
    /// The project name is derived from the directory name, falling back
    /// to "my-project" if the directory name can't be determined.
    pub fn init(dir: &Path) -> Result<Self, ManifestError> {
        let envo_dir = dir.join(ENVO_DIR);
        if envo_dir.exists() {
            return Err(ManifestError::AlreadyExists(envo_dir));
        }

        let project_name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("my-project")
            .to_string();

        let raw = ManifestToml {
            project: schema::ProjectConfig {
                name: project_name,
                description: None,
                version: None,
            },
            packages: HashMap::new(),
            vars: HashMap::new(),
            hook: None,
            services: HashMap::new(),
            options: ManifestOptions::default(),
        };

        let manifest = Self { raw };
        manifest.validate()?;

        // Create directory and write file
        std::fs::create_dir_all(&envo_dir)?;
        let toml_str = manifest.to_toml_string()?;
        std::fs::write(envo_dir.join(MANIFEST_FILENAME), toml_str)?;

        Ok(manifest)
    }

    // ── Accessors (interface contract for downstream modules) ─────────

    /// The project name.
    pub fn project_name(&self) -> &str {
        &self.raw.project.name
    }

    /// The project description, if set.
    pub fn project_description(&self) -> Option<&str> {
        self.raw.project.description.as_deref()
    }

    /// The project version, if set.
    pub fn project_version(&self) -> Option<&str> {
        self.raw.project.version.as_deref()
    }

    /// All declared packages, normalized to full PackageSpec form.
    /// Keys are the package names as written in the manifest.
    pub fn packages(&self) -> HashMap<String, PackageSpec> {
        self.raw
            .packages
            .iter()
            .map(|(name, entry)| (name.clone(), entry.to_spec()))
            .collect()
    }

    /// Environment variables to set on activation.
    pub fn vars(&self) -> &HashMap<String, String> {
        &self.raw.vars
    }

    /// Hook configuration, if any.
    pub fn hooks(&self) -> Option<&HookConfig> {
        self.raw.hook.as_ref()
    }

    /// Service declarations.
    pub fn services(&self) -> &HashMap<String, ServiceConfig> {
        &self.raw.services
    }

    /// Global manifest options (nixpkgs channel, allow-unfree, target systems).
    pub fn options(&self) -> &ManifestOptions {
        &self.raw.options
    }

    // ── Mutation ───────────────────────────────────────────────────────

    /// Add a package to the manifest. If it already exists, it is replaced.
    pub fn add_package(&mut self, name: &str, entry: PackageEntry) -> Result<(), ManifestError> {
        validate_package_name(name)?;
        self.raw.packages.insert(name.to_string(), entry);
        Ok(())
    }

    /// Remove a package from the manifest. Returns true if it was present.
    pub fn remove_package(&mut self, name: &str) -> bool {
        self.raw.packages.remove(name).is_some()
    }

    /// Set an environment variable.
    pub fn set_var(&mut self, key: &str, value: &str) {
        self.raw.vars.insert(key.to_string(), value.to_string());
    }

    // ── Serialization ─────────────────────────────────────────────────

    /// Serialize the manifest to a TOML string.
    pub fn to_toml_string(&self) -> Result<String, ManifestError> {
        let s = toml::to_string_pretty(&self.raw)?;
        Ok(s)
    }

    /// Write the manifest to `.envo/manifest.toml` in the given directory.
    pub fn save(&self, dir: &Path) -> Result<(), ManifestError> {
        let path = dir.join(ENVO_DIR).join(MANIFEST_FILENAME);
        let toml_str = self.to_toml_string()?;
        std::fs::write(path, toml_str)?;
        Ok(())
    }

    // ── Validation ────────────────────────────────────────────────────

    /// Validate the manifest for correctness.
    fn validate(&self) -> Result<(), ManifestError> {
        // Project name must be non-empty
        if self.raw.project.name.trim().is_empty() {
            return Err(ManifestError::Validation(
                "project.name must not be empty".to_string(),
            ));
        }

        // Validate all package names
        for name in self.raw.packages.keys() {
            validate_package_name(name)?;
        }

        // Validate system strings if present
        for sys in &self.raw.options.systems {
            validate_system_string(sys)?;
        }

        // Validate service configs
        for (name, svc) in &self.raw.services {
            if svc.command.trim().is_empty() {
                return Err(ManifestError::Validation(format!(
                    "service '{name}' has an empty command"
                )));
            }
        }

        Ok(())
    }
}

/// Validate that a package name contains only allowed characters.
/// Allowed: alphanumeric, hyphens, underscores, dots, and plus signs.
/// This matches what nixpkgs attribute paths typically look like.
fn validate_package_name(name: &str) -> Result<(), ManifestError> {
    if name.is_empty() {
        return Err(ManifestError::Validation(
            "package name must not be empty".to_string(),
        ));
    }

    // Must start with a letter or underscore
    let first = name.chars().next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        return Err(ManifestError::Validation(format!(
            "package name '{name}' must start with a letter or underscore"
        )));
    }

    // Remaining characters: alphanumeric, hyphen, underscore, dot, plus
    for ch in name.chars() {
        if !ch.is_ascii_alphanumeric() && !"-_.+".contains(ch) {
            return Err(ManifestError::Validation(format!(
                "package name '{name}' contains invalid character '{ch}'. \
                 Allowed: letters, digits, hyphens, underscores, dots, plus signs"
            )));
        }
    }

    Ok(())
}

/// Validate a system string (e.g., "x86_64-linux").
fn validate_system_string(sys: &str) -> Result<(), ManifestError> {
    let valid_systems = [
        "x86_64-linux",
        "aarch64-linux",
        "x86_64-darwin",
        "aarch64-darwin",
    ];
    if !valid_systems.contains(&sys) {
        return Err(ManifestError::Validation(format!(
            "unknown system '{sys}'. Valid systems: {}",
            valid_systems.join(", ")
        )));
    }
    Ok(())
}

/// Get the path to the manifest file for a given project directory.
pub fn manifest_path(dir: Option<&Path>) -> PathBuf {
    let base = dir.unwrap_or_else(|| Path::new("."));
    base.join(ENVO_DIR).join(MANIFEST_FILENAME)
}

/// Get the path to the `.envo` directory for a given project directory.
pub fn envo_dir(dir: Option<&Path>) -> PathBuf {
    let base = dir.unwrap_or_else(|| Path::new("."));
    base.join(ENVO_DIR)
}

// ── Unit tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_MANIFEST: &str = r#"
[project]
name = "test-project"
"#;

    const FULL_MANIFEST: &str = r#"
[project]
name = "my-app"
description = "A test application"
version = "1.0.0"

[packages]
ripgrep = "*"
python = { version = "3.12", pkg-path = "python3", systems = ["x86_64-linux"] }
jq = { priority = 5 }
nodejs = "20.15"

[vars]
EDITOR = "vim"
DATABASE_URL = "postgres://localhost/dev"

[hook]
on-activate = '''
echo "Welcome to my-app!"
'''

[services.postgres]
command = "postgres -D ./data/pg"
shutdown = "pg_ctl stop -D ./data/pg"

[services.redis]
command = "redis-server"

[options]
nixpkgs-channel = "nixpkgs/nixos-24.11"
allow-unfree = true
systems = ["x86_64-linux", "aarch64-linux"]
"#;

    #[test]
    fn test_parse_minimal_manifest() {
        let manifest = Manifest::from_str(MINIMAL_MANIFEST).unwrap();
        assert_eq!(manifest.project_name(), "test-project");
        assert!(manifest.packages().is_empty());
        assert!(manifest.vars().is_empty());
        assert!(manifest.hooks().is_none());
        assert!(manifest.services().is_empty());
        assert_eq!(manifest.options().nixpkgs_channel, "nixpkgs");
        assert!(!manifest.options().allow_unfree);
    }

    #[test]
    fn test_parse_full_manifest() {
        let manifest = Manifest::from_str(FULL_MANIFEST).unwrap();

        // Project
        assert_eq!(manifest.project_name(), "my-app");
        assert_eq!(
            manifest.project_description(),
            Some("A test application")
        );
        assert_eq!(manifest.project_version(), Some("1.0.0"));

        // Packages
        let pkgs = manifest.packages();
        assert_eq!(pkgs.len(), 4);

        let rg = &pkgs["ripgrep"];
        assert_eq!(rg.version, None); // "*" normalizes to None

        let py = &pkgs["python"];
        assert_eq!(py.version, Some("3.12".to_string()));
        assert_eq!(py.pkg_path, Some("python3".to_string()));
        assert_eq!(
            py.systems,
            Some(vec!["x86_64-linux".to_string()])
        );

        let jq = &pkgs["jq"];
        assert_eq!(jq.version, None);
        assert_eq!(jq.priority, Some(5));

        let node = &pkgs["nodejs"];
        assert_eq!(node.version, Some("20.15".to_string()));

        // Vars
        assert_eq!(manifest.vars().get("EDITOR").unwrap(), "vim");
        assert_eq!(
            manifest.vars().get("DATABASE_URL").unwrap(),
            "postgres://localhost/dev"
        );

        // Hooks
        let hooks = manifest.hooks().unwrap();
        assert!(hooks.on_activate.as_ref().unwrap().contains("Welcome"));

        // Services
        let svcs = manifest.services();
        assert_eq!(svcs.len(), 2);
        assert!(svcs["postgres"].command.contains("postgres"));
        assert!(svcs["postgres"].shutdown.is_some());
        assert_eq!(svcs["redis"].command, "redis-server");
        assert!(svcs["redis"].shutdown.is_none());

        // Options
        assert_eq!(manifest.options().nixpkgs_channel, "nixpkgs/nixos-24.11");
        assert!(manifest.options().allow_unfree);
        assert_eq!(manifest.options().systems.len(), 2);
    }

    #[test]
    fn test_parse_shorthand_package_syntax() {
        let toml_str = r#"
[project]
name = "test"

[packages]
ripgrep = "14.1"
jq = "*"
"#;
        let manifest = Manifest::from_str(toml_str).unwrap();
        let pkgs = manifest.packages();

        assert_eq!(
            pkgs["ripgrep"].version,
            Some("14.1".to_string())
        );
        // "*" means latest → normalized to None
        assert_eq!(pkgs["jq"].version, None);
    }

    #[test]
    fn test_validation_missing_project_name() {
        let toml_str = r#"
[project]
name = ""
"#;
        let result = Manifest::from_str(toml_str);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("project.name must not be empty"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_validation_no_project_section() {
        let toml_str = r#"
[packages]
ripgrep = "*"
"#;
        let result = Manifest::from_str(toml_str);
        assert!(result.is_err()); // TOML parse error — project is required
    }

    #[test]
    fn test_validation_invalid_package_name_starts_with_digit() {
        let toml_str = r#"
[project]
name = "test"

[packages]
123bad = "*"
"#;
        let result = Manifest::from_str(toml_str);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("must start with a letter"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_validation_invalid_package_name_special_chars() {
        let toml_str = r#"
[project]
name = "test"

[packages]
"bad/name" = "*"
"#;
        let result = Manifest::from_str(toml_str);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("invalid character"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_validation_valid_package_names() {
        // These should all pass validation
        let names = [
            "ripgrep",
            "python3",
            "gcc-unwrapped",
            "libiconv",
            "nodejs_20",
            "coreutils-full",
            "tree-sitter",
            "SDL2",
            "_private",
            "boost.python",
            "c++",
        ];
        for name in names {
            assert!(
                validate_package_name(name).is_ok(),
                "expected '{name}' to be valid"
            );
        }
    }

    #[test]
    fn test_validation_invalid_system() {
        let toml_str = r#"
[project]
name = "test"

[options]
systems = ["x86_64-windows"]
"#;
        let result = Manifest::from_str(toml_str);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("unknown system"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_validation_empty_service_command() {
        let toml_str = r#"
[project]
name = "test"

[services.bad]
command = ""
"#;
        let result = Manifest::from_str(toml_str);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("empty command"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_round_trip_serialization() {
        let original = Manifest::from_str(FULL_MANIFEST).unwrap();
        let serialized = original.to_toml_string().unwrap();
        let reparsed = Manifest::from_str(&serialized).unwrap();

        assert_eq!(original.project_name(), reparsed.project_name());
        assert_eq!(
            original.project_description(),
            reparsed.project_description()
        );
        assert_eq!(original.packages().len(), reparsed.packages().len());
        assert_eq!(original.vars(), reparsed.vars());
        assert_eq!(original.options(), reparsed.options());

        // Verify specific package data survives round-trip
        let orig_pkgs = original.packages();
        let rt_pkgs = reparsed.packages();
        assert_eq!(orig_pkgs["python"].version, rt_pkgs["python"].version);
        assert_eq!(orig_pkgs["python"].pkg_path, rt_pkgs["python"].pkg_path);
    }

    #[test]
    fn test_init_creates_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest = Manifest::init(tmp.path()).unwrap();

        // Verify file was created
        let manifest_path = tmp.path().join(ENVO_DIR).join(MANIFEST_FILENAME);
        assert!(manifest_path.exists());

        // Verify project name derived from dir name
        let dir_name = tmp.path().file_name().unwrap().to_str().unwrap();
        assert_eq!(manifest.project_name(), dir_name);

        // Verify we can reload it
        let reloaded = Manifest::load(Some(tmp.path())).unwrap();
        assert_eq!(reloaded.project_name(), dir_name);
    }

    #[test]
    fn test_init_fails_if_already_exists() {
        let tmp = tempfile::tempdir().unwrap();
        Manifest::init(tmp.path()).unwrap();

        // Second init should fail
        let result = Manifest::init(tmp.path());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("already exists"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_add_and_remove_package() {
        let mut manifest = Manifest::from_str(MINIMAL_MANIFEST).unwrap();
        assert!(manifest.packages().is_empty());

        manifest
            .add_package("ripgrep", PackageEntry::Short("*".to_string()))
            .unwrap();
        assert_eq!(manifest.packages().len(), 1);

        let removed = manifest.remove_package("ripgrep");
        assert!(removed);
        assert!(manifest.packages().is_empty());

        let removed_again = manifest.remove_package("ripgrep");
        assert!(!removed_again);
    }

    #[test]
    fn test_add_package_validates_name() {
        let mut manifest = Manifest::from_str(MINIMAL_MANIFEST).unwrap();
        let result = manifest.add_package("bad/name", PackageEntry::Short("*".to_string()));
        assert!(result.is_err());
    }

    #[test]
    fn test_set_var() {
        let mut manifest = Manifest::from_str(MINIMAL_MANIFEST).unwrap();
        manifest.set_var("EDITOR", "vim");
        assert_eq!(manifest.vars().get("EDITOR").unwrap(), "vim");
    }

    #[test]
    fn test_load_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let result = Manifest::load(Some(tmp.path()));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("no envo environment found"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_save_and_reload() {
        let tmp = tempfile::tempdir().unwrap();

        // Init, then modify, then save
        let mut manifest = Manifest::init(tmp.path()).unwrap();
        manifest
            .add_package("jq", PackageEntry::Short("*".to_string()))
            .unwrap();
        manifest.set_var("MY_VAR", "hello");
        manifest.save(tmp.path()).unwrap();

        // Reload and verify
        let reloaded = Manifest::load(Some(tmp.path())).unwrap();
        assert_eq!(reloaded.packages().len(), 1);
        assert!(reloaded.packages().contains_key("jq"));
        assert_eq!(reloaded.vars().get("MY_VAR").unwrap(), "hello");
    }
}
