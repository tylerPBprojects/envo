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

        let script = match shell {
            ShellType::Bash | ShellType::Zsh => snapshot::render_posix_snapshot(
                &bin_dir_str,
                manifest.project_name(),
                &project_dir_str,
                &vars,
                lockfile_hash,
                hook_script,
            ),
            ShellType::Fish => snapshot::render_fish_snapshot(
                &bin_dir_str,
                manifest.project_name(),
                &project_dir_str,
                &vars,
                lockfile_hash,
            ),
        };

        Ok(script)
    }

    /// Generate the deactivation script as a string.
    pub fn generate_deactivation(
        &self,
        manifest: &Manifest,
        shell: ShellType,
    ) -> Result<String, ActivateError> {
        let abs_project = self.absolute_project_dir()?;
        let bin_dir = abs_project.join(ENVO_DIR).join("bin");
        let bin_dir_str = bin_dir.to_string_lossy();

        let var_keys: Vec<&str> = manifest.vars().keys().map(|s| s.as_str()).collect();

        let script = match shell {
            ShellType::Bash | ShellType::Zsh => {
                snapshot::render_posix_deactivation(&bin_dir_str, &var_keys)
            }
            ShellType::Fish => {
                snapshot::render_fish_deactivation(&bin_dir_str, &var_keys)
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
            .generate_deactivation(&test_manifest(), ShellType::Bash)
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
}