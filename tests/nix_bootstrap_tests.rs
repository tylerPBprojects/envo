//! External integration tests for the nix_bootstrap module.

use envo::nix_bootstrap::{
    detect_nix, format_nix_status, nix_status_to_json, parse_nix_version, NixStatus,
};

#[test]
fn test_parse_determinate_version_string() {
    let (version, is_det) = parse_nix_version("nix (Determinate Nix 3.18.0) 2.33.3");
    assert_eq!(version, "2.33.3");
    assert!(is_det);
}

#[test]
fn test_parse_standard_version_string() {
    let (version, is_det) = parse_nix_version("nix (Nix) 2.18.1");
    assert_eq!(version, "2.18.1");
    assert!(!is_det);
}

#[test]
fn test_parse_minimal_version_string() {
    let (version, is_det) = parse_nix_version("nix 2.24.0");
    assert_eq!(version, "2.24.0");
    assert!(!is_det);
}

#[test]
fn test_format_determinate_status() {
    let status = NixStatus::Available {
        version: "2.33.3".to_string(),
        is_determinate: true,
    };
    let formatted = format_nix_status(&status);
    assert!(formatted.contains("Determinate"));
    assert!(formatted.contains("2.33.3"));
}

#[test]
fn test_format_standard_status() {
    let status = NixStatus::Available {
        version: "2.18.1".to_string(),
        is_determinate: false,
    };
    let formatted = format_nix_status(&status);
    assert_eq!(formatted, "nix 2.18.1");
}

#[test]
fn test_format_not_installed() {
    let formatted = format_nix_status(&NixStatus::NotInstalled);
    assert_eq!(formatted, "not installed");
}

#[test]
fn test_json_available() {
    let status = NixStatus::Available {
        version: "2.33.3".to_string(),
        is_determinate: true,
    };
    let json = nix_status_to_json(&status);
    assert_eq!(json["installed"], true);
    assert_eq!(json["version"], "2.33.3");
    assert_eq!(json["determinate"], true);
}

#[test]
fn test_json_not_installed() {
    let json = nix_status_to_json(&NixStatus::NotInstalled);
    assert_eq!(json["installed"], false);
    assert!(json.get("version").is_none());
}

#[test]
fn test_detect_nix_does_not_crash() {
    // This test verifies detect_nix() never panics,
    // regardless of whether Nix is installed.
    let status = detect_nix();
    match status {
        NixStatus::Available { version, .. } => {
            assert!(!version.is_empty(), "version should not be empty if available");
        }
        NixStatus::NotInstalled => {
            // Also valid
        }
    }
}

#[test]
fn test_nix_status_equality() {
    let a = NixStatus::Available {
        version: "2.33.3".to_string(),
        is_determinate: true,
    };
    let b = NixStatus::Available {
        version: "2.33.3".to_string(),
        is_determinate: true,
    };
    assert_eq!(a, b);
    assert_ne!(a, NixStatus::NotInstalled);
}
