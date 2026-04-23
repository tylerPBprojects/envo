#!/bin/sh
# envo installer
# Usage: curl -sSf https://envo.dev/install.sh | sh
#
# This script downloads the envo binary for your platform and installs
# it to ~/.envo/bin/envo. It does NOT require root or sudo.

set -e

# ── Configuration ──────────────────────────────────────────────────

VERSION="0.1.0"
GITHUB_OWNER="envo-dev"
GITHUB_REPO="envo"
INSTALL_DIR="$HOME/.envo/bin"
CONFIG_DIR="$HOME/.envo"
BINARY_NAME="envo"

# ── Platform detection ─────────────────────────────────────────────

detect_platform() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Linux)  PLATFORM_OS="linux" ;;
        Darwin) PLATFORM_OS="darwin" ;;
        *)
            echo "✗ Unsupported operating system: $OS" >&2
            echo "  envo supports Linux and macOS." >&2
            exit 1
            ;;
    esac

    case "$ARCH" in
        x86_64)  PLATFORM_ARCH="x86_64" ;;
        aarch64) PLATFORM_ARCH="aarch64" ;;
        arm64)   PLATFORM_ARCH="aarch64" ;;  # macOS reports arm64
        *)
            echo "✗ Unsupported architecture: $ARCH" >&2
            echo "  envo supports x86_64 and aarch64 (arm64)." >&2
            exit 1
            ;;
    esac

    PLATFORM="${PLATFORM_OS}-${PLATFORM_ARCH}"
    BINARY_FILENAME="envo-${VERSION}-${PLATFORM}"
    DOWNLOAD_URL="https://github.com/${GITHUB_OWNER}/${GITHUB_REPO}/releases/download/v${VERSION}/${BINARY_FILENAME}"
}

# ── Dependency checks ──────────────────────────────────────────────

check_curl() {
    if ! command -v curl >/dev/null 2>&1; then
        echo "✗ curl is required but not installed." >&2
        echo "  Install curl and try again." >&2
        exit 1
    fi
}

# ── Download and install ───────────────────────────────────────────

download_binary() {
    echo "ℹ Downloading envo ${VERSION} for ${PLATFORM}..."

    # Create install directory
    mkdir -p "$INSTALL_DIR"

    # Download binary
    HTTP_CODE=$(curl -sSfL -w "%{http_code}" -o "${INSTALL_DIR}/${BINARY_NAME}.new" "$DOWNLOAD_URL" 2>/dev/null || echo "000")

    if [ "$HTTP_CODE" != "200" ] && [ ! -f "${INSTALL_DIR}/${BINARY_NAME}.new" ]; then
        echo "✗ Download failed (HTTP $HTTP_CODE)." >&2
        echo "  URL: $DOWNLOAD_URL" >&2
        echo "  This may mean version ${VERSION} has not been released yet." >&2
        rm -f "${INSTALL_DIR}/${BINARY_NAME}.new"
        exit 1
    fi

    # Make executable
    chmod +x "${INSTALL_DIR}/${BINARY_NAME}.new"

    # Atomic replace — rename is atomic on the same filesystem
    mv "${INSTALL_DIR}/${BINARY_NAME}.new" "${INSTALL_DIR}/${BINARY_NAME}"
}

# ── PATH configuration ────────────────────────────────────────────

configure_path() {
    PATH_LINE="export PATH=\"\$HOME/.envo/bin:\$PATH\" # envo PATH"
    FISH_PATH_LINE="set -gx PATH \$HOME/.envo/bin \$PATH # envo PATH"

    # Bash
    if [ -f "$HOME/.bashrc" ] || [ "$(basename "$SHELL" 2>/dev/null)" = "bash" ]; then
        BASHRC="$HOME/.bashrc"
        if [ ! -f "$BASHRC" ]; then
            touch "$BASHRC"
        fi
        if ! grep -q "# envo PATH" "$BASHRC" 2>/dev/null; then
            printf '\n%s\n' "$PATH_LINE" >> "$BASHRC"
        fi
    fi

    # Zsh
    if [ -f "$HOME/.zshrc" ] || [ "$(basename "$SHELL" 2>/dev/null)" = "zsh" ]; then
        ZSHRC="$HOME/.zshrc"
        if [ ! -f "$ZSHRC" ]; then
            touch "$ZSHRC"
        fi
        if ! grep -q "# envo PATH" "$ZSHRC" 2>/dev/null; then
            printf '\n%s\n' "$PATH_LINE" >> "$ZSHRC"
        fi
    fi

    # Fish
    FISH_CONFIG="$HOME/.config/fish/config.fish"
    if [ -d "$HOME/.config/fish" ]; then
        if [ ! -f "$FISH_CONFIG" ]; then
            touch "$FISH_CONFIG"
        fi
        if ! grep -q "# envo PATH" "$FISH_CONFIG" 2>/dev/null; then
            printf '\n%s\n' "$FISH_PATH_LINE" >> "$FISH_CONFIG"
        fi
    fi
}

# ── Config file ────────────────────────────────────────────────────

create_config() {
    CONFIG_FILE="$CONFIG_DIR/config.toml"
    if [ ! -f "$CONFIG_FILE" ]; then
        cat > "$CONFIG_FILE" << 'CONFIGEOF'
# envo configuration

# Telemetry is not yet implemented — this is a placeholder for future use.
# [telemetry]
# enabled = true
CONFIGEOF
    fi
}

# ── Nix check ──────────────────────────────────────────────────────

check_nix() {
    if command -v nix >/dev/null 2>&1; then
        NIX_VERSION="$(nix --version 2>/dev/null || echo "unknown")"
        echo "ℹ Nix detected: $NIX_VERSION"
    else
        echo ""
        echo "ℹ Nix is not installed."
        echo "  envo uses Nix to manage packages. Install it with:"
        echo "    curl --proto '=https' --tlsv1.2 -sSf -L https://install.determinate.systems/nix | sh -s -- install"
        echo ""
        echo "  You can install Nix later — envo will prompt you when needed."
    fi
}

# ── Main ───────────────────────────────────────────────────────────

main() {
    echo ""
    echo "envo installer v${VERSION}"
    echo ""

    check_curl
    detect_platform
    download_binary
    configure_path
    create_config
    check_nix

    echo ""
    echo "✓ envo installed to ${INSTALL_DIR}/${BINARY_NAME}"
    echo ""
    echo "Restart your shell or run:"
    echo "  export PATH=\"\$HOME/.envo/bin:\$PATH\""
    echo ""
    echo "Then get started:"
    echo "  envo init"
    echo "  envo install ripgrep"
    echo "  source <(envo activate --inline)"
    echo ""
}

main
