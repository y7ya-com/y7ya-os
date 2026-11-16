// A macOS-style window shadow in three layers: a hairline edge, a tight
// shadow and a large soft drop shadow, cut out under the window itself.
//
// The blurred rounded box uses Evan Wallace's approximation:
// https://madebyevan.com/shaders/fast-rounded-rectangle-shadows/ (CC0)
precision highp float;
varying vec2 v_coords;
uniform vec2 size;
uniform float alpha;
#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

uniform float scale;
uniform vec4 rect;      // the window frame, logical px relative to this element
uniform float radius;
// Per layer: x = blur sigma, y = vertical offset, z = opacity.
uniform vec3 edge;
uniform vec3 mid;
uniform vec3 far;

float gaussian(float x, float sigma) {
    return exp(-(x * x) / (2.0 * sigma * sigma)) / (2.5066283 * sigma);
}

vec2 erf2(vec2 x) {
    vec2 s = sign(x), a = abs(x);
    x = 1.0 + (0.278393 + (0.230389 + 0.078108 * (a * a)) * a) * a;
    x *= x;
    return s - s / (x * x);
}

float shadow_x(float x, float y, float sigma, float corner, vec2 half_size) {
    float delta = min(half_size.y - corner - abs(y), 0.0);
    float curved = half_size.x - corner + sqrt(max(0.0, corner * corner - delta * delta));
    vec2 integral = 0.5 + 0.5 * erf2((x + vec2(-curved, curved)) * (0.7071068 / sigma));
    return integral.y - integral.x;
}

float box_shadow(vec2 lower, vec2 upper, vec2 p, float sigma, float corner) {
    vec2 center = (lower + upper) * 0.5;
    vec2 half_size = (upper - lower) * 0.5;
    p -= center;
    float low = p.y - half_size.y;
    float high = p.y + half_size.y;
    float start = clamp(-3.0 * sigma, low, high);
    float end = clamp(3.0 * sigma, low, high);
    float step = (end - start) / 8.0;
    float y = start + step * 0.5;
    float value = 0.0;
    for (int i = 0; i < 8; i++) {
        value += shadow_x(p.x, p.y - y, sigma, corner, half_size) * gaussian(y, sigma) * step;
        y += step;
    }
    return value;
}

float inside_rounded(vec2 p, vec2 lo, vec2 hi, float r) {
    vec2 q = abs(p - (lo + hi) * 0.5) - (hi - lo) * 0.5 + r;
    float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
    return clamp(0.5 - d * scale, 0.0, 1.0);
}

float layer(vec2 p, vec2 lo, vec2 hi, vec3 l) {
    vec2 dy = vec2(0.0, l.y);
    float sigma = max(l.x, 0.3);
    // A layer is nothing beyond 3 sigma of its box; the tight ones only
    // cover a thin band, so most pixels skip them.
    vec2 gap = max(lo + dy - p, p - hi - dy);
    if (max(gap.x, gap.y) > 3.0 * sigma) return 0.0;
    return box_shadow(lo + dy, hi + dy, p, sigma, radius) * l.z;
}

void main() {
    // `size` is the element's logical size; `scale` only sharpens edges.
    vec2 p = v_coords * size;
    vec2 lo = rect.xy;
    vec2 hi = rect.xy + rect.zw;

    // Nothing under the window itself (keeps translucent windows clean). Most
    // of the element is window, so skip the costly blur there...
    float outside = 1.0 - inside_rounded(p, lo, hi, radius);
    if (outside <= 0.0) {
        gl_FragColor = vec4(0.0);
        return;
    }
    // ...and beyond the reach of the widest layer (3 sigma past its offset).
    float reach = 3.0 * max(max(edge.x, mid.x), far.x) + max(max(abs(edge.y), abs(mid.y)), abs(far.y));
    vec2 gap = max(lo - p, p - hi);
    if (max(gap.x, gap.y) > reach) {
        gl_FragColor = vec4(0.0);
        return;
    }

    // Layers stack like separate shadows would.
    float a = 1.0 - (1.0 - layer(p, lo, hi, edge)) * (1.0 - layer(p, lo, hi, mid)) * (1.0 - layer(p, lo, hi, far));
    a *= outside;

    gl_FragColor = vec4(0.0, 0.0, 0.0, a) * alpha;
#if defined(DEBUG_FLAGS)
    if (tint == 1.0) gl_FragColor = vec4(0.0, 0.2, 0.0, 0.2) + gl_FragColor * 0.8;
#endif
}
