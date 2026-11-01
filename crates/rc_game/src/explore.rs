//! Explore mode: for walking every level to compare it with the original
//! game. Not part of the original.
//!
//! - `explore_mode` (F6) turns it on or off (or start with `--explore`).
//!   While it's on the player takes no damage (`rc_combat::Invulnerable`)
//!   and enemies don't notice them (`rc_game_core::Unseen`).
//! - `explore_fly` (F7): fly through walls, where you look. Jump goes up,
//!   crouch goes down, walk (Shift) goes slower. Off again, you fall from
//!   where you are.
//! - `next_level` / `prev_level` (Page Down / Page Up): straight to the
//!   next or previous campaign map, through the same level change the
//!   game's exits use. The order is the original's own campaign list
//!   (`GameEngine.ProgressLevels` in `engine.u`, 56 maps from the prologue
//!   through the briefings to the epilogue), read when explore mode first
//!   needs it; without it, our level list's order.
//!
//! The fly and level keys only work while explore mode is on, so they can't
//! be pressed by accident in a normal game. Every change is logged, and
//! each screenshot logs where it was taken (`screenshot.rs`), so the same
//! spot can be found in the original.
//!
//! The keys live in `data/input/bindings.toml` like every other action; the
//! fly speeds are below.

use crate::levels::LevelList;
use crate::player::Body;
use bevy::prelude::*;
use rc_game_core::{
    playing, Action, ActionState, ChangeLevel, CurrentLevel, GameLog, GamePaths, GameSet,
    GameState, HudMessage, Player, Unseen,
};

/// Flying speed, Unreal units a second (running is 450).
pub const FLY_SPEED: f32 = 1500.0;
/// Flying speed while holding walk.
pub const FLY_SLOW_SPEED: f32 = 300.0;

/// Whether explore mode and flying are on. Kept from map to map.
#[derive(Resource, Debug, Default, Clone)]
pub struct Explore {
    pub on: bool,
    pub fly: bool,
    /// The player was made unhurtable by explore mode (not by a level
    /// script), so turning explore mode off takes it away again.
    added_invulnerable: bool,
    /// The campaign maps in order, read on first use.
    order: Option<Vec<String>>,
}

impl Explore {
    pub fn starting(on: bool) -> Self {
        Explore {
            on,
            ..Default::default()
        }
    }
}

/// The original's campaign order: `GameEngine`'s `ProgressLevels` default
/// values in `engine.u` (lower case map names), or `None` if unreadable.
pub fn original_order(paths: &GamePaths) -> Option<Vec<String>> {
    let data = std::fs::read(paths.game_file("GameData/System/engine.u")).ok()?;
    let mut lib = rc_package::class_library::ClassLibrary::new();
    lib.add_package("engine", data).ok()?;
    let mut levels: Vec<(u32, String)> = lib
        .defaults("GameEngine")
        .entries
        .into_iter()
        .filter(|e| e.name.eq_ignore_ascii_case("ProgressLevels"))
        .filter_map(|e| match e.value {
            rc_package::class_library::DefaultValue::Raw(b) => {
                string_value(&b).map(|s| (e.index, s.to_ascii_lowercase()))
            }
            _ => None,
        })
        .collect();
    levels.sort();
    let names: Vec<String> = levels.into_iter().map(|(_, n)| n).collect();
    (!names.is_empty()).then_some(names)
}

/// A stored string value: a length byte (counting the closing zero), the
/// letters, then a zero.
fn string_value(b: &[u8]) -> Option<String> {
    let (&len, rest) = b.split_first()?;
    let text = rest.get(..(len as usize).checked_sub(1)?)?;
    text.is_ascii()
        .then(|| String::from_utf8_lossy(text).into_owned())
}

pub struct ExplorePlugin {
    /// Start with explore mode on (`--explore`).
    pub start_on: bool,
}

impl Plugin for ExplorePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Explore::starting(self.start_on))
            .add_systems(
                Update,
                (keys, keep_player_safe)
                    .chain()
                    .in_set(GameSet::Simulate)
                    .before(crate::player::PlayerMoveSet)
                    .run_if(playing),
            )
            .add_systems(OnEnter(GameState::InGame), announce);
    }
}

fn say(text: impl Into<String>, hud: &mut MessageWriter<HudMessage>, log: &GameLog) {
    let text = text.into();
    log.line(format!("explore: {text}"));
    hud.write(HudMessage { text, seconds: 3.0 });
}

/// On each map, say when explore mode is on, so it's never on unnoticed.
fn announce(explore: Res<Explore>, mut hud: MessageWriter<HudMessage>, log: Res<GameLog>) {
    if explore.on {
        say(
            format!(
                "Explore mode on{} (F6 to turn off)",
                if explore.fly { ", flying" } else { "" }
            ),
            &mut hud,
            &log,
        );
    }
}

/// The campaign maps in order (the level list without "Other maps").
pub fn campaign_order(list: &LevelList) -> Vec<String> {
    list.campaigns
        .iter()
        .filter(|c| c.name != "Other maps")
        .flat_map(|c| c.levels.iter().map(|l| l.map.clone()))
        .collect()
}

/// The map `step` places after (or before, for -1) `current` in `order`.
/// A map not in the list goes to the first map.
pub fn neighbour(order: &[String], current: &str, step: i32) -> Option<String> {
    if order.is_empty() {
        return None;
    }
    let Some(i) = order.iter().position(|m| m.eq_ignore_ascii_case(current)) else {
        return Some(order[0].clone());
    };
    let j = i as i32 + step;
    if j < 0 || j >= order.len() as i32 {
        return None;
    }
    Some(order[j as usize].clone())
}

#[allow(clippy::too_many_arguments)]
fn keys(
    actions: Res<ActionState>,
    mut explore: ResMut<Explore>,
    list: Option<Res<LevelList>>,
    current: Res<CurrentLevel>,
    paths: Res<GamePaths>,
    body: Query<&Body, With<Player>>,
    mut travel: MessageWriter<ChangeLevel>,
    mut hud: MessageWriter<HudMessage>,
    log: Res<GameLog>,
) {
    if actions.just_pressed(Action::ExploreMode) {
        explore.on = !explore.on;
        if !explore.on {
            explore.fly = false;
        }
        let text = if explore.on {
            "Explore mode on: no damage, enemies ignore you. F7 fly, PgUp/PgDn change level"
        } else {
            "Explore mode off"
        };
        say(text, &mut hud, &log);
    }
    let wants_other = actions.just_pressed(Action::ExploreFly)
        || actions.just_pressed(Action::NextLevel)
        || actions.just_pressed(Action::PrevLevel);
    if !explore.on {
        if wants_other {
            say("Explore mode is off: press F6 first", &mut hud, &log);
        }
        return;
    }
    if actions.just_pressed(Action::ExploreFly) {
        explore.fly = !explore.fly;
        let at = body
            .single()
            .map(|b| {
                let f = b.pawn.feet();
                format!(" at ({:.0}, {:.0}, {:.0})", f.x, f.y, f.z)
            })
            .unwrap_or_default();
        say(
            if explore.fly {
                "Flying on"
            } else {
                "Flying off"
            },
            &mut hud,
            &log,
        );
        log.line(format!(
            "explore: flying {}{at}",
            if explore.fly { "on" } else { "off" }
        ));
    }
    for (action, step, word) in [
        (Action::NextLevel, 1, "next"),
        (Action::PrevLevel, -1, "previous"),
    ] {
        if !actions.just_pressed(action) {
            continue;
        }
        if explore.order.is_none() {
            let order = match original_order(&paths) {
                Some(o) => {
                    log.line(format!(
                        "explore: level order from engine.u ProgressLevels: {} maps",
                        o.len()
                    ));
                    o
                }
                None => {
                    let o = list.as_deref().map(campaign_order).unwrap_or_default();
                    log.line(format!(
                        "explore: engine.u's level order unreadable; using the level list ({} maps)",
                        o.len()
                    ));
                    o
                }
            };
            explore.order = Some(order);
        }
        let order = explore.order.clone().unwrap_or_default();
        let here = current.map.clone().unwrap_or_default();
        match neighbour(&order, &here, step) {
            Some(map) => {
                say(format!("Going to the {word} level: {map}"), &mut hud, &log);
                travel.write(ChangeLevel {
                    map,
                    portal: None,
                    by: format!("explore mode ({word} level key)"),
                });
            }
            None => say(format!("No {word} level after {here}"), &mut hud, &log),
        }
        break;
    }
}

/// While explore mode is on: the player can't be hurt and enemies don't
/// see them. Checked every frame, so a level script switching the player's
/// invulnerability off doesn't undo it.
fn keep_player_safe(
    mut commands: Commands,
    mut explore: ResMut<Explore>,
    player: Query<(Entity, Has<rc_combat::Invulnerable>, Has<Unseen>), With<Player>>,
) {
    let Ok((e, invulnerable, unseen)) = player.single() else {
        return;
    };
    if explore.on {
        if !invulnerable {
            commands.entity(e).insert(rc_combat::Invulnerable);
            explore.added_invulnerable = true;
        }
        if !unseen {
            commands.entity(e).insert(Unseen);
        }
    } else {
        if explore.added_invulnerable && invulnerable {
            commands.entity(e).remove::<rc_combat::Invulnerable>();
        }
        explore.added_invulnerable = false;
        if unseen {
            commands.entity(e).remove::<Unseen>();
        }
    }
}

/// How far to fly this frame, Unreal units: along the view for forward and
/// back, sideways for strafing, straight up or down for jump and crouch.
pub fn fly_step(
    yaw: f32,
    pitch: f32,
    forward: f32,
    strafe: f32,
    up: f32,
    slow: bool,
    dt: f32,
) -> rc_movement::Vec3 {
    use rc_movement::Vec3;
    let look = Vec3::new(
        yaw.cos() * pitch.cos(),
        yaw.sin() * pitch.cos(),
        pitch.sin(),
    );
    // Unreal's yaw turns from +X towards +Y, so +Y is to the right.
    let right = Vec3::new(-yaw.sin(), yaw.cos(), 0.0);
    let mut dir = look * forward + right * strafe + Vec3::new(0.0, 0.0, up);
    let len = dir.length();
    if len > 1.0 {
        dir = dir * (1.0 / len);
    }
    let speed = if slow { FLY_SLOW_SPEED } else { FLY_SPEED };
    dir * (speed * dt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_stored_strings() {
        assert_eq!(
            string_value(&[8, 89, 89, 89, 95, 48, 53, 69, 0]).as_deref(),
            Some("YYY_05E")
        );
        assert_eq!(string_value(&[]), None);
        assert_eq!(string_value(&[0]), None);
    }

    #[test]
    fn neighbours_follow_the_list() {
        let order: Vec<String> = ["pro", "geo_01a", "geo_01b"].map(String::from).to_vec();
        assert_eq!(neighbour(&order, "geo_01a", 1).as_deref(), Some("geo_01b"));
        assert_eq!(neighbour(&order, "GEO_01A", -1).as_deref(), Some("pro"));
        assert_eq!(neighbour(&order, "geo_01b", 1), None);
        assert_eq!(neighbour(&order, "pro", -1), None);
        assert_eq!(neighbour(&order, "ctf_siege", 1).as_deref(), Some("pro"));
    }

    #[test]
    fn flying_goes_where_you_look() {
        // Facing +X, level: forward is +X.
        let d = fly_step(0.0, 0.0, 1.0, 0.0, 0.0, false, 1.0);
        assert!((d.x - FLY_SPEED).abs() < 0.01 && d.y.abs() < 0.01 && d.z.abs() < 0.01);
        // Looking straight up: forward is up.
        let d = fly_step(0.0, std::f32::consts::FRAC_PI_2, 1.0, 0.0, 0.0, false, 1.0);
        assert!((d.z - FLY_SPEED).abs() < 0.1);
        // Facing +X, strafing right goes to +Y (Unreal's right).
        let d = fly_step(0.0, 0.0, 0.0, 1.0, 0.0, true, 1.0);
        assert!((d.y - FLY_SLOW_SPEED).abs() < 0.01);
        // Diagonals are no faster.
        let d = fly_step(0.0, 0.0, 1.0, 1.0, 1.0, false, 1.0);
        assert!((d.length() - FLY_SPEED).abs() < 0.1);
    }
}
