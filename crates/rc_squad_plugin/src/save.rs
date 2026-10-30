//! The `[squad]` section of a save file: where each squad mate is, what
//! they're doing, whether they're down or reviving someone, their health
//! and shield, and the squad's stance and orders (see
//! `rc_game_core::save`).
//!
//! The squad is made on its first frame of play (`spawn_squad`), after the
//! save is handed out, so `load` keeps the section in [`PendingSquadSave`]
//! and [`apply_pending`] puts it back right after the squad appears.

use crate::{yaw_rotation, SquadMate, SquadState};
use bevy::prelude::*;
use rc_combat::{Dead, Health, Shield};
use rc_game_core::save::SavePart;
use rc_game_core::units::to_bevy;
use rc_game_core::GameLog;
use rc_movement::Vec3 as UVec3;
use rc_squad::{MarkerState, MemberState, Patient, Stance};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SquadSave {
    /// form_up, secure_position, search_and_destroy or hold.
    pub stance: String,
    /// Where the squad was sent (secure position), Unreal units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<[f32; 3]>,
    pub leader_down: bool,
    /// A mate the player is reviving: (index, seconds left).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player_revive: Option<(usize, f32)>,
    /// Each marked spot on the map, in order: open, done, or the index of
    /// the mate on it ("mate 2").
    pub markers: Vec<String>,
    pub members: Vec<MemberSave>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemberSave {
    /// Sev, Fixer or Scorch: checked on load, so the right mate gets it.
    pub name: String,
    pub feet: [f32; 3],
    pub yaw: f32,
    /// idle, moving, urgent, working or reviving.
    pub state: String,
    /// Seconds left of a job or a revive.
    #[serde(default)]
    pub left: f32,
    pub goal: [f32; 3],
    pub hold_point: [f32; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker: Option<usize>,
    pub down: bool,
    /// Who they're on the way to revive: "leader" or "mate N".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revive: Option<String>,
    pub health: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shield: Option<f32>,
}

/// A loaded `[squad]` section waiting for the squad to appear.
#[derive(Resource, Debug, Clone)]
pub struct PendingSquadSave(pub SquadSave);

fn arr(v: UVec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

fn vec([x, y, z]: [f32; 3]) -> UVec3 {
    UVec3::new(x, y, z)
}

fn stance_name(s: Stance) -> &'static str {
    match s {
        Stance::FormUp => "form_up",
        Stance::SecurePosition => "secure_position",
        Stance::SearchAndDestroy => "search_and_destroy",
        Stance::Hold => "hold",
    }
}

fn stance(name: &str) -> Result<Stance, String> {
    Ok(match name {
        "form_up" => Stance::FormUp,
        "secure_position" => Stance::SecurePosition,
        "search_and_destroy" => Stance::SearchAndDestroy,
        "hold" => Stance::Hold,
        other => return Err(format!("unknown stance {other:?}")),
    })
}

fn state_parts(s: MemberState) -> (&'static str, f32) {
    match s {
        MemberState::Idle => ("idle", 0.0),
        MemberState::Moving { urgent: false, .. } => ("moving", 0.0),
        MemberState::Moving { urgent: true, .. } => ("urgent", 0.0),
        MemberState::Working { left } => ("working", left),
        MemberState::Reviving { left } => ("reviving", left),
    }
}

fn member_state(name: &str, left: f32) -> MemberState {
    match name {
        "working" => MemberState::Working { left },
        "reviving" => MemberState::Reviving { left },
        "urgent" => MemberState::Moving {
            urgent: true,
            by_order: false,
        },
        "moving" => MemberState::Moving {
            urgent: false,
            by_order: false,
        },
        _ => MemberState::Idle,
    }
}

fn patient_name(p: Patient) -> String {
    match p {
        Patient::Leader => "leader".into(),
        Patient::Mate(i) => format!("mate {i}"),
    }
}

fn patient(name: &str) -> Option<Patient> {
    if name == "leader" {
        return Some(Patient::Leader);
    }
    name.strip_prefix("mate ")?.parse().ok().map(Patient::Mate)
}

fn marker_name(m: MarkerState) -> String {
    match m {
        MarkerState::Open => "open".into(),
        MarkerState::Done => "done".into(),
        MarkerState::Assigned(i) => format!("mate {i}"),
    }
}

fn marker(name: &str) -> MarkerState {
    match name.strip_prefix("mate ").and_then(|n| n.parse().ok()) {
        Some(i) => MarkerState::Assigned(i),
        None if name == "done" => MarkerState::Done,
        None => MarkerState::Open,
    }
}

impl SavePart for SquadSave {
    const KEY: &'static str = "squad";

    fn save(world: &mut World) -> Option<Self> {
        // Health and shields live on the mates' bodies.
        let mut q = world.query::<(&SquadMate, Option<&Health>, Option<&Shield>)>();
        let bodies: Vec<(usize, f32, Option<f32>)> = q
            .iter(world)
            .map(|(m, h, s)| (m.index, h.map_or(0.0, |h| h.current), s.map(|s| s.current)))
            .collect();
        let squad = world.get_resource::<SquadState>()?.squad.as_ref()?;
        Some(SquadSave {
            stance: stance_name(squad.stance).into(),
            objective: squad.objective.map(arr),
            leader_down: squad.leader_down,
            player_revive: squad.player_revive,
            markers: squad.markers.iter().map(|(_, m)| marker_name(*m)).collect(),
            members: squad
                .members
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    let (state, left) = state_parts(m.state);
                    let body = bodies.iter().find(|b| b.0 == i);
                    MemberSave {
                        name: m.settings.name.clone(),
                        feet: arr(m.pawn.feet()),
                        yaw: m.yaw,
                        state: state.into(),
                        left,
                        goal: arr(m.goal),
                        hold_point: arr(m.hold_point),
                        marker: m.marker,
                        down: m.down,
                        revive: m.revive.map(patient_name),
                        health: body.map_or(0.0, |b| b.1),
                        shield: body.and_then(|b| b.2),
                    }
                })
                .collect(),
        })
    }

    fn load(world: &mut World, data: Self) -> Result<(), String> {
        stance(&data.stance)?;
        world.insert_resource(PendingSquadSave(data));
        Ok(())
    }
}

/// Put a loaded `[squad]` section back once the squad and its bodies exist.
pub(crate) fn apply_pending(world: &mut World) {
    let Some(want) = world
        .get_resource::<SquadState>()
        .and_then(|s| s.squad.as_ref())
        .map(|s| s.members.len())
    else {
        return;
    };
    let mut q = world.query::<&SquadMate>();
    if q.iter(world).count() < want {
        return;
    }
    let Some(PendingSquadSave(d)) = world.remove_resource::<PendingSquadSave>() else {
        return;
    };
    let mut problems = Vec::new();
    {
        let mut state = world.resource_mut::<SquadState>();
        let squad = state.squad.as_mut().expect("checked above");
        if let Ok(s) = stance(&d.stance) {
            squad.stance = s;
        }
        squad.objective = d.objective.map(vec);
        squad.leader_down = d.leader_down;
        squad.player_revive = d.player_revive.filter(|(i, _)| *i < squad.members.len());
        for ((_, m), name) in squad.markers.iter_mut().zip(&d.markers) {
            *m = marker(name);
        }
        if d.markers.len() != squad.markers.len() {
            problems.push(format!(
                "{} markers saved, {} on this map",
                d.markers.len(),
                squad.markers.len()
            ));
        }
        for m in squad.members.iter_mut() {
            let Some(s) = d.members.iter().find(|s| s.name == m.settings.name) else {
                problems.push(format!("{} isn't in the save", m.settings.name));
                continue;
            };
            let delta = vec(s.feet) - m.pawn.feet();
            m.pawn.location += delta;
            m.pawn.velocity = UVec3::ZERO;
            m.yaw = s.yaw;
            m.state = member_state(&s.state, s.left);
            m.goal = vec(s.goal);
            m.hold_point = vec(s.hold_point);
            m.marker = s.marker;
            m.engaging = None;
            m.down = s.down;
            m.revive = s.revive.as_deref().and_then(patient);
        }
    }
    // Their bodies: place, health, shield, down or up.
    let members: Vec<(usize, String, UVec3, f32)> = world
        .resource::<SquadState>()
        .squad
        .as_ref()
        .map(|s| {
            s.members
                .iter()
                .enumerate()
                .map(|(i, m)| (i, m.settings.name.clone(), m.pawn.feet(), m.yaw))
                .collect()
        })
        .unwrap_or_default();
    let mut q = world.query::<(Entity, &SquadMate)>();
    let bodies: Vec<(Entity, usize)> = q.iter(world).map(|(e, m)| (e, m.index)).collect();
    for (e, index) in bodies {
        let Some((_, name, feet, yaw)) = members.iter().find(|m| m.0 == index) else {
            continue;
        };
        let Some(s) = d.members.iter().find(|s| &s.name == name) else {
            continue;
        };
        let mut ent = world.entity_mut(e);
        if let Some(mut t) = ent.get_mut::<Transform>() {
            t.translation = to_bevy(*feet);
            t.rotation = yaw_rotation(*yaw);
        }
        if let Some(mut h) = ent.get_mut::<Health>() {
            h.current = s.health.min(h.max);
        }
        if let (Some(mut sh), Some(v)) = (ent.get_mut::<Shield>(), s.shield) {
            sh.current = v.min(sh.max);
        }
        if s.down {
            ent.insert(Dead);
        } else {
            ent.remove::<Dead>();
        }
    }
    let log = world.resource::<GameLog>();
    log.line(format!(
        "squad: put back: stance {}, {} mates ({} down){}",
        d.stance,
        d.members.len(),
        d.members.iter().filter(|m| m.down).count(),
        if problems.is_empty() {
            String::new()
        } else {
            format!("; {}", problems.join("; "))
        }
    ));
}
