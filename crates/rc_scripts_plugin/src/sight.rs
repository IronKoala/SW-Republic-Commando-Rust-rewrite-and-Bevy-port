//! Line-of-sight triggers (`LineOfSightTrigger`): a level waits for the
//! player to *see* something (a droid army over a ridge, a general on a
//! tower) and its event sets the next scene going.
//!
//! The director (`rc_scripts`) knows each trigger's place, distance and
//! angle. Each frame this asks it which ones the view from the player's
//! camera points at, checks nothing in the level is in the way with a line
//! through the level's collision (walls, doors), and tells it which were
//! seen. Logged with the `scripts:` prefix.

use crate::{LevelScript, PART};
use bevy::prelude::*;
use rc_combat::{Dead, Hittable};
use rc_game_core::units::to_unreal;
use rc_game_core::{GameLog, LevelCollision, Player, PlayerCamera, ScriptTags};
use rc_movement::Vec3 as UVec3;
use rc_scripts::sight::clear_line;

/// How far above its feet a character's middle is, uu, when it has no
/// size of its own (a cutscene character): the commando's.
const DEFAULT_CENTRE_UP: f32 = 84.0;

/// Characters a trigger may watch: their tag, where they are, their size,
/// and whether they died.
type Watched<'w, 's> = Query<
    'w,
    's,
    (
        &'static ScriptTags,
        &'static GlobalTransform,
        Option<&'static Hittable>,
        Has<Dead>,
    ),
    Without<Player>,
>;

/// Check the level's line-of-sight triggers against the player's view.
pub fn look(
    script: Option<ResMut<LevelScript>>,
    collision: Option<Res<LevelCollision>>,
    player: Query<Has<Dead>, With<Player>>,
    camera: Query<&GlobalTransform, With<PlayerCamera>>,
    characters: Watched,
    log: Res<GameLog>,
) {
    let Some(mut script) = script else { return };
    // A dead player sees nothing; no player, no view.
    match player.single() {
        Ok(false) => {}
        _ => return,
    }
    let Ok(cam) = camera.single() else { return };
    let eye = to_unreal(cam.translation());
    let f = cam.forward();
    // Bevy direction to Unreal: swap Y and Z (no scaling for a direction).
    let forward = UVec3::new(f.x, f.z, f.y).normalize_or_zero();
    // Where a watched character is now: the middle of its body. A living
    // one first; one that died still counts (its body is still there).
    let live = |tag: &str| -> Option<[f32; 3]> {
        let mut best: Option<([f32; 3], bool)> = None;
        for (tags, t, hit, dead) in &characters {
            let Some(own) = &tags.tag else { continue };
            if !own.eq_ignore_ascii_case(tag) {
                continue;
            }
            let p = to_unreal(t.translation());
            let up = hit.map_or(DEFAULT_CENTRE_UP, |h| h.center_up);
            let at = [p.x, p.y, p.z + up];
            if best.is_none_or(|(_, was_dead)| was_dead && !dead) {
                best = Some((at, dead));
            }
        }
        best.map(|(at, _)| at)
    };
    let candidates = script.director.sight_candidates_live(
        [eye.x, eye.y, eye.z],
        [forward.x, forward.y, forward.z],
        &live,
    );
    if candidates.is_empty() {
        return;
    }
    let script = &mut *script;
    for (i, at) in candidates {
        let at = UVec3::new(at[0], at[1], at[2]);
        let ok = collision
            .as_ref()
            .is_none_or(|c| clear_line(c.0.as_ref(), eye, at));
        if !ok {
            continue;
        }
        let name = script
            .director
            .sight_triggers()
            .get(i)
            .map(|t| t.0.clone())
            .unwrap_or_default();
        log.line(format!(
            "{PART}: t={:.2} the player sees what {name} watches for: from ({:.0}, {:.0}, {:.0}), {:.0} uu away",
            script.director.time,
            eye.x,
            eye.y,
            eye.z,
            (at - eye).length()
        ));
        script.director.seen(i);
        script.stats.sight_seen.push(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rc_game_core::units::to_bevy;
    use rc_level::scripts::{MapScripts, ScriptActor, ScriptObject, Val};
    use rc_scripts::{Director, Settings};
    use std::sync::Arc;

    fn actor(
        export: usize,
        class: &str,
        name: &str,
        vals: &[(&str, &str)],
        at: [f32; 3],
    ) -> ScriptActor {
        let mut values = rc_level::scripts::Vals::new();
        for (k, v) in vals {
            values.insert(k.to_string(), Val::Text(v.to_string()));
        }
        ScriptActor {
            obj: ScriptObject {
                export,
                class: class.into(),
                name: name.into(),
                values,
            },
            location: at,
            actions: vec![],
        }
    }

    /// The game with one trigger at Unreal (1000, 0, 0) watching a
    /// character placed there, a player at the origin and a camera
    /// looking at Unreal point `target` from Unreal point `eye`.
    fn game(eye: [f32; 3], target: [f32; 3], wall: bool, moved_to: Option<[f32; 3]>) -> App {
        let map = MapScripts {
            actors: vec![
                actor(
                    1,
                    "LineOfSightTrigger",
                    "LineOfSightTrigger0",
                    &[("Event", "SeeTarful"), ("SeenActorTag", "TarfulScreen")],
                    [900.0, 0.0, 0.0],
                ),
                actor(
                    2,
                    "StaticProp",
                    "StaticProp1",
                    &[("Tag", "TarfulScreen")],
                    [1000.0, 0.0, 0.0],
                ),
            ],
            ..Default::default()
        };
        let mut app = App::new();
        app.insert_resource(GameLog::terminal());
        app.insert_resource(LevelScript::bare(Director::new(
            &map,
            &[],
            Settings::default(),
        )));
        if wall {
            // A wall across Unreal X = 500.
            let p = |y: f32, z: f32| UVec3::new(500.0, y, z);
            let w = rc_movement::TriangleWorld::new(vec![
                [p(-2000.0, -2000.0), p(2000.0, -2000.0), p(2000.0, 2000.0)],
                [p(-2000.0, -2000.0), p(2000.0, 2000.0), p(-2000.0, 2000.0)],
            ]);
            app.insert_resource(LevelCollision(Arc::new(w)));
        }
        let b = |p: [f32; 3]| to_bevy(UVec3::new(p[0], p[1], p[2]));
        app.world_mut().spawn((Player, Transform::default()));
        app.world_mut().spawn((
            PlayerCamera,
            GlobalTransform::from(
                Transform::from_translation(b(eye)).looking_at(b(target), Vec3::Y),
            ),
        ));
        if let Some(at) = moved_to {
            // The watched thing, moved by the game (feet; its middle is
            // 84 uu up).
            let feet = [at[0], at[1], at[2] - DEFAULT_CENTRE_UP];
            app.world_mut().spawn((
                ScriptTags {
                    tag: Some("TarfulScreen".into()),
                    script: None,
                    follows_orders: false,
                },
                GlobalTransform::from(Transform::from_translation(b(feet))),
            ));
        }
        app.add_systems(Update, look);
        app
    }

    fn seen(app: &mut App) -> Vec<String> {
        app.update();
        app.world()
            .resource::<LevelScript>()
            .stats
            .sight_seen
            .clone()
    }

    #[test]
    fn looking_at_it_sets_it_off() {
        let mut app = game([0.0; 3], [1000.0, 0.0, 0.0], false, None);
        assert_eq!(seen(&mut app), vec!["LineOfSightTrigger0".to_string()]);
        // Its event goes out with the next step of the scripts.
        let mut s = app.world_mut().resource_mut::<LevelScript>();
        s.director.step(0.01, None);
        assert!(s.director.history.iter().any(|(_, e)| e == "SeeTarful"));
    }

    #[test]
    fn looking_away_or_through_a_wall_does_not() {
        // Looking along Unreal +Y (sideways): the 15 degree default misses.
        let mut app = game([0.0; 3], [0.0, 1000.0, 0.0], false, None);
        assert!(seen(&mut app).is_empty());
        // Looking straight at it, but a wall is in between.
        let mut app = game([0.0; 3], [1000.0, 0.0, 0.0], true, None);
        assert!(seen(&mut app).is_empty());
        // Looking up and down count too: from below, looking level.
        let mut app = game([0.0, 0.0, -2000.0], [1000.0, 0.0, -2000.0], false, None);
        assert!(seen(&mut app).is_empty());
    }

    #[test]
    fn a_moved_character_is_looked_for_where_it_is() {
        // It moved to Unreal +Y: looking at where the map put it misses.
        let mut app = game(
            [0.0; 3],
            [1000.0, 0.0, 0.0],
            false,
            Some([0.0, 1000.0, 0.0]),
        );
        assert!(seen(&mut app).is_empty());
        let mut app = game(
            [0.0; 3],
            [0.0, 1000.0, 0.0],
            false,
            Some([0.0, 1000.0, 0.0]),
        );
        assert_eq!(seen(&mut app).len(), 1);
    }
}
