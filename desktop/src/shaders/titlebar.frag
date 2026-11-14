// The title bar of a system-decorated window: background with rounded top
// corners, a hairline under it, and the traffic lights.
precision highp float;
varying vec2 v_coords;
uniform vec2 size;   // element size in physical pixels
uniform float alpha;
#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

uniform float scale;    // physical pixels per logical pixel
uniform float radius;   // logical
uniform vec4 bg;        // straight alpha
uniform vec4 hairline;
uniform float focused;  // 1.0 when the window is active
uniform float hover;    // 1.0 when the pointer is over the lights
uniform float pressed;  // 0 = none, 1..3 = close, minimize, zoom
uniform float dark;
uniform vec3 lights;    // x of the first center, gap between centers, radius (logical)

float coverage(float d) {
    // d: signed distance in logical px, negative inside.
    return clamp(0.5 - d * scale, 0.0, 1.0);
}

float sd_segment(vec2 p, vec2 a, vec2 b) {
    vec2 pa = p - a, ba = b - a;
    float h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

vec4 over(vec4 dst, vec3 color, float a) {
    // Premultiplied "source over destination".
    return vec4(color * a, a) + dst * (1.0 - a);
}

void main() {
    vec2 logical = size / scale;
    vec2 p = v_coords * logical;

    // Rounded top corners.
    float shape = 1.0;
    if (p.y < radius && (p.x < radius || p.x > logical.x - radius)) {
        vec2 c = vec2(p.x < radius ? radius : logical.x - radius, radius);
        shape = coverage(length(p - c) - radius);
    }

    vec4 color = vec4(bg.rgb * bg.a, bg.a);
    // Hairline separating the bar from the content.
    if (p.y > logical.y - 1.0 / scale) {
        color = over(color, hairline.rgb, hairline.a);
    }

    for (int i = 0; i < 3; i++) {
        vec2 c = vec2(lights.x + float(i) * lights.y, logical.y * 0.5);
        float r = lights.z;
        float d = length(p - c);
        if (d > r + 1.0) continue;

        vec3 fill;
        vec3 rim;
        if (focused > 0.5 || hover > 0.5) {
            if (i == 0) { fill = vec3(1.0, 0.373, 0.341); rim = vec3(0.878, 0.267, 0.243); }
            else if (i == 1) { fill = vec3(0.996, 0.737, 0.180); rim = vec3(0.871, 0.604, 0.098); }
            else { fill = vec3(0.157, 0.784, 0.251); rim = vec3(0.106, 0.639, 0.192); }
        } else {
            fill = dark > 0.5 ? vec3(0.302) : vec3(0.808);
            rim = dark > 0.5 ? vec3(0.255) : vec3(0.745);
        }
        if (pressed > 0.5 && abs(pressed - float(i + 1)) < 0.5) {
            fill *= 0.78;
            rim *= 0.78;
        }
        color = over(color, rim, coverage(d - r));
        color = over(color, fill, coverage(d - (r - 0.5)));

        if (hover > 0.5) {
            vec2 q = p - c;
            vec3 ink = i == 0 ? vec3(0.302, 0.0, 0.0) : (i == 1 ? vec3(0.392, 0.235, 0.0) : vec3(0.0, 0.302, 0.0));
            float g = 1e3;
            float k = r * 0.42;
            if (i == 0) {
                g = min(sd_segment(q, vec2(-k), vec2(k)), sd_segment(q, vec2(-k, k), vec2(k, -k))) - 0.55;
            } else if (i == 1) {
                g = sd_segment(q, vec2(-k * 1.25, 0.0), vec2(k * 1.25, 0.0)) - 0.6;
            } else {
                // Two triangles pointing apart: enter full screen.
                float s = r * 0.62;
                vec2 a = q + vec2(s * 0.18);
                float t1 = max(max(-(a.x + s * 0.5), -(a.y + s * 0.5)), (a.x + a.y) * 0.7071);
                vec2 b = -q + vec2(s * 0.18);
                float t2 = max(max(-(b.x + s * 0.5), -(b.y + s * 0.5)), (b.x + b.y) * 0.7071);
                g = min(t1, t2);
            }
            color = over(color, ink, coverage(g) * 0.85);
        }
    }

    gl_FragColor = color * shape * alpha;
#if defined(DEBUG_FLAGS)
    if (tint == 1.0) gl_FragColor = vec4(0.0, 0.2, 0.0, 0.2) + gl_FragColor * 0.8;
#endif
}
