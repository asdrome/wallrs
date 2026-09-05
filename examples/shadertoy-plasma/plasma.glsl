// Shadertoy-compatible plasma shader
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = (fragCoord - 0.5 * iResolution.xy) / iResolution.y;

    float v = 0.0;
    float t = iTime * 0.5;

    v += sin((uv.x + t));
    v += sin((uv.y + t) / 2.0);
    v += sin((uv.x + uv.y + t) / 2.0);

    vec2 c = uv + vec2(sin(t / 3.0), cos(t / 2.0)) * 0.5;
    v += sin(sqrt(c.x * c.x + c.y * c.y + 1.0) + t);

    v = v / 2.0;
    vec3 col = vec3(sin(v * 3.14159), cos(v * 3.14159), sin(v * 3.14159 + 2.0));
    col = col * 0.5 + 0.5;

    fragColor = vec4(col, 1.0);
}

