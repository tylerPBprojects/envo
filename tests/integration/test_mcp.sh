#!/usr/bin/env bash
# Integration test: envo-mcp server
# Starts the MCP server, sends JSON-RPC messages via stdin, verifies responses.
# Usage: bash tests/integration/test_mcp.sh

set -euo pipefail

MCP_BIN="${MCP_BIN:-./target/debug/envo-mcp}"
PASS=0
FAIL=0

pass() { echo "  ✓ $1"; PASS=$((PASS + 1)); }
fail() { echo "  ✗ $1"; FAIL=$((FAIL + 1)); }

echo "=== Integration Test: envo-mcp ==="
echo ""

if [ ! -f "$MCP_BIN" ]; then
    echo "ERROR: MCP binary not found at $MCP_BIN"
    echo "Run 'cargo build' first."
    exit 1
fi

# ── Helper: format a JSON-RPC message with Content-Length ──────────

send_msg() {
    local body="$1"
    local length=${#body}
    printf "Content-Length: %d\r\n\r\n%s" "$length" "$body"
}

# ── Helper: extract JSON from Content-Length framed response ───────

extract_json() {
    local input="$1"
    # Extract everything after the double CRLF (end of headers)
    echo "$input" | sed 's/^Content-Length: [0-9]*\r\?\n\r\?\n//' | grep -o '{.*}'
}

# ── Test 1: Initialize and tools/list ──────────────────────────────

echo "Test 1: Initialize and tools/list"

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

INIT_MSG='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}'
TOOLS_LIST='{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}'
SHUTDOWN='{"jsonrpc":"2.0","id":99,"method":"shutdown"}'
EXIT_MSG='{"jsonrpc":"2.0","method":"exit"}'

RESPONSE=$(
    {
        send_msg "$INIT_MSG"
        sleep 0.2
        send_msg "$TOOLS_LIST"
        sleep 0.2
        send_msg "$SHUTDOWN"
        sleep 0.1
        send_msg "$EXIT_MSG"
    } | timeout 10 "$MCP_BIN" 2>/dev/null || true
)

if echo "$RESPONSE" | grep -q "envo-mcp"; then
    pass "server returns server info in initialize"
else
    if echo "$RESPONSE" | grep -q "protocolVersion"; then
        pass "server responds to initialize"
    else
        fail "no initialize response"
    fi
fi

if echo "$RESPONSE" | grep -q "envo_init"; then
    pass "tools/list includes envo_init"
else
    fail "tools/list missing envo_init"
fi

if echo "$RESPONSE" | grep -q "envo_install"; then
    pass "tools/list includes envo_install"
else
    fail "tools/list missing envo_install"
fi

if echo "$RESPONSE" | grep -q "envo_search"; then
    pass "tools/list includes envo_search"
else
    fail "tools/list missing envo_search"
fi

if echo "$RESPONSE" | grep -q "envo_env_info"; then
    pass "tools/list includes envo_env_info"
else
    fail "tools/list missing envo_env_info"
fi

if echo "$RESPONSE" | grep -q "envo_activate"; then
    pass "tools/list includes envo_activate"
else
    fail "tools/list missing envo_activate"
fi

if echo "$RESPONSE" | grep -q "envo_uninstall"; then
    pass "tools/list includes envo_uninstall"
else
    fail "tools/list missing envo_uninstall"
fi

# ── Test 2: envo_init via tools/call ───────────────────────────────

echo ""
echo "Test 2: envo_init via tools/call"

INIT_TOOL="{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"envo_init\",\"arguments\":{\"directory\":\"$TMPDIR\"}}}"

RESPONSE2=$(
    {
        send_msg "$INIT_MSG"
        sleep 0.2
        send_msg '{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}'
        sleep 0.1
        send_msg "$INIT_TOOL"
        sleep 0.3
        send_msg "$SHUTDOWN"
        sleep 0.1
        send_msg "$EXIT_MSG"
    } | timeout 10 "$MCP_BIN" 2>/dev/null || true
)

if echo "$RESPONSE2" | grep -q "success"; then
    pass "envo_init succeeded"
else
    fail "envo_init response: $(echo "$RESPONSE2" | tail -c 200)"
fi

if [ -f "$TMPDIR/.envo/manifest.toml" ]; then
    pass "manifest.toml created by envo_init"
else
    fail "manifest.toml not found after envo_init"
fi

# ── Test 3: resources/list and resources/read ──────────────────────

echo ""
echo "Test 3: Resources"

RES_LIST='{"jsonrpc":"2.0","id":4,"method":"resources/list","params":{}}'
RES_READ='{"jsonrpc":"2.0","id":5,"method":"resources/read","params":{"uri":"envo://status"}}'

RESPONSE3=$(
    {
        send_msg "$INIT_MSG"
        sleep 0.2
        send_msg "$RES_LIST"
        sleep 0.2
        send_msg "$RES_READ"
        sleep 0.2
        send_msg "$SHUTDOWN"
        sleep 0.1
        send_msg "$EXIT_MSG"
    } | timeout 10 "$MCP_BIN" 2>/dev/null || true
)

if echo "$RESPONSE3" | grep -q "envo://manifest"; then
    pass "resources/list includes envo://manifest"
else
    fail "resources/list missing envo://manifest"
fi

if echo "$RESPONSE3" | grep -q "envo://status"; then
    pass "resources/list includes envo://status"
else
    fail "resources/list missing envo://status"
fi

if echo "$RESPONSE3" | grep -q "envo_version"; then
    pass "status resource contains version"
elif echo "$RESPONSE3" | grep -q "initialized"; then
    pass "status resource contains initialization info"
else
    fail "status resource response unexpected"
fi

# ── Test 4: envo_env_info on initialized project ──────────────────

echo ""
echo "Test 4: envo_env_info"

ENV_INFO="{\"jsonrpc\":\"2.0\",\"id\":6,\"method\":\"tools/call\",\"params\":{\"name\":\"envo_env_info\",\"arguments\":{\"directory\":\"$TMPDIR\"}}}"

RESPONSE4=$(
    {
        send_msg "$INIT_MSG"
        sleep 0.2
        send_msg "$ENV_INFO"
        sleep 0.3
        send_msg "$SHUTDOWN"
        sleep 0.1
        send_msg "$EXIT_MSG"
    } | timeout 10 "$MCP_BIN" 2>/dev/null || true
)

if echo "$RESPONSE4" | grep -q "project_name"; then
    pass "env_info returns project_name"
else
    if echo "$RESPONSE4" | grep -q "initialized"; then
        pass "env_info returns initialized status"
    else
        fail "env_info response: $(echo "$RESPONSE4" | tail -c 200)"
    fi
fi

# ── Test 5: Unknown method returns error ───────────────────────────

echo ""
echo "Test 5: Unknown method"

UNKNOWN='{"jsonrpc":"2.0","id":7,"method":"bogus/method","params":{}}'

RESPONSE5=$(
    {
        send_msg "$INIT_MSG"
        sleep 0.2
        send_msg "$UNKNOWN"
        sleep 0.2
        send_msg "$SHUTDOWN"
        sleep 0.1
        send_msg "$EXIT_MSG"
    } | timeout 10 "$MCP_BIN" 2>/dev/null || true
)

if echo "$RESPONSE5" | grep -q "unknown method\|-32601"; then
    pass "unknown method returns proper error"
else
    fail "unknown method response: $(echo "$RESPONSE5" | tail -c 200)"
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
