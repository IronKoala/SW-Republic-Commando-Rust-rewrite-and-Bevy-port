//! What a particle effect is made of, as plain values: an effect
//! (`Emitter` actor class in the game, such as `Explosion_Barrel_GEO` in
//! `Properties/cteffects.u`) holds a list of emitters, and each emitter
//! sends out particles with its own rules.
//!
//! The names follow the game's `ParticleEmitter` properties (in
//! brackets). Values not set by an effect come from the class defaults of
//! `ParticleEmitter`, `SpriteEmitter`, ... in `Engine.u` (read by
//! `read.rs`), and if those can't be read, from `Default` below.
//!
//! Distances are Unreal units (uu, about 1.1 cm), times are seconds,
//! colours 0-1.

/// A value picked at random between `min` and `max` (`Range`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Range {
    pub min: f32,
    pub max: f32,
}

impl Range {
    pub const fn new(min: f32, max: f32) -> Self {
        Range { min, max }
    }
    pub const fn fixed(v: f32) -> Self {
        Range { min: v, max: v }
    }
    /// The value at `t` (0-1) between min and max.
    pub fn at(&self, t: f32) -> f32 {
        self.min + (self.max - self.min) * t
    }
}

/// Three ranges, one per axis (`RangeVector`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RangeVector {
    pub x: Range,
    pub y: Range,
    pub z: Range,
}

impl RangeVector {
    pub const fn fixed(v: f32) -> Self {
        RangeVector {
            x: Range::fixed(v),
            y: Range::fixed(v),
            z: Range::fixed(v),
        }
    }
}

/// What kind of emitter it is. Only sprites are drawn so far; the rest
/// are read and logged.
#[derive(Debug, Clone, PartialEq)]
pub enum EmitterKind {
    /// Flat pictures that always face the camera (`SpriteEmitter`).
    Sprite,
    /// A 3D model per particle (`MeshEmitter`): the model's path
    /// (package, groups..., name).
    Mesh(Option<Vec<String>>),
    /// Short lines along the particle's flight (`SparkEmitter`).
    Spark,
    /// Beams and trails (`BeamEmitter`, `TrailEmitter`).
    Other(String),
}

/// How a particle's picture is mixed with what is behind it
/// (`EParticleDrawStyle`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawStyle {
    /// Solid (`PTDS_Regular`).
    Regular,
    /// See-through by the picture's alpha (`PTDS_AlphaBlend`).
    AlphaBlend,
    /// Multiplies what is behind (`PTDS_Modulated`).
    Modulated,
    /// Adds light (`PTDS_Translucent`).
    Translucent,
    /// See-through by alpha, then multiplies (`PTDS_AlphaModulate_...`).
    AlphaModulate,
    /// Darkens (`PTDS_Darken`).
    Darken,
    /// Adds light (`PTDS_Brighten`).
    Brighten,
}

impl DrawStyle {
    pub fn from_byte(b: u8) -> DrawStyle {
        match b {
            0 => DrawStyle::Regular,
            1 => DrawStyle::AlphaBlend,
            2 => DrawStyle::Modulated,
            4 => DrawStyle::AlphaModulate,
            5 => DrawStyle::Darken,
            6 => DrawStyle::Brighten,
            _ => DrawStyle::Translucent,
        }
    }
}

/// Which way a sprite particle faces (`SpriteEmitter.UseDirectionAs`,
/// `EParticleDirectionUsage` in `Engine.u`). How each is drawn was read
/// from the engine's own code (`USpriteEmitter::FillVertexBuffer` in
/// `Engine.dll`); see `crate::sim::sprite_axes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Facing {
    /// `PTDU_None` (the class default): flat to the camera.
    #[default]
    Camera,
    /// `PTDU_Up`: stretched along the way it moves, turned to the camera
    /// round that line (beams, sprays, lightning).
    Up,
    /// `PTDU_Forward`: flat across the way it moves.
    Forward,
    /// `PTDU_Normal`: flat across `ProjectionNormal`. Not drawn that way
    /// yet (drawn flat to the camera); no bacta effect uses it.
    Normal,
    /// `PTDU_UpAndNormal`. Not drawn that way yet (as `Normal`).
    UpAndNormal,
}

impl Facing {
    pub fn from_byte(b: u8) -> Self {
        match b {
            1 => Facing::Up,
            2 => Facing::Forward,
            3 => Facing::Normal,
            4 => Facing::UpAndNormal,
            _ => Facing::Camera,
        }
    }
}

/// Where new particles start around the emitter (`EParticleStartLocationShape`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartShape {
    Box,
    Sphere,
    Polar,
}

/// Whose axes an emitter's particles live in (`EParticleCoordinateSystem`,
/// whose three names are in `Engine.u`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CoordinateSystem {
    /// `PTCS_Independent` (the class default): the level's axes.
    #[default]
    Independent,
    /// `PTCS_Relative`: the effect's own, so where its particles start and
    /// which way they fly turn with the effect (see
    /// [`crate::EffectState::placed`]).
    Relative,
    /// `PTCS_Absolute`.
    Absolute,
}

impl CoordinateSystem {
    pub fn from_byte(b: u8) -> Self {
        match b {
            1 => CoordinateSystem::Relative,
            2 => CoordinateSystem::Absolute,
            _ => CoordinateSystem::Independent,
        }
    }
}

/// Which way new particles fly (`EParticleVelocityDirection`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VelocityFrom {
    /// Just `start_velocity`.
    None,
    /// Away from the effect's centre (`PTVD_StartPositionAndOwner`).
    StartPositionAndOwner,
    /// Towards the centre (`PTVD_OwnerAndStartPosition`).
    OwnerAndStartPosition,
    /// `start_velocity`, plus `radial` speed away from the centre
    /// (`PTVD_AddRadial`).
    AddRadial,
}

/// One emitter of an effect.
#[derive(Debug, Clone, PartialEq)]
pub struct EmitterDef {
    /// The emitter's object name (`SpriteEmitter12`).
    pub name: String,
    pub kind: EmitterKind,
    /// `Disabled`: never sends anything.
    pub disabled: bool,
    /// `MaxParticles`: how many can be alive at once.
    pub max_particles: u32,
    /// `RespawnDeadParticles`: keep sending new ones (a looping effect).
    pub respawn: bool,
    /// `AutomaticInitialSpawning`: with no rate set, send
    /// `max_particles / lifetime.max` a second.
    pub auto_rate: bool,
    /// `InitialParticlesPerSecond`: how fast the first `max_particles`
    /// come out (explosions use a big number: all at once).
    pub initial_rate: f32,
    /// `ParticlesPerSecond`.
    pub rate: f32,
    /// `InitialDelayRange`: the emitter starts this late.
    pub initial_delay: Range,
    /// `LifetimeRange`.
    pub lifetime: Range,
    /// `StartLocationOffset`, `StartLocationRange`, `StartLocationShape`,
    /// `SphereRadiusRange`.
    pub start_offset: [f32; 3],
    pub start_range: RangeVector,
    pub start_shape: StartShape,
    pub sphere_radius: Range,
    /// `StartVelocityRange`, `StartVelocityRadialRange`,
    /// `GetVelocityDirectionFrom`.
    pub start_velocity: RangeVector,
    pub radial_velocity: Range,
    pub velocity_from: VelocityFrom,
    /// `VelocityLossRange`: share of speed lost a second, per axis.
    pub velocity_loss: RangeVector,
    /// `Acceleration` (gravity on falling debris), uu/s².
    pub acceleration: [f32; 3],
    /// `StartSizeRange` (X for square particles, X wide and Y high
    /// otherwise), `UniformSize`. For a mesh emitter it scales the model.
    pub start_size: RangeVector,
    pub uniform_size: bool,
    /// `UseSizeScale`, `UseRegularSizeScale` and `SizeScale`: size over
    /// the particle's life, (relative time 0-1, scale).
    pub size_scale: Vec<(f32, f32)>,
    /// `UseColorScale` and `ColorScale`: colour over its life, (relative
    /// time, RGBA 0-1).
    pub colour_scale: Vec<(f32, [f32; 4])>,
    /// `ColorMultiplierRange`.
    pub colour_multiplier: RangeVector,
    /// `Opacity`.
    pub opacity: f32,
    /// `FadeOut` with `FadeOutStartTime`, `FadeIn` with `FadeInEndTime`
    /// (seconds into the particle's life).
    pub fade_out: Option<f32>,
    pub fade_in: Option<f32>,
    /// `Texture`: its path (package, groups..., name).
    pub texture: Option<Vec<String>>,
    pub draw_style: DrawStyle,
    /// `TextureUSubdivisions`, `TextureVSubdivisions`: a picture made of
    /// smaller frames.
    pub frames_u: u32,
    pub frames_v: u32,
    /// `UseRandomSubdivision`, `SubdivisionStart`, `SubdivisionEnd`.
    pub random_frame: bool,
    pub frame_start: u32,
    pub frame_end: u32,
    /// `SpinParticles`, `StartSpinRange` (X, in turns),
    /// `SpinsPerSecondRange` (X).
    pub spin: bool,
    pub start_spin: Range,
    pub spins_per_second: Range,
    /// `UseCollision`, `UseCollisionPlanes` or `UseMaxCollisions`: the
    /// particles bounce off the ground. **Our simplification:** the ground
    /// is a flat floor under the effect (`EffectState::floor`), not the
    /// level itself.
    pub collides: bool,
    /// `DampingFactorRange` (Z): how much of its speed a particle keeps
    /// when it bounces.
    pub damping: Range,
    /// `CoordinateSystem`.
    pub coordinate_system: CoordinateSystem,
    /// `UseMeshBlendMode` (mesh emitters; `MeshEmitter`'s default is on):
    /// the model is drawn the way its own materials say (solid), not with
    /// the emitter's `draw_style`.
    pub mesh_blend: bool,
    /// `UseDirectionAs` (sprite emitters).
    pub facing: Facing,
    /// `ZTest` (the class default is on): off draws the particles over
    /// everything, walls included (the bacta dispenser's hologram).
    pub z_test: bool,
    /// `BlendBetweenSubdivisions`: the engine mixes each frame into the
    /// next. **Not done yet**: we show whole frames (see
    /// `crate::sim::frame_at`).
    pub blend_frames: bool,
}

impl Default for EmitterDef {
    /// The engine's own defaults for a particle emitter, as far as we
    /// know them; `read.rs` overwrites them with `Engine.u`'s when it can.
    fn default() -> Self {
        EmitterDef {
            name: String::new(),
            kind: EmitterKind::Sprite,
            disabled: false,
            max_particles: 10,
            respawn: true,
            auto_rate: true,
            initial_rate: 0.0,
            rate: 0.0,
            initial_delay: Range::fixed(0.0),
            lifetime: Range::fixed(4.0),
            start_offset: [0.0; 3],
            start_range: RangeVector::fixed(0.0),
            start_shape: StartShape::Box,
            sphere_radius: Range::fixed(0.0),
            start_velocity: RangeVector::fixed(0.0),
            radial_velocity: Range::fixed(0.0),
            velocity_from: VelocityFrom::None,
            velocity_loss: RangeVector::fixed(0.0),
            acceleration: [0.0; 3],
            start_size: RangeVector::fixed(100.0),
            uniform_size: false,
            size_scale: Vec::new(),
            colour_scale: Vec::new(),
            colour_multiplier: RangeVector::fixed(1.0),
            opacity: 1.0,
            fade_out: None,
            fade_in: None,
            texture: None,
            draw_style: DrawStyle::Translucent,
            frames_u: 0,
            frames_v: 0,
            random_frame: false,
            frame_start: 0,
            frame_end: 0,
            spin: false,
            start_spin: Range::fixed(0.0),
            spins_per_second: Range::fixed(0.0),
            collides: false,
            damping: Range::fixed(1.0),
            coordinate_system: CoordinateSystem::Independent,
            mesh_blend: false,
            facing: Facing::Camera,
            z_test: true,
            blend_frames: false,
        }
    }
}

impl EmitterDef {
    /// How many frames its picture holds (1 if it isn't split).
    pub fn frames(&self) -> u32 {
        self.frames_u.max(1) * self.frames_v.max(1)
    }

    /// Whether it ever stops by itself.
    pub fn ends(&self) -> bool {
        self.disabled || !self.respawn
    }
}

/// A whole effect.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EffectDef {
    /// The class name (`Explosion_Barrel_GEO`).
    pub name: String,
    pub emitters: Vec<EmitterDef>,
    /// `SpawnSound`: the sound it makes as it appears: its path, package
    /// first (`["GEN_Sound", "Impacts_Explos", "exp_barrels"]`).
    pub sound: Option<Vec<String>>,
    /// `AmbientSound`: a sound that loops for as long as the effect is
    /// there (the bacta dispenser's zap hums with
    /// `GEO_Sound.GEO_03.ele_GEO_genEnrgy_lp_01`); path as `sound`.
    pub ambient_sound: Option<Vec<String>>,
    /// `LifeSpan`: the effect is removed after this long (0: when its
    /// emitters are done).
    pub life_span: f32,
    /// `bUnlit` switched off: its models are lit by the level's light
    /// where they are. The `Emitter` class default is unlit, and most
    /// effects keep it; the flying gunships and fighters
    /// (`cFX_Gunships`, `cFX_GEOfighters`) switch it off.
    pub lit: bool,
}
