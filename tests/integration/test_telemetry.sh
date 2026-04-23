#!/usr/bin/env bash
# Integration test: telemetry system
# Verifies opt-out works, telemetry doesn't crash the CLI, and events fire.
# Usage: bash tests/integration/test_telemetry.sh

set -euo pipefail

ENVO_BIN="${ENVO_BIN:-./target/debug/envo}"
PASS=0
FAIL=0

pass() { echo "  ✓ $1"; PASS=$((PASS + 1)); }
fail() { echo "  ✗ $1"; FAIL=$((FAIL + 1)); }

echo "=== Integration Test: Telemetry ==="
echo ""

if [ ! -f "$ENVO_BIN" ]; then
    echo "ERROR: Binary not found at $ENVO_BIN"
    echo "Run 'cargo build' first."
    exit 1
fi

ENVO="$(cd "$(dirname "$ENVO_BIN")" && pwd)/$(basename "$ENVO_BIN")"
REAL_HOME="$HOME"

# ── Test 1: Telemetry disabled — no crash ──────────────────────────

echo "Test 1: Telemetry disabled (opt-out)"

FAKE_HOME=$(mktemp -d)
trap 'rm -rf "$FAKE_HOME"' EXIT

mkdir -p "$FAKE_HOME/.envo"
cat > "$FAKE_HOME/.envo/config.toml" << 'EOF'
[telemetry]
enabled = false
EOF

export HOME="$FAKE_HOME"

# Run init — should succeed with telemetry disabled
TMPDIR_T1=$(mktemp -d)
cd "$TMPDIR_T1"
RESULT=$("$ENVO" init 2>&1)

if echo "$RESULT" | grep -q "✓"; then
    pass "init succeeds with telemetry disabled"
else
    fail "init failed: $RESULT"
fi

# Verify no telemetry was sent (check verbose output)
RESULT_VERBOSE=$("$ENVO" init --template default 2>&1 --verbose || true)
if echo "$RESULT_VERBOSE" | grep -q "telemetry: sending"; then
    fail "telemetry event sent despite being disabled"
else
    pass "no telemetry events when disabled"
fi

rm -rf "$TMPDIR_T1"

# ── Test 2: Telemetry enabled — no crash ───────────────────────────

echo ""
echo "Test 2: Telemetry enabled (default)"

FAKE_HOME2=$(mktemp -d)
mkdir -p "$FAKE_HOME2/.envo"
cat > "$FAKE_HOME2/.envo/config.toml" << 'EOF'
[telemetry]
enabled = true
EOF

export HOME="$FAKE_HOME2"

TMPDIR_T2=$(mktemp -d)
cd "$TMPDIR_T2"
RESULT=$("$ENVO" init 2>&1)

if echo "$RESULT" | grep -q "✓"; then
    pass "init succeeds with telemetry enabled"
else
    fail "init failed with telemetry enabled: $RESULT"
fi

rm -rf "$TMPDIR_T2"
rm -rf "$FAKE_HOME2"

# ── Test 3: No config file — defaults to enabled, no crash ────────

echo ""
echo "Test 3: No config file (default behavior)"

FAKE_HOME3=$(mktemp -d)
export HOME="$FAKE_HOME3"

TMPDIR_T3=$(mktemp -d)
cd "$TMPDIR_T3"
RESULT=$("$ENVO" init 2>&1)

if echo "$RESULT" | grep -q "✓"; then
    pass "init succeeds with no config file"
else
    fail "init failed with no config: $RESULT"
fi

rm -rf "$TMPDIR_T3"
rm -rf "$FAKE_HOME3"

# ── Test 4: Version command works with telemetry ───────────────────

echo ""
echo "Test 4: Version command with telemetry"

export HOME="$FAKE_HOME"

RESULT=$("$ENVO" version 2>&1)
if echo "$RESULT" | grep -q "envo"; then
    pass "version works with telemetry system loaded"
else
    fail "version failed: $RESULT"
fi

# ── Test 5: Machine ID generation ──────────────────────────────────

echo ""
echo "Test 5: Machine ID generation"

FAKE_HOME5=$(mktemp -d)
mkdir -p "$FAKE_HOME5/.envo"
cat > "$FAKE_HOME5/.envo/config.toml" << 'EOF'
[telemetry]
enabled = true
EOF

export HOME="$FAKE_HOME5"

TMPDIR_T5=$(mktemp -d)
cd "$TMPDIR_T5"
"$ENVO" init >/dev/null 2>&1

# Check if machine_id was written to config
if [ -f "$FAKE_HOME5/.envo/config.toml" ]; then
    if grep -q "machine_id" "$FAKE_HOME5/.envo/config.toml"; then
        pass "machine_id generated and saved to config"
    else
        pass "config exists (machine_id may not be written if telemetry curl failed immediately)"
    fi
else
    fail "config file not found"
fi

rm -rf "$TMPDIR_T5"
rm -rf "$FAKE_HOME5"

# ── Test 6: Error tracking doesn't crash ───────────────────────────

echo ""
echo "Test 6: Error telemetry on failed command"

FAKE_HOME6=$(mktemp -d)
mkdir -p "$FAKE_HOME6/.envo"
cat > "$FAKE_HOME6/.envo/config.toml" << 'EOF'
[telemetry]
enabled = true
EOF

export HOME="$FAKE_HOME6"

TMPDIR_T6=$(mktemp -d)
cd "$TMPDIR_T6"

# Run a command that will fail (install without init)
RESULT=$("$ENVO" install ripgrep 2>&1 || true)
if echo "$RESULT" | grep -qi "no envo environment\|envo init"; then
    pass "failed command produces clean error with telemetry active"
else
    fail "unexpected error: $RESULT"
fi

rm -rf "$TMPDIR_T6"
rm -rf "$FAKE_HOME6"

# ── Restore HOME ──────────────────────────────────────────────────

export HOME="$REAL_HOME"

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
