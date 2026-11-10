// A character's shadow: its mesh drawn again, flattened onto the floor
// along the light's direction, in a see-through dark colour. Our own
// shader. It gives the look of the original's shadow projector (the
// character's shape laid on the floor, from a light high above) without
// drawing any extra picture, so phones and VR headsets can afford it.
//
// Each spot of the floor must be darkened once, even where several of the
// body's triangles land on it. The shadow writes depth and only draws where
// nothing at least as near is drawn yet; depth is rounded (the low bits
// dropped) so that two triangles flattened onto the same floor spot get
// exactly the same depth and the second one is skipped.

import bevy_pbr::render::{
    mesh_functions,
    view_transformations::position_world_to_clip,
};

struct Shadow {
    // xyz: towards the light (length 1). w: lift above the floor.
    light: vec4<f32>,
    // x: floor height. y: darkness 0 to 1. z, w unused.
    floor: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> shadow: Shadow;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
};

struct Out {
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vertex(v: Vertex) -> Out {
    let world_from_local = mesh_functions::get_world_from_local(v.instance_index);
    let world = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(v.position, 1.0)).xyz;
    let floor = shadow.floor.x;
    let light = shadow.light.xyz;
    // Slide the point down along the light until it meets the floor. A
    // point already below the floor is put on the floor where it is.
    let above = max(world.y - floor, 0.0);
    var flat = world - light * (above / max(light.y, 0.001));
    flat.y = floor + shadow.light.w;
    var out: Out;
    out.position = position_world_to_clip(flat);
    return out;
}

struct FragmentOut {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fragment(in: Out) -> FragmentOut {
    var out: FragmentOut;
    // Black, see-through by the darkness: what is behind is darkened.
    out.color = vec4<f32>(0.0, 0.0, 0.0, shadow.floor.y);
    // Drop the lowest 10 bits of the depth (about 1 part in 8000).
    out.depth = bitcast<f32>(bitcast<u32>(in.position.z) & 0xFFFFFC00u);
    return out;
}
