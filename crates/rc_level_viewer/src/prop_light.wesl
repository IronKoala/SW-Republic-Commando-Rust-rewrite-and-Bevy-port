// Props and terrain: the map's baked light plus the real-time lights.
//
// A prop's baked light comes with the map as one colour per corner (the
// mesh's vertex colours). Bevy's own material would treat those as the
// surface's paint colour, and its lights would then be multiplied by that
// dark paint. Here they are kept apart instead:
//
//   shown = texture x baked colour            (as before, no lights needed)
//         + texture x real-time lights x live_gain
//
// The real-time part is Bevy's normal lighting (moving, flickering and
// switched lights, the helmet lamp). Ambient light is off in our levels,
// so it adds only those lights. The cost is the same as any lit surface:
// fine for phones and VR headsets.

import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
};

struct PropLight {
    // x: how strongly real-time lights show on props (1 = like on walls).
    // y, z, w unused (kept for 16-byte alignment on older phones).
    settings: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> prop_light: PropLight;

@fragment
fn fragment(vertex_output: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    // The baked light, before Bevy can mistake it for paint. Alpha stays:
    // terrain layers use it to paint over each other.
    var in = vertex_output;
    var baked = vec3<f32>(1.0);
@if(VERTEX_COLORS) {
    baked = vertex_output.color.rgb;
    in.color = vec4<f32>(1.0, 1.0, 1.0, vertex_output.color.a);
}

    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material.flags,
        pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color,
    );
    let surface = pbr_input.material.base_color;

    let live = apply_pbr_lighting(pbr_input);
    let gain = max(prop_light.settings.x, 0.0);

    var out: FragmentOutput;
    out.color = vec4<f32>(surface.rgb * baked + live.rgb * gain, surface.a);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
