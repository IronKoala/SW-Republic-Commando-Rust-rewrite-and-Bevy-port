//! Runs a map's level scripting: the "director" of a mission.
//!
//! Republic Commando, like other Unreal Engine 2 games, ties a level
//! together with **events** and **tags**. Every actor has a `Tag` (a name)
//! and many have an `Event` (another name). When something "triggers an
//! event", every actor whose `Tag` is that name hears it: a door opens, a
//! factory spawns droids, a counter counts, a script waiting for that name
//! carries on. Scripts (`ScriptedTrigger`, `ScriptedSequence`) are lists of
//! small actions read from the map (`rc_level::scripts`): wait for an
//! event, wait some seconds, send an event, play a line, hurt something,
//! update an objective, and so on.
//!
//! [`Director`] runs those lists and the simple actors around them
//! (touch triggers, counters, true/false conditions, checkpoints,
//! destructible props). Everything that needs the rest of the game (doors,
//! sounds, the player's health, droids) comes out of [`Director::step`] as
//! an [`Effect`] for the game to carry out, and the game reports back with
//! [`Director::fire`] (something happened) and the player's position.
//!
//! Plain Rust with no Bevy, so the rules can be tested on their own and
//! used by any tool. How each action behaves is our reading of Unreal
//! Engine 2's public descriptions of these classes and of what the maps
//! store; no game code was copied. Our own choices (how long an unknown
//! sound lasts, the random seed) are in `data/scripts/scripts.toml`
//! ([`Settings`]).

pub mod save;
pub mod settings;

use rc_level::movers::TriggerInfo;
use rc_level::scripts::{MapScripts, ScriptObject, Val};
pub use settings::Settings;
use std::collections::{HashMap, VecDeque};

/// What the director asks the game to do, or tells it about.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// An event was sent (by a script, a trigger, a counter, ...). The
    /// game passes it on to what the director doesn't run itself: doors
    /// and lifts with that `Tag`, factories that spawn on it, ...
    Event { name: String, by: String },
    /// The event is over (the player left the trigger that sent it).
    Untrigger { name: String, by: String },
    /// A script action for the game to carry out (play a sound, hurt the
    /// player, switch collision off, show a prompt, update an objective).
    Action {
        script: String,
        index: usize,
        action: ScriptObject,
    },
    /// A prop was destroyed (blown up by a script or hurt past its
    /// health): its collision and drawing go away, or `mesh` takes its
    /// place.
    PropDestroyed {
        export: usize,
        name: String,
        tag: String,
        mesh: Option<String>,
        effect: Option<String>,
    },
    /// A checkpoint was reached: the game saves.
    Checkpoint { name: String },
    /// Something the log should show (a script waiting, a counter, ...).
    Note(String),
}

/// What a script is waiting for.
#[derive(Debug, Clone, PartialEq)]
enum Wait {
    Nothing,
    /// An event, by lower-case name.
    Event(String),
    /// A time, in the director's seconds.
    Until(f32),
}

struct Script {
    name: String,
    class: String,
    tag: String,
    actions: Vec<ScriptObject>,
    /// The next action to run.
    pc: usize,
    wait: Wait,
    running: bool,
    finished: bool,
}

struct Trigger {
    info: TriggerInfo,
    /// Lower-case tag.
    tag: String,
    active: bool,
    touching: bool,
    used: bool,
    cooldown: f32,
}

struct Counter {
    name: String,
    left: i32,
    event: Option<String>,
}

struct Condition {
    name: String,
    enabled: bool,
    initial: bool,
    toggled: bool,
    trigger_controlled: bool,
}

struct Prop {
    export: usize,
    name: String,
    tag: String,
    health: f32,
    destroyed: bool,
    event: Option<String>,
    mesh: Option<String>,
    effect: Option<String>,
}

/// Counts for the log and the self-tests.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stats {
    pub scripts: usize,
    pub sequences: usize,
    pub actions: usize,
    pub triggers: usize,
    pub counters: usize,
    pub conditions: usize,
    pub checkpoints: usize,
    pub props: usize,
    /// Events sent so far, and actions run so far.
    pub events_sent: usize,
    pub actions_run: usize,
    pub scripts_finished: usize,
}

/// The player, as the director needs them: where they are (Unreal units,
/// the middle of their body) and their size.
#[derive(Debug, Clone, Copy)]
pub struct PlayerBody {
    pub location: [f32; 3],
    pub radius: f32,
    pub half_height: f32,
}

/// Runs one map's scripting. See the crate description.
pub struct Director {
    pub settings: Settings,
    scripts: Vec<Script>,
    triggers: Vec<Trigger>,
    counters: HashMap<String, Vec<Counter>>,
    conditions: HashMap<String, Vec<Condition>>,
    /// Checkpoint actor names by lower-case tag.
    checkpoints: HashMap<String, Vec<String>>,
    props: Vec<Prop>,
    /// Sequences to start when an event (a factory's tag) is sent, by
    /// lower-case event name.
    start_on: HashMap<String, Vec<usize>>,
    /// Events sent at level start (the `PlayerStart`'s `Event`).
    start_events: Vec<String>,
    queue: VecDeque<(String, String)>,
    out: Vec<Effect>,
    /// Seconds since the level started.
    pub time: f32,
    rng: u64,
    pub stats: Stats,
    /// Every event sent so far, in order, with the time. For the log and
    /// the self-tests.
    pub history: Vec<(f32, String)>,
}

fn low(s: &str) -> String {
    s.to_ascii_lowercase()
}

/// The part of a class name after `ACTION_`.
fn action_kind(a: &ScriptObject) -> &str {
    a.class.strip_prefix("ACTION_").unwrap_or(&a.class)
}

impl Director {
    /// Build the director for a map: its scripts and actors
    /// (`rc_level::scripts::read`) and its touch triggers
    /// (`rc_level::movers::read_movers`, which fills in class defaults).
    pub fn new(map: &MapScripts, triggers: &[TriggerInfo], settings: Settings) -> Self {
        let mut d = Director {
            rng: settings.random_seed.max(1),
            settings,
            scripts: Vec::new(),
            triggers: Vec::new(),
            counters: HashMap::new(),
            conditions: HashMap::new(),
            checkpoints: HashMap::new(),
            props: Vec::new(),
            start_on: HashMap::new(),
            start_events: Vec::new(),
            queue: VecDeque::new(),
            out: Vec::new(),
            time: 0.0,
            stats: Stats::default(),
            history: Vec::new(),
        };
        // Placed characters run the sequence named by their AIScriptTag.
        let placed_scripts: Vec<String> = map
            .actors
            .iter()
            .filter_map(|a| a.obj.text("AIScriptTag").map(low))
            .collect();
        let mut seen_start = false;
        for a in &map.actors {
            let o = &a.obj;
            let class = o.class.as_str();
            if !a.actions.is_empty() {
                let sequence = class.eq_ignore_ascii_case("ScriptedSequence");
                let tag = o.tag().to_string();
                // Scripted triggers run from the start. A sequence belongs
                // to a character: it runs if one placed in the map uses it,
                // or once a factory that gives it to what it spawns goes off.
                let running = !sequence || placed_scripts.contains(&low(&tag));
                d.stats.actions += a.actions.len();
                if sequence {
                    d.stats.sequences += 1;
                } else {
                    d.stats.scripts += 1;
                }
                d.scripts.push(Script {
                    name: o.name.clone(),
                    class: o.class.clone(),
                    tag,
                    actions: a.actions.clone(),
                    pc: 0,
                    wait: Wait::Nothing,
                    running,
                    finished: false,
                });
                continue;
            }
            if class.eq_ignore_ascii_case("Counter") {
                d.stats.counters += 1;
                d.counters.entry(low(o.tag())).or_default().push(Counter {
                    name: o.name.clone(),
                    // UE2's Counter counts to 2 unless told otherwise.
                    left: o.int("NumToCount").unwrap_or(2),
                    event: o.event().map(str::to_string),
                });
            } else if class.eq_ignore_ascii_case("TriggeredCondition") {
                d.stats.conditions += 1;
                let enabled = o.flag("bEnabled", false);
                d.conditions
                    .entry(low(o.tag()))
                    .or_default()
                    .push(Condition {
                        name: o.name.clone(),
                        enabled,
                        initial: enabled,
                        toggled: o.flag("bToggled", false),
                        trigger_controlled: o.flag("bTriggerControlled", false),
                    });
            } else if class.eq_ignore_ascii_case("Checkpoint") {
                d.stats.checkpoints += 1;
                d.checkpoints
                    .entry(low(o.tag()))
                    .or_default()
                    .push(o.name.clone());
            } else if class.eq_ignore_ascii_case("PlayerStart") {
                // The player starts at the first one (as the level part
                // does), and only that one's event is sent.
                if !seen_start {
                    seen_start = true;
                    if let Some(e) = o.event() {
                        d.start_events.push(e.to_string());
                    }
                }
            }
            if class.eq_ignore_ascii_case("Prop") || (o.has_own_tag() && o.get("Health").is_some())
            {
                let destroyed = o.structure("DestroyedState");
                let text = |k: &str| {
                    destroyed.and_then(|s| match s.get(k) {
                        Some(Val::Text(t)) if !t.eq_ignore_ascii_case("None") => Some(t.clone()),
                        _ => None,
                    })
                };
                d.stats.props += 1;
                d.props.push(Prop {
                    export: o.export,
                    name: o.name.clone(),
                    tag: o.tag().to_string(),
                    health: o.float("Health").unwrap_or(d.settings.prop_health),
                    destroyed: false,
                    event: text("Event"),
                    mesh: text("Mesh"),
                    effect: text("TransitionEffect"),
                });
            }
        }
        // Factories give their spawn a sequence: it starts when they go off.
        for a in map.of_class("PawnFactory") {
            if let Some(s) = a.obj.text("AIScriptFromFactory") {
                let s = low(s);
                for (i, sc) in d.scripts.iter().enumerate() {
                    let list = d.start_on.entry(low(a.obj.tag())).or_default();
                    if low(&sc.tag) == s && !list.contains(&i) {
                        list.push(i);
                    }
                }
            }
        }
        // Touch triggers, with the map's own on/off switch.
        for t in triggers {
            let obj = map.actors.iter().find(|a| a.obj.export == t.export);
            let active = obj.is_none_or(|a| a.obj.flag("bInitiallyActive", true));
            d.triggers.push(Trigger {
                tag: low(&t.tag),
                info: t.clone(),
                active,
                touching: false,
                used: false,
                cooldown: 0.0,
            });
        }
        d.stats.triggers = d.triggers.len();
        d
    }

    /// The events the map sends when the level starts (the player start's
    /// `Event`, `levelstart` in the campaign maps). Send them with
    /// [`Self::start`] unless a saved game is being restored.
    pub fn start_events(&self) -> &[String] {
        &self.start_events
    }

    /// Send the level-start events.
    pub fn start(&mut self) {
        for e in self.start_events.clone() {
            self.fire(&e, "level start");
        }
    }

    /// Something happened that sends `event` (a door opened, a droid died,
    /// the player used a bacta dispenser). Handled on the next step.
    pub fn fire(&mut self, event: &str, by: &str) {
        if !event.is_empty() && !event.eq_ignore_ascii_case("None") {
            self.queue.push_back((event.to_string(), by.to_string()));
        }
    }

    /// Whether a `TriggeredCondition` with this tag is on.
    pub fn condition(&self, tag: &str) -> bool {
        self.conditions
            .get(&low(tag))
            .is_some_and(|cs| cs.iter().any(|c| c.enabled))
    }

    /// What each script is doing, for the log: (name, next action, waits).
    pub fn script_states(&self) -> Vec<(String, String, String)> {
        self.scripts
            .iter()
            .map(|s| {
                let what = if s.finished {
                    "finished".to_string()
                } else if !s.running {
                    "not started".to_string()
                } else {
                    match &s.wait {
                        Wait::Nothing => "running".to_string(),
                        Wait::Event(e) => format!("waits for {e}"),
                        Wait::Until(t) => format!("waits until {t:.1} s"),
                    }
                };
                let next = s.actions.get(s.pc).map_or("end".to_string(), |a| {
                    format!("{} {}", s.pc, action_kind(a))
                });
                (s.name.clone(), next, what)
            })
            .collect()
    }

    /// Props not yet destroyed, by tag (lower case): how many.
    pub fn props_standing(&self, tag: &str) -> usize {
        self.props
            .iter()
            .filter(|p| !p.destroyed && p.tag.eq_ignore_ascii_case(tag))
            .count()
    }

    /// Hurt every prop with this tag. Destroyed props come out as
    /// [`Effect::PropDestroyed`] on the next step. Returns how many props
    /// have the tag.
    pub fn hurt_props(&mut self, tag: &str, amount: f32) -> usize {
        let mut n = 0;
        let mut events = Vec::new();
        for p in self
            .props
            .iter_mut()
            .filter(|p| p.tag.eq_ignore_ascii_case(tag))
        {
            n += 1;
            if p.destroyed {
                continue;
            }
            p.health -= amount;
            if p.health <= 0.0 {
                p.destroyed = true;
                self.out.push(Effect::PropDestroyed {
                    export: p.export,
                    name: p.name.clone(),
                    tag: p.tag.clone(),
                    mesh: p.mesh.clone(),
                    effect: p.effect.clone(),
                });
                if let Some(e) = &p.event {
                    events.push((e.clone(), p.name.clone()));
                }
            }
        }
        for (e, by) in events {
            self.fire(&e, &by);
        }
        n
    }

    /// Remove every prop with this tag at once (`ACTION_DestroyActor`).
    pub fn destroy_props(&mut self, tag: &str) -> usize {
        self.hurt_props(tag, f32::INFINITY)
    }

    fn random(&mut self) -> f32 {
        // xorshift64: small, and the same on every machine.
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        (x >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Advance by `dt` seconds. `player` is where the player is, if there is
    /// one (touch triggers need it). Returns what the game should do.
    pub fn step(&mut self, dt: f32, player: Option<PlayerBody>) -> Vec<Effect> {
        self.time += dt;
        if let Some(p) = player {
            self.touch(dt, p);
        }
        // Events and scripts can set each other off; settle within the
        // frame, a few rounds at most.
        // Scripts run first, so ones that are about to wait for an event
        // are waiting when it is handed out.
        for _ in 0..8 {
            let ran = self.run_scripts();
            let had_events = !self.queue.is_empty();
            self.dispatch();
            if !ran && !had_events {
                break;
            }
        }
        std::mem::take(&mut self.out)
    }

    fn touch(&mut self, dt: f32, p: PlayerBody) {
        let mut fire = Vec::new();
        let mut release = Vec::new();
        for t in &mut self.triggers {
            t.cooldown = (t.cooldown - dt).max(0.0);
            let i = &t.info;
            let d = [
                p.location[0] - i.location[0],
                p.location[1] - i.location[1],
                p.location[2] - i.location[2],
            ];
            let inside = (d[0] * d[0] + d[1] * d[1]).sqrt() < i.radius + p.radius
                && d[2].abs() < i.height + p.half_height;
            // Trigger types the player sets off (UE2 numbering).
            let player_ok = matches!(i.trigger_type, 0 | 1 | 3 | 5);
            if inside
                && !t.touching
                && t.active
                && player_ok
                && !(i.once_only && t.used)
                && t.cooldown <= 0.0
            {
                t.used = true;
                t.cooldown = i.retrigger_delay;
                fire.push((i.event.clone(), i.name.clone()));
            }
            if !inside && t.touching {
                release.push((i.event.clone(), i.name.clone()));
            }
            t.touching = inside;
        }
        for (e, by) in fire {
            self.out
                .push(Effect::Note(format!("{by}: the player walked in")));
            self.fire(&e, &by);
        }
        for (e, by) in release {
            if !e.is_empty() && !e.eq_ignore_ascii_case("None") {
                self.out.push(Effect::Untrigger { name: e, by });
            }
        }
    }

    /// Hand every queued event to whatever has its name as a tag.
    fn dispatch(&mut self) {
        let mut guard = 0;
        while let Some((name, by)) = self.queue.pop_front() {
            guard += 1;
            if guard > 1000 {
                self.out.push(Effect::Note(format!(
                    "more than 1000 events in one frame; stopped at {name}"
                )));
                self.queue.clear();
                return;
            }
            let key = low(&name);
            self.stats.events_sent += 1;
            self.history.push((self.time, name.clone()));
            let mut heard = Vec::new();
            // Scripts waiting for it carry on.
            for s in &mut self.scripts {
                if s.running && !s.finished && s.wait == Wait::Event(key.clone()) {
                    s.wait = Wait::Nothing;
                    s.pc += 1;
                    heard.push(s.name.clone());
                }
            }
            // Sequences handed to what a factory spawns start.
            if let Some(list) = self.start_on.get(&key) {
                for &i in list {
                    let s = &mut self.scripts[i];
                    if !s.running && !s.finished {
                        s.running = true;
                        heard.push(format!("{} starts", s.name));
                    }
                }
            }
            // Triggers switched on or off by other triggers.
            for t in &mut self.triggers {
                if t.tag != key {
                    continue;
                }
                let state = low(&t.info.state);
                let before = t.active;
                match state.as_str() {
                    "othertriggerturnson" => t.active = true,
                    "othertriggerturnsoff" => t.active = false,
                    "othertriggertoggles" => t.active = !t.active,
                    _ => continue,
                }
                if before != t.active {
                    heard.push(format!(
                        "{} {}",
                        t.info.name,
                        if t.active {
                            "switched on"
                        } else {
                            "switched off"
                        }
                    ));
                }
            }
            // Counters count down, and send their event at zero.
            let mut more = Vec::new();
            if let Some(cs) = self.counters.get_mut(&key) {
                for c in cs {
                    if c.left > 0 {
                        c.left -= 1;
                        heard.push(format!("{} has {} to go", c.name, c.left));
                        if c.left == 0 {
                            if let Some(e) = &c.event {
                                more.push((e.clone(), c.name.clone()));
                            }
                        }
                    }
                }
            }
            // True/false conditions (UE2 TriggeredCondition): a toggled one
            // flips each time; otherwise it takes the opposite of its start.
            if let Some(cs) = self.conditions.get_mut(&key) {
                for c in cs {
                    c.enabled = if c.toggled { !c.enabled } else { !c.initial };
                    heard.push(format!("condition {} is now {}", c.name, c.enabled));
                }
            }
            // Checkpoints: the game saves.
            if let Some(cps) = self.checkpoints.get(&key) {
                for cp in cps {
                    heard.push(format!("checkpoint {cp}"));
                    self.out.push(Effect::Checkpoint { name: cp.clone() });
                }
            }
            let note = if heard.is_empty() {
                format!("event {name} (by {by})")
            } else {
                format!("event {name} (by {by}) -> {}", heard.join(", "))
            };
            self.out.push(Effect::Note(note));
            self.out.push(Effect::Event { name, by });
            for (e, b) in more {
                self.fire(&e, &b);
            }
        }
    }

    /// An untrigger (a condition controlled by a trigger goes back).
    pub fn untrigger(&mut self, event: &str) {
        if let Some(cs) = self.conditions.get_mut(&low(event)) {
            for c in cs.iter_mut().filter(|c| c.trigger_controlled) {
                c.enabled = c.initial;
            }
        }
    }

    /// Run each script until it waits. Returns whether any action ran.
    fn run_scripts(&mut self) -> bool {
        let mut any = false;
        for i in 0..self.scripts.len() {
            let mut budget = self.settings.max_actions_per_frame;
            loop {
                let s = &mut self.scripts[i];
                if !s.running || s.finished {
                    break;
                }
                match &s.wait {
                    Wait::Event(_) => break,
                    Wait::Until(t) if self.time < *t => break,
                    Wait::Until(_) => {
                        s.wait = Wait::Nothing;
                        s.pc += 1;
                        continue;
                    }
                    Wait::Nothing => {}
                }
                if s.pc >= s.actions.len() {
                    s.finished = true;
                    self.stats.scripts_finished += 1;
                    let name = s.name.clone();
                    self.out.push(Effect::Note(format!("{name} finished")));
                    break;
                }
                if budget == 0 {
                    let name = s.name.clone();
                    self.out.push(Effect::Note(format!(
                        "{name} ran {} actions without waiting; carries on next frame",
                        self.settings.max_actions_per_frame
                    )));
                    break;
                }
                budget -= 1;
                any = true;
                self.stats.actions_run += 1;
                self.run_action(i);
            }
        }
        any
    }

    fn label(&self, i: usize, name: &str) -> Option<usize> {
        self.scripts[i].actions.iter().position(|a| {
            action_kind(a).eq_ignore_ascii_case("Label")
                && a.text("LabelName")
                    .is_some_and(|l| l.eq_ignore_ascii_case(name))
        })
    }

    /// Run the script's current action. Sets `wait`, or moves `pc` on.
    fn run_action(&mut self, i: usize) {
        let pc = self.scripts[i].pc;
        let a = self.scripts[i].actions[pc].clone();
        let name = self.scripts[i].name.clone();
        let kind = action_kind(&a).to_ascii_lowercase();
        let mut next = pc + 1;
        match kind.as_str() {
            "waitforevent" => {
                let e = a.text("ExternalEvent").unwrap_or("").to_string();
                self.scripts[i].wait = Wait::Event(low(&e));
                return;
            }
            "triggerevent" => {
                if let Some(e) = a.text("Event") {
                    self.fire(e, &name);
                }
            }
            "waitfortimer" => {
                let t = a.float("PauseTime").unwrap_or(0.0);
                self.scripts[i].wait = Wait::Until(self.time + t);
                return;
            }
            "waitforrandomtimer" => {
                let (lo, hi) = match a.structure("PauseTime") {
                    Some(r) => {
                        let f = |k: &str| match r.get(k) {
                            Some(Val::Float(f)) => *f,
                            _ => 0.0,
                        };
                        (f("Min"), f("Max"))
                    }
                    None => (0.0, 0.0),
                };
                let t = lo + (hi - lo) * self.random();
                self.out.push(Effect::Note(format!(
                    "{name}: waits {t:.2} s (random {lo}-{hi})"
                )));
                self.scripts[i].wait = Wait::Until(self.time + t);
                return;
            }
            "label" | "endsection" => {}
            "gotolabel" => {
                let l = a.text("LabelName").unwrap_or("");
                match self.label(i, l) {
                    Some(j) => next = j,
                    None => self.out.push(Effect::Note(format!("{name}: no label {l}"))),
                }
            }
            "gotoaction" => {
                next = a.int("ActionNumber").unwrap_or(0).max(0) as usize;
            }
            "ifcondition" => {
                let tag = a.text("TriggeredConditionTag").unwrap_or("");
                if !self.condition(tag) {
                    next = self.end_of_section(i, pc);
                }
            }
            "ifrandompct" => {
                let pct = a.float("Probability").unwrap_or(0.5);
                if self.random() > pct {
                    next = self.end_of_section(i, pc);
                }
            }
            _ => {
                // Everything else is for the game to carry out. Some also
                // take time.
                let wait = match kind.as_str() {
                    "playsound" if a.flag("bWaitForFinish", false) => self.settings.sound_seconds,
                    "playanim" | "playpropanim" if a.flag("bWaitUntilAnimEnd", true) => {
                        self.settings.anim_seconds
                    }
                    "movetopoint" | "movetoplayer" => self.settings.move_seconds,
                    _ => 0.0,
                };
                // Props are ours to hurt and remove; the rest goes to the
                // game too (characters, the player).
                match kind.as_str() {
                    "hurtactor" => {
                        let tag = a.text("TargetTag").unwrap_or("").to_string();
                        let amount = a.float("amount").unwrap_or(0.0);
                        self.hurt_props(&tag, amount);
                    }
                    "destroyactor" => {
                        let tag = a.text("DestroyTag").unwrap_or("").to_string();
                        self.destroy_props(&tag);
                    }
                    _ => {}
                }
                self.out.push(Effect::Action {
                    script: name,
                    index: pc,
                    action: a,
                });
                if wait > 0.0 {
                    self.scripts[i].wait = Wait::Until(self.time + wait);
                    return;
                }
            }
        }
        self.scripts[i].pc = next;
    }

    /// The action after the `ACTION_EndSection` that closes the section an
    /// `If...` action at `pc` opens (sections can nest).
    fn end_of_section(&self, i: usize, pc: usize) -> usize {
        let actions = &self.scripts[i].actions;
        let mut depth = 0;
        for (j, a) in actions.iter().enumerate().skip(pc + 1) {
            let k = action_kind(a).to_ascii_lowercase();
            if k.starts_with("if") {
                depth += 1;
            } else if k == "endsection" {
                if depth == 0 {
                    return j + 1;
                }
                depth -= 1;
            }
        }
        actions.len()
    }

    /// The scripts' names and classes, for the log.
    pub fn scripts(&self) -> impl Iterator<Item = (&str, &str, &str, usize)> {
        self.scripts.iter().map(|s| {
            (
                s.name.as_str(),
                s.class.as_str(),
                s.tag.as_str(),
                s.actions.len(),
            )
        })
    }
}

#[cfg(test)]
mod tests;
