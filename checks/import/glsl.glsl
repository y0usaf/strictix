const float gamma = 2.2;

float luma(vec3 c) {
    return dot(c, vec3(0.299, 0.587, 0.114));
}

vec4 shade(vec4 t) {
    float l = pow(luma(t.rgb), 1.0 / gamma);
    if (l > 0.5) return vec4(t.rgb * 0.9, t.a);
    return t;
}
