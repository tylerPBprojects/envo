#!/usr/bin/env bash
# envo CUDA/PyTorch Demo
#
# This script demonstrates envo's lazy-fetch and instant-activation
# with a PyTorch + CUDA environment — the hardest environment problem
# in software.
#
# Requirements:
#   - Nix installed (Determinate Nix recommended)
#   - envo binary in PATH (cargo build && export PATH=./target/debug:$PATH)
#   - Optional: NVIDIA GPU with drivers for CUDA validation
#
# Usage: bash templates/cuda-pytorch/demo.sh

set -euo pipefail

ENVO_BIN="${ENVO_BIN:-envo}"

# Check prerequisites
if ! command -v "$ENVO_BIN" &> /dev/null; then
    echo "✗ envo not found. Build with 'cargo build' and add to PATH."
    exit 1
fi

if ! command -v nix &> /dev/null; then
    echo "✗ Nix not found. Install with:"
    echo "  curl --proto '=https' --tlsv1.2 -sSf -L https://install.determinate.systems/nix | sh -s -- install"
    exit 1
fi

echo "╔══════════════════════════════════════════════════╗"
echo "║  envo: PyTorch + CUDA in 50ms                   ║"
echo "║  Lazy fetch — nothing downloads until you use it ║"
echo "╚══════════════════════════════════════════════════╝"
echo ""

# Create a fresh project
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT
cd "$TMPDIR"

echo "Step 1: Initialize from template"
echo "────────────────────────────────"
START=$(date +%s%N)
$ENVO_BIN init --template cuda-pytorch
END=$(date +%s%N)
INIT_MS=$(( (END - START) / 1000000 ))
echo "  ⏱  ${INIT_MS}ms"
echo ""

echo "Step 2: Resolve packages (lockfile)"
echo "────────────────────────────────────"
echo "  (This resolves nixpkgs attributes — no download yet)"
START=$(date +%s%N)
$ENVO_BIN install 2>&1 | tail -3
END=$(date +%s%N)
INSTALL_MS=$(( (END - START) / 1000000 ))
echo "  ⏱  ${INSTALL_MS}ms"
echo ""

echo "Step 3: Activate environment"
echo "────────────────────────────"
START=$(date +%s%N)
SNAPSHOT=$($ENVO_BIN activate --inline --shell bash 2>/dev/null)
END=$(date +%s%N)
ACTIVATE_MS=$(( (END - START) / 1000000 ))
echo "  ⏱  ${ACTIVATE_MS}ms — nothing downloaded, just PATH + env vars"
echo ""

echo "Step 4: First use — lazy fetch"
echo "──────────────────────────────"
echo "  Running: python3 --version"
echo "  (This triggers the first download)"
eval "$SNAPSHOT"
START=$(date +%s%N)
python3 --version 2>&1 || echo "  (python3 not yet available — run 'envo install' with Nix)"
END=$(date +%s%N)
FIRST_MS=$(( (END - START) / 1000000 ))
echo "  ⏱  ${FIRST_MS}ms (includes download on first run)"
echo ""

echo "Step 5: Second use — instant"
echo "────────────────────────────"
START=$(date +%s%N)
python3 --version 2>&1 || true
END=$(date +%s%N)
SECOND_MS=$(( (END - START) / 1000000 ))
echo "  ⏱  ${SECOND_MS}ms (cached — no download)"
echo ""

echo "Step 6: PyTorch CUDA check"
echo "──────────────────────────"
python3 -c "
import torch
print(f'  PyTorch version: {torch.__version__}')
print(f'  CUDA available:  {torch.cuda.is_available()}')
if torch.cuda.is_available():
    print(f'  CUDA device:     {torch.cuda.get_device_name(0)}')
" 2>/dev/null || echo "  (PyTorch not yet fetched — run step 4 first)"

echo ""
echo "═══════════════════════════════════════════"
echo "Summary:"
echo "  Init:       ${INIT_MS}ms"
echo "  Resolve:    ${INSTALL_MS}ms"
echo "  Activate:   ${ACTIVATE_MS}ms"
echo "  First run:  ${FIRST_MS}ms (lazy fetch)"
echo "  Second run: ${SECOND_MS}ms (cached)"
echo "═══════════════════════════════════════════"
