//! The squad's marked jobs ("context orders"): what each kind of map
//! marker makes a squad mate do. The settings are `[job.<name>]` sections
//! in `data/squad/squad.toml`, so a mod can change any job or add new ones.
//!
//! Two kinds of job:
//! - **once**: walk to the spot, work for `work_time` seconds, then the
//!   job is done and its `result` happens (after `fuse` seconds, if set).
//!   A `group` job (door breach) first sends a member to every spot that
//!   shares the same anchor, and work starts when all of them are in place.
//! - **post**: walk to the spot and stay there, fighting from it with the
//!   job's own weapon and sight (sniper, anti-armour) or throwing grenades
//!   (`throw_...`), until the player recalls the squad (F4). A `mount`
//!   post (manning a turret) gets on the anchor instead: the game holds
//!   them on it and fires the turret, not their own gun.
//!
//! A map marker gets the first job whose `match` words appear in its order
//! marker's class (`MarkerDoorBreach`), or, for one with no order marker,
//! its anchor's class (`BactaDispenserGEO`).

/// One-off job or a post held until recalled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Once,
    Post,
}

/// When a squad mate whose `speciality` is this job uses it on their own
/// in a fight, with no marker (the original's Sev snipes, Fixer throws
/// grenades and Scorch fires anti-armour rounds without being told).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoUse {
    /// Only from a marked post.
    Never,
    /// The job's gun, at enemies at least `auto_range` away (Sev).
    Far,
    /// The job's gun, at armoured enemies (Scorch).
    Armoured,
    /// The job's throw, at a bunch of enemies (Fixer).
    Group,
}

/// What happens when a one-off job is finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobResult {
    /// Nothing more than the job being done.
    Nothing,
    /// The member who did it gets their health and shield back (bacta).
    Heal,
    /// The anchor is set off: its door opens, or what it guards is
    /// destroyed. The game turns this into the anchor's own event.
    Trigger,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JobSettings {
    /// The key in the file, like `bacta`.
    pub key: String,
    /// What the squad panel and the log call it.
    pub name: String,
    /// Words to find in the marker's class name (lower case).
    pub matches: Vec<String>,
    pub kind: JobKind,
    /// The squad mate who does it when free (a member's `class` or key);
    /// "" = whoever is nearest.
    pub preferred: String,
    /// Seconds of work at the spot (once).
    pub work_time: f32,
    pub result: JobResult,
    /// Send a member to every spot of the same anchor (door breach).
    pub group: bool,
    /// Post: with "auto pull maneuvers" on, a new order that needs more
    /// squad mates than are free may take a member off this post (the
    /// original's `bCancelIfMembersNeeded`, set on its sniper, grenade,
    /// anti-armour and turret markers).
    pub pullable: bool,
    /// Post: the anchor is a mounted turret the squad mate gets on once at
    /// the spot (the original's `MarkerTurret`). The game holds them on it
    /// and fires the turret; their own gun stays quiet. Where the turret
    /// is taken (the player is on it) or missing, they fight from the spot.
    pub mount: bool,
    /// Seconds from the end of the work to the blast and the result.
    pub fuse: f32,
    /// A blast at the anchor when the fuse runs out: damage at the middle,
    /// falling to nothing at `blast_radius`. 0 = no blast.
    pub blast_damage: f32,
    pub blast_radius: f32,
    /// Post: the gun used from it (a file in data/weapons), "" for the
    /// usual one.
    pub weapon: String,
    /// Post: shooting numbers that replace `[fight]`'s while posted. 0 (or
    /// missing) keeps `[fight]`'s.
    pub sight_radius: f32,
    pub accuracy: f32,
    pub burst_count: u32,
    pub shot_interval: f32,
    pub burst_pause_min: f32,
    pub burst_pause_max: f32,
    /// Post: throw a grenade every `throw_interval` seconds at an enemy
    /// within `throw_range`. 0 = don't throw.
    pub throw_interval: f32,
    pub throw_range: f32,
    /// Seconds from the throw to the blast, and the blast's size.
    pub throw_fuse: f32,
    pub throw_damage: f32,
    pub throw_radius: f32,
    /// Post with a target area (a grenade marker's `WeaponTarget`): throw
    /// only at enemies this close to it.
    pub target_radius: f32,
    /// Used on their own by a squad mate with this `speciality` (see
    /// `AutoUse`), and the conditions for it.
    pub auto: AutoUse,
    /// The target must be at least this far away, uu (Far: what counts as
    /// far; the others: out of the blast).
    pub auto_range: f32,
    /// Group: seconds between throws on their own.
    pub auto_interval: f32,
    /// Group: enemies within `throw_radius` of the target, counting it.
    pub auto_min_enemies: u32,
    /// No friend (the player, a squad mate, or the thrower) within this
    /// of the target, uu. 0 = don't check.
    pub auto_clearance: f32,
}

impl JobSettings {
    fn new(key: &str, name: &str, matches: &[&str], kind: JobKind) -> Self {
        JobSettings {
            key: key.into(),
            name: name.into(),
            matches: matches.iter().map(|s| s.to_string()).collect(),
            kind,
            preferred: String::new(),
            work_time: 3.0,
            result: JobResult::Nothing,
            group: false,
            pullable: false,
            mount: false,
            fuse: 0.0,
            blast_damage: 0.0,
            blast_radius: 0.0,
            weapon: String::new(),
            sight_radius: 0.0,
            accuracy: 0.0,
            burst_count: 0,
            shot_interval: 0.0,
            burst_pause_min: 0.0,
            burst_pause_max: 0.0,
            throw_interval: 0.0,
            throw_range: 0.0,
            throw_fuse: 0.0,
            throw_damage: 0.0,
            throw_radius: 0.0,
            target_radius: 0.0,
            auto: AutoUse::Never,
            auto_range: 0.0,
            auto_interval: 0.0,
            auto_min_enemies: 1,
            auto_clearance: 0.0,
        }
    }

    /// A blank job for a new `[job.<key>]` section in a mod.
    pub fn blank(key: &str) -> Self {
        JobSettings::new(key, key, &[], JobKind::Once)
    }

    /// Does this job fit a marker whose anchor has this class (or, with
    /// no anchor, this name)?
    pub fn fits(&self, class_or_name: &str) -> bool {
        let c = class_or_name.to_ascii_lowercase();
        self.matches
            .iter()
            .any(|m| !m.is_empty() && c.contains(m.as_str()))
    }

    /// One line for the log.
    pub fn summary(&self) -> String {
        let mut s = format!(
            "job {} ({}): matches {:?}, {}",
            self.key,
            self.name,
            self.matches,
            match self.kind {
                JobKind::Once => "once",
                JobKind::Post => "post",
            }
        );
        match self.kind {
            JobKind::Once => {
                s += &format!(", work {} s, result {:?}", self.work_time, self.result);
                if self.group {
                    s += ", group";
                }
                if self.fuse > 0.0 {
                    s += &format!(", fuse {} s", self.fuse);
                }
                if self.blast_damage > 0.0 {
                    s += &format!(
                        ", blast {} within {} uu",
                        self.blast_damage, self.blast_radius
                    );
                }
            }
            JobKind::Post => {
                if !self.weapon.is_empty() {
                    s += &format!(", weapon {}", self.weapon);
                }
                if self.sight_radius > 0.0 {
                    s += &format!(", sees {} uu", self.sight_radius);
                }
                if self.pullable {
                    s += ", pullable";
                }
                if self.mount {
                    s += ", gets on the anchor (a turret)";
                }
                if self.throw_interval > 0.0 {
                    s += &format!(
                        ", throws every {} s within {} uu ({} damage within {} uu after {} s)",
                        self.throw_interval,
                        self.throw_range,
                        self.throw_damage,
                        self.throw_radius,
                        self.throw_fuse
                    );
                }
            }
        }
        match self.auto {
            AutoUse::Never => {}
            AutoUse::Far => {
                s += &format!(", on their own at enemies {}+ uu away", self.auto_range)
            }
            AutoUse::Armoured => s += &format!(
                ", on their own at armoured enemies {}+ uu away, friends {}+ uu from them",
                self.auto_range, self.auto_clearance
            ),
            AutoUse::Group => s += &format!(
                ", on their own every {} s at {}+ enemies together {}+ uu away, friends {}+ uu from them",
                self.auto_interval, self.auto_min_enemies, self.auto_range, self.auto_clearance
            ),
        }
        s
    }
}

/// The jobs of the shipped `squad.toml`.
pub fn default_jobs() -> Vec<JobSettings> {
    let mut bacta = JobSettings::new("bacta", "Bacta", &["bacta"], JobKind::Once);
    bacta.result = JobResult::Heal;

    let mut demolition =
        JobSettings::new("demolition", "Demolition", &["demolition"], JobKind::Once);
    demolition.result = JobResult::Trigger;
    demolition.preferred = "Commando62Delta".into();
    demolition.fuse = 10.0;
    demolition.blast_damage = 256.0;
    demolition.blast_radius = 256.0;

    let mut breach = JobSettings::new("breach", "Breach door", &["breach"], JobKind::Once);
    breach.result = JobResult::Trigger;
    breach.group = true;
    breach.fuse = 2.5;
    breach.blast_damage = 200.0;
    breach.blast_radius = 256.0;

    let mut slice = JobSettings::new("slice", "Slice", &["hack", "slice"], JobKind::Once);
    slice.result = JobResult::Trigger;
    slice.preferred = "Commando40Delta".into();
    slice.work_time = 8.0;

    let mut sniper = JobSettings::new("sniper", "Sniper position", &["snipe"], JobKind::Post);
    sniper.weapon = "dc17m_sniper".into();
    sniper.sight_radius = 8000.0;
    sniper.preferred = "Commando07Delta".into();
    sniper.pullable = true;
    sniper.accuracy = 1.0;
    sniper.burst_count = 1;
    sniper.shot_interval = 0.1;
    sniper.burst_pause_min = 1.5;
    sniper.burst_pause_max = 2.5;
    sniper.auto = AutoUse::Far;
    sniper.auto_range = 1500.0;

    let mut grenade = JobSettings::new("grenade", "Grenade position", &["grenade"], JobKind::Post);
    grenade.pullable = true;
    grenade.throw_interval = 6.0;
    grenade.throw_range = 1500.0;
    grenade.throw_fuse = 2.0;
    grenade.throw_damage = 400.0;
    grenade.throw_radius = 512.0;
    grenade.target_radius = 512.0;
    grenade.auto = AutoUse::Group;
    grenade.auto_range = 600.0;
    grenade.auto_interval = 10.0;
    grenade.auto_min_enemies = 2;
    grenade.auto_clearance = 512.0;

    let mut anti_armour = JobSettings::new(
        "anti_armour",
        "Anti-armour position",
        &["antiarmor", "antiarmour"],
        JobKind::Post,
    );
    anti_armour.preferred = "Commando62Delta".into();
    anti_armour.pullable = true;
    anti_armour.weapon = "dc17m_antiarmor".into();
    anti_armour.burst_count = 1;
    anti_armour.shot_interval = 0.1;
    anti_armour.burst_pause_min = 2.0;
    anti_armour.burst_pause_max = 3.0;
    anti_armour.auto = AutoUse::Armoured;
    anti_armour.auto_range = 600.0;
    anti_armour.auto_clearance = 400.0;

    let mut turret = JobSettings::new("turret", "Man turret", &["turret"], JobKind::Post);
    turret.pullable = true;
    turret.mount = true;

    vec![
        bacta,
        demolition,
        breach,
        slice,
        sniper,
        grenade,
        anti_armour,
        turret,
    ]
}
