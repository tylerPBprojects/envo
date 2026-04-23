//! Hover documentation for envo manifests.
//!
//! Provides contextual documentation when the user hovers over
//! section headers, keys, or values in a manifest.toml file.

/// Documentation entries for section headers.
const SECTION_DOCS: &[(&str, &str)] = &[
    ("project", "Project metadata.\n\nRequired fields:\n- `name`: The project name (used in status displays and activation).\n\nOptional fields:\n- `description`: A short description of the project.\n- `version`: The project version."),
    ("packages", "Packages to install in this environment.\n\nEach key is a package name from nixpkgs. Values can be:\n- `\"*\"` — latest version\n- `\"1.2\"` — version constraint\n- `{ version = \"1.2\", systems = [...] }` — full specification"),
    ("vars", "Environment variables set on activation.\n\nEach key-value pair becomes an `export KEY=\"VALUE\"` in the activation snapshot.\n\nExample:\n```toml\n[vars]\nEDITOR = \"vim\"\nDATABASE_URL = \"postgres://localhost/dev\"\n```"),
    ("hook", "Hook scripts that run during environment lifecycle events.\n\nCurrently supported:\n- `on-activate`: Bash script that runs once when the environment is activated (guarded against re-execution on nested activations)."),
    ("services", "Background services managed by envo.\n\nEach service is a named table with a `command` field:\n```toml\n[services.web]\ncommand = \"python -m http.server 8000\"\n```"),
    ("options", "Global environment options.\n\nAvailable options:\n- `nixpkgs-channel`: The nixpkgs flake reference (default: `\"nixpkgs\"`)\n- `allow-unfree`: Allow packages with non-free licenses (default: `false`)\n- `systems`: Target systems to resolve for (default: current system)"),
];

/// Documentation entries for specific keys.
const KEY_DOCS: &[(&str, &str)] = &[
    ("name", "The project name.\n\nUsed in status bar displays, terminal titles, and the `ENVO_ENV` environment variable."),
    ("description", "A short description of the project."),
    ("version", "The project version string."),
    ("on-activate", "Bash script that runs when the environment is activated.\n\nThe script is guarded against re-execution: if `ENVO_HOOK_DONE` is set, the hook is skipped. This prevents double-execution in nested activations.\n\nExample:\n```toml\n[hook]\non-activate = '''\necho \"Welcome to the project!\"\nexport CUSTOM_VAR=value\n'''"),
    ("nixpkgs-channel", "The nixpkgs flake reference used for package resolution.\n\nDefault: `\"nixpkgs\"` (follows the system flake registry).\n\nTo pin to a specific nixpkgs revision:\n```toml\nnixpkgs-channel = \"github:NixOS/nixpkgs/nixos-24.11\"\n```"),
    ("allow-unfree", "Allow packages with non-free licenses.\n\nSet to `true` to install packages like CUDA, VS Code, or other proprietary software.\n\nDefault: `false`"),
    ("systems", "Target systems to resolve packages for.\n\nDefault: the current system only.\n\nExample:\n```toml\nsystems = [\"x86_64-linux\", \"aarch64-linux\"]\n```\n\nValid systems: `x86_64-linux`, `aarch64-linux`, `x86_64-darwin`, `aarch64-darwin`"),
    ("command", "The command to run for this service.\n\nExample: `command = \"python -m http.server 8000\"`"),
    ("pkg-path", "Override the nixpkgs attribute path for this package.\n\nUseful when the package name doesn't match the nixpkgs attribute.\n\nExample:\n```toml\n[packages.python]\npkg-path = \"python312\"\n```"),
    ("priority", "Priority for resolving binary conflicts between packages.\n\nLower numbers have higher priority. Default: `5`.\n\nWhen two packages provide the same binary name, the one with lower priority wins."),
];

/// Get hover documentation for a position in the manifest.
///
/// Returns the documentation string if hover content is available
/// for the given position, or None if there's nothing to show.
pub fn get_hover(source: &str, line: u32, character: u32) -> Option<String> {
    let lines: Vec<&str> = source.lines().collect();
    let current_line = lines.get(line as usize)?;
    let trimmed = current_line.trim();

    // Check if hovering over a section header
    if trimmed.starts_with('[') && !trimmed.starts_with("[[") {
        let section = trimmed
            .trim_start_matches('[')
            .trim_end_matches(']')
            .trim();

        // Handle dotted sections like [services.web] → look up "services"
        let base = section.split('.').next().unwrap_or(section);

        return SECTION_DOCS
            .iter()
            .find(|(name, _)| *name == base)
            .map(|(_, doc)| doc.to_string());
    }

    // Check if hovering over a key
    if let Some(key) = extract_key_at_position(current_line, character as usize) {
        return KEY_DOCS
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, doc)| doc.to_string());
    }

    None
}

/// Extract the TOML key name at the given character position.
///
/// For a line like `nixpkgs-channel = "nixpkgs"`, if the cursor is
/// on "nixpkgs-channel", returns Some("nixpkgs-channel").
fn extract_key_at_position(line: &str, character: usize) -> Option<&str> {
    // If the line has an = sign, the key is everything before it
    if let Some(eq_pos) = line.find('=') {
        let key = line[..eq_pos].trim();
        // Check if the cursor is within the key portion
        let key_start = line.find(key)?;
        let key_end = key_start + key.len();
        if character >= key_start && character <= key_end {
            return Some(key);
        }
    }

    // If no = sign, the whole trimmed line might be a key being typed
    let trimmed = line.trim();
    if !trimmed.is_empty() && !trimmed.starts_with('#') && !trimmed.starts_with('[') {
        return Some(trimmed);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hover_section_header() {
        let source = "[project]\nname = \"test\"\n\n[packages]\nripgrep = \"*\"";
        let hover = get_hover(source, 0, 3);
        assert!(hover.is_some());
        assert!(hover.unwrap().contains("Project metadata"));
    }

    #[test]
    fn test_hover_packages_section() {
        let source = "[project]\nname = \"test\"\n\n[packages]\nripgrep = \"*\"";
        let hover = get_hover(source, 3, 3);
        assert!(hover.is_some());
        assert!(hover.unwrap().contains("Packages to install"));
    }

    #[test]
    fn test_hover_options_section() {
        let source = "[options]\nnixpkgs-channel = \"nixpkgs\"";
        let hover = get_hover(source, 0, 3);
        assert!(hover.is_some());
        assert!(hover.unwrap().contains("Global environment options"));
    }

    #[test]
    fn test_hover_key_on_activate() {
        let source = "[hook]\non-activate = \"echo hi\"";
        let hover = get_hover(source, 1, 5);
        assert!(hover.is_some());
        assert!(hover.unwrap().contains("Bash script"));
    }

    #[test]
    fn test_hover_key_allow_unfree() {
        let source = "[options]\nallow-unfree = true";
        let hover = get_hover(source, 1, 5);
        assert!(hover.is_some());
        assert!(hover.unwrap().contains("non-free licenses"));
    }

    #[test]
    fn test_hover_dotted_section() {
        let source = "[services.web]\ncommand = \"python -m http.server\"";
        let hover = get_hover(source, 0, 5);
        assert!(hover.is_some());
        assert!(hover.unwrap().contains("Background services"));
    }

    #[test]
    fn test_hover_no_content() {
        let source = "[project]\nname = \"test\"\n\n# just a comment";
        let hover = get_hover(source, 3, 5);
        assert!(hover.is_none());
    }

    #[test]
    fn test_extract_key_at_position() {
        assert_eq!(
            extract_key_at_position("nixpkgs-channel = \"nixpkgs\"", 5),
            Some("nixpkgs-channel")
        );
        assert_eq!(
            extract_key_at_position("name = \"test\"", 2),
            Some("name")
        );
    }
}
