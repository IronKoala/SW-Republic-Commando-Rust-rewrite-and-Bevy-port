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
//!   (`throw_...`), until the player recalls the squad (F4).
//!
//! A map marker gets the first job whose `match` words appear in the
//! anchor's class name (or, failing that, the marker's own name).

/// One-off job or a post held until recalled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Once,
    Post,
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
    /// Words to find in the anchor's class name (lower case).
    pub matches: Vec<String>,
    pub kind: JobKind,
    /// Seconds of work at the spot (once).
    pub work_time: f32,
    pub result: JobResult,
    /// Send a member to every spot of the same anchor (door breach).
    pub group: bool,
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
}

impl JobSettings {
    fn new(key: &str, name: &str, matches: &[&str], kind: JobKind) -> Self {
        JobSettings {
            key: key.into(),
            name: name.into(),
            matches: matches.iter().map(|s| s.to_string()).collect(),
            kind,
            work_time: 3.0,
            result: JobResult::Nothing,
            group: false,
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
    demolition.fuse = 5.0;
    demolition.blast_damage = 500.0;
    demolition.blast_radius = 512.0;

    let mut breach = JobSettings::new("breach", "Breach door", &["breach"], JobKind::Once);
    breach.result = JobResult::Trigger;
    breach.group = true;
    breach.fuse = 3.0;
    breach.blast_damage = 200.0;
    breach.blast_radius = 256.0;

    let mut slice = JobSettings::new(
        "slice",
        "Slice",
        &["slice", "hack", "terminal", "console"],
        JobKind::Once,
    );
    slice.result = JobResult::Trigger;
    slice.work_time = 8.0;

    let mut sniper = JobSettings::new("sniper", "Sniper position", &["snipe"], JobKind::Post);
    sniper.weapon = "dc17m_sniper".into();
    sniper.sight_radius = 8000.0;
    sniper.accuracy = 0.97;
    sniper.burst_count = 1;
    sniper.shot_interval = 0.1;
    sniper.burst_pause_min = 1.5;
    sniper.burst_pause_max = 2.5;

    let mut grenade = JobSettings::new("grenade", "Grenade position", &["grenade"], JobKind::Post);
    grenade.throw_interval = 6.0;
    grenade.throw_range = 1500.0;
    grenade.throw_fuse = 2.0;
    grenade.throw_damage = 300.0;
    grenade.throw_radius = 384.0;

    let mut anti_armour = JobSettings::new(
        "anti_armour",
        "Anti-armour position",
        &["antiarmor", "antiarmour"],
        JobKind::Post,
    );
    anti_armour.weapon = "dc17m_antiarmor".into();
    anti_armour.burst_count = 1;
    anti_armour.shot_interval = 0.1;
    anti_armour.burst_pause_min = 2.0;
    anti_armour.burst_pause_max = 3.0;

    vec![
        bacta,
        demolition,
        breach,
        slice,
        sniper,
        grenade,
        anti_armour,
    ]
}
