// A child of the canvas drawn where its marker puts it. Its pixels are
// premultiplied, as the host paints them.

@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var picture_sampler: sampler;

struct Varyings {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
}

@vertex
fn vs(@location(0) position: vec2f, @location(1) uv: vec2f) -> Varyings {
    var out: Varyings;
    out.position = vec4f(position, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs(in: Varyings) -> @location(0) vec4f {
    return textureSample(picture, picture_sampler, in.uv);
}

// For an sRGB target, which encodes what it is given: the pixels are decoded
// first, out of their premultiplication and back into it.
@fragment
fn fs_linear(in: Varyings) -> @location(0) vec4f {
    let color = textureSample(picture, picture_sampler, in.uv);
    if color.a <= 0.0 {
        return vec4f(0.0);
    }
    let encoded = color.rgb / color.a;
    let low = encoded / 12.92;
    let high = pow((encoded + 0.055) / 1.055, vec3f(2.4));
    let linear = select(high, low, encoded <= vec3f(0.04045));
    return vec4f(linear * color.a, color.a);
}
