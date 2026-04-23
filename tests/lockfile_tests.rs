//! Integration tests for the lockfile module.
//!
//! These test the lockfile public API from outside the crate, using
//! dry-run mode (no Nix required) for unit-level testing, and
//! verify the interface contract that downstream modules depend on.

use envo::lockfile::resolver::{detect_current_system, NixEvaluator};
use envo::lockfile::Lockfile;
use envo::manifest::Manifest;

fn test_manifest_str(packages: &str) -> String {
    format!(
        "[project]\nname = \"test\"\n\n[packages]\n{packages}\n"
    )
}

#[test]
fn test_resolve_and_query_workflow() {
    // Simulate: parse manifest -> resolve -> query lockfile
    let manifest = Manifest::from_str(&test_manifest_str("ripgrep = \"*\"\njq = \"*\"")).unwrap();
    let mut eval = NixEvaluator::dry_run();

    let lockfile =
        envo::lockfile::resolver::resolve_manifest(&manifest, &mut eval, None).unwrap();

    // Lockfile should have both packages
    assert_eq!(lockfile.packages.len(), 2);

    let sys = detect_current_system();

    // Query via get_store_path
    let rg_path = lockfile.get_store_path("ripgrep", &sys);
    assert!(rg_path.is_some());
    assert!(rg_path.unwrap().starts_with("/nix/store/"));

    let jq_path = lockfile.get_store_path("jq", &sys);
    assert!(jq_path.is_some());

    // Query nonexistent
    assert!(lockfile.get_store_path("nonexistent", &sys).is_none());
}

#[test]
fn test_save_resolve_reload_cycle() {
    let tmp = tempfile::tempdir().unwrap();
    let envo_dir = tmp.path().join(".envo");
    std::fs::create_dir_all(&envo_dir).unwrap();

    let manifest = Manifest::from_str(&test_manifest_str("ripgrep = \"*\"")).unwrap();
    let mut eval = NixEvaluator::dry_run();

    let lockfile =
        envo::lockfile::resolver::resolve_manifest(&manifest, &mut eval, None).unwrap();

    // Save
    lockfile.save(tmp.path()).unwrap();

    // Reload
    let reloaded = Lockfile::load(Some(tmp.path())).unwrap();
    assert_eq!(lockfile, reloaded);

    // Staleness: same manifest -> not stale
    assert!(!reloaded.is_stale(&manifest));

    // Modify manifest -> stale
    let manifest2 =
        Manifest::from_str(&test_manifest_str("ripgrep = \"*\"\njq = \"*\"")).unwrap();
    assert!(reloaded.is_stale(&manifest2));
}

#[test]
fn test_partial_resolution_preserves_existing() {
    let manifest1 = Manifest::from_str(&test_manifest_str("ripgrep = \"*\"")).unwrap();
    let mut eval1 = NixEvaluator::dry_run();
    let lockfile1 =
        envo::lockfile::resolver::resolve_manifest(&manifest1, &mut eval1, None).unwrap();

    // Now add jq and re-resolve with the existing lockfile
    let manifest2 =
        Manifest::from_str(&test_manifest_str("ripgrep = \"*\"\njq = \"*\"")).unwrap();
    let mut eval2 = NixEvaluator::dry_run();
    let lockfile2 =
        envo::lockfile::resolver::resolve_manifest(&manifest2, &mut eval2, Some(&lockfile1))
            .unwrap();

    assert_eq!(lockfile2.packages.len(), 2);

    let sys = detect_current_system();

    // ripgrep's store path should be identical (reused from lockfile1)
    assert_eq!(
        lockfile1.get_store_path("ripgrep", &sys),
        lockfile2.get_store_path("ripgrep", &sys),
    );

    // jq should be newly resolved
    assert!(lockfile2.get_store_path("jq", &sys).is_some());
}

#[test]
fn test_all_packages_iterator_contract() {
    let manifest =
        Manifest::from_str(&test_manifest_str("ripgrep = \"*\"\njq = \"*\"")).unwrap();
    let mut eval = NixEvaluator::dry_run();
    let lockfile =
        envo::lockfile::resolver::resolve_manifest(&manifest, &mut eval, None).unwrap();

    let all: Vec<_> = lockfile.all_packages().collect();

    // 2 packages × 1 system each = 2 entries
    assert_eq!(all.len(), 2);

    for (name, system, store_path) in &all {
        assert!(!name.is_empty());
        assert!(system.contains('-'));
        assert!(store_path.starts_with("/nix/store/"));
    }
}

#[test]
fn test_pkg_path_override_in_resolution() {
    // Test that pkg-path in the manifest is respected during resolution
    let manifest = Manifest::from_str(
        r#"
[project]
name = "test"

[packages]
python = { version = "3.12", pkg-path = "python312" }
"#,
    )
    .unwrap();

    let mut eval = NixEvaluator::dry_run();
    let lockfile =
        envo::lockfile::resolver::resolve_manifest(&manifest, &mut eval, None).unwrap();

    // The recorded command should use "python312" not "python"
    let cmds = eval.recorded_commands();
    // cmd[0] = flake metadata, cmd[1] = python eval
    let eval_cmd = &cmds[1];
    let installable = &eval_cmd[3]; // the installable argument
    assert!(
        installable.contains("python312"),
        "expected python312 in installable, got: {installable}"
    );
    assert!(
        installable.contains("python312.outPath"),
        "expected python312.outPath in installable, got: {installable}"
    );
}
