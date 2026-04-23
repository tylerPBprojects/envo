//! Integration tests for the realize module.
//!
//! Tests the shim generation, binary discovery, and stale-shim cleanup
//! from outside the crate using the public API.

use envo::lockfile::{Lockfile, PackageResolution, ResolvedPackage, LOCKFILE_VERSION};
use envo::realize::{Realizer, ShimManifest};
use std::collections::HashMap;
use tempfile::tempdir;

fn make_lockfile(packages: Vec<(&str, &str, &str)>) -> Lockfile {
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
        nixpkgs_revision: "test-rev-abc123".to_string(),
        manifest_hash: "test-hash".to_string(),
        packages: pkg_map,
    }
}

#[test]
fn test_end_to_end_shim_generation_unrealized() {
    let tmp = tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join(".envo")).unwrap();

    let lockfile = make_lockfile(vec![
        ("ripgrep", "/nix/store/fake-rg", "ripgrep"),
        ("jq", "/nix/store/fake-jq", "jq"),
    ]);

    let realizer = Realizer::new(tmp.path());
    let manifest = realizer.generate_shims(&lockfile, "x86_64-linux").unwrap();

    // Both should be meta-shims
    assert_eq!(manifest.shims.len(), 2);
    assert!(manifest.shims.contains_key("ripgrep"));
    assert!(manifest.shims.contains_key("jq"));

    // Shim files should exist
    assert!(realizer.shim_bin_dir().join("ripgrep").exists());
    assert!(realizer.shim_bin_dir().join("jq").exists());

    // Shim content should contain store paths
    let rg_content =
        std::fs::read_to_string(realizer.shim_bin_dir().join("ripgrep")).unwrap();
    assert!(rg_content.contains("/nix/store/fake-rg"));
    assert!(rg_content.contains("#!/usr/bin/env bash"));
}

#[test]
fn test_end_to_end_shim_generation_realized() {
    let tmp = tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join(".envo")).unwrap();

    // Create a fake realized store path
    let fake_store = tmp.path().join("nix-store-fake");
    let fake_bin = fake_store.join("bin");
    std::fs::create_dir_all(&fake_bin).unwrap();
    std::fs::write(fake_bin.join("rg"), "#!/bin/sh\necho rg").unwrap();
    std::fs::write(fake_bin.join("ripgrep"), "#!/bin/sh\necho ripgrep").unwrap();

    let store_path = fake_store.to_str().unwrap();
    let lockfile = make_lockfile(vec![("ripgrep", store_path, "ripgrep")]);

    let realizer = Realizer::new(tmp.path());
    let manifest = realizer.generate_shims(&lockfile, "x86_64-linux").unwrap();

    // Should have per-binary shims (discovered), not meta-shims
    assert!(manifest.shims.contains_key("rg"));
    assert!(manifest.shims.contains_key("ripgrep"));
    assert!(!manifest.shims["rg"].is_meta);

    // Should record discovered binaries
    assert!(manifest.discovered_packages.contains_key("ripgrep"));
    let binaries = &manifest.discovered_packages["ripgrep"];
    assert!(binaries.contains(&"rg".to_string()));
}

#[test]
fn test_remove_package_cleans_shims() {
    let tmp = tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join(".envo")).unwrap();

    let realizer = Realizer::new(tmp.path());

    // Pass 1: two packages
    let lf1 = make_lockfile(vec![
        ("ripgrep", "/nix/store/fake-rg", "ripgrep"),
        ("jq", "/nix/store/fake-jq", "jq"),
    ]);
    realizer.generate_shims(&lf1, "x86_64-linux").unwrap();
    assert!(realizer.shim_bin_dir().join("jq").exists());

    // Pass 2: one package removed
    let lf2 = make_lockfile(vec![("ripgrep", "/nix/store/fake-rg", "ripgrep")]);
    let manifest = realizer.generate_shims(&lf2, "x86_64-linux").unwrap();

    assert!(realizer.shim_bin_dir().join("ripgrep").exists());
    assert!(!realizer.shim_bin_dir().join("jq").exists());
    assert!(!manifest.shims.contains_key("jq"));
}

#[test]
fn test_bin_map_persists_across_calls() {
    let tmp = tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join(".envo")).unwrap();

    let lockfile = make_lockfile(vec![("ripgrep", "/nix/store/fake-rg", "ripgrep")]);
    let realizer = Realizer::new(tmp.path());

    realizer.generate_shims(&lockfile, "x86_64-linux").unwrap();

    // bin-map.json should exist
    let bin_map_path = tmp.path().join(".envo/bin-map.json");
    assert!(bin_map_path.exists());

    // Should be valid JSON
    let content = std::fs::read_to_string(&bin_map_path).unwrap();
    let reloaded: ShimManifest = serde_json::from_str(&content).unwrap();
    assert!(reloaded.shims.contains_key("ripgrep"));
}
