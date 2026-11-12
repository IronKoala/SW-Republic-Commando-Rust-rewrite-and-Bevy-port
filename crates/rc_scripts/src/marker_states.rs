//! The states of squad order markers and of their anchors (demolition
//! charges, consoles, door locks), as level scripts switch them with
//! `ACTION_GotoState` and as each one starts the level. No Bevy: this
//! works out what happens ([`StateEffect`]); `rc_scripts_plugin` carries
//! it out (the squad's markers, the doors, the map events).
//!
//! The rules are the original's own script code, read from its packages
//! as text (nothing copied into the port):
//!
//! - engine.u `SquadMarker`: `Enabled` (its start state) can be ordered;
//!   `Disabled` can't (and a squad mate on it leaves when the marker can be
//!   cancelled after it's under way). Triggering a disabled marker
//!   enables it, but no Geonosis map sends an event to a marker, so the
//!   port doesn't listen for that.
//! - ctgame.u `HackTerminal`: `Standby` (start) enables its marker,
//!   `Disabled` makes it obsolete; each sends its state's `Event`.
//! - ctgame.u `ActivateDoor` (`HackDoor*`, `BreachDoor*`): `Locked`
//!   (start) sends `DoorLockedState.Event`, enables its marker and locks
//!   the door; `Disabled` sends `DoorLockedState.Event`, makes the marker
//!   obsolete and locks the door; `Unlocked` makes the marker obsolete,
//!   unlocks the other lock on the same door (`MatchingComponent`) and
//!   the door, and sends `DoorUnlockedState.Event`. Only a breach lock
//!   moves the doors: locking sends `DoorLeft` and `DoorRight` to
//!   `Locked`, `Close` and locks the other lock too; unlocking puts them
//!   in `TriggerControl` (they open while their trigger is touched).
//! - ctmarkers.u `DemolitionCharge`: `Unplaced` (start) enables its
//!   marker; `Disabled` hides the charge and makes the marker obsolete
//!   (leaving `Disabled` shows it again).
//!
//! An anchor only touches its marker while no squad mate is on it
//! (`!SquadMarker.bMarkerActive`), except a disabled charge.
//!
//! At the level's start every actor goes to its `InitialState` (or its
//! start state when it has none), in the map's order, unless another
//! actor already set it up (`bScriptInitialized`, Engine.dll, read
//! locally): an anchor sets up its marker, so the anchor wins.
//! The core's `GotoState` (Core.dll, read locally): going to the state
//! it is already in does nothing; `Auto` is the start state.

use rc_level::scripts::{MapScripts, ScriptObject, Val};

/// What kind of thing has these states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateKind {
    /// An order marker (`MarkerSnipe`, `MarkerDemolition`, ...).
    Marker,
    DemolitionCharge,
    HackTerminal,
    HackDoor,
    BreachDoor,
}

impl StateKind {
    fn of(class: &str) -> Option<StateKind> {
        let c = class.to_ascii_lowercase();
        Some(
            if c.starts_with("marker") && !c.starts_with("markersnipeanchor") {
                StateKind::Marker
            } else if c.starts_with("demolitioncharge") {
                StateKind::DemolitionCharge
            } else if c.starts_with("hackterminal") {
                StateKind::HackTerminal
            } else if c.starts_with("hackdoor") {
                StateKind::HackDoor
            } else if c.starts_with("breachdoor") {
                StateKind::BreachDoor
            } else {
                return None;
            },
        )
    }

    /// The state it starts in (its `auto` state).
    fn auto_state(self) -> &'static str {
        match self {
            StateKind::Marker => "enabled",
            StateKind::DemolitionCharge => "unplaced",
            StateKind::HackTerminal => "standby",
            StateKind::HackDoor | StateKind::BreachDoor => "locked",
        }
    }

    /// The states the port carries out.
    fn known(self, state: &str) -> bool {
        let list: &[&str] = match self {
            StateKind::Marker => &["enabled", "disabled"],
            StateKind::DemolitionCharge => &["unplaced", "disabled"],
            StateKind::HackTerminal => &["standby", "disabled"],
            StateKind::HackDoor | StateKind::BreachDoor => &["locked", "disabled", "unlocked"],
        };
        list.contains(&state)
    }
}

/// Something that follows from a state change.
#[derive(Debug, Clone, PartialEq)]
pub enum StateEffect {
    /// Switch the order marker (actor name) on or off. `only_if_free`:
    /// not while a squad mate is on it.
    Marker {
        name: String,
        on: bool,
        only_if_free: bool,
    },
    /// Send a map event.
    Fire { event: String, by: String },
    /// Doors (mover actor names): lock them shut (`Locked`, `Close`) or
    /// let their trigger open them (`TriggerControl`).
    Doors { names: Vec<String>, lock: bool },
    /// Hide or show an actor (by export).
    Hide { export: usize, hidden: bool },
    /// `ACTION_SetCanCancelMarker` on the order marker (actor name).
    CanCancel { name: String, can: bool },
    /// Something the port doesn't do yet, for the log.
    NotDone(String),
}

/// One actor with marker or anchor states.
#[derive(Debug, Clone)]
pub struct StateActor {
    pub name: String,
    pub class: String,
    pub tag: String,
    pub export: usize,
    pub kind: StateKind,
    /// `InitialState` from the map, lower case (empty for none).
    pub initial: String,
    /// The state it is in, lower case (empty before the start).
    pub state: String,
    /// Set up by another actor (`bScriptInitialized`).
    pub initialized: bool,
    /// An anchor's order marker (actor name).
    pub marker: Option<String>,
    /// A door lock's other lock on the same door.
    pub matching: Option<String>,
    /// A breach lock's doors (`DoorLeft`, `DoorRight`).
    pub doors: Vec<String>,
    /// Events sent on entering a state, by lower-case state name.
    pub events: Vec<(String, String)>,
}

fn text(o: &ScriptObject, key: &str) -> Option<String> {
    o.text(key)
        .map(str::trim)
        .filter(|t| !t.is_empty() && !t.eq_ignore_ascii_case("None"))
        .map(str::to_string)
}

fn state_event(o: &ScriptObject, key: &str) -> Option<String> {
    match o.structure(key)?.get("Event") {
        Some(Val::Text(t)) if !t.trim().is_empty() && !t.eq_ignore_ascii_case("None") => {
            Some(t.trim().to_string())
        }
        _ => None,
    }
}

/// Every order marker and anchor in a map, and the states they're in.
#[derive(Debug, Clone, Default)]
pub struct MarkerStates {
    pub actors: Vec<StateActor>,
}

impl MarkerStates {
    pub fn read(map: &MapScripts) -> MarkerStates {
        let mut actors: Vec<StateActor> = Vec::new();
        for a in &map.actors {
            let o = &a.obj;
            let Some(kind) = StateKind::of(&o.class) else {
                continue;
            };
            let mut events = Vec::new();
            let mut ev = |state: &str, key: &str| {
                if let Some(e) = state_event(o, key) {
                    events.push((state.to_string(), e));
                }
            };
            match kind {
                StateKind::HackTerminal => {
                    ev("standby", "StandbyState");
                    ev("disabled", "DisabledState");
                }
                StateKind::HackDoor | StateKind::BreachDoor => {
                    ev("locked", "DoorLockedState");
                    ev("disabled", "DoorLockedState");
                    ev("unlocked", "DoorUnlockedState");
                }
                StateKind::DemolitionCharge => ev("unplaced", "ChargeUnplaced"),
                StateKind::Marker => {}
            }
            let doors = if kind == StateKind::BreachDoor {
                ["DoorLeft", "DoorRight"]
                    .iter()
                    .filter_map(|k| text(o, k))
                    .collect()
            } else {
                Vec::new()
            };
            actors.push(StateActor {
                name: o.name.clone(),
                class: o.class.clone(),
                tag: o.tag().to_string(),
                export: o.export,
                kind,
                initial: text(o, "InitialState")
                    .unwrap_or_default()
                    .to_ascii_lowercase(),
                state: String::new(),
                initialized: false,
                marker: text(o, "SquadMarker"),
                matching: text(o, "MatchingComponent"),
                doors,
                events,
            });
        }
        // An anchor with no `SquadMarker` of its own: the order marker
        // naming it as its `Anchor`.
        for a in &map.actors {
            if StateKind::of(&a.obj.class) != Some(StateKind::Marker) {
                continue;
            }
            let Some(anchor) = text(&a.obj, "Anchor") else {
                continue;
            };
            if let Some(x) = actors
                .iter_mut()
                .find(|x| x.kind != StateKind::Marker && x.name.eq_ignore_ascii_case(&anchor))
            {
                x.marker.get_or_insert_with(|| a.obj.name.clone());
            }
        }
        MarkerStates { actors }
    }

    fn index(&self, name: &str) -> Option<usize> {
        self.actors
            .iter()
            .position(|a| a.name.eq_ignore_ascii_case(name))
    }

    /// The level starts: each actor goes to its `InitialState` (or its
    /// start state), in the map's order, unless another set it up first.
    pub fn start(&mut self) -> Vec<StateEffect> {
        let mut out = Vec::new();
        for i in 0..self.actors.len() {
            if self.actors[i].initialized {
                continue;
            }
            self.actors[i].initialized = true;
            let state = match self.actors[i].initial.as_str() {
                "" => "auto".to_string(),
                s => s.to_string(),
            };
            self.enter(i, &state, &mut out);
        }
        // Each marker ends up as the last word on it says; nobody is on a
        // marker yet, and markers start switched on, so only those left
        // off need saying.
        let mut last: Vec<(String, bool)> = Vec::new();
        for e in &out {
            if let StateEffect::Marker { name, on, .. } = e {
                match last.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
                    Some(l) => l.1 = *on,
                    None => last.push((name.clone(), *on)),
                }
            }
        }
        out.retain(|e| !matches!(e, StateEffect::Marker { .. }));
        out.extend(
            last.into_iter()
                .filter(|(_, on)| !on)
                .map(|(name, _)| StateEffect::Marker {
                    name,
                    on: false,
                    only_if_free: false,
                }),
        );
        out
    }

    /// `ACTION_GotoState`: every actor tagged `tag` goes to `state`.
    /// Returns how many there are, and what follows.
    pub fn goto_state(&mut self, tag: &str, state: &str) -> (usize, Vec<StateEffect>) {
        let mut out = Vec::new();
        let hits: Vec<usize> = (0..self.actors.len())
            .filter(|&i| !tag.is_empty() && self.actors[i].tag.eq_ignore_ascii_case(tag))
            .collect();
        for &i in &hits {
            self.enter(i, state, &mut out);
        }
        (hits.len(), out)
    }

    /// The order markers tagged `tag` (actor names), for
    /// `ACTION_ShowMarker` and `ACTION_SetCanCancelMarker`.
    pub fn markers_tagged(&self, tag: &str) -> Vec<String> {
        self.actors
            .iter()
            .filter(|a| {
                a.kind == StateKind::Marker && !tag.is_empty() && a.tag.eq_ignore_ascii_case(tag)
            })
            .map(|a| a.name.clone())
            .collect()
    }

    /// Whether any actor here has this tag.
    pub fn has_tag(&self, tag: &str) -> bool {
        self.actors.iter().any(|a| a.tag.eq_ignore_ascii_case(tag))
    }

    /// The anchor's order marker goes to a state: `obsolete` is the
    /// anchor's `MakeObsolete` (to `Disabled`).
    fn set_marker(&mut self, i: usize, on: bool, only_if_free: bool, out: &mut Vec<StateEffect>) {
        let Some(m) = self.actors[i].marker.clone() else {
            return;
        };
        if let Some(k) = self.index(&m) {
            self.actors[k].initialized = true;
            // The squad keeps the marker's real state (it knows whether a
            // squad mate is on it); this one is for the log and `Trigger`.
            self.actors[k].state = if on { "enabled" } else { "disabled" }.into();
        }
        out.push(StateEffect::Marker {
            name: m,
            on,
            only_if_free,
        });
    }

    fn fire(&self, i: usize, state: &str, out: &mut Vec<StateEffect>) {
        let a = &self.actors[i];
        for (s, e) in &a.events {
            if s == state {
                out.push(StateEffect::Fire {
                    event: e.clone(),
                    by: format!("{} ({state})", a.name),
                });
            }
        }
    }

    /// `LockDoor` (only a breach lock does anything).
    fn lock_door(&mut self, i: usize, out: &mut Vec<StateEffect>) {
        if self.actors[i].kind != StateKind::BreachDoor {
            return;
        }
        let doors = self.actors[i].doors.clone();
        if !doors.is_empty() {
            out.push(StateEffect::Doors {
                names: doors,
                lock: true,
            });
        }
        if let Some(k) = self.actors[i].matching.clone().and_then(|m| self.index(&m)) {
            if self.actors[k].state != "locked" {
                self.actors[k].initialized = true;
                self.enter(k, "locked", out);
            }
        }
    }

    /// Actor `i` goes to `state` (`GotoState`).
    fn enter(&mut self, i: usize, state: &str, out: &mut Vec<StateEffect>) {
        let kind = self.actors[i].kind;
        let mut state = state.to_ascii_lowercase();
        if state == "auto" {
            state = kind.auto_state().to_string();
        }
        if self.actors[i].state == state {
            return;
        }
        if !kind.known(&state) {
            out.push(StateEffect::NotDone(format!(
                "{} ({}) to state {state}",
                self.actors[i].name, self.actors[i].class
            )));
            self.actors[i].state = state;
            return;
        }
        let was = std::mem::replace(&mut self.actors[i].state, state.clone());
        match (kind, state.as_str()) {
            (StateKind::Marker, s) => out.push(StateEffect::Marker {
                name: self.actors[i].name.clone(),
                on: s == "enabled",
                only_if_free: false,
            }),
            (StateKind::HackTerminal, "standby") => {
                self.set_marker(i, true, true, out);
                self.fire(i, &state, out);
            }
            (StateKind::HackTerminal, _) => {
                self.set_marker(i, false, true, out);
                self.fire(i, &state, out);
            }
            (StateKind::HackDoor | StateKind::BreachDoor, "locked") => {
                self.fire(i, &state, out);
                self.set_marker(i, true, true, out);
                self.lock_door(i, out);
            }
            (StateKind::HackDoor | StateKind::BreachDoor, "disabled") => {
                self.fire(i, &state, out);
                self.set_marker(i, false, true, out);
                self.lock_door(i, out);
            }
            (StateKind::HackDoor | StateKind::BreachDoor, _) => {
                self.set_marker(i, false, true, out);
                if let Some(k) = self.actors[i].matching.clone().and_then(|m| self.index(&m)) {
                    if self.actors[k].state != "unlocked" {
                        self.actors[k].initialized = true;
                        self.enter(k, "unlocked", out);
                    }
                }
                if kind == StateKind::BreachDoor && !self.actors[i].doors.is_empty() {
                    out.push(StateEffect::Doors {
                        names: self.actors[i].doors.clone(),
                        lock: false,
                    });
                }
                self.fire(i, &state, out);
            }
            (StateKind::DemolitionCharge, "unplaced") => {
                if was == "disabled" {
                    out.push(StateEffect::Hide {
                        export: self.actors[i].export,
                        hidden: false,
                    });
                }
                self.fire(i, &state, out);
                self.set_marker(i, true, true, out);
            }
            (StateKind::DemolitionCharge, _) => {
                out.push(StateEffect::Hide {
                    export: self.actors[i].export,
                    hidden: true,
                });
                self.set_marker(i, false, false, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rc_level::scripts::{ScriptActor, Vals};

    fn actor(export: usize, class: &str, name: &str, vals: &[(&str, &str)]) -> ScriptActor {
        let mut values = Vals::new();
        for (k, v) in vals {
            let val = if k.ends_with("State") && *k != "InitialState" {
                let mut s = Vals::new();
                s.insert("Event".into(), Val::Text(v.to_string()));
                Val::Struct(s)
            } else {
                Val::Text(v.to_string())
            };
            values.insert(k.to_string(), val);
        }
        ScriptActor {
            obj: ScriptObject {
                export,
                class: class.into(),
                name: name.into(),
                values,
            },
            location: [0.0; 3],
            actions: vec![],
        }
    }

    /// geo_03d's control room door: a slice lock and a breach lock on one
    /// double door, both starting `Disabled`, both tagged `LogicalDoor`;
    /// geo_03d's shield console starting `Disabled`; and a charge.
    fn map() -> MapScripts {
        MapScripts {
            actors: vec![
                actor(
                    1,
                    "HackDoorGeo",
                    "HackDoorGeo6",
                    &[
                        ("InitialState", "Disabled"),
                        ("MatchingComponent", "BreachDoorGeo6"),
                        ("SquadMarker", "MarkerHackDoor53"),
                        ("Tag", "LogicalDoor"),
                    ],
                ),
                actor(
                    2,
                    "BreachDoorGeo",
                    "BreachDoorGeo6",
                    &[
                        ("InitialState", "Disabled"),
                        ("DoorLeft", "MoverDB_GEO6"),
                        ("DoorRight", "MoverDBSlave_GEO6"),
                        ("DoorUnlockedState", "DoorOpenEvent"),
                        ("MatchingComponent", "HackDoorGeo6"),
                        ("SquadMarker", "MarkerDoorBreach53"),
                        ("Tag", "LogicalDoor"),
                    ],
                ),
                actor(
                    3,
                    "MarkerHackDoor",
                    "MarkerHackDoor53",
                    &[("Anchor", "HackDoorGeo6")],
                ),
                actor(
                    4,
                    "MarkerDoorBreach",
                    "MarkerDoorBreach53",
                    &[("Anchor", "BreachDoorGeo6")],
                ),
                actor(
                    5,
                    "HackTerminalGeo",
                    "HackTerminalGeo4",
                    &[("InitialState", "Disabled"), ("Tag", "Shield03")],
                ),
                actor(
                    6,
                    "MarkerHackTerminal",
                    "MarkerHackTerminal3",
                    &[("Anchor", "HackTerminalGeo4")],
                ),
                actor(
                    7,
                    "DemolitionCharge",
                    "DemolitionCharge5",
                    &[("Tag", "Charge5")],
                ),
                actor(
                    8,
                    "MarkerDemolition",
                    "MarkerDemolition5",
                    &[("Anchor", "DemolitionCharge5"), ("Tag", "Demo5")],
                ),
                // A sniper post: its anchor doesn't set it, so its own
                // InitialState counts.
                actor(
                    9,
                    "MarkerSnipe",
                    "MarkerSnipe2",
                    &[("InitialState", "Disabled"), ("Tag", "Snipe2")],
                ),
            ],
            failures: vec![],
            sounds: Default::default(),
        }
    }

    fn marker(name: &str, on: bool, only_if_free: bool) -> StateEffect {
        StateEffect::Marker {
            name: name.into(),
            on,
            only_if_free,
        }
    }

    #[test]
    fn the_level_starts_them() {
        let mut s = MarkerStates::read(&map());
        assert_eq!(s.actors.len(), 9);
        let fx = s.start();
        // The disabled door locks shut (once: the slice lock leaves the
        // doors to the breach lock).
        let doors: Vec<_> = fx
            .iter()
            .filter(|e| matches!(e, StateEffect::Doors { .. }))
            .collect();
        assert_eq!(
            doors,
            vec![&StateEffect::Doors {
                names: vec!["MoverDB_GEO6".into(), "MoverDBSlave_GEO6".into()],
                lock: true
            }]
        );
        // Markers left off: the breach lock's, the console's, the
        // sniper's. The slice lock doesn't stay disabled: the breach lock
        // locking the door sends its other lock to `Locked` (ctgame.u
        // `BreachDoor.LockDoor`), which enables its marker. That is what
        // the original's code does, whichever of the two comes first.
        let mut off: Vec<String> = fx
            .iter()
            .filter_map(|e| match e {
                StateEffect::Marker {
                    name, on: false, ..
                } => Some(name.clone()),
                _ => None,
            })
            .collect();
        off.sort();
        assert_eq!(
            off,
            ["MarkerDoorBreach53", "MarkerHackTerminal3", "MarkerSnipe2"]
        );
        assert_eq!(s.actors[0].state, "locked");
        assert_eq!(s.actors[1].state, "disabled");
        assert!(!fx
            .iter()
            .any(|e| matches!(e, StateEffect::Marker { on: true, .. })));
    }

    #[test]
    fn scripts_switch_them() {
        let mut s = MarkerStates::read(&map());
        s.start();
        // Unlock the door: both locks (same tag), their markers made
        // obsolete (unless a squad mate is on them), the doors left to
        // their trigger, the breach lock's event sent once.
        let (n, fx) = s.goto_state("LogicalDoor", "Unlocked");
        assert_eq!(n, 2);
        assert!(fx.contains(&marker("MarkerHackDoor53", false, true)));
        assert!(fx.contains(&marker("MarkerDoorBreach53", false, true)));
        assert!(fx.contains(&StateEffect::Doors {
            names: vec!["MoverDB_GEO6".into(), "MoverDBSlave_GEO6".into()],
            lock: false
        }));
        let sent: Vec<_> = fx
            .iter()
            .filter(|e| matches!(e, StateEffect::Fire { .. }))
            .collect();
        assert_eq!(sent.len(), 1, "{fx:?}");
        // Already unlocked: nothing more.
        assert_eq!(s.goto_state("LogicalDoor", "Unlocked").1, vec![]);
        // Locked (the start state, `Auto`): markers on again, doors shut.
        let (_, fx) = s.goto_state("LogicalDoor", "Auto");
        assert!(fx.contains(&marker("MarkerHackDoor53", true, true)));
        assert!(fx.contains(&marker("MarkerDoorBreach53", true, true)));
        assert!(fx.contains(&StateEffect::Doors {
            names: vec!["MoverDB_GEO6".into(), "MoverDBSlave_GEO6".into()],
            lock: true
        }));
        // The console comes on.
        let (_, fx) = s.goto_state("Shield03", "Standby");
        assert_eq!(fx, vec![marker("MarkerHackTerminal3", true, true)]);
        // A charge disabled is hidden and its marker off, whoever is on
        // it; back to its start state it shows again.
        let (_, fx) = s.goto_state("Charge5", "Disabled");
        assert_eq!(
            fx,
            vec![
                StateEffect::Hide {
                    export: 7,
                    hidden: true
                },
                marker("MarkerDemolition5", false, false)
            ]
        );
        let (_, fx) = s.goto_state("Charge5", "Auto");
        assert_eq!(
            fx,
            vec![
                StateEffect::Hide {
                    export: 7,
                    hidden: false
                },
                marker("MarkerDemolition5", true, true)
            ]
        );
        // A marker itself.
        assert_eq!(
            s.goto_state("Snipe2", "Enabled").1,
            vec![marker("MarkerSnipe2", true, false)]
        );
        assert_eq!(s.markers_tagged("snipe2"), ["MarkerSnipe2"]);
        // A state the port doesn't carry out, and a tag nobody has.
        let (n, fx) = s.goto_state("Shield03", "Hacked");
        assert_eq!(n, 1);
        assert!(matches!(fx[..], [StateEffect::NotDone(_)]));
        assert_eq!(s.goto_state("Nobody", "Disabled").0, 0);
    }
}
