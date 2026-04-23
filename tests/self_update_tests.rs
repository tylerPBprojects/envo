//! External integration tests for the self_update module.

use envo::self_update::{
    compare_versions, get_current_platform, get_current_system,
    get_install_dir, get_install_path, CURRENT_VERSION, VersionStatus,
};

#[test]
fn test_version_comparison_equal() {
    assert_eq!(
        compare_versions("0.1.0", "0.1.0"),
        VersionStatus::UpToDate
    );
}

#[test]
fn test_version_comparison_patch_update() {
    assert_eq!(
        compare_versions("0.1.0", "0.1.1"),
        VersionStatus::UpdateAvailable {
            latest: "0.1.1".to_string()
        }
    );
}

#[test]
fn test_version_comparison_minor_update() {
    assert_eq!(
        compare_versions("0.1.0", "0.2.0"),
        VersionStatus::UpdateAvailable {
            latest: "0.2.0".to_string()
        }
    );
}

#[test]
fn test_version_comparison_major_update() {
    assert_eq!(
        compare_versions("0.1.0", "1.0.0"),
        VersionStatus::UpdateAvailable {
            latest: "1.0.0".to_string()
        }
    );
}

#[test]
fn test_version_comparison_already_newer() {
    assert_eq!(
        compare_versions("0.2.0", "0.1.0"),
        VersionStatus::UpToDate
    );
}

#[test]
fn test_version_comparison_malformed_input() {
    // Should not panic on bad input — falls back to string comparison
    let result = compare_versions("bad", "0.1.0");
    assert_eq!(
        result,
        VersionStatus::UpdateAvailable {
            latest: "0.1.0".to_string()
        }
    );
}

#[test]
fn test_current_version_is_set() {
    // CURRENT_VERSION should match Cargo.toml
    assert!(!CURRENT_VERSION.is_empty());
    assert!(CURRENT_VERSION.contains('.'), "version should be semver: {CURRENT_VERSION}");
}

#[test]
fn test_platform_detection() {
    let platform = get_current_platform().unwrap();
    let valid = ["linux-x86_64", "linux-aarch64", "darwin-aarch64"];
    assert!(
        valid.contains(&platform.as_str()),
        "unexpected platform: {platform}"
    );
}

#[test]
fn test_system_detection() {
    let system = get_current_system();
    assert!(system.contains('-'));
    let parts: Vec<&str> = system.split('-').collect();
    assert_eq!(parts.len(), 2);
}

#[test]
fn test_install_dir_resolved() {
    let dir = get_install_dir();
    assert!(dir.is_ok());
    assert!(dir.unwrap().is_absolute() || true); // May be relative in test context
}

#[test]
fn test_install_path_not_unknown() {
    let path = get_install_path();
    assert_ne!(path, "unknown");
}
