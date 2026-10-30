// The Panini view: for each pixel on screen, find where it was in the
// normal (straight-line) 3D view and copy that colour. Our own code; the
// projection is the one described by Thomas Sharpless and others
// ("Pannini: a new projection for rendering wide angle perspective
// images", 2010). The maths is explained in crates/rc_panini/src/lib.rs.

struct Panini {
    // How strong the bend is: 0 is none, 1 is full Panini.
    strength: f32,
    // The Panini x at the screen's left and right edges.
    x_edge: f32,
    // tan of half the 3D view's angle across.
    src_tan: f32,
    // Screen width / height.
    aspect: f32,
};

@group(0) @binding(0) var screen: texture_2d<f32>;
@group(0) @binding(1) var screen_sampler: sampler;
@group(0) @binding(2) var<uniform> panini: Panini;

// Bilinear: the pass's sampler takes the nearest pixel, which would look
// jagged when the picture is bent.
fn sample(uv: vec2<f32>) -> vec4<f32> {
    let size = vec2<f32>(textureDimensions(screen));
    let p = clamp(uv * size - 0.5, vec2(0.0), size - 1.0);
    let i = vec2<i32>(floor(p));
    let f = p - floor(p);
    let m = vec2<i32>(size) - 1;
    let a = textureLoad(screen, i, 0);
    let b = textureLoad(screen, min(i + vec2(1, 0), m), 0);
    let c = textureLoad(screen, min(i + vec2(0, 1), m), 0);
    let d = textureLoad(screen, min(i + vec2(1, 1), m), 0);
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

@fragment
fn fragment(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let d = panini.strength;
    // Screen position, -1..1 across, up positive, square pixels.
    let s = vec2(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let x = s.x * panini.x_edge;
    let y = s.y * panini.x_edge / panini.aspect;
    // Back from Panini to the direction: cos and sin of the angle across,
    // and the height on the cylinder.
    let k = x / (d + 1.0);
    let kk = k * k;
    let c = (-kk * d + sqrt(1.0 + kk * (1.0 - d * d))) / (1.0 + kk);
    let sn = k * (d + c);
    let h = y * (d + c) / (d + 1.0);
    // Then into the straight-line view.
    let src = vec2(sn / c, h / c) / vec2(panini.src_tan, panini.src_tan / panini.aspect);
    return sample(vec2(src.x * 0.5 + 0.5, 0.5 - src.y * 0.5));
}
