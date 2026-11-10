// Holograms, drawn the way Republic Commando's DynamicHologram hardware
// shader does (its assembly text is in the game's hardwareshaders.utx; it
// was read to learn what it does, nothing is copied). See lib.rs for the
// numbers and where each comes from.
//
// Per pixel, on the game's brightness curve:
//
//   shown = picture (its brightness in the hologram colour)
//         x glow picture, looked up by how squarely the surface faces the eye
//           (edges seen side-on fade out)
//         x noise picture, laid across the model and jumping to a new
//           place every flicker step (the shimmer)
//         x facing x small flicker (the frequent brightness wobble)
//
// and added to what is behind it. Per corner, the whole model is squashed
// in height now and then (the big, rare flicker) and nudged by a tiny
// amount (the small flicker).
//
// The flicker values change in steps (params.glow.w per second), from a
// hash of the step number, so they don't depend on the frame rate.
//
// Cost: three texture reads and a few sums per pixel; fine for phones and
// VR headsets.

import bevy_pbr::render::{
    mesh_functions,
    mesh_view_bindings::{globals, view},
    view_transformations::position_world_to_clip,
};

struct Params {
    // x: glow picture across per unit of facing, y: offset,
    // z: overall brightness, w: flicker steps per second.
    glow: vec4<f32>,
    // x: noise repeats per Unreal unit, y, z: how far the noise jumps
    // across and down per flicker step, w: Unreal units per Bevy unit.
    noise: vec4<f32>,
    // x: chance a step keeps the full height, y: most it squashes.
    big: vec4<f32>,
    // x: chance a step keeps full brightness, y: most it dims,
    // z: how far (Unreal units) the model is nudged per unit of flicker.
    small: vec4<f32>,
    // x: chance the noise stays put, y: how far it may jump (0 to 1).
    shimmer: vec4<f32>,
    // The point the height squash pulls towards, in the mesh's own space.
    pivot: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> params: Params;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var picture_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var picture_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var glow_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var glow_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var noise_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var noise_sampler: sampler;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct Corner {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) world_position: vec4<f32>,
    // Position in the mesh's own space, before any flicker (for the noise).
    @location(3) local: vec3<f32>,
};

// A number from 0 to 1 for this step and purpose (PCG hash).
fn rand(step: u32, salt: u32) -> f32 {
    var s = step * 747796405u + salt * 2891336453u + 1u;
    s = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    s = (s >> 22u) ^ s;
    return f32(s) / 4294967295.0;
}

// One flicker value: 1 most of the time; on a step that doesn't keep it,
// 1 minus up to `amount`.
fn flicker(step: u32, salt: u32, keep: f32, amount: f32) -> f32 {
    if rand(step, salt) < keep {
        return 1.0;
    }
    return 1.0 - rand(step, salt + 7u) * amount;
}

fn step_now() -> u32 {
    return u32(max(globals.time * params.glow.w, 0.0));
}

@vertex
fn vertex(v: Vertex) -> Corner {
    var out: Corner;
    let step = step_now();
    let big = flicker(step, 1u, params.big.x, params.big.y);
    let small = flicker(step, 2u, params.small.x, params.small.y);
    // Squash the height towards the pivot (Bevy's up is Y), then nudge.
    var p = v.position - params.pivot.xyz;
    p.y = p.y * big;
    p = p + params.pivot.xyz + vec3<f32>(small * params.small.z / max(params.noise.w, 1.0));
    let world_from_local = mesh_functions::get_world_from_local(v.instance_index);
    let world = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(p, 1.0));
    out.clip_position = position_world_to_clip(world.xyz);
    out.world_position = world;
    out.uv = v.uv;
    out.normal = mesh_functions::mesh_normal_local_to_world(v.normal, v.instance_index);
    out.local = v.position;
    return out;
}

// The sRGB curve (the game's brightness curve) to linear light, and back.
fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

fn to_curve(c: vec3<f32>) -> vec3<f32> {
    let low = c * 12.92;
    let high = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3<f32>(0.0031308));
}

@fragment
fn fragment(in: Corner) -> @location(0) vec4<f32> {
    let step = step_now();
    let n = normalize(in.normal);
    let to_eye = normalize(view.world_position.xyz - in.world_position.xyz);
    // Drawn from both sides, so the far side counts as facing too.
    let facing = clamp(abs(dot(n, to_eye)), 0.0, 1.0);

    let glow = textureSample(glow_texture, glow_sampler, vec2<f32>(facing * params.glow.x + params.glow.y, 0.5));

    // The noise lies across the model's width and height (Unreal X and Z,
    // Bevy X and Y), and jumps each step.
    let jump = flicker(step, 3u, params.shimmer.x, params.shimmer.y);
    let across = vec2<f32>(in.local.x, in.local.y) * params.noise.w * params.noise.x;
    let noise_uv = across - jump * params.noise.yz;
    let noise = textureSample(noise_texture, noise_sampler, noise_uv);

    let small = flicker(step, 2u, params.small.x, params.small.y);
    let picture = to_curve(textureSample(picture_texture, picture_sampler, in.uv).rgb);
    let shown = picture * glow.rgb * noise.rgb * clamp(facing * small, 0.0, 1.0) * glow.a * params.glow.z;
    // Alpha 0: Bevy's added blending then adds the colour as it is.
    return vec4<f32>(to_linear(clamp(shown, vec3<f32>(0.0), vec3<f32>(1.0))), 0.0);
}
