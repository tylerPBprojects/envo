#!/usr/bin/env bash
# Integration test: error cases
# Verifies that envo produces clear error messages for common mistakes.
# Usage: bash tests/integration/test_error_cases.sh

set -euo pipefail

ENVO_BIN="${ENVO_BIN:-./target/debug/envo}"
PASS=0
FAIL=0

pass() { echo "  ✓ $1"; PASS=$((PASS + 1)); }
fail() { echo "  ✗ $1"; FAIL=$((FAIL + 1)); }

echo "=== Integration Test: Error Cases ==="
echo ""

if [ ! -f "$ENVO_BIN" ]; then
    echo "ERROR: Binary not found at $ENVO_BIN"
    echo "Run 'cargo build' first."
    exit 1
fi

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

ENVO="$(cd "$(dirname "$ENVO_BIN")" && pwd)/$(basename "$ENVO_BIN")"

# ── Test 1: install without init ───────────────────────────────────

echo "Test 1: install without init"
cd "$TMPDIR"
mkdir test1 && cd test1

RESULT=$("$ENVO" install ripgrep 2>&1 || true)
if echo "$RESULT" | grep -qi "no envo environment\|envo init"; then
    pass "clear error for install without init"
else
    fail "error message: $RESULT"
fi

# ── Test 2: activate without init ──────────────────────────────────

echo ""
echo "Test 2: activate without init"
cd "$TMPDIR"
mkdir test2 && cd test2

RESULT=$("$ENVO" activate --inline 2>&1 || true)
if echo "$RESULT" | grep -qi "no envo environment\|envo init"; then
    pass "clear error for activate without init"
else
    fail "error message: $RESULT"
fi

# ── Test 3: double init ───────────────────────────────────────────

echo ""
echo "Test 3: double init"
cd "$TMPDIR"
mkdir test3 && cd test3

"$ENVO" init >/dev/null 2>&1
RESULT=$("$ENVO" init 2>&1 || true)
if echo "$RESULT" | grep -qi "already exists"; then
    pass "clear error for double init"
else
    fail "error message: $RESULT"
fi

# ── Test 4: uninstall package not installed ────────────────────────

echo ""
echo "Test 4: uninstall non-installed package"
cd "$TMPDIR"
mkdir test4 && cd test4
"$ENVO" init >/dev/null 2>&1

RESULT=$("$ENVO" uninstall nonexistent-pkg 2>&1 || true)
if echo "$RESULT" | grep -qi "not installed"; then
    pass "clear error for uninstalling missing package"
else
    fail "error message: $RESULT"
fi

# ── Test 5: export without lockfile ────────────────────────────────

echo ""
echo "Test 5: export sbom without lockfile"
cd "$TMPDIR"
mkdir test5 && cd test5
"$ENVO" init >/dev/null 2>&1

RESULT=$("$ENVO" export sbom 2>&1 || true)
if echo "$RESULT" | grep -qi "no lockfile\|install"; then
    pass "clear error for export without lockfile"
else
    fail "error message: $RESULT"
fi

# ── Test 6: unknown export format ──────────────────────────────────

echo ""
echo "Test 6: unknown export format"
cd "$TMPDIR"
mkdir test6 && cd test6
"$ENVO" init >/dev/null 2>&1

RESULT=$("$ENVO" export csv 2>&1 || true)
if echo "$RESULT" | grep -qi "unknown export format"; then
    pass "clear error for unknown export format"
else
    fail "error message: $RESULT"
fi

# ── Test 7: install nonexistent package (requires Nix) ─────────────

echo ""
echo "Test 7: install nonexistent package"

if command -v nix &> /dev/null; then
    cd "$TMPDIR"
    mkdir test7 && cd test7
    "$ENVO" init >/dev/null 2>&1

    RESULT=$("$ENVO" install this-package-definitely-does-not-exist-xyz 2>&1 || true)
    EXIT_CODE=$?
    if [ $EXIT_CODE -ne 0 ] || echo "$RESULT" | grep -qi "failed\|error\|not found"; then
        pass "error for nonexistent package"
    else
        fail "no error for nonexistent package: $RESULT"
    fi
else
    pass "nonexistent package test (skipped — Nix not installed)"
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
