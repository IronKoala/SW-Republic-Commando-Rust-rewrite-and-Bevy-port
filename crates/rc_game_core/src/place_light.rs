//! Light by place: how bright things that move are drawn, by where they
//! are in the level.
//!
//! The level's surfaces carry the light the game baked into them. Squad
//! mates, droids, the first-person gun and the helmet frame have none, and
//! the original lights them by where they stand: dim in a dark tunnel,
//! bright out in the sun. The level part (`rc_level_plugin`) builds a light
//! probe from the baked light and puts it here as [`PlaceLight`]; any part
//! puts [`LitByPlace`] on an entity whose model should follow it, and the
//! level part keeps that model's materials matched to the light around it.
//!
//! Light here is on the game's own brightness scale: 1.0 shows a texture as
//! stored, 0.5 at half, and baked light reaches up to 2.0.

use bevy::prelude::*;
use std::sync::Arc;

/// Something that can say how much light reaches a point.
pub trait LightAt: Send + Sync {
    /// The light at `p` (Bevy position), red, green, blue, or `None` when
    /// no lit surface is near enough to tell.
    fn light_at(&self, p: Vec3) -> Option<Vec3>;
}

/// The current level's light, for parts that draw things that move. Not
/// there between levels or when the level's look couldn't be built.
#[derive(Resource, Clone)]
pub struct PlaceLight(pub Arc<dyn LightAt>);

impl PlaceLight {
    pub fn at(&self, p: Vec3) -> Option<Vec3> {
        self.0.light_at(p)
    }
}

/// Which part of an entity's materials the place's light sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightTarget {
    /// Lit models (squad mates, droids): the glow they give off stands in
    /// for the light around them; real-time lights still add on top.
    Emissive,
    /// Unlit models (the first-person gun, the helmet frame): the colour
    /// the texture is multiplied by.
    BaseColor,
}

/// Put on an entity (or the parent of its meshes) to draw it by the light
/// where it is. Its materials (its own and its descendants') become its
/// own copies the first time, so each character gets its own light.
///
/// The brightness drawn at is `own + scale x light`, on the game's scale
/// (see the module notes), worked out at `height` Bevy units above the
/// entity's position and eased in over a fraction of a second.
#[derive(Component, Debug, Clone, Copy)]
pub struct LitByPlace {
    pub target: LightTarget,
    /// Light the thing gives itself (the helmet's lights inside), 0 for none.
    pub own: f32,
    /// How strongly the place's light shows on it.
    pub scale: f32,
    /// Where to measure, above the entity's position (Bevy units).
    pub height: f32,
    /// The brightness its materials were made with (0-1, the same scale),
    /// so it can be swapped for the place's light.
    pub made_at: f32,
}

impl LitByPlace {
    /// The brightness to draw at for `light`, per colour, on the game's
    /// scale, kept between 0 and 2.
    pub fn brightness(&self, light: Vec3) -> Vec3 {
        (Vec3::splat(self.own) + light * self.scale).clamp(Vec3::ZERO, Vec3::splat(2.0))
    }

    /// How much to multiply the materials' made colour by (linear light)
    /// to draw at `brightness` instead of `made_at`. The game's scale is
    /// roughly "gamma 2.2", as the level's drawing assumes.
    pub fn factor(&self, brightness: Vec3) -> Vec3 {
        let made = self.made_at.max(0.01).powf(2.2);
        Vec3::new(
            brightness.x.powf(2.2),
            brightness.y.powf(2.2),
            brightness.z.powf(2.2),
        ) / made
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_brightness_as_made_changes_nothing() {
        let l = LitByPlace {
            target: LightTarget::BaseColor,
            own: 0.0,
            scale: 1.0,
            height: 0.0,
            made_at: 0.75,
        };
        let f = l.factor(l.brightness(Vec3::splat(0.75)));
        assert!((f - Vec3::ONE).abs().max_element() < 1e-5, "{f}");
    }

    #[test]
    fn own_light_adds_and_is_capped() {
        let l = LitByPlace {
            target: LightTarget::BaseColor,
            own: 0.5,
            scale: 1.0,
            height: 0.0,
            made_at: 0.5,
        };
        assert_eq!(l.brightness(Vec3::ZERO), Vec3::splat(0.5));
        assert_eq!(l.brightness(Vec3::splat(9.0)), Vec3::splat(2.0));
    }
}
