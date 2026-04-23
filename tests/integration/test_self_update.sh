#!/usr/bin/env bash
# Integration test: envo self-update and version commands
# Requires: envo binary built (cargo build)
# Usage: bash tests/integration/test_self_update.sh

set -euo pipefail

ENVO_BIN="${ENVO_BIN:-./target/debug/envo}"
PASS=0
FAIL=0

pass() { echo "  ✓ $1"; PASS=$((PASS + 1)); }
fail() { echo "  ✗ $1"; FAIL=$((FAIL + 1)); }

echo "=== Integration Test: Self-Update & Version ==="
echo ""

if [ ! -f "$ENVO_BIN" ]; then
    echo "ERROR: Binary not found at $ENVO_BIN"
    echo "Run 'cargo build' first."
    exit 1
fi

# ── Test 1: envo version output format ─────────────────────────────

echo "Test 1: envo version output"

VERSION_OUTPUT=$("$ENVO_BIN" version 2>&1)

if echo "$VERSION_OUTPUT" | grep -q "^envo "; then
    pass "version line present"
else
    fail "missing version line: $VERSION_OUTPUT"
fi

if echo "$VERSION_OUTPUT" | grep -q "^installed:"; then
    pass "install path line present"
else
    fail "missing installed line"
fi

if echo "$VERSION_OUTPUT" | grep -q "^nix:"; then
    pass "nix status line present"
else
    fail "missing nix line"
fi

if echo "$VERSION_OUTPUT" | grep -q "^system:"; then
    pass "system line present"
else
    fail "missing system line"
fi

# Verify the version matches Cargo.toml
CARGO_VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
if echo "$VERSION_OUTPUT" | grep -q "$CARGO_VERSION"; then
    pass "version matches Cargo.toml: $CARGO_VERSION"
else
    fail "version mismatch — expected $CARGO_VERSION in output"
fi

# ── Test 2: envo self-update --check handles no release gracefully ─

echo ""
echo "Test 2: envo self-update --check (no release exists yet)"

CHECK_OUTPUT=$("$ENVO_BIN" self-update --check 2>&1 || true)
EXIT_CODE=$?

# This should either say "up to date" or give a network error —
# it should NOT crash with a panic or stack trace
if echo "$CHECK_OUTPUT" | grep -qi "panic\|thread.*panicked"; then
    fail "self-update --check panicked: $CHECK_OUTPUT"
else
    pass "self-update --check did not panic"
fi

# It's OK if it errors (no release exists) — we just want a clean message
if echo "$CHECK_OUTPUT" | grep -qiE "up to date|update available|could not|error|network"; then
    pass "self-update --check produced a clean message"
else
    fail "unexpected output: $CHECK_OUTPUT"
fi

# ── Test 3: Version string is valid semver ─────────────────────────

echo ""
echo "Test 3: Version string format"

VERSION_LINE=$(echo "$VERSION_OUTPUT" | head -1)
VERSION_NUM=$(echo "$VERSION_LINE" | sed 's/envo //')

if echo "$VERSION_NUM" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+'; then
    pass "version is valid semver: $VERSION_NUM"
else
    fail "version is not semver: $VERSION_NUM"
fi

# ── Test 4: System string is valid ─────────────────────────────────

echo ""
echo "Test 4: System string format"

SYSTEM_LINE=$(echo "$VERSION_OUTPUT" | grep "^system:")
SYSTEM_VAL=$(echo "$SYSTEM_LINE" | sed 's/system: //')

case "$SYSTEM_VAL" in
    x86_64-linux|aarch64-linux|aarch64-darwin)
        pass "valid system: $SYSTEM_VAL"
        ;;
    *)
        fail "unexpected system: $SYSTEM_VAL"
        ;;
esac

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
