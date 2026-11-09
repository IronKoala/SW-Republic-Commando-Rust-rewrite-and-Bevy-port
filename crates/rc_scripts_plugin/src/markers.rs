//! Squad marker jobs reach the level: demolition charges, consoles and
//! doors to slice, and doors to breach.
//!
//! Each squad marker in a map has an "anchor": the map actor its job is
//! for. The anchor keeps the level's side of the job, as map settings:
//!
//! - `DemolitionCharge`: `EventUnderway` when the charge is being placed,
//!   `ChargeArmed.Event` when its fuse is lit, `ChargeExplode.Event` when
//!   it goes off, and `PropToDestroy`, the prop it blows away (a bunker
//!   door, a barricade).
//! - `HackTerminal*` (a console): `EventUnderway` and `ActiveState.Event`
//!   when slicing starts, `HackedState.Event` when it is done.
//! - `HackDoor*` (a door's lock, sliced): `EventUnderway`, then
//!   `DoorUnlockedState.Event`; the door it locks opens.
//! - `BreachDoor*` (a door's lock, blown): `DoorDamagedState.Event` and
//!   `DoorUnlockedState.Event` when the charge goes off; its doors
//!   (`DoorLeft`, `DoorRight`) open.
//!
//! The order marker itself (`MarkerDemolition`, `MarkerDoorBreach`, ...)
//! can have its own `EventUnderway` too, sent when the job starts.
//! A hack door and a breach door on the same door name each other
//! (`MatchingComponent`): doing either opens it.
//!
//! The squad part says how a job is going with
//! [`rc_game_core::MarkerJob`]; this part sends the events, opens the
//! doors and removes the prop. Each step is sent once per anchor.
//!
//! Not done yet: the anchor's own look changing (a lit charge, a sliced
//! console's screen, the blown lock's mesh) and its sounds; the player
//! doing these jobs himself (Use at the marker); doors swinging shut
//! again after a slice (they stay open).

use crate::{LevelScript, PART};
use bevy::prelude::*;
use rc_game_core::{GameLog, MarkerJob, MarkerJobStep};
use rc_level_plugin::LevelParts;
use rc_movers_plugin::LevelMovers;
pub use rc_scripts::markers::{read_anchors, Anchor, AnchorKind};

/// Carry out marker job steps: send the anchor's events, open its doors,
/// blow away its prop.
pub fn run(
    mut jobs: MessageReader<MarkerJob>,
    script: Option<ResMut<LevelScript>>,
    movers: Option<Res<LevelMovers>>,
    parts: Option<Res<LevelParts>>,
    log: Res<GameLog>,
) {
    let Some(mut script) = script else {
        jobs.clear();
        return;
    };
    let script = &mut *script;
    let now = script.director.time;
    for job in jobs.read() {
        let Some(i) = script
            .anchors
            .iter()
            .position(|a| a.name.eq_ignore_ascii_case(&job.anchor))
        else {
            log.line(format!(
                "{PART}: t={now:.2} {} {:?} at {}: not a marker anchor the level knows; nothing to send",
                job.by, job.step, job.anchor
            ));
            continue;
        };
        let slot = match job.step {
            MarkerJobStep::Started => 0,
            MarkerJobStep::Armed => 1,
            MarkerJobStep::Done => 2,
        };
        let a = &mut script.anchors[i];
        if a.sent[slot] {
            continue;
        }
        a.sent[slot] = true;
        // Finishing counts as having started and been armed.
        let mut events: Vec<String> = Vec::new();
        for (k, list) in [&a.started, &a.armed, &a.done].into_iter().enumerate() {
            if k == slot || (k < slot && !a.sent[k]) {
                events.extend(list.iter().cloned());
            }
        }
        for k in 0..slot {
            a.sent[k] = true;
        }
        let a = a.clone();
        log.line(format!(
            "{PART}: t={now:.2} {} {} {}: {:?} by {}; sends {:?}",
            a.what(),
            a.name,
            a.class,
            job.step,
            job.by,
            events
        ));
        let by = format!("{} ({})", a.name, a.what());
        for e in &events {
            script.director.fire(e, &by);
        }
        script
            .stats
            .marker_steps
            .push(format!("{} {:?}", a.name, job.step));
        if job.step != MarkerJobStep::Done {
            continue;
        }
        if let Some((prop, export)) = &a.destroys {
            // A prop the scripts know sends its own destroyed event too.
            if !script.director.destroy_prop_named(prop) {
                if let Some(export) = export {
                    if let Some(p) = &parts {
                        p.set_solid_export(*export, false);
                    }
                    script.hidden.insert(*export);
                    script.stats.props_destroyed.push(prop.clone());
                }
            }
            log.line(format!(
                "{PART}: t={now:.2} {} blows away {prop}: no longer solid or drawn",
                a.name
            ));
        }
        for d in &a.doors {
            let opened = movers.as_ref().is_some_and(|m| m.open_named(d));
            log.line(format!(
                "{PART}: t={now:.2} {} unlocked: door {d} {}",
                a.name,
                if opened {
                    "opens"
                } else {
                    "not found among the doors"
                }
            ));
            if opened {
                script.stats.doors_unlocked.push(d.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LevelScript, PlayStats};
    use bevy::ecs::message::Messages;
    use bevy::ecs::system::RunSystemOnce;
    use rc_level::scripts::{MapScripts, ScriptActor, ScriptObject, Val, Vals};
    use rc_scripts::{Director, Settings};

    fn obj(export: usize, class: &str, name: &str, vals: &[(&str, Val)]) -> ScriptObject {
        let mut values = Vals::new();
        for (k, v) in vals {
            values.insert(k.to_string(), v.clone());
        }
        ScriptObject {
            export,
            class: class.into(),
            name: name.into(),
            values,
        }
    }

    fn actor(o: ScriptObject, actions: Vec<ScriptObject>) -> ScriptActor {
        ScriptActor {
            obj: o,
            location: [0.0; 3],
            actions,
        }
    }

    fn t(s: &str) -> Val {
        Val::Text(s.into())
    }

    fn state(event: &str) -> Val {
        let mut v = Vals::new();
        v.insert("Event".into(), t(event));
        v.insert("Mesh".into(), t("None"));
        Val::Struct(v)
    }

    /// geo_01a's bunker charge and geo_01b's hangar door (a slice lock and
    /// a breach lock on the same double door), as the maps store them.
    fn map() -> MapScripts {
        let act =
            |class: &str, vals: &[(&str, Val)]| obj(0, &format!("ACTION_{class}"), class, vals);
        MapScripts {
            actors: vec![
                actor(
                    obj(
                        10,
                        "DemolitionCharge",
                        "DemolitionCharge2",
                        &[
                            ("ChargeArmed", state("DemoChargeArmed")),
                            ("ChargeExplode", state("BunkerDoorDestroyed")),
                            ("EventUnderway", t("DemoUnderway")),
                            ("PropToDestroy", t("Prop12")),
                        ],
                    ),
                    vec![],
                ),
                actor(
                    obj(
                        11,
                        "MarkerDemolition",
                        "MarkerDemolition2",
                        &[
                            ("Anchor", t("DemolitionCharge2")),
                            ("EventUnderway", t("SpawnDemoDroids")),
                        ],
                    ),
                    vec![],
                ),
                actor(
                    obj(
                        12,
                        "Prop",
                        "Prop12",
                        &[("Tag", t("BunkerDoor")), ("Health", Val::Float(100.0))],
                    ),
                    vec![],
                ),
                actor(
                    obj(
                        20,
                        "HackDoorGeo",
                        "HackDoorGeo6",
                        &[
                            ("DoorUnlockedState", state("HangarDoorHacked")),
                            ("EventUnderway", t("StartingHangarDoorHack")),
                            ("MatchingComponent", t("BreachDoorGeo0")),
                            ("Touching", Val::List(vec!["MoverDB_GEO6".into()])),
                        ],
                    ),
                    vec![],
                ),
                actor(
                    obj(
                        21,
                        "MarkerHackDoor",
                        "MarkerHackDoor53",
                        &[("Anchor", t("HackDoorGeo6"))],
                    ),
                    vec![],
                ),
                actor(
                    obj(
                        22,
                        "BreachDoorGeo",
                        "BreachDoorGeo0",
                        &[
                            ("DoorLeft", t("MoverDB_GEO6")),
                            ("DoorRight", t("MoverDBSlave_GEO6")),
                        ],
                    ),
                    vec![],
                ),
                actor(
                    obj(
                        1,
                        "ScriptedTrigger",
                        "ScriptedTrigger4",
                        &[("Tag", t("ScriptedTrigger"))],
                    ),
                    vec![
                        act(
                            "WaitForEvent",
                            &[("ExternalEvent", t("BunkerDoorDestroyed"))],
                        ),
                        act("TriggerEvent", &[("Event", t("OnToTheTrench"))]),
                    ],
                ),
            ],
            failures: vec![],
            sounds: Default::default(),
        }
    }

    #[test]
    fn reads_the_anchors() {
        let a = read_anchors(&map());
        assert_eq!(a.len(), 2, "{a:?}");
        let demo = &a[0];
        assert_eq!(demo.kind, AnchorKind::DemolitionCharge);
        assert_eq!(demo.started, vec!["DemoUnderway", "SpawnDemoDroids"]);
        assert_eq!(demo.armed, vec!["DemoChargeArmed"]);
        assert_eq!(demo.done, vec!["BunkerDoorDestroyed"]);
        assert_eq!(demo.destroys, Some(("Prop12".to_string(), Some(12))));
        let door = &a[1];
        assert_eq!(door.kind, AnchorKind::HackDoor);
        assert_eq!(door.done, vec!["HangarDoorHacked"]);
        // Its own door, plus the breach lock's other leaf.
        assert_eq!(door.doors, vec!["MoverDB_GEO6", "MoverDBSlave_GEO6"]);
    }

    fn world() -> World {
        let m = map();
        let mut w = World::new();
        w.insert_resource(GameLog::terminal());
        w.init_resource::<Messages<MarkerJob>>();
        w.insert_resource(LevelScript {
            director: Director::new(&m, &[], Settings::default()),
            dispensers: vec![],
            hidden: Default::default(),
            held: vec![],
            calls: vec![],
            places: Default::default(),
            sounds: vec![],
            anchors: read_anchors(&m),
            scene: crate::cutscene::Scene::default(),
            started: true,
            stats: PlayStats::default(),
        });
        w
    }

    fn send(w: &mut World, anchor: &str, step: MarkerJobStep) {
        w.write_message(MarkerJob {
            anchor: anchor.into(),
            step,
            by: "Scorch".into(),
        });
        w.run_system_once(run).unwrap();
        w.resource_mut::<LevelScript>().director.step(0.1, None);
    }

    fn sent(w: &World) -> Vec<String> {
        w.resource::<LevelScript>()
            .director
            .history
            .iter()
            .map(|(_, e)| e.clone())
            .collect()
    }

    /// The charge's events come in order, the script waiting for the
    /// explosion carries on, and the bunker door prop is blown away.
    #[test]
    fn a_charge_sets_off_the_level() {
        let mut w = world();
        w.resource_mut::<LevelScript>().director.start();
        send(&mut w, "DemolitionCharge2", MarkerJobStep::Started);
        send(&mut w, "DemolitionCharge2", MarkerJobStep::Started);
        send(&mut w, "DemolitionCharge2", MarkerJobStep::Armed);
        assert!(!sent(&w).iter().any(|e| e == "BunkerDoorDestroyed"));
        send(&mut w, "DemolitionCharge2", MarkerJobStep::Done);
        let s = sent(&w);
        let pos = |n: &str| {
            s.iter()
                .position(|e| e == n)
                .unwrap_or_else(|| panic!("{n} not sent: {s:?}"))
        };
        assert!(pos("DemoUnderway") < pos("DemoChargeArmed"));
        assert!(pos("DemoChargeArmed") < pos("BunkerDoorDestroyed"));
        assert!(pos("BunkerDoorDestroyed") < pos("OnToTheTrench"));
        // Sent once, though the squad said "started" twice.
        assert_eq!(s.iter().filter(|e| *e == "DemoUnderway").count(), 1);
        let script = w.resource::<LevelScript>();
        assert_eq!(script.director.props_standing("BunkerDoor"), 0);
    }

    /// Finishing a job whose start nobody reported still sends the start.
    #[test]
    fn done_alone_sends_everything() {
        let mut w = world();
        send(&mut w, "HackDoorGeo6", MarkerJobStep::Done);
        let s = sent(&w);
        assert!(s.contains(&"StartingHangarDoorHack".to_string()), "{s:?}");
        assert!(s.contains(&"HangarDoorHacked".to_string()), "{s:?}");
        send(&mut w, "Nobody", MarkerJobStep::Done);
        assert_eq!(
            w.resource::<LevelScript>().stats.marker_steps,
            vec!["HackDoorGeo6 Done"]
        );
    }
}
