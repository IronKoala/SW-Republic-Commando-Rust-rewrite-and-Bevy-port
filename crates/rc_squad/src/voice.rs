//! When Sev, Fixer, Scorch (and the player) speak: the squad's voice rules.
//!
//! The lines themselves are the game's. Each commando class names an
//! audio table in `System/CTAudio.u` (`Delta07Audio` for Sev, and so on):
//! one sound group in `character_voice.uax` for each event of the engine's
//! `EPawnAudioEvent` list (`AcknowledgeOrder`, `SpottedEnemy`, `AllyDown`,
//! ...). The game turns those tables into sounds; `rc_squad_plugin` reads
//! them at run time.
//!
//! *When* the original says which event lives in its compiled code, which
//! we don't read. So the rules here are **our own**: the file
//! `data/squad/voice.toml` ties things that happen to the squad
//! (`SquadEvent`s) to event names, with who says it, how likely, and how
//! often. Plain Rust, no Bevy: `Chatter::hear` takes the squad's events and
//! hands back `Say`s.
//!
//! The file uses the same small format as `squad.toml`:
//!
//! ```text
//! [voice]
//! gap = 0.5                 # seconds of quiet after any squad line
//!
//! [line.spotted]
//! on = "spotted"            # what happened (see `Trigger`)
//! event = "SpottedEnemy"    # the table's event; several = first one found
//! who = "self"              # self, other, any or leader
//! chance = 0.6
//! cooldown = 12
//! ```

use crate::fight::Rng;
use crate::settings::{one, parse_value, strip_comment, yes_no, Value};
use crate::squad::{Squad, SquadEvent};

/// What sets a line off: one kind of `SquadEvent`, sometimes narrowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trigger {
    /// The player gave an order. `None` is any order; otherwise one of
    /// `form_up`, `secure_position`, `search_and_destroy`, `hold`,
    /// `cancel`, `marker`.
    Order(Option<String>),
    /// An order the squad couldn't follow.
    OrderIgnored,
    /// A member picked an enemy after having none (first sight).
    Spotted,
    /// A member held fire because the player was in the way.
    PlayerInLineOfFire,
    /// A member held fire because another squad mate was in the way.
    MateInLineOfFire,
    /// A member went down. "about" is the downed member.
    Down,
    /// The player went down.
    LeaderDown,
    /// A member set off to revive someone. "about" is the patient.
    ReviveSetOff,
    /// A member is back up. "about" (and "self") is them.
    Revived,
    /// A member headed for cover.
    CoverTaken,
    /// A marked job: given, started, finished, cancelled. `Some(job)`
    /// narrows it to one `[job.*]` key of squad.toml.
    JobAssigned(Option<String>),
    JobStarted(Option<String>),
    JobDone(Option<String>),
    JobCancelled(Option<String>),
    /// Took up a sniper, grenade or anti-armour post.
    Posted(Option<String>),
    /// A charge was set.
    FuseLit,
}

impl Trigger {
    fn parse(t: &str) -> Result<Trigger, String> {
        let (head, tail) = match t.split_once(':') {
            Some((h, rest)) => (h, Some(rest.trim().to_string())),
            None => (t, None),
        };
        let none_after = |tr: Trigger| {
            if tail.is_some() {
                Err(format!("`{head}` takes nothing after a `:`"))
            } else {
                Ok(tr)
            }
        };
        match head.trim() {
            "order" => {
                if let Some(k) = &tail {
                    if !ORDER_KINDS.contains(&k.as_str()) {
                        return Err(format!(
                            "unknown order `{k}`; use one of {}",
                            ORDER_KINDS.join(", ")
                        ));
                    }
                }
                Ok(Trigger::Order(tail))
            }
            "order_ignored" => none_after(Trigger::OrderIgnored),
            "spotted" => none_after(Trigger::Spotted),
            "player_in_line_of_fire" => none_after(Trigger::PlayerInLineOfFire),
            "mate_in_line_of_fire" => none_after(Trigger::MateInLineOfFire),
            "down" => none_after(Trigger::Down),
            "leader_down" => none_after(Trigger::LeaderDown),
            "revive_set_off" => none_after(Trigger::ReviveSetOff),
            "revived" => none_after(Trigger::Revived),
            "cover_taken" => none_after(Trigger::CoverTaken),
            "job_assigned" => Ok(Trigger::JobAssigned(tail)),
            "job_started" => Ok(Trigger::JobStarted(tail)),
            "job_done" => Ok(Trigger::JobDone(tail)),
            "job_cancelled" => Ok(Trigger::JobCancelled(tail)),
            "posted" => Ok(Trigger::Posted(tail)),
            "fuse_lit" => none_after(Trigger::FuseLit),
            other => Err(format!("unknown `on = \"{other}\"`")),
        }
    }
}

/// The `order:` kinds, as `Squad::command` reports them.
pub const ORDER_KINDS: [&str; 6] = [
    "form_up",
    "secure_position",
    "search_and_destroy",
    "hold",
    "cancel",
    "marker",
];

/// Who says a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Who {
    /// The member the event is about (the one who spotted, took cover...).
    /// For an order: one standing member, taking turns.
    Myself,
    /// A standing member other than the one it's about, taking turns.
    Other,
    /// Any standing member, taking turns.
    Any,
    /// The player (the leader's own table, `[voice] leader_table`).
    Leader,
}

/// One rule: `[line.<key>]` in voice.toml.
#[derive(Debug, Clone, PartialEq)]
pub struct VoiceLine {
    pub key: String,
    pub on: Trigger,
    /// Event names (without the game's `PAE_`), tried in order: the first
    /// the speaker's table has a sound for is said. `{about}` is replaced
    /// by the number of the member it's about ("07"), `{me}` by the
    /// speaker's own number.
    pub events: Vec<String>,
    pub who: Who,
    /// 0 to 1.
    pub chance: f32,
    /// Seconds before this rule may speak again (anyone).
    pub cooldown: f32,
    /// Said even while someone else is talking (cuts nobody off; the two
    /// overlap). For lines the player must not miss.
    pub urgent: bool,
    /// When someone is talking, wait up to this many seconds for them to
    /// finish instead of staying quiet (0: stay quiet). An answer to the
    /// player's order waits for the order to be said.
    pub wait: f32,
}

impl VoiceLine {
    fn blank(key: &str) -> Self {
        VoiceLine {
            key: key.into(),
            on: Trigger::Spotted,
            events: Vec::new(),
            who: Who::Myself,
            chance: 1.0,
            cooldown: 0.0,
            urgent: false,
            wait: 0.0,
        }
    }
}

/// The whole voice file.
#[derive(Debug, Clone, PartialEq)]
pub struct VoiceSettings {
    /// The squad talks at all.
    pub enabled: bool,
    /// Sound package (`GameData/Sounds/<package>.uax`) the tables point
    /// into.
    pub package: String,
    /// The player's own table, for `who = "leader"` (Boss is Delta 38).
    pub leader_table: String,
    /// Seconds of quiet after any squad line before the next.
    pub gap: f32,
    /// Squad mates are heard from where they stand (and quieter further
    /// away). Off, they are heard at full volume from everywhere, like a
    /// helmet radio.
    pub positional: bool,
    /// With `positional`: full volume within this many uu.
    pub full_volume_within: f32,
    /// The rules, in file order. Every rule that fits an event is tried.
    pub lines: Vec<VoiceLine>,
}

impl Default for VoiceSettings {
    /// No rules: a missing file means a silent squad, never a crash.
    fn default() -> Self {
        VoiceSettings {
            enabled: true,
            package: "character_voice".into(),
            leader_table: "Delta38Audio".into(),
            gap: 0.5,
            positional: false,
            full_volume_within: 512.0,
            lines: Vec::new(),
        }
    }
}

impl VoiceSettings {
    /// Read `voice.toml` on top of `self`. A `[line.*]` section with the
    /// key of an existing rule changes that rule; a new key adds one.
    pub fn with_overrides(mut self, text: &str) -> Result<Self, String> {
        let mut section = String::new();
        for (i, raw) in text.lines().enumerate() {
            let n = i + 1;
            let line = strip_comment(raw).trim();
            if line.is_empty() {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = name.trim().to_string();
                if let Some(key) = section.strip_prefix("line.") {
                    if !self.lines.iter().any(|l| l.key == key) {
                        self.lines.push(VoiceLine::blank(key));
                    }
                } else if section != "voice" {
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
            let res = if let Some(lk) = section.strip_prefix("line.") {
                let l = self
                    .lines
                    .iter_mut()
                    .find(|l| l.key == lk)
                    .expect("added with its section");
                set_line(l, key, value)
            } else if section == "voice" {
                self.set(key, value)
            } else {
                Err("settings must come after a [section] line".into())
            };
            res.map_err(|e| format!("line {n}: {e}"))?;
        }
        for l in &self.lines {
            if l.events.is_empty() {
                return Err(format!("[line.{}] has no `event`", l.key));
            }
        }
        Ok(self)
    }

    fn set(&mut self, key: &str, value: Value) -> Result<(), String> {
        let text = |v: Value| match v {
            Value::Text(t) => Ok(t),
            _ => Err(format!("{key} must be \"text\"")),
        };
        match key {
            "enabled" => self.enabled = yes_no(key, value)?,
            "package" => self.package = text(value)?,
            "leader_table" => self.leader_table = text(value)?,
            "gap" => self.gap = one(value)?,
            "full_volume_within" => self.full_volume_within = one(value)?,
            "positional" => self.positional = yes_no(key, value)?,
            _ => return Err(format!("unknown setting `{key}` in [voice]")),
        }
        Ok(())
    }

    /// One line per rule, for the log.
    pub fn summary(&self) -> Vec<String> {
        let mut out = vec![format!(
            "voice: {}, {} rules, lines from {}, {} s gap, player table {}",
            if self.enabled { "on" } else { "off" },
            self.lines.len(),
            self.package,
            self.gap,
            self.leader_table
        )];
        for l in &self.lines {
            out.push(format!(
                "voice rule {}: on {:?} -> {} ({:?}, chance {}, cooldown {} s, wait {} s{})",
                l.key,
                l.on,
                l.events.join(" / "),
                l.who,
                l.chance,
                l.cooldown,
                l.wait,
                if l.urgent { ", urgent" } else { "" }
            ));
        }
        out
    }
}

fn set_line(l: &mut VoiceLine, key: &str, value: Value) -> Result<(), String> {
    let text = |v: Value| match v {
        Value::Text(t) => Ok(t),
        _ => Err(format!("{key} must be \"text\"")),
    };
    match key {
        "on" => l.on = Trigger::parse(&text(value)?)?,
        "event" => {
            l.events = text(value)?
                .split([',', ' '])
                .filter(|s| !s.is_empty())
                .map(|s| s.trim_start_matches("PAE_").to_string())
                .collect();
        }
        "who" => {
            l.who = match text(value)?.as_str() {
                "self" => Who::Myself,
                "other" => Who::Other,
                "any" => Who::Any,
                "leader" => Who::Leader,
                t => {
                    return Err(format!(
                        "who must be \"self\", \"other\", \"any\" or \"leader\", not \"{t}\""
                    ))
                }
            }
        }
        "chance" => l.chance = one(value)?.clamp(0.0, 1.0),
        "cooldown" => l.cooldown = one(value)?.max(0.0),
        "urgent" => l.urgent = yes_no(key, value)?,
        "wait" => l.wait = one(value)?.max(0.0),
        _ => return Err(format!("unknown setting `{key}` in a [line.*]")),
    }
    Ok(())
}

/// Someone should say something now.
#[derive(Debug, Clone, PartialEq)]
pub struct Say {
    /// Index in `Squad::members`, or `None` for the player.
    pub speaker: Option<usize>,
    /// Event names to try in order, `{about}`/`{me}` already filled in.
    pub events: Vec<String>,
    /// The rule (`[line.<key>]`) that chose it.
    pub rule: String,
    pub urgent: bool,
}

/// What `Chatter::hear` decided about one event, for the log.
#[derive(Debug, Clone, PartialEq)]
pub enum Heard {
    Say(Say),
    /// A rule fitted but stayed quiet, and why.
    Quiet {
        rule: String,
        why: &'static str,
    },
}

/// The squad's talking state: cooldowns, turns, who has a target.
#[derive(Debug, Clone)]
pub struct Chatter {
    pub settings: VoiceSettings,
    /// `Squad::time` each rule last spoke.
    last: Vec<Option<f32>>,
    /// No squad line before this `Squad::time` (the gap after the last).
    quiet_until: f32,
    /// Whose turn it is for `who = "any"`/`"other"`.
    turn: usize,
    /// Members that had a target last we heard.
    had_target: Vec<bool>,
    /// The job each member is on, by member index (from MarkerAssigned).
    jobs: Vec<Option<String>>,
    leader_was_down: bool,
    /// Lines waiting for a quiet moment: (rule, about, doer, give up at).
    waiting: Vec<(usize, Option<usize>, Option<usize>, f32)>,
    rng: Rng,
}

impl Chatter {
    pub fn new(settings: VoiceSettings) -> Self {
        Chatter {
            last: vec![None; settings.lines.len()],
            settings,
            quiet_until: 0.0,
            turn: 0,
            had_target: Vec::new(),
            jobs: Vec::new(),
            leader_was_down: false,
            waiting: Vec::new(),
            rng: Rng::new(38),
        }
    }

    /// A new squad (a new level or a loaded game): forget cooldowns and
    /// who had a target. The rules stay.
    pub fn reset(&mut self) {
        *self = Chatter::new(self.settings.clone());
    }

    /// A line was just said and lasts `seconds`: keep quiet until it ends
    /// plus the gap.
    pub fn speaking(&mut self, now: f32, seconds: f32) {
        self.quiet_until = self.quiet_until.max(now + seconds + self.settings.gap);
    }

    /// Look at this frame's squad events and decide who says what.
    /// `busy` is true while any squad line is still playing.
    pub fn hear(&mut self, squad: &Squad, events: &[SquadEvent], busy: bool) -> Vec<Heard> {
        let n = squad.members.len();
        self.had_target.resize(n, false);
        self.jobs.resize(n, None);
        let mut out = Vec::new();
        if !self.settings.enabled {
            return out;
        }
        let mut busy = busy || squad.time < self.quiet_until;
        let mut happened: Vec<(Trigger, Option<usize>, Option<usize>)> = Vec::new();
        if squad.leader_down && !self.leader_was_down {
            happened.push((Trigger::LeaderDown, None, None));
        }
        self.leader_was_down = squad.leader_down;
        let index = |name: &str| squad.members.iter().position(|m| m.settings.name == name);
        for e in events {
            // (what, the member it's about, the member who does it)
            let (t, about, doer) = match e {
                SquadEvent::OrderGiven { order, .. } => {
                    (Trigger::Order(Some(order_kind(order).into())), None, None)
                }
                SquadEvent::OrderIgnored { .. } => (Trigger::OrderIgnored, None, None),
                SquadEvent::TargetPicked { member, .. } => {
                    let Some(i) = index(member) else { continue };
                    let first = !self.had_target[i];
                    self.had_target[i] = true;
                    if !first {
                        continue;
                    }
                    (Trigger::Spotted, Some(i), Some(i))
                }
                SquadEvent::TargetLost { member } => {
                    if let Some(i) = index(member) {
                        self.had_target[i] = false;
                    }
                    continue;
                }
                SquadEvent::HeldFire { member, friend } => {
                    let i = index(member);
                    if index(friend).is_some() {
                        (Trigger::MateInLineOfFire, index(friend), i)
                    } else {
                        (Trigger::PlayerInLineOfFire, None, i)
                    }
                }
                SquadEvent::Down { member, .. } => {
                    let i = index(member);
                    if let Some(i) = i {
                        self.had_target[i] = false;
                    }
                    (Trigger::Down, i, None)
                }
                SquadEvent::Revived { member } => (Trigger::Revived, index(member), index(member)),
                SquadEvent::ReviveSetOff {
                    member, patient, ..
                } => (Trigger::ReviveSetOff, index(patient), index(member)),
                SquadEvent::CoverTaken { member, .. } => {
                    (Trigger::CoverTaken, index(member), index(member))
                }
                SquadEvent::MarkerAssigned { member, job, .. } => {
                    let i = index(member);
                    if let Some(i) = i {
                        self.jobs[i] = Some(job.clone());
                    }
                    (Trigger::JobAssigned(Some(job.clone())), i, i)
                }
                SquadEvent::MarkerStarted { member, .. } => {
                    let i = index(member);
                    (Trigger::JobStarted(self.job_of(i)), i, i)
                }
                SquadEvent::MarkerDone { member, .. } => {
                    let i = index(member);
                    (Trigger::JobDone(self.job_of(i)), i, i)
                }
                SquadEvent::MarkerCancelled { member, .. }
                | SquadEvent::MarkerPulled { member, .. } => {
                    let i = index(member);
                    (Trigger::JobCancelled(self.job_of(i)), i, i)
                }
                SquadEvent::Posted { member, job, .. } => {
                    let i = index(member);
                    (Trigger::Posted(Some(job.clone())), i, i)
                }
                SquadEvent::FuseLit { member, .. } => {
                    let i = index(member);
                    (Trigger::FuseLit, i, i)
                }
                _ => continue,
            };
            happened.push((t, about, doer));
        }
        // Lines that waited for quiet go first, in the order they came.
        for (r, about, doer, until) in std::mem::take(&mut self.waiting) {
            if busy {
                if squad.time < until {
                    self.waiting.push((r, about, doer, until));
                } else {
                    out.push(Heard::Quiet {
                        rule: self.settings.lines[r].key.clone(),
                        why: "waited too long",
                    });
                }
                continue;
            }
            let heard = self.decide(squad, r, about, doer, false);
            if let Heard::Say(s) = &heard {
                self.last[r] = Some(squad.time);
                busy = busy || !s.urgent;
            }
            out.push(heard);
        }
        for (t, about, doer) in happened {
            // Every rule that fits has its say (the player's order, then a
            // squad mate's answer).
            for r in 0..self.settings.lines.len() {
                if !fits(&self.settings.lines[r].on, &t) {
                    continue;
                }
                let line = &self.settings.lines[r];
                if busy && !line.urgent && line.wait > 0.0 {
                    let on_cooldown = self.last[r].is_some_and(|l| squad.time - l < line.cooldown);
                    if !on_cooldown && !self.waiting.iter().any(|w| w.0 == r) {
                        self.waiting.push((r, about, doer, squad.time + line.wait));
                        out.push(Heard::Quiet {
                            rule: line.key.clone(),
                            why: "waiting for quiet",
                        });
                        continue;
                    }
                }
                let heard = self.decide(squad, r, about, doer, busy);
                if let Heard::Say(s) = &heard {
                    self.last[r] = Some(squad.time);
                    busy = busy || !s.urgent;
                }
                out.push(heard);
            }
        }
        out
    }

    fn job_of(&self, i: Option<usize>) -> Option<String> {
        i.and_then(|i| self.jobs.get(i).cloned().flatten())
    }

    fn decide(
        &mut self,
        squad: &Squad,
        r: usize,
        about: Option<usize>,
        doer: Option<usize>,
        busy: bool,
    ) -> Heard {
        let line = self.settings.lines[r].clone();
        let quiet = |why| Heard::Quiet {
            rule: line.key.clone(),
            why,
        };
        if busy && !line.urgent {
            return quiet("someone is talking");
        }
        if let Some(t) = self.last[r] {
            if squad.time - t < line.cooldown {
                return quiet("cooldown");
            }
        }
        if line.chance < 1.0 && self.rng.next_f32() >= line.chance {
            return quiet("chance");
        }
        let standing = |i: usize| !squad.members[i].down;
        let n = squad.members.len();
        let speaker = match line.who {
            Who::Leader => None,
            Who::Myself if doer.is_some() => {
                match doer.filter(|&i| standing(i) || Some(i) == about) {
                    Some(i) => Some(i),
                    None => return quiet("speaker is down"),
                }
            }
            Who::Myself | Who::Any | Who::Other => {
                let skip = if line.who == Who::Other {
                    about.or(doer)
                } else {
                    None
                };
                let mut pick = None;
                for k in 0..n {
                    let i = (self.turn + k) % n.max(1);
                    if standing(i) && Some(i) != skip {
                        pick = Some(i);
                        break;
                    }
                }
                match pick {
                    Some(i) => {
                        self.turn = i + 1;
                        Some(i)
                    }
                    None => return quiet("nobody to say it"),
                }
            }
        };
        let number = |i: Option<usize>| match i {
            Some(i) => squad.members[i].settings.number.clone(),
            None => "38".to_string(),
        };
        let about_n = about.map(|i| squad.members[i].settings.number.clone());
        let me = number(speaker);
        let events = line
            .events
            .iter()
            .filter_map(|e| {
                if e.contains("{about}") {
                    about_n.as_ref().map(|a| e.replace("{about}", a))
                } else {
                    Some(e.clone())
                }
            })
            .map(|e| e.replace("{me}", &me))
            .collect();
        Heard::Say(Say {
            speaker,
            events,
            rule: line.key.clone(),
            urgent: line.urgent,
        })
    }
}

/// Does a rule's `on` cover what happened? A rule without a narrowing
/// (`order`, `job_done`) covers every kind.
fn fits(rule: &Trigger, t: &Trigger) -> bool {
    use Trigger::*;
    let narrow = |r: &Option<String>, h: &Option<String>| r.is_none() || r == h;
    match (rule, t) {
        (Order(r), Order(h)) => narrow(r, h),
        (JobAssigned(r), JobAssigned(h))
        | (JobStarted(r), JobStarted(h))
        | (JobDone(r), JobDone(h))
        | (JobCancelled(r), JobCancelled(h))
        | (Posted(r), Posted(h)) => narrow(r, h),
        _ => rule == t,
    }
}

/// The kind of order in an `OrderGiven` (see `Order`'s Display).
fn order_kind(order: &str) -> &'static str {
    if order.starts_with("form up") {
        "form_up"
    } else if order.starts_with("secure position") {
        "secure_position"
    } else if order.starts_with("search and destroy") {
        "search_and_destroy"
    } else if order.starts_with("hold") {
        "hold"
    } else if order.starts_with("cancel") {
        "cancel"
    } else {
        "marker"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Leader, SquadSettings};
    use rc_movement::room::test_room;
    use rc_movement::{MoveParams, Vec3};

    const SHIPPED: &str = include_str!("../../../data/squad/voice.toml");

    fn squad() -> Squad {
        let world = test_room();
        let leader = Leader {
            feet: Vec3::new(0.0, 0.0, 0.0),
            yaw: 0.0,
        };
        Squad::new(
            SquadSettings::default(),
            MoveParams::player_commando(),
            leader,
            &world,
        )
        .0
    }

    fn says(h: &[Heard]) -> Vec<(Option<usize>, String)> {
        h.iter()
            .filter_map(|h| match h {
                Heard::Say(s) => Some((s.speaker, s.events[0].clone())),
                _ => None,
            })
            .collect()
    }

    fn rules(text: &str) -> Chatter {
        Chatter::new(VoiceSettings::default().with_overrides(text).unwrap())
    }

    #[test]
    fn shipped_file_reads() {
        let s = VoiceSettings::default().with_overrides(SHIPPED).unwrap();
        assert!(s.enabled);
        assert!(s.lines.len() > 20, "{}", s.lines.len());
        assert!(s.lines.iter().any(|l| l.on == Trigger::Spotted));
        // The player's own order lines are never dropped for a squad mate
        // who is talking, and squad mates are heard like a helmet radio.
        for l in s.lines.iter().filter(|l| l.who == Who::Leader) {
            assert!(l.urgent, "{} is not urgent", l.key);
        }
        assert!(!s.positional);
    }

    #[test]
    fn a_typo_names_the_line() {
        let e = VoiceSettings::default()
            .with_overrides("[line.x]\non = \"spoted\"\nevent = \"A\"\n")
            .unwrap_err();
        assert!(e.starts_with("line 2:"), "{e}");
        let e = VoiceSettings::default()
            .with_overrides("[line.x]\non = \"spotted\"\n")
            .unwrap_err();
        assert!(e.contains("no `event`"), "{e}");
    }

    #[test]
    fn the_player_orders_then_a_mate_answers_after_the_line() {
        let mut sq = squad();
        let mut c = rules(
            "[line.boss]\non = \"order:form_up\"\nevent = \"FormUp\"\nwho = \"leader\"\n\
             [line.answer]\non = \"order\"\nevent = \"AcknowledgeOrder\"\nwho = \"any\"\nwait = 3\n",
        );
        let order = SquadEvent::OrderGiven {
            order: "form up".into(),
            stance: "Form up".into(),
        };
        let h = c.hear(&sq, &[order], false);
        assert_eq!(says(&h), vec![(None, "FormUp".to_string())]);
        // The player's line lasts a second; the answer waits for it and
        // the gap.
        c.speaking(sq.time, 1.0);
        sq.time += 1.0;
        assert!(says(&c.hear(&sq, &[], false)).is_empty());
        sq.time += 0.6;
        let h = c.hear(&sq, &[], false);
        assert_eq!(says(&h), vec![(Some(0), "AcknowledgeOrder".to_string())]);
        // Search and destroy has no player rule: only the answer, by the
        // next member.
        sq.time += 5.0;
        let order = SquadEvent::OrderGiven {
            order: "search and destroy".into(),
            stance: "Search and destroy".into(),
        };
        let h = c.hear(&sq, &[order], false);
        assert_eq!(says(&h), vec![(Some(1), "AcknowledgeOrder".to_string())]);
    }

    #[test]
    fn spotting_is_said_once_per_sight_and_respects_cooldown() {
        let mut sq = squad();
        let mut c = rules("[line.s]\non = \"spotted\"\nevent = \"SpottedEnemy\"\ncooldown = 10\n");
        let pick = |m: &str| SquadEvent::TargetPicked {
            member: m.into(),
            target: 1,
            distance: 500.0,
        };
        let h = c.hear(&sq, &[pick("Fixer")], false);
        assert_eq!(says(&h), vec![(Some(1), "SpottedEnemy".to_string())]);
        sq.time += 3.0;
        // Same member picking again is not a new sight; another member is,
        // but the rule is cooling down.
        let h = c.hear(&sq, &[pick("Fixer"), pick("Sev")], false);
        assert!(says(&h).is_empty(), "{h:?}");
        sq.time += 10.0;
        let lost = SquadEvent::TargetLost {
            member: "Fixer".into(),
        };
        let h = c.hear(&sq, &[lost, pick("Fixer")], false);
        assert_eq!(says(&h), vec![(Some(1), "SpottedEnemy".to_string())]);
    }

    #[test]
    fn a_downed_mate_is_called_by_someone_else() {
        let mut sq = squad();
        let mut c = rules(
            "[line.d]\non = \"down\"\nevent = \"Delta{about}Down AllyDown\"\nwho = \"other\"\nurgent = \"yes\"\n",
        );
        let ev = sq.set_down(0, true).unwrap();
        // Urgent: said even while someone talks.
        let h = c.hear(&sq, &[ev], true);
        let Heard::Say(s) = &h[0] else {
            panic!("{h:?}")
        };
        assert_ne!(s.speaker, Some(0));
        assert_eq!(s.events, vec!["Delta07Down", "AllyDown"]);
    }

    #[test]
    fn quiet_while_someone_talks_unless_waiting() {
        let sq = squad();
        let mut c = rules("[line.c]\non = \"cover_taken\"\nevent = \"TakingCoverHealthy\"\n");
        let cover = SquadEvent::CoverTaken {
            member: "Scorch".into(),
            at: Vec3::new(0.0, 0.0, 0.0),
            distance: 100.0,
            enemy_distance: 900.0,
            why: "test".into(),
        };
        let h = c.hear(&sq, std::slice::from_ref(&cover), true);
        assert!(matches!(
            &h[0],
            Heard::Quiet {
                why: "someone is talking",
                ..
            }
        ));
        let h = c.hear(&sq, &[cover], false);
        assert_eq!(says(&h), vec![(Some(2), "TakingCoverHealthy".to_string())]);
    }
}
