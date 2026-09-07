#!/usr/bin/env bash
# ==============================================================================
# kde-accent-color.sh - Update KDE Plasma 6 Accent Color from wallrs preview
# ==============================================================================
# Usage:
#   contrib/kde-accent-color.sh [-o <OUTPUT>] [--snapshot]
#
# Description:
#   Queries wallrs for the active wallpaper preview image, extracts the dominant
#   vibrant accent color, and applies it to KDE Plasma via `kwriteconfig6` and D-Bus.
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

# 2. Check for KDE configuration tools
KWRITECONFIG=""
if command -v kwriteconfig6 &>/dev/null; then
    KWRITECONFIG="kwriteconfig6"
elif command -v kwriteconfig5 &>/dev/null; then
    KWRITECONFIG="kwriteconfig5"
else
    echo "Error: 'kwriteconfig6' or 'kwriteconfig5' not found. Is KDE Plasma installed?" >&2
    exit 1
fi

# 3. Retrieve preview image path from wallrs
PREVIEW_IMAGE="$("$WALLCTL" preview "$@")"

if [[ -z "$PREVIEW_IMAGE" || ! -f "$PREVIEW_IMAGE" ]]; then
    echo "Error: wallctl preview returned an invalid or missing image: '$PREVIEW_IMAGE'" >&2
    exit 1
fi

echo "wallrs preview image: $PREVIEW_IMAGE"

# 4. Extract dominant accent color (RGB)
RGB=""
if command -v python3 &>/dev/null; then
    RGB="$(python3 - "$PREVIEW_IMAGE" << 'EOF'
import sys
try:
    from PIL import Image
    im = Image.open(sys.argv[1]).convert('RGB')
    im.thumbnail((100, 100))
    # Quantize to 8 colors to find dominant clusters
    q = im.quantize(colors=8, method=Image.Quantize.MEDIANCUT).convert('RGB')
    colors = q.getcolors(maxcolors=1000)
    if colors:
        # Sort by vibrancy/saturation and pixel count (avoid pure black or near white)
        def score(item):
            count, (r, g, b) = item
            max_c, min_c = max(r, g, b), min(r, g, b)
            sat = (max_c - min_c) / (max_c + 1e-5)
            brightness = (r + g + b) / 3.0
            # penalize extremely dark or washed-out white
            penalty = 0.1 if (brightness < 30 or brightness > 235) else 1.0
            return (sat * 2.0 + 0.5) * count * penalty
        best = max(colors, key=score)
        print(f"{best[1][0]},{best[1][1]},{best[1][2]}")
        sys.exit(0)
except Exception:
    pass
sys.exit(1)
EOF
)" || true
fi

# Fallback with imagemagick if python extraction was empty
if [[ -z "$RGB" ]] && command -v magick &>/dev/null; then
    HEX="$(magick "$PREVIEW_IMAGE" -resize 1x1\! -format "%[hex:p{0,0}]" info: | head -c 6)"
    R=$((16#${HEX:0:2}))
    G=$((16#${HEX:2:2}))
    B=$((16#${HEX:4:2}))
    RGB="$R,$G,$B"
fi

if [[ -z "$RGB" ]]; then
    echo "Error: Failed to extract dominant color from '$PREVIEW_IMAGE'" >&2
    exit 1
fi

echo "Extracted accent color: RGB($RGB)"

# 5. Apply to KDE Plasma kdeglobals
"$KWRITECONFIG" --file kdeglobals --group General --key AccentColor "$RGB"

# 6. Notify KWin and Plasma to reload colors
if command -v qdbus6 &>/dev/null; then
    qdbus6 org.kde.KWin /KWin org.kde.KWin.reconfigure 2>/dev/null || true
    qdbus6 org.kde.plasmashell /PlasmaShell org.kde.PlasmaShell.refreshCurrentShell 2>/dev/null || true
elif command -v qdbus &>/dev/null; then
    qdbus org.kde.KWin /KWin org.kde.KWin.reconfigure 2>/dev/null || true
    qdbus org.kde.plasmashell /PlasmaShell org.kde.PlasmaShell.refreshCurrentShell 2>/dev/null || true
fi

echo "KDE Plasma accent color successfully updated to $RGB!"
