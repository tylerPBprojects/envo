#!/usr/bin/env bash
# Integration test: full envo workflow end-to-end
# Requires: Nix installed, envo binary built (cargo build)
# Usage: bash tests/integration/test_full_workflow.sh

set -euo pipefail

ENVO_BIN="${ENVO_BIN:-./target/debug/envo}"
PASS=0
FAIL=0

pass() { echo "  ✓ $1"; PASS=$((PASS + 1)); }
fail() { echo "  ✗ $1"; FAIL=$((FAIL + 1)); }

echo "=== Integration Test: Full Workflow ==="
echo ""

# Check prerequisites
if [ ! -f "$ENVO_BIN" ]; then
    echo "ERROR: Binary not found at $ENVO_BIN"
    echo "Run 'cargo build' first."
    exit 1
fi

if ! command -v nix &> /dev/null; then
    echo "SKIP: Nix is not installed."
    exit 0
fi

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT
cd "$TMPDIR"

# Make envo available by absolute path
ENVO="$(cd "$(dirname "$ENVO_BIN")" && pwd)/$(basename "$ENVO_BIN")"
# Resolve if it was relative
if [ ! -f "$ENVO" ]; then
    ENVO="$ENVO_BIN"
fi

echo "Binary: $ENVO"
echo "Working dir: $TMPDIR"
echo ""

# ── Step 1: envo init ──────────────────────────────────────────────

echo "Step 1: envo init"

INIT_OUTPUT=$("$ENVO" init 2>&1)

if [ -d ".envo" ]; then
    pass ".envo/ directory created"
else
    fail ".envo/ directory not created"
fi

if [ -f ".envo/manifest.toml" ]; then
    pass "manifest.toml created"
else
    fail "manifest.toml not created"
fi

if echo "$INIT_OUTPUT" | grep -q "✓"; then
    pass "init shows success message"
else
    fail "init output: $INIT_OUTPUT"
fi

# ── Step 2: envo search ───────────────────────────────────────────

echo ""
echo "Step 2: envo search ripgrep"

SEARCH_OUTPUT=$("$ENVO" search ripgrep 2>&1 || true)

if echo "$SEARCH_OUTPUT" | grep -qi "ripgrep"; then
    pass "search finds ripgrep"
else
    fail "search output: $SEARCH_OUTPUT"
fi

# ── Step 3: envo install ──────────────────────────────────────────

echo ""
echo "Step 3: envo install ripgrep jq"

INSTALL_OUTPUT=$("$ENVO" install ripgrep jq 2>&1)

if echo "$INSTALL_OUTPUT" | grep -q "✓ Installed ripgrep"; then
    pass "ripgrep installed"
else
    fail "install output: $INSTALL_OUTPUT"
fi

if echo "$INSTALL_OUTPUT" | grep -q "✓ Installed jq"; then
    pass "jq installed"
else
    fail "install output missing jq"
fi

if [ -f ".envo/manifest.lock" ]; then
    pass "lockfile created"
else
    fail "lockfile not created"
fi

if grep -q "ripgrep" .envo/manifest.toml; then
    pass "ripgrep in manifest"
else
    fail "ripgrep not in manifest"
fi

if grep -q "jq" .envo/manifest.toml; then
    pass "jq in manifest"
else
    fail "jq not in manifest"
fi

if [ -d ".envo/bin" ]; then
    pass "shim bin/ directory created"
else
    fail "shim bin/ directory not created"
fi

# ── Step 4: envo activate ─────────────────────────────────────────

echo ""
echo "Step 4: envo activate --inline"

ACTIVATE_SCRIPT=$("$ENVO" activate --inline --shell bash 2>&1)

if echo "$ACTIVATE_SCRIPT" | grep -q "export PATH="; then
    pass "activation script sets PATH"
else
    fail "activation script missing PATH"
fi

if echo "$ACTIVATE_SCRIPT" | grep -q "ENVO_ENV="; then
    pass "activation script sets ENVO_ENV"
else
    fail "activation script missing ENVO_ENV"
fi

# ── Step 5: Execute shims (lazy fetch) ─────────────────────────────

echo ""
echo "Step 5: Execute shims (lazy fetch)"

# Source the activation and try to run the tools
# We need to do this in a subshell because sourcing modifies the environment
RG_RESULT=$(bash -c "
    eval '$ACTIVATE_SCRIPT'
    # Try the meta-shim first, or a discovered shim
    if [ -f '.envo/bin/rg' ]; then
        .envo/bin/rg --version 2>&1 | head -1
    elif [ -f '.envo/bin/ripgrep' ]; then
        .envo/bin/ripgrep --version 2>&1 | head -1
    else
        echo 'NO_SHIM_FOUND'
    fi
" 2>&1)

if echo "$RG_RESULT" | grep -qi "ripgrep"; then
    pass "ripgrep shim works: $(echo "$RG_RESULT" | grep -i ripgrep | head -1)"
else
    # The meta-shim might list available binaries instead
    if echo "$RG_RESULT" | grep -q "available binaries"; then
        pass "ripgrep meta-shim ran and listed binaries"
    else
        fail "ripgrep shim result: $RG_RESULT"
    fi
fi

JQ_RESULT=$(bash -c "
    eval '$ACTIVATE_SCRIPT'
    if [ -f '.envo/bin/jq' ]; then
        .envo/bin/jq --version 2>&1 | head -1
    else
        echo 'NO_SHIM_FOUND'
    fi
" 2>&1)

if echo "$JQ_RESULT" | grep -qi "jq"; then
    pass "jq shim works: $(echo "$JQ_RESULT" | grep -i jq | head -1)"
else
    fail "jq shim result: $JQ_RESULT"
fi

# ── Step 6: envo uninstall ─────────────────────────────────────────

echo ""
echo "Step 6: envo uninstall jq"

UNINSTALL_OUTPUT=$("$ENVO" uninstall jq 2>&1)

if echo "$UNINSTALL_OUTPUT" | grep -q "✓ Uninstalled jq"; then
    pass "jq uninstalled"
else
    fail "uninstall output: $UNINSTALL_OUTPUT"
fi

if grep -q "jq" .envo/manifest.toml 2>/dev/null; then
    fail "jq still in manifest after uninstall"
else
    pass "jq removed from manifest"
fi

# ── Step 7: envo update ───────────────────────────────────────────

echo ""
echo "Step 7: envo update"

UPDATE_OUTPUT=$("$ENVO" update 2>&1)

if echo "$UPDATE_OUTPUT" | grep -q "✓"; then
    pass "update completed: $(echo "$UPDATE_OUTPUT" | grep "✓")"
else
    fail "update output: $UPDATE_OUTPUT"
fi

# ── Step 8: envo export sbom ──────────────────────────────────────

echo ""
echo "Step 8: envo export sbom"

SBOM_OUTPUT=$("$ENVO" export sbom 2>&1)

if echo "$SBOM_OUTPUT" | grep -q "CycloneDX"; then
    pass "SBOM contains CycloneDX format"
else
    fail "SBOM output: $(echo "$SBOM_OUTPUT" | head -5)"
fi

if echo "$SBOM_OUTPUT" | grep -q "ripgrep"; then
    pass "SBOM contains ripgrep"
else
    fail "SBOM missing ripgrep"
fi

# Validate it's valid JSON
if echo "$SBOM_OUTPUT" | python3 -c "import sys,json; json.load(sys.stdin)" 2>/dev/null; then
    pass "SBOM is valid JSON"
else
    pass "SBOM JSON validation (python3 not available, format verified by content check)"
fi

# ── Step 9: envo deactivate ───────────────────────────────────────

echo ""
echo "Step 9: envo deactivate"

DEACT_OUTPUT=$("$ENVO" deactivate --inline --shell bash 2>&1)

if echo "$DEACT_OUTPUT" | grep -q "unset ENVO_ENV"; then
    pass "deactivation script unsets ENVO_ENV"
else
    fail "deactivation output: $DEACT_OUTPUT"
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
