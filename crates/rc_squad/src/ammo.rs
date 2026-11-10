//! Optional squad ammo: squad mates who can run out, and who walk to the
//! level's ammo and grenade boxes to fill up. Off by default.
//!
//! The original doesn't do this (docs/RESEARCH.md, "Squad ammo and ammo
//! boxes"): only the player can take a box, and a squad mate whose gun runs
//! dry gets a fresh magazine for free. So with `[ammo] limited = "no"` (the
//! shipped setting) nothing here does anything, and the squad never runs
//! out, like the original. `limited = "yes"` is for mods that want squad
//! mates to count their shots. Every rule and number in this file is then
//! our own.
//!
//! With it on:
//!
//! - Each squad mate carries the guns and grenades listed in `[ammo]`
//!   (`carry_<id> = most, start`), starting with `start`. Anything not
//!   listed never runs out.
//! - Each shot takes one from the gun fired, each grenade thrown (on their
//!   own or from a grenade post) one from `throws`. An empty speciality gun
//!   isn't picked; with the usual gun empty they hold fire.
//! - A squad mate on no job, with a gun at or below `fetch_below` of what it
//!   holds, walks to the nearest box that fills it, if the box is within
//!   `fetch_range` of them and of the spot they're tied to. In a fight they
//!   only go once the gun is empty, unless `fetch_in_fights` is "yes". Two
//!   squad mates never go for the same box. After `fetch_give_up` seconds
//!   without reaching it they leave that box alone, and go back to their
//!   place for as long again before trying another.
//! - A box is taken by touching it (the game part checks this, as for the
//!   player): it fills only ammo that isn't full, up to the most carried,
//!   and is used up.

use crate::settings::{one, yes_no, Value};
use crate::squad::{Leader, MemberState, Squad, SquadEvent};
use rc_movement::Vec3;
use std::collections::BTreeMap;

/// The `[ammo]` section of `squad.toml`.
#[derive(Debug, Clone, PartialEq)]
pub struct AmmoSettings {
    /// Squad mates count their shots at all. Off: they never run out (the
    /// original's rule).
    pub limited: bool,
    /// Gun or grenade id (a file in data/weapons or data/grenades, as
    /// `data/pickups/ammo_boxes.toml` names them) to (most carried, at the
    /// start).
    pub carry: BTreeMap<String, (u32, u32)>,
    /// What grenades thrown by squad mates use up.
    pub throws: String,
    /// Go for a box at or below this share of the most carried.
    pub fetch_below: f32,
    /// How far a box may be from them, and from what they're tied to, uu.
    pub fetch_range: f32,
    /// Go for a box in the middle of a fight before the gun is empty.
    pub fetch_in_fights: bool,
    /// Seconds before they give up on a box they can't reach.
    pub fetch_give_up: f32,
}

impl Default for AmmoSettings {
    /// The values of the shipped `data/squad/squad.toml`.
    fn default() -> Self {
        let carry = [
            // DC17mBlasterAmmoCommando MaxAmmo 300; DC17mBlasterCommando
            // PickupAmmoCount 120.
            ("dc17m_blaster", (300, 120)),
            // DC17mSniperAmmo MaxAmmo 20, PickupAmmoCount 20.
            ("dc17m_sniper", (20, 20)),
            // DC17mAntiArmorAmmoCommando MaxAmmo 4, PickupAmmoCount 4.
            ("dc17m_antiarmor", (4, 4)),
            // ThermalDetonator MaxAmmo 5, InitAmmoAmount 2.
            ("thermal_detonator", (5, 2)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        AmmoSettings {
            limited: false,
            carry,
            throws: "thermal_detonator".into(),
            fetch_below: 0.25,
            fetch_range: 1500.0,
            fetch_in_fights: false,
            fetch_give_up: 15.0,
        }
    }
}

pub(crate) fn set_ammo(a: &mut AmmoSettings, key: &str, value: Value) -> Result<(), String> {
    if let Some(id) = key.strip_prefix("carry_") {
        let Value::Numbers(v) = value else {
            return Err(format!("{key} must be two numbers: most, start"));
        };
        let [most, start] = v[..] else {
            return Err(format!("{key} must be two numbers: most, start"));
        };
        if most < 0.0 || start < 0.0 || start > most {
            return Err(format!("{key}: start must be from 0 to most"));
        }
        a.carry
            .insert(id.to_ascii_lowercase(), (most as u32, start as u32));
        return Ok(());
    }
    match key {
        "limited" => a.limited = yes_no(key, value)?,
        "fetch_in_fights" => a.fetch_in_fights = yes_no(key, value)?,
        "throws" => match value {
            Value::Text(t) => a.throws = t.to_ascii_lowercase(),
            _ => return Err("throws must be \"text\"".into()),
        },
        "fetch_below" => a.fetch_below = one(value)?,
        "fetch_range" => a.fetch_range = one(value)?,
        "fetch_give_up" => a.fetch_give_up = one(value)?,
        _ => return Err(format!("unknown setting `{key}` in [ammo]")),
    }
    Ok(())
}

impl AmmoSettings {
    pub fn summary(&self) -> String {
        if !self.limited {
            return "ammo: unlimited (as the original)".into();
        }
        let carry: Vec<String> = self
            .carry
            .iter()
            .map(|(k, (m, s))| format!("{k} {s}/{m}"))
            .collect();
        format!(
            "ammo: limited, carry {}, throws use {}, fetch boxes at {:.0}% within {} uu{}, give up after {} s",
            carry.join(", "),
            self.throws,
            self.fetch_below * 100.0,
            self.fetch_range,
            if self.fetch_in_fights { " (also in fights)" } else { "" },
            self.fetch_give_up
        )
    }

    /// What a squad mate starts with: nothing tracked when unlimited.
    pub fn starting(&self) -> BTreeMap<String, u32> {
        if !self.limited {
            return BTreeMap::new();
        }
        self.carry
            .iter()
            .map(|(k, (_, s))| (k.clone(), *s))
            .collect()
    }
}

/// An ammo or grenade box lying in the level, as the squad sees it. The
/// game part fills `Squad::boxes` every frame with the boxes not taken.
#[derive(Debug, Clone, PartialEq)]
pub struct SupplyBox {
    /// Stays the same for as long as the box exists.
    pub id: u64,
    /// Its name in the map, for the log.
    pub name: String,
    /// The floor under it, uu (where to walk to).
    pub feet: Vec3,
    /// The gun or grenade id it fills.
    pub gives: String,
    pub amount: u32,
}

fn dist(a: Vec3, b: Vec3) -> f32 {
    (a - b).length()
}

impl Squad {
    /// Shots or grenades member `i` has of `id`; `None` if they never run
    /// out of it.
    pub fn ammo_of(&self, i: usize, id: &str) -> Option<u32> {
        self.members.get(i)?.ammo.get(id).copied()
    }

    /// Member `i` can fire (or throw) `id` now.
    pub(crate) fn has_ammo(&self, i: usize, id: &str) -> bool {
        self.ammo_of(i, id).is_none_or(|n| n > 0)
    }

    /// One shot or grenade of `id` used.
    pub(crate) fn use_ammo(&mut self, i: usize, id: &str, events: &mut Vec<SquadEvent>) {
        let m = &mut self.members[i];
        let Some(n) = m.ammo.get_mut(id) else {
            return;
        };
        *n = n.saturating_sub(1);
        if *n == 0 {
            events.push(SquadEvent::OutOfAmmo {
                member: m.settings.name.clone(),
                weapon: id.to_string(),
            });
        }
    }

    /// Room member `i` has for more `id`: 0 if full or not counted.
    pub fn ammo_room(&self, i: usize, id: &str) -> u32 {
        let (Some(n), Some((most, _))) = (self.ammo_of(i, id), self.settings.ammo.carry.get(id))
        else {
            return 0;
        };
        most.saturating_sub(n)
    }

    /// Member `i` touches box `b`: they take it if it fills something not
    /// full. Returns the event (the game then marks the box taken), or
    /// `None` if they leave it lying.
    pub fn take_box(&mut self, i: usize, b: &SupplyBox) -> Option<SquadEvent> {
        let room = self.ammo_room(i, &b.gives);
        let m = self.members.get_mut(i)?;
        if m.down {
            return None;
        }
        if room == 0 {
            return None;
        }
        let added = b.amount.min(room);
        let n = m.ammo.get_mut(&b.gives)?;
        *n += added;
        let now = *n;
        if m.fetching == Some(b.id) {
            m.fetching = None;
        }
        Some(SquadEvent::AmmoTaken {
            member: m.settings.name.clone(),
            index: i,
            name: b.name.clone(),
            gives: b.gives.clone(),
            added,
            now,
            max: now + room - added,
        })
    }

    /// The gun member `i` most needs filled, and whether it's empty: the
    /// lowest share of the most carried, at or below `fetch_below`.
    fn most_needed(&self, i: usize) -> Option<(String, bool)> {
        // (the gun, and whether it's empty)
        let a = &self.settings.ammo;
        self.members[i]
            .ammo
            .iter()
            .filter_map(|(id, &n)| {
                let (most, _) = *a.carry.get(id)?;
                if most == 0 {
                    return None;
                }
                let share = n as f32 / most as f32;
                (share <= a.fetch_below).then(|| (id.clone(), n == 0, share))
            })
            .min_by(|x, y| x.2.total_cmp(&y.2))
            .map(|(id, empty, _)| (id, empty))
    }

    /// Member `i` has an enemy to fight, or one within sight.
    fn in_fight(&self, i: usize) -> bool {
        let m = &self.members[i];
        let sight = self.settings.fight.sight_radius;
        m.fight.target.is_some()
            || self
                .threats
                .iter()
                .any(|t| dist(t.feet, m.pawn.feet()) <= sight)
    }

    /// Member `i` may leave a fight for a box of `gives`: only once it's
    /// empty, unless `fetch_in_fights`.
    fn may_leave_fight(&self, i: usize, gives: &str) -> bool {
        self.settings.ammo.fetch_in_fights || !self.in_fight(i) || self.ammo_of(i, gives) == Some(0)
    }

    fn stop_fetching(&mut self, i: usize, why: &str, events: &mut Vec<SquadEvent>) {
        let m = &mut self.members[i];
        if let Some(id) = m.fetching.take() {
            let name = self
                .boxes
                .iter()
                .find(|b| b.id == id)
                .map_or_else(|| format!("box {id}"), |b| b.name.clone());
            events.push(SquadEvent::FetchStopped {
                member: m.settings.name.clone(),
                name,
                why: why.into(),
            });
            m.state = MemberState::Idle;
            m.planned_for = None;
        }
    }

    /// Called on each think, before the usual one. Picks a box to go to
    /// (or keeps going to one) and steers member `i` there. `true` while
    /// they're on the way, so the usual think leaves them be.
    pub(crate) fn think_fetch(
        &mut self,
        i: usize,
        tether: Vec3,
        _leader: Leader,
        events: &mut Vec<SquadEvent>,
    ) -> bool {
        if !self.settings.ammo.limited {
            return false;
        }
        let m = &self.members[i];
        let free = !m.down
            && m.marker.is_none()
            && m.revive.is_none()
            && !matches!(
                m.state,
                MemberState::Working { .. }
                    | MemberState::Reviving { .. }
                    | MemberState::Ready
                    | MemberState::Posted
            );
        if !free {
            self.stop_fetching(i, "busy with something else", events);
            return false;
        }
        let interval = self.settings.think_interval;
        if let Some(id) = self.members[i].fetching {
            let Some(b) = self.boxes.iter().find(|b| b.id == id).cloned() else {
                self.stop_fetching(i, "someone else took it", events);
                return false;
            };
            if self.ammo_room(i, &b.gives) == 0 {
                self.stop_fetching(i, "full again", events);
                return false;
            }
            if !self.may_leave_fight(i, &b.gives) {
                self.stop_fetching(i, "an enemy to fight first", events);
                return false;
            }
            let give_up = self.settings.ammo.fetch_give_up;
            let m = &mut self.members[i];
            m.fetch_time += interval;
            if m.fetch_time > give_up {
                m.skip_boxes.push(id);
                // Back to their place for a while before trying another
                // box, rather than wandering from box to box.
                m.fetch_wait = give_up;
                self.stop_fetching(i, "can't get to it", events);
                return false;
            }
            let m = &mut self.members[i];
            m.goal = b.feet;
            m.engaging = None;
            m.cover = None;
            if !matches!(m.state, MemberState::Moving { .. }) {
                m.state = MemberState::Moving {
                    urgent: false,
                    by_order: true,
                };
            }
            return true;
        }
        let m = &mut self.members[i];
        if m.fetch_wait > 0.0 {
            m.fetch_wait -= interval;
            return false;
        }
        let Some((gives, _)) = self.most_needed(i) else {
            return false;
        };
        if !self.may_leave_fight(i, &gives) {
            return false;
        }
        let a = &self.settings.ammo;
        let m = &self.members[i];
        let feet = m.pawn.feet();
        let taken: Vec<u64> = self
            .members
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .filter_map(|(_, o)| o.fetching)
            .collect();
        let best = self
            .boxes
            .iter()
            .filter(|b| b.gives == gives)
            .filter(|b| !taken.contains(&b.id) && !m.skip_boxes.contains(&b.id))
            .filter(|b| {
                dist(b.feet, feet) <= a.fetch_range && dist(b.feet, tether) <= a.fetch_range
            })
            .min_by(|x, y| dist(x.feet, feet).total_cmp(&dist(y.feet, feet)))
            .cloned();
        let Some(b) = best else {
            return false;
        };
        let have = m.ammo.get(&gives).copied().unwrap_or(0);
        let m = &mut self.members[i];
        m.fetching = Some(b.id);
        m.fetch_time = 0.0;
        m.goal = b.feet;
        m.engaging = None;
        m.cover = None;
        m.planned_for = None;
        m.state = MemberState::Moving {
            urgent: false,
            by_order: true,
        };
        events.push(SquadEvent::FetchAmmo {
            member: m.settings.name.clone(),
            name: b.name.clone(),
            gives,
            have,
            distance: dist(b.feet, feet),
        });
        true
    }
}
