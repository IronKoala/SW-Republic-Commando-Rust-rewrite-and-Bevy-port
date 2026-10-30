//! Where a map saves the game: its `Checkpoint` actors (class
//! `CTGame.Checkpoint`), and what sets each one off.
//!
//! In the original a checkpoint does nothing until an event with its `Tag`
//! fires (from a trigger the player walks into, or from a level script).
//! Then it waits until the player is up (not incapacitated) and autosaves.
//! This reads the checkpoints and the actors in the map whose `Event`
//! names each one's tag, so the level scripts part can fire them and the
//! save part can name them.

use crate::actors;
use crate::bsp::Vec3;
use rc_package::properties::{self, Value};
use rc_package::Package;

/// One checkpoint in a map.
#[derive(Debug, Clone, PartialEq)]
pub struct Checkpoint {
    /// The actor's name in the map, like `Checkpoint0`. Unique in the map.
    pub name: String,
    /// The event that sets it off (its `Tag`; the class name if unset).
    pub tag: String,
    pub location: Vec3,
    /// Actors whose `Event` is this tag: (class, name), like
    /// `("Trigger", "Trigger12")`. Empty when only a script fires it.
    pub fired_by: Vec<(String, String)>,
}

/// Every checkpoint in a map, in the map's actor order.
pub fn read(pkg: &Package, data: &[u8]) -> Result<Vec<Checkpoint>, String> {
    let level = actors::read_level(pkg, data).map_err(|e| format!("level: {e}"))?;
    let mut found = Vec::new();
    // (class, name, Event) of every actor with an Event set.
    let mut senders: Vec<(String, String, String)> = Vec::new();
    for &a in level.actors.iter().flatten() {
        let e = &pkg.exports[a];
        let class = pkg.export_class_name(e).to_string();
        let name = pkg.name(e.object_name).to_string();
        let Ok(props) = actors::read_actor_properties(pkg, data, a) else {
            continue;
        };
        let name_prop = |n: &str| match properties::find(pkg, &props, n).map(|p| &p.value) {
            Some(Value::Name(i)) => Some(pkg.name(*i).to_string()),
            _ => None,
        };
        if let Some(ev) = name_prop("Event").filter(|e| !e.eq_ignore_ascii_case("None")) {
            senders.push((class.clone(), name.clone(), ev));
        }
        if class.eq_ignore_ascii_case("Checkpoint") {
            let location = actors::read_placement(pkg, data, a)
                .map(|p| p.location)
                .unwrap_or([0.0; 3]);
            found.push(Checkpoint {
                name,
                tag: name_prop("Tag").unwrap_or_else(|| class.clone()),
                location,
                fired_by: Vec::new(),
            });
        }
    }
    for c in &mut found {
        c.fired_by = senders
            .iter()
            .filter(|(_, _, ev)| ev.eq_ignore_ascii_case(&c.tag))
            .map(|(class, name, _)| (class.clone(), name.clone()))
            .collect();
    }
    Ok(found)
}
