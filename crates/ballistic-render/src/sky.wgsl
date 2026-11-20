// The sky box: texture × vertex colour, no lighting, no depth.

struct Globals {
    view_proj: mat4x4<f32>,
    color: vec4<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>) -> VsOut {
    var o: VsOut;
    o.position = globals.view_proj * vec4<f32>(position, 1.0);
    o.uv = uv;
    return o;
}

@fragment
fn fs_main(v: VsOut) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, v.uv) * globals.color;
}
