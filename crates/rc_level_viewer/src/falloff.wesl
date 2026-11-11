// The game's falloff shader (HsFalloff, its shader FalloffImpl): light
// shafts and god rays. See falloff.rs for where each step was read.
//
//   picture place = texture place + oscillation x sin(time) + pan x time
//   facing        = max(0, eye direction . surface normal)
//   colour        = picture x tint x gradient(facing) x vertex colour
//   added         = colour (SrcBlend ONE) or colour x colour (SRCCOLOR)
//
// The maths runs on the game's 0-1 colour values (the sRGB curve); the
// result is added as it is (see the end of `fragment`). No fog: the game's shader writes
// no fog value.
//
// Cost: two texture reads and a few sums per pixel; fine for phones and
// VR headsets.

import bevy_pbr::render::{
    mesh_functions,
    mesh_view_bindings::{globals, view},
    view_transformations::position_world_to_clip,
};

struct Params {
    // DiffuseTint x GradientTint (red, green, blue), 0-1.
    tint: vec4<f32>,
    // x, y: UOscilationRate, VOscilationRate; z, w: UPanRate, VPanRate.
    motion: vec4<f32>,
    // x: 1 when SrcBlend is SRCCOLOR (the colour is squared), else 0.
    settings: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> params: Params;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var picture_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var picture_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var gradient_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var gradient_sampler: sampler;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) colour: vec4<f32>,
};

struct Corner {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) world_position: vec4<f32>,
    @location(3) colour: vec4<f32>,
};

@vertex
fn vertex(v: Vertex) -> Corner {
    var out: Corner;
    let world_from_local = mesh_functions::get_world_from_local(v.instance_index);
    let world = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(v.position, 1.0));
    out.clip_position = position_world_to_clip(world.xyz);
    out.world_position = world;
    let t = globals.time;
    out.uv = v.uv + params.motion.xy * sin(t) + params.motion.zw * t;
    out.normal = mesh_functions::mesh_normal_local_to_world(v.normal, v.instance_index);
    out.colour = v.colour;
    return out;
}

// Linear light to the sRGB curve (the game's brightness curve).
fn to_curve(c: vec3<f32>) -> vec3<f32> {
    let low = c * 12.92;
    let high = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3<f32>(0.0031308));
}

@fragment
fn fragment(in: Corner) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let to_eye = normalize(view.world_position.xyz - in.world_position.xyz);
    let facing = max(dot(to_eye, n), 0.0);
    let picture = to_curve(textureSample(picture_texture, picture_sampler, in.uv).rgb);
    let gradient = to_curve(textureSample(gradient_texture, gradient_sampler, vec2<f32>(facing, 0.0)).rgb);
    var colour = clamp(picture * params.tint.rgb * gradient * in.colour.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    if params.settings.x > 0.5 {
        colour = colour * colour;
    }
    // The game adds on its brightness curve; we add in linear light. Adding
    // x on the curve to a mid-grey background (0.45) raises it by about
    // 0.84 x in linear light (1.2 x over bright ground, 0.5 x over dark),
    // so the value is added as it is, not turned into linear light first
    // (which would make it about five times too faint here).
    // Alpha 0: Bevy's added blending then adds the colour as it is.
    return vec4<f32>(colour, 0.0);
}
