#!/usr/bin/env bash
# Integration test: lazy fetch via shims with real Nix
# Requires: Nix installed, network access
# Usage: bash tests/integration/test_lazy_fetch.sh
#
# This test generates shims manually (simulating what the realize module does)
# and verifies that the shim correctly fetches and execs a real Nix package.

set -euo pipefail

PASS=0
FAIL=0

pass() { echo "  ✓ $1"; PASS=$((PASS + 1)); }
fail() { echo "  ✗ $1"; FAIL=$((FAIL + 1)); }

echo "=== Integration Test: Lazy Fetch via Shims ==="
echo ""

if ! command -v nix &> /dev/null; then
    echo "SKIP: nix is not installed."
    exit 0
fi

# Detect current system
SYSTEM=$(nix eval --raw --impure --expr 'builtins.currentSystem' 2>/dev/null || echo "unknown")
if [ "$SYSTEM" = "unknown" ]; then
    echo "SKIP: could not detect current system"
    exit 0
fi
echo "System: $SYSTEM"

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT
BIN_DIR="$TMPDIR/.envo/bin"
mkdir -p "$BIN_DIR"

# ── Test 1: Resolve ripgrep and generate a shim ────────────────────

echo ""
echo "Test 1: Resolve ripgrep, generate shim, execute"

# Resolve ripgrep's store path
export NIXPKGS_ALLOW_UNFREE=1
STORE_PATH=$(nix eval --json "nixpkgs#legacyPackages.${SYSTEM}.ripgrep.outPath" --impure 2>/dev/null | tr -d '"')

if [ -z "$STORE_PATH" ]; then
    fail "could not resolve ripgrep store path"
else
    pass "resolved ripgrep: $STORE_PATH"

    # Get nixpkgs revision
    NIXPKGS_REV=$(nix flake metadata nixpkgs --json 2>/dev/null | python3 -c "import sys,json; print(json.load(sys.stdin)['revision'])" 2>/dev/null || echo "unknown")

    # Generate a shim script (simulating what the Rust code does)
    cat > "$BIN_DIR/rg" << SHIMEOF
#!/usr/bin/env bash
set -euo pipefail

STORE_PATH="$STORE_PATH"
BINARY="rg"

# Fast path: already realized
if [ -e "\$STORE_PATH/bin/\$BINARY" ]; then
    exec "\$STORE_PATH/bin/\$BINARY" "\$@"
fi

# Slow path: fetch
echo "envo: fetching \$BINARY on first use..." >&2
if nix-store --realise "\$STORE_PATH" >/dev/null 2>&1; then
    exec "\$STORE_PATH/bin/\$BINARY" "\$@"
fi

echo "envo: failed to fetch \$BINARY" >&2
exit 1
SHIMEOF
    chmod +x "$BIN_DIR/rg"

    # Execute the shim
    RG_OUTPUT=$("$BIN_DIR/rg" --version 2>&1)
    if echo "$RG_OUTPUT" | grep -q "ripgrep"; then
        pass "shim executed ripgrep successfully: $(echo "$RG_OUTPUT" | head -1)"
    else
        fail "shim did not produce expected output: $RG_OUTPUT"
    fi

    # Verify the store path now exists
    if [ -e "$STORE_PATH/bin/rg" ]; then
        pass "store path is realized after first shim execution"
    else
        fail "store path not found after shim execution"
    fi

    # Run again — should be instant (no fetch message)
    SECOND_OUTPUT=$("$BIN_DIR/rg" --version 2>&1)
    if echo "$SECOND_OUTPUT" | grep -q "fetching"; then
        fail "second run should not fetch again"
    else
        pass "second run is instant (no fetch message)"
    fi
fi

# ── Test 2: Shim passes arguments correctly ────────────────────────

echo ""
echo "Test 2: Shim passes arguments"

# Search for a pattern in a string using ripgrep via the shim
SEARCH_RESULT=$(echo "hello world" | "$BIN_DIR/rg" "world" 2>/dev/null || true)
if echo "$SEARCH_RESULT" | grep -q "world"; then
    pass "shim passes arguments correctly"
else
    fail "shim did not pass arguments: $SEARCH_RESULT"
fi

# ── Test 3: Multiple packages ─────────────────────────────────────

echo ""
echo "Test 3: Multiple packages (add jq shim)"

JQ_STORE=$(nix eval --json "nixpkgs#legacyPackages.${SYSTEM}.jq.outPath" --impure 2>/dev/null | tr -d '"')

if [ -n "$JQ_STORE" ]; then
    cat > "$BIN_DIR/jq" << JQEOF
#!/usr/bin/env bash
set -euo pipefail
STORE_PATH="$JQ_STORE"
BINARY="jq"
if [ -e "\$STORE_PATH/bin/\$BINARY" ]; then
    exec "\$STORE_PATH/bin/\$BINARY" "\$@"
fi
echo "envo: fetching \$BINARY on first use..." >&2
nix-store --realise "\$STORE_PATH" >/dev/null 2>&1 || { echo "envo: failed" >&2; exit 1; }
exec "\$STORE_PATH/bin/\$BINARY" "\$@"
JQEOF
    chmod +x "$BIN_DIR/jq"

    JQ_OUTPUT=$("$BIN_DIR/jq" --version 2>&1)
    if echo "$JQ_OUTPUT" | grep -q "jq"; then
        pass "jq shim works: $JQ_OUTPUT"
    else
        fail "jq shim failed: $JQ_OUTPUT"
    fi

    # Both shims should coexist
    if [ -f "$BIN_DIR/rg" ] && [ -f "$BIN_DIR/jq" ]; then
        pass "multiple shims coexist"
    else
        fail "shims missing after adding second package"
    fi
else
    fail "could not resolve jq store path"
fi

# ── Test 4: Shim with PATH ────────────────────────────────────────

echo ""
echo "Test 4: Using shims via PATH"

# Add the shim dir to PATH and verify tools are callable by name
export PATH="$BIN_DIR:$PATH"

if command -v rg &> /dev/null; then
    RG_VIA_PATH=$(rg --version 2>&1 | head -1)
    pass "rg available via PATH: $RG_VIA_PATH"
else
    fail "rg not found via PATH"
fi

# ── Results ────────────────────────────────────────────────────────

echo ""
echo "=== Results ==="
echo "  Passed: $PASS"
echo "  Failed: $FAIL"
echo ""

if [ "$FAIL" -gt 0 ]; then
    echo "SOME TESTS FAILED"
    exit 1
else
    echo "ALL TESTS PASSED"
    exit 0
fi
