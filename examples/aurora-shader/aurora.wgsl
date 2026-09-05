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
};

@group(0) @binding(0)
var<uniform> u_params: ShaderUniforms;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let t = u_params.time * 0.4;

    // Deep night sky gradient background
    var col = mix(
        vec3<f32>(0.02, 0.03, 0.08),
        vec3<f32>(0.05, 0.08, 0.18),
        uv.y
    );

    // Multi-layered undulating ribbons
    for (var i = 1.0; i <= 3.0; i += 1.0) {
        let wave = sin(uv.x * (2.5 * i) + t * (0.8 * i)) * 0.18
                 + cos(uv.x * (1.2 * i) - t * 0.5) * 0.12;
        let dist = abs(uv.y - (0.45 + wave));
        let glow = exp(-dist * (12.0 + i * 4.0));

        let aurora_color = vec3<f32>(
            0.1 * i + 0.1 * sin(t + i),
            0.85 - 0.15 * i,
            0.55 + 0.3 * cos(t * 0.7 + i)
        );
        col += aurora_color * glow * 0.7;
    }

    return vec4<f32>(clamp(col, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}

