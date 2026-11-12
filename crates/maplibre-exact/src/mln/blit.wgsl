// The map's frame drawn over the whole canvas.

@group(0) @binding(0) var frame_texture: texture_2d<f32>;
@group(0) @binding(1) var frame_sampler: sampler;

struct Varyings {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
}

// One triangle that covers the target, with uv (0, 0) at its top-left.
@vertex
fn vs(@builtin(vertex_index) index: u32) -> Varyings {
    let uv = vec2f(f32((index << 1u) & 2u), f32(index & 2u));
    var out: Varyings;
    out.position = vec4f(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs(in: Varyings) -> @location(0) vec4f {
    return textureSample(frame_texture, frame_sampler, in.uv);
}

// For an sRGB target, which encodes what it is given: the frame's values are
// already encoded, so they are decoded first.
@fragment
fn fs_linear(in: Varyings) -> @location(0) vec4f {
    let color = textureSample(frame_texture, frame_sampler, in.uv);
    let low = color.rgb / 12.92;
    let high = pow((color.rgb + 0.055) / 1.055, vec3f(2.4));
    return vec4f(select(high, low, color.rgb <= vec3f(0.04045)), color.a);
}
