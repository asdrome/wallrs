struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

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

@group(0) @binding(0)
var<uniform> u_params: ShaderUniforms;

fn get_band(idx: u32) -> f32 {
    let clamped = min(idx, 31u);
    let v_idx = clamped / 4u;
    let c_idx = clamped % 4u;
    return u_params.audio_spectrum[v_idx][c_idx];
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let t = u_params.time;

    // Dark sleek gradient background with radial bass glow
    let center_dist = length(uv - vec2<f32>(0.5, 0.4));
    let bass_glow = u_params.audio_bass * exp(-center_dist * 3.0) * 0.4;
    var bg = mix(
        vec3<f32>(0.03, 0.04, 0.07),
        vec3<f32>(0.07, 0.08, 0.14),
        uv.y
    ) + vec3<f32>(0.25, 0.05, 0.35) * bass_glow;

    // Visualizer region setup (32 bars across center 80% of width)
    let margin = 0.10;
    let vis_width = 1.0 - 2.0 * margin;
    let baseline_y = 0.25;

    var bar_col = vec3<f32>(0.0);
    var in_bar = false;

    if (uv.x >= margin && uv.x <= (1.0 - margin)) {
        let local_x = (uv.x - margin) / vis_width;
        let bar_float = local_x * 32.0;
        let bar_idx = clamp(u32(floor(bar_float)), 0u, 31u);
        let bar_frac = fract(bar_float);

        // 75% bar fill, 25% gap between bars
        if (bar_frac >= 0.125 && bar_frac <= 0.875) {
            let spec_val = get_band(bar_idx);
            // Ambient idle breathing wave when silent
            let idle = 0.03 + 0.02 * sin(t * 2.0 + f32(bar_idx) * 0.25);
            let bar_height = clamp(idle + spec_val * 0.65, 0.03, 0.70);

            // Color gradient across the 32 frequency bands (Magenta -> Cyan -> Gold)
            let band_norm = f32(bar_idx) / 31.0;
            var tint = vec3<f32>(0.0);
            if (band_norm < 0.5) {
                let k = band_norm * 2.0;
                // Bass to Mid: Electric Magenta to Vivid Cyan
                tint = mix(vec3<f32>(0.95, 0.12, 0.65), vec3<f32>(0.05, 0.80, 0.95), k);
            } else {
                let k = (band_norm - 0.5) * 2.0;
                // Mid to Treble: Vivid Cyan to Radiant Gold
                tint = mix(vec3<f32>(0.05, 0.80, 0.95), vec3<f32>(1.00, 0.85, 0.20), k);
            }

            // Main vertical bar upward from baseline
            if (uv.y >= baseline_y && uv.y <= (baseline_y + bar_height)) {
                let vert_progress = (uv.y - baseline_y) / bar_height;
                // Brighter cap at the tip of each bar
                let cap = smoothstep(0.85, 1.0, vert_progress);
                bar_col = tint * (0.8 + 0.5 * vert_progress) + vec3<f32>(cap * 0.7);
                in_bar = true;
            }
            // Glassy reflection below baseline
            else if (uv.y < baseline_y && uv.y >= (baseline_y - bar_height * 0.45)) {
                let ref_dist = (baseline_y - uv.y) / (bar_height * 0.45);
                let ref_fade = (1.0 - ref_dist) * 0.35;
                bar_col = tint * ref_fade;
                in_bar = true;
            }
        }
    }

    // Baseline separator line
    let base_line_dist = abs(uv.y - baseline_y);
    let base_line = exp(-base_line_dist * 200.0) * 0.5;

    let final_col = select(bg + vec3<f32>(base_line * 0.5), bar_col, in_bar);
    return vec4<f32>(clamp(final_col, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}

