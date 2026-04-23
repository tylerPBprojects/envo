//! Nix evaluation and resolution logic.
//!
//! This module wraps `nix` CLI commands to resolve package attribute paths
//! into concrete Nix store paths. It provides a `NixEvaluator` struct that
//! constructs and executes nix commands, and a `resolve_manifest` function
//! that resolves all packages in a manifest into a lockfile.

use crate::manifest::Manifest;
use std::collections::HashMap;
use std::process::Command;
use std::time::Duration;
use thiserror::Error;

use super::{Lockfile, LockfileError, PackageResolution, ResolvedPackage, LOCKFILE_VERSION};

/// Timeout for individual nix evaluation commands.
const NIX_EVAL_TIMEOUT_SECS: u64 = 30;

/// Errors specific to Nix evaluation.
#[derive(Debug, Error)]
pub enum NixError {
    #[error("nix is not installed or not in PATH")]
    NixNotFound,

    #[error("nix evaluation failed for '{attr}': {message}")]
    EvalFailed { attr: String, message: String },

    #[error("package '{name}' not found in nixpkgs (attr: {attr})")]
    PackageNotFound { name: String, attr: String },

    #[error("package '{name}' has an unfree license. Set allow-unfree = true in [options]")]
    UnfreePackage { name: String },

    #[error("nix command timed out after {seconds}s")]
    Timeout { seconds: u64 },

    #[error("failed to parse nix output: {0}")]
    ParseError(String),

    #[error("IO error running nix: {0}")]
    Io(#[from] std::io::Error),
}

/// Wraps `nix` CLI commands for evaluating packages and fetching metadata.
///
/// Supports a `dry_run` mode where commands are recorded but not executed,
/// which is used for unit testing command construction.
#[derive(Debug, Clone)]
pub struct NixEvaluator {
    /// If true, commands are recorded but not executed.
    dry_run: bool,

    /// Commands that would be executed (populated in dry_run mode).
    recorded_commands: Vec<Vec<String>>,
}

/// The result of a nix evaluation — a store path.
#[derive(Debug, Clone)]
pub struct EvalResult {
    pub store_path: String,
}

/// Metadata about the nixpkgs flake being used.
#[derive(Debug, Clone)]
pub struct FlakeMetadata {
    pub revision: String,
}

impl NixEvaluator {
    /// Create a new NixEvaluator that executes commands.
    pub fn new() -> Self {
        Self {
            dry_run: false,
            recorded_commands: Vec::new(),
        }
    }

    /// Create a NixEvaluator in dry-run mode for testing.
    /// Commands are recorded in `recorded_commands` but not executed.
    pub fn dry_run() -> Self {
        Self {
            dry_run: true,
            recorded_commands: Vec::new(),
        }
    }

    /// Get the commands that were recorded during dry-run mode.
    pub fn recorded_commands(&self) -> &[Vec<String>] {
        &self.recorded_commands
    }

    /// Check that nix is installed and accessible.
    pub fn check_nix_available(&self) -> Result<(), NixError> {
        if self.dry_run {
            return Ok(());
        }

        let output = Command::new("nix")
            .arg("--version")
            .output()
            .map_err(|_| NixError::NixNotFound)?;

        if !output.status.success() {
            return Err(NixError::NixNotFound);
        }
        Ok(())
    }

    /// Evaluate a package attribute path to get its store path.
    ///
    /// Runs: `nix eval --json {flake_ref}#legacyPackages.{system}.{attr_path}.outPath`
    ///
    /// If `allow_unfree` is true, sets `NIXPKGS_ALLOW_UNFREE=1` and passes `--impure`.
    pub fn eval_package(
        &mut self,
        flake_ref: &str,
        system: &str,
        attr_path: &str,
        allow_unfree: bool,
    ) -> Result<EvalResult, NixError> {
        let installable = format!(
            "{flake_ref}#legacyPackages.{system}.{attr_path}.outPath"
        );

        let mut args = vec![
            "eval".to_string(),
            "--json".to_string(),
        ];

        if allow_unfree {
            args.push("--impure".to_string());
        }

        args.push(installable.clone());

        if self.dry_run {
            self.recorded_commands.push(
                std::iter::once("nix".to_string()).chain(args).collect()
            );
            return Ok(EvalResult {
                store_path: format!("/nix/store/dry-run-hash-{attr_path}"),
            });
        }

        let mut cmd = Command::new("nix");
        cmd.args(&args);

        if allow_unfree {
            cmd.env("NIXPKGS_ALLOW_UNFREE", "1");
        }

        let output = run_with_timeout(&mut cmd, Duration::from_secs(NIX_EVAL_TIMEOUT_SECS))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();

            if stderr.contains("unfree license") || stderr.contains("Refusing to evaluate") {
                return Err(NixError::UnfreePackage {
                    name: attr_path.to_string(),
                });
            }

            if stderr.contains("does not provide attribute")
                || (stderr.contains("attribute '") && stderr.contains("' missing"))
            {
                return Err(NixError::PackageNotFound {
                    name: attr_path.to_string(),
                    attr: installable,
                });
            }

            return Err(NixError::EvalFailed {
                attr: installable,
                message: stderr,
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let store_path: String = serde_json::from_str(stdout.trim())
            .map_err(|e| NixError::ParseError(format!("failed to parse store path: {e}")))?;

        Ok(EvalResult { store_path })
    }

    /// Get the nixpkgs flake revision currently being used.
    ///
    /// Runs: `nix flake metadata {flake_ref} --json`
    pub fn get_flake_metadata(&mut self, flake_ref: &str) -> Result<FlakeMetadata, NixError> {
        let args = vec![
            "flake".to_string(),
            "metadata".to_string(),
            flake_ref.to_string(),
            "--json".to_string(),
        ];

        if self.dry_run {
            self.recorded_commands.push(
                std::iter::once("nix".to_string()).chain(args).collect()
            );
            return Ok(FlakeMetadata {
                revision: "dry-run-revision".to_string(),
            });
        }

        let mut cmd = Command::new("nix");
        cmd.args(&args);

        let output = run_with_timeout(&mut cmd, Duration::from_secs(NIX_EVAL_TIMEOUT_SECS))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(NixError::EvalFailed {
                attr: flake_ref.to_string(),
                message: stderr,
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let metadata: serde_json::Value = serde_json::from_str(stdout.trim())
            .map_err(|e| NixError::ParseError(format!("failed to parse flake metadata: {e}")))?;

        let revision = metadata
            .get("revision")
            .and_then(|v| v.as_str())
            .ok_or_else(|| NixError::ParseError("no 'revision' field in flake metadata".into()))?
            .to_string();

        Ok(FlakeMetadata { revision })
    }
}

/// Resolve all packages in a manifest into a lockfile.
///
/// For each package in the manifest, resolves it against each target system
/// using the NixEvaluator. If an existing lockfile is provided, only
/// re-resolves packages that have changed (partial re-resolution).
pub fn resolve_manifest(
    manifest: &Manifest,
    evaluator: &mut NixEvaluator,
    existing_lockfile: Option<&Lockfile>,
) -> Result<Lockfile, LockfileError> {
    let options = manifest.options();
    let flake_ref = &options.nixpkgs_channel;

    let systems = if options.systems.is_empty() {
        vec![detect_current_system()]
    } else {
        options.systems.clone()
    };

    let metadata = evaluator
        .get_flake_metadata(flake_ref)
        .map_err(|e| LockfileError::Resolution(e.to_string()))?;

    let manifest_hash = compute_manifest_hash(manifest)?;
    let packages_spec = manifest.packages();

    let existing_packages = existing_lockfile.map(|lf| &lf.packages);

    let mut resolved_packages: HashMap<String, ResolvedPackage> = HashMap::new();

    for (pkg_name, spec) in &packages_spec {
        let attr_path = spec.pkg_path.as_deref().unwrap_or(pkg_name.as_str());

        let pkg_systems: Vec<&str> = if let Some(ref pkg_sys) = spec.systems {
            pkg_sys.iter().map(|s| s.as_str()).collect()
        } else {
            systems.iter().map(|s| s.as_str()).collect()
        };

        // Check if we can reuse the existing resolution
        if let Some(existing) = existing_packages.and_then(|p| p.get(pkg_name)) {
            let all_systems_present = pkg_systems.iter().all(|sys| {
                existing
                    .systems
                    .get(*sys)
                    .map(|r| r.resolved_attr == attr_path)
                    .unwrap_or(false)
            });

            if all_systems_present {
                resolved_packages.insert(pkg_name.clone(), existing.clone());
                continue;
            }
        }

        let mut system_resolutions: HashMap<String, PackageResolution> = HashMap::new();

        for system in &pkg_systems {
            let result = evaluator
                .eval_package(flake_ref, system, attr_path, options.allow_unfree)
                .map_err(|e| LockfileError::Resolution(e.to_string()))?;

            system_resolutions.insert(
                system.to_string(),
                PackageResolution {
                    store_path: result.store_path,
                    resolved_attr: attr_path.to_string(),
                },
            );
        }

        resolved_packages.insert(
            pkg_name.clone(),
            ResolvedPackage {
                systems: system_resolutions,
            },
        );
    }

    Ok(Lockfile {
        version: LOCKFILE_VERSION,
        nixpkgs_revision: metadata.revision,
        manifest_hash,
        packages: resolved_packages,
    })
}

/// Compute a SHA256 hash of the manifest's TOML content.
pub(crate) fn compute_manifest_hash(manifest: &Manifest) -> Result<String, LockfileError> {
    use sha2::{Digest, Sha256};

    let toml_str = manifest
        .to_toml_string()
        .map_err(|e| LockfileError::Resolution(format!("failed to serialize manifest: {e}")))?;

    let mut hasher = Sha256::new();
    hasher.update(toml_str.as_bytes());
    let result = hasher.finalize();
    Ok(format!("{:x}", result))
}

/// Detect the current system's Nix platform string.
pub fn detect_current_system() -> String {
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

/// Run a command with a timeout.
fn run_with_timeout(
    cmd: &mut Command,
    timeout: Duration,
) -> Result<std::process::Output, NixError> {
    let child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    let start = std::time::Instant::now();
    let mut child = child;

    loop {
        match child.try_wait()? {
            Some(_status) => {
                let output = child.wait_with_output()?;
                return Ok(output);
            }
            None => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(NixError::Timeout {
                        seconds: timeout.as_secs(),
                    });
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

// ── Unit tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_manifest(packages: Vec<(&str, &str)>) -> Manifest {
        let mut toml = String::from("[project]\nname = \"test\"\n\n[packages]\n");
        for (name, version) in packages {
            toml.push_str(&format!("{name} = \"{version}\"\n"));
        }
        Manifest::from_str(&toml).unwrap()
    }

    #[test]
    fn test_dry_run_eval_records_command() {
        let mut eval = NixEvaluator::dry_run();
        let result = eval
            .eval_package("nixpkgs", "x86_64-linux", "ripgrep", false)
            .unwrap();

        assert!(result.store_path.contains("dry-run"));

        let cmds = eval.recorded_commands();
        assert_eq!(cmds.len(), 1);

        let cmd = &cmds[0];
        assert_eq!(cmd[0], "nix");
        assert_eq!(cmd[1], "eval");
        assert_eq!(cmd[2], "--json");
        assert_eq!(
            cmd[3],
            "nixpkgs#legacyPackages.x86_64-linux.ripgrep.outPath"
        );
    }

    #[test]
    fn test_dry_run_eval_with_unfree() {
        let mut eval = NixEvaluator::dry_run();
        eval.eval_package("nixpkgs", "x86_64-linux", "cudatoolkit", true)
            .unwrap();

        let cmd = &eval.recorded_commands()[0];
        assert!(cmd.contains(&"--impure".to_string()));
    }

    #[test]
    fn test_dry_run_eval_custom_channel() {
        let mut eval = NixEvaluator::dry_run();
        eval.eval_package("nixpkgs/nixos-24.11", "aarch64-linux", "python3", false)
            .unwrap();

        let cmd = &eval.recorded_commands()[0];
        assert_eq!(
            cmd[3],
            "nixpkgs/nixos-24.11#legacyPackages.aarch64-linux.python3.outPath"
        );
    }

    #[test]
    fn test_dry_run_flake_metadata() {
        let mut eval = NixEvaluator::dry_run();
        let meta = eval.get_flake_metadata("nixpkgs").unwrap();
        assert_eq!(meta.revision, "dry-run-revision");

        let cmd = &eval.recorded_commands()[0];
        assert_eq!(cmd[0], "nix");
        assert_eq!(cmd[1], "flake");
        assert_eq!(cmd[2], "metadata");
        assert_eq!(cmd[3], "nixpkgs");
        assert_eq!(cmd[4], "--json");
    }

    #[test]
    fn test_resolve_manifest_dry_run() {
        let manifest = test_manifest(vec![("ripgrep", "*"), ("jq", "*")]);
        let mut eval = NixEvaluator::dry_run();

        let lockfile = resolve_manifest(&manifest, &mut eval, None).unwrap();

        assert_eq!(lockfile.packages.len(), 2);
        assert!(lockfile.packages.contains_key("ripgrep"));
        assert!(lockfile.packages.contains_key("jq"));

        let current_sys = detect_current_system();
        let rg = &lockfile.packages["ripgrep"];
        assert!(rg.systems.contains_key(&current_sys));
    }

    #[test]
    fn test_resolve_partial_reuses_existing() {
        let manifest1 = test_manifest(vec![("ripgrep", "*")]);
        let mut eval1 = NixEvaluator::dry_run();
        let lockfile1 = resolve_manifest(&manifest1, &mut eval1, None).unwrap();

        let manifest2 = test_manifest(vec![("ripgrep", "*"), ("jq", "*")]);
        let mut eval2 = NixEvaluator::dry_run();
        let lockfile2 = resolve_manifest(&manifest2, &mut eval2, Some(&lockfile1)).unwrap();

        assert_eq!(lockfile2.packages.len(), 2);

        // Should be 2 commands: 1 metadata + 1 jq eval (ripgrep reused)
        let cmds = eval2.recorded_commands();
        assert_eq!(cmds.len(), 2, "expected 2 commands (metadata + jq), got {}", cmds.len());
    }

    #[test]
    fn test_detect_current_system() {
        let sys = detect_current_system();
        assert!(sys.contains('-'), "system string should contain a hyphen: {sys}");
        let parts: Vec<&str> = sys.split('-').collect();
        assert_eq!(parts.len(), 2);
    }

    #[test]
    fn test_manifest_hash_deterministic() {
        let manifest = test_manifest(vec![("ripgrep", "*")]);
        let hash1 = compute_manifest_hash(&manifest).unwrap();
        let hash2 = compute_manifest_hash(&manifest).unwrap();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_manifest_hash_changes_on_modification() {
        let manifest1 = test_manifest(vec![("ripgrep", "*")]);
        let manifest2 = test_manifest(vec![("ripgrep", "*"), ("jq", "*")]);
        let hash1 = compute_manifest_hash(&manifest1).unwrap();
        let hash2 = compute_manifest_hash(&manifest2).unwrap();
        assert_ne!(hash1, hash2);
    }
}
