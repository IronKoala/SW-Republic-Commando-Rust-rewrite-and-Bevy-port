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
        sounds: Default::default(),
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
        sounds: Default::default(),
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
        sounds: Default::default(),
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
        sounds: Default::default(),
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
        sounds: Default::default(),
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
        sounds: Default::default(),
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
        sounds: Default::default(),
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
        sounds: Default::default(),
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
        sounds: Default::default(),
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
        sounds: Default::default(),
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

/// A script that waits for a line waits for the line's own length when the
/// game has told the director, and the settings' guess otherwise. A line
/// switched off (`bDisabled`) is not waited for.
#[test]
fn waits_for_the_sounds_length() {
    let line = |name: &str| {
        act(
            "PlaySound",
            &[("Sound", t(name)), ("bWaitForFinish", Val::Bool(true))],
        )
    };
    let map = MapScripts {
        actors: vec![script(
            "Talk",
            vec![
                line("CA1G1701"),
                act("TriggerEvent", &[("Event", t("AfterKnown"))]),
                line("Unknown01"),
                act("TriggerEvent", &[("Event", t("AfterUnknown"))]),
                act(
                    "PlaySound",
                    &[
                        ("Sound", t("Off01")),
                        ("bWaitForFinish", Val::Bool(true)),
                        ("bDisabled", Val::Bool(true)),
                    ],
                ),
                act("TriggerEvent", &[("Event", t("AfterOff"))]),
            ],
        )],
        failures: vec![],
        sounds: Default::default(),
    };
    let mut d = Director::new(&map, &[], Settings::default());
    d.set_sound_length("ca1g1701", 1.0);
    assert_eq!(d.sound_length("CA1G1701"), 1.0);
    assert_eq!(d.sound_length("Unknown01"), d.settings.sound_seconds);
    run(&mut d, 0.9);
    assert!(sent(&d).is_empty());
    run(&mut d, 0.2);
    assert_eq!(sent(&d), ["AfterKnown"]);
    // The unknown line takes the settings' 3 s.
    run(&mut d, 2.8);
    assert_eq!(sent(&d), ["AfterKnown"]);
    run(&mut d, 0.3);
    assert_eq!(sent(&d), ["AfterKnown", "AfterUnknown", "AfterOff"]);
}

fn body_at(x: f32, y: f32, z: f32) -> Option<PlayerBody> {
    Some(PlayerBody {
        location: [x, y, z],
        radius: 40.0,
        half_height: 84.0,
    })
}

fn teleporter(name: &str, vals: &[(&str, Val)], at: [f32; 3]) -> ScriptActor {
    let mut a = actor(obj(30, "Teleporter", name, vals), vec![]);
    a.location = at;
    a
}

fn changes(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::ChangeLevel { url, .. } => Some(url.clone()),
            _ => None,
        })
        .collect()
}

/// A level exit that a script opens (as in geo_03a, ras_01c): closed at
/// first, `ACTION_SetCollision` opens it, then walking in leaves, once.
#[test]
fn exit_opens_and_leaves_once() {
    let map = MapScripts {
        actors: vec![
            teleporter(
                "Teleporter0",
                &[
                    ("Tag", t("EndPort")),
                    ("URL", t("GEO_04A.ctm#playerstart")),
                    ("CollisionRadius", Val::Float(300.0)),
                    ("bCollideActors", Val::Bool(false)),
                ],
                [1000.0, 0.0, 100.0],
            ),
            script(
                "Ending",
                vec![
                    act("WaitForEvent", &[("ExternalEvent", t("AllClear"))]),
                    act("SetCollision", &[("CollideActorTag", t("EndPort"))]),
                ],
            ),
        ],
        failures: vec![],
        sounds: Default::default(),
    };
    let mut d = Director::new(&map, &[], Settings::default());
    assert_eq!(d.stats.exits, 1);
    assert!(!d.exits()[0].3, "closed until the script opens it");
    // Standing in it while closed does nothing.
    let out = d.step(0.05, body_at(1100.0, 0.0, 120.0));
    assert!(changes(&out).is_empty());
    d.fire("AllClear", "test");
    d.step(0.05, body_at(0.0, 0.0, 120.0));
    assert!(d.exits()[0].3, "open now");
    // Still outside: nothing.
    assert!(changes(&d.step(0.05, body_at(0.0, 0.0, 120.0))).is_empty());
    // Walk in.
    let out = d.step(0.05, body_at(1250.0, 0.0, 120.0));
    assert_eq!(changes(&out), ["GEO_04A.ctm#playerstart"]);
    assert!(d.leaving());
    // Only once, and nothing runs any more.
    d.fire("AllClear", "test");
    let out = d.step(0.05, body_at(1250.0, 0.0, 120.0));
    assert!(
        changes(&out).is_empty() && d.stats.events_sent == 1,
        "{out:?}"
    );
}

/// A switched-off exit (geo_04c's `bEnabled=false`) is switched on by an
/// event with its tag, as UE2 teleporters are. Too high or low misses.
#[test]
fn exit_switched_on_by_its_tag() {
    let map = MapScripts {
        actors: vec![teleporter(
            "Teleporter1",
            &[
                ("Tag", t("DemoMarker00")),
                ("URL", t("Geo_04D.ctm#FromC")),
                ("bEnabled", Val::Bool(false)),
            ],
            [0.0, 0.0, 0.0],
        )],
        failures: vec![],
        sounds: Default::default(),
    };
    let mut d = Director::new(&map, &[], Settings::default());
    assert!(changes(&d.step(0.05, body_at(0.0, 0.0, 84.0))).is_empty());
    d.fire("demomarker00", "test");
    // Above it: default height 100 + 84 for the body.
    assert!(changes(&d.step(0.05, body_at(0.0, 0.0, 200.0))).is_empty());
    let out = d.step(0.05, body_at(50.0, 0.0, 84.0));
    assert_eq!(changes(&out), ["Geo_04D.ctm#FromC"]);
}

/// `ACTION_ChangeLevel` and the debriefing's `NewLevel` leave too; an
/// action the map switched off (`bDisabled`) is skipped.
#[test]
fn scripts_change_level_and_skip_disabled_actions() {
    let map = MapScripts {
        actors: vec![script(
            "Cutscene",
            vec![
                act(
                    "ChangeLevel",
                    &[("URL", t("Geo_01a.ctm")), ("bDisabled", Val::Bool(true))],
                ),
                act("TriggerEvent", &[("Event", t("Shown"))]),
                act("ChangeLevel", &[("URL", t("RAS_03B.ctm#FromA"))]),
                act("TriggerEvent", &[("Event", t("TooLate"))]),
            ],
        )],
        failures: vec![],
        sounds: Default::default(),
    };
    let mut d = Director::new(&map, &[], Settings::default());
    let out = run(&mut d, 0.2);
    assert_eq!(changes(&out), ["RAS_03B.ctm#FromA"]);
    assert_eq!(sent(&d), ["Shown"]);

    let map = MapScripts {
        actors: vec![script(
            "End",
            vec![act(
                "ShowDebriefingScreen",
                &[
                    ("NewLevel", t("YYY_01Briefing.ctm")),
                    ("MenuClass", t("XInterfaceCTMenus.CTDebriefingInfoRAS")),
                    ("Title", t("CRITICAL DISCOVERIES")),
                    ("Text", t("Well done.")),
                    ("PicName", t("GUIContent.RAS_Debrief")),
                ],
            )],
        )],
        failures: vec![],
        sounds: Default::default(),
    };
    let mut d = Director::new(&map, &[], Settings::default());
    let out = run(&mut d, 0.1);
    assert_eq!(changes(&out), ["YYY_01Briefing.ctm"]);
    // It carries what the screen shows, so the game can show it first.
    let shown: Vec<&Debriefing> = out
        .iter()
        .filter_map(|e| match e {
            Effect::ChangeLevel { debriefing, .. } => debriefing.as_ref(),
            _ => None,
        })
        .collect();
    assert_eq!(
        shown,
        [&Debriefing {
            menu_class: "XInterfaceCTMenus.CTDebriefingInfoRAS".into(),
            title: "CRITICAL DISCOVERIES".into(),
            text: "Well done.".into(),
            picture: "GUIContent.RAS_Debrief".into(),
        }]
    );
    // A hint screen (same action, no NewLevel) doesn't leave.
    let map = MapScripts {
        actors: vec![script(
            "Hint",
            vec![act("ShowDebriefingScreen", &[("Text", t("Crouch!"))])],
        )],
        failures: vec![],
        sounds: Default::default(),
    };
    let mut d = Director::new(&map, &[], Settings::default());
    assert!(changes(&run(&mut d, 0.1)).is_empty() && !d.leaving());
}

/// An exit opened by a script stays open through a save and load.
#[test]
fn opened_exit_survives_a_save() {
    let map = MapScripts {
        actors: vec![teleporter(
            "Teleporter0",
            &[
                ("Tag", t("ToC")),
                ("URL", t("GEO_03C.ctm#playerstart")),
                ("bCollideActors", Val::Bool(false)),
            ],
            [0.0; 3],
        )],
        failures: vec![],
        sounds: Default::default(),
    };
    let mut a = Director::new(&map, &[], Settings::default());
    a.teleporters[0].collide = true;
    let snap = a.snapshot();
    let mut b = Director::new(&map, &[], Settings::default());
    b.restore(&snap);
    assert!(b.exits()[0].3);
}

#[test]
fn level_addresses_read_as_map_and_portal() {
    let r = |u: &str| level_url(u);
    assert_eq!(
        r("RAS_03B.ctm#FromA"),
        Some(("ras_03b".into(), Some("FromA".into())))
    );
    assert_eq!(
        r("YYY_05A#PlayerStart"),
        Some(("yyy_05a".into(), Some("PlayerStart".into())))
    );
    assert_eq!(r("Geo_01Briefing"), Some(("geo_01briefing".into(), None)));
    assert_eq!(r(" EPILOGUE.ctm?Game=X "), Some(("epilogue".into(), None)));
    assert_eq!(r(""), None);
    assert_eq!(r("#FromA"), None);
    assert_eq!(r("../../x.ctm"), None);
}

/// A Dispatcher (the prologue's and briefings' timing) sends its list in
/// order, each after its own delay, when its tag is sent; it ignores its
/// tag while it runs, and can run again afterwards.
#[test]
fn dispatcher_sends_its_list_in_time() {
    let entry = |event: &str, delay: f32| {
        let mut v = Vals::new();
        v.insert("Event".into(), t(event));
        v.insert("Delay".into(), Val::Float(delay));
        v.insert("DelayMax".into(), Val::Float(0.0));
        v.insert("Sound".into(), t("None"));
        v.insert("bWaitForSound".into(), Val::Bool(false));
        v
    };
    let map = MapScripts {
        actors: vec![
            actor(
                obj(2, "PlayerStart", "PlayerStart0", &[("Event", t("Go"))]),
                vec![],
            ),
            actor(
                obj(
                    3,
                    "Dispatcher",
                    "Dispatcher1",
                    &[
                        ("Tag", t("Go")),
                        (
                            "Events",
                            Val::Structs(vec![
                                entry("First", 0.0),
                                entry("Second", 2.0),
                                entry("Third", 1.0),
                            ]),
                        ),
                    ],
                ),
                vec![],
            ),
        ],
        ..Default::default()
    };
    let mut d = Director::new(&map, &[], Settings::default());
    assert_eq!(d.stats.dispatchers, 1);
    d.start();
    run(&mut d, 0.1);
    assert_eq!(sent(&d), ["Go", "First"]);
    // Sent again while it runs: not heard.
    d.fire("Go", "test");
    run(&mut d, 2.0);
    assert_eq!(sent(&d), ["Go", "First", "Go", "Second"]);
    run(&mut d, 1.2);
    assert_eq!(sent(&d), ["Go", "First", "Go", "Second", "Third"]);
    let at = |e: &str| d.history.iter().find(|(_, n)| n == e).unwrap().0;
    assert!((at("Second") - at("First") - 2.0).abs() < 0.11);
    assert!((at("Third") - at("Second") - 1.0).abs() < 0.11);
    // Done: it runs again on its tag.
    d.fire("Go", "test");
    run(&mut d, 0.1);
    assert_eq!(sent(&d).iter().filter(|e| *e == "First").count(), 2);
}

fn breakable(export: usize, name: &str, tag: &str, health: f32) -> Breakable {
    Breakable {
        export,
        name: name.into(),
        class: "ExplosiveBarrelGEO".into(),
        tag: tag.into(),
        location: [0.0; 3],
        rotation: [0; 3],
        draw_scale: 1.0,
        draw_scale_3d: [1.0; 3],
        pre_pivot: [0.0; 3],
        health,
        damaged_ratio: 0.0,
        event: None,
        damaged: rc_level::breakables::Transition::default(),
        destroyed: rc_level::breakables::Transition::default(),
        explosion: None,
        damage_table: None,
        hidden: false,
        anim_prop: false,
    }
}

/// geo_05b's clamps: shooting one breaks it at no health, its event
/// reaches the script waiting for it, and more damage does nothing.
#[test]
fn shot_props_break_and_tell_the_scripts() {
    let map = MapScripts {
        actors: vec![script(
            "WaitClamp",
            vec![
                act("WaitForEvent", &[("ExternalEvent", t("Clamp00"))]),
                act("TriggerEvent", &[("Event", t("ClampGone"))]),
            ],
        )],
        failures: vec![],
        sounds: Default::default(),
    };
    let mut d = Director::new(&map, &[], Settings::default());
    let mut clamp = breakable(40, "KarmaProp2", "Attachment00", 50.0);
    clamp.event = Some("Clamp00".into());
    d.add_breakables(&[clamp]);
    assert_eq!(d.stats.breakables, 1);
    run(&mut d, 0.1);
    assert_eq!(d.hurt_prop(40, 20.0), PropHurt::Hurt(30.0));
    assert_eq!(d.hurt_prop(99, 20.0), PropHurt::NotBreakable);
    assert_eq!(d.hurt_prop(40, 30.0), PropHurt::Broken);
    assert_eq!(d.hurt_prop(40, 30.0), PropHurt::AlreadyBroken);
    let fx = run(&mut d, 0.1);
    assert!(fx
        .iter()
        .any(|e| matches!(e, Effect::PropDestroyed { export: 40, .. })));
    let s = sent(&d);
    assert!(s.contains(&"Clamp00".to_string()), "{s:?}");
    assert!(s.contains(&"ClampGone".to_string()), "{s:?}");
    assert_eq!(d.props_standing("Attachment00"), 0);
    assert_eq!(d.destroyed_props(), vec![(40, "Attachment00".to_string())]);
}

/// A prop with a damaged step: at ratio x its health it goes there
/// (sending the damaged event); with no damaged look it is simply gone
/// and sends no broken event (rock cover); with a look and a time limit
/// it breaks by itself when the time runs out.
#[test]
fn damaged_step_rules() {
    let map = MapScripts::default();
    let mut d = Director::new(&map, &[], Settings::default());
    let mut rock = breakable(1, "GEO4_RockCover4", "GEO4_RockCover00", 2000.0);
    rock.damaged_ratio = 0.01;
    rock.event = Some("RockProp03".into());
    let mut panel = breakable(2, "Panel", "Panel", 100.0);
    panel.damaged_ratio = 0.5;
    panel.damaged.mesh = Some("FlyingGrate".into());
    panel.damaged.event = Some("PanelHit".into());
    panel.damaged.life_span = 2.0;
    panel.event = Some("PanelGone".into());
    d.add_breakables(&[rock, panel]);
    assert_eq!(d.hurt_prop(1, 1500.0), PropHurt::Hurt(500.0));
    assert_eq!(d.hurt_prop(1, 5000.0), PropHurt::Damaged);
    assert_eq!(d.hurt_prop(1, 5.0), PropHurt::AlreadyBroken);
    let fx = run(&mut d, 0.1);
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::PropDamaged {
            export: 1,
            removed: true,
            ..
        }
    )));
    assert!(!sent(&d).contains(&"RockProp03".to_string()));

    assert_eq!(d.hurt_prop(2, 60.0), PropHurt::Damaged);
    assert_eq!(d.hurt_prop(2, 10.0), PropHurt::Hurt(30.0));
    run(&mut d, 1.0);
    assert!(sent(&d).contains(&"PanelHit".to_string()));
    assert!(!sent(&d).contains(&"PanelGone".to_string()));
    run(&mut d, 1.5);
    assert!(
        sent(&d).contains(&"PanelGone".to_string()),
        "{:?}",
        sent(&d)
    );
    assert_eq!(d.prop_health(2), Some(0.0));
}

/// A broken prop stays broken after a save and a load.
#[test]
fn broken_props_are_saved() {
    let map = MapScripts::default();
    let mut d = Director::new(&map, &[], Settings::default());
    d.add_breakables(&[breakable(7, "Barrel", "B", 60.0)]);
    d.hurt_prop(7, 100.0);
    run(&mut d, 0.1);
    let snap = d.snapshot();
    let mut e = Director::new(&map, &[], Settings::default());
    e.add_breakables(&[breakable(7, "Barrel", "B", 60.0)]);
    e.restore(&snap);
    assert_eq!(e.hurt_prop(7, 1.0), PropHurt::AlreadyBroken);
    assert_eq!(e.destroyed_props(), vec![(7, "B".to_string())]);
}

fn sight(name: &str, vals: &[(&str, Val)], at: [f32; 3]) -> ScriptActor {
    let mut a = actor(obj(50, "LineOfSightTrigger", name, vals), vec![]);
    a.location = at;
    a
}

/// A line-of-sight trigger sends its event once, only when the player
/// looks at it from near enough, straight on, with a clear line.
#[test]
fn line_of_sight_trigger_fires_when_looked_at() {
    let map = MapScripts {
        actors: vec![
            sight(
                "LineOfSightTrigger0",
                &[
                    ("Event", t("SawDroids")),
                    ("MaxViewDist", Val::Float(1000.0)),
                    ("MaxViewAngle", Val::Int(10)),
                ],
                [500.0, 0.0, 0.0],
            ),
            script(
                "Waits",
                vec![act("WaitForEvent", &[("ExternalEvent", t("SawDroids"))])],
            ),
        ],
        ..Default::default()
    };
    let mut d = Director::new(&map, &[], Settings::default());
    assert_eq!(d.stats.sight_triggers, 1);
    let eye = [0.0, 0.0, 0.0];
    // Looking the other way, or too far off to the side: nothing.
    assert!(d.sight_candidates(eye, [-1.0, 0.0, 0.0]).is_empty());
    assert!(d.sight_candidates(eye, [0.0, 1.0, 0.0]).is_empty());
    // 20 degrees off: outside the 10 degree angle.
    let a = 20f32.to_radians();
    assert!(d.sight_candidates(eye, [a.cos(), a.sin(), 0.0]).is_empty());
    // Too far away, even straight on.
    assert!(d
        .sight_candidates([-600.0, 0.0, 0.0], [1.0, 0.0, 0.0])
        .is_empty());
    // Straight on but a wall in the way.
    assert_eq!(d.look(eye, [1.0, 0.0, 0.0], &mut |_, _| false), 0);
    run(&mut d, 0.1);
    assert!(!sent(&d).contains(&"SawDroids".to_string()));
    // 5 degrees off with a clear line: it goes off.
    let a = 5f32.to_radians();
    assert_eq!(d.look(eye, [a.cos(), a.sin(), 0.0], &mut |_, _| true), 1);
    run(&mut d, 0.1);
    assert_eq!(sent(&d), vec!["SawDroids".to_string()]);
    // Only once.
    assert_eq!(d.look(eye, [1.0, 0.0, 0.0], &mut |_, _| true), 0);
    let st = d.script_states();
    assert!(
        st.iter().any(|(n, _, s)| n == "Waits" && s == "finished"),
        "{st:?}"
    );
}

/// `SeenActorTag` points it at another actor; `bEnabled=False` keeps it
/// off until an event with its tag switches it on; saves keep both.
#[test]
fn line_of_sight_trigger_seen_actor_and_switch_on() {
    let mut droid = actor(
        obj(60, "Prop", "Target0", &[("Tag", t("LookHere"))]),
        vec![],
    );
    droid.location = [0.0, 800.0, 0.0];
    let map = MapScripts {
        actors: vec![
            sight(
                "LOS1",
                &[
                    ("Tag", t("ArmLOS")),
                    ("Event", t("Seen")),
                    ("SeenActorTag", t("LookHere")),
                    ("bEnabled", Val::Bool(false)),
                ],
                [5000.0, 0.0, 0.0],
            ),
            droid,
        ],
        ..Default::default()
    };
    let mut d = Director::new(&map, &[], Settings::default());
    let eye = [0.0, 0.0, 0.0];
    // Off at first.
    assert!(d.sight_candidates(eye, [0.0, 1.0, 0.0]).is_empty());
    d.fire("ArmLOS", "test");
    run(&mut d, 0.05);
    // Looks at the seen actor, not the trigger's own place.
    assert!(d.sight_candidates(eye, [1.0, 0.0, 0.0]).is_empty());
    let c = d.sight_candidates(eye, [0.0, 1.0, 0.0]);
    assert_eq!(c, vec![(0, [0.0, 800.0, 0.0])]);
    let save = d.snapshot();
    assert_eq!(save.sight.len(), 1);
    assert!(save.sight[0].enabled && !save.sight[0].triggered);
    d.look(eye, [0.0, 1.0, 0.0], &mut |_, _| true);
    run(&mut d, 0.05);
    assert!(sent(&d).contains(&"Seen".to_string()));
    // A load from before it was seen makes it ready again.
    let mut fresh = Director::new(&map, &[], Settings::default());
    fresh.restore(&save);
    assert_eq!(
        fresh.sight_triggers(),
        vec![("LOS1".to_string(), true, false)]
    );
}

/// A watched character that moved: the game says where it is now.
#[test]
fn line_of_sight_trigger_follows_a_moving_character() {
    let mut guy = actor(
        obj(
            61,
            "TrandoshanScav",
            "TrandoshanScav0",
            &[("Tag", t("Dragger1"))],
        ),
        vec![],
    );
    guy.location = [1000.0, 0.0, 0.0];
    let map = MapScripts {
        actors: vec![
            sight(
                "LineOfSightTrigger1",
                &[
                    ("Event", t("Dragger1Seen")),
                    ("SeenActorTag", t("Dragger1")),
                    ("MaxViewAngle", Val::Int(30)),
                ],
                [0.0; 3],
            ),
            guy,
        ],
        ..Default::default()
    };
    let d = Director::new(&map, &[], Settings::default());
    let eye = [0.0; 3];
    // Placed along +X; it walked to +Y.
    let moved = |tag: &str| (tag == "dragger1").then_some([0.0, 1000.0, 0.0]);
    assert_eq!(d.sight_candidates(eye, [1.0, 0.0, 0.0]).len(), 1);
    assert!(d
        .sight_candidates_live(eye, [1.0, 0.0, 0.0], &moved)
        .is_empty());
    assert_eq!(
        d.sight_candidates_live(eye, [0.0, 1.0, 0.0], &moved),
        vec![(0, [0.0, 1000.0, 0.0])]
    );
}

/// gameplay.u's `ACTION_PlayPropAnim` is a plain `ScriptedAction`: the
/// script goes straight on (geo_01a's intro plays three of them before its
/// `SpawnLandingZone`).
#[test]
fn play_prop_anim_does_not_hold_the_script() {
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
                "Intro",
                vec![
                    act("WaitForEvent", &[("ExternalEvent", t("levelstart"))]),
                    act(
                        "PlayPropAnim",
                        &[
                            ("AnimPropTag", t("PlayerCommando")),
                            ("BaseAnim", t("ActionBreathe")),
                        ],
                    ),
                    act(
                        "PlayPropAnim",
                        &[
                            ("AnimPropTag", t("CloneRope62")),
                            ("BaseAnim", t("ActionBreathe")),
                        ],
                    ),
                    act("TriggerEvent", &[("Event", t("SpawnLandingZone"))]),
                ],
            ),
        ],
        failures: vec![],
        sounds: Default::default(),
    };
    let mut d = Director::new(&map, &[], Settings::default());
    d.start();
    run(&mut d, 0.2);
    assert_eq!(sent(&d), ["levelstart", "SpawnLandingZone"]);
}
