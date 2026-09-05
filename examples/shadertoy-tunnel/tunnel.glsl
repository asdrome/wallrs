// Classic procedural tunnel shader
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 p = (-iResolution.xy + 2.0 * fragCoord) / iResolution.y;

    float a = atan(p.y, p.x);
    float r = length(p);

    vec2 uv = vec2(0.3 / r + iTime * 0.4, a / 3.14159265);

    // Checkerboard / rings procedural pattern
    vec2 grid = floor(uv * 8.0);
    float c = mod(grid.x + grid.y, 2.0);

    vec3 col = mix(vec3(0.05, 0.1, 0.2), vec3(0.9, 0.4, 0.2), c);
    col *= r;

    fragColor = vec4(col, 1.0);
}

