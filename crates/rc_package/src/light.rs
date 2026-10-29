//! Turns the properties of a map's light actors (`Light`, `Spotlight`,
//! `Sunlight`, `TriggerLight`, ...) and zones (`ZoneInfo`) into plain numbers
//! a renderer can use: a position, a colour, a brightness and a reach.
//!
//! This module does not find the actors or skip their state frame; it is
//! handed an actor's property list (see `rc_level`'s actor reader).
//!
//! Property names and meanings come from the Unreal Wiki's descriptions of
//! UE2 lighting ("Legacy:Light", "Legacy:LightHue", "Legacy:LightSaturation",
//! "Legacy:ZoneInfo"). Republic Commando differences were measured on the
//! game's maps: `LightBrightness` and `LightRadius` are floats (UE2 used a
//! byte), and there is an extra `LightRadiusInner` (int). How Republic
//! Commando's renderer used `LightRadiusInner` is not known. No code copied.

use crate::properties::{self, Property, Value};
use crate::Package;

/// Values used when an actor doesn't store a property, because the editor
/// only saves values that differ from the class defaults. These are UE2's
/// documented `Light` defaults. Republic Commando's own defaults live in its
/// `Engine.u` and have not been read yet, so these are an assumption.
pub const DEFAULT_BRIGHTNESS: f32 = 64.0;
pub const DEFAULT_HUE: u8 = 0;
pub const DEFAULT_SATURATION: u8 = 255;
pub const DEFAULT_RADIUS: f32 = 64.0;
pub const DEFAULT_CONE: u8 = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightKind {
    /// Shines in every direction from a point (`Light`, `TriggerLight`, ...).
    Point,
    /// Shines in a cone along its rotation (`Spotlight`, or `LightEffect` 3).
    Spot,
    /// Shines everywhere from one direction, like the sun (`Sunlight`).
    Directional,
}

/// One light, in the game's own units (Unreal units, 0-255 colour wheel).
#[derive(Debug, Clone)]
pub struct Light {
    pub kind: LightKind,
    /// Position in Unreal units: X forward, Y right, Z up.
    pub location: [f32; 3],
    /// Pitch, yaw, roll in Unreal's units (65536 = a full turn).
    pub rotation: [i32; 3],
    /// How bright. 0-255 in UE2; Republic Commando stores a float and some
    /// lights go above 255 (400 was seen).
    pub brightness: f32,
    /// Position on the colour wheel, 0-255 (0 red, 85 green, 170 blue).
    pub hue: u8,
    /// 255 = white, lower = more coloured.
    pub saturation: u8,
    /// How far the light reaches, in Unreal units (see `docs/DESIGN.md`).
    pub radius: f32,
    /// Republic Commando only. Meaning unknown; stored as given.
    pub radius_inner: Option<i32>,
    /// Spotlight cone width, 0-255.
    pub cone: u8,
    /// `LightType` (0 none, 1 steady, 2 pulse, ... 4 was seen on TriggerLights).
    pub light_type: u8,
    /// Whether it lights moving things at run time (`bDynamicLight`). When
    /// false, its light was baked into the map when the level was built.
    pub dynamic: bool,
}

/// A zone's flat "ambient" light, added everywhere inside the zone.
#[derive(Debug, Clone)]
pub struct ZoneAmbient {
    pub brightness: u8,
    pub hue: u8,
    pub saturation: u8,
}

fn float(pkg: &Package, props: &[Property], name: &str) -> Option<f32> {
    match properties::find(pkg, props, name).map(|p| &p.value) {
        Some(Value::Float(f)) => Some(*f),
        // UE2 stored these as bytes; accept either.
        Some(Value::Byte(b)) => Some(*b as f32),
        Some(Value::Int(i)) => Some(*i as f32),
        _ => None,
    }
}

fn byte(pkg: &Package, props: &[Property], name: &str) -> Option<u8> {
    match properties::find(pkg, props, name).map(|p| &p.value) {
        Some(Value::Byte(b)) => Some(*b),
        _ => None,
    }
}

fn raw12(pkg: &Package, props: &[Property], name: &str) -> Option<[[u8; 4]; 3]> {
    match properties::find(pkg, props, name).map(|p| &p.value) {
        Some(Value::Raw(r)) if r.len() == 12 => Some(std::array::from_fn(|i| {
            r[i * 4..i * 4 + 4].try_into().unwrap()
        })),
        _ => None,
    }
}

/// `Location`, a struct of three floats. (0, 0, 0) if absent.
pub fn location(pkg: &Package, props: &[Property]) -> [f32; 3] {
    raw12(pkg, props, "Location")
        .map(|v| v.map(f32::from_le_bytes))
        .unwrap_or([0.0; 3])
}

/// `Rotation`, a struct of three ints (pitch, yaw, roll). Zero if absent.
pub fn rotation(pkg: &Package, props: &[Property]) -> [i32; 3] {
    raw12(pkg, props, "Rotation")
        .map(|v| v.map(i32::from_le_bytes))
        .unwrap_or([0; 3])
}

/// Whether this class name is one of the light classes seen in the maps.
pub fn is_light_class(class: &str) -> bool {
    let c = class.to_ascii_lowercase();
    c.ends_with("light") && !c.contains("texture")
}

/// Read a light from an actor's class name and property list. `None` if the
/// light is switched off (`LightType` 0) or isn't a light class.
pub fn light_from_properties(pkg: &Package, class: &str, props: &[Property]) -> Option<Light> {
    if !is_light_class(class) {
        return None;
    }
    let light_type = byte(pkg, props, "LightType").unwrap_or(1);
    if light_type == 0 {
        return None;
    }
    let effect = byte(pkg, props, "LightEffect").unwrap_or(0);
    let lower = class.to_ascii_lowercase();
    let kind = if lower.contains("sun") {
        LightKind::Directional
    } else if lower.contains("spot") || effect == 3 {
        LightKind::Spot
    } else {
        LightKind::Point
    };
    let bool_prop = |name| {
        matches!(
            properties::find(pkg, props, name).map(|p| &p.value),
            Some(Value::Bool(true))
        )
    };
    Some(Light {
        kind,
        location: location(pkg, props),
        rotation: rotation(pkg, props),
        brightness: float(pkg, props, "LightBrightness").unwrap_or(DEFAULT_BRIGHTNESS),
        hue: byte(pkg, props, "LightHue").unwrap_or(DEFAULT_HUE),
        saturation: byte(pkg, props, "LightSaturation").unwrap_or(DEFAULT_SATURATION),
        radius: float(pkg, props, "LightRadius").unwrap_or(DEFAULT_RADIUS),
        radius_inner: match properties::find(pkg, props, "LightRadiusInner").map(|p| &p.value) {
            Some(Value::Int(i)) => Some(*i),
            _ => None,
        },
        cone: byte(pkg, props, "LightCone").unwrap_or(DEFAULT_CONE),
        light_type,
        dynamic: bool_prop("bDynamicLight"),
    })
}

/// Read a zone's ambient light. `None` if it sets no ambient brightness.
pub fn zone_ambient(pkg: &Package, props: &[Property]) -> Option<ZoneAmbient> {
    let brightness = byte(pkg, props, "AmbientBrightness")?;
    Some(ZoneAmbient {
        brightness,
        hue: byte(pkg, props, "AmbientHue").unwrap_or(0),
        saturation: byte(pkg, props, "AmbientSaturation").unwrap_or(255),
    })
}

/// Unreal's hue and saturation as a red, green, blue colour, each 0..1, at
/// full brightness. Hue goes round the colour wheel in 256 steps. Saturation
/// works backwards from what the word suggests: 255 is white, 0 is the pure
/// colour.
pub fn hue_saturation_to_rgb(hue: u8, saturation: u8) -> [f32; 3] {
    let h = hue as f32 / 256.0 * 6.0;
    let sector = h.floor() as i32 % 6;
    let f = h - h.floor();
    let pure = match sector {
        0 => [1.0, f, 0.0],
        1 => [1.0 - f, 1.0, 0.0],
        2 => [0.0, 1.0, f],
        3 => [0.0, 1.0 - f, 1.0],
        4 => [f, 0.0, 1.0],
        _ => [1.0, 0.0, 1.0 - f],
    };
    let white = saturation as f32 / 255.0;
    pure.map(|c| c + (1.0 - c) * white)
}

impl Light {
    /// The light's colour (0..1 per channel), not including brightness.
    pub fn color(&self) -> [f32; 3] {
        hue_saturation_to_rgb(self.hue, self.saturation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_wheel() {
        assert_eq!(hue_saturation_to_rgb(0, 0), [1.0, 0.0, 0.0]);
        let g = hue_saturation_to_rgb(85, 0);
        assert!(g[1] == 1.0 && g[0] < 0.02 && g[2] == 0.0, "{g:?}");
        let b = hue_saturation_to_rgb(171, 0);
        assert!(b[2] == 1.0 && b[0] < 0.02 && b[1] < 0.02, "{b:?}");
        // Saturation 255 is white whatever the hue.
        assert_eq!(hue_saturation_to_rgb(123, 255), [1.0, 1.0, 1.0]);
    }

    #[test]
    fn light_classes() {
        assert!(is_light_class("Light"));
        assert!(is_light_class("Spotlight"));
        assert!(is_light_class("K_POWSpotLight"));
        assert!(!is_light_class("LightMapTexture"));
        assert!(!is_light_class("StaticMeshActor"));
    }
}
