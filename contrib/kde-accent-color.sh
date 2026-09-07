#!/usr/bin/env bash
# ==============================================================================
# kde-accent-color.sh - Update KDE Plasma 6 Accent Color from wallrs preview
# ==============================================================================
# Usage:
#   contrib/kde-accent-color.sh [-o <OUTPUT>] [--snapshot]
#
# Description:
#   Queries wallrs for the active wallpaper preview image, extracts the dominant
#   vibrant accent color, and applies it to KDE Plasma via `plasma-apply-colorscheme`
#   (or `kwriteconfig6` + KWin reconfigure), avoiding destructive shell refreshes.
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

# 2. Retrieve preview image path from wallrs
PREVIEW_IMAGE="$("$WALLCTL" preview "$@")"

if [[ -z "$PREVIEW_IMAGE" || ! -f "$PREVIEW_IMAGE" ]]; then
    echo "Error: wallctl preview returned an invalid or missing image: '$PREVIEW_IMAGE'" >&2
    exit 1
fi

echo "wallrs preview image: $PREVIEW_IMAGE"

# 3. Extract dominant accent color (HEX & RGB)
COLOR_OUTPUT=""
if command -v python3 &>/dev/null; then
    COLOR_OUTPUT="$(python3 - "$PREVIEW_IMAGE" << 'EOF'
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
        r, g, b = best[1]
        print(f"#{r:02x}{g:02x}{b:02x} {r},{g},{b}")
        sys.exit(0)
except Exception:
    pass
sys.exit(1)
EOF
)" || true
fi

# Fallback with imagemagick if python extraction was empty
if [[ -z "$COLOR_OUTPUT" ]] && command -v magick &>/dev/null; then
    HEX_RAW="$(magick "$PREVIEW_IMAGE" -resize 1x1\! -format "%[hex:p{0,0}]" info: | head -c 6)"
    R=$((16#${HEX_RAW:0:2}))
    G=$((16#${HEX_RAW:2:2}))
    B=$((16#${HEX_RAW:4:2}))
    COLOR_OUTPUT="#$HEX_RAW $R,$G,$B"
fi

if [[ -z "$COLOR_OUTPUT" ]]; then
    echo "Error: Failed to extract dominant color from '$PREVIEW_IMAGE'" >&2
    exit 1
fi

HEX="$(echo "$COLOR_OUTPUT" | awk '{print $1}')"
RGB="$(echo "$COLOR_OUTPUT" | awk '{print $2}')"

echo "Extracted accent color: $HEX (RGB: $RGB)"

# 4. Apply to KDE Plasma
# Explicitly persist AccentColor and LastUsedCustomAccentColor in kdeglobals.
# This ensures that both the System Settings GUI (kcm_colors) and the Plasma
# panel/dock immediately recognize and select the custom accent color.
KWRITECONFIG=""
if command -v kwriteconfig6 &>/dev/null; then
    KWRITECONFIG="kwriteconfig6"
elif command -v kwriteconfig5 &>/dev/null; then
    KWRITECONFIG="kwriteconfig5"
fi

if [[ -n "$KWRITECONFIG" ]]; then
    "$KWRITECONFIG" --file kdeglobals --group General --key AccentColor "$RGB"
    "$KWRITECONFIG" --file kdeglobals --group General --key LastUsedCustomAccentColor "$RGB"
    "$KWRITECONFIG" --file kdeglobals --group General --key accentColorFromWallpaper --type bool false
fi

# Prefer official plasma-apply-colorscheme tool which applies the accent color
# cleanly across Qt/KDE/GTK apps without touching or crashing the desktop layer surface.
if command -v plasma-apply-colorscheme &>/dev/null; then
    SCHEME="$(grep -m1 '^ColorScheme=' "$HOME/.config/kdeglobals" 2>/dev/null | cut -d= -f2 || true)"
    if [[ -z "$SCHEME" ]]; then
        SCHEME="BreezeDark"
    fi
    plasma-apply-colorscheme --accent-color "$HEX" "$SCHEME"
fi

# Reconfigure KWin to sync window decorations and titlebars
if command -v qdbus6 &>/dev/null; then
    qdbus6 org.kde.KWin /KWin org.kde.KWin.reconfigure 2>/dev/null || true
elif command -v qdbus &>/dev/null; then
    qdbus org.kde.KWin /KWin org.kde.KWin.reconfigure 2>/dev/null || true
fi

echo "KDE Plasma accent color successfully updated to $HEX!"
