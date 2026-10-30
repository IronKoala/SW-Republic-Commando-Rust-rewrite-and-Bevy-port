use super::*;
use rc_level::scripts::{ScriptActor, Vals};

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

fn t(s: &str) -> Val {
    Val::Text(s.into())
}

fn actor(o: ScriptObject, actions: Vec<ScriptObject>) -> ScriptActor {
    ScriptActor {
        obj: o,
        location: [0.0; 3],
        actions,
    }
}

fn act(class: &str, vals: &[(&str, Val)]) -> ScriptObject {
    obj(0, &format!("ACTION_{class}"), class, vals)
}

fn script(name: &str, actions: Vec<ScriptObject>) -> ScriptActor {
    actor(
        obj(1, "ScriptedTrigger", name, &[("Tag", t("ScriptedTrigger"))]),
        actions,
    )
}

fn run(d: &mut Director, secs: f32) -> Vec<Effect> {
    let mut out = Vec::new();
    let steps = (secs / 0.05).round() as usize;
    for _ in 0..steps {
        out.extend(d.step(0.05, None));
    }
    out
}

fn sent(d: &Director) -> Vec<String> {
    d.history.iter().map(|(_, e)| e.clone()).collect()
}

/// Level start, wait for an event, wait a timer, send events in order.
#[test]
fn events_and_timers_chain() {
    let map = MapScripts {
        actors: vec![
            actor(
                obj(
                    2,
                    "PlayerStart",
                    "PlayerStart0",
                    &[("Event", t("levelstart"))],
                ),
                vec![],
            ),
            script(
                "First",
                vec![
                    act("WaitForEvent", &[("ExternalEvent", t("levelstart"))]),
                    act("WaitForTimer", &[("PauseTime", Val::Float(1.5))]),
                    act("TriggerEvent", &[("Event", t("Rappel"))]),
                ],
            ),
            script(
                "Second",
                vec![
                    act("WaitForEvent", &[("ExternalEvent", t("rappel"))]),
                    act("TriggerEvent", &[("Event", t("StartBactaTraining"))]),
                ],
            ),
        ],
        failures: vec![],
    };
    let mut d = Director::new(&map, &[], Settings::default());
    assert_eq!(d.start_events(), ["levelstart"]);
    d.start();
    run(&mut d, 1.0);
    assert_eq!(sent(&d), ["levelstart"]);
    run(&mut d, 1.0);
    assert_eq!(sent(&d), ["levelstart", "Rappel", "StartBactaTraining"]);
    let rappel = d.history[1].0;
    assert!((rappel - 1.55).abs() < 0.06, "Rappel at {rappel}");
    assert_eq!(d.stats.scripts_finished, 2);
}

/// `IfCondition` runs its section only while the condition is on.
#[test]
fn if_condition_skips_its_section() {
    let reminder = |name: &str| {
        script(
            name,
            vec![
                act("WaitForEvent", &[("ExternalEvent", t("Go"))]),
                act(
                    "IfCondition",
                    &[("TriggeredConditionTag", t("BactaDispenserActive"))],
                ),
                act("GotoLabel", &[("LabelName", t("End"))]),
                act("EndSection", &[]),
                act("TriggerEvent", &[("Event", t("Reminder"))]),
                act("Label", &[("LabelName", t("End"))]),
            ],
        )
    };
    let cond = actor(
        obj(
            3,
            "TriggeredCondition",
            "TriggeredCondition4",
            &[("Tag", t("BactaDispenserActive"))],
        ),
        vec![],
    );
    let map = MapScripts {
        actors: vec![cond.clone(), reminder("A")],
        failures: vec![],
    };
    // Condition off: the reminder plays.
    let mut d = Director::new(&map, &[], Settings::default());
    d.fire("Go", "test");
    run(&mut d, 0.2);
    assert_eq!(sent(&d), ["Go", "Reminder"]);
    // Condition on: skipped.
    let mut d = Director::new(&map, &[], Settings::default());
    d.fire("BactaDispenserActive", "test");
    run(&mut d, 0.1);
    assert!(d.condition("bactadispenseractive"));
    d.fire("Go", "test");
    run(&mut d, 0.2);
    assert_eq!(sent(&d), ["BactaDispenserActive", "Go"]);
}

/// A counter sends its event on the last count; a checkpoint asks for a
/// save.
#[test]
fn counter_and_checkpoint() {
    let map = MapScripts {
        actors: vec![
            actor(
                obj(
                    4,
                    "Counter",
                    "Counter1",
                    &[
                        ("Tag", t("DeadGunshipDroid")),
                        ("NumToCount", Val::Byte(3)),
                        ("Event", t("CrashSiteSecured")),
                    ],
                ),
                vec![],
            ),
            actor(
                obj(
                    5,
                    "Checkpoint",
                    "checkpoint1",
                    &[("Tag", t("CrashSiteSecured"))],
                ),
                vec![],
            ),
        ],
        failures: vec![],
    };
    let mut d = Director::new(&map, &[], Settings::default());
    let mut fx = Vec::new();
    for _ in 0..2 {
        d.fire("DeadGunshipDroid", "droid");
        fx.extend(d.step(0.05, None));
    }
    assert!(!sent(&d).contains(&"CrashSiteSecured".to_string()));
    d.fire("DeadGunshipDroid", "droid");
    fx.extend(d.step(0.05, None));
    assert_eq!(sent(&d).last().unwrap(), "CrashSiteSecured");
    assert!(fx.contains(&Effect::Checkpoint {
        name: "checkpoint1".into()
    }));
}

fn trigger(name: &str, tag: &str, event: &str, state: &str, once: bool) -> TriggerInfo {
    TriggerInfo {
        export: 10,
        class: "Trigger".into(),
        name: name.into(),
        location: [0.0, 0.0, 0.0],
        radius: 100.0,
        height: 50.0,
        event: event.into(),
        tag: tag.into(),
        trigger_type: 0,
        once_only: once,
        state: state.into(),
        retrigger_delay: 0.0,
    }
}

/// Walking into a trigger sends its event once; one that another trigger
/// switches on does nothing until then.
#[test]
fn touch_triggers() {
    let map = MapScripts::default();
    let triggers = [
        trigger(
            "Trigger1",
            "Trigger",
            "DialogMakingABarricade",
            "NormalTrigger",
            true,
        ),
        trigger(
            "Trigger2",
            "ObjTrackerTarget03",
            "Late",
            "OtherTriggerTurnsOn",
            false,
        ),
    ];
    let mut d = Director::new(&map, &triggers, Settings::default());
    let far = PlayerBody {
        location: [1000.0, 0.0, 0.0],
        radius: 40.0,
        half_height: 84.0,
    };
    let near = PlayerBody {
        location: [50.0, 0.0, 20.0],
        ..far
    };
    d.step(0.05, Some(far));
    assert!(d.history.is_empty());
    d.step(0.05, Some(near));
    assert_eq!(sent(&d), ["DialogMakingABarricade", "Late"]);
    // Leave and come back: the first fires only once; the second does
    // again.
    let fx = d.step(0.05, Some(far));
    assert!(fx
        .iter()
        .any(|e| matches!(e, Effect::Untrigger { name, .. } if name == "Late")));
    d.step(0.05, Some(near));
    assert_eq!(sent(&d), ["DialogMakingABarricade", "Late", "Late"]);

    // A trigger that starts off and is switched on by an event.
    let map = MapScripts {
        actors: vec![actor(
            obj(
                10,
                "Trigger",
                "Trigger2",
                &[("bInitiallyActive", Val::Bool(false))],
            ),
            vec![],
        )],
        failures: vec![],
    };
    let mut d = Director::new(&map, &triggers[1..], Settings::default());
    d.step(0.05, Some(near));
    assert!(d.history.is_empty());
    d.step(0.05, Some(far));
    d.fire("ObjTrackerTarget03", "test");
    d.step(0.05, Some(far));
    d.step(0.05, Some(near));
    assert_eq!(sent(&d), ["ObjTrackerTarget03", "Late"]);
}

/// A script blows up a barricade prop: it is destroyed once its health
/// runs out, and the game hears about it.
#[test]
fn hurt_actor_destroys_props() {
    let map = MapScripts {
        actors: vec![
            actor(
                obj(
                    616,
                    "Prop",
                    "Prop0",
                    &[
                        ("Tag", t("PathToGunshipBarricade")),
                        ("Health", Val::Int(49000)),
                    ],
                ),
                vec![],
            ),
            script(
                "Blow",
                vec![
                    act("WaitForEvent", &[("ExternalEvent", t("BactaUsed"))]),
                    act(
                        "HurtActor",
                        &[
                            ("TargetTag", t("PathToGunshipBarricade")),
                            ("amount", Val::Float(40000.0)),
                        ],
                    ),
                    act(
                        "HurtActor",
                        &[
                            ("TargetTag", t("PathToGunshipBarricade")),
                            ("amount", Val::Float(10000.0)),
                        ],
                    ),
                ],
            ),
        ],
        failures: vec![],
    };
    let mut d = Director::new(&map, &[], Settings::default());
    assert_eq!(d.props_standing("PathToGunshipBarricade"), 1);
    d.fire("BactaUsed", "dispenser");
    let fx = d.step(0.05, None);
    assert_eq!(d.props_standing("PathToGunshipBarricade"), 0);
    let destroyed: Vec<_> = fx
        .iter()
        .filter(|e| matches!(e, Effect::PropDestroyed { export: 616, .. }))
        .collect();
    assert_eq!(destroyed.len(), 1);
    assert_eq!(
        fx.iter()
            .filter(|e| matches!(e, Effect::Action { .. }))
            .count(),
        2
    );
}

/// A script that loops with no wait can't freeze the game; a sequence
/// starts when the factory that gives it out goes off.
#[test]
fn loops_are_bounded_and_sequences_start_with_their_factory() {
    let map = MapScripts {
        actors: vec![
            script(
                "Loop",
                vec![
                    act("Label", &[("LabelName", t("Top"))]),
                    act("GotoLabel", &[("LabelName", t("Top"))]),
                ],
            ),
            actor(
                obj(
                    20,
                    "PawnFactory",
                    "PawnFactory40",
                    &[
                        ("Tag", t("SpawnBarricadeTroopers")),
                        ("AIScriptFromFactory", t("Kirk")),
                    ],
                ),
                vec![],
            ),
            actor(
                obj(
                    21,
                    "ScriptedSequence",
                    "ScriptedSequence12",
                    &[("Tag", t("Kirk"))],
                ),
                vec![act("TriggerEvent", &[("Event", t("KirkSpawned"))])],
            ),
        ],
        failures: vec![],
    };
    let mut d = Director::new(&map, &[], Settings::default());
    run(&mut d, 0.2);
    assert!(sent(&d).is_empty());
    d.fire("SpawnBarricadeTroopers", "trigger");
    run(&mut d, 0.1);
    assert_eq!(sent(&d), ["SpawnBarricadeTroopers", "KirkSpawned"]);
}

#[test]
fn settings_file() {
    let s = Settings::default()
        .with_overrides("[timing]\nsound_seconds = 4.5\n[actors]\nplayer_tags = [\"Me\"]\n")
        .unwrap();
    assert_eq!(s.sound_seconds, 4.5);
    assert!(s.is_player("me"));
    assert!(Settings::default()
        .with_overrides("[timing]\nsound_secs = 1\n")
        .is_err());
    // The file we ship parses.
    let shipped = include_str!("../../../data/scripts/scripts.toml");
    assert_eq!(
        Settings::default().with_overrides(shipped).unwrap(),
        Settings::default()
    );
}
