// Particle sprites with what the engine's particle drawing adds to Bevy's
// unlit standard material (see SpriteLook in lib.rs). The depth test is
// switched in the pipeline, not here. Without MIX_FRAMES this is the same
// as Bevy's own unlit path.
//
// MIX_FRAMES (BlendBetweenSubdivisions): the graphics driver (D3DDrv.dll,
// FD3DRenderInterface::SetParticleMaterial) draws
//   stage 1: the sprite's frame (first texture coordinates)
//   stage 2: the next frame (second coordinates) mixed in by the vertex
//            alpha (D3DTOP_BLENDDIFFUSEALPHA), colour and alpha alike
//   stage 3: colour times the vertex colour, alpha from the mix alone
// The vertex alpha is the next frame's share, written by Engine.dll.

import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, main_pass_post_lighting_processing},
};

@fragment
fn fragment(vertex_output: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(vertex_output, is_front);

@if(MIX_FRAMES && VERTEX_COLORS && VERTEX_UVS_B) {
    var plain = vertex_output;
    plain.color = vec4<f32>(1.0);
    let this_frame = pbr_input_from_standard_material(plain, is_front).material.base_color;
    var next_in = plain;
    next_in.uv = vertex_output.uv_b;
    let next_frame = pbr_input_from_standard_material(next_in, is_front).material.base_color;
    let mixed = mix(this_frame, next_frame, vertex_output.color.a);
    pbr_input.material.base_color = vec4<f32>(mixed.rgb * vertex_output.color.rgb, mixed.a);
}

    pbr_input.material.base_color = alpha_discard(
        pbr_input.material.flags,
        pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color,
    );
    var out: FragmentOutput;
    out.color = main_pass_post_lighting_processing(pbr_input, pbr_input.material.base_color);
    return out;
}
