//! How a level is lit in Bevy: the settings, and the map's real-time lights.
//!
//! Most of a Republic Commando level's light was worked out when the level
//! was built and saved with it ("baked"): lightmap pages for the walls and a
//! colour per corner for the props. `scene.rs` reads those; this module holds
//! how bright they are drawn, and turns the few lights that change during
//! play (`bDynamicLight`) into Bevy lights.
//!
//! Input: the lights and zone ambients read by `rc_package::light`.
//! Output: Bevy `PointLight`, `SpotLight` and `DirectionalLight` entities and
//! the scene-wide ambient light, placed with the project's axis rule
//! (Bevy x, y, z = Unreal x, z, y, divided by `units_per_bevy`).
//!
//! Every number that decides how bright or how far things are comes from
//! `LightingSettings`, which a mod can change with a plain text file (see
//! `LightingSettings::load`). The baked-light gains follow how UE2-era
//! engines drew lightmaps (at double strength, so a stored 128 means
//! "normal"); the real-time light factors are still placeholders.

use bevy::prelude::*;
use rc_package::light::{Light, LightKind, ZoneAmbient};
use std::path::Path;

/// Which of the map's lights become real-time Bevy lights.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveLights {
    /// Every light, on top of the baked light (too bright; for testing, or
    /// with `baked_light = false`).
    All,
    /// Only lights marked `bDynamicLight`; the rest is in the baked lighting.
    /// This is the faster choice (and the one that suits VR's frame rates).
    DynamicOnly,
    None,
}

/// How the game's light numbers become Bevy's. Defaults are placeholders.
#[derive(Resource, Debug, Clone)]
pub struct LightingSettings {
    /// Unreal units per Bevy unit (50, as agreed for the whole viewer).
    pub units_per_bevy: f32,
    /// Draw the light the game baked into the level. When false, everything
    /// is drawn at full brightness (the old look, handy for comparing).
    pub baked_light: bool,
    /// Lightmap brightness multiplier, in the game's own brightness curve
    /// (texture x lightmap x this). 2 matches the "double strength" way
    /// lightmaps were drawn in that era of the engine; measured pages store
    /// almost everything below 128, which fits.
    pub lightmap_gain: f32,
    /// The same for the props' per-corner colours.
    pub vertex_colour_gain: f32,
    pub live_lights: LiveLights,
    /// Bevy lumens = this * (brightness / 255) * range². Scaling with range²
    /// keeps a light's look at a given fraction of its reach the same for
    /// small and big lights, which is how the game describes its lights.
    pub lumens_per_range_squared: f32,
    /// Sunlight brightness 255 becomes this many lux.
    pub sun_lux_at_255: f32,
    /// Zone ambient brightness 255 becomes this ambient brightness (cd/m²).
    pub ambient_at_255: f32,
    pub shadows: bool,
}

impl Default for LightingSettings {
    fn default() -> Self {
        Self {
            units_per_bevy: 50.0,
            baked_light: true,
            lightmap_gain: 2.0,
            vertex_colour_gain: 2.0,
            live_lights: LiveLights::DynamicOnly,
            lumens_per_range_squared: 2000.0,
            sun_lux_at_255: 10_000.0,
            ambient_at_255: 2000.0,
            shadows: false,
        }
    }
}

impl LightingSettings {
    /// Read `key = value` lines from a text file, starting from the defaults.
    /// Unknown keys and bad values are reported, not fatal. Lines starting
    /// with `#` are comments. A missing file just means the defaults.
    pub fn load(path: &Path) -> (Self, Vec<String>) {
        let mut s = Self::default();
        let mut notes = Vec::new();
        let Ok(text) = std::fs::read_to_string(path) else {
            return (s, notes);
        };
        for (n, line) in text.lines().enumerate() {
            // Everything after a `#` is a comment.
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                notes.push(format!("line {}: no '=' in \"{line}\"", n + 1));
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            let num = value.parse::<f32>();
            match (key, num) {
                ("units_per_bevy", Ok(v)) if v > 0.0 => s.units_per_bevy = v,
                ("lightmap_gain", Ok(v)) if v >= 0.0 => s.lightmap_gain = v,
                ("vertex_colour_gain", Ok(v)) if v >= 0.0 => s.vertex_colour_gain = v,
                ("baked_light", _) => s.baked_light = value == "true",
                ("lumens_per_range_squared", Ok(v)) => s.lumens_per_range_squared = v,
                ("sun_lux_at_255", Ok(v)) => s.sun_lux_at_255 = v,
                ("ambient_at_255", Ok(v)) => s.ambient_at_255 = v,
                ("shadows", _) => s.shadows = value == "true",
                ("live_lights", _) => match value {
                    "all" => s.live_lights = LiveLights::All,
                    "dynamic" => s.live_lights = LiveLights::DynamicOnly,
                    "none" => s.live_lights = LiveLights::None,
                    _ => notes.push(format!(
                        "line {}: live_lights must be all, dynamic or none",
                        n + 1
                    )),
                },
                _ => notes.push(format!(
                    "line {}: unknown key or bad value \"{line}\"",
                    n + 1
                )),
            }
        }
        (s, notes)
    }

    /// The game multiplied colours on its own brightness curve (roughly
    /// "gamma 2.2"); Bevy multiplies in linear light. Doubling on the game's
    /// curve is the same as multiplying by 2^2.2 (about 4.6) in linear light.
    pub fn linear_gain(gain: f32) -> f32 {
        gain.max(0.0).powf(2.2)
    }

    /// An Unreal position as a Bevy position.
    pub fn position(&self, ue: [f32; 3]) -> Vec3 {
        Vec3::new(ue[0], ue[2], ue[1]) / self.units_per_bevy
    }
}

/// Unreal rotation (pitch, yaw, roll; 65536 = a full turn) as the direction
/// the actor faces, in Bevy axes. Roll doesn't change the facing direction.
pub fn facing(rotation: [i32; 3]) -> Vec3 {
    let to_rad = |r: i32| r as f32 * std::f32::consts::TAU / 65536.0;
    let (pitch, yaw) = (to_rad(rotation[0]), to_rad(rotation[1]));
    let ue = [
        pitch.cos() * yaw.cos(),
        pitch.cos() * yaw.sin(),
        pitch.sin(),
    ];
    Vec3::new(ue[0], ue[2], ue[1]).normalize_or(Vec3::NEG_Y)
}

fn color(l: &Light) -> Color {
    let [r, g, b] = l.color();
    Color::srgb(r, g, b)
}

/// What `spawn_lights` did, for the log.
#[derive(Debug, Default, Clone, Copy)]
pub struct SpawnReport {
    pub point: usize,
    pub spot: usize,
    pub directional: usize,
    pub skipped: usize,
}

pub fn spawn_lights(
    commands: &mut Commands,
    settings: &LightingSettings,
    lights: &[Light],
) -> SpawnReport {
    let mut report = SpawnReport::default();
    for l in lights {
        let live = match settings.live_lights {
            LiveLights::All => true,
            LiveLights::DynamicOnly => l.dynamic,
            LiveLights::None => false,
        };
        if !live {
            report.skipped += 1;
            continue;
        }
        let range = l.radius / settings.units_per_bevy;
        let lumens = settings.lumens_per_range_squared * (l.brightness / 255.0) * range * range;
        let at = settings.position(l.location);
        match l.kind {
            LightKind::Point => {
                commands.spawn((
                    PointLight {
                        color: color(l),
                        intensity: lumens,
                        range,
                        shadow_maps_enabled: settings.shadows,
                        ..default()
                    },
                    Transform::from_translation(at),
                ));
                report.point += 1;
            }
            LightKind::Spot => {
                // Guess: LightCone 0-255 spans 0-90 degrees from the centre
                // line. Not checked against the game.
                let outer = (l.cone as f32 / 255.0 * std::f32::consts::FRAC_PI_2).clamp(0.02, 1.55);
                commands.spawn((
                    SpotLight {
                        color: color(l),
                        intensity: lumens,
                        range,
                        outer_angle: outer,
                        inner_angle: outer * 0.8,
                        shadow_maps_enabled: settings.shadows,
                        ..default()
                    },
                    Transform::from_translation(at).looking_to(facing(l.rotation), Vec3::Y),
                ));
                report.spot += 1;
            }
            LightKind::Directional => {
                commands.spawn((
                    DirectionalLight {
                        color: color(l),
                        illuminance: settings.sun_lux_at_255 * l.brightness / 255.0,
                        shadow_maps_enabled: settings.shadows,
                        ..default()
                    },
                    Transform::default().looking_to(facing(l.rotation), Vec3::Y),
                ));
                report.directional += 1;
            }
        }
    }
    report
}

/// Not used while the baked light is on (it already includes the zones'
/// ambient light); kept for drawing without it.
#[allow(dead_code)]
/// Bevy has one scene-wide ambient light, but the game has one per zone. Until
/// zones are known in the viewer, use the dimmest non-zero zone ambient: very
/// bright ones (192-255 seen) are sky zones. Returns what it chose.
pub fn global_ambient(
    settings: &LightingSettings,
    zones: &[ZoneAmbient],
) -> Option<GlobalAmbientLight> {
    let z = zones
        .iter()
        .filter(|z| z.brightness > 0)
        .min_by_key(|z| z.brightness)?;
    let [r, g, b] = rc_package::light::hue_saturation_to_rgb(z.hue, z.saturation);
    Some(GlobalAmbientLight {
        color: Color::srgb(r, g, b),
        brightness: settings.ambient_at_255 * z.brightness as f32 / 255.0,
        ..default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axes_and_facing() {
        let s = LightingSettings::default();
        // Unreal Z is up; Bevy Y is up.
        assert_eq!(s.position([0.0, 0.0, 50.0]), Vec3::Y);
        assert_eq!(s.position([50.0, 100.0, 0.0]), Vec3::new(1.0, 0.0, 2.0));
        // Yaw 0 faces Unreal +X; pitch -16384 (quarter turn down) faces down.
        assert!((facing([0, 0, 0]) - Vec3::X).length() < 1e-5);
        assert!((facing([-16384, 0, 0]) - Vec3::NEG_Y).length() < 1e-5);
        // Yaw 16384 faces Unreal +Y, which is Bevy +Z.
        assert!((facing([0, 16384, 0]) - Vec3::Z).length() < 1e-5);
    }

    #[test]
    fn settings_file() {
        let dir = std::env::temp_dir().join("rc_viewer_lighting_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("lighting.cfg");
        std::fs::write(
            &path,
            "# test\nlive_lights = dynamic # baked rest\nsun_lux_at_255 = 5\nnonsense = 1\n",
        )
        .unwrap();
        let (s, notes) = LightingSettings::load(&path);
        assert_eq!(s.live_lights, LiveLights::DynamicOnly);
        assert_eq!(s.sun_lux_at_255, 5.0);
        assert_eq!(notes.len(), 1);
        std::fs::write(&path, "baked_light = false\nlightmap_gain = 1.5\n").unwrap();
        let (s, notes) = LightingSettings::load(&path);
        assert!(!s.baked_light);
        assert_eq!(s.lightmap_gain, 1.5);
        assert!(notes.is_empty());
        assert!((LightingSettings::linear_gain(2.0) - 4.595).abs() < 0.01);
    }
}
