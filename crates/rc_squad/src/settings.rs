//! The squad settings file, `data/squad/squad.toml`.
//!
//! The format is a small part of TOML: `[section]` lines, then one
//! `name = value` per line, `#` starts a comment. A value is a number, a
//! list of numbers separated by commas, or text in double quotes. Every
//! value has a built-in default (the same as the shipped file), so a mod
//! only needs the lines it changes. An unknown section or name, or a bad
//! value, is an error naming the line, so a typo is reported, not ignored.

use crate::jobs::{default_jobs, JobKind, JobResult, JobSettings};
use std::collections::BTreeMap;

/// What a stance ties each member to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tether {
    /// The squad leader (the player).
    Leader,
    /// The spot the order was given for.
    Objective,
    /// Where the member stood when the order came.
    SelfPos,
}

/// How one squad order behaves. See the comments in `squad.toml`.
#[derive(Debug, Clone, PartialEq)]
pub struct StanceSettings {
    pub name: String,
    pub tether: Tether,
    pub lp_activate: f32,
    pub lp_deactivate: f32,
    pub hp_activate: f32,
    pub hp_deactivate: f32,
    pub prevent_range: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemberSettings {
    /// The key in the file, like `sev`.
    pub key: String,
    pub class: String,
    pub name: String,
    pub number: String,
    pub hud_slot: u32,
    /// Formation slot, 1 to 3.
    pub slot: usize,
    pub colour: [f32; 3],
    pub max_health: f32,
    pub max_shields: f32,
    /// Shield points per second.
    pub shield_recharge_rate: f32,
    /// Seconds after a hit before the shield recharges.
    pub shield_recharge_delay: f32,
}

/// How squad mates fight. See `[fight]` in `squad.toml`.
#[derive(Debug, Clone, PartialEq)]
pub struct FightSettings {
    /// A weapon file in `data/weapons`, without `.toml`.
    pub weapon: String,
    /// 0 (wild) to 1 (perfect).
    pub accuracy: f32,
    /// Aim error at accuracy 0, radians.
    pub max_aim_error: f32,
    /// Farthest a member sees an enemy, uu.
    pub sight_radius: f32,
    /// Seconds from first seeing an enemy to the first shot.
    pub reaction_time: f32,
    pub burst_count: u32,
    /// Seconds between shots in a burst.
    pub shot_interval: f32,
    pub burst_pause_min: f32,
    pub burst_pause_max: f32,
    /// How far off their facing a standing member can shoot, radians.
    pub max_aim_yaw: f32,
    /// Hold fire when a friend is this close to the line of fire, uu.
    pub friendly_clearance: f32,
}

impl Default for FightSettings {
    fn default() -> Self {
        FightSettings {
            weapon: "dc17m_blaster".into(),
            accuracy: 0.9,
            max_aim_error: 10f32.to_radians(),
            sight_radius: 5000.0,
            reaction_time: 0.5,
            burst_count: 5,
            shot_interval: 0.1,
            burst_pause_min: 0.6,
            burst_pause_max: 1.2,
            max_aim_yaw: 60f32.to_radians(),
            friendly_clearance: 60.0,
        }
    }
}

/// How downed commandos get back up. See `[revive]` in `squad.toml`.
#[derive(Debug, Clone, PartialEq)]
pub struct ReviveSettings {
    /// Seconds a squad mate takes to revive someone.
    pub time: f32,
    /// Seconds the player holds Use to revive a squad mate.
    pub player_time: f32,
    /// Health a revived commando gets back, as a share of their maximum.
    pub health: f32,
    /// How close to the downed body the reviver must be, uu.
    pub reach: f32,
    /// Squad mates only go to revive someone this close, uu.
    pub range: f32,
    /// Squad mates come to revive the player on their own.
    pub revive_player: bool,
}

impl Default for ReviveSettings {
    fn default() -> Self {
        ReviveSettings {
            time: 3.0,
            player_time: 3.0,
            health: 0.5,
            reach: 100.0,
            range: 4000.0,
            revive_player: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spawn {
    WithPlayer,
    None,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SquadSettings {
    pub autorun_distance: f32,
    pub think_interval: f32,
    pub arrive_radius: f32,
    pub waypoint_radius: f32,
    pub replan_distance: f32,
    /// Radians per second.
    pub turn_rate: f32,
    pub marker_work_time: f32,
    pub search_radius: f32,
    pub spawn: Spawn,
    /// Form-up slots: (forward, right) from the leader, uu.
    pub formation: [[f32; 2]; 3],
    /// Search-and-destroy slots.
    pub formation_search: [[f32; 2]; 3],
    /// Secure position: distance from the spot.
    pub secure_radius: f32,
    pub form_up: StanceSettings,
    pub secure_position: StanceSettings,
    pub search_and_destroy: StanceSettings,
    pub hold: StanceSettings,
    pub fight: FightSettings,
    pub revive: ReviveSettings,
    /// The marked jobs ("context orders"), in the order they're matched.
    pub jobs: Vec<JobSettings>,
    pub members: Vec<MemberSettings>,
}

fn stance(
    name: &str,
    tether: Tether,
    lp: (f32, f32),
    hp: (f32, f32),
    prevent: f32,
) -> StanceSettings {
    StanceSettings {
        name: name.into(),
        tether,
        lp_activate: lp.0,
        lp_deactivate: lp.1,
        hp_activate: hp.0,
        hp_deactivate: hp.1,
        prevent_range: prevent,
    }
}

fn member(
    key: &str,
    class: &str,
    name: &str,
    number: &str,
    slot: u32,
    colour: [f32; 3],
) -> MemberSettings {
    MemberSettings {
        key: key.into(),
        class: class.into(),
        name: name.into(),
        number: number.into(),
        hud_slot: slot,
        slot: slot as usize,
        colour,
        // Commando07/40/62DeltaDefaults in the Properties package.
        max_health: 400.0,
        max_shields: 50.0,
        shield_recharge_rate: 15.0,
        shield_recharge_delay: 4.0,
    }
}

impl Default for SquadSettings {
    /// The values of the shipped `data/squad/squad.toml`.
    fn default() -> Self {
        SquadSettings {
            autorun_distance: 400.0,
            think_interval: 0.25,
            arrive_radius: 40.0,
            waypoint_radius: 64.0,
            replan_distance: 96.0,
            turn_rate: 360f32.to_radians(),
            marker_work_time: 3.0,
            search_radius: 768.0,
            spawn: Spawn::WithPlayer,
            formation: [[-130.0, -110.0], [-130.0, 110.0], [-180.0, 0.0]],
            formation_search: [[200.0, -220.0], [200.0, 220.0], [320.0, 0.0]],
            secure_radius: 110.0,
            form_up: stance(
                "Form up",
                Tether::Leader,
                (768.0, 192.0),
                (1280.0, 768.0),
                1024.0,
            ),
            secure_position: stance(
                "Secure area",
                Tether::Objective,
                (768.0, 128.0),
                (1024.0, 512.0),
                640.0,
            ),
            search_and_destroy: stance(
                "Search and destroy",
                Tether::Leader,
                (512.0, 256.0),
                (2048.0, 1280.0),
                1024.0,
            ),
            hold: stance(
                "Hold",
                Tether::SelfPos,
                (512.0, 256.0),
                (768.0, 384.0),
                256.0,
            ),
            fight: FightSettings::default(),
            revive: ReviveSettings::default(),
            jobs: default_jobs(),
            members: vec![
                member("sev", "Commando07Delta", "Sev", "07", 1, [0.71, 0.18, 0.17]),
                member(
                    "fixer",
                    "Commando40Delta",
                    "Fixer",
                    "40",
                    2,
                    [0.21, 0.49, 0.17],
                ),
                member(
                    "scorch",
                    "Commando62Delta",
                    "Scorch",
                    "62",
                    3,
                    [0.85, 0.80, 0.13],
                ),
            ],
        }
    }
}

/// One value from the file.
enum Value {
    Numbers(Vec<f32>),
    Text(String),
}

fn parse_value(s: &str) -> Option<Value> {
    let s = s.trim();
    if let Some(inner) = s.strip_prefix('"') {
        return inner.strip_suffix('"').map(|t| Value::Text(t.to_string()));
    }
    s.split(',')
        .map(|p| p.trim().parse::<f32>().ok())
        .collect::<Option<Vec<_>>>()
        .map(Value::Numbers)
}

impl SquadSettings {
    /// Read a settings file, starting from `self` and changing only what
    /// the file names.
    pub fn with_overrides(mut self, text: &str) -> Result<Self, String> {
        let mut section = String::new();
        // Members named in the file, in file order, starting from the
        // built-in member of the same key if there is one.
        let mut members: BTreeMap<String, MemberSettings> = BTreeMap::new();
        let mut member_order: Vec<String> = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let n = i + 1;
            let line = strip_comment(raw).trim();
            if line.is_empty() {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = name.trim().to_string();
                if let Some(key) = section.strip_prefix("member.") {
                    if !members.contains_key(key) {
                        let base = self
                            .members
                            .iter()
                            .find(|m| m.key == key)
                            .cloned()
                            .unwrap_or_else(|| member(key, "", key, "", 0, [0.5, 0.5, 0.5]));
                        members.insert(key.to_string(), base);
                        member_order.push(key.to_string());
                    }
                } else if let Some(key) = section.strip_prefix("job.") {
                    if !self.jobs.iter().any(|j| j.key == key) {
                        self.jobs.push(JobSettings::blank(key));
                    }
                } else if !matches!(
                    section.as_str(),
                    "squad"
                        | "formation"
                        | "formation_search"
                        | "formation_secure"
                        | "stance.form_up"
                        | "stance.secure_position"
                        | "stance.search_and_destroy"
                        | "stance.hold"
                        | "fight"
                        | "revive"
                ) {
                    return Err(format!("line {n}: unknown section [{section}]"));
                }
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| format!("line {n}: expected `name = value`, got `{line}`"))?;
            let key = key.trim();
            let value = parse_value(value).ok_or_else(|| {
                format!(
                    "line {n}: `{}` is not a number, numbers or \"text\"",
                    value.trim()
                )
            })?;
            let res = if let Some(jk) = section.strip_prefix("job.") {
                let j = self
                    .jobs
                    .iter_mut()
                    .find(|j| j.key == jk)
                    .expect("added with its section");
                set_job(j, key, value)
            } else if let Some(mk) = section.strip_prefix("member.") {
                let m = members.get_mut(mk).expect("added with its section");
                set_member(m, key, value)
            } else {
                self.set(&section, key, value)
            };
            res.map_err(|e| format!("line {n}: {e}"))?;
        }
        if !member_order.is_empty() {
            // Members named in the file replace the built-in list, so a mod
            // can add or remove squad mates.
            self.members = member_order.iter().map(|k| members[k].clone()).collect();
        }
        for m in &self.members {
            if !(1..=3).contains(&m.slot) {
                return Err(format!("member {}: slot must be 1, 2 or 3", m.key));
            }
        }
        Ok(self)
    }

    fn set(&mut self, section: &str, key: &str, value: Value) -> Result<(), String> {
        let unknown = || format!("unknown setting `{key}` in [{section}]");
        match section {
            "squad" => {
                if key == "spawn" {
                    let Value::Text(t) = value else {
                        return Err("spawn must be \"with_player\" or \"none\"".into());
                    };
                    self.spawn = match t.as_str() {
                        "with_player" => Spawn::WithPlayer,
                        "none" => Spawn::None,
                        _ => {
                            return Err(format!(
                                "spawn must be \"with_player\" or \"none\", not \"{t}\""
                            ))
                        }
                    };
                    return Ok(());
                }
                let v = one(value)?;
                let slot = match key {
                    "autorun_distance" => &mut self.autorun_distance,
                    "think_interval" => &mut self.think_interval,
                    "arrive_radius" => &mut self.arrive_radius,
                    "waypoint_radius" => &mut self.waypoint_radius,
                    "replan_distance" => &mut self.replan_distance,
                    "turn_rate" => {
                        self.turn_rate = v.to_radians();
                        return Ok(());
                    }
                    "marker_work_time" => &mut self.marker_work_time,
                    "search_radius" => &mut self.search_radius,
                    _ => return Err(unknown()),
                };
                *slot = v;
            }
            "formation" | "formation_search" => {
                let i = match key {
                    "slot1" => 0,
                    "slot2" => 1,
                    "slot3" => 2,
                    _ => return Err(unknown()),
                };
                let Value::Numbers(v) = value else {
                    return Err(format!("{key} must be two numbers: forward, right"));
                };
                let [f, r] = v[..] else {
                    return Err(format!("{key} must be two numbers: forward, right"));
                };
                let table = if section == "formation" {
                    &mut self.formation
                } else {
                    &mut self.formation_search
                };
                table[i] = [f, r];
            }
            "fight" => set_fight(&mut self.fight, key, value)?,
            "revive" => set_revive(&mut self.revive, key, value)?,
            "formation_secure" => match key {
                "radius" => self.secure_radius = one(value)?,
                _ => return Err(unknown()),
            },
            _ => {
                let st = match section {
                    "stance.form_up" => &mut self.form_up,
                    "stance.secure_position" => &mut self.secure_position,
                    "stance.search_and_destroy" => &mut self.search_and_destroy,
                    _ => &mut self.hold,
                };
                set_stance(st, key, value).map_err(|e| if e.is_empty() { unknown() } else { e })?;
            }
        }
        Ok(())
    }

    /// Every number setting, for the log.
    pub fn summary(&self) -> Vec<String> {
        let mut out = vec![
            format!(
                "squad: autorun {} uu, think every {} s, arrive within {} uu, marker work {} s, search radius {} uu, spawn {:?}",
                self.autorun_distance,
                self.think_interval,
                self.arrive_radius,
                self.marker_work_time,
                self.search_radius,
                self.spawn
            ),
            format!(
                "formation slots (forward, right): {:?}; search {:?}; secure radius {}",
                self.formation, self.formation_search, self.secure_radius
            ),
        ];
        for s in [
            &self.form_up,
            &self.secure_position,
            &self.search_and_destroy,
            &self.hold,
        ] {
            out.push(format!(
                "stance {}: tether {:?}, low-priority {} / {}, high-priority {} / {}, prevent {}",
                s.name,
                s.tether,
                s.lp_activate,
                s.lp_deactivate,
                s.hp_activate,
                s.hp_deactivate,
                s.prevent_range
            ));
        }
        let f = &self.fight;
        out.push(format!(
            "fight: {}, accuracy {}, aim error up to {:.1} deg, sees {} uu, reacts in {} s, bursts of {} every {} s, pause {}-{} s, aims {:.0} deg off facing, friendly clearance {} uu",
            f.weapon,
            f.accuracy,
            f.max_aim_error.to_degrees(),
            f.sight_radius,
            f.reaction_time,
            f.burst_count,
            f.shot_interval,
            f.burst_pause_min,
            f.burst_pause_max,
            f.max_aim_yaw.to_degrees(),
            f.friendly_clearance
        ));
        let r = &self.revive;
        out.push(format!(
            "revive: squad mate {} s, player {} s, back with {:.0}% health, reach {} uu, range {} uu, revive the player {}",
            r.time,
            r.player_time,
            r.health * 100.0,
            r.reach,
            r.range,
            if r.revive_player { "yes" } else { "no" }
        ));
        out.extend(self.jobs.iter().map(JobSettings::summary));
        for m in &self.members {
            out.push(format!(
                "member {} ({}, {}): HUD slot {}, formation slot {}, health {}, shields {} (+{}/s after {} s)",
                m.name,
                m.number,
                m.class,
                m.hud_slot,
                m.slot,
                m.max_health,
                m.max_shields,
                m.shield_recharge_rate,
                m.shield_recharge_delay
            ));
        }
        out
    }
}

fn strip_comment(line: &str) -> &str {
    // A `#` inside quotes is text, not a comment.
    let mut quoted = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '#' if !quoted => return &line[..i],
            _ => {}
        }
    }
    line
}

fn one(value: Value) -> Result<f32, String> {
    match value {
        Value::Numbers(v) if v.len() == 1 => Ok(v[0]),
        _ => Err("expected one number".into()),
    }
}

fn set_stance(st: &mut StanceSettings, key: &str, value: Value) -> Result<(), String> {
    match key {
        "name" => match value {
            Value::Text(t) => st.name = t,
            _ => return Err("name must be \"text\"".into()),
        },
        "tether" => {
            let Value::Text(t) = value else {
                return Err("tether must be \"leader\", \"objective\" or \"self\"".into());
            };
            st.tether = match t.as_str() {
                "leader" => Tether::Leader,
                "objective" => Tether::Objective,
                "self" => Tether::SelfPos,
                _ => {
                    return Err(format!(
                        "tether must be \"leader\", \"objective\" or \"self\", not \"{t}\""
                    ))
                }
            };
        }
        _ => {
            let v = one(value)?;
            *match key {
                "lp_activate" => &mut st.lp_activate,
                "lp_deactivate" => &mut st.lp_deactivate,
                "hp_activate" => &mut st.hp_activate,
                "hp_deactivate" => &mut st.hp_deactivate,
                "prevent_range" => &mut st.prevent_range,
                _ => return Err(String::new()),
            } = v;
        }
    }
    Ok(())
}

fn set_member(m: &mut MemberSettings, key: &str, value: Value) -> Result<(), String> {
    let text = |v: Value| match v {
        Value::Text(t) => Ok(t),
        _ => Err(format!("{key} must be \"text\"")),
    };
    match key {
        "class" => m.class = text(value)?,
        "name" => m.name = text(value)?,
        "number" => m.number = text(value)?,
        "hud_slot" => m.hud_slot = one(value)? as u32,
        "slot" => m.slot = one(value)? as usize,
        "colour" => match value {
            Value::Numbers(v) if v.len() == 3 => m.colour = [v[0], v[1], v[2]],
            _ => return Err("colour must be three numbers: red, green, blue (0 to 1)".into()),
        },
        "max_health" => m.max_health = one(value)?,
        "max_shields" => m.max_shields = one(value)?,
        "shield_recharge_rate" => m.shield_recharge_rate = one(value)?,
        "shield_recharge_delay" => m.shield_recharge_delay = one(value)?,
        _ => return Err(format!("unknown setting `{key}` for a squad member")),
    }
    Ok(())
}

fn set_fight(f: &mut FightSettings, key: &str, value: Value) -> Result<(), String> {
    if key == "weapon" {
        match value {
            Value::Text(t) => f.weapon = t,
            _ => return Err("weapon must be \"text\" (a file name in data/weapons)".into()),
        }
        return Ok(());
    }
    let v = one(value)?;
    match key {
        "accuracy" => f.accuracy = v,
        "max_aim_error" => f.max_aim_error = v.to_radians(),
        "sight_radius" => f.sight_radius = v,
        "reaction_time" => f.reaction_time = v,
        "burst_count" => f.burst_count = (v.max(1.0)) as u32,
        "shot_interval" => f.shot_interval = v,
        "burst_pause_min" => f.burst_pause_min = v,
        "burst_pause_max" => f.burst_pause_max = v,
        "max_aim_yaw" => f.max_aim_yaw = v.to_radians(),
        "friendly_clearance" => f.friendly_clearance = v,
        _ => return Err(format!("unknown setting `{key}` in [fight]")),
    }
    Ok(())
}

fn set_revive(r: &mut ReviveSettings, key: &str, value: Value) -> Result<(), String> {
    if key == "revive_player" {
        r.revive_player = match value {
            Value::Text(t) if t == "yes" => true,
            Value::Text(t) if t == "no" => false,
            _ => return Err("revive_player must be \"yes\" or \"no\"".into()),
        };
        return Ok(());
    }
    let v = one(value)?;
    match key {
        "time" => r.time = v,
        "player_time" => r.player_time = v,
        "health" => r.health = v,
        "reach" => r.reach = v,
        "range" => r.range = v,
        _ => return Err(format!("unknown setting `{key}` in [revive]")),
    }
    Ok(())
}

fn set_job(j: &mut JobSettings, key: &str, value: Value) -> Result<(), String> {
    let text = |v: Value| match v {
        Value::Text(t) => Ok(t),
        _ => Err(format!("{key} must be \"text\"")),
    };
    match key {
        "name" => j.name = text(value)?,
        "match" => {
            j.matches = text(value)?
                .split(',')
                .map(|w| w.trim().to_ascii_lowercase())
                .filter(|w| !w.is_empty())
                .collect()
        }
        "weapon" => j.weapon = text(value)?,
        "preferred" => j.preferred = text(value)?,
        "kind" => {
            j.kind = match text(value)?.as_str() {
                "once" => JobKind::Once,
                "post" => JobKind::Post,
                t => return Err(format!("kind must be \"once\" or \"post\", not \"{t}\"")),
            }
        }
        "result" => {
            j.result = match text(value)?.as_str() {
                "none" => JobResult::Nothing,
                "heal" => JobResult::Heal,
                "trigger" => JobResult::Trigger,
                t => {
                    return Err(format!(
                        "result must be \"none\", \"heal\" or \"trigger\", not \"{t}\""
                    ))
                }
            }
        }
        "group" => {
            j.group = match text(value)?.as_str() {
                "yes" => true,
                "no" => false,
                _ => return Err("group must be \"yes\" or \"no\"".into()),
            }
        }
        _ => {
            let v = one(value)?;
            match key {
                "work_time" => j.work_time = v,
                "fuse" => j.fuse = v,
                "blast_damage" => j.blast_damage = v,
                "blast_radius" => j.blast_radius = v,
                "sight_radius" => j.sight_radius = v,
                "accuracy" => j.accuracy = v,
                "burst_count" => j.burst_count = v.max(0.0) as u32,
                "shot_interval" => j.shot_interval = v,
                "burst_pause_min" => j.burst_pause_min = v,
                "burst_pause_max" => j.burst_pause_max = v,
                "throw_interval" => j.throw_interval = v,
                "throw_range" => j.throw_range = v,
                "throw_fuse" => j.throw_fuse = v,
                "throw_damage" => j.throw_damage = v,
                "throw_radius" => j.throw_radius = v,
                "target_radius" => j.target_radius = v,
                _ => return Err(format!("unknown setting `{key}` for a job")),
            }
        }
    }
    Ok(())
}
