//! Manifest validation → LSP diagnostics.
//!
//! Converts envo manifest parse/validation errors into LSP diagnostic
//! messages with proper positions. Uses the existing `envo::manifest`
//! module for parsing — does NOT reimplement TOML parsing.

use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};

/// Known valid top-level TOML sections in a manifest.
const VALID_SECTIONS: &[&str] = &[
    "project", "packages", "vars", "hook", "services", "options",
];

/// Valid system strings for the `[options].systems` field.
pub const VALID_SYSTEMS: &[&str] = &[
    "x86_64-linux",
    "aarch64-linux",
    "x86_64-darwin",
    "aarch64-darwin",
];

/// Generate LSP diagnostics for a manifest TOML string.
///
/// This is the primary entry point. It attempts to parse the manifest
/// using `envo::manifest::Manifest::from_str` and converts any errors
/// to positioned LSP diagnostics. It also runs additional checks for
/// warnings (e.g., unknown sections).
pub fn diagnose(source: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // Try parsing with the manifest module
    match crate::manifest::Manifest::from_str(source) {
        Ok(_manifest) => {
            // Parse succeeded — check for warnings only
            check_unknown_sections(source, &mut diagnostics);
        }
        Err(e) => {
            let message = e.to_string();

            // Try to find the position of the error in the source
            let range = find_error_range(source, &message);

            let severity = if message.contains("warning") {
                DiagnosticSeverity::WARNING
            } else {
                DiagnosticSeverity::ERROR
            };

            diagnostics.push(Diagnostic {
                range,
                severity: Some(severity),
                source: Some("envo".to_string()),
                message: clean_error_message(&message),
                ..Default::default()
            });

            // Even if parsing failed, check for warnings
            check_unknown_sections(source, &mut diagnostics);
        }
    }

    diagnostics
}

/// Check for unknown top-level section names and emit warnings.
fn check_unknown_sections(source: &str, diagnostics: &mut Vec<Diagnostic>) {
    for (line_num, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        // Match [section] but not [[array]] or [section.subsection]
        if trimmed.starts_with('[') && !trimmed.starts_with("[[") {
            let section_name = trimmed
                .trim_start_matches('[')
                .trim_end_matches(']')
                .trim();

            // Skip dotted keys like [project] is fine, but [project.sub] might be ok too
            let base_section = section_name.split('.').next().unwrap_or(section_name);

            if !VALID_SECTIONS.contains(&base_section) {
                diagnostics.push(Diagnostic {
                    range: Range {
                        start: Position {
                            line: line_num as u32,
                            character: 0,
                        },
                        end: Position {
                            line: line_num as u32,
                            character: line.len() as u32,
                        },
                    },
                    severity: Some(DiagnosticSeverity::WARNING),
                    source: Some("envo".to_string()),
                    message: format!(
                        "unknown section '[{section_name}]'. Valid sections: {}",
                        VALID_SECTIONS.join(", ")
                    ),
                    ..Default::default()
                });
            }
        }
    }
}

/// Try to find the source range for an error message.
///
/// This does a best-effort search for relevant text in the source.
/// If the error mentions a specific name (package, system, etc.),
/// we try to find that name in the source.
fn find_error_range(source: &str, message: &str) -> Range {
    // Try to extract a quoted name from the error message
    if let Some(name) = extract_quoted_name(message) {
        for (line_num, line) in source.lines().enumerate() {
            if let Some(col) = line.find(&name) {
                return Range {
                    start: Position {
                        line: line_num as u32,
                        character: col as u32,
                    },
                    end: Position {
                        line: line_num as u32,
                        character: (col + name.len()) as u32,
                    },
                };
            }
        }
    }

    // Check for common error patterns
    if message.contains("project.name") {
        for (line_num, line) in source.lines().enumerate() {
            if line.contains("name") && line.contains('=') {
                return Range {
                    start: Position {
                        line: line_num as u32,
                        character: 0,
                    },
                    end: Position {
                        line: line_num as u32,
                        character: line.len() as u32,
                    },
                };
            }
        }
    }

    // If we can't find the specific location, report on line 0
    Range {
        start: Position {
            line: 0,
            character: 0,
        },
        end: Position {
            line: 0,
            character: 0,
        },
    }
}

/// Extract a quoted name like 'foo' from an error message.
fn extract_quoted_name(message: &str) -> Option<String> {
    let start = message.find('\'')?;
    let rest = &message[start + 1..];
    let end = rest.find('\'')?;
    Some(rest[..end].to_string())
}

/// Clean up error messages for LSP display.
/// Removes prefixes like "validation error: " or "manifest parse error: ".
fn clean_error_message(message: &str) -> String {
    message
        .replace("validation error: ", "")
        .replace("manifest parse error: ", "")
        .replace("manifest serialization error: ", "")
}

// ── Unit tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_manifest_no_diagnostics() {
        let source = r#"
[project]
name = "test-project"

[packages]
ripgrep = "*"
"#;
        let diags = diagnose(source);
        // Should have no errors (might have 0 or more warnings)
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == Some(DiagnosticSeverity::ERROR))
            .collect();
        assert!(errors.is_empty(), "valid manifest should have no errors: {errors:?}");
    }

    #[test]
    fn test_missing_project_name_error() {
        let source = r#"
[project]
name = ""

[packages]
ripgrep = "*"
"#;
        let diags = diagnose(source);
        assert!(!diags.is_empty(), "empty name should produce diagnostics");
        assert!(
            diags.iter().any(|d| d.message.contains("name")),
            "should mention name: {:?}",
            diags
        );
    }

    #[test]
    fn test_invalid_package_name_error() {
        let source = r#"
[project]
name = "test"

[packages]
"123bad" = "*"
"#;
        let diags = diagnose(source);
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == Some(DiagnosticSeverity::ERROR))
            .collect();
        assert!(!errors.is_empty(), "invalid package name should produce error");
    }

    #[test]
    fn test_unknown_section_warning() {
        let source = r#"
[project]
name = "test"

[bogus]
foo = "bar"
"#;
        let diags = diagnose(source);
        let warnings: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == Some(DiagnosticSeverity::WARNING))
            .collect();
        assert!(
            warnings.iter().any(|w| w.message.contains("unknown section")),
            "should warn about unknown section: {warnings:?}"
        );
    }

    #[test]
    fn test_all_valid_sections_no_warnings() {
        let source = r#"
[project]
name = "test"

[packages]
ripgrep = "*"

[vars]
EDITOR = "vim"

[hook]
on-activate = "echo hi"

[services]

[options]
allow-unfree = false
"#;
        let diags = diagnose(source);
        let section_warnings: Vec<_> = diags
            .iter()
            .filter(|d| {
                d.severity == Some(DiagnosticSeverity::WARNING)
                    && d.message.contains("unknown section")
            })
            .collect();
        assert!(
            section_warnings.is_empty(),
            "valid sections should not warn: {section_warnings:?}"
        );
    }

    #[test]
    fn test_empty_service_command_error() {
        let source = r#"
[project]
name = "test"

[services.web]
command = ""
"#;
        let diags = diagnose(source);
        let errors: Vec<_> = diags
            .iter()
            .filter(|d| d.severity == Some(DiagnosticSeverity::ERROR))
            .collect();
        assert!(
            !errors.is_empty(),
            "empty service command should produce error"
        );
    }

    #[test]
    fn test_extract_quoted_name() {
        assert_eq!(extract_quoted_name("package 'foo' is bad"), Some("foo".to_string()));
        assert_eq!(extract_quoted_name("no quotes here"), None);
    }

    #[test]
    fn test_diagnostic_has_position() {
        let source = r#"
[project]
name = "test"

[unknown_section]
foo = "bar"
"#;
        let diags = diagnose(source);
        let warning = diags
            .iter()
            .find(|d| d.message.contains("unknown section"))
            .expect("should have unknown section warning");
        // Should point to the line with [unknown_section]
        assert!(
            warning.range.start.line > 0,
            "warning should have a non-zero line"
        );
    }
}
