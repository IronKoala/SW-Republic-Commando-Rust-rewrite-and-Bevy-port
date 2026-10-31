// The tactical visor's glow, drawn as a second copy of a body on top of
// it and added to what is already there. Edges (where the surface turns
// away from the eye) glow most, so the body gets a coloured outline. Our
// own shader; the maths is the same as rules::glow_amount in rules.rs.
//
// Cheap on purpose (no textures, a few sums per pixel) so phones and VR
// headsets can draw it. It uses each camera's own position, so it is right
// for every eye of a VR headset too.

import bevy_pbr::render::{
    forward_io::VertexOutput,
    mesh_view_bindings::view,
};

struct Glow {
    // Linear red, green, blue; alpha unused.
    color: vec4<f32>,
    edge_power: f32,
    fill: f32,
    strength: f32,
    unused: f32,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> glow: Glow;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let n = normalize(in.world_normal);
    let to_eye = normalize(view.world_position - in.world_position.xyz);
    let facing = clamp(abs(dot(n, to_eye)), 0.0, 1.0);
    let edge = pow(1.0 - facing, max(glow.edge_power, 0.01));
    let amount = (glow.fill + (1.0 - glow.fill) * edge) * max(glow.strength, 0.0);
    // Alpha 0 with additive blending: only adds light, never darkens.
    return vec4<f32>(glow.color.rgb * amount, 0.0);
}
