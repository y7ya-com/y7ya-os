// Draws a window's surface with its corners rounded off.
//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

precision highp float;
#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif
uniform float alpha;
varying vec2 v_coords;
#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

uniform vec2 elem_size;  // this surface, physical px
uniform vec4 geo;        // window geometry relative to the surface, physical px
uniform vec4 radii;      // top-left, top-right, bottom-right, bottom-left, physical px
uniform float flip;      // 1.0 when the buffer is stored upside down

float corner_alpha(vec2 p, vec2 size, vec4 r) {
    vec2 c;
    float radius;
    if (p.x < r.x && p.y < r.x) { radius = r.x; c = vec2(radius, radius); }
    else if (p.x > size.x - r.y && p.y < r.y) { radius = r.y; c = vec2(size.x - radius, radius); }
    else if (p.x > size.x - r.z && p.y > size.y - r.z) { radius = r.z; c = vec2(size.x - radius, size.y - radius); }
    else if (p.x < r.w && p.y > size.y - r.w) { radius = r.w; c = vec2(radius, size.y - radius); }
    else return 1.0;
    return clamp(radius - length(p - c) + 0.5, 0.0, 1.0);
}

void main() {
    vec4 color = texture2D(tex, v_coords);
#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0);
#endif
    vec2 uv = v_coords;
    if (flip > 0.5) uv.y = 1.0 - uv.y;
    vec2 p = uv * elem_size - geo.xy;
    color *= corner_alpha(p, geo.zw, radii);
    gl_FragColor = color * alpha;
#if defined(DEBUG_FLAGS)
    if (tint == 1.0) gl_FragColor = vec4(0.0, 0.2, 0.0, 0.2) + gl_FragColor * 0.8;
#endif
}
