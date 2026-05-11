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
/// On first invocation the meta-shim:
///   1. Realizes the Nix store path (fetching from the binary cache)
///   2. Creates lightweight per-binary shims for every binary in the store
///      path's `bin/` directory so callers can use the real binary name
///      (e.g. `rg` for the `ripgrep` package) immediately after
///   3. Execs the target binary, handling the common case where the binary
///      name differs from the package name (e.g. ripgrep → rg)
///
/// The lightweight inline shims are replaced by full shims on the next
/// `envo activate` call via the `.needs-rescan` marker.
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

# Ensure the store path is realized (fetch on first use)
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

# Create lightweight per-binary shims for every binary in the store path so
# callers can use the real binary name (e.g. `rg`) right away.  These are
# replaced by full shims on the next `envo activate` call.
_envo_bin="$(dirname "$(readlink -f "$0")")"
if [ -d "$STORE_PATH/bin" ]; then
    for _bp in "$STORE_PATH/bin/"*; do
        _bn="$(basename "$_bp")"
        case "$_bn" in .*) continue ;; esac
        [ -f "$_bp" ] || [ -L "$_bp" ] || continue
        _shim="$_envo_bin/$_bn"
        # Only create if no shim exists yet (avoids clobbering real shims)
        if [ ! -e "$_shim" ]; then
            printf '#!/usr/bin/env bash\nexec "%s/bin/%s" "$@"\n' "$STORE_PATH" "$_bn" > "$_shim"
            chmod +x "$_shim"
        fi
    done
fi

# Mark for rescan so the next `envo activate` generates full per-binary shims
touch "$(dirname "$_envo_bin")/.needs-rescan"

# Exec the target binary.  Try the package name first (common case: jq, python,
# etc.), then fall back to scanning bin/ for packages where the binary name
# differs from the package name (e.g. ripgrep → rg).
if [ -e "$STORE_PATH/bin/$PACKAGE" ]; then
    exec "$STORE_PATH/bin/$PACKAGE" "$@"
fi

_first="" _count=0
if [ -d "$STORE_PATH/bin" ]; then
    for _bp in "$STORE_PATH/bin/"*; do
        case "$(basename "$_bp")" in .*) continue ;; esac
        [ -f "$_bp" ] || [ -L "$_bp" ] || continue
        _count=$((_count + 1))
        [ -z "$_first" ] && _first="$_bp"
    done
fi

if [ "$_count" -eq 0 ]; then
    echo "envo: no binaries found for package '$PACKAGE' in $STORE_PATH/bin" >&2
    exit 1
elif [ "$_count" -eq 1 ]; then
    exec "$_first" "$@"
else
    # Multiple binaries and none matches the package name — tell the user
    echo "envo: package '$PACKAGE' provides multiple binaries; run by name:" >&2
    ls -1 "$STORE_PATH/bin/" | grep -v '^\.' | sed 's/^/  /' >&2
    exit 1
fi
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
        // Creates per-binary inline shims on first use
        assert!(script.contains("_envo_bin="));
        assert!(script.contains("chmod +x"));
        // Falls back to scanning bin/ when binary name != package name
        assert!(script.contains("_count=0"));
        assert!(script.contains("_first="));
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