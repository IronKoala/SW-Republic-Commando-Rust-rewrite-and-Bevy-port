//! Consoles to slice, door locks to slice or breach, and demolition
//! charges: the original's `ActivateItem` classes, as state machines with
//! no Bevy. The game (`rc_scripts_plugin::panels`) feeds them the player's
//! Use button and time, and carries out what they ask ([`Out`]).
//!
//! Everything here follows the original's script source (`Engine.u`
//! `ActivateItem`, `CTGame.u` `HackTerminal`, `ActivateDoor`, `HackDoor`,
//! `BreachDoor`, `CTMarkers.u` `ExplosiveCharge`, `DemolitionCharge`) and
//! its engine code (`Engine.dll` `AActivateItem::DurationMultiplier`):
//!
//! - **States.** A console: `Disabled`, `Standby` (where it starts),
//!   `Active` (being sliced), `Hacked`. A door lock: `Disabled`, `Locked`
//!   (where it starts; being sliced or charged), `Unlocked`, `Damaged`,
//!   and for a breach lock `ChargeLocked` (the charge is on, its fuse
//!   burning). A demolition charge: `Unplaced` (where it starts),
//!   `Placed` (being set), `Armed` (set; the plunger fires it),
//!   `Explode`, `Disabled` (hidden).
//! - **Each state's look and event** is an `ActivationInfo` (its `Event`,
//!   `TransitionSound`, `TransitionEffect`, `Mesh`, `Material`, damage):
//!   entering a state "makes the transition" ([`Out::Transition`]).
//! - **Doing the job.** Starting (`StartSetup`) moves a console to
//!   `Active`, a door lock to `Locked`, a charge to `Placed`. Each frame
//!   while someone is on it, the time grows; it is done after the state's
//!   `Duration` times the item's difficulty multiplier (`Difficulty`
//!   Easy 1, Medium 2, Hard 3, Hardest 6). The player must keep Use held:
//!   letting go stops (`AbortSetup`), and the time starts again unless the
//!   item keeps it (`bSavesProgress`: consoles, slice locks, demolition
//!   charges).
//! - **A sliced door lock** unlocks its partner (`MatchingComponent`, the
//!   breach lock on the same door) and that one unlocks the doors.
//! - **A breach lock** done goes to `ChargeLocked`: the slice lock beside
//!   it can't be used any more, and after `DoorChargeLockedState.Duration`
//!   seconds it goes `Damaged` (and its partner too) and the doors open.
//! - **A demolition charge** done is `Armed`: it then works as a plunger.
//!   Firing the plunger blows it up (`Explode`): its `ChargeExplode` look,
//!   event and damage, and its `PropToDestroy` is destroyed.

use rc_package::class_library::DefaultValue;
use std::collections::HashSet;

/// `ActivateItem.DurationMultiplier` (Engine.dll 0x103b20d0), by the
/// item's `Difficulty`.
pub const MULTIPLIERS: [f32; 4] = [1.0, 2.0, 3.0, 6.0];

/// The `ActivateItemType` of an armed demolition charge (`AIT_Plunger`).
pub const AIT_PLUNGER: u8 = 6;
/// A demolition charge's own (`AIT_Demolition`).
pub const AIT_DEMOLITION: u8 = 5;

/// What kind of item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `HackTerminal`.
    Console,
    /// `HackDoor`.
    SliceLock,
    /// `BreachDoor`.
    BreachLock,
    /// `DemolitionCharge`.
    Charge,
}

impl Kind {
    /// From the class and its parents (`["HackTerminalGeo",
    /// "HackTerminal", "ActivateItem", ...]`).
    pub fn of_chain(chain: &[String]) -> Option<Kind> {
        chain
            .iter()
            .find_map(|c| match c.to_ascii_lowercase().as_str() {
                "hackterminal" => Some(Kind::Console),
                "hackdoor" => Some(Kind::SliceLock),
                "breachdoor" => Some(Kind::BreachLock),
                "demolitioncharge" => Some(Kind::Charge),
                _ => None,
            })
    }

    /// The state it starts in (its `auto state`).
    pub fn auto_state(self) -> State {
        match self {
            Kind::Console => State::Standby,
            Kind::SliceLock | Kind::BreachLock => State::Locked,
            Kind::Charge => State::Unplaced,
        }
    }

    /// Words for the log.
    pub fn what(self) -> &'static str {
        match self {
            Kind::Console => "console",
            Kind::SliceLock => "door lock (slice)",
            Kind::BreachLock => "door lock (breach)",
            Kind::Charge => "demolition charge",
        }
    }

    /// The `ActivationInfo` properties this kind has.
    pub fn infos(self) -> &'static [&'static str] {
        match self {
            Kind::Console => &[
                "DisabledState",
                "StandbyState",
                "ActiveState",
                "HackedState",
            ],
            Kind::SliceLock => &["DoorLockedState", "DoorUnlockedState", "DoorDamagedState"],
            Kind::BreachLock => &[
                "DoorLockedState",
                "DoorUnlockedState",
                "DoorDamagedState",
                "DoorChargeLockedState",
            ],
            Kind::Charge => &[
                "ChargeUnplaced",
                "ChargePlaced",
                "ChargeArmed",
                "ChargeExplode",
            ],
        }
    }
}

/// An item's state (the original's script states).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Disabled,
    Standby,
    Active,
    Hacked,
    Locked,
    Unlocked,
    Damaged,
    ChargeLocked,
    Unplaced,
    Placed,
    Armed,
    Explode,
}

impl State {
    /// A state by its name in a script (`ACTION_GotoState`) for `kind`;
    /// `Auto` is the state it starts in. `None` if `kind` has no such
    /// state.
    pub fn named(kind: Kind, name: &str) -> Option<State> {
        let s = match name.to_ascii_lowercase().as_str() {
            "auto" => return Some(kind.auto_state()),
            "disabled" => State::Disabled,
            "standby" => State::Standby,
            "active" => State::Active,
            "hacked" => State::Hacked,
            "locked" => State::Locked,
            "unlocked" => State::Unlocked,
            "damaged" => State::Damaged,
            "chargelocked" => State::ChargeLocked,
            "unplaced" => State::Unplaced,
            "placed" => State::Placed,
            "armed" => State::Armed,
            "explode" => State::Explode,
            _ => return None,
        };
        let ok = match kind {
            Kind::Console => matches!(
                s,
                State::Disabled | State::Standby | State::Active | State::Hacked
            ),
            Kind::SliceLock => matches!(
                s,
                State::Disabled | State::Locked | State::Unlocked | State::Damaged
            ),
            Kind::BreachLock => matches!(
                s,
                State::Disabled
                    | State::Locked
                    | State::Unlocked
                    | State::Damaged
                    | State::ChargeLocked
            ),
            Kind::Charge => matches!(
                s,
                State::Disabled | State::Unplaced | State::Placed | State::Armed | State::Explode
            ),
        };
        ok.then_some(s)
    }

    pub fn name(self) -> &'static str {
        match self {
            State::Disabled => "Disabled",
            State::Standby => "Standby",
            State::Active => "Active",
            State::Hacked => "Hacked",
            State::Locked => "Locked",
            State::Unlocked => "Unlocked",
            State::Damaged => "Damaged",
            State::ChargeLocked => "ChargeLocked",
            State::Unplaced => "Unplaced",
            State::Placed => "Placed",
            State::Armed => "Armed",
            State::Explode => "Explode",
        }
    }
}

/// One state's look, event and damage: the original's `ActivationInfo`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Info {
    pub event: Option<String>,
    /// `TransitionEffect`: an effect class name (`Explosion_Sparks`).
    pub effect: Option<String>,
    pub effect_offset: [f32; 3],
    /// `TransitionSound`: full path, package first.
    pub sound: Option<String>,
    /// `Material` put on skin `material_index`: full path.
    pub material: Option<String>,
    pub material_index: i32,
    /// `Mesh`: the static mesh it changes to, full path.
    pub mesh: Option<String>,
    /// Seconds (see the module notes for what it times).
    pub duration: f32,
    /// `MyDamageType`: a class name.
    pub damage_type: Option<String>,
    pub damage: f32,
    pub momentum: f32,
    pub radius: f32,
}

impl Info {
    /// From an `ActivationInfo`'s stored fields (missing ones are the
    /// struct's own zero / None).
    pub fn from_fields(fields: &[(String, DefaultValue)]) -> Info {
        let mut i = Info::default();
        let obj = |v: &DefaultValue| match v {
            DefaultValue::Object(s) | DefaultValue::Name(s)
                if !s.is_empty() && !s.eq_ignore_ascii_case("None") =>
            {
                Some(s.clone())
            }
            _ => None,
        };
        let num = |v: &DefaultValue| match v {
            DefaultValue::Float(f) => *f,
            DefaultValue::Int(n) => *n as f32,
            DefaultValue::Byte(b) => *b as f32,
            _ => 0.0,
        };
        for (k, v) in fields {
            match k.to_ascii_lowercase().as_str() {
                "event" => i.event = obj(v),
                "transitioneffect" => i.effect = obj(v).map(|p| last(&p)),
                "transitioneffectoffset" => {
                    if let DefaultValue::Raw(b) = v {
                        if b.len() == 12 {
                            i.effect_offset = [0, 4, 8]
                                .map(|o| f32::from_le_bytes(b[o..o + 4].try_into().unwrap()));
                        }
                    }
                }
                "transitionsound" => i.sound = obj(v),
                "material" => i.material = obj(v),
                "materialindex" => i.material_index = num(v) as i32,
                "mesh" => i.mesh = obj(v),
                "duration" => i.duration = num(v),
                "mydamagetype" => i.damage_type = obj(v).map(|p| last(&p)),
                "damage" => i.damage = num(v),
                "momentumtransfer" => i.momentum = num(v),
                "damageradius" => i.radius = num(v),
                _ => {}
            }
        }
        i
    }
}

fn last(path: &str) -> String {
    path.rsplit('.').next().unwrap_or(path).to_string()
}

/// Who is doing an item's job (its `User`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum User {
    Player,
    /// A squad mate, by name.
    Squad(String),
}

/// What a door lock does to its doors (`BreachDoor`'s `LockDoor`,
/// `UnlockDoor`, `OpenDoor`; a slice lock's do nothing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoorAct {
    /// `GotoState('Locked', 'Close')`.
    Lock,
    /// `GotoState('TriggerControl')`: they open while their trigger is
    /// touched.
    Unlock,
    /// `StayOpenTime = 4`, `GotoState('TriggerControl', 'Open')`.
    Open,
}

/// Something an item asks the game to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    /// `MakeTransition(info)`: send its event, play its sound, start its
    /// effect at `at` plus its offset, change mesh / material, and hurt
    /// what is within its radius.
    Transition {
        panel: String,
        info_name: &'static str,
        info: Info,
        at: [f32; 3],
    },
    /// `TriggerEvent(EventUnderway)` (the player pressed Use on it).
    Event { panel: String, event: String },
    /// `SquadMarker.MakeObsolete()` (`true`) or
    /// `SquadMarker.GotoState('Enabled')` (`false`).
    Marker { panel: String, obsolete: bool },
    Doors {
        panel: String,
        doors: Vec<String>,
        act: DoorAct,
    },
    /// `PropToDestroy.GotoState('PropDestroyed')`.
    DestroyProp { panel: String, prop: String },
    /// `DeductAmmo()`: one of this ammunition class from the user (the
    /// squad leader's, for a squad mate).
    DeductAmmo {
        panel: String,
        ammo: String,
        user: Option<User>,
    },
    /// `SetStaticMesh(mesh)` (a breach lock shows the charge while it is
    /// being placed).
    Mesh { panel: String, mesh: Option<String> },
    /// `bHidden` changed (a disabled demolition charge is hidden).
    Hidden { panel: String, hidden: bool },
    /// `Destroy()`: gone from the level (an exploded charge).
    Gone { panel: String },
    /// `PostStimulusToWorld(...)`: the AI hears it (`ST_DoorBreach`, a
    /// charge's `ExplosionStimulus`).
    Stimulus { panel: String, what: String },
}

/// One item, as read from the map and its class.
#[derive(Debug, Clone, PartialEq)]
pub struct Panel {
    pub name: String,
    /// The map's export number of the actor.
    pub export: usize,
    pub class: String,
    pub tag: String,
    pub kind: Kind,
    /// Unreal units.
    pub location: [f32; 3],
    /// Pitch, yaw, roll, Unreal rotation units.
    pub rotation: [i32; 3],
    /// The state the map starts it in (`InitialState`), else its auto
    /// state.
    pub initial: State,
    pub usable_by_player: bool,
    /// `ActivatePromptText`, `@` for the Use button.
    pub prompt: String,
    pub player_range: f32,
    pub player_offset: f32,
    pub dot_max: f32,
    pub lock_player: bool,
    pub saves_progress: bool,
    /// Whether a squad mate's work is timed here too
    /// (`bNonPlayerUsesTimer`).
    pub squad_timer: bool,
    /// 0 Easy to 3 Hardest.
    pub difficulty: u8,
    pub hud_z_offset: f32,
    pub event_underway: Option<String>,
    /// `AmmunitionRequired`: an ammunition class name.
    pub ammo_required: Option<String>,
    /// `ActivateItemType`.
    pub item_type: u8,
    /// `SquadMarker`'s name.
    pub marker: Option<String>,
    /// The squad marker's `HUDIconX`, `HUDIconY`: the icon the helmet
    /// shows in the use panel (`UHelmet::GetMarkerIconUVs`; with no
    /// marker it shows none).
    pub marker_icon: Option<[u8; 2]>,
    /// `MatchingComponent`'s name.
    pub matching: Option<String>,
    /// A breach lock's `DoorLeft`, `DoorRight`.
    pub doors: Vec<String>,
    /// A charge's `PropToDestroy`.
    pub prop: Option<String>,
    pub damaged_delay: f32,
    pub explode_on_timer: bool,
    /// `default.DurationRemaining` (the class's own).
    pub default_duration: f32,
    /// `AttachItemClass` (the charge the hands carry), a class name.
    pub attach_item: Option<String>,
    /// The `ActivationInfo`s, by property name (`ActiveState`).
    pub infos: Vec<(String, Info)>,
    /// `ExplosiveCharge.ExplosionStimulus`, a stimulus name.
    pub explosion_stimulus: Option<String>,
    // What changes as it is played:
    pub state: State,
    pub usable: bool,
    pub activating: bool,
    pub activated: bool,
    pub player_activated: bool,
    pub used: bool,
    pub elapsed: f32,
    pub remaining: f32,
    pub fraction: f32,
    pub user: Option<User>,
    /// Seconds before the state's `Timer()` (a breach lock's fuse, a
    /// charge's timer).
    pub timer: Option<f32>,
    /// Seconds left of `Sleep(DamagedDelay)` before a damaged door lock
    /// opens its doors.
    pub sleep: Option<f32>,
    pub hidden: bool,
    pub gone: bool,
}

impl Panel {
    /// An item of `kind` with the original's class defaults for
    /// `ActivateItem` and nothing else set: for tests and for building
    /// from a map.
    pub fn new(name: &str, kind: Kind) -> Panel {
        Panel {
            name: name.into(),
            export: 0,
            class: String::new(),
            tag: String::new(),
            kind,
            location: [0.0; 3],
            rotation: [0; 3],
            initial: kind.auto_state(),
            usable_by_player: true,
            prompt: "PRESS AND HOLD @ TO ACTIVATE".into(),
            player_range: 192.0,
            player_offset: 80.0,
            dot_max: 0.0,
            lock_player: false,
            saves_progress: false,
            squad_timer: false,
            difficulty: 0,
            hud_z_offset: 0.0,
            event_underway: None,
            ammo_required: None,
            item_type: 0,
            marker: None,
            marker_icon: None,
            matching: None,
            doors: Vec::new(),
            prop: None,
            damaged_delay: 0.0,
            explode_on_timer: false,
            default_duration: 0.0,
            attach_item: None,
            infos: Vec::new(),
            explosion_stimulus: None,
            state: kind.auto_state(),
            usable: true,
            activating: false,
            activated: false,
            player_activated: false,
            used: false,
            elapsed: 0.0,
            remaining: 0.0,
            fraction: 0.0,
            user: None,
            timer: None,
            sleep: None,
            hidden: false,
            gone: false,
        }
    }

    pub fn info(&self, name: &str) -> Info {
        self.infos
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, i)| i.clone())
            .unwrap_or_default()
    }

    /// `DurationMultiplier()`.
    pub fn multiplier(&self) -> f32 {
        MULTIPLIERS[(self.difficulty as usize).min(3)]
    }

    /// `DefaultDuration()`.
    pub fn default_remaining(&self) -> f32 {
        self.default_duration * self.multiplier()
    }

    /// The `ActivationInfo` its job is timed by in its state now
    /// (`BeginActivating`'s), if it has a job in this state.
    pub fn working_info(&self) -> Option<&'static str> {
        match (self.kind, self.state) {
            (Kind::Console, State::Active) => Some("ActiveState"),
            (Kind::SliceLock | Kind::BreachLock, State::Locked) => Some("DoorLockedState"),
            (Kind::Charge, State::Placed) => Some("ChargePlaced"),
            _ => None,
        }
    }

    /// The direction it faces (`Vector(Rotation)`).
    pub fn facing(&self) -> [f32; 3] {
        let r = |u: i32| u as f32 * std::f32::consts::TAU / 65536.0;
        let (p, y) = (r(self.rotation[0]), r(self.rotation[1]));
        [p.cos() * y.cos(), p.cos() * y.sin(), p.sin()]
    }

    /// Where `UsingItem` holds the player (x, y): `PlayerOffset` along
    /// `Vector(Rotation)` from it, when it locks the player's place.
    pub fn stand(&self) -> Option<[f32; 2]> {
        let f = self.facing();
        self.lock_player.then(|| {
            [
                self.location[0] + self.player_offset * f[0],
                self.location[1] + self.player_offset * f[1],
            ]
        })
    }

    /// Which hand moves `HandSignal.MarkerHandSetup` picks for it now (a
    /// section of `data/combat/hand_signal.toml`), and whether the hands
    /// carry something that starts the job only when it is put on
    /// (`AttachItemClass`: the job starts at the `HolsterAttachment`
    /// moment).
    pub fn hands(&self) -> (&'static str, bool) {
        if self.item_type == AIT_PLUNGER {
            return ("plunger", self.attach_item.is_some());
        }
        match self.kind {
            Kind::Console => ("slice", false),
            Kind::BreachLock => ("breach", self.attach_item.is_some()),
            Kind::Charge if self.state == State::Unplaced => {
                ("demolition_place", self.attach_item.is_some())
            }
            Kind::Charge => ("demolition", false),
            // `HackDoor` is `AIT_HackDoor`, which `MarkerHandSetup` has
            // no case for: `SetupDefaults`.
            Kind::SliceLock => ("defaults", false),
        }
    }
}

/// Every item in a level.
#[derive(Debug, Clone, Default)]
pub struct Panels {
    pub list: Vec<Panel>,
    /// Items (lower-case names) a squad mate is working on now: their
    /// marker is active (`SquadMarker.bMarkerActive`).
    pub busy: HashSet<String>,
    /// What they asked for since the last [`Panels::take`].
    out: Vec<Out>,
    begun: bool,
}

impl Panels {
    pub fn new(list: Vec<Panel>) -> Panels {
        Panels {
            list,
            ..Default::default()
        }
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        self.list
            .iter()
            .position(|p| p.name.eq_ignore_ascii_case(name))
    }

    /// Everything asked for since the last call, in order.
    pub fn take(&mut self) -> Vec<Out> {
        std::mem::take(&mut self.out)
    }

    pub fn begun(&self) -> bool {
        self.begun
    }

    /// The level starts: `PostBeginPlay` (`DurationRemaining =
    /// DefaultDuration()`), and each item enters the state it starts in.
    pub fn begin(&mut self) {
        self.begun = true;
        for i in 0..self.list.len() {
            let p = &mut self.list[i];
            p.remaining = p.default_remaining();
            let s = p.initial;
            self.enter(i, s);
        }
    }

    /// Put back after loading a save: mark as begun without entering any
    /// state again.
    pub fn mark_begun(&mut self) {
        self.begun = true;
    }

    fn busy(&self, i: usize) -> bool {
        self.busy.contains(&self.list[i].name.to_ascii_lowercase())
    }

    fn transition(&mut self, i: usize, info_name: &'static str) {
        let p = &self.list[i];
        self.out.push(Out::Transition {
            panel: p.name.clone(),
            info_name,
            info: p.info(info_name),
            at: p.location,
        });
    }

    /// `SquadMarker.MakeObsolete()` / `GotoState('Enabled')` when its
    /// marker isn't active; `always` for a disabled charge, which doesn't
    /// ask.
    fn marker(&mut self, i: usize, obsolete: bool, always: bool) {
        if self.list[i].marker.is_some() && (always || !self.busy(i)) {
            self.out.push(Out::Marker {
                panel: self.list[i].name.clone(),
                obsolete,
            });
        }
    }

    fn matching(&self, i: usize) -> Option<usize> {
        self.list[i].matching.as_deref().and_then(|m| self.find(m))
    }

    fn doors(&mut self, i: usize, act: DoorAct) {
        let p = &self.list[i];
        if p.kind == Kind::BreachLock && !p.doors.is_empty() {
            self.out.push(Out::Doors {
                panel: p.name.clone(),
                doors: p.doors.clone(),
                act,
            });
        }
    }

    /// `LockDoor()`.
    fn lock_door(&mut self, i: usize) {
        if self.list[i].kind != Kind::BreachLock {
            return;
        }
        self.doors(i, DoorAct::Lock);
        if let Some(m) = self.matching(i) {
            if self.list[m].state != State::Locked {
                self.goto(m, State::Locked);
            }
        }
    }

    /// `OpenDoor()`.
    fn open_door(&mut self, i: usize) {
        if self.list[i].kind != Kind::BreachLock {
            return;
        }
        self.doors(i, DoorAct::Open);
        self.out.push(Out::Stimulus {
            panel: self.list[i].name.clone(),
            what: "ST_DoorBreach".into(),
        });
    }

    fn deduct_ammo(&mut self, i: usize) {
        let p = &self.list[i];
        if let (Some(ammo), Some(_)) = (&p.ammo_required, &p.user) {
            self.out.push(Out::DeductAmmo {
                panel: p.name.clone(),
                ammo: ammo.clone(),
                user: p.user.clone(),
            });
        }
    }

    /// `GotoState(state)`: leave the state it is in, enter `state` (even
    /// the same one again, as the original does).
    pub fn goto(&mut self, i: usize, state: State) {
        if self.list[i].gone {
            return;
        }
        // EndState.
        if self.list[i].kind == Kind::Charge && self.list[i].state == State::Disabled {
            self.list[i].hidden = false;
            self.out.push(Out::Hidden {
                panel: self.list[i].name.clone(),
                hidden: false,
            });
        }
        self.enter(i, state);
    }

    /// Enter `state`: its `BeginState`.
    fn enter(&mut self, i: usize, state: State) {
        self.list[i].state = state;
        // Timers and latent sleeps belong to the state left.
        self.list[i].timer = None;
        self.list[i].sleep = None;
        let kind = self.list[i].kind;
        match (kind, state) {
            (Kind::Console, State::Disabled) => {
                self.list[i].usable = false;
                self.marker(i, true, false);
                self.transition(i, "DisabledState");
            }
            (Kind::Console, State::Standby) => {
                self.list[i].usable = true;
                self.marker(i, false, false);
                self.transition(i, "StandbyState");
            }
            (Kind::Console, State::Active) => {
                self.list[i].usable = true;
                self.transition(i, "ActiveState");
            }
            (Kind::Console, State::Hacked) => {
                self.list[i].usable = false;
                self.marker(i, true, false);
                self.transition(i, "HackedState");
            }
            (Kind::SliceLock | Kind::BreachLock, State::Disabled) => {
                self.list[i].usable = false;
                self.transition(i, "DoorLockedState");
                self.marker(i, true, false);
                self.lock_door(i);
            }
            (Kind::SliceLock | Kind::BreachLock, State::Locked) => {
                self.list[i].usable = true;
                self.transition(i, "DoorLockedState");
                self.marker(i, false, false);
                self.lock_door(i);
            }
            (Kind::SliceLock | Kind::BreachLock, State::Unlocked) => {
                self.list[i].usable = false;
                self.marker(i, true, false);
                if let Some(m) = self.matching(i) {
                    if self.list[m].state != State::Unlocked {
                        self.goto(m, State::Unlocked);
                    }
                }
                self.doors(i, DoorAct::Unlock);
                self.transition(i, "DoorUnlockedState");
            }
            (Kind::SliceLock | Kind::BreachLock, State::Damaged) => {
                self.list[i].usable = false;
                self.marker(i, true, false);
                if let Some(m) = self.matching(i) {
                    if self.list[m].state != State::Damaged {
                        self.goto(m, State::Damaged);
                    }
                }
                self.transition(i, "DoorDamagedState");
                // Begin: Sleep(DamagedDelay) if above 0, then OpenDoor().
                let delay = self.list[i].damaged_delay;
                if delay > 0.0 {
                    self.list[i].sleep = Some(delay);
                } else {
                    self.open_door(i);
                }
            }
            (Kind::BreachLock, State::ChargeLocked) => {
                self.deduct_ammo(i);
                if let Some(m) = self.matching(i) {
                    self.list[m].usable = false;
                    self.marker(m, true, false);
                }
                self.transition(i, "DoorChargeLockedState");
                let fuse = self.list[i].info("DoorChargeLockedState").duration;
                // SetTimer(0, false) clears the timer.
                self.list[i].timer = (fuse > 0.0).then_some(fuse);
                self.marker(i, true, false);
                self.list[i].usable = false;
            }
            (Kind::Charge, State::Unplaced) => {
                self.list[i].usable = true;
                self.transition(i, "ChargeUnplaced");
                self.marker(i, false, false);
            }
            (Kind::Charge, State::Placed) => {
                self.transition(i, "ChargePlaced");
            }
            (Kind::Charge, State::Armed) => {
                self.deduct_ammo(i);
                self.list[i].activated = true;
                self.list[i].usable = false;
                self.marker(i, true, false);
                self.transition(i, "ChargeArmed");
                let armed = self.list[i].info("ChargeArmed").duration;
                if armed > 0.0 && self.list[i].explode_on_timer {
                    self.list[i].timer = Some(armed);
                }
                // DemolitionCharge's own Armed: now a plunger.
                self.list[i].item_type = AIT_PLUNGER;
            }
            (Kind::Charge, State::Explode) => {
                self.list[i].usable = false;
                self.list[i].item_type = AIT_DEMOLITION;
                self.marker(i, true, false);
                self.transition(i, "ChargeExplode");
                if let Some(prop) = self.list[i].prop.clone() {
                    self.out.push(Out::DestroyProp {
                        panel: self.list[i].name.clone(),
                        prop,
                    });
                }
                if let Some(s) = self.list[i].explosion_stimulus.clone() {
                    self.out.push(Out::Stimulus {
                        panel: self.list[i].name.clone(),
                        what: s,
                    });
                }
                self.list[i].hidden = true;
                self.list[i].gone = true;
                self.out.push(Out::Gone {
                    panel: self.list[i].name.clone(),
                });
            }
            (Kind::Charge, State::Disabled) => {
                self.list[i].hidden = true;
                self.list[i].usable = false;
                self.out.push(Out::Hidden {
                    panel: self.list[i].name.clone(),
                    hidden: true,
                });
                self.marker(i, true, true);
            }
            // A state this kind doesn't have: the original stays in it
            // with no BeginState of its own.
            _ => {}
        }
    }

    /// The player pressed Use while aimed at it (`CTPlayer.Use`, case
    /// `TGT_Panel`). Returns whether the player starts using it (the hands
    /// come up and the player is held: state `UsingItem`).
    pub fn press_use(&mut self, i: usize) -> bool {
        if self.list[i].activating {
            return false;
        }
        let p = &mut self.list[i];
        p.player_activated = true;
        p.used = true;
        if let Some(e) = p.event_underway.clone() {
            self.out.push(Out::Event {
                panel: p.name.clone(),
                event: e,
            });
        }
        self.list[i].usable
    }

    /// `StartSetup(user)`: someone starts the job.
    pub fn start_setup(&mut self, i: usize, user: User) {
        self.list[i].user = Some(user);
        // InternalSetup.
        match self.list[i].kind {
            Kind::Console => {
                if self.list[i].state != State::Active {
                    self.goto(i, State::Active);
                }
            }
            Kind::SliceLock | Kind::BreachLock => {
                if self.list[i].state != State::Locked {
                    self.goto(i, State::Locked);
                }
                self.list[i].activating = true;
                if self.list[i].kind == Kind::BreachLock {
                    let mesh = self.list[i].info("DoorChargeLockedState").mesh;
                    self.out.push(Out::Mesh {
                        panel: self.list[i].name.clone(),
                        mesh,
                    });
                }
            }
            Kind::Charge => {
                if self.list[i].state != State::Placed {
                    self.goto(i, State::Placed);
                }
            }
        }
        self.list[i].activating = true;
    }

    /// `AbortSetup()`: the user stopped.
    pub fn abort(&mut self, i: usize) {
        let p = &mut self.list[i];
        p.activating = false;
        p.player_activated = false;
        p.user = None;
        if !p.saves_progress {
            p.remaining = p.default_remaining();
            p.elapsed = 0.0;
            p.fraction = 0.0;
            let auto = p.kind.auto_state();
            self.goto(i, auto);
        }
        match self.list[i].kind {
            Kind::Console if self.list[i].state == State::Active => {
                self.goto(i, State::Standby);
            }
            Kind::BreachLock => {
                let mesh = self.list[i].info("DoorLockedState").mesh;
                self.out.push(Out::Mesh {
                    panel: self.list[i].name.clone(),
                    mesh,
                });
            }
            _ => {}
        }
    }

    /// `NextState()` of the state it is in.
    fn next_state(&mut self, i: usize) {
        let next = match (self.list[i].kind, self.list[i].state) {
            (Kind::Console, State::Active) => Some(State::Hacked),
            (Kind::SliceLock, State::Locked) => Some(State::Unlocked),
            (Kind::BreachLock, State::Locked) => Some(State::ChargeLocked),
            (Kind::Charge, State::Placed) => Some(State::Armed),
            _ => None,
        };
        if let Some(s) = next {
            self.goto(i, s);
        }
    }

    /// One frame of `dt` seconds. `holding` is whether the player keeps
    /// Use held (in `UsingItem` the target stays the panel). Runs each
    /// state's `Tick` (`BeginActivating`), timers and sleeps.
    pub fn tick(&mut self, dt: f32, holding: bool) {
        for i in 0..self.list.len() {
            if self.list[i].gone {
                continue;
            }
            if let Some(name) = self.list[i].working_info() {
                if self.list[i].user.is_some() {
                    self.begin_activating(i, name, dt, holding);
                }
            }
            if let Some(t) = self.list[i].timer {
                let left = t - dt;
                if left <= 0.0 {
                    self.list[i].timer = None;
                    // The state's Timer().
                    match (self.list[i].kind, self.list[i].state) {
                        (Kind::BreachLock, State::ChargeLocked) => self.goto(i, State::Damaged),
                        (Kind::Charge, State::Armed) => self.goto(i, State::Explode),
                        _ => {}
                    }
                } else {
                    self.list[i].timer = Some(left);
                }
            }
            if let Some(t) = self.list[i].sleep {
                let left = t - dt;
                if left <= 0.0 {
                    self.list[i].sleep = None;
                    if self.list[i].state == State::Damaged {
                        self.open_door(i);
                    }
                } else {
                    self.list[i].sleep = Some(left);
                }
            }
        }
    }

    /// `BeginActivating(State, deltaTime)`.
    fn begin_activating(&mut self, i: usize, info_name: &str, dt: f32, holding: bool) {
        let p = &self.list[i];
        let Some(user) = p.user.clone() else {
            self.abort(i);
            return;
        };
        let player = user == User::Player;
        if !player && !p.squad_timer {
            return;
        }
        if !player || holding {
            let duration = p.info(info_name).duration * p.multiplier();
            if p.elapsed + dt >= duration {
                self.next_state(i);
                let p = &mut self.list[i];
                p.activating = false;
                p.remaining = 0.0;
            } else {
                let p = &mut self.list[i];
                p.elapsed += dt;
                p.remaining -= dt;
                p.fraction = if duration > 0.0 {
                    p.elapsed / duration
                } else {
                    0.0
                };
            }
        } else {
            self.abort(i);
        }
    }

    /// The plunger fired (`HandSignal.FireWeapon` on an armed charge).
    pub fn fire_plunger(&mut self, i: usize) {
        if self.list[i].item_type == AIT_PLUNGER {
            self.goto(i, State::Explode);
        }
    }
}

/// Read every console, door lock and demolition charge in a map: the
/// map's own values over its class's defaults (`lib`, the game's script
/// packages and `Properties/MarkerProperties.u`). Returns them and lines
/// for the log about anything that couldn't be read.
pub fn read(
    pkg: &rc_package::Package,
    data: &[u8],
    level: &rc_level::actors::LevelInfo,
    lib: &rc_package::class_library::ClassLibrary,
) -> (Vec<Panel>, Vec<String>) {
    use rc_package::class_library::struct_fields;
    use rc_package::properties::{find, Value};
    let mut out = Vec::new();
    let mut problems = Vec::new();
    for &a in level.actors.iter().flatten() {
        let class = pkg.export_class_name(&pkg.exports[a]).to_string();
        let chain = lib.chain(&class);
        let Some(kind) = Kind::of_chain(&chain) else {
            continue;
        };
        let name = pkg.name(pkg.exports[a].object_name).to_string();
        let props = match rc_level::actors::read_actor_properties(pkg, data, a) {
            Ok(p) => p,
            Err(e) => {
                problems.push(format!("{name} ({class}): {e}"));
                continue;
            }
        };
        let defaults = lib.defaults(&class);
        let map = |n: &str| find(pkg, &props, n).map(|p| &p.value);
        let float = |n: &str| match map(n) {
            Some(Value::Float(f)) => Some(*f),
            _ => defaults.number(n),
        };
        let boolean = |n: &str| match map(n) {
            Some(Value::Bool(b)) => Some(*b),
            _ => defaults.bool(n),
        };
        let byte = |n: &str| match map(n) {
            Some(Value::Byte(b)) => Some(*b),
            _ => match defaults.get(n).map(|e| &e.value) {
                Some(DefaultValue::Byte(b)) => Some(*b),
                _ => None,
            },
        };
        let object = |n: &str| -> Option<String> {
            let s = match map(n) {
                Some(Value::Object(o)) => match rc_package::ObjectRef::from_index(*o) {
                    rc_package::ObjectRef::None => return None,
                    _ => pkg.object_name(*o).to_string(),
                },
                Some(Value::Name(o)) => pkg.name(*o).to_string(),
                _ => defaults.object(n)?.to_string(),
            };
            (!s.is_empty() && !s.eq_ignore_ascii_case("None")).then_some(s)
        };
        let string = |n: &str| match map(n) {
            Some(Value::Raw(b)) => rc_package::Reader::new(b).fstring().ok(),
            _ => defaults.string(n),
        };
        let raw12 = |n: &str| match map(n) {
            Some(Value::Raw(b)) if b.len() == 12 => Some(b.clone()),
            _ => None,
        };
        let mut p = Panel::new(&name, kind);
        p.export = a;
        p.class = class.clone();
        p.tag = object("Tag").unwrap_or_else(|| class.clone());
        if let Some(b) = raw12("Location") {
            p.location = [0, 4, 8].map(|o| f32::from_le_bytes(b[o..o + 4].try_into().unwrap()));
        }
        if let Some(b) = raw12("Rotation") {
            p.rotation = [0, 4, 8].map(|o| i32::from_le_bytes(b[o..o + 4].try_into().unwrap()));
        }
        if let Some(s) = object("InitialState").and_then(|s| State::named(kind, &s)) {
            p.initial = s;
            p.state = s;
        }
        p.usable_by_player = boolean("bIsUsableByPlayer").unwrap_or(true);
        p.usable = boolean("bIsUsable").unwrap_or(true);
        if let Some(t) = string("ActivatePromptText") {
            p.prompt = t;
        }
        p.player_range = float("PlayerRange").unwrap_or(0.0);
        p.player_offset = float("PlayerOffset").unwrap_or(0.0);
        p.dot_max = float("ActivateDotProdMax").unwrap_or(0.0);
        p.lock_player = boolean("bLockPlayerLocation").unwrap_or(false);
        p.saves_progress = boolean("bSavesProgress").unwrap_or(false);
        p.squad_timer = boolean("bNonPlayerUsesTimer").unwrap_or(false);
        p.difficulty = byte("Difficulty").unwrap_or(0);
        p.hud_z_offset = float("HUDMarkerIconZOffset").unwrap_or(0.0);
        p.event_underway = object("EventUnderway");
        p.ammo_required = object("AmmunitionRequired");
        p.item_type = byte("ActivateItemType").unwrap_or(0);
        p.marker = object("SquadMarker");
        p.marker_icon = p
            .marker
            .as_deref()
            .and_then(|m| marker_icon(pkg, data, level, lib, m));
        p.matching = object("MatchingComponent");
        p.doors = ["DoorLeft", "DoorRight"]
            .iter()
            .filter_map(|n| object(n))
            .collect();
        p.prop = object("PropToDestroy");
        p.damaged_delay = float("DamagedDelay").unwrap_or(0.0);
        p.explode_on_timer = boolean("bExplodeOnTimer").unwrap_or(false);
        // `default.DurationRemaining`: the class's, never the map's.
        p.default_duration = defaults.number("DurationRemaining").unwrap_or(0.0);
        p.attach_item = object("AttachItemClass");
        p.explosion_stimulus = match defaults.get("ExplosionStimulus").map(|e| &e.value) {
            Some(DefaultValue::Byte(b)) => Some(format!("stimulus {b}")),
            _ => None,
        };
        for info in kind.infos() {
            // The map stores a struct whole when it changes any of it.
            let fields = match find(pkg, &props, info).map(|p| &p.value) {
                Some(Value::Raw(b)) => struct_fields(pkg, Some("myLevel"), b),
                _ => lib.struct_default(&class, info),
            };
            match fields {
                Some(f) => p.infos.push((info.to_string(), Info::from_fields(&f))),
                None => problems.push(format!("{name} ({class}): {info} not found")),
            }
        }
        out.push(p);
    }
    (out, problems)
}

/// A squad marker's `HUDIconX`, `HUDIconY` (the map's, else its class's).
fn marker_icon(
    pkg: &rc_package::Package,
    data: &[u8],
    level: &rc_level::actors::LevelInfo,
    lib: &rc_package::class_library::ClassLibrary,
    name: &str,
) -> Option<[u8; 2]> {
    use rc_package::properties::{find, Value};
    let &a = level.actors.iter().flatten().find(|&&a| {
        pkg.name(pkg.exports[a].object_name)
            .eq_ignore_ascii_case(name)
    })?;
    let class = pkg.export_class_name(&pkg.exports[a]).to_string();
    let props = rc_level::actors::read_actor_properties(pkg, data, a).ok()?;
    let defaults = lib.defaults(&class);
    let byte = |n: &str| match find(pkg, &props, n).map(|p| &p.value) {
        Some(Value::Byte(b)) => Some(*b),
        _ => match defaults.get(n).map(|e| &e.value) {
            Some(DefaultValue::Byte(b)) => Some(*b),
            _ => None,
        },
    };
    // Unset is the byte's zero, as in UnrealScript.
    Some([byte("HUDIconX").unwrap_or(0), byte("HUDIconY").unwrap_or(0)])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(event: &str, duration: f32) -> Info {
        Info {
            event: Some(event.into()),
            duration,
            ..Info::default()
        }
    }

    /// geo_01b's shield console, as its class and the map set it.
    fn console() -> Panel {
        let mut p = Panel::new("HackTerminalGeo0", Kind::Console);
        p.saves_progress = true;
        p.lock_player = true;
        p.player_offset = 85.0;
        p.default_duration = 10.0;
        p.marker = Some("MarkerHackTerminal0".into());
        p.infos = vec![
            ("StandbyState".into(), info("", 10.0)),
            ("ActiveState".into(), info("ShieldHackActivated", 10.0)),
            ("HackedState".into(), info("EntranceShieldHacked", 0.0)),
        ];
        p
    }

    fn events(out: &[Out]) -> Vec<String> {
        out.iter()
            .filter_map(|o| match o {
                Out::Transition { info, .. } => info.event.clone().filter(|e| !e.is_empty()),
                Out::Event { event, .. } => Some(event.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn slicing_a_console_takes_its_time_times_the_difficulty() {
        let mut c = console();
        c.difficulty = 1; // Medium: x2
        let mut ps = Panels::new(vec![c]);
        ps.begin();
        assert_eq!(ps.list[0].state, State::Standby);
        assert_eq!(ps.list[0].remaining, 20.0);
        ps.take();
        assert!(ps.press_use(0));
        ps.start_setup(0, User::Player);
        assert_eq!(ps.list[0].state, State::Active);
        for _ in 0..190 {
            ps.tick(0.1, true);
        }
        assert_eq!(ps.list[0].state, State::Active);
        assert!((ps.list[0].fraction - 0.95).abs() < 1e-3);
        for _ in 0..11 {
            ps.tick(0.1, true);
        }
        assert_eq!(ps.list[0].state, State::Hacked);
        assert!(!ps.list[0].usable);
        let e = events(&ps.take());
        assert_eq!(e, vec!["ShieldHackActivated", "EntranceShieldHacked"]);
    }

    #[test]
    fn letting_go_keeps_a_consoles_progress() {
        let mut ps = Panels::new(vec![console()]);
        ps.begin();
        ps.press_use(0);
        ps.start_setup(0, User::Player);
        for _ in 0..40 {
            ps.tick(0.1, true);
        }
        ps.tick(0.1, false);
        // Back to Standby, the time kept, nobody on it.
        assert_eq!(ps.list[0].state, State::Standby);
        assert!(ps.list[0].user.is_none());
        assert!((ps.list[0].elapsed - 4.0).abs() < 1e-3);
        ps.press_use(0);
        ps.start_setup(0, User::Player);
        for _ in 0..61 {
            ps.tick(0.1, true);
        }
        assert_eq!(ps.list[0].state, State::Hacked);
    }

    /// geo_01b's hangar door: a slice lock and a breach lock on the same
    /// double door.
    fn door() -> Vec<Panel> {
        let mut slice = Panel::new("HackDoorGeo6", Kind::SliceLock);
        slice.saves_progress = true;
        slice.default_duration = 10.0;
        slice.item_type = 1;
        slice.matching = Some("BreachDoorGeo0".into());
        slice.marker = Some("MarkerHackDoor53".into());
        slice.infos = vec![
            ("DoorLockedState".into(), info("", 10.0)),
            ("DoorUnlockedState".into(), info("HangarDoorHacked", 0.0)),
        ];
        let mut breach = Panel::new("BreachDoorGeo0", Kind::BreachLock);
        breach.usable_by_player = false;
        breach.item_type = 2;
        breach.player_offset = 87.0;
        breach.attach_item = Some("DoorChargeObject".into());
        breach.doors = vec!["MoverDB_GEO6".into(), "MoverDBSlave_GEO6".into()];
        breach.infos = vec![
            ("DoorLockedState".into(), info("", 2.0)),
            ("DoorChargeLockedState".into(), info("", 2.5)),
            ("DoorDamagedState".into(), info("DoorBlown", 0.0)),
            ("DoorUnlockedState".into(), info("", 0.0)),
        ];
        vec![slice, breach]
    }

    #[test]
    fn slicing_a_door_lock_unlocks_its_partner_and_the_doors() {
        let mut ps = Panels::new(door());
        ps.begin();
        ps.take();
        ps.press_use(0);
        ps.start_setup(0, User::Player);
        for _ in 0..101 {
            ps.tick(0.1, true);
        }
        assert_eq!(ps.list[0].state, State::Unlocked);
        assert_eq!(ps.list[1].state, State::Unlocked);
        let out = ps.take();
        assert!(
            out.iter().any(
                |o| matches!(o, Out::Doors { act: DoorAct::Unlock, doors, .. } if doors.len() == 2)
            ),
            "{out:?}"
        );
        assert_eq!(events(&out), vec!["HangarDoorHacked"]);
    }

    #[test]
    fn a_breach_charge_burns_then_blows_the_door() {
        let mut ps = Panels::new(door());
        ps.begin();
        ps.take();
        ps.start_setup(1, User::Player);
        for _ in 0..21 {
            ps.tick(0.1, true);
        }
        assert_eq!(ps.list[1].state, State::ChargeLocked);
        // geo_01b's breach lock doesn't name the slice lock (only the slice
        // lock names it), so the slice lock is left as it is.
        assert!(ps.list[0].usable);
        // The fuse runs whether or not Use is held.
        for _ in 0..22 {
            ps.tick(0.1, false);
        }
        assert_eq!(ps.list[1].state, State::ChargeLocked);
        ps.tick(0.1, false);
        ps.tick(0.1, false);
        assert_eq!(ps.list[1].state, State::Damaged);
        assert_eq!(ps.list[0].state, State::Locked);
        let out = ps.take();
        assert!(
            out.iter().any(|o| matches!(
                o,
                Out::Doors {
                    act: DoorAct::Open,
                    ..
                }
            )),
            "{out:?}"
        );
    }

    #[test]
    fn a_breach_lock_starts_over_when_let_go() {
        let mut ps = Panels::new(door());
        ps.begin();
        ps.start_setup(1, User::Player);
        for _ in 0..10 {
            ps.tick(0.1, true);
        }
        ps.tick(0.1, false);
        assert_eq!(ps.list[1].state, State::Locked);
        assert_eq!(ps.list[1].elapsed, 0.0);
    }

    #[test]
    fn a_demolition_charge_is_placed_then_fired_by_plunger() {
        let mut c = Panel::new("DemolitionCharge0", Kind::Charge);
        c.saves_progress = true;
        c.item_type = AIT_DEMOLITION;
        c.prop = Some("StaticProp4".into());
        c.infos = vec![
            ("ChargePlaced".into(), info("", 10.0)),
            ("ChargeArmed".into(), info("ChargeArmed", 0.0)),
            ("ChargeExplode".into(), info("Fighter01Destroyed", 0.0)),
        ];
        let mut ps = Panels::new(vec![c]);
        ps.begin();
        ps.take();
        assert_eq!(ps.list[0].hands(), ("demolition_place", false));
        ps.start_setup(0, User::Player);
        for _ in 0..101 {
            ps.tick(0.1, true);
        }
        assert_eq!(ps.list[0].state, State::Armed);
        assert_eq!(ps.list[0].item_type, AIT_PLUNGER);
        assert_eq!(ps.list[0].hands().0, "plunger");
        // Nothing goes off on its own.
        for _ in 0..100 {
            ps.tick(0.1, false);
        }
        assert_eq!(ps.list[0].state, State::Armed);
        ps.fire_plunger(0);
        assert!(ps.list[0].gone);
        let out = ps.take();
        assert!(out
            .iter()
            .any(|o| matches!(o, Out::DestroyProp { prop, .. } if prop == "StaticProp4")));
        assert_eq!(events(&out), vec!["ChargeArmed", "Fighter01Destroyed"]);
    }

    #[test]
    fn a_disabled_console_comes_back_on_standby() {
        let mut ps = Panels::new(vec![console()]);
        ps.list[0].initial = State::Disabled;
        ps.begin();
        assert!(!ps.list[0].usable);
        let out = ps.take();
        assert!(out.contains(&Out::Marker {
            panel: "HackTerminalGeo0".into(),
            obsolete: true
        }));
        let s = State::named(Kind::Console, "Auto").unwrap();
        ps.goto(0, s);
        assert!(ps.list[0].usable);
    }

    #[test]
    fn where_the_player_stands() {
        let mut c = console();
        c.location = [100.0, 0.0, 0.0];
        c.rotation = [0, 16384, 0];
        let s = c.stand().unwrap();
        assert!(
            (s[0] - 100.0).abs() < 1e-3 && (s[1] - 85.0).abs() < 1e-3,
            "{s:?}"
        );
    }
}
