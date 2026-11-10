// The low light visor's night-vision picture: one full-screen pass over the
// finished 3D view of the player's camera. Our own shader, written from
// what the original's low light settings say it does (see
// night_vision.rs and docs/DESIGN.md); the pictures it uses (the visor
// window and the film grain) are read from the install at run time.
//
// The original worked on the picture's screen values (0 to 1, as shown),
// so this does too: it turns the colours it reads back into screen values
// first, and the result back at the end. rules::night_brightness in
// rules.rs is the same sum, checked by the tests.

struct Night {
    tint: vec4<f32>,
    boost: f32,
    grain: f32,
    fade: f32,
    window: f32,
    window_middle: f32,
    blur: f32,
    grain_tiles: vec2<f32>,
    grain_offset: vec2<f32>,
    unused: vec2<f32>,
};

@group(0) @binding(0) var screen: texture_2d<f32>;
@group(0) @binding(1) var clamp_sampler: sampler;
@group(0) @binding(2) var<uniform> night: Night;
@group(0) @binding(3) var window_picture: texture_2d<f32>;
@group(0) @binding(4) var grain_picture: texture_2d<f32>;
@group(0) @binding(5) var repeat_sampler: sampler;

fn to_screen(c: vec3<f32>) -> vec3<f32> {
    return pow(clamp(c, vec3(0.0), vec3(1.0)), vec3(1.0 / 2.2));
}

fn from_screen(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3(0.0)), vec3(2.2));
}

// Across the visor window picture (0 to 1) for a place across the screen.
// The picture was made for 4:3. On wider screens its middle and its outer
// parts keep their shape (in screen heights) and the plain part between
// them is drawn longer, so nothing is stretched sideways; on narrower ones
// it is squeezed to fit.
fn window_u(x: f32, aspect: f32) -> f32 {
    let four_three = 4.0 / 3.0;
    if aspect <= four_three {
        return x;
    }
    let half_picture = four_three * 0.5;
    let middle = clamp(night.window_middle, 0.0, half_picture);
    let d = (x - 0.5) * aspect;
    let a = abs(d);
    var o = middle;
    if a <= middle {
        o = a;
    } else if a >= aspect * 0.5 - (half_picture - middle) {
        o = half_picture - (aspect * 0.5 - a);
    }
    return 0.5 + sign(d) * o / four_three;
}

@fragment
fn fragment(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(screen));
    let aspect = size.x / size.y;

    // A slightly blurred copy, as the original read.
    let s = night.blur / size;
    var b = textureSample(screen, clamp_sampler, uv).rgb * 2.0;
    b += textureSample(screen, clamp_sampler, uv + vec2(s.x, 0.0)).rgb;
    b += textureSample(screen, clamp_sampler, uv - vec2(s.x, 0.0)).rgb;
    b += textureSample(screen, clamp_sampler, uv + vec2(0.0, s.y)).rgb;
    b += textureSample(screen, clamp_sampler, uv - vec2(0.0, s.y)).rgb;
    let c = to_screen(b / 6.0);

    // One brightness: red, green and blue squared and added up.
    let v = clamp(dot(c, c) * night.boost, 0.0, 1.0) * clamp(night.fade, 0.0, 1.0);
    var col = vec3(v) * night.tint.rgb;

    // Through the visor window; its colour is added on top.
    let w = textureSample(window_picture, clamp_sampler, vec2(window_u(uv.x, aspect), uv.y));
    let open = mix(1.0, w.a, night.window);
    col = col * open + w.rgb * night.window;

    // Film grain, a new place every frame. Square specks: the repeats
    // across grow with the screen's width.
    let tiles = night.grain_tiles * vec2(aspect / (4.0 / 3.0), 1.0);
    let g = textureSample(grain_picture, repeat_sampler, uv * tiles + night.grain_offset).r;
    col = col + vec3(night.grain * (g - 0.5));

    return vec4(from_screen(col), 1.0);
}
