#!/bin/sh
# envo uninstaller
# Removes the envo binary and PATH configuration.
# Does NOT remove .envo/ project directories — those belong to your projects.

set -e

INSTALL_DIR="$HOME/.envo/bin"
CONFIG_DIR="$HOME/.envo"
BINARY_NAME="envo"

echo ""
echo "envo uninstaller"
echo ""

# Remove binary
if [ -f "${INSTALL_DIR}/${BINARY_NAME}" ]; then
    rm -f "${INSTALL_DIR}/${BINARY_NAME}"
    echo "✓ Removed ${INSTALL_DIR}/${BINARY_NAME}"
else
    echo "ℹ Binary not found at ${INSTALL_DIR}/${BINARY_NAME}"
fi

# Remove temp binary if it exists
rm -f "${INSTALL_DIR}/${BINARY_NAME}.new"

# Remove config file
if [ -f "${CONFIG_DIR}/config.toml" ]; then
    rm -f "${CONFIG_DIR}/config.toml"
    echo "✓ Removed ${CONFIG_DIR}/config.toml"
fi

# Remove bin directory if empty
if [ -d "$INSTALL_DIR" ]; then
    rmdir "$INSTALL_DIR" 2>/dev/null && echo "✓ Removed ${INSTALL_DIR}" || true
fi

# Remove PATH entries from shell configs
remove_path_entry() {
    FILE="$1"
    if [ -f "$FILE" ] && grep -q "# envo PATH" "$FILE" 2>/dev/null; then
        # Create a temp file without the envo PATH line
        grep -v "# envo PATH" "$FILE" > "${FILE}.tmp"
        mv "${FILE}.tmp" "$FILE"
        echo "✓ Removed PATH entry from $FILE"
    fi
}

remove_path_entry "$HOME/.bashrc"
remove_path_entry "$HOME/.zshrc"
remove_path_entry "$HOME/.config/fish/config.fish"

echo ""
echo "✓ envo has been uninstalled."
echo ""
echo "Note: .envo/ directories inside your projects were NOT removed."
echo "These contain your environment configurations and can be safely"
echo "deleted manually if no longer needed."
echo ""
