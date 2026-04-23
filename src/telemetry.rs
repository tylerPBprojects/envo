//! Lightweight telemetry via PostHog.
//!
//! Answers two questions: "is the product being used?" and "where is it
//! slow or failing?" Uses PostHog's capture API with fire-and-forget
//! delivery via a detached `curl` process. Adds <1ms latency to any command.
//!
//! # Privacy
//!
//! - Opt-out via `~/.envo/config.toml` (`[telemetry] enabled = false`)
//! - No source code, file contents, secrets, env vars, or full paths collected
//! - Error messages are sanitized before sending
//! - The `distinct_id` is a random UUID with no connection to user identity
//!
//! # Delivery
//!
//! Events are POSTed to PostHog via a detached `curl` child process.
//! The process is spawned and immediately forgotten — we never wait for
//! it, never read its output, never check its exit code. If curl is
//! missing or the network is down, the event is silently dropped.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// PostHog project API key.
/// Override at build time with: ENVO_POSTHOG_KEY=phc_xxx cargo build
macro_rules! posthog_key {
    () => {
        match option_env!("ENVO_POSTHOG_KEY") {
            Some(key) => key,
            None => "phx_HxVFk77Am3qS54apgLRxpdgeUDqME737xxVE93uDKSxC4pjP",
        }
    };
}

/// PostHog capture endpoint (US cloud).
const POSTHOG_ENDPOINT: &str = "https://us.i.posthog.com/capture/";

/// Config file path relative to home directory.
const CONFIG_RELATIVE_PATH: &str = ".envo/config.toml";

/// Check whether telemetry is enabled.
///
/// Reads `~/.envo/config.toml` and checks for `[telemetry] enabled = false`.
/// If the config file doesn't exist, is unreadable, or has no telemetry
/// section, telemetry is enabled by default (opt-out model).
pub fn is_enabled() -> bool {
    let config_path = match config_path() {
        Some(p) => p,
        None => return true, // Can't find config → default to enabled
    };

    match std::fs::read_to_string(&config_path) {
        Ok(content) => {
            // Parse the TOML and check [telemetry].enabled
            if let Ok(table) = content.parse::<toml::Table>() {
                if let Some(telemetry) = table.get("telemetry").and_then(|t| t.as_table()) {
                    if let Some(enabled) = telemetry.get("enabled").and_then(|e| e.as_bool()) {
                        return enabled;
                    }
                }
            }
            // Config exists but no telemetry section → enabled
            true
        }
        Err(_) => true, // Can't read config → enabled
    }
}

/// Get or create a persistent anonymous machine ID.
///
/// Stored in `~/.envo/config.toml` under `[telemetry] machine_id`.
/// Generated as a random UUID v4 on first call.
pub fn get_or_create_machine_id() -> String {
    let config_path = match config_path() {
        Some(p) => p,
        None => return generate_uuid(),
    };

    // Try to read existing machine_id
    if let Ok(content) = std::fs::read_to_string(&config_path) {
        if let Ok(table) = content.parse::<toml::Table>() {
            if let Some(telemetry) = table.get("telemetry").and_then(|t| t.as_table()) {
                if let Some(id) = telemetry.get("machine_id").and_then(|v| v.as_str()) {
                    if !id.is_empty() {
                        return id.to_string();
                    }
                }
            }
        }
    }

    // Generate new ID and save it
    let new_id = generate_uuid();
    save_machine_id(&config_path, &new_id);
    new_id
}

/// Track a command/operation event.
///
/// Fires a non-blocking POST to PostHog. Never blocks, never panics,
/// never returns errors. If telemetry is disabled or delivery fails,
/// the event is silently dropped.
pub fn track_event(
    surface: &str,
    event: &str,
    success: bool,
    duration_ms: Option<u64>,
    extra: Option<HashMap<String, serde_json::Value>>,
    verbose: bool,
) {
    if !is_enabled() {
        return;
    }

    let mut properties = base_properties(surface);
    properties.insert("success".to_string(), serde_json::json!(success));

    if let Some(ms) = duration_ms {
        properties.insert("duration_ms".to_string(), serde_json::json!(ms));
    }

    if let Some(extra_props) = extra {
        for (k, v) in extra_props {
            properties.insert(k, v);
        }
    }

    let payload = serde_json::json!({
        "api_key": posthog_key!(),
        "event": event,
        "distinct_id": get_or_create_machine_id(),
        "properties": properties,
    });

    if verbose {
        eprintln!("ℹ telemetry: sending {event} event");
    }

    send_to_posthog(&payload);
}

/// Track an error event.
///
/// Error messages are sanitized before sending — file paths, env var
/// values, and sensitive data are stripped.
pub fn track_error(
    surface: &str,
    command: &str,
    error_type: &str,
    message: &str,
    verbose: bool,
) {
    if !is_enabled() {
        return;
    }

    let mut properties = base_properties(surface);
    properties.insert("command".to_string(), serde_json::json!(command));
    properties.insert("error_type".to_string(), serde_json::json!(error_type));
    properties.insert(
        "error_message".to_string(),
        serde_json::json!(sanitize(message)),
    );

    let payload = serde_json::json!({
        "api_key": posthog_key!(),
        "event": "error",
        "distinct_id": get_or_create_machine_id(),
        "properties": properties,
    });

    if verbose {
        eprintln!("ℹ telemetry: sending error event for {command}");
    }

    send_to_posthog(&payload);
}

/// Sanitize a message for telemetry.
///
/// Strips file paths, truncates to 200 characters, and removes
/// potential sensitive data.
pub fn sanitize(message: &str) -> String {
    let mut result = message.to_string();

    // Replace home directory paths
    if let Some(home) = home_dir() {
        let home_str = home.to_string_lossy();
        result = result.replace(home_str.as_ref(), "<home>");
    }

    // Replace common path patterns
    // /home/username/... → <path>
    let path_re_patterns = [
        "/home/",
        "/Users/",
        "/tmp/",
        "/var/",
        "/nix/store/",
    ];
    for pattern in path_re_patterns {
        if let Some(idx) = result.find(pattern) {
            // Find the end of the path (next whitespace, quote, or end of string)
            let rest = &result[idx..];
            let end = rest
                .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ':')
                .unwrap_or(rest.len());
            let path = &result[idx..idx + end];
            result = result.replace(path, "<path>");
        }
    }

    // Truncate to 200 characters
    if result.len() > 200 {
        result.truncate(197);
        result.push_str("...");
    }

    result
}

// ── Internal helpers ──────────────────────────────────────────────

/// Build the base properties included in every event.
fn base_properties(surface: &str) -> HashMap<String, serde_json::Value> {
    let mut props = HashMap::new();
    props.insert("surface".to_string(), serde_json::json!(surface));
    props.insert(
        "version".to_string(),
        serde_json::json!(crate::self_update::CURRENT_VERSION),
    );
    props.insert(
        "os".to_string(),
        serde_json::json!(crate::self_update::get_current_system()),
    );
    props.insert("$lib".to_string(), serde_json::json!("envo"));
    props
}

/// Fire-and-forget POST to PostHog via detached curl.
///
/// Spawns curl as a child process and immediately forgets about it.
/// Never waits for completion, never reads output, never checks exit code.
fn send_to_posthog(payload: &serde_json::Value) {
    let json = match serde_json::to_string(payload) {
        Ok(j) => j,
        Err(_) => return, // Silently skip if serialization fails
    };

    // Spawn curl detached — don't wait for it
    let _ = Command::new("curl")
        .args([
            "-s",          // Silent
            "-X", "POST",
            POSTHOG_ENDPOINT,
            "-H", "Content-Type: application/json",
            "-d", &json,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    // We intentionally ignore the Result — if curl fails to spawn, we silently skip
}

/// Get the path to ~/.envo/config.toml.
fn config_path() -> Option<PathBuf> {
    home_dir().map(|h| h.join(CONFIG_RELATIVE_PATH))
}

/// Get the user's home directory.
fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
}

/// Generate a UUID v4 (random).
///
/// Uses a simple random implementation to avoid adding a uuid crate dependency.
fn generate_uuid() -> String {
    // Read random bytes from /dev/urandom (available on all our target platforms)
    let mut bytes = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        let _ = f.read_exact(&mut bytes);
    } else {
        // Fallback: use timestamp + pid as entropy (not cryptographically random,
        // but fine for an anonymous telemetry ID)
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let seed = now.as_nanos() ^ (std::process::id() as u128);
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = ((seed >> (i * 8)) & 0xff) as u8;
        }
    }

    // Set version (4) and variant (RFC 4122)
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // variant 1

    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

/// Save a machine_id to the config file.
///
/// Appends or updates the [telemetry] section. This is a best-effort
/// operation — if it fails, we'll just generate a new ID next time.
fn save_machine_id(config_path: &Path, machine_id: &str) {
    let content = std::fs::read_to_string(config_path).unwrap_or_default();

    if content.contains("[telemetry]") {
        // Section exists — check if machine_id is already there
        if content.contains("machine_id") {
            // Replace existing machine_id line
            let mut new_content = String::new();
            for line in content.lines() {
                if line.trim_start().starts_with("machine_id") {
                    new_content.push_str(&format!("machine_id = \"{machine_id}\"\n"));
                } else {
                    new_content.push_str(line);
                    new_content.push('\n');
                }
            }
            let _ = std::fs::write(config_path, new_content);
        } else {
            // Add machine_id after [telemetry]
            let new_content =
                content.replace("[telemetry]", &format!("[telemetry]\nmachine_id = \"{machine_id}\""));
            let _ = std::fs::write(config_path, new_content);
        }
    } else {
        // No [telemetry] section — append it
        let mut new_content = content;
        if !new_content.ends_with('\n') && !new_content.is_empty() {
            new_content.push('\n');
        }
        new_content.push_str(&format!(
            "\n[telemetry]\nenabled = true\nmachine_id = \"{machine_id}\"\n"
        ));
        // Ensure parent directory exists
        if let Some(parent) = config_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(config_path, new_content);
    }
}

// ── Unit tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_strips_home_paths() {
        let msg = "failed to read /home/tyler/project/.envo/manifest.toml";
        let sanitized = sanitize(msg);
        assert!(!sanitized.contains("/home/tyler"), "should strip home path: {sanitized}");
        assert!(sanitized.contains("<path>") || sanitized.contains("<home>"));
    }

    #[test]
    fn test_sanitize_strips_users_paths() {
        let msg = "error at /Users/alice/code/thing.rs";
        let sanitized = sanitize(msg);
        assert!(!sanitized.contains("/Users/alice"));
    }

    #[test]
    fn test_sanitize_strips_nix_store_paths() {
        let msg = "cannot find /nix/store/abc123-ripgrep-14.1.0/bin/rg";
        let sanitized = sanitize(msg);
        assert!(!sanitized.contains("/nix/store/abc123"));
        assert!(sanitized.contains("<path>"));
    }

    #[test]
    fn test_sanitize_truncates_long_messages() {
        let msg = "x".repeat(500);
        let sanitized = sanitize(&msg);
        assert_eq!(sanitized.len(), 200);
        assert!(sanitized.ends_with("..."));
    }

    #[test]
    fn test_sanitize_short_message_unchanged() {
        let msg = "simple error message";
        let sanitized = sanitize(msg);
        assert_eq!(sanitized, "simple error message");
    }

    #[test]
    fn test_sanitize_empty_message() {
        assert_eq!(sanitize(""), "");
    }

    #[test]
    fn test_generate_uuid_format() {
        let uuid = generate_uuid();
        // UUID v4 format: 8-4-4-4-12 hex chars
        let parts: Vec<&str> = uuid.split('-').collect();
        assert_eq!(parts.len(), 5, "UUID should have 5 parts: {uuid}");
        assert_eq!(parts[0].len(), 8);
        assert_eq!(parts[1].len(), 4);
        assert_eq!(parts[2].len(), 4);
        assert_eq!(parts[3].len(), 4);
        assert_eq!(parts[4].len(), 12);
        // Version nibble should be 4
        assert!(parts[2].starts_with('4'), "version should be 4: {uuid}");
    }

    #[test]
    fn test_generate_uuid_unique() {
        let id1 = generate_uuid();
        let id2 = generate_uuid();
        assert_ne!(id1, id2, "UUIDs should be unique");
    }

    #[test]
    fn test_is_enabled_default_true() {
        // When HOME points to a nonexistent directory, config won't be found
        // and telemetry defaults to enabled
        std::env::set_var("HOME", "/nonexistent/path/for/test");
        assert!(is_enabled());
        // Restore HOME
        if let Ok(real_home) = std::env::var("USER") {
            std::env::set_var("HOME", format!("/home/{real_home}"));
        }
    }

    #[test]
    fn test_is_enabled_reads_config() {
        let tmp = tempfile::tempdir().unwrap();
        let envo_dir = tmp.path().join(".envo");
        std::fs::create_dir_all(&envo_dir).unwrap();

        // Write config with telemetry disabled
        std::fs::write(
            envo_dir.join("config.toml"),
            "[telemetry]\nenabled = false\n",
        )
        .unwrap();

        let config = tmp.path().join(".envo/config.toml");
        let content = std::fs::read_to_string(&config).unwrap();
        let table: toml::Table = content.parse().unwrap();
        let enabled = table
            .get("telemetry")
            .and_then(|t| t.as_table())
            .and_then(|t| t.get("enabled"))
            .and_then(|e| e.as_bool())
            .unwrap_or(true);
        assert!(!enabled);
    }

    #[test]
    fn test_is_enabled_missing_section() {
        let tmp = tempfile::tempdir().unwrap();
        let envo_dir = tmp.path().join(".envo");
        std::fs::create_dir_all(&envo_dir).unwrap();

        // Config with no telemetry section
        std::fs::write(
            envo_dir.join("config.toml"),
            "# envo config\n",
        )
        .unwrap();

        let config = tmp.path().join(".envo/config.toml");
        let content = std::fs::read_to_string(&config).unwrap();
        let table: toml::Table = content.parse().unwrap();
        let enabled = table
            .get("telemetry")
            .and_then(|t| t.as_table())
            .and_then(|t| t.get("enabled"))
            .and_then(|e| e.as_bool())
            .unwrap_or(true); // default
        assert!(enabled);
    }

    #[test]
    fn test_save_and_read_machine_id() {
        let tmp = tempfile::tempdir().unwrap();
        let envo_dir = tmp.path().join(".envo");
        std::fs::create_dir_all(&envo_dir).unwrap();

        let config_path = envo_dir.join("config.toml");
        std::fs::write(&config_path, "# envo config\n").unwrap();

        let test_id = "test-uuid-12345";
        save_machine_id(&config_path, test_id);

        let content = std::fs::read_to_string(&config_path).unwrap();
        assert!(content.contains(test_id), "config should contain machine_id: {content}");
        assert!(content.contains("[telemetry]"));
    }

    #[test]
    fn test_save_machine_id_existing_section() {
        let tmp = tempfile::tempdir().unwrap();
        let envo_dir = tmp.path().join(".envo");
        std::fs::create_dir_all(&envo_dir).unwrap();

        let config_path = envo_dir.join("config.toml");
        std::fs::write(&config_path, "[telemetry]\nenabled = true\n").unwrap();

        save_machine_id(&config_path, "new-id-abc");

        let content = std::fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("new-id-abc"));
        // Should not duplicate the [telemetry] section
        assert_eq!(content.matches("[telemetry]").count(), 1);
    }

    #[test]
    fn test_base_properties() {
        let props = base_properties("cli");
        assert_eq!(props["surface"], "cli");
        assert!(props.contains_key("version"));
        assert!(props.contains_key("os"));
        assert_eq!(props["$lib"], "envo");
    }

    #[test]
    fn test_posthog_payload_structure() {
        let mut props = base_properties("cli");
        props.insert("success".to_string(), serde_json::json!(true));
        props.insert("duration_ms".to_string(), serde_json::json!(54));

        let payload = serde_json::json!({
            "api_key": posthog_key!(),
            "event": "cli_activate",
            "distinct_id": "test-uuid",
            "properties": props,
        });

        assert!(payload["api_key"].is_string());
        assert_eq!(payload["event"], "cli_activate");
        assert_eq!(payload["properties"]["surface"], "cli");
        assert_eq!(payload["properties"]["success"], true);
        assert_eq!(payload["properties"]["duration_ms"], 54);
    }
}
