#!/usr/bin/env bash
# ==============================================================================
# hyprland-matugen.sh - Update Hyprland/system colors via Matugen from wallrs preview
# ==============================================================================
# Usage:
#   contrib/hyprland-matugen.sh [-o <OUTPUT>] [--snapshot]
#
# Description:
#   Queries wallrs for the active wallpaper's representative image (static image,
#   manifest thumbnail, or live GPU snapshot for procedural shaders/videos),
#   and passes it directly to `matugen image` to generate Material You colors.
# ==============================================================================

set -euo pipefail

# 1. Locate wallctl binary
WALLCTL="${WALLCTL_BIN:-}"
if [[ -z "$WALLCTL" ]]; then
    if command -v wallctl &>/dev/null; then
        WALLCTL="wallctl"
    elif [[ -x "$HOME/.cargo/bin/wallctl" ]]; then
        WALLCTL="$HOME/.cargo/bin/wallctl"
    elif [[ -x "./target/release/wallctl" ]]; then
        WALLCTL="./target/release/wallctl"
    elif [[ -x "./target/debug/wallctl" ]]; then
        WALLCTL="./target/debug/wallctl"
    else
        echo "Error: wallctl binary not found in PATH, ~/.cargo/bin, or ./target/" >&2
        exit 1
    fi
fi

# 2. Check matugen availability
if ! command -v matugen &>/dev/null; then
    echo "Error: 'matugen' executable not found in PATH." >&2
    echo "Install it via cargo (cargo install matugen) or your distro's package manager." >&2
    exit 1
fi

# 3. Retrieve preview image path from wallrs
PREVIEW_IMAGE="$("$WALLCTL" preview "$@")"

if [[ -z "$PREVIEW_IMAGE" || ! -f "$PREVIEW_IMAGE" ]]; then
    echo "Error: wallctl preview returned an invalid or missing image: '$PREVIEW_IMAGE'" >&2
    exit 1
fi

echo "wallrs preview image: $PREVIEW_IMAGE"
echo "Generating Material You theme with matugen..."
matugen image "$PREVIEW_IMAGE"
echo "Theme applied successfully!"

