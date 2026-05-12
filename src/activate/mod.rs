//! Activation snapshot generation for envo environments.
//!
//! This module generates shell-sourceable scripts that activate an envo
//! environment by setting PATH (pointing at `.envo/bin/`), exporting
//! environment variables, and running on-activate hooks.
//!
//! **Core architectural commitment:** Activation NEVER spawns a subshell.
//! The generated script is `source`d or `eval`'d in the user's current shell.
//! This is what enables sub-100ms activation.

pub mod snapshot;

use crate::lockfile::Lockfile;
use crate::lockfile::resolver::detect_current_system;
use crate::manifest::{Manifest, ENVO_DIR};
use crate::realize::ShimManifest;
use snapshot::ShellType;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Snapshot filename (without extension — extension depends on shell type).
const SNAPSHOT_BASE: &str = "env-snapshot";

/// Errors that can occur during activation.
#[derive(Debug, Error)]
pub enum ActivateError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("cannot determine absolute path for project directory")]
    AbsolutePath,
}

/// The activation engine.
///
/// Generates shell-sourceable snapshots from the manifest, lockfile,
/// and shim manifest. Also provides raw key-value env vars for
/// programmatic consumers (VS Code extension, MCP server).
pub struct Activator {
    /// The project root directory (contains `.envo/`).
    project_dir: PathBuf,
}

impl Activator {
    /// Create a new Activator for the given project directory.
    pub fn new(project_dir: &Path) -> Self {
        Self {
            project_dir: project_dir.to_path_buf(),
        }
    }

    /// Path to the snapshot file for a given shell type.
    pub fn snapshot_path(&self, shell: ShellType) -> PathBuf {
        let filename = format!("{SNAPSHOT_BASE}.{}", shell.snapshot_extension());
        self.project_dir.join(ENVO_DIR).join(filename)
    }

    /// Generate the activation snapshot script as a string.
    ///
    /// This is the primary entry point. The returned string can be
    /// `source`d (bash/zsh) or `source`d (fish) to activate the environment.
    pub fn generate_snapshot(
        &self,
        manifest: &Manifest,
        lockfile: &Lockfile,
        _shim_manifest: &ShimManifest,
        shell: ShellType,
    ) -> Result<String, ActivateError> {
        let abs_project = self.absolute_project_dir()?;
        let bin_dir = abs_project.join(ENVO_DIR).join("bin");
        let bin_dir_str = bin_dir.to_string_lossy();
        let project_dir_str = abs_project.to_string_lossy();
        let lockfile_hash = &lockfile.manifest_hash;

        let vars = manifest.vars().clone();

        let hook_script = manifest
            .hooks()
            .and_then(|h| h.on_activate.as_deref());

        let system = detect_current_system();
        let python_paths = python_site_packages_paths(lockfile, &system);

        let script = match shell {
            ShellType::Bash | ShellType::Zsh => snapshot::render_posix_snapshot(
                &bin_dir_str,
                manifest.project_name(),
                &project_dir_str,
                &vars,
                lockfile_hash,
                hook_script,
                &python_paths,
            ),
            ShellType::Fish => snapshot::render_fish_snapshot(
                &bin_dir_str,
                manifest.project_name(),
                &project_dir_str,
                &vars,
                lockfile_hash,
                &python_paths,
            ),
        };

        Ok(script)
    }

    /// Generate the deactivation script as a string.
    pub fn generate_deactivation(
        &self,
        manifest: &Manifest,
        lockfile: Option<&Lockfile>,
        shell: ShellType,
    ) -> Result<String, ActivateError> {
        let abs_project = self.absolute_project_dir()?;
        let bin_dir = abs_project.join(ENVO_DIR).join("bin");
        let bin_dir_str = bin_dir.to_string_lossy();

        let var_keys: Vec<&str> = manifest.vars().keys().map(|s| s.as_str()).collect();

        let had_python_paths = lockfile.map_or(false, |lf| {
            let system = detect_current_system();
            !python_site_packages_paths(lf, &system).is_empty()
        });

        let script = match shell {
            ShellType::Bash | ShellType::Zsh => {
                snapshot::render_posix_deactivation(&bin_dir_str, &var_keys, had_python_paths)
            }
            ShellType::Fish => {
                snapshot::render_fish_deactivation(&bin_dir_str, &var_keys, had_python_paths)
            }
        };

        Ok(script)
    }

    /// Get the raw environment variables that activation would set.
    ///
    /// Returns a HashMap of all key-value pairs, including PATH modification,
    /// ENVO_ENV, ENVO_DIR, and user-defined vars. This is consumed by
    /// programmatic surfaces (VS Code extension, MCP server) that need
    /// structured data rather than a shell script.
    pub fn env_vars(
        &self,
        manifest: &Manifest,
        _lockfile: &Lockfile,
        _shim_manifest: &ShimManifest,
    ) -> Result<HashMap<String, String>, ActivateError> {
        let abs_project = self.absolute_project_dir()?;
        let bin_dir = abs_project.join(ENVO_DIR).join("bin");

        let mut env = HashMap::new();

        // Note: PATH is represented as just the bin_dir to prepend.
        // The consumer is responsible for prepending it to the actual PATH.
        env.insert(
            "ENVO_BIN_DIR".to_string(),
            bin_dir.to_string_lossy().to_string(),
        );
        env.insert("ENVO_ENV".to_string(), manifest.project_name().to_string());
        env.insert(
            "ENVO_DIR".to_string(),
            abs_project.to_string_lossy().to_string(),
        );

        // Add user-defined vars
        for (key, value) in manifest.vars() {
            env.insert(key.clone(), value.clone());
        }

        Ok(env)
    }

    /// Write the snapshot to disk at `.envo/env-snapshot.{sh,fish}`.
    pub fn save_snapshot(
        &self,
        manifest: &Manifest,
        lockfile: &Lockfile,
        shim_manifest: &ShimManifest,
        shell: ShellType,
    ) -> Result<PathBuf, ActivateError> {
        let script = self.generate_snapshot(manifest, lockfile, shim_manifest, shell)?;
        let path = self.snapshot_path(shell);
        std::fs::write(&path, &script)?;
        Ok(path)
    }

    /// Get the absolute path to the project directory.
    fn absolute_project_dir(&self) -> Result<PathBuf, ActivateError> {
        self.project_dir
            .canonicalize()
            .or_else(|_| {
                // canonicalize fails if the path doesn't exist yet.
                // Fall back to the path as-is if it's already absolute.
                if self.project_dir.is_absolute() {
                    Ok(self.project_dir.clone())
                } else {
                    std::env::current_dir()
                        .map(|cwd| cwd.join(&self.project_dir))
                        .map_err(|_| ActivateError::AbsolutePath)
                }
            })
    }
}

/// Collect `<store_path>/lib/python3.X/site-packages` paths for every Python
/// package in the lockfile on the current system.
///
/// Nixpkgs Python packages have store paths named like
/// `/nix/store/<hash>-python3.12-torch-2.11.0`. We detect them by looking for
/// the `-python3.` prefix (with a leading hyphen) in the path basename, which
/// distinguishes them from the Python interpreter itself
/// (`python3-3.12.13` has no dot after `python3`).
fn python_site_packages_paths(lockfile: &Lockfile, system: &str) -> Vec<String> {
    let mut paths: Vec<String> = lockfile
        .packages
        .values()
        .filter_map(|resolved| {
            let resolution = resolved.systems.get(system)?;
            let version = extract_python_minor_version(&resolution.store_path)?;
            Some(format!(
                "{}/lib/python{}/site-packages",
                resolution.store_path, version
            ))
        })
        .collect();
    paths.sort();
    paths
}

/// Extract the Python minor version string (e.g. `"3.12"`) from a Nix store
/// path that belongs to a Python package.
///
/// Matches `-python3.X` (hyphen-prefixed) in the basename, e.g.:
///   `/nix/store/<hash>-python3.12-torch-2.11.0`  → `"3.12"`
///   `/nix/store/<hash>-python3-3.12.13`           → `None`  (the interpreter)
fn extract_python_minor_version(store_path: &str) -> Option<String> {
    let basename = store_path.rsplit('/').next().unwrap_or("");
    let needle = "python3.";
    let pos = basename.find(needle)?;
    // Require a preceding '-' so we only match packages, not the interpreter
    if pos == 0 || basename.as_bytes()[pos - 1] != b'-' {
        return None;
    }
    let after = &basename[pos + needle.len()..];
    let minor: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    if minor.is_empty() {
        return None;
    }
    Some(format!("3.{minor}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockfile::{Lockfile, PackageResolution, ResolvedPackage, LOCKFILE_VERSION};
    use crate::realize::ShimManifest;

    fn test_manifest() -> Manifest {
        Manifest::from_str(
            r#"
[project]
name = "test-app"

[packages]
ripgrep = "*"

[vars]
EDITOR = "vim"
MY_VAR = "hello"

[hook]
on-activate = '''
echo "Welcome!"
'''
"#,
        )
        .unwrap()
    }

    fn test_lockfile() -> Lockfile {
        let mut systems = HashMap::new();
        systems.insert(
            "x86_64-linux".to_string(),
            PackageResolution {
                store_path: "/nix/store/abc-ripgrep".to_string(),
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
            nixpkgs_revision: "rev123".to_string(),
            manifest_hash: "hash456".to_string(),
            packages,
        }
    }

    fn test_shim_manifest() -> ShimManifest {
        ShimManifest::new()
    }

    #[test]
    fn test_generate_posix_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".envo/bin")).unwrap();

        let activator = Activator::new(tmp.path());
        let snapshot = activator
            .generate_snapshot(
                &test_manifest(),
                &test_lockfile(),
                &test_shim_manifest(),
                ShellType::Bash,
            )
            .unwrap();

        assert!(snapshot.contains("export PATH="));
        assert!(snapshot.contains(".envo/bin"));
        assert!(snapshot.contains("export ENVO_ENV=\"test-app\""));
        assert!(snapshot.contains("export EDITOR=\"vim\""));
        assert!(snapshot.contains("export MY_VAR=\"hello\""));
        assert!(snapshot.contains("echo \"Welcome!\""));
        assert!(snapshot.contains("ENVO_HOOK_DONE"));
        assert!(snapshot.contains("# lockfile_hash: hash456"));
    }

    #[test]
    fn test_generate_fish_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".envo/bin")).unwrap();

        let activator = Activator::new(tmp.path());
        let snapshot = activator
            .generate_snapshot(
                &test_manifest(),
                &test_lockfile(),
                &test_shim_manifest(),
                ShellType::Fish,
            )
            .unwrap();

        assert!(snapshot.contains("set -gx PATH"));
        assert!(snapshot.contains("set -gx ENVO_ENV test-app"));
        assert!(!snapshot.contains("export "));
        // Fish hooks are not supported in V1
        assert!(!snapshot.contains("Welcome!"));
    }

    #[test]
    fn test_generate_deactivation() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".envo/bin")).unwrap();

        let activator = Activator::new(tmp.path());
        let deact = activator
            .generate_deactivation(&test_manifest(), None, ShellType::Bash)
            .unwrap();

        assert!(deact.contains("unset ENVO_ENV"));
        assert!(deact.contains("unset EDITOR"));
        assert!(deact.contains("unset MY_VAR"));
    }

    #[test]
    fn test_env_vars_structured() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".envo/bin")).unwrap();

        let activator = Activator::new(tmp.path());
        let env = activator
            .env_vars(&test_manifest(), &test_lockfile(), &test_shim_manifest())
            .unwrap();

        assert_eq!(env.get("ENVO_ENV").unwrap(), "test-app");
        assert_eq!(env.get("EDITOR").unwrap(), "vim");
        assert_eq!(env.get("MY_VAR").unwrap(), "hello");
        assert!(env.contains_key("ENVO_BIN_DIR"));
        assert!(env.contains_key("ENVO_DIR"));
    }

    #[test]
    fn test_save_snapshot_creates_file() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".envo/bin")).unwrap();

        let activator = Activator::new(tmp.path());
        let path = activator
            .save_snapshot(
                &test_manifest(),
                &test_lockfile(),
                &test_shim_manifest(),
                ShellType::Bash,
            )
            .unwrap();

        assert!(path.exists());
        assert!(path.to_string_lossy().ends_with("env-snapshot.sh"));

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("export ENVO_ENV"));
    }

    #[test]
    fn test_snapshot_path_by_shell() {
        let activator = Activator::new(Path::new("/project"));
        assert!(activator
            .snapshot_path(ShellType::Bash)
            .to_string_lossy()
            .ends_with("env-snapshot.sh"));
        assert!(activator
            .snapshot_path(ShellType::Fish)
            .to_string_lossy()
            .ends_with("env-snapshot.fish"));
    }

    #[test]
    fn test_extract_python_minor_version_matches_package() {
        assert_eq!(
            extract_python_minor_version(
                "/nix/store/abc123-python3.12-torch-2.11.0"
            ),
            Some("3.12".to_string())
        );
        assert_eq!(
            extract_python_minor_version(
                "/nix/store/def456-python3.11-numpy-1.26.0"
            ),
            Some("3.11".to_string())
        );
    }

    #[test]
    fn test_extract_python_minor_version_skips_interpreter() {
        // The Python interpreter itself uses "python3-3.12.13" — no dot after "python3"
        assert_eq!(
            extract_python_minor_version(
                "/nix/store/jczbi6lb8vws7zc251v47bpijh805lyg-python3-3.12.13"
            ),
            None
        );
    }

    #[test]
    fn test_extract_python_minor_version_skips_non_python() {
        assert_eq!(
            extract_python_minor_version("/nix/store/abc-ripgrep-14.1.0"),
            None
        );
    }

    #[test]
    fn test_python_site_packages_paths_from_lockfile() {
        let mut systems = HashMap::new();
        systems.insert(
            "x86_64-linux".to_string(),
            PackageResolution {
                store_path: "/nix/store/abc-python3.12-torch-2.11.0".to_string(),
                resolved_attr: "python312Packages.torch".to_string(),
            },
        );
        let mut packages = HashMap::new();
        packages.insert("torch".to_string(), ResolvedPackage { systems });

        // Add the Python interpreter (should NOT produce a PYTHONPATH entry)
        let mut py_systems = HashMap::new();
        py_systems.insert(
            "x86_64-linux".to_string(),
            PackageResolution {
                store_path: "/nix/store/xyz-python3-3.12.13".to_string(),
                resolved_attr: "python312".to_string(),
            },
        );
        packages.insert("python".to_string(), ResolvedPackage { systems: py_systems });

        let lf = Lockfile {
            version: LOCKFILE_VERSION,
            nixpkgs_revision: "rev".to_string(),
            manifest_hash: "hash".to_string(),
            packages,
        };

        let paths = python_site_packages_paths(&lf, "x86_64-linux");
        assert_eq!(paths.len(), 1);
        assert_eq!(
            paths[0],
            "/nix/store/abc-python3.12-torch-2.11.0/lib/python3.12/site-packages"
        );
    }

    #[test]
    fn test_snapshot_includes_pythonpath_for_torch() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".envo/bin")).unwrap();

        // Build a lockfile with a Python package and the interpreter
        let system = detect_current_system();
        let mut packages = std::collections::HashMap::new();

        let mut torch_systems = std::collections::HashMap::new();
        torch_systems.insert(
            system.clone(),
            PackageResolution {
                store_path: "/nix/store/abc-python3.12-torch-2.11.0".to_string(),
                resolved_attr: "python312Packages.torch".to_string(),
            },
        );
        packages.insert("torch".to_string(), ResolvedPackage { systems: torch_systems });

        let mut py_systems = std::collections::HashMap::new();
        py_systems.insert(
            system.clone(),
            PackageResolution {
                store_path: "/nix/store/xyz-python3-3.12.13".to_string(),
                resolved_attr: "python312".to_string(),
            },
        );
        packages.insert("python".to_string(), ResolvedPackage { systems: py_systems });

        let lockfile = Lockfile {
            version: LOCKFILE_VERSION,
            nixpkgs_revision: "rev".to_string(),
            manifest_hash: "hash".to_string(),
            packages,
        };

        let manifest = Manifest::from_str(
            "[project]\nname = \"ml\"\n[packages]\npython = \"*\"\ntorch = \"*\"\n",
        ).unwrap();

        let activator = Activator::new(tmp.path());
        let snapshot = activator
            .generate_snapshot(&manifest, &lockfile, &ShimManifest::new(), ShellType::Bash)
            .unwrap();

        assert!(snapshot.contains("export PYTHONPATH="));
        assert!(snapshot.contains("python3.12-torch"));
        assert!(snapshot.contains("site-packages"));
        // Python interpreter should NOT be in PYTHONPATH
        assert!(!snapshot.contains("python3-3.12.13/lib/python"));
    }
}