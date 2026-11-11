//! Distance fog, as each zone of the map sets it.
//!
//! Every Unreal map is split into zones (by `ZoneInfo` actors; the
//! `LevelInfo` covers the rest), and each zone can turn on distance fog:
//! `bDistanceFog`, `DistanceFogColor`, `DistanceFogStart` and
//! `DistanceFogEnd` (Unreal units), plus `DistanceFogBlendTime`. The values
//! a map doesn't store come from the `ZoneInfo` class defaults in the
//! game's Engine.u (read at run time: start 3000, end 8000, colour
//! 128,128,128, blend time 1 second).
//!
//! geo_01a's arena zone (`ZoneInfo12`) stores fog on, colour 142,119,94
//! (a sandy brown) and end 64000, with the start left at the default 3000.
//! That is the brown haze over the far arena tiers in the original.
//!
//! How the game uses them (read from its `D3DDrv.dll`, not copied): the
//! fog is plain linear Direct3D fog (`D3DRS_FOGTABLEMODE` or
//! `D3DRS_FOGVERTEXMODE` set to `D3DFOG_LINEAR`), so a surface at distance
//! d keeps `(end - d) / (end - start)` of its own colour (0 to 1) and the
//! rest is fog colour. Bevy's `FogFalloff::Linear` is the same formula.
//!
//! What we can't read from the files, so it is our choice, said here:
//! - The fog used is the one of the zone the camera is in. (`ZoneInfo`
//!   has `DistanceFogBlendTime`, which only makes sense if the fog changes
//!   as the viewer moves between zones.) When the zone changes we blend
//!   the numbers over that zone's blend time, in a straight line.
//! - The sky box (seen by its own camera) gets no fog: geo_01a's
//!   `SkyZoneInfo` doesn't turn it on.
//! - Direct3D mixes the fog in on screen colours; Bevy mixes it in linear
//!   light, before its tone mapping, so half-fogged surfaces may come out a
//!   little different.
//! - `bClearToFogColor` (also set on geo_01a's arena) isn't used.

use crate::materials::Materials;
use rc_level::actors;
use rc_level::bsp::Bsp;
use rc_package::class_library::{ClassLibrary, DefaultValue};
use rc_package::properties::{self, Property, Value};
use rc_package::{light, Package};
use std::collections::HashMap;

/// One zone's fog, in Unreal units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoneFog {
    /// Red, green, blue (0-255), as on screen.
    pub colour: [u8; 3],
    pub start: f32,
    pub end: f32,
    /// Seconds to blend to this zone's fog when the camera enters it.
    pub blend_time: f32,
}

impl ZoneFog {
    /// How much of a surface's own colour is left at distance `d` (Unreal
    /// units): 1 = no fog, 0 = all fog colour. Direct3D's linear fog.
    pub fn visibility(&self, d: f32) -> f32 {
        if (self.end - self.start).abs() < 1e-3 {
            return if d < self.end { 1.0 } else { 0.0 };
        }
        ((self.end - d) / (self.end - self.start)).clamp(0.0, 1.0)
    }
}

/// The values a zone uses when the map doesn't store them: the `ZoneInfo`
/// class defaults in Engine.u.
#[derive(Debug, Clone, Copy)]
pub struct FogDefaults {
    pub enabled: bool,
    pub colour: [u8; 3],
    pub start: f32,
    pub end: f32,
    pub blend_time: f32,
}

impl FogDefaults {
    /// Read from the game's Engine.u (and Core.u, its parent classes).
    pub fn read(mats: &mut Materials) -> Result<Self, String> {
        let mut lib = ClassLibrary::new();
        for name in ["Core", "Engine"] {
            let p = mats
                .package(name)
                .ok_or_else(|| format!("{name}.u not found"))?;
            lib.add_package(name, p.1.clone())
                .map_err(|e| format!("{name}.u: {e}"))?;
        }
        let d = lib.defaults("ZoneInfo");
        let colour = match d.get("DistanceFogColor").map(|e| &e.value) {
            Some(DefaultValue::Raw(b)) if b.len() >= 4 => {
                let c = rc_image::unreal_color([b[0], b[1], b[2], b[3]]);
                [c[0], c[1], c[2]]
            }
            _ => return Err("ZoneInfo has no DistanceFogColor default".into()),
        };
        Ok(Self {
            enabled: d.bool("bDistanceFog").unwrap_or(false),
            colour,
            start: d
                .number("DistanceFogStart")
                .ok_or("ZoneInfo has no DistanceFogStart default")?,
            end: d
                .number("DistanceFogEnd")
                .ok_or("ZoneInfo has no DistanceFogEnd default")?,
            blend_time: d.number("DistanceFogBlendTime").unwrap_or(0.0),
        })
    }
}

/// One BSP node, just what finding a point's zone needs.
#[derive(Debug, Clone, Copy)]
struct ZoneNode {
    plane: [f32; 4],
    front: i32,
    back: i32,
    zone: [u8; 2],
}

/// Every zone's fog and the map's zone tree, to look up the fog wherever
/// the camera is.
#[derive(Debug, Clone, Default)]
pub struct ZoneFogs {
    nodes: Vec<ZoneNode>,
    /// Fog by BSP zone number; zones with fog off are absent.
    by_zone: HashMap<u8, ZoneFog>,
    /// Zones (with a `ZoneInfo`, or the `LevelInfo`) that turn fog on.
    pub zones_with_fog: usize,
    /// Zones whose settings were read.
    pub zones_read: usize,
}

impl ZoneFogs {
    /// Read every zone's fog from the map.
    pub fn read(
        pkg: &Package,
        data: &[u8],
        level: &actors::LevelInfo,
        bsp: &Bsp,
        defaults: &FogDefaults,
    ) -> Self {
        let mut out = ZoneFogs {
            nodes: bsp
                .nodes
                .iter()
                .map(|n| ZoneNode {
                    plane: n.plane,
                    front: n.front,
                    back: n.back,
                    zone: n.zone,
                })
                .collect(),
            ..Default::default()
        };
        let mut level_fog = None;
        for &a in level.actors.iter().flatten() {
            let class = pkg.export_class_name(&pkg.exports[a]).to_ascii_lowercase();
            let is_level = class == "levelinfo";
            if !is_level && !class.contains("zoneinfo") {
                continue;
            }
            let Ok(props) = actors::read_actor_properties(pkg, data, a) else {
                continue;
            };
            out.zones_read += 1;
            let fog = zone_fog(pkg, &props, defaults);
            out.zones_with_fog += fog.is_some() as usize;
            if is_level {
                level_fog = Some(fog);
            } else if let Some(zone) = bsp.point_zone(light::location(pkg, &props)) {
                if let Some(f) = fog {
                    out.by_zone.insert(zone, f);
                }
            }
        }
        // Zones without a ZoneInfo of their own take the LevelInfo's.
        if let Some(Some(f)) = level_fog {
            let zones: Vec<u8> = bsp.nodes.iter().flat_map(|n| n.zone).collect();
            for z in zones {
                out.by_zone.entry(z).or_insert(f);
            }
        }
        out
    }

    /// The zone a point (Unreal units) is in, as `Bsp::point_zone`.
    pub fn zone_at(&self, p: [f32; 3]) -> Option<u8> {
        let mut i = 0usize;
        for _ in 0..self.nodes.len() {
            let n = self.nodes.get(i)?;
            let d = n.plane[0] * p[0] + n.plane[1] * p[1] + n.plane[2] * p[2] - n.plane[3];
            let front = d >= 0.0;
            let child = if front { n.front } else { n.back };
            if child < 0 || child as usize >= self.nodes.len() {
                return Some(n.zone[front as usize]);
            }
            i = child as usize;
        }
        None
    }

    /// The fog at a point (Unreal units): its zone's, `None` where the
    /// zone has fog off.
    pub fn at(&self, p: [f32; 3]) -> (Option<u8>, Option<ZoneFog>) {
        let zone = self.zone_at(p);
        (zone, zone.and_then(|z| self.by_zone.get(&z).copied()))
    }

    /// The fog of one zone.
    pub fn zone(&self, zone: u8) -> Option<ZoneFog> {
        self.by_zone.get(&zone).copied()
    }
}

fn zone_fog(pkg: &Package, props: &[Property], d: &FogDefaults) -> Option<ZoneFog> {
    let find = |n: &str| properties::find(pkg, props, n).map(|p| &p.value);
    let enabled = match find("bDistanceFog") {
        Some(Value::Bool(b)) => *b,
        _ => d.enabled,
    };
    if !enabled {
        return None;
    }
    let float = |n: &str, default: f32| match find(n) {
        Some(Value::Float(f)) => *f,
        _ => default,
    };
    let colour = match find("DistanceFogColor") {
        Some(Value::Raw(b)) if b.len() >= 4 => {
            let c = rc_image::unreal_color([b[0], b[1], b[2], b[3]]);
            [c[0], c[1], c[2]]
        }
        _ => d.colour,
    };
    Some(ZoneFog {
        colour,
        start: float("DistanceFogStart", d.start),
        end: float("DistanceFogEnd", d.end),
        blend_time: float("DistanceFogBlendTime", d.blend_time),
    })
}

/// Blends from one fog to another as the camera changes zone.
#[derive(Debug, Clone, Default)]
pub struct FogBlend {
    zone: Option<u8>,
    from: Option<ZoneFog>,
    to: Option<ZoneFog>,
    /// Seconds into the blend.
    t: f32,
}

impl FogBlend {
    /// Move on by `dt` seconds with the camera in `zone` (whose fog is
    /// `fog`), and return the fog to draw now. The first call takes the
    /// zone's fog at once.
    pub fn step(&mut self, zone: Option<u8>, fog: Option<ZoneFog>, dt: f32) -> Option<ZoneFog> {
        if zone != self.zone {
            let first = self.zone.is_none() && self.to.is_none() && self.from.is_none();
            self.from = if first { fog } else { self.current() };
            self.to = fog;
            self.zone = zone;
            self.t = 0.0;
        }
        self.t += dt.max(0.0);
        self.current()
    }

    /// The zone whose fog is being blended in.
    pub fn zone(&self) -> Option<u8> {
        self.zone
    }

    fn current(&self) -> Option<ZoneFog> {
        let time = self.to.map_or(0.0, |f| f.blend_time);
        let k = if time <= 0.0 {
            1.0
        } else {
            (self.t / time).clamp(0.0, 1.0)
        };
        if k >= 1.0 {
            return self.to;
        }
        // Fog off is fog that never starts: blend to or from that.
        let off = |other: ZoneFog| ZoneFog {
            start: 1.0e7,
            end: 1.0e7 + 1.0,
            ..other
        };
        let (a, b) = match (self.from, self.to) {
            (Some(a), Some(b)) => (a, b),
            (Some(a), None) => (a, off(a)),
            (None, Some(b)) => (off(b), b),
            (None, None) => return None,
        };
        let mix = |x: f32, y: f32| x + (y - x) * k;
        Some(ZoneFog {
            colour: [0, 1, 2].map(|i| mix(a.colour[i] as f32, b.colour[i] as f32).round() as u8),
            start: mix(a.start, b.start),
            end: mix(a.end, b.end),
            blend_time: b.blend_time,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fog(start: f32, end: f32) -> ZoneFog {
        ZoneFog {
            colour: [142, 119, 94],
            start,
            end,
            blend_time: 1.0,
        }
    }

    #[test]
    fn linear_like_direct3d() {
        let f = fog(3000.0, 64000.0);
        assert_eq!(f.visibility(0.0), 1.0);
        assert_eq!(f.visibility(3000.0), 1.0);
        assert!((f.visibility(33500.0) - 0.5).abs() < 1e-4);
        assert_eq!(f.visibility(64000.0), 0.0);
        assert_eq!(f.visibility(90000.0), 0.0);
    }

    #[test]
    fn blends_over_the_new_zones_time() {
        let mut b = FogBlend::default();
        // First zone: taken at once.
        assert_eq!(
            b.step(Some(1), Some(fog(3000.0, 64000.0)), 0.016),
            Some(fog(3000.0, 64000.0))
        );
        // Into a zone with nearer fog: halfway after half a second.
        let half = b.step(Some(2), Some(fog(1000.0, 8000.0)), 0.5).unwrap();
        assert!((half.start - 2000.0).abs() < 1.0, "{half:?}");
        assert!((half.end - 36000.0).abs() < 1.0, "{half:?}");
        let done = b.step(Some(2), Some(fog(1000.0, 8000.0)), 0.6).unwrap();
        assert_eq!(done, fog(1000.0, 8000.0));
        // Into a zone with fog off: none once blended.
        b.step(Some(3), None, 0.0);
        assert_eq!(b.step(Some(3), None, 2.0), None);
    }
}

/// The Bevy side: the map's zone fogs as a resource, and a system that
/// puts the right one on every camera marked [`ZoneFogView`].
pub mod view {
    use super::{FogBlend, ZoneFog, ZoneFogs};
    use bevy::ecs::query::QueryFilter;
    use bevy::pbr::{DistanceFog, FogFalloff};
    use bevy::prelude::*;

    /// Unreal units per Bevy unit (`scene::to_bevy`).
    const UNITS: f32 = 50.0;

    /// The loaded map's zone fogs. Remove it when the level goes.
    #[derive(Resource, Clone, Default)]
    pub struct LevelFog(pub ZoneFogs);

    /// Put on a camera that should show the fog of the zone it's in.
    #[derive(Component, Default)]
    pub struct ZoneFogView {
        blend: FogBlend,
    }

    impl ZoneFogView {
        /// The zone whose fog is shown (or being blended in).
        pub fn zone(&self) -> Option<u8> {
            self.blend.zone()
        }
    }

    /// Bevy's fog for one zone's (Unreal units -> Bevy units).
    pub fn distance_fog(f: &ZoneFog) -> DistanceFog {
        DistanceFog {
            color: Color::srgb_u8(f.colour[0], f.colour[1], f.colour[2]),
            falloff: FogFalloff::Linear {
                start: f.start / UNITS,
                end: f.end / UNITS,
            },
            ..default()
        }
    }

    /// Adds [`follow_zone_fog`] for every [`ZoneFogView`] camera (the
    /// viewer). The game's level plugin adds its own filtered copy instead.
    pub struct ZoneFogPlugin;

    impl Plugin for ZoneFogPlugin {
        fn build(&self, app: &mut App) {
            app.add_systems(
                PostUpdate,
                follow_zone_fog::<()>.after(TransformSystems::Propagate),
            );
        }
    }

    /// Every frame: find each fog camera's zone and set its fog. `F`
    /// filters the cameras further: the game passes
    /// `Without<rc_game_core::OwnFog>` so a mode with fog of its own (the
    /// low light visor) keeps it (`ZoneFogPlugin` passes `()`).
    pub fn follow_zone_fog<F: QueryFilter>(
        mut commands: Commands,
        time: Res<Time>,
        fogs: Option<Res<LevelFog>>,
        mut views: Query<
            (
                Entity,
                &GlobalTransform,
                &mut ZoneFogView,
                Option<&mut DistanceFog>,
            ),
            F,
        >,
    ) {
        let Some(fogs) = fogs else {
            return;
        };
        for (e, at, mut view, current) in &mut views {
            let b = at.translation();
            // Bevy (x, y up, z) -> Unreal (x, y, z up).
            let p = [b.x * UNITS, b.z * UNITS, b.y * UNITS];
            let (zone, fog) = fogs.0.at(p);
            match view.blend.step(zone, fog, time.delta_secs()) {
                Some(f) => {
                    let want = distance_fog(&f);
                    match current {
                        Some(mut c) => {
                            c.color = want.color;
                            c.falloff = want.falloff;
                        }
                        None => {
                            commands.entity(e).insert(want);
                        }
                    }
                }
                None => {
                    if current.is_some() {
                        commands.entity(e).remove::<DistanceFog>();
                    }
                }
            }
        }
    }
}
