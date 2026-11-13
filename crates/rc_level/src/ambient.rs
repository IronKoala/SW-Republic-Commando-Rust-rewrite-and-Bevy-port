//! Sounds that loop where the map puts them: an actor's `AmbientSound`.
//!
//! In Unreal every actor can carry an `AmbientSound` (engine.u: `var(Sound)
//! sound AmbientSound;`), a sound that loops for as long as the actor is
//! there. Maps use it two ways:
//!
//! - an `AmbientSound` actor (a speaker icon in the editor) the designer
//!   gave a sound: geo_01a's battle in the distance,
//!   `GEO_Sound.ambPS_GEO_battle_lp_01`;
//! - an actor whose class gives one: the `Fire_Medium` emitter crackles
//!   (`ambPS_GEN_fireMd_lp_01`), `WhiteSmoke` hisses
//!   (`ambPS_GEN_steam_lp_05`), both in `Properties/CTEffects.u`.
//!
//! The map's own value wins over the class's. How far each carries is the
//! sound's own settings (`AudioParams`), read where it is played.
//!
//! Left out: movers (their `AmbientSound` is switched on while they move,
//! from `MoveAmbientSound`, see `crate::movers`) and characters (`Pawn`:
//! the game sets theirs as they act). Plain Rust, no Bevy.

use crate::actors::{self, LevelInfo};
use crate::class_look::{object_path, ClassLooks, Loaded};
use rc_package::properties::{self, Value};
use rc_package::Package;

/// One looping sound with a place in the map.
#[derive(Debug, Clone, PartialEq)]
pub struct AmbientSound {
    /// Export index of the actor.
    pub export: usize,
    pub class: String,
    /// The actor's name (`AmbientSound0`, `Fire_Medium3`).
    pub name: String,
    /// Unreal units.
    pub location: [f32; 3],
    /// Full path of the sound, package first:
    /// `["GEO_Sound", "ambPS_GEO_battle_lp_01"]`.
    pub sound: Vec<String>,
    /// Whether the value came from the map (`false`: from the class).
    pub from_map: bool,
    /// The actor's `SoundOcclusion` (engine.u `ESoundOcclusion`): 0
    /// `OCCLUSION_Default` and 2 `OCCLUSION_BSP` let the level's walls
    /// muffle it, 1 `OCCLUSION_None` never, 3 `OCCLUSION_StaticMeshes`
    /// also checks placed meshes. Map value, else the class's, else 0.
    pub sound_occlusion: u8,
}

/// Every actor in `level` with an ambient sound of its own or its class's.
pub fn read(
    looks: &mut ClassLooks,
    map: &Package,
    data: &[u8],
    level: &LevelInfo,
    open: &mut dyn FnMut(&str) -> Option<Loaded>,
) -> Vec<AmbientSound> {
    let mut out = Vec::new();
    for &a in level.actors.iter().flatten() {
        let Ok(props) = actors::read_actor_properties(map, data, a) else {
            continue;
        };
        let look = looks.for_actor(map, a, open);
        if look
            .as_ref()
            .is_some_and(|l| l.is_a("Mover") || l.is_a("Pawn"))
        {
            continue;
        }
        let own = match properties::find(map, &props, "AmbientSound").map(|p| &p.value) {
            Some(Value::Object(o)) => Some(*o),
            _ => None,
        };
        let (sound, from_map) = match own {
            // Set to None in the map: no sound, whatever the class says.
            Some(0) => continue,
            Some(o) => (object_path(map, o), true),
            None => {
                let Some((pkg, p)) = look.as_ref().and_then(|l| l.find("AmbientSound", 0)) else {
                    continue;
                };
                match p.value {
                    Value::Object(o) if o != 0 => (object_path(&pkg.0, o), false),
                    _ => continue,
                }
            }
        };
        if sound.len() < 2 {
            continue;
        }
        let Ok(place) = actors::read_placement(map, data, a) else {
            continue;
        };
        let sound_occlusion =
            match properties::find(map, &props, "SoundOcclusion").map(|p| &p.value) {
                Some(Value::Byte(b)) => *b,
                _ => match look.as_ref().and_then(|l| l.find("SoundOcclusion", 0)) {
                    Some((_, p)) => match p.value {
                        Value::Byte(b) => b,
                        _ => 0,
                    },
                    None => 0,
                },
            };
        out.push(AmbientSound {
            export: a,
            class: place.class,
            name: place.name,
            location: place.location,
            sound,
            from_map,
            sound_occlusion,
        });
    }
    out
}
