// Props with the game's bump maps (see bump.rs). Our own shader.
//
// Each corner carries the light the game stored for it (with the zone's
// ambient added), the direction the main light comes from and that light's
// colour. Per pixel:
//
//   light = corner colour + light colour x (pixel facing - corner facing)
//
// "facing" is how squarely the surface faces the light, using the bump
// map's tilt for the pixel and the plain surface for the corner, so a flat
// pixel gets exactly the stored colour. The stored colours are on the
// game's own brightness curve; each corner's is turned into linear light
// and blended across the triangle the same way flat props are drawn
// (draw.rs), the change is added back on the game's curve, and the result
// multiplies the picture.
//
// Real-time lights (moving or switched lights, the helmet lamp) are added
// on top with Bevy's own lighting, using the bump-tilted surface, times
// params.live_light (0 = none). With none near, that adds nothing.
//
// Cheap on purpose (one extra texture read, Bevy's usual work for the few
// real-time lights near) so phones and VR headsets can draw it.

import bevy_pbr::render::{
    mesh_functions,
    pbr_functions::{apply_pbr_lighting, calculate_view},
    pbr_types::pbr_input_new,
    view_transformations::position_world_to_clip,
};

struct Params {
    gain: f32,
    bump_uv_scale: f32,
    strength: f32,
    live_light: f32,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> params: Params;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var diffuse_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var diffuse_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var bump_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var bump_sampler: sampler;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(4) tangent: vec4<f32>,
    @location(5) color: vec4<f32>,
    @location(8) light_dir: vec3<f32>,
    @location(9) light_colour: vec3<f32>,
};

struct Corner {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tangent: vec4<f32>,
    // The stored light in linear light (turned from the game's curve per
    // corner, so it blends across a triangle the same way as on flat
    // props).
    @location(3) color: vec4<f32>,
    @location(4) light_dir: vec3<f32>,
    @location(5) light_colour: vec3<f32>,
    @location(6) world_position: vec4<f32>,
};

@vertex
fn vertex(v: Vertex) -> Corner {
    var out: Corner;
    let world_from_local = mesh_functions::get_world_from_local(v.instance_index);
    let world = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(v.position, 1.0));
    out.clip_position = position_world_to_clip(world.xyz);
    out.world_position = world;
    out.uv = v.uv;
    out.normal = mesh_functions::mesh_normal_local_to_world(v.normal, v.instance_index);
    out.tangent = mesh_functions::mesh_tangent_local_to_world(world_from_local, v.tangent, v.instance_index);
    out.color = vec4<f32>(to_linear(v.color.rgb), 1.0);
    // Doors and lifts only move and turn, so a plain turn is right here.
    out.light_dir = (world_from_local * vec4<f32>(v.light_dir, 0.0)).xyz;
    out.light_colour = v.light_colour;
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
    let n = normalize(in.normal);
    // Along the picture's width (U) and down its height (V).
    let u_dir = normalize(in.tangent.xyz - n * dot(n, in.tangent.xyz));
    let v_dir = cross(n, u_dir) * in.tangent.w;
    // Bump map: red = tilt along U, green = along V, blue = straight out.
    let tilt = textureSample(bump_texture, bump_sampler, in.uv * params.bump_uv_scale).xyz * 2.0 - 1.0;
    let bumped = normalize(u_dir * tilt.x + v_dir * tilt.y + n * max(tilt.z, 0.05));
    let pixel = normalize(mix(n, bumped, params.strength));

    var light = in.color.rgb;
    let len = length(in.light_dir);
    if len > 0.01 {
        let l = in.light_dir / len;
        let change = max(dot(pixel, l), 0.0) - max(dot(n, l), 0.0);
        // The change is added on the game's curve, where the stored
        // colours and the light's strength were measured.
        let on_curve = max(to_curve(light) + in.light_colour * change, vec3<f32>(0.0));
        light = to_linear(on_curve);
    }
    let picture = textureSample(diffuse_texture, diffuse_sampler, in.uv);
    var shown = picture.rgb * light * params.gain;

    // Real-time lights on top (Bevy's lighting: plain diffuse, no shine).
    if params.live_light > 0.0 {
        var pbr = pbr_input_new();
        pbr.material.base_color = vec4<f32>(picture.rgb, 1.0);
        pbr.material.perceptual_roughness = 1.0;
        pbr.material.reflectance = vec3<f32>(0.0);
        pbr.frag_coord = in.clip_position;
        pbr.world_position = in.world_position;
        pbr.world_normal = n;
        pbr.N = pixel;
        pbr.V = calculate_view(in.world_position, false);
        shown += apply_pbr_lighting(pbr).rgb * params.live_light;
    }
    return vec4<f32>(shown, 1.0);
}
