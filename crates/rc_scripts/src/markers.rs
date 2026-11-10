//! Squad marker anchors: the map actors a squad marker's job is for
//! (a demolition charge, a console or door lock to slice, a door lock to
//! breach), and the map events each one sends as the job goes on. Read
//! here with no Bevy; the game carries them out in `rc_scripts_plugin`
//! (`markers.rs`, which describes the settings), and `rc_script_check`
//! lists them for every map.

use rc_level::scripts::{MapScripts, ScriptObject, Val};

/// What kind of thing a marker's anchor is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorKind {
    DemolitionCharge,
    HackTerminal,
    HackDoor,
    BreachDoor,
    /// Any other anchor with an `EventUnderway` (a sniper post's).
    Other,
}

impl AnchorKind {
    fn of(class: &str) -> AnchorKind {
        let c = class.to_ascii_lowercase();
        if c.starts_with("demolitioncharge") {
            AnchorKind::DemolitionCharge
        } else if c.starts_with("hackterminal") {
            AnchorKind::HackTerminal
        } else if c.starts_with("hackdoor") {
            AnchorKind::HackDoor
        } else if c.starts_with("breachdoor") {
            AnchorKind::BreachDoor
        } else {
            AnchorKind::Other
        }
    }
}

/// One marker anchor and what it sets off.
#[derive(Debug, Clone)]
pub struct Anchor {
    pub name: String,
    pub class: String,
    pub kind: AnchorKind,
    /// Events sent at each step: started, armed, done.
    pub started: Vec<String>,
    pub armed: Vec<String>,
    pub done: Vec<String>,
    /// The prop a demolition charge blows away (actor name), and its
    /// export (to hide it and take away its collision).
    pub destroys: Option<(String, Option<usize>)>,
    /// Doors (mover actor names) that open when it is done.
    pub doors: Vec<String>,
    /// Steps already sent (started, armed, done).
    pub sent: [bool; 3],
}

impl Anchor {
    /// Who the step is for, as words for the log.
    pub fn what(&self) -> &'static str {
        match self.kind {
            AnchorKind::DemolitionCharge => "demolition charge",
            AnchorKind::HackTerminal => "console",
            AnchorKind::HackDoor => "door lock (slice)",
            AnchorKind::BreachDoor => "door lock (breach)",
            AnchorKind::Other => "marker",
        }
    }
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

fn list(o: &ScriptObject, key: &str) -> Vec<String> {
    match o.get(key) {
        Some(Val::List(l)) => l
            .iter()
            .filter(|n| !n.eq_ignore_ascii_case("None"))
            .cloned()
            .collect(),
        _ => Vec::new(),
    }
}

/// The doors a lock actor opens: `DoorLeft` / `DoorRight`, else the
/// double-door movers it touches.
fn doors_of(o: &ScriptObject) -> Vec<String> {
    let mut doors: Vec<String> = ["DoorLeft", "DoorRight"]
        .iter()
        .filter_map(|k| text(o, k))
        .collect();
    if doors.is_empty() {
        doors = list(o, "Touching")
            .into_iter()
            .filter(|n| n.to_ascii_lowercase().starts_with("moverdb"))
            .collect();
    }
    doors
}

/// Read every squad marker's anchor from the map. Anchors with nothing
/// to send or open are left out.
pub fn read_anchors(map: &MapScripts) -> Vec<Anchor> {
    let find = |name: &str| {
        map.actors
            .iter()
            .find(|a| a.obj.name.eq_ignore_ascii_case(name))
    };
    let mut out: Vec<Anchor> = Vec::new();
    // Order markers name their anchor.
    for m in &map.actors {
        let mc = m.obj.class.to_ascii_lowercase();
        if !mc.starts_with("marker") {
            continue;
        }
        let Some(anchor_name) = text(&m.obj, "Anchor") else {
            continue;
        };
        let Some(a) = find(&anchor_name) else {
            continue;
        };
        let o = &a.obj;
        let marker_underway = text(&m.obj, "EventUnderway");
        if let Some(existing) = out
            .iter_mut()
            .find(|x| x.name.eq_ignore_ascii_case(&o.name))
        {
            existing.started.extend(marker_underway);
            continue;
        }
        let kind = AnchorKind::of(&o.class);
        let mut started: Vec<String> = text(o, "EventUnderway").into_iter().collect();
        started.extend(marker_underway);
        let mut armed = Vec::new();
        let mut done = Vec::new();
        let mut destroys = None;
        let mut doors = Vec::new();
        match kind {
            AnchorKind::DemolitionCharge => {
                armed.extend(state_event(o, "ChargeArmed"));
                done.extend(state_event(o, "ChargeExplode"));
                destroys = text(o, "PropToDestroy").map(|p| {
                    let export = find(&p).map(|a| a.obj.export);
                    (p, export)
                });
            }
            AnchorKind::HackTerminal => {
                started.extend(state_event(o, "ActiveState"));
                done.extend(state_event(o, "HackedState"));
            }
            AnchorKind::HackDoor | AnchorKind::BreachDoor => {
                if kind == AnchorKind::BreachDoor {
                    done.extend(state_event(o, "DoorDamagedState"));
                }
                done.extend(state_event(o, "DoorUnlockedState"));
                doors = doors_of(o);
                // The other lock on the same door (slice or breach).
                if let Some(partner) = text(o, "MatchingComponent").and_then(|p| find(&p)) {
                    for d in doors_of(&partner.obj) {
                        if !doors.iter().any(|x| x.eq_ignore_ascii_case(&d)) {
                            doors.push(d);
                        }
                    }
                }
            }
            AnchorKind::Other => {}
        }
        if started.is_empty() && armed.is_empty() && done.is_empty() && doors.is_empty() {
            continue;
        }
        out.push(Anchor {
            name: o.name.clone(),
            class: o.class.clone(),
            kind,
            started,
            armed,
            done,
            destroys,
            doors,
            sent: [false; 3],
        });
    }
    out
}
