//! Integration tests for the manifest module.
//!
//! These test the public API from outside the crate, the same way
//! downstream modules and the CLI will use it.

use envo::manifest::schema::{PackageEntry, PackageSpec};
use envo::manifest::Manifest;
use tempfile::tempdir;

#[test]
fn test_full_workflow_init_modify_save_reload() {
    let tmp = tempdir().unwrap();

    // 1. Init
    let mut manifest = Manifest::init(tmp.path()).unwrap();
    let original_name = manifest.project_name().to_string();
    assert!(!original_name.is_empty());

    // 2. Add packages using both shorthand and full syntax
    manifest
        .add_package("ripgrep", PackageEntry::Short("*".to_string()))
        .unwrap();
    manifest
        .add_package(
            "python",
            PackageEntry::Full(PackageSpec {
                version: Some("3.12".to_string()),
                pkg_path: Some("python3".to_string()),
                systems: Some(vec!["x86_64-linux".to_string()]),
                priority: None,
            }),
        )
        .unwrap();

    // 3. Add vars
    manifest.set_var("EDITOR", "vim");
    manifest.set_var("RUST_LOG", "debug");

    // 4. Save
    manifest.save(tmp.path()).unwrap();

    // 5. Reload and verify
    let reloaded = Manifest::load(Some(tmp.path())).unwrap();
    assert_eq!(reloaded.project_name(), original_name);

    let pkgs = reloaded.packages();
    assert_eq!(pkgs.len(), 2);
    assert_eq!(pkgs["ripgrep"].version, None); // "*" → None
    assert_eq!(pkgs["python"].version, Some("3.12".to_string()));
    assert_eq!(pkgs["python"].pkg_path, Some("python3".to_string()));

    assert_eq!(reloaded.vars().get("EDITOR").unwrap(), "vim");
    assert_eq!(reloaded.vars().get("RUST_LOG").unwrap(), "debug");
}

#[test]
fn test_complex_manifest_from_string() {
    let toml = r#"
[project]
name = "ml-project"
description = "GPU-accelerated ML environment"

[packages]
python = { version = "3.12", pkg-path = "python3" }
cudatoolkit = { pkg-path = "cudaPackages.cudatoolkit", systems = ["x86_64-linux"] }
ripgrep = "*"
jq = "1.7"
nodejs = { priority = 10 }

[vars]
CUDA_HOME = "/nix/store/fake-cuda"
PYTHONPATH = "./src"

[hook]
on-activate = '''
echo "Setting up ML environment..."
export VIRTUAL_ENV="$PWD/.venv"
'''

[services.jupyter]
command = "jupyter lab --port 8888"
shutdown = "jupyter notebook stop 8888"

[services.tensorboard]
command = "tensorboard --logdir ./logs"

[options]
nixpkgs-channel = "nixpkgs/nixos-24.11"
allow-unfree = true
systems = ["x86_64-linux", "aarch64-linux"]
"#;

    let manifest = Manifest::from_str(toml).unwrap();
    assert_eq!(manifest.project_name(), "ml-project");
    assert_eq!(manifest.packages().len(), 5);
    assert!(manifest.options().allow_unfree);
    assert_eq!(manifest.services().len(), 2);
    assert!(manifest.hooks().unwrap().on_activate.is_some());

    // Verify specific package details survived parsing
    let pkgs = manifest.packages();
    assert_eq!(
        pkgs["cudatoolkit"].pkg_path,
        Some("cudaPackages.cudatoolkit".to_string())
    );
    assert_eq!(
        pkgs["cudatoolkit"].systems,
        Some(vec!["x86_64-linux".to_string()])
    );
}
