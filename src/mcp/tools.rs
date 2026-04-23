//! MCP tool implementations.
//!
//! Each tool wraps envo library functions and returns structured JSON
//! results. Tools are testable independently of the JSON-RPC transport.

use crate::activate::snapshot::ShellType;
use crate::activate::Activator;
use crate::lockfile::resolver::{detect_current_system, resolve_manifest, NixEvaluator};
use crate::lockfile::Lockfile;
use crate::manifest::schema::PackageEntry;
use crate::manifest::Manifest;
use crate::nix_bootstrap;
use crate::realize::Realizer;
use serde_json::json;
use std::path::Path;
use std::process::Command;

/// Tool descriptor for MCP tools/list.
pub fn tool_definitions() -> serde_json::Value {
    json!([
        {
            "name": "envo_init",
            "description": "Initialize a new envo environment in a directory",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "directory": {
                        "type": "string",
                        "description": "Project directory (defaults to current working directory)"
                    }
                }
            }
        },
        {
            "name": "envo_install",
            "description": "Install one or more packages into the envo environment",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "packages": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Package names to install"
                    },
                    "directory": {
                        "type": "string",
                        "description": "Project directory"
                    }
                },
                "required": ["packages"]
            }
        },
        {
            "name": "envo_uninstall",
            "description": "Remove a package from the envo environment",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "package": {
                        "type": "string",
                        "description": "Package name to remove"
                    },
                    "directory": {
                        "type": "string",
                        "description": "Project directory"
                    }
                },
                "required": ["package"]
            }
        },
        {
            "name": "envo_search",
            "description": "Search for packages in nixpkgs",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query"
                    }
                },
                "required": ["query"]
            }
        },
        {
            "name": "envo_env_info",
            "description": "Get full environment information",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "directory": {
                        "type": "string",
                        "description": "Project directory"
                    }
                }
            }
        },
        {
            "name": "envo_activate",
            "description": "Get the environment variables that activation would set",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "directory": {
                        "type": "string",
                        "description": "Project directory"
                    }
                }
            }
        }
    ])
}

/// Dispatch a tool call to the appropriate handler.
pub fn call_tool(name: &str, args: &serde_json::Value) -> serde_json::Value {
    let start = std::time::Instant::now();

    let result = match name {
        "envo_init" => tool_init(args),
        "envo_install" => tool_install(args),
        "envo_uninstall" => tool_uninstall(args),
        "envo_search" => tool_search(args),
        "envo_env_info" => tool_env_info(args),
        "envo_activate" => tool_activate(args),
        _ => tool_error(&format!("unknown tool: {name}")),
    };

    let is_error = result.get("isError").and_then(|v| v.as_bool()).unwrap_or(false);
    let duration_ms = start.elapsed().as_millis() as u64;

    let mut extra = std::collections::HashMap::new();
    extra.insert("tool_name".to_string(), serde_json::json!(name));

    crate::telemetry::track_event(
        "mcp",
        if is_error { "mcp_error" } else { "mcp_tool_call" },
        !is_error,
        Some(duration_ms),
        Some(extra),
        false, // MCP server doesn't have a verbose flag per-request
    );

    result
}

/// Get the directory from args, defaulting to cwd.
fn get_directory(args: &serde_json::Value) -> Result<String, String> {
    if let Some(dir) = args.get("directory").and_then(|d| d.as_str()) {
        if !dir.is_empty() {
            return Ok(dir.to_string());
        }
    }
    std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| format!("could not determine working directory: {e}"))
}

/// Format a tool error as an MCP tool result.
fn tool_error(message: &str) -> serde_json::Value {
    json!({
        "content": [{
            "type": "text",
            "text": message
        }],
        "isError": true
    })
}

/// Format a tool success result.
fn tool_success(data: serde_json::Value) -> serde_json::Value {
    let text = serde_json::to_string_pretty(&data).unwrap_or_default();
    json!({
        "content": [{
            "type": "text",
            "text": text
        }]
    })
}

/// Initialize a new envo environment.
fn tool_init(args: &serde_json::Value) -> serde_json::Value {
    let dir = match get_directory(args) {
        Ok(d) => d,
        Err(e) => return tool_error(&e),
    };

    match Manifest::init(Path::new(&dir)) {
        Ok(manifest) => tool_success(json!({
            "success": true,
            "project_name": manifest.project_name(),
            "manifest_path": ".envo/manifest.toml"
        })),
        Err(e) => tool_error(&format!("init failed: {e}")),
    }
}

/// Install packages.
fn tool_install(args: &serde_json::Value) -> serde_json::Value {
    let dir = match get_directory(args) {
        Ok(d) => d,
        Err(e) => return tool_error(&e),
    };

    let packages: Vec<String> = match args.get("packages").and_then(|p| p.as_array()) {
        Some(arr) => arr.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
        None => return tool_error("missing required parameter: packages"),
    };

    if packages.is_empty() {
        return tool_error("packages array must not be empty");
    }

    let dir_path = Path::new(&dir);

    // Load manifest
    let mut manifest = match Manifest::load(Some(dir_path)) {
        Ok(m) => m,
        Err(e) => return tool_error(&format!("no envo environment found: {e}")),
    };

    // Add packages
    for pkg in &packages {
        if let Err(e) = manifest.add_package(pkg, PackageEntry::Short("*".to_string())) {
            return tool_error(&format!("invalid package name '{pkg}': {e}"));
        }
    }

    if let Err(e) = manifest.save(dir_path) {
        return tool_error(&format!("failed to save manifest: {e}"));
    }

    // Resolve
    let existing = Lockfile::load(Some(dir_path)).ok();
    let mut evaluator = NixEvaluator::new();
    let lockfile = match resolve_manifest(&manifest, &mut evaluator, existing.as_ref()) {
        Ok(lf) => lf,
        Err(e) => return tool_error(&format!("resolution failed: {e}")),
    };

    if let Err(e) = lockfile.save(dir_path) {
        return tool_error(&format!("failed to save lockfile: {e}"));
    }

    // Generate shims
    let system = detect_current_system();
    let realizer = Realizer::new(dir_path);
    if let Err(e) = realizer.generate_shims(&lockfile, &system) {
        return tool_error(&format!("shim generation failed: {e}"));
    }

    tool_success(json!({
        "success": true,
        "installed": packages,
        "lockfile_updated": true
    }))
}

/// Uninstall a package.
fn tool_uninstall(args: &serde_json::Value) -> serde_json::Value {
    let dir = match get_directory(args) {
        Ok(d) => d,
        Err(e) => return tool_error(&e),
    };

    let package = match args.get("package").and_then(|p| p.as_str()) {
        Some(p) => p,
        None => return tool_error("missing required parameter: package"),
    };

    let dir_path = Path::new(&dir);

    let mut manifest = match Manifest::load(Some(dir_path)) {
        Ok(m) => m,
        Err(e) => return tool_error(&format!("no envo environment found: {e}")),
    };

    if !manifest.remove_package(package) {
        return tool_error(&format!("package '{package}' is not installed"));
    }

    if let Err(e) = manifest.save(dir_path) {
        return tool_error(&format!("failed to save manifest: {e}"));
    }

    // Re-resolve
    let mut evaluator = NixEvaluator::new();
    let lockfile = match resolve_manifest(&manifest, &mut evaluator, None) {
        Ok(lf) => lf,
        Err(e) => return tool_error(&format!("resolution failed: {e}")),
    };

    if let Err(e) = lockfile.save(dir_path) {
        return tool_error(&format!("failed to save lockfile: {e}"));
    }

    let system = detect_current_system();
    let realizer = Realizer::new(dir_path);
    if let Err(e) = realizer.generate_shims(&lockfile, &system) {
        return tool_error(&format!("shim generation failed: {e}"));
    }

    tool_success(json!({
        "success": true,
        "removed": package
    }))
}

/// Search for packages.
fn tool_search(args: &serde_json::Value) -> serde_json::Value {
    let query = match args.get("query").and_then(|q| q.as_str()) {
        Some(q) => q,
        None => return tool_error("missing required parameter: query"),
    };

    // Shell out to nix search for consistency with the CLI
    let output = match Command::new("nix")
        .args(["search", "nixpkgs", query, "--json"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return tool_error("Nix is not installed or not in PATH"),
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return tool_error(&format!("search failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let results: serde_json::Value = match serde_json::from_str(&stdout) {
        Ok(v) => v,
        Err(e) => return tool_error(&format!("failed to parse search results: {e}")),
    };

    let results_map = match results.as_object() {
        Some(m) => m,
        None => return tool_error("unexpected search output format"),
    };

    let mut items = Vec::new();
    let mut count = 0;
    for (attr_path, info) in results_map {
        if count >= 20 {
            break;
        }
        let pkg_name = attr_path.rsplit('.').next().unwrap_or(attr_path);
        let description = info.get("description").and_then(|d| d.as_str()).unwrap_or("");
        let version = info.get("version").and_then(|v| v.as_str()).unwrap_or("");

        items.push(json!({
            "name": pkg_name,
            "version": version,
            "description": description,
        }));
        count += 1;
    }

    tool_success(json!({ "results": items }))
}

/// Get environment information.
pub fn tool_env_info(args: &serde_json::Value) -> serde_json::Value {
    let dir = match get_directory(args) {
        Ok(d) => d,
        Err(e) => return tool_error(&e),
    };

    let dir_path = Path::new(&dir);

    let manifest = match Manifest::load(Some(dir_path)) {
        Ok(m) => m,
        Err(_) => {
            return tool_success(json!({
                "initialized": false,
                "nix_status": nix_bootstrap::nix_status_to_json(&nix_bootstrap::detect_nix()),
                "system": detect_current_system(),
            }));
        }
    };

    let packages: Vec<String> = manifest.packages().keys().cloned().collect();
    let vars = manifest.vars().clone();

    let has_lockfile = Lockfile::load(Some(dir_path)).is_ok();

    tool_success(json!({
        "initialized": true,
        "project_name": manifest.project_name(),
        "packages": packages,
        "vars": vars,
        "has_lockfile": has_lockfile,
        "nix_status": nix_bootstrap::nix_status_to_json(&nix_bootstrap::detect_nix()),
        "system": detect_current_system(),
    }))
}

/// Get activation environment variables.
fn tool_activate(args: &serde_json::Value) -> serde_json::Value {
    let dir = match get_directory(args) {
        Ok(d) => d,
        Err(e) => return tool_error(&e),
    };

    let dir_path = Path::new(&dir);

    let manifest = match Manifest::load(Some(dir_path)) {
        Ok(m) => m,
        Err(e) => return tool_error(&format!("no envo environment found: {e}")),
    };

    let lockfile = match Lockfile::load(Some(dir_path)) {
        Ok(lf) => lf,
        Err(e) => return tool_error(&format!("no lockfile found (run envo_install first): {e}")),
    };

    let system = detect_current_system();
    let realizer = Realizer::new(dir_path);
    let shim_manifest = match realizer.generate_shims(&lockfile, &system) {
        Ok(sm) => sm,
        Err(e) => return tool_error(&format!("shim generation failed: {e}")),
    };

    let activator = Activator::new(dir_path);
    let env_vars = match activator.env_vars(&manifest, &lockfile, &shim_manifest) {
        Ok(ev) => ev,
        Err(e) => return tool_error(&format!("activation failed: {e}")),
    };

    // Also generate the snapshot for reference
    let snapshot = activator
        .generate_snapshot(&manifest, &lockfile, &shim_manifest, ShellType::Bash)
        .ok();

    tool_success(json!({
        "env_vars": env_vars,
        "snapshot": snapshot,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_definitions_has_all_tools() {
        let defs = tool_definitions();
        let arr = defs.as_array().unwrap();
        assert_eq!(arr.len(), 6);

        let names: Vec<&str> = arr
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"envo_init"));
        assert!(names.contains(&"envo_install"));
        assert!(names.contains(&"envo_uninstall"));
        assert!(names.contains(&"envo_search"));
        assert!(names.contains(&"envo_env_info"));
        assert!(names.contains(&"envo_activate"));
    }

    #[test]
    fn test_call_unknown_tool() {
        let result = call_tool("nonexistent", &json!({}));
        assert!(result["isError"].as_bool().unwrap_or(false));
    }

    #[test]
    fn test_tool_init_creates_environment() {
        let tmp = tempfile::tempdir().unwrap();
        let result = call_tool("envo_init", &json!({ "directory": tmp.path().to_str().unwrap() }));
        assert!(!result.get("isError").and_then(|v| v.as_bool()).unwrap_or(false),
            "init should succeed: {result}");

        let text = result["content"][0]["text"].as_str().unwrap();
        let data: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(data["success"], true);
        assert!(data["project_name"].is_string());
    }

    #[test]
    fn test_tool_install_missing_packages() {
        let result = call_tool("envo_install", &json!({}));
        assert!(result["isError"].as_bool().unwrap_or(false));
    }

    #[test]
    fn test_tool_install_empty_packages() {
        let result = call_tool("envo_install", &json!({ "packages": [] }));
        assert!(result["isError"].as_bool().unwrap_or(false));
    }

    #[test]
    fn test_tool_uninstall_missing_package() {
        let result = call_tool("envo_uninstall", &json!({}));
        assert!(result["isError"].as_bool().unwrap_or(false));
    }

    #[test]
    fn test_tool_search_missing_query() {
        let result = call_tool("envo_search", &json!({}));
        assert!(result["isError"].as_bool().unwrap_or(false));
    }

    #[test]
    fn test_tool_env_info_no_environment() {
        let tmp = tempfile::tempdir().unwrap();
        let result = call_tool("envo_env_info", &json!({ "directory": tmp.path().to_str().unwrap() }));
        // Should succeed with initialized: false
        assert!(!result.get("isError").and_then(|v| v.as_bool()).unwrap_or(false));

        let text = result["content"][0]["text"].as_str().unwrap();
        let data: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(data["initialized"], false);
    }

    #[test]
    fn test_tool_env_info_with_environment() {
        let tmp = tempfile::tempdir().unwrap();
        // Init first
        call_tool("envo_init", &json!({ "directory": tmp.path().to_str().unwrap() }));

        let result = call_tool("envo_env_info", &json!({ "directory": tmp.path().to_str().unwrap() }));
        assert!(!result.get("isError").and_then(|v| v.as_bool()).unwrap_or(false));

        let text = result["content"][0]["text"].as_str().unwrap();
        let data: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(data["initialized"], true);
        assert!(data["project_name"].is_string());
    }

    #[test]
    fn test_tool_activate_no_environment() {
        let tmp = tempfile::tempdir().unwrap();
        let result = call_tool("envo_activate", &json!({ "directory": tmp.path().to_str().unwrap() }));
        assert!(result["isError"].as_bool().unwrap_or(false));
    }
}
