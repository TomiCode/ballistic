// Particles: texture × vertex colour, unlit.

struct Globals {
    view_proj: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
) -> VsOut {
    var o: VsOut;
    o.position = globals.view_proj * vec4<f32>(position, 1.0);
    o.uv = uv;
    o.color = color;
    return o;
}

@fragment
fn fs_main(v: VsOut) -> @location(0) vec4<f32> {
    let t = textureSample(tex, samp, v.uv);
    // A colour-keyed texture's key is cut out (Virtools' alpha test), whatever
    // the blend: the extra point's sparks blend One / Src Color, where the
    // key's black would otherwise darken what's behind.
    if t.a < 0.5 {
        discard;
    }
    return t * v.color;
}
