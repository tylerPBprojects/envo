//! MCP resource implementations.
//!
//! Resources are read-only data that agents can access. Each resource
//! has a URI, MIME type, and content.

use crate::lockfile::Lockfile;
use crate::manifest::Manifest;
use crate::nix_bootstrap;
use crate::self_update;
use serde_json::json;
use std::path::Path;

/// Resource descriptor for MCP resources/list.
pub fn resource_definitions() -> serde_json::Value {
    json!([
        {
            "uri": "envo://manifest",
            "name": "Manifest",
            "description": "The current envo manifest.toml content",
            "mimeType": "application/toml"
        },
        {
            "uri": "envo://lockfile",
            "name": "Lockfile",
            "description": "The current envo lockfile content",
            "mimeType": "application/json"
        },
        {
            "uri": "envo://status",
            "name": "Status",
            "description": "Current environment status",
            "mimeType": "application/json"
        }
    ])
}

/// Read a resource by URI.
///
/// Returns a JSON object with `contents` array following the MCP spec.
pub fn read_resource(uri: &str, working_dir: &Path) -> serde_json::Value {
    match uri {
        "envo://manifest" => read_manifest(working_dir),
        "envo://lockfile" => read_lockfile(working_dir),
        "envo://status" => read_status(working_dir),
        _ => resource_error(&format!("unknown resource: {uri}")),
    }
}

/// Read the manifest.toml content.
fn read_manifest(working_dir: &Path) -> serde_json::Value {
    let manifest_path = working_dir.join(".envo").join("manifest.toml");

    match std::fs::read_to_string(&manifest_path) {
        Ok(content) => json!({
            "contents": [{
                "uri": "envo://manifest",
                "mimeType": "application/toml",
                "text": content,
            }]
        }),
        Err(_) => resource_error("no manifest found (run envo init first)"),
    }
}

/// Read the lockfile content.
fn read_lockfile(working_dir: &Path) -> serde_json::Value {
    let lockfile_path = working_dir.join(".envo").join("manifest.lock");

    match std::fs::read_to_string(&lockfile_path) {
        Ok(content) => json!({
            "contents": [{
                "uri": "envo://lockfile",
                "mimeType": "application/json",
                "text": content,
            }]
        }),
        Err(_) => resource_error("no lockfile found (run envo install first)"),
    }
}

/// Read the environment status.
fn read_status(working_dir: &Path) -> serde_json::Value {
    let has_manifest = working_dir.join(".envo").join("manifest.toml").exists();
    let has_lockfile = working_dir.join(".envo").join("manifest.lock").exists();

    let nix_status = nix_bootstrap::detect_nix();

    let mut status = json!({
        "initialized": has_manifest,
        "has_lockfile": has_lockfile,
        "nix": nix_bootstrap::nix_status_to_json(&nix_status),
        "system": self_update::get_current_system(),
        "envo_version": self_update::CURRENT_VERSION,
    });

    // Add project details if manifest exists
    if has_manifest {
        if let Ok(manifest) = Manifest::load(Some(working_dir)) {
            status["project_name"] = json!(manifest.project_name());
            let packages: Vec<String> = manifest.packages().keys().cloned().collect();
            status["packages"] = json!(packages);
            status["vars"] = json!(manifest.vars());
        }
    }

    json!({
        "contents": [{
            "uri": "envo://status",
            "mimeType": "application/json",
            "text": serde_json::to_string_pretty(&status).unwrap_or_default(),
        }]
    })
}

/// Format a resource error.
fn resource_error(message: &str) -> serde_json::Value {
    json!({
        "contents": [{
            "uri": "envo://error",
            "mimeType": "text/plain",
            "text": message,
        }],
        "isError": true
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resource_definitions() {
        let defs = resource_definitions();
        let arr = defs.as_array().unwrap();
        assert_eq!(arr.len(), 3);

        let uris: Vec<&str> = arr.iter().map(|r| r["uri"].as_str().unwrap()).collect();
        assert!(uris.contains(&"envo://manifest"));
        assert!(uris.contains(&"envo://lockfile"));
        assert!(uris.contains(&"envo://status"));
    }

    #[test]
    fn test_read_manifest_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let result = read_resource("envo://manifest", tmp.path());
        assert!(result.get("isError").and_then(|v| v.as_bool()).unwrap_or(false));
    }

    #[test]
    fn test_read_manifest_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let envo_dir = tmp.path().join(".envo");
        std::fs::create_dir_all(&envo_dir).unwrap();
        std::fs::write(
            envo_dir.join("manifest.toml"),
            "[project]\nname = \"test\"\n",
        )
        .unwrap();

        let result = read_resource("envo://manifest", tmp.path());
        assert!(!result.get("isError").and_then(|v| v.as_bool()).unwrap_or(false));

        let text = result["contents"][0]["text"].as_str().unwrap();
        assert!(text.contains("[project]"));
        assert_eq!(result["contents"][0]["mimeType"], "application/toml");
    }

    #[test]
    fn test_read_lockfile_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let result = read_resource("envo://lockfile", tmp.path());
        assert!(result.get("isError").and_then(|v| v.as_bool()).unwrap_or(false));
    }

    #[test]
    fn test_read_status_no_environment() {
        let tmp = tempfile::tempdir().unwrap();
        let result = read_resource("envo://status", tmp.path());
        assert!(!result.get("isError").and_then(|v| v.as_bool()).unwrap_or(false));

        let text = result["contents"][0]["text"].as_str().unwrap();
        let status: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(status["initialized"], false);
    }

    #[test]
    fn test_read_status_with_environment() {
        let tmp = tempfile::tempdir().unwrap();
        let envo_dir = tmp.path().join(".envo");
        std::fs::create_dir_all(&envo_dir).unwrap();
        std::fs::write(
            envo_dir.join("manifest.toml"),
            "[project]\nname = \"status-test\"\n\n[packages]\nripgrep = \"*\"\n",
        )
        .unwrap();

        let result = read_resource("envo://status", tmp.path());
        let text = result["contents"][0]["text"].as_str().unwrap();
        let status: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(status["initialized"], true);
        assert_eq!(status["project_name"], "status-test");
    }

    #[test]
    fn test_read_unknown_resource() {
        let tmp = tempfile::tempdir().unwrap();
        let result = read_resource("envo://nonexistent", tmp.path());
        assert!(result.get("isError").and_then(|v| v.as_bool()).unwrap_or(false));
    }
}
