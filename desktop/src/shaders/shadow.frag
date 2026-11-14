// A soft macOS-style window shadow: a tight contact shadow plus a large
// diffuse one, cut out under the window itself.
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
uniform vec4 near;      // contact shadow color (straight alpha)
uniform vec4 far;       // diffuse shadow color (straight alpha)
uniform vec2 sigmas;    // contact, diffuse
uniform float far_dy;   // diffuse shadow drops a little

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
    float step = (end - start) / 4.0;
    float y = start + step * 0.5;
    float value = 0.0;
    for (int i = 0; i < 4; i++) {
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

void main() {
    vec2 p = v_coords * size / scale;
    vec2 lo = rect.xy;
    vec2 hi = rect.xy + rect.zw;

    float a_near = box_shadow(lo, hi, p, sigmas.x, radius) * near.a;
    float a_far = box_shadow(lo + vec2(0.0, far_dy), hi + vec2(0.0, far_dy), p, sigmas.y, radius) * far.a;
    // Both shadows are black-ish; combine coverage.
    float a = 1.0 - (1.0 - a_near) * (1.0 - a_far);
    vec3 c = (near.rgb * a_near + far.rgb * a_far) / max(a_near + a_far, 1e-4);

    // Nothing under the window itself (keeps translucent windows clean).
    a *= 1.0 - inside_rounded(p, lo, hi, radius);

    gl_FragColor = vec4(c * a, a) * alpha;
#if defined(DEBUG_FLAGS)
    if (tint == 1.0) gl_FragColor = vec4(0.0, 0.2, 0.0, 0.2) + gl_FragColor * 0.8;
#endif
}
