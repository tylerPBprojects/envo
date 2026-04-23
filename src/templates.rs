//! Embedded environment templates for `envo init --template`.
//!
//! Templates are compiled into the binary as string constants so the CLI
//! is fully self-contained — no network fetch, no "templates directory not found"
//! failures. Templates provide pre-built manifest.toml content for common
//! use cases.
//!
//! # Design decision: embedded vs. fetched
//!
//! We embed templates as `const &str` rather than fetching from a registry
//! because: (1) the binary stays self-contained, (2) templates are small
//! (<1KB each), (3) offline usage works, (4) no version-mismatch risk.
//! A template registry can be added later as an additional source.

/// A template definition with name, description, and manifest content.
pub struct Template {
    /// Template name used in `--template <name>`.
    pub name: &'static str,

    /// One-line description shown in `--template list`.
    pub description: &'static str,

    /// The manifest.toml content.
    pub manifest: &'static str,
}

/// All available templates.
///
/// When adding a new template, add it here and it will automatically
/// appear in `envo init --template list`.
pub const TEMPLATES: &[Template] = &[
    Template {
        name: "default",
        description: "Empty environment (add packages manually)",
        manifest: DEFAULT_TEMPLATE,
    },
    Template {
        name: "cuda-pytorch",
        description: "PyTorch + CUDA environment with lazy fetch",
        manifest: CUDA_PYTORCH_TEMPLATE,
    },
    Template {
        name: "cpu-pytorch",
        description: "PyTorch (CPU-only) environment for development without GPU",
        manifest: CPU_PYTORCH_TEMPLATE,
    },
];

/// The default empty template (matches what `envo init` creates without --template).
const DEFAULT_TEMPLATE: &str = r#"[project]
name = "{project_name}"

[packages]

[vars]

[options]
"#;

/// PyTorch + CUDA template.
///
/// Uses `pkg-path` to map simple package names to nixpkgs attribute paths.
/// Python packages in nixpkgs are nested under `python3Packages.*`, which
/// TOML would interpret as a nested table if used as a bare key. The
/// `pkg-path` field avoids this by keeping the package name simple and
/// specifying the full nixpkgs attribute separately.
///
/// This template works on both GPU and CPU machines — PyTorch detects
/// CUDA availability at runtime, not at install time.
const CUDA_PYTORCH_TEMPLATE: &str = r#"[project]
name = "{project_name}"
description = "PyTorch + CUDA environment with lazy fetch"

[packages]
python = { pkg-path = "python312" }
torch = { pkg-path = "python312Packages.torch" }
torchvision = { pkg-path = "python312Packages.torchvision" }

[vars]
CUDA_VISIBLE_DEVICES = "0"

[hook]
on-activate = '''
echo "PyTorch CUDA environment ready"
python3 -c "import torch; print(f'PyTorch {torch.__version__}, CUDA available: {torch.cuda.is_available()}')" 2>/dev/null || true
'''

[options]
allow-unfree = true
"#;

/// CPU-only PyTorch template.
///
/// Uses `python312Packages.torch` without CUDA — on CPU-only machines
/// this is the same package (nixpkgs builds CPU-only by default on systems
/// without CUDA support). The `allow-unfree` flag is still set because
/// some PyTorch dependencies may be unfree.
///
/// This template is useful for:
/// - Development on machines without GPUs
/// - CI testing
/// - Quick validation of the lazy-fetch workflow
const CPU_PYTORCH_TEMPLATE: &str = r#"[project]
name = "{project_name}"
description = "PyTorch (CPU-only) environment for development"

[packages]
python = { pkg-path = "python312" }
torch = { pkg-path = "python312Packages.torch" }

[vars]

[hook]
on-activate = '''
echo "PyTorch CPU environment ready"
python3 -c "import torch; print(f'PyTorch {torch.__version__}')" 2>/dev/null || true
'''

[options]
allow-unfree = true
"#;

/// Look up a template by name.
///
/// Returns `None` if the template name is not recognized.
pub fn get_template(name: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|t| t.name == name)
}

/// Get a list of all available template names and descriptions.
pub fn list_templates() -> Vec<(&'static str, &'static str)> {
    TEMPLATES.iter().map(|t| (t.name, t.description)).collect()
}

/// Render a template's manifest content, substituting `{project_name}`.
///
/// If the template contains `{project_name}`, it is replaced with the
/// given project name. This allows templates to have a meaningful
/// project name rather than a hardcoded one.
pub fn render_template(template: &Template, project_name: &str) -> String {
    template.manifest.replace("{project_name}", project_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;

    #[test]
    fn test_all_templates_parse_successfully() {
        for template in TEMPLATES {
            let rendered = render_template(template, "test-project");
            let result = Manifest::from_str(&rendered);
            assert!(
                result.is_ok(),
                "template '{}' failed to parse: {:?}",
                template.name,
                result.err()
            );
        }
    }

    #[test]
    fn test_get_template_by_name() {
        assert!(get_template("default").is_some());
        assert!(get_template("cuda-pytorch").is_some());
        assert!(get_template("cpu-pytorch").is_some());
        assert!(get_template("nonexistent").is_none());
    }

    #[test]
    fn test_list_templates_has_all() {
        let list = list_templates();
        assert_eq!(list.len(), 3);
        let names: Vec<&str> = list.iter().map(|(n, _)| *n).collect();
        assert!(names.contains(&"default"));
        assert!(names.contains(&"cuda-pytorch"));
        assert!(names.contains(&"cpu-pytorch"));
    }

    #[test]
    fn test_render_substitutes_project_name() {
        let template = get_template("default").unwrap();
        let rendered = render_template(template, "my-cool-project");
        assert!(rendered.contains("my-cool-project"));
        assert!(!rendered.contains("{project_name}"));
    }

    #[test]
    fn test_cuda_template_has_unfree() {
        let template = get_template("cuda-pytorch").unwrap();
        assert!(template.manifest.contains("allow-unfree = true"));
    }

    #[test]
    fn test_cuda_template_has_pkg_path() {
        let template = get_template("cuda-pytorch").unwrap();
        assert!(template.manifest.contains("pkg-path"));
        assert!(template.manifest.contains("python312Packages.torch"));
    }

    #[test]
    fn test_cpu_template_parses_with_correct_packages() {
        let template = get_template("cpu-pytorch").unwrap();
        let rendered = render_template(template, "test");
        let manifest = Manifest::from_str(&rendered).unwrap();
        let packages = manifest.packages();
        assert!(packages.contains_key("python"));
        assert!(packages.contains_key("torch"));
        assert_eq!(
            packages["python"].pkg_path.as_deref(),
            Some("python312")
        );
        assert_eq!(
            packages["torch"].pkg_path.as_deref(),
            Some("python312Packages.torch")
        );
    }
}
