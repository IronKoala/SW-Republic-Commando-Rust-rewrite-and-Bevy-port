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

/// A save made while a script waits on a timer, with a counter half
/// counted, loads into a fresh director that carries on the same way
/// without sending the level-start event again.
#[test]
fn snapshot_restores_scripts_mid_wait() {
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
            actor(
                obj(
                    4,
                    "Counter",
                    "Counter1",
                    &[
                        ("Tag", t("DeadDroid")),
                        ("NumToCount", Val::Byte(3)),
                        ("Event", t("AllDead")),
                    ],
                ),
                vec![],
            ),
            script(
                "First",
                vec![
                    act("WaitForEvent", &[("ExternalEvent", t("levelstart"))]),
                    act("WaitForTimer", &[("PauseTime", Val::Float(1.5))]),
                    act("TriggerEvent", &[("Event", t("Rappel"))]),
                    act("WaitForEvent", &[("ExternalEvent", t("AllDead"))]),
                    act("TriggerEvent", &[("Event", t("Done"))]),
                ],
            ),
        ],
        failures: vec![],
    };
    let mut a = Director::new(&map, &[], Settings::default());
    a.start();
    run(&mut a, 1.0);
    a.fire("DeadDroid", "droid");
    run(&mut a, 0.1);
    let snap = a.snapshot();
    let first = &snap.script[0];
    assert!(first.waits_until.is_some() && first.next == 1, "{first:?}");
    assert_eq!(snap.counter[0].left, 2);

    // Through the file and back.
    let text = toml::to_string(&snap).unwrap();
    let back: crate::save::DirectorSave = toml::from_str(&text).unwrap();
    assert_eq!(back, snap, "{text}");

    let mut b = Director::new(&map, &[], Settings::default());
    let report = b.restore(&back);
    assert!(
        report.unknown.is_empty() && report.past_end.is_empty(),
        "{report:?}"
    );
    assert_eq!(b.snapshot(), snap);
    // No level start this time, and it carries on as the first did.
    run(&mut a, 1.0);
    run(&mut b, 1.0);
    assert_eq!(sent(&b), ["Rappel"]);
    for d in [&mut a, &mut b] {
        d.fire("DeadDroid", "droid");
        d.fire("DeadDroid", "droid");
        run(d, 0.1);
    }
    assert_eq!(
        sent(&b),
        ["Rappel", "DeadDroid", "DeadDroid", "AllDead", "Done"]
    );
    assert_eq!(a.snapshot(), b.snapshot());
}

/// Names the map doesn't have are skipped and reported.
#[test]
fn restore_skips_unknown_names() {
    let map = MapScripts {
        actors: vec![script(
            "First",
            vec![act("WaitForEvent", &[("ExternalEvent", t("Go"))])],
        )],
        failures: vec![],
    };
    let mut d = Director::new(&map, &[], Settings::default());
    let mut snap = d.snapshot();
    snap.script[0].next = 7;
    snap.counter.push(crate::save::CounterSave {
        name: "Counter9".into(),
        left: 1,
    });
    let r = d.restore(&snap);
    assert_eq!(r.unknown, ["Counter9"]);
    assert_eq!(r.past_end, ["First"]);
}

/// A placed character's sequence: its actions name it as the owner, a
/// walk waits for the game to say it arrived (or the time-out), and an
/// action the map switched off is skipped.
#[test]
fn character_actions_wait_for_the_game() {
    let seq = |n: usize| {
        actor(
            obj(
                30 + n,
                "ScriptedSequence",
                &format!("Seq{n}"),
                &[("Tag", t(&format!("WalkerAI{n}")))],
            ),
            vec![
                act("MakeInvulnerable", &[("bDisabled", Val::Bool(true))]),
                act("MoveToPoint", &[("DestinationTag", t("Here"))]),
                act("TriggerEvent", &[("Event", t(&format!("Arrived{n}")))]),
            ],
        )
    };
    let placed = |n: usize| {
        actor(
            obj(
                40 + n,
                "CloneTrooper",
                &format!("CloneTrooper{n}"),
                &[("AIScriptTag", t(&format!("WalkerAI{n}")))],
            ),
            vec![],
        )
    };
    let map = MapScripts {
        actors: vec![seq(1), placed(1), seq(2), placed(2)],
        failures: vec![],
    };
    let mut d = Director::new(&map, &[], Settings::default());
    d.game_runs_characters = true;
    let out = run(&mut d, 0.1);
    let actions: Vec<_> = out
        .iter()
        .filter_map(|e| match e {
            Effect::Action {
                script,
                index,
                action,
                owner,
                waits,
            } => Some((
                script.clone(),
                *index,
                action.name.clone(),
                owner.clone(),
                *waits,
            )),
            _ => None,
        })
        .collect();
    // The switched-off action never reaches the game.
    assert_eq!(
        actions,
        [
            (
                "Seq1".into(),
                1,
                "MoveToPoint".into(),
                Some("WalkerAI1".into()),
                true
            ),
            (
                "Seq2".into(),
                1,
                "MoveToPoint".into(),
                Some("WalkerAI2".into()),
                true
            ),
        ]
    );
    assert!(out
        .iter()
        .any(|e| matches!(e, Effect::Note(n) if n.contains("switched off"))));
    // Long past the old fixed 2 s, they still wait for the game.
    run(&mut d, 5.0);
    assert!(sent(&d).is_empty());
    // The first arrives; a wrong index is ignored.
    assert!(!d.action_done("Seq1", 0));
    assert!(d.action_done("Seq1", 1));
    run(&mut d, 0.1);
    assert_eq!(sent(&d), ["Arrived1"]);
    // The second never does: it carries on at the time-out.
    let w = d.settings.move_timeout_seconds;
    run(&mut d, w);
    assert_eq!(sent(&d), ["Arrived1", "Arrived2"]);

    // With no game (the headless tools) the walk takes the fixed time.
    let mut d = Director::new(&map, &[], Settings::default());
    let w = d.settings.move_seconds + 0.2;
    run(&mut d, w);
    assert_eq!(sent(&d), ["Arrived1", "Arrived2"]);
}

/// A save made while a character walks: after the load the walk is
/// handed to the game again, and finishing it carries the script on.
#[test]
fn saved_walk_is_started_again() {
    let map = MapScripts {
        actors: vec![
            actor(
                obj(30, "ScriptedSequence", "Seq", &[("Tag", t("WalkerAI"))]),
                vec![
                    act("MoveToPoint", &[("DestinationTag", t("Here"))]),
                    act("TriggerEvent", &[("Event", t("Arrived"))]),
                ],
            ),
            actor(
                obj(
                    40,
                    "CloneTrooper",
                    "CloneTrooper0",
                    &[("AIScriptTag", t("WalkerAI"))],
                ),
                vec![],
            ),
        ],
        failures: vec![],
    };
    let mut d = Director::new(&map, &[], Settings::default());
    d.game_runs_characters = true;
    run(&mut d, 1.0);
    let save = d.snapshot();
    assert_eq!(save.script[0].waits_for_action, Some(0));
    let mut e = Director::new(&map, &[], Settings::default());
    e.game_runs_characters = true;
    e.restore(&save);
    let out = run(&mut e, 0.1);
    assert!(out.iter().any(|x| matches!(
        x,
        Effect::Action {
            index: 0,
            waits: true,
            ..
        }
    )));
    assert!(e.action_done("Seq", 0));
    run(&mut e, 0.1);
    assert_eq!(sent(&e), ["Arrived"]);
}
