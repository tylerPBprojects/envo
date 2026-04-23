//! External integration tests for LSP diagnostics, completion, and hover.

use envo::lsp::completion::get_completions;
use envo::lsp::diagnostics::diagnose;
use envo::lsp::hover::get_hover;
use tower_lsp::lsp_types::DiagnosticSeverity;

// ── Diagnostics tests ─────────────────────────────────────────────

#[test]
fn test_diagnose_valid_manifest() {
    let source = r#"
[project]
name = "my-app"
description = "A test project"

[packages]
ripgrep = "*"
jq = "*"

[vars]
EDITOR = "vim"
"#;
    let diags = diagnose(source);
    let errors: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == Some(DiagnosticSeverity::ERROR))
        .collect();
    assert!(errors.is_empty(), "valid manifest should have no errors: {errors:?}");
}

#[test]
fn test_diagnose_missing_project_section() {
    let source = r#"
[packages]
ripgrep = "*"
"#;
    let diags = diagnose(source);
    let errors: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == Some(DiagnosticSeverity::ERROR))
        .collect();
    assert!(!errors.is_empty(), "missing [project] should produce an error");
}

#[test]
fn test_diagnose_unknown_section_warns() {
    let source = r#"
[project]
name = "test"

[foobar]
key = "value"
"#;
    let diags = diagnose(source);
    let warnings: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.severity == Some(DiagnosticSeverity::WARNING)
                && d.message.contains("unknown section")
        })
        .collect();
    assert!(!warnings.is_empty(), "unknown section should warn: {diags:?}");
}

#[test]
fn test_diagnose_invalid_system_string() {
    let source = r#"
[project]
name = "test"

[options]
systems = ["x86_64-linux", "invalid-system"]
"#;
    let diags = diagnose(source);
    let errors: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == Some(DiagnosticSeverity::ERROR))
        .collect();
    assert!(
        !errors.is_empty(),
        "invalid system should produce error: {diags:?}"
    );
}

// ── Completion tests ──────────────────────────────────────────────

#[test]
fn test_completions_at_top_level() {
    let source = "\n[project]\nname = \"test\"\n";
    let items = get_completions(source, 0, 0);
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"[packages]"), "should offer [packages]: {labels:?}");
    assert!(labels.contains(&"[vars]"), "should offer [vars]: {labels:?}");
}

#[test]
fn test_completions_in_options_section() {
    let source = "[project]\nname = \"test\"\n\n[options]\n";
    let items = get_completions(source, 4, 0);
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(
        labels.contains(&"allow-unfree"),
        "should offer allow-unfree: {labels:?}"
    );
    assert!(
        labels.contains(&"nixpkgs-channel"),
        "should offer nixpkgs-channel: {labels:?}"
    );
}

#[test]
fn test_completions_in_packages_with_equals() {
    let source = "[project]\nname = \"test\"\n\n[packages]\nripgrep = ";
    let items = get_completions(source, 4, 10);
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(
        labels.contains(&"\"*\""),
        "should offer wildcard version: {labels:?}"
    );
}

// ── Hover tests ───────────────────────────────────────────────────

#[test]
fn test_hover_project_section() {
    let source = "[project]\nname = \"test\"";
    let hover = get_hover(source, 0, 3);
    assert!(hover.is_some());
    assert!(hover.unwrap().contains("Project metadata"));
}

#[test]
fn test_hover_packages_section() {
    let source = "[project]\nname = \"test\"\n\n[packages]\nrg = \"*\"";
    let hover = get_hover(source, 3, 3);
    assert!(hover.is_some());
    assert!(hover.unwrap().contains("Packages to install"));
}

#[test]
fn test_hover_on_activate_key() {
    let source = "[hook]\non-activate = \"echo hi\"";
    let hover = get_hover(source, 1, 5);
    assert!(hover.is_some());
    assert!(hover.unwrap().contains("Bash script"));
}

#[test]
fn test_hover_no_content_on_comment() {
    let source = "# just a comment\n[project]\nname = \"test\"";
    let hover = get_hover(source, 0, 5);
    assert!(hover.is_none());
}
