// A rounded rectangle with an optional border: panels, highlights, tiles.
precision highp float;
varying vec2 v_coords;
uniform vec2 size;   // logical
uniform float alpha;
#if defined(DEBUG_FLAGS)
uniform float tint;
#endif
uniform float scale;
uniform float radius;
uniform vec4 fill;     // straight alpha
uniform vec4 border;   // straight alpha
uniform float border_w;

float sd_round_box(vec2 p, vec2 half_size, float r) {
    vec2 q = abs(p) - half_size + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

void main() {
    vec2 p = v_coords * size - size * 0.5;
    float d = sd_round_box(p, size * 0.5, min(radius, min(size.x, size.y) * 0.5));
    float outer = clamp(0.5 - d * scale, 0.0, 1.0);
    float inner = clamp(0.5 - (d + border_w) * scale, 0.0, 1.0);
    vec4 color = vec4(fill.rgb * fill.a, fill.a) * inner + vec4(border.rgb * border.a, border.a) * (outer - inner);
    gl_FragColor = color * alpha;
#if defined(DEBUG_FLAGS)
    if (tint == 1.0) gl_FragColor = vec4(0.0, 0.2, 0.0, 0.2) + gl_FragColor * 0.8;
#endif
}
