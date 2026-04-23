//! Command implementations for the envo CLI.
//!
//! Each function here orchestrates the library modules (manifest, lockfile,
//! realize, activate) to perform a user-facing operation. Business logic
//! lives in the library modules, not here — these are thin orchestrators.

use crate::activate::snapshot::ShellType;
use crate::activate::Activator;
use crate::lockfile::resolver::{detect_current_system, resolve_manifest, NixEvaluator};
use crate::lockfile::Lockfile;
use crate::manifest::schema::PackageEntry;
use crate::manifest::Manifest;
use crate::nix_bootstrap;
use crate::realize::Realizer;
use crate::self_update;
use crate::telemetry;
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

/// Initialize a new envo environment in the current directory.
pub fn cmd_init(template_name: Option<&str>, verbose: bool) -> Result<()> {
    let cwd = std::env::current_dir().context("could not determine current directory")?;

    // Handle --template list
    if template_name == Some("list") {
        println!("Available templates:");
        for (name, description) in crate::templates::list_templates() {
            println!("  {name:<20} {description}");
        }
        return Ok(());
    }

    if verbose {
        eprintln!("ℹ Initializing envo environment in {}", cwd.display());
    }

    // Determine project name from directory name
    let project_name = cwd
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("my-project")
        .to_string();

    match template_name {
        Some(name) => {
            // Use the specified template
            let template = crate::templates::get_template(name).ok_or_else(|| {
                let available: Vec<&str> = crate::templates::list_templates()
                    .iter()
                    .map(|(n, _)| *n)
                    .collect();
                anyhow::anyhow!(
                    "unknown template '{name}'. Available templates: {}",
                    available.join(", ")
                )
            })?;

            // Create the .envo directory
            let envo_dir = cwd.join(".envo");
            if envo_dir.exists() {
                bail!("envo environment already exists at {}", envo_dir.display());
            }
            std::fs::create_dir_all(&envo_dir)
                .context("failed to create .envo directory")?;

            // Render and write the template manifest
            let content = crate::templates::render_template(template, &project_name);
            std::fs::write(envo_dir.join("manifest.toml"), &content)
                .context("failed to write manifest.toml")?;

            // Verify it parses correctly
            let manifest = Manifest::from_str(&content)
                .context("template manifest failed validation — this is a bug")?;

            // Telemetry placeholder: record template usage
            // TODO(telemetry): emit event { action: "init", template: name }

            println!("✓ Created envo environment from template '{name}'");
            println!("  Project name: {}", manifest.project_name());
            println!("  Run `envo install` to resolve packages");
        }
        None => {
            // Default init (no template) — use existing Manifest::init
            let manifest = Manifest::init(&cwd)
                .context("failed to initialize envo environment")?;

            println!("✓ Created envo environment in .envo/");
            println!("  Project name: {}", manifest.project_name());
            println!("  Edit .envo/manifest.toml to add packages, then run `envo install`");
        }
    }

    let mut extra = HashMap::new();
    if let Some(t) = template_name {
        extra.insert("template".to_string(), serde_json::json!(t));
    }
    telemetry::track_event("cli", "cli_init", true, None, Some(extra), verbose);

    Ok(())
}

/// Install one or more packages.
pub fn cmd_install(packages: &[String], verbose: bool) -> Result<()> {
    let start = Instant::now();
    let cwd = std::env::current_dir()?;
    let mut manifest = Manifest::load(Some(&cwd))
        .context("no envo environment found (run `envo init` first)")?;

    // Check Nix is available (prompts for install if interactive)
    ensure_nix_available(verbose)?;

    let resolve_only = packages.is_empty();

    // Add packages to manifest
    for pkg in packages {
        if verbose {
            eprintln!("ℹ Adding {pkg} to manifest");
        }
        manifest
            .add_package(pkg, PackageEntry::Short("*".to_string()))
            .with_context(|| format!("invalid package name: {pkg}"))?;
    }

    // Save updated manifest
    manifest.save(&cwd).context("failed to save manifest")?;

    // Resolve lockfile
    if verbose {
        eprintln!("ℹ Resolving packages...");
    }
    let existing_lockfile = Lockfile::load(Some(&cwd)).ok();
    let mut evaluator = NixEvaluator::new();
    let lockfile = resolve_manifest(
        &manifest,
        &mut evaluator,
        existing_lockfile.as_ref(),
    )
    .context("failed to resolve packages")?;

    lockfile.save(&cwd).context("failed to save lockfile")?;

    // Generate shims
    if verbose {
        eprintln!("ℹ Generating shims...");
    }
    let system = detect_current_system();
    let realizer = Realizer::new(&cwd);
    let shim_manifest = realizer
        .generate_shims(&lockfile, &system)
        .context("failed to generate shims")?;

    // Regenerate activation snapshot
    let shell = ShellType::detect();
    let activator = Activator::new(&cwd);
    activator
        .save_snapshot(&manifest, &lockfile, &shim_manifest, shell)
        .context("failed to generate activation snapshot")?;

    if resolve_only {
        println!("✓ Resolved {} package(s)", manifest.packages().len());
    } else {
        for pkg in packages {
            println!("✓ Installed {pkg}");
        }
    }

    let mut extra = HashMap::new();
    extra.insert("package_count".to_string(), serde_json::json!(packages.len()));
    telemetry::track_event(
        "cli", "cli_install", true,
        Some(start.elapsed().as_millis() as u64),
        Some(extra), verbose,
    );

    Ok(())
}

/// Remove a package.
pub fn cmd_uninstall(package: &str, verbose: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let mut manifest = Manifest::load(Some(&cwd))
        .context("no envo environment found (run `envo init` first)")?;

    ensure_nix_available(verbose)?;

    if !manifest.remove_package(package) {
        bail!("package '{package}' is not installed");
    }

    manifest.save(&cwd).context("failed to save manifest")?;

    if verbose {
        eprintln!("ℹ Re-resolving packages...");
    }

    let mut evaluator = NixEvaluator::new();
    let lockfile = resolve_manifest(&manifest, &mut evaluator, None)
        .context("failed to resolve packages")?;

    lockfile.save(&cwd).context("failed to save lockfile")?;

    let system = detect_current_system();
    let realizer = Realizer::new(&cwd);
    let shim_manifest = realizer
        .generate_shims(&lockfile, &system)
        .context("failed to generate shims")?;

    let shell = ShellType::detect();
    let activator = Activator::new(&cwd);
    activator
        .save_snapshot(&manifest, &lockfile, &shim_manifest, shell)
        .context("failed to generate activation snapshot")?;

    println!("✓ Uninstalled {package}");

    Ok(())
}

/// Activate the environment.
pub fn cmd_activate(inline: bool, shell_arg: Option<&str>, verbose: bool) -> Result<()> {
    let start = Instant::now();
    let cwd = std::env::current_dir()?;
    let manifest = Manifest::load(Some(&cwd))
        .context("no envo environment found (run `envo init` first)")?;

    let shell = shell_arg
        .and_then(ShellType::from_str)
        .unwrap_or_else(ShellType::detect);

    // Load or regenerate lockfile if stale
    let lockfile = load_or_resolve_lockfile(&cwd, &manifest)?;

    // Ensure shims exist
    let system = detect_current_system();
    let realizer = Realizer::new(&cwd);
    let shim_manifest = realizer
        .generate_shims(&lockfile, &system)
        .context("failed to generate shims")?;

    let activator = Activator::new(&cwd);

    if inline {
        let script = activator
            .generate_snapshot(&manifest, &lockfile, &shim_manifest, shell)
            .context("failed to generate activation snapshot")?;
        print!("{script}");
    } else {
        let path = activator
            .save_snapshot(&manifest, &lockfile, &shim_manifest, shell)
            .context("failed to save activation snapshot")?;
        println!("{}", path.display());
    }

    let mut extra = HashMap::new();
    extra.insert("shell".to_string(), serde_json::json!(format!("{shell:?}")));
    telemetry::track_event(
        "cli", "cli_activate", true,
        Some(start.elapsed().as_millis() as u64),
        Some(extra), verbose,
    );

    Ok(())
}

/// Deactivate the environment.
pub fn cmd_deactivate(inline: bool, shell_arg: Option<&str>, _verbose: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let manifest = Manifest::load(Some(&cwd))
        .context("no envo environment found (run `envo init` first)")?;

    let shell = shell_arg
        .and_then(ShellType::from_str)
        .unwrap_or_else(ShellType::detect);

    let activator = Activator::new(&cwd);

    if inline {
        let script = activator
            .generate_deactivation(&manifest, shell)
            .context("failed to generate deactivation script")?;
        print!("{script}");
    } else {
        // Without --inline, just print instructions
        println!("Run: eval \"$(envo deactivate --inline)\"");
    }

    Ok(())
}

/// Search for packages in nixpkgs.
pub fn cmd_search(query: &str, json_output: bool, verbose: bool) -> Result<()> {
    ensure_nix_available(verbose)?;

    let output = Command::new("nix")
        .args(["search", "nixpkgs", query, "--json"])
        .output()
        .context("failed to run nix search")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("search failed: {stderr}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let results: serde_json::Value = serde_json::from_str(&stdout)
        .context("failed to parse search results")?;

    let results_map = results.as_object().context("unexpected search output format")?;

    if results_map.is_empty() {
        if json_output {
            println!("[]");
        } else {
            println!("No packages found matching '{query}'");
        }
        return Ok(());
    }

    if json_output {
        // Structured JSON output for programmatic consumers (LSP, extensions)
        let mut items = Vec::new();
        let mut count = 0;
        for (attr_path, info) in results_map {
            if count >= 20 {
                break;
            }
            let pkg_name = attr_path.rsplit('.').next().unwrap_or(attr_path);
            let description = info
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("");
            let version = info
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            items.push(serde_json::json!({
                "name": pkg_name,
                "version": version,
                "description": description,
            }));
            count += 1;
        }
        println!("{}", serde_json::to_string(&items).unwrap());
    } else {
        let mut count = 0;
        for (attr_path, info) in results_map {
            if count >= 20 {
                println!("  ... and {} more (showing first 20)", results_map.len() - 20);
                break;
            }

            let pkg_name = attr_path.rsplit('.').next().unwrap_or(attr_path);
            let description = info
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("");
            let version = info
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if version.is_empty() {
                println!("  {pkg_name} — {description}");
            } else {
                println!("  {pkg_name} ({version}) — {description}");
            }

            count += 1;
        }
    }

    telemetry::track_event("cli", "cli_search", true, None, None, verbose);

    Ok(())
}

/// Run a command inside the activated environment.
pub fn cmd_run(command: &[String], verbose: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let manifest = Manifest::load(Some(&cwd))
        .context("no envo environment found (run `envo init` first)")?;

    ensure_nix_available(verbose)?;

    let lockfile = load_or_resolve_lockfile(&cwd, &manifest)?;

    let system = detect_current_system();
    let realizer = Realizer::new(&cwd);
    let shim_manifest = realizer
        .generate_shims(&lockfile, &system)
        .context("failed to generate shims")?;

    let activator = Activator::new(&cwd);
    let snapshot = activator
        .generate_snapshot(&manifest, &lockfile, &shim_manifest, ShellType::Bash)
        .context("failed to generate activation snapshot")?;

    // Build the command string: source the snapshot, then exec the user's command
    let cmd_str = command.join(" ");
    let full_script = format!("{snapshot}\nexec {cmd_str}");

    let status = Command::new("bash")
        .args(["-c", &full_script])
        .status()
        .with_context(|| format!("failed to run: {cmd_str}"))?;

    std::process::exit(status.code().unwrap_or(1));
}

/// Update all packages to latest versions.
pub fn cmd_update(verbose: bool) -> Result<()> {
    let start = Instant::now();
    let cwd = std::env::current_dir()?;
    let manifest = Manifest::load(Some(&cwd))
        .context("no envo environment found (run `envo init` first)")?;

    ensure_nix_available(verbose)?;

    // Load existing lockfile to compare
    let old_lockfile = Lockfile::load(Some(&cwd)).ok();

    if verbose {
        eprintln!("ℹ Re-resolving all packages...");
    }

    // Resolve from scratch (no existing lockfile = force full re-resolution)
    let mut evaluator = NixEvaluator::new();
    let new_lockfile = resolve_manifest(&manifest, &mut evaluator, None)
        .context("failed to resolve packages")?;

    // Report changes
    let system = detect_current_system();
    if let Some(ref old) = old_lockfile {
        let mut changes = 0;
        for (name, new_pkg) in &new_lockfile.packages {
            if let Some(new_res) = new_pkg.systems.get(&system) {
                let old_path = old.get_store_path(name, &system).unwrap_or("");
                if old_path != new_res.store_path {
                    println!("  ↑ {name}: updated");
                    changes += 1;
                }
            }
        }
        if changes == 0 {
            println!("✓ All packages are up to date");
        } else {
            println!("✓ Updated {changes} package(s)");
        }
    } else {
        println!("✓ Resolved {} package(s)", new_lockfile.packages.len());
    }

    new_lockfile.save(&cwd).context("failed to save lockfile")?;

    // Regenerate shims and snapshot
    let realizer = Realizer::new(&cwd);
    let shim_manifest = realizer
        .generate_shims(&new_lockfile, &system)
        .context("failed to generate shims")?;

    let shell = ShellType::detect();
    let activator = Activator::new(&cwd);
    activator
        .save_snapshot(&manifest, &new_lockfile, &shim_manifest, shell)
        .context("failed to generate activation snapshot")?;

    telemetry::track_event(
        "cli", "cli_update", true,
        Some(start.elapsed().as_millis() as u64),
        None, verbose,
    );

    Ok(())
}

/// Export environment data (currently only SBOM).
pub fn cmd_export(format: &str, output: Option<&str>, _verbose: bool) -> Result<()> {
    if format != "sbom" {
        bail!("unknown export format '{format}'. Supported formats: sbom");
    }

    let cwd = std::env::current_dir()?;
    let _manifest = Manifest::load(Some(&cwd))
        .context("no envo environment found (run `envo init` first)")?;
    let lockfile = Lockfile::load(Some(&cwd))
        .context("no lockfile found (run `envo install` first)")?;

    let sbom = generate_cyclonedx_sbom(&lockfile);

    let json = serde_json::to_string_pretty(&sbom)
        .context("failed to serialize SBOM")?;

    match output {
        Some(path) => {
            std::fs::write(path, &json)
                .with_context(|| format!("failed to write SBOM to {path}"))?;
            println!("✓ SBOM written to {path}");
        }
        None => {
            println!("{json}");
        }
    }

    Ok(())
}

/// Update envo itself to the latest version.
pub fn cmd_self_update(check_only: bool, verbose: bool) -> Result<()> {
    if verbose {
        eprintln!("ℹ Checking for updates...");
    }

    let latest = match self_update::check_latest_version() {
        Ok(v) => v,
        Err(e) => {
            // Don't crash on network errors — just report
            bail!("{e}");
        }
    };

    let current = self_update::CURRENT_VERSION;
    let status = self_update::compare_versions(current, &latest);

    match status {
        self_update::VersionStatus::UpToDate => {
            println!("✓ envo is up to date ({current})");
        }
        self_update::VersionStatus::UpdateAvailable { latest } => {
            if check_only {
                println!(
                    "ℹ Update available: {latest} (current: {current}). \
                     Run 'envo self-update' to install."
                );
            } else {
                if verbose {
                    eprintln!("ℹ Downloading envo {latest}...");
                }
                self_update::download_and_replace(&latest)
                    .context("failed to update envo")?;
                println!("✓ Updated envo from {current} to {latest}");
            }
        }
    }

    Ok(())
}

/// Show version, install location, Nix status, and system info.
pub fn cmd_version(json: bool, _verbose: bool) -> Result<()> {
    let version = self_update::CURRENT_VERSION;
    let install_path = self_update::get_install_path();
    let system = self_update::get_current_system();
    let nix_status = nix_bootstrap::detect_nix();

    if json {
        let output = serde_json::json!({
            "version": version,
            "install_path": install_path,
            "nix": nix_bootstrap::nix_status_to_json(&nix_status),
            "system": system,
        });
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
    } else {
        println!("envo {version}");
        println!("installed: {install_path}");
        println!("nix: {}", nix_bootstrap::format_nix_status(&nix_status));
        println!("system: {system}");
    }

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────

/// Ensure Nix is available, prompting for installation if needed.
///
/// This replaces the old `check_nix_installed()` — instead of just erroring,
/// it offers to install Nix interactively (in TTY mode) or provides a clear
/// error message (in CI/non-interactive mode).
fn ensure_nix_available(verbose: bool) -> Result<()> {
    nix_bootstrap::ensure_nix()
        .context("Nix is required for this command")?;

    // Check that flakes/nix-command are enabled
    if let Err(warning) = nix_bootstrap::check_flake_support() {
        eprintln!("{warning}");
    }

    if verbose {
        let status = nix_bootstrap::detect_nix();
        eprintln!("ℹ Nix: {}", nix_bootstrap::format_nix_status(&status));
    }

    Ok(())
}

/// Load the lockfile, or resolve it if stale or missing.
fn load_or_resolve_lockfile(cwd: &Path, manifest: &Manifest) -> Result<Lockfile> {
    match Lockfile::load(Some(cwd)) {
        Ok(lockfile) if !lockfile.is_stale(manifest) => Ok(lockfile),
        _ => {
            // Stale or missing — resolve
            let existing = Lockfile::load(Some(cwd)).ok();
            let mut evaluator = NixEvaluator::new();
            let lockfile = resolve_manifest(manifest, &mut evaluator, existing.as_ref())
                .context("failed to resolve packages")?;
            lockfile.save(cwd).context("failed to save lockfile")?;
            Ok(lockfile)
        }
    }
}

/// Generate a basic CycloneDX 1.5 SBOM from the lockfile.
fn generate_cyclonedx_sbom(lockfile: &Lockfile) -> serde_json::Value {
    let mut components = Vec::new();

    for (name, pkg) in &lockfile.packages {
        for (system, resolution) in &pkg.systems {
            components.push(serde_json::json!({
                "type": "library",
                "name": name,
                "version": "nixpkgs",
                "purl": format!(
                    "pkg:nix/nixpkgs/{}@{}?system={}",
                    resolution.resolved_attr,
                    lockfile.nixpkgs_revision(),
                    system
                ),
                "properties": [
                    { "name": "nix:store_path", "value": resolution.store_path },
                    { "name": "nix:system", "value": system },
                    { "name": "nix:attr", "value": resolution.resolved_attr },
                ]
            }));
        }
    }

    serde_json::json!({
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "version": 1,
        "metadata": {
            "tools": [{
                "vendor": "envo",
                "name": "envo",
                "version": env!("CARGO_PKG_VERSION"),
            }],
            "properties": [{
                "name": "nix:nixpkgs_revision",
                "value": lockfile.nixpkgs_revision(),
            }]
        },
        "components": components,
    })
}
