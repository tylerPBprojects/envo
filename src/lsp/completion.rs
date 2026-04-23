//! Autocompletion logic for envo manifests.
//!
//! Provides context-aware completions depending on the cursor position
//! within the manifest TOML file.

use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind};

/// Top-level TOML section headers that can be completed.
const SECTION_COMPLETIONS: &[(&str, &str)] = &[
    ("[project]", "Project metadata (name, description, version)"),
    ("[packages]", "Packages to install in this environment"),
    ("[vars]", "Environment variables to set on activation"),
    ("[hook]", "Hook scripts (on-activate)"),
    ("[services]", "Background services to manage"),
    ("[options]", "Global options (nixpkgs channel, unfree, systems)"),
];

/// Keys available in the `[options]` section.
const OPTIONS_KEYS: &[(&str, &str)] = &[
    ("nixpkgs-channel", "The nixpkgs flake reference (default: \"nixpkgs\")"),
    ("allow-unfree", "Allow packages with non-free licenses (e.g., CUDA)"),
    ("systems", "Target systems to resolve for (e.g., [\"x86_64-linux\"])"),
];

/// Keys available in a full package spec (table form).
const PACKAGE_SPEC_KEYS: &[(&str, &str)] = &[
    ("version", "Version constraint (e.g., \"3.12\")"),
    ("systems", "Override target systems for this package"),
    ("pkg-path", "Alternative nixpkgs attribute path"),
    ("priority", "Priority for conflicting binaries (lower = higher priority)"),
];

/// Valid system strings for completion.
const SYSTEM_STRINGS: &[&str] = &[
    "x86_64-linux",
    "aarch64-linux",
    "x86_64-darwin",
    "aarch64-darwin",
];

/// Determine the completion context from the cursor position in the document.
///
/// Returns a list of completion items appropriate for the context.
pub fn get_completions(source: &str, line: u32, _character: u32) -> Vec<CompletionItem> {
    let lines: Vec<&str> = source.lines().collect();
    let current_line = lines.get(line as usize).map(|s| s.trim()).unwrap_or("");

    // Determine which section the cursor is in
    let section = find_current_section(&lines, line as usize);

    // If the line starts with '[', offer section completions
    if current_line.starts_with('[') && (current_line == "[" || !current_line.contains(']')) {
        return section_completions();
    }

    // Context-specific completions based on current section
    // (even if the current line is empty — we're inside a section)
    match section.as_deref() {
        Some("options") => options_completions(),
        Some("packages") => package_context_completions(current_line),
        Some(s) if s.starts_with("packages.") => package_spec_completions(),
        Some(s) if s.starts_with("services.") => service_completions(),
        Some("hook") => hook_completions(),
        Some("project") => project_completions(),
        _ => section_completions(),
    }
}

/// Find which TOML section the given line is in.
fn find_current_section(lines: &[&str], line_idx: usize) -> Option<String> {
    // Clamp to valid range — the cursor might be past the last line
    let start = line_idx.min(lines.len().saturating_sub(1));
    for i in (0..=start).rev() {
        let trimmed = lines[i].trim();
        if trimmed.starts_with('[') && !trimmed.starts_with("[[") {
            let section = trimmed
                .trim_start_matches('[')
                .trim_end_matches(']')
                .trim()
                .to_string();
            return Some(section);
        }
    }
    None
}

/// Complete top-level section headers.
fn section_completions() -> Vec<CompletionItem> {
    SECTION_COMPLETIONS
        .iter()
        .map(|(label, detail)| CompletionItem {
            label: label.to_string(),
            kind: Some(CompletionItemKind::MODULE),
            detail: Some(detail.to_string()),
            ..Default::default()
        })
        .collect()
}

/// Complete keys in the `[options]` section.
fn options_completions() -> Vec<CompletionItem> {
    let mut items: Vec<CompletionItem> = OPTIONS_KEYS
        .iter()
        .map(|(key, detail)| CompletionItem {
            label: key.to_string(),
            kind: Some(CompletionItemKind::PROPERTY),
            detail: Some(detail.to_string()),
            ..Default::default()
        })
        .collect();

    // Also offer system string completions for the systems array
    for sys in SYSTEM_STRINGS {
        items.push(CompletionItem {
            label: format!("\"{sys}\""),
            kind: Some(CompletionItemKind::ENUM_MEMBER),
            detail: Some("Target system".to_string()),
            ..Default::default()
        });
    }

    items
}

/// Complete in the `[packages]` section context.
///
/// If the line looks like a key = value, offer package spec completions.
/// Otherwise, this is where search-based completions would go (debounced).
fn package_context_completions(current_line: &str) -> Vec<CompletionItem> {
    // If the line has an = sign, user is editing a value — offer value completions
    if current_line.contains('=') {
        return vec![
            CompletionItem {
                label: "\"*\"".to_string(),
                kind: Some(CompletionItemKind::VALUE),
                detail: Some("Any version (latest)".to_string()),
                ..Default::default()
            },
            CompletionItem {
                label: "{ version = \"\" }".to_string(),
                kind: Some(CompletionItemKind::SNIPPET),
                detail: Some("Full package specification".to_string()),
                ..Default::default()
            },
        ];
    }

    // Otherwise, user is typing a package name — search completions would go here.
    // For now, return empty. The LSP server triggers async search via envo search.
    Vec::new()
}

/// Complete keys in a package spec table (e.g., `[packages.python3]`).
fn package_spec_completions() -> Vec<CompletionItem> {
    PACKAGE_SPEC_KEYS
        .iter()
        .map(|(key, detail)| CompletionItem {
            label: key.to_string(),
            kind: Some(CompletionItemKind::PROPERTY),
            detail: Some(detail.to_string()),
            ..Default::default()
        })
        .collect()
}

/// Complete keys in the `[hook]` section.
fn hook_completions() -> Vec<CompletionItem> {
    vec![CompletionItem {
        label: "on-activate".to_string(),
        kind: Some(CompletionItemKind::PROPERTY),
        detail: Some("Bash script that runs when the environment is activated".to_string()),
        ..Default::default()
    }]
}

/// Complete keys in a `[services.*]` section.
fn service_completions() -> Vec<CompletionItem> {
    vec![
        CompletionItem {
            label: "command".to_string(),
            kind: Some(CompletionItemKind::PROPERTY),
            detail: Some("Command to run the service".to_string()),
            ..Default::default()
        },
    ]
}

/// Complete keys in the `[project]` section.
fn project_completions() -> Vec<CompletionItem> {
    vec![
        CompletionItem {
            label: "name".to_string(),
            kind: Some(CompletionItemKind::PROPERTY),
            detail: Some("Project name (required)".to_string()),
            ..Default::default()
        },
        CompletionItem {
            label: "description".to_string(),
            kind: Some(CompletionItemKind::PROPERTY),
            detail: Some("Project description".to_string()),
            ..Default::default()
        },
        CompletionItem {
            label: "version".to_string(),
            kind: Some(CompletionItemKind::PROPERTY),
            detail: Some("Project version".to_string()),
            ..Default::default()
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_section_completions_at_empty_line() {
        // Line 0, before any section — should offer section completions
        let source = "\n[project]\nname = \"test\"\n";
        let items = get_completions(source, 0, 0);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"[packages]"));
        assert!(labels.contains(&"[project]"));
        assert!(labels.contains(&"[options]"));
    }

    #[test]
    fn test_options_completions() {
        let source = "[project]\nname = \"test\"\n\n[options]\n";
        let items = get_completions(source, 4, 0);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"nixpkgs-channel"));
        assert!(labels.contains(&"allow-unfree"));
        assert!(labels.contains(&"systems"));
    }

    #[test]
    fn test_package_value_completions() {
        let source = "[project]\nname = \"test\"\n\n[packages]\nripgrep = ";
        let items = get_completions(source, 4, 10);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"\"*\""));
    }

    #[test]
    fn test_hook_completions() {
        let source = "[project]\nname = \"test\"\n\n[hook]\n";
        let items = get_completions(source, 4, 0);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"on-activate"));
    }

    #[test]
    fn test_project_completions() {
        let source = "[project]\n";
        let items = get_completions(source, 1, 0);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"name"));
        assert!(labels.contains(&"description"));
    }

    #[test]
    fn test_package_spec_completions() {
        let source = "[project]\nname = \"test\"\n\n[packages.python3]\n";
        let items = get_completions(source, 4, 0);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"version"));
        assert!(labels.contains(&"pkg-path"));
    }

    #[test]
    fn test_find_current_section() {
        let lines = vec!["[project]", "name = \"test\"", "", "[packages]", "rg = \"*\""];
        assert_eq!(find_current_section(&lines, 1), Some("project".to_string()));
        assert_eq!(find_current_section(&lines, 4), Some("packages".to_string()));
    }
}
