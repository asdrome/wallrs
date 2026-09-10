# Wallpaper Authoring Guide

This guide provides the technical specification, directory structure, and authoring guidelines for live wallpaper packages supported by `wallrs`.

## Overview

A `wallrs` wallpaper is a self-contained directory containing:
1. A required configuration manifest named `wallpaper.toml`.
2. Associated media assets (images, shaders, video files, audio tracks) resolved relative to the manifest directory.

```text
my-wallpaper/
├── wallpaper.toml       # Required configuration manifest
├── background.png       # Base image layer (or shader / video file)
├── foreground.png       # Optional parallax or panning layer
└── ambient.ogg          # Optional background audio track
```

## Directory Locations and XDG Standards

`wallrs` adheres to the XDG Base Directory specification:

- **User Wallpapers**: `$XDG_DATA_HOME/wallrs/wallpapers` (fallback: `~/.local/share/wallrs/wallpapers`).
- **System Wallpapers**: `$XDG_DATA_DIRS/wallrs/wallpapers` (e.g., `/usr/share/wallrs/wallpapers`, `/usr/local/share/wallrs/wallpapers`).
- **Templates**: `$XDG_DATA_HOME/wallrs/templates` and `$XDG_DATA_DIRS/wallrs/templates` (or system-installed examples at `/usr/share/wallrs/examples`).

When referencing a wallpaper by name without path separators in CLI commands (e.g., `wallctl set-wallpaper aurora-shader`), `wallctl` searches the local directory first, followed by the user and system XDG wallpaper directories.

## Scaffolding Wallpapers (`wallctl new`)

The `wallctl new` command (aliased as `wallctl init`) initializes a new wallpaper directory from installed templates or repository examples:

```bash
# Initialize a shader wallpaper in the user XDG wallpaper directory (~/.local/share/wallrs/wallpapers)
wallctl new dynamic-aurora --type shader --audio

# Initialize in the current working directory
wallctl new local-scene --type image --local

# Initialize using an existing template
wallctl new custom-plasma --template shadertoy-plasma

# Initialize with custom assets
wallctl new city-night --type image -l bg.png -l fg.png --parallax 0.35

# Initialize video wallpaper
wallctl new lofi-study --type video --video clip.mp4 --volume 30.0
```

### Command Options

| Option                 | Type   | Description                                                                              |
| :--------------------- | :----- | :--------------------------------------------------------------------------------------- |
| `<NAME>`               | String | Wallpaper name or path.                                                                  |
| `-t, --type <TYPE>`    | String | Wallpaper backend type: `shader`, `image`, or `video`.                                   |
| `--template <NAME>`    | String | Template directory name (e.g. `aurora-shader`, `parallax-landscape`, `video-wallpaper`). |
| `-d, --dir <DIR>`      | Path   | Custom parent directory for output.                                                      |
| `--local`              | Flag   | Output in the current working directory instead of XDG wallpaper directory.              |
| `--author <NAME>`      | String | Author name. Automatically extracted from `git config user.name` or `$USER` if omitted.  |
| `--description <DESC>` | String | Wallpaper description.                                                                   |
| `-f, --force`          | Flag   | Overwrite destination directory if it already exists.                                    |
| `--shader <PATH>`      | Path   | Source shader file (`.wgsl` or `.glsl`) to copy into the package.                        |
| `--audio`              | Flag   | Enable PipeWire audio reactivity for shaders.                                            |
| `--glsl`               | Flag   | Generate a Shadertoy-compatible GLSL template instead of WGSL.                           |
| `-l, --layer <PATH>`   | Path   | Image layer file(s) to copy into the package. Can be passed multiple times.              |
| `--parallax <FACTOR>`  | Float  | Parallax motion intensity for the top image layer.                                       |
| `--video <PATH>`       | Path   | Video file to copy into the package.                                                     |
| `--volume <N>`         | Float  | Playback volume for video or audio track (0.0 to 100.0).                                 |
| `--no-loop`            | Flag   | Disable video looping.                                                                   |
| `--audio-track <PATH>` | Path   | Optional ambient audio track to copy into the package.                                   |
| `--audio-volume <N>`   | Float  | Volume for the ambient audio track (0.0 to 100.0).                                       |

## Manifest Specification (`wallpaper.toml`)

### Common Metadata (`[wallpaper]`)

Every manifest must start with the `[wallpaper]` section:

```toml
[wallpaper]
type = "shader"         # Required: "shader", "image", or "video"
name = "aurora-flow"    # Required: Unique identifier or display name
author = "Artist Name"  # Optional: Creator name
description = "A procedural aurora shader" # Optional: Description
thumbnail = "thumb.png" # Optional: Relative path to static preview image
```

### 1. Image Wallpapers (`type = "image"`)

Composes one or more image layers in depth order. The first layer (`index = 0`) serves as the background. Subsequent layers can configure cursor-driven parallax displacement, continuous linear panning, and sinusoidal periodic oscillation (floating/swaying).

```toml
[wallpaper]
type = "image"
name = "cyber-city"

[[image.layers]]
path = "sky.png"

[[image.layers]]
path = "fog.png"
pan = { speed = 0.005, axis = "x" }

[[image.layers]]
path = "drone.png"
parallax = 0.25
oscillation = { speed = 1.2, amplitude = 0.02, axis = "y" }

[[image.layers]]
path = "buildings.png"
parallax = 0.35
```

#### Layer Configuration Options (`[[image.layers]]`)

| Key           | Type          | Description                                                                                                                                     |
| :------------ | :------------ | :---------------------------------------------------------------------------------------------------------------------------------------------- |
| `path`        | String        | Relative path to image file (PNG, JPEG, WebP).                                                                                                  |
| `parallax`    | Float         | Pointer tracking displacement intensity (typically 0.1 to 0.7).                                                                                 |
| `pan`         | Table         | Continuous linear panning: `{ speed = <f32>, axis = "x" \| "y" }`.                                                                              |
| `oscillation` | Table         | Sinusoidal periodic oscillation: `{ speed = <f32>, amplitude = <f32>, axis = "x" \| "y", phase = <f32> }`.                                      |
| `day_night`   | String / Bool | Dynamic lighting mode: `"tint"` (ambient diurnal tint), `"night"` (fades in at night, e.g. stars), `"day"` (day only), or bool (`true` = tint). |
| `tint`        | Array         | Static base RGB multiplier: `[r, g, b]` (e.g. `[0.9, 0.9, 1.0]`).                                                                               |

#### Dynamic Day/Night Lighting Cycle

Layers configured with `day_night` respond automatically to the host's local clock:
- **`"tint"`**: Modulates the layer through smooth ambient color grades (golden amber at dawn/sunset, neutral daylight at noon, midnight indigo after dusk).
- **`"night"`**: Dynamically fades layer opacity based on solar elevation, rendering at 100% opacity at night and 0% during full daylight (ideal for stars, nebulae, city lamps, and night skies).
- **`"day"`**: Renders at full opacity during daylight hours and fades out smoothly at dusk (ideal for sun, daytime skies, birds).

##### Dual-Layer Day/Night Sky Crossfade

For dramatic transitions between a brilliant daylight sky and a pitch-black starry night, you can crossfade two dedicated sky textures:

```toml
# Daytime sky (fades out at dusk)
[[image.layers]]
path = "sky-day.png"
parallax = 0.04
day_night = "day"

# Deep nocturnal sky (fades in at dusk)
[[image.layers]]
path = "sky-night.png"
parallax = 0.04
day_night = "night"
```

##### Custom Schedule & Lighting Curves (`[image.day_night]`)

Wallpaper authors can customize the solar schedule and ambient tint color curve:

```toml
[image.day_night]
dawn_start = 6.0    # Dawn begins (06:00)
day_start = 8.5     # Full daylight reached (08:30)
dusk_start = 18.0   # Sunset / dusk begins (18:00)
night_start = 21.0  # Full night reached (21:00)

# Optional keyframe nodes overriding default diurnal tint curve
[[image.day_night.tint_curve]]
hour = 2.0
tint = [0.15, 0.22, 0.40] # Midnight deep cool blue

[[image.day_night.tint_curve]]
hour = 12.0
tint = [1.00, 1.00, 1.00] # Midday neutral daylight

[[image.day_night.tint_curve]]
hour = 18.5
tint = [1.05, 0.80, 0.58] # Golden hour sunset warmth
```

##### Interactive Runtime & Preview Controls

You can interactively preview or override lighting and time at runtime via `wallctl set-property`:

```bash
# Time simulation (accepts decimal hour, HH:MM string, or "auto"/"reset")
wallctl set-property hour 18.5          # Decimal hour (18:30)
wallctl set-property time_of_day 14:30   # Clock string format
wallctl set-property hour reset         # Revert back to live host clock (or -1)

# Direct daylight factor override [0.0..1.0]
wallctl set-property daylight 1.0       # Force full daytime lighting
wallctl set-property daylight 0.0       # Force full nighttime lighting
wallctl set-property daylight reset     # Revert to automatic solar cycle

# Direct ambient tint override
wallctl set-property ambient_tint "#ffaa66"     # Golden evening hex tint
wallctl set-property ambient_tint "0.2,0.3,0.5" # RGB float values
wallctl set-property ambient_tint reset         # Revert to diurnal tint curve
```

### 2. Shader Wallpapers (`type = "shader"`)

Renders a procedural fragment shader on the GPU using `wgpu` / Vulkan. Supports native WGSL and Shadertoy-compatible GLSL.

```toml
[wallpaper]
type = "shader"
name = "aurora"

[shader]
entry = "aurora.wgsl"
audio = true
uniforms = { custom0 = 1.0, custom1 = 0.5 }
```

#### Shader Configuration Options (`[shader]`)

| Key        | Type    | Description                                                                    |
| :--------- | :------ | :----------------------------------------------------------------------------- |
| `entry`    | String  | Relative path to shader source file (`.wgsl` or `.glsl`).                      |
| `audio`    | Boolean | Connects to PipeWire stream to populate audio FFT uniforms (default: `false`). |
| `uniforms` | Table   | Key-value table of custom float parameters (`String` to `f32`).                |

### 3. Video Wallpapers (`type = "video"`)

Hardware-accelerated video playback in a loop via `libmpv`.

```toml
[wallpaper]
type = "video"
name = "lofi-desk"

[video]
path = "video.mp4"
volume = 35.0
loop = true
```

#### Video Configuration Options (`[video]`)

| Key      | Type    | Default    | Description                                                    |
| :------- | :------ | :--------- | :------------------------------------------------------------- |
| `path`   | String  | (Required) | Relative path to video file (H.264, VP9, AV1 in MP4/WebM/MKV). |
| `volume` | Float   | `50.0`     | Initial audio volume (0.0 to 100.0).                           |
| `loop`   | Boolean | `true`     | Loop playback continuously.                                    |

### 4. Background Audio Track (`[audio]`)

An optional ambient audio track can be attached to any image or shader wallpaper:

```toml
[audio]
path = "ambient.ogg"
volume = 40.0
loop = true
```

Supported codecs include OGG Vorbis, MP3, FLAC, WAV, and Opus.

## GPU Shader Uniforms Specification

For `shader` backends, `wallrsd` binds a uniform buffer of exactly 192 bytes aligned to `std140` at `@group(0) @binding(0)`:

| Byte Offset | WGSL Identifier  | WGSL Type             | GLSL Identifier     | Description                                   |
| :---------- | :--------------- | :-------------------- | :------------------ | :-------------------------------------------- |
| `0..8`      | `resolution`     | `vec2<f32>`           | `iResolution2D`     | Display width and height in pixels            |
| `8..12`     | `time`           | `f32`                 | `iTime`             | Elapsed time in seconds                       |
| `12..16`    | `time_delta`     | `f32`                 | `iTimeDelta`        | Frame time delta in seconds                   |
| `16..32`    | `mouse`          | `vec4<f32>`           | `iMouse`            | Cursor `(x, y, click_x, click_y)` coordinates |
| `32..36`    | `frame`          | `u32`                 | `iFrame`            | Monotonic rendered frame counter              |
| `36..48`    | `custom0, 1, 2`  | `f32, f32, f32`       | `u_custom0, 1, 2`   | Runtime properties controllable via `wallctl` |
| `48..52`    | `audio_bass`     | `f32`                 | `iBass`             | PipeWire bass energy [0.0, 1.0]               |
| `52..56`    | `audio_mid`      | `f32`                 | `iMid`              | PipeWire mid-range energy [0.0, 1.0]          |
| `56..60`    | `audio_treble`   | `f32`                 | `iTreble`           | PipeWire treble energy [0.0, 1.0]             |
| `60..64`    | `audio_volume`   | `f32`                 | `iVolume`           | PipeWire overall RMS volume [0.0, 1.0]        |
| `64..192`   | `audio_spectrum` | `array<vec4<f32>, 8>` | `audio_spectrum[8]` | 32 logarithmic FFT frequency bands            |

### WGSL Struct Definition

```wgsl
struct ShaderUniforms {
    resolution: vec2<f32>,
    time: f32,
    time_delta: f32,
    mouse: vec4<f32>,
    frame: u32,
    custom0: f32,
    custom1: f32,
    custom2: f32,
    audio_bass: f32,
    audio_mid: f32,
    audio_treble: f32,
    audio_volume: f32,
    audio_spectrum: array<vec4<f32>, 8>,
};

@group(0) @binding(0) var<uniform> u_params: ShaderUniforms;
```

### Shadertoy GLSL Compatibility

GLSL fragment shaders declaring `void mainImage(out vec4 fragColor, in vec2 fragCoord)` are automatically translated to WGSL via Naga. Uniforms `iResolution`, `iTime`, `iTimeDelta`, `iFrame`, `iMouse`, `iBass`, `iMid`, `iTreble`, and `iVolume` are provided automatically.

## Validation and Linting (`wallctl validate`)

The validator checks manifest syntax, file existence, image readability, and shader compilation:

```bash
wallctl validate ~/.local/share/wallrs/wallpapers/aurora-live
```

Exit code `0` indicates success. A non-zero exit code and error description are produced on failure.

## Technical Requirements for External Tool Authors

1. **Self-Contained Packages**: Wallpapers must reside entirely within their root folder. Relative paths pointing outside the folder (`../`) are disallowed.
2. **Deterministic Manifest Validation**: External tools should run `wallctl validate <path>` to confirm packages are compliant prior to installation or publishing.
3. **Packaging**: Standard archives (`.tar.gz` or `.zip`) must contain a single top-level directory matching the package name, with `wallpaper.toml` at the root of that directory.
