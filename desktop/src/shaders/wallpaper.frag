// The default wallpaper: soft overlapping color fields, drawn procedurally so
// it's sharp at any resolution. It only renders when the screen changes.
precision highp float;
varying vec2 v_coords;
uniform vec2 size;
uniform float alpha;
#if defined(DEBUG_FLAGS)
uniform float tint;
#endif
uniform float dark;

vec3 blob(vec2 p, vec2 c, float r, vec3 color, vec3 base) {
    float d = length((p - c) / vec2(1.0, 0.8));
    return mix(base, color, smoothstep(r, 0.0, d) * 0.9);
}

void main() {
    float aspect = size.x / size.y;
    vec2 p = vec2(v_coords.x * aspect, v_coords.y);
    // Gentle waves so the fields don't look like circles.
    p.y += 0.06 * sin(p.x * 3.1 + 0.6) + 0.03 * sin(p.x * 7.3);
    p.x += 0.04 * sin(p.y * 4.7);

    vec3 base, a, b, c, d;
    if (dark > 0.5) {
        base = vec3(0.035, 0.045, 0.11);
        a = vec3(0.10, 0.20, 0.52);
        b = vec3(0.42, 0.16, 0.55);
        c = vec3(0.86, 0.36, 0.42);
        d = vec3(0.05, 0.38, 0.62);
    } else {
        base = vec3(0.70, 0.80, 0.95);
        a = vec3(0.33, 0.55, 0.96);
        b = vec3(0.70, 0.52, 0.95);
        c = vec3(1.00, 0.70, 0.60);
        d = vec3(0.35, 0.82, 0.92);
    }
    vec3 col = base;
    col = blob(p, vec2(0.15 * aspect, 0.95), 0.95, a, col);
    col = blob(p, vec2(0.55 * aspect, 0.55), 0.70, b, col);
    col = blob(p, vec2(0.92 * aspect, 0.25), 0.65, c, col);
    col = blob(p, vec2(0.80 * aspect, 1.05), 0.60, d, col);
    // Very light vignette.
    float v = smoothstep(1.3, 0.3, length(v_coords - 0.5));
    col *= mix(0.82, 1.0, v);
    // A little dithering avoids banding in the smooth gradient.
    float n = fract(sin(dot(gl_FragCoord.xy, vec2(12.9898, 78.233))) * 43758.5453);
    col += (n - 0.5) / 255.0;

    gl_FragColor = vec4(col, 1.0) * alpha;
#if defined(DEBUG_FLAGS)
    if (tint == 1.0) gl_FragColor = vec4(0.0, 0.2, 0.0, 0.2) + gl_FragColor * 0.8;
#endif
}
