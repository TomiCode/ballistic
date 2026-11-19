// Fixed-function style shading for Ballance scenes. All maths in gamma space.

struct Globals {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    camera_pos: vec4<f32>,
    // Direction *towards* the light, normalised.
    light_dir: vec4<f32>,
    light_color: vec4<f32>,
    ambient: vec4<f32>,
    fog_color: vec4<f32>,
    // x = start, y = end, z = enabled (0/1)
    fog: vec4<f32>,
};

struct Object {
    model: mat4x4<f32>,
    normal: mat4x4<f32>,
};

struct Material {
    diffuse: vec4<f32>,
    ambient: vec4<f32>,
    emissive: vec4<f32>,
    // x = alpha ref (0..1), y = alpha func (0 = no test), z = texture blend, w = flags
    params: vec4<f32>,
    // xy = texture coordinate offset
    uv: vec4<f32>,
};

const FLAG_ENV_MAP: u32 = 1u;

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var<uniform> object: Object;
@group(2) @binding(0) var<uniform> material: Material;
@group(2) @binding(1) var tex: texture_2d<f32>;
@group(2) @binding(2) var samp: sampler;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

@vertex
fn vs_main(v: VsIn) -> VsOut {
    let world = object.model * vec4<f32>(v.position, 1.0);
    var o: VsOut;
    o.clip = globals.view_proj * world;
    o.world_pos = world.xyz;
    o.normal = (object.normal * vec4<f32>(v.normal, 0.0)).xyz;
    o.uv = v.uv;
    return o;
}

// Compare functions use the bundle's order: 1 Never … 8 Always.
fn alpha_passes(a: f32, func: u32, r: f32) -> bool {
    switch func {
        case 1u: { return false; }
        case 2u: { return a < r; }
        case 3u: { return a == r; }
        case 4u: { return a <= r; }
        case 5u: { return a > r; }
        case 6u: { return a != r; }
        case 7u: { return a >= r; }
        default: { return true; }
    }
}

@fragment
fn fs_main(i: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let flags = u32(material.params.w);
    var n = normalize(i.normal);
    n = select(-n, n, front);

    let ndl = max(dot(n, globals.light_dir.xyz), 0.0);
    let lit_rgb = material.emissive.rgb
        + globals.ambient.rgb * material.ambient.rgb
        + globals.light_color.rgb * material.diffuse.rgb * ndl;
    let lit = vec4<f32>(min(lit_rgb, vec3<f32>(1.0)), material.diffuse.a);

    // Sphere-map style environment mapping from the view-space normal.
    let vn = normalize((globals.view * vec4<f32>(n, 0.0)).xyz);
    let env_uv = vec2<f32>(vn.x * 0.5 + 0.5, -vn.y * 0.5 + 0.5);
    let uv = select(i.uv + material.uv.xy, env_uv, (flags & FLAG_ENV_MAP) != 0u);
    let t = textureSample(tex, samp, uv);

    var c: vec4<f32>;
    switch u32(material.params.z) {
        case 0u, 6u: { c = t; }                                         // decal, copy
        case 2u: { c = vec4<f32>(mix(lit.rgb, t.rgb, t.a), lit.a); }   // decal alpha
        case 4u: { c = vec4<f32>(t.rgb, lit.a); }                       // decal mask
        case 7u: { c = vec4<f32>(min(t.rgb + lit.rgb, vec3<f32>(1.0)), lit.a * t.a); } // add
        default: { c = t * lit; }                                       // modulate variants
    }

    let func = u32(material.params.y);
    if func != 0u && !alpha_passes(c.a, func, material.params.x) {
        discard;
    }

    if globals.fog.z > 0.5 {
        let d = distance(i.world_pos, globals.camera_pos.xyz);
        let f = clamp((d - globals.fog.x) / max(globals.fog.y - globals.fog.x, 1e-3), 0.0, 1.0);
        c = vec4<f32>(mix(c.rgb, globals.fog_color.rgb, f), c.a);
    }
    return c;
}
