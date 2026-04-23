//! Shim script generation for lazy package realization.
//!
//! Shims are small bash scripts placed in `.envo/bin/` that act as proxies
//! for real binaries. When a shim is executed:
//!
//! 1. It checks if the Nix store path is already realized locally
//! 2. If not, it fetches the package via `nix build --no-link`
//! 3. Once the store path exists, it `exec`s the real binary
//!
//! This enables the "lazy fetch" behavior: nothing downloads until the
//! user actually invokes a tool.

use std::path::Path;

/// Generate the bash shim script content for a single binary.
///
/// The shim checks if `store_path` exists on disk. If not, it runs
/// `nix build` to fetch it from the binary cache. Then it execs the
/// real binary, passing through all arguments.
pub fn generate_shim_script(
    store_path: &str,
    binary_name: &str,
    nixpkgs_revision: &str,
    resolved_attr: &str,
) -> String {
    // We try nix-store --realise first (fastest, no eval), then fall back
    // to nix build with a pinned nixpkgs revision if the store path
    // isn't in any configured substituter.
    format!(
        r#"#!/usr/bin/env bash
set -euo pipefail

STORE_PATH="{store_path}"
BINARY="{binary_name}"

# Fast path: store path already exists locally
if [ -e "$STORE_PATH/bin/$BINARY" ]; then
    exec "$STORE_PATH/bin/$BINARY" "$@"
fi

# Slow path: fetch from binary cache
echo "envo: fetching $BINARY on first use..." >&2

# Try direct store path realization first (fastest, no eval needed)
if nix-store --realise "$STORE_PATH" >/dev/null 2>&1; then
    exec "$STORE_PATH/bin/$BINARY" "$@"
fi

# Fallback: build from pinned nixpkgs revision
if nix build --no-link "nixpkgs/{nixpkgs_revision}#legacyPackages.${{ENVO_SYSTEM:-$(nix eval --raw --impure --expr 'builtins.currentSystem')}}.{resolved_attr}" 2>/dev/null; then
    if [ -e "$STORE_PATH/bin/$BINARY" ]; then
        exec "$STORE_PATH/bin/$BINARY" "$@"
    fi
fi

echo "envo: failed to fetch $BINARY" >&2
echo "envo: store path: $STORE_PATH" >&2
exit 1
"#
    )
}

/// Generate a "meta-shim" for a package whose binaries haven't been
/// discovered yet (the store path hasn't been realized).
///
/// After first realization, the realize module scans the store path's
/// `bin/` directory and replaces this with individual per-binary shims.
pub fn generate_meta_shim_script(
    store_path: &str,
    package_name: &str,
    nixpkgs_revision: &str,
    resolved_attr: &str,
) -> String {
    format!(
        r#"#!/usr/bin/env bash
set -euo pipefail

STORE_PATH="{store_path}"
PACKAGE="{package_name}"

# Check if the store path is already realized
if [ ! -e "$STORE_PATH" ]; then
    echo "envo: fetching $PACKAGE on first use..." >&2

    # Try direct store path realization first
    if ! nix-store --realise "$STORE_PATH" >/dev/null 2>&1; then
        # Fallback: build from pinned nixpkgs
        nix build --no-link "nixpkgs/{nixpkgs_revision}#legacyPackages.${{ENVO_SYSTEM:-$(nix eval --raw --impure --expr 'builtins.currentSystem')}}.{resolved_attr}" 2>/dev/null || {{
            echo "envo: failed to fetch $PACKAGE" >&2
            exit 1
        }}
    fi
fi

# Mark that this package needs binary discovery
ENVO_DIR="$(dirname "$(dirname "$(readlink -f "$0")")")"
touch "$ENVO_DIR/.needs-rescan"

# Try to exec the binary matching the package name
if [ -e "$STORE_PATH/bin/$PACKAGE" ]; then
    exec "$STORE_PATH/bin/$PACKAGE" "$@"
fi

# Package name doesn't match a binary — list available binaries
echo "envo: package '$PACKAGE' is installed but has no binary named '$PACKAGE'" >&2
if [ -d "$STORE_PATH/bin" ]; then
    echo "envo: available binaries:" >&2
    ls -1 "$STORE_PATH/bin/" | sed 's/^/  /' >&2
fi
exit 1
"#
    )
}

/// Check if a shim script's content targets the given store path.
pub fn shim_targets_store_path(shim_content: &str, store_path: &str) -> bool {
    shim_content.contains(&format!("STORE_PATH=\"{store_path}\""))
}

/// Scan a realized store path's `bin/` directory and return the binary names found.
///
/// Returns an empty vec if the path doesn't exist or has no `bin/` directory.
pub fn discover_binaries(store_path: &Path) -> Vec<String> {
    let bin_dir = store_path.join("bin");
    if !bin_dir.is_dir() {
        return Vec::new();
    }

    let mut binaries = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&bin_dir) {
        for entry in entries.flatten() {
            if let Ok(ft) = entry.file_type() {
                if ft.is_file() || ft.is_symlink() {
                    if let Some(name) = entry.file_name().to_str() {
                        if !name.starts_with('.') {
                            binaries.push(name.to_string());
                        }
                    }
                }
            }
        }
    }

    binaries.sort();
    binaries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_shim_script_content() {
        let script = generate_shim_script(
            "/nix/store/abc123-ripgrep-14.1.0",
            "rg",
            "deadbeef",
            "ripgrep",
        );

        assert!(script.starts_with("#!/usr/bin/env bash"));
        assert!(script.contains("set -euo pipefail"));
        assert!(script.contains("STORE_PATH=\"/nix/store/abc123-ripgrep-14.1.0\""));
        assert!(script.contains("BINARY=\"rg\""));
        assert!(script.contains("exec \"$STORE_PATH/bin/$BINARY\" \"$@\""));
        assert!(script.contains(r#"envo: fetching $BINARY on first use..."#));
        assert!(script.contains("nix-store --realise"));
        assert!(script.contains("nixpkgs/deadbeef#legacyPackages"));
    }

    #[test]
    fn test_generate_meta_shim_script_content() {
        let script = generate_meta_shim_script(
            "/nix/store/abc123-ripgrep-14.1.0",
            "ripgrep",
            "deadbeef",
            "ripgrep",
        );

        assert!(script.starts_with("#!/usr/bin/env bash"));
        assert!(script.contains("PACKAGE=\"ripgrep\""));
        assert!(script.contains(".needs-rescan"));
        assert!(script.contains("available binaries"));
    }

    #[test]
    fn test_shim_targets_store_path() {
        let script = generate_shim_script(
            "/nix/store/abc123-ripgrep",
            "rg",
            "rev",
            "ripgrep",
        );

        assert!(shim_targets_store_path(&script, "/nix/store/abc123-ripgrep"));
        assert!(!shim_targets_store_path(&script, "/nix/store/other-path"));
    }

    #[test]
    fn test_discover_binaries_nonexistent_path() {
        let binaries = discover_binaries(Path::new("/nonexistent/path"));
        assert!(binaries.is_empty());
    }

    #[test]
    fn test_discover_binaries_in_temp_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let bin_dir = tmp.path().join("bin");
        std::fs::create_dir(&bin_dir).unwrap();

        std::fs::write(bin_dir.join("rg"), "fake").unwrap();
        std::fs::write(bin_dir.join("ripgrep"), "fake").unwrap();
        std::fs::write(bin_dir.join(".hidden"), "fake").unwrap();

        let binaries = discover_binaries(tmp.path());
        assert_eq!(binaries, vec!["rg", "ripgrep"]);
    }
}