# What already exists for Republic Commando

Researched 2026-10-29. These are references only. Nothing from them is copied into
this repository. When code here follows something we learned from one of them, the
code comment names the source.

## The game itself

- Unreal Engine 2, engine version 2226, build 139 for the Steam, GOG and retail PC
  versions (all the same build). Source: [SWRC Wiki, Game Versions](https://wiki.swrc-modding.net/index.php?title=Game_Versions).
- Files live under `GameData\`: `Maps` (`.ctm` maps), `Textures` (`.utx`),
  `Sounds` (`.uax`), `StaticMeshes` (`.usx`), `Animations` (`.ukx`), `System`
  (`.u` script packages, `.ini` settings, the `.exe`), `Music` (`.ogg`),
  `Movies`, `KarmaData` (physics). Source: [SWRC Wiki, Directory Layout](https://wiki.swrc-modding.net/index.php?title=Directory_Layout).
- Every `.ctm .utx .uax .usx .ukx .u` file is an "Unreal package": one container
  format with a header, a name table, an import table and an export table.

## File format documentation

| Source | What it gives us | License |
|---|---|---|
| [Unreal Wiki: Package File Format](https://unrealarchive.org/wikis/unreal-wiki/Legacy:Package_File_Format.html) and [Data Details](https://unrealarchive.org/wikis/unreal-wiki/Legacy:Package_File_Format/Data_Details.html) | The generic UE1/UE2 package layout: header, "compact index" numbers, name/import/export tables | Wiki text, used as description only |
| [UE Viewer / UModel](https://github.com/gildor2/UEViewer) by Konstantin Nosov (Gildor) | Reads Republic Commando packages. Has the Republic Commando quirks: package versions 133-148 and 154-159 with licensee version 1, a changed export-table layout from version 151, and mesh/texture differences | MIT |
| [SWRC Wiki](https://wiki.swrc-modding.net) | Versions, folder layout, ini files, class reference, UnrealScript reference | Community wiki |

## Decompilation and reverse-engineering projects

| Project | What it is | License |
|---|---|---|
| [SWRC-Modding/CT](https://github.com/SWRC-Modding/CT) (mirror: [doublej472/CT](https://github.com/doublej472/CT)) | C++ headers for the game's engine, rebuilt from Unreal Tournament 432 public sources plus reverse engineering of the game's DLLs. Includes an OpenGL renderer, a custom launcher and the "Republic Commando Fix". The wiki says it also holds decompiled pseudo-C. Not a full decompilation: the README says parts are missing | **No license stated.** Treat as read-only reference; never copy |
| The game's own UnrealScript | The `.u` packages in `System` contain the game's script source text for most classes. UCC or UModel can export it to a gitignored folder | Game content, never committed |

No complete decompilation of the game exists that we found. No existing Rust
reader for this game was found.

## Reference projects for structure (picked by the project owner)

| Project | What we take from it | License |
|---|---|---|
| [vladtrc/iw4L](https://github.com/vladtrc/iw4L) | Many small crates, one per format or system. Points at the user's own install, ships no assets | Apache-2.0 |
| [muffinmxn/gang-beasts-rust](https://github.com/muffinmxn/gang-beasts-rust) | Game folder set by an environment variable, clear "no game content" statement | MIT |
| [samwhosung/benilla](https://github.com/samwhosung/benilla) | Format readers are plain Rust with no Bevy in them, so they can be tested alone | MIT OR Apache-2.0 |

## Community

- SWRC Modding Discord, linked from the wiki.
- [Gildor's forums](https://www.gildor.org/smf/) for UModel support. An old thread
  reported UModel failing on `vehicles.ukx`; newer UModel versions list the game
  as supported.

## Player movement

Found on 2026-10-29 by reading the installed game, read only. Nothing below is
copied from the game: it's the numbers, and the rules described in our own
words.

**Where the numbers live.** `System/user.ini` names the player's class:
`[DefaultPlayer] Class=CTCharacters.PlayerCommando`. Its parent classes are
CloneCommando, Republic, CTPawn and Engine's Pawn. Each class's default values
("defaultproperties") are stored at the end of the class's data in its script
package (`ctcharacters.u`, `ctgame.u`, `engine.u`) as the same tagged property
list textures use. We found the start of that list by trying every position
near the end of the class data and keeping the one whose list reads cleanly to
the exact end, using only names of real properties. A few classes still show
one or two junk entries at the front with this method; the movement values
were all clean. Physics volume values come from `PhysicsVolume` and
`LevelInfo` in `engine.u`.

| Setting | Value | Set by |
|---|---|---|
| Run speed (GroundSpeed) | 450 uu/s | CloneCommando (Pawn: 600) |
| Acceleration (AccelRate) | 1024 uu/s² | CloneCommando (Pawn: 2048) |
| Jump speed (JumpZ) | 475 uu/s | CloneCommando |
| Air control | 0.35 | Pawn |
| Walk / crouch speed ratio | 0.5 / 0.5 | Pawn |
| Back / side speed ratio | 0.6 / 0.75 | Pawn / CloneCommando (Pawn side: 0.8) |
| Wounded speed ratio | 0.833, but PlayerCommando turns off HealthAffectsGameplay | Pawn |
| Collision radius / half-height | 40 / 84 uu | Pawn / CloneCommando (Pawn: 78) |
| Crouch radius / half-height | 40 / 56 uu | Pawn / CloneCommando |
| Eye height above centre | 60 uu | CloneCommando (Pawn: 72) |
| Fall speed that hurts (MaxFallSpeed) | 1200 uu/s | Pawn |
| DecelRate ("how fast we slow down") | 600 | Pawn. Used by the compiled engine; we don't know how. Not used yet |
| Gravity | -1100 uu/s² | PhysicsVolume, and LevelInfo DefaultGravity |
| Ground friction | 8 | PhysicsVolume |
| Terminal velocity | 12000 uu/s | PhysicsVolume |
| Step height (MAXSTEPHEIGHT) | 35 uu | constant in Engine script |
| Walkable floor (MINFLOORZ) | normal Z ≥ 0.7 (about 45.6°) | constant in Engine script |
| Field of view | 85° (horizontal) | PlayerController DefaultFOV |
| Bob | 0.008, but `bWeaponBob` is off (not set anywhere), so no view bob | Pawn |

Other characters for later: clone troopers 450, battle droids 375, Geonosian
warriors jump 2000.

**Controls** (`System/defaultctrl.ini`, `user.ini`): W/S/A/D move, Space jumps,
C crouches (held, "Button bDuck"), Shift is "Walking" (held makes you walk;
running is the default). Keys give axis values of 1200, which is more than
AccelRate, so a key always gives full acceleration. Mouse sensitivity 3.

**Rules from the game's script text** (the `.u` files carry the script source):

- A jump only starts on the ground, not crouched or wanting to crouch, and sets
  upward speed to JumpZ. A press in the air is lost.
- Crouch is only read while not falling. Crouched eye height is the smaller of
  80% of the crouch height and the crouch height minus 10 (44.8 for the
  player).
- Eye height eases toward its resting value at 10 per second, and while
  walking it also absorbs sudden height changes (steps, crouching), so the
  camera doesn't jump.
- Falling damage: below -0.5 × MaxFallSpeed the view shakes; below
  -MaxFallSpeed you take 100 × (speed - MaxFallSpeed) / MaxFallSpeed.
- The landing bob in the script can never trigger (it's computed from the
  downward speed, which is negative, and only applied if positive).

**What we don't know** (it's in the compiled engine, `engine.dll`, which we
don't read): the exact walking and falling physics step, how DecelRate and the
back/side ratios are applied, and how mouse movement turns into turning. For
those we follow how Unreal Engine 2 is publicly documented to work and mark
them as guesses in `docs/DESIGN.md`.

## The squad (read for `rc_squad`, 2026-10-29)

From the script packages (class defaults, read with the same "property list
at the end of the class" method as the weapon numbers) and `.ini` files:

- **Keys** (`System/defaultctrl.ini`, `user.ini`): F1 `SetStanceOffensive`
  (search and destroy), F2 `SetStanceDefensive` (form up), F3
  `SquadEngage` (secure position, at the spot aimed at), F4
  `CancelAllMarkers` (cancel/recall). `SetStanceHold` and `SquadRecall`
  exist but have no key.
- **Stances** (CTGame.u): `StanceFormUp`, `StanceFormUpTight`,
  `StanceFormUpLoose`, `StanceSecurePosition`, `StanceSearchAndDestroy`,
  `StanceEngageTarget`, `StanceHold`, `StancePreviousOrder`, all children of
  Engine's `SquadStance`. Each sets tether distances and priorities, a
  "prevent range", what it's tied to (`EStanceTetherObject`: leader,
  objective, self) and a HUD icon. Values are in `data/squad/squad.toml`
  and the table in `docs/DESIGN.md`. Squad defaults:
  `DefaultSearchAndDestroyRadius` 768. `CTBot.AUTORUNDISTANCE` 400.
  `CloneCommandoBot.DecisionInterval` 0.25.
- **The squad mates** (CTCharacters.u): `Commando07Delta` (HUD nickname SEV,
  slot 1, bot `Commando07Bot`, auto-snipes), `Commando40Delta` (FIXER, slot 2,
  auto-grenades), `Commando62Delta` (SCORCH, slot 3, auto anti-armour). From
  the Properties package (`GameData/Properties/properties.u`,
  `CommandoNNDeltaDefaults`): MaxHealth 400, MaxShields 50, shield recharge
  15 per second after 4 s, `bCannotDie` (they go down instead of dying),
  and each one's `EnhancedVisionColor` (Sev red, Fixer green, Scorch yellow).
- **Marked jobs**: `SquadMarkerSpot` actors, `MyAnchor` naming the object
  (bacta dispenser, demolition charge, door, terminal...). Marker classes
  live in CTMarkers.u (`MarkerSnipe`, `MarkerGrenade`, `MarkerDoorBreach`,
  ...).
- **What isn't readable**: the squad's decisions (`CTSquadNative.Update`,
  `SetStance`, `EngageLocation`...) are native, in `ctgame.dll`. We don't
  read compiled code, so how members pick spots and move is our own design.

## Squad markers (read for the context orders, 2026-10-29)

Read with `rc_probe --classes ctmarkers.u <Class>`, `rc_probe --class`, and
the new `marker_survey` example over all 50 campaign maps, read only.

- **Three kinds of actor make a marked job.** The *anchor* is the thing
  itself: `BactaDispenserGEO/RAS/YYY/TRN/CORE`, `BreachDoor...`,
  `HackDoor...`, `HackTerminal...`, `DemolitionCharge`, or a firing position
  `MarkerSnipeAnchor...`. The *order marker* is a `CTMarker` class from
  CTMarkers.u whose `Anchor` names it: `MarkerDoorBreach` (its
  `AlternateMarkers` names the door's `MarkerHackDoor`), `MarkerHackDoor`,
  `MarkerHackTerminal`, `MarkerDemolition`, `MarkerSnipe`, `MarkerGrenade`
  (with a `WeaponTarget` area, sometimes an EMP `GrenadeClass`),
  `MarkerAntiArmor`, and three we don't do yet: `MarkerDisarmTrap`,
  `MarkerSetProximityMine`, `MarkerTurret`. Bacta dispensers have no order
  marker. *`SquadMarkerSpot`s* (`MyAnchor` = the anchor) are where squad
  mates stand; a door breach has three (CTMarker `Participants`: doorman,
  grenadier, cover), most firing positions have none.
- **In the 50 campaign maps:** 240 bacta spots, 159 breach spots (54
  doors), 101 slice, 82 demolition, 179 sniper, 30 grenade and 46
  anti-armour markers; 157 we don't do yet (100 mines, 34 traps, 23 turret
  markers and a few turrets).
- **Numbers:** `DemolitionCharge` `ChargeExplode` Damage 256, DamageRadius
  256, `ChargePlaced` Duration 10. `BreachDoor` `DoorDamagedState` Damage
  200, radius 256, `DoorChargeLockedState` Duration 2.5. Thermal detonator:
  `ThermalDetonatorProj` FuseTime 2, `CTProjExplosive` Damage 400,
  DamageRadius 512, speed 2200. `MarkerWeapon` WeaponTargetRadius 512;
  `CTMarker` TargetingDistance 3000. `MarkerSnipe` Accuracy 1.0.
  `BactaDispenser` BactaRate 4. Which Duration is the fuse is our reading.
- **Who goes** (`PreferredPawnClass`): demolition and anti-armour Scorch
  (`Commando62Delta`), slicing Fixer (`Commando40Delta`), sniping Sev
  (`Commando07Delta`).
- **Not readable:** how long each job takes, how the grenadier picks a
  target, and what a breach does after the blast (the marker names
  `TossGrenadeEvent` and a thermal detonator, so the grenadier throws one
  in) are compiled code.

## Battle droids

Found on 2026-10-29 by reading the installed game's script packages
(`ctcharacters.u`, `ctgame.u`, `engine.u`, `ctinventory.u`), read only, with
`rc_package::class_library` (class defaults merged with their parents'). The
numbers are in `data/enemies/battle_droid.toml`, each marked "game" or
"guess"; the rules in `docs/DESIGN.md`, "Enemies", are our own.

geo_01a's spawners make `BattleDroidTM` (the Geonosis trench droid; parents
BattleDroid, Separatist, CTPawn, Pawn). Its controller is `BattleDroidBot`
(parent CTBot), its attack state `StateBattleDroidAttack` (parent
StateAbstractAttack), its weapon `BattleDroidBlasterTM`.

| Setting | Value | Set by |
|---|---|---|
| Health | 100 | BattleDroidTM MaxHealth |
| Accuracy | 0.4 (0 wild, 1 perfect) | BattleDroidTM |
| Sight radius | 5000 uu | BattleDroidTM |
| Run speed | 375 uu/s, walk 0.52 of that | BattleDroid GroundSpeed, WalkSpeedRatio |
| Acceleration | 1000 | BattleDroid AccelRate |
| Collision radius / half-height | 35 / 80 uu (crouched 35 / 50) | BattleDroid |
| Eye height above centre | 60 uu | BattleDroid BaseEyeHeight |
| Can shoot off its facing by | 60° | BattleDroid MaxAimYaw |
| View cone | 0.2 (about 78° each side) | Pawn PeripheralVision |
| Notices anyway within | 200 uu | Pawn AutoDetectRadius |
| Turn rate | 20000 per second (110°/s) | Pawn RotationRate |
| Forgets the enemy after | 15 s | CTBot EnemyMemoryTime |
| Thinks every | 0.5 s | CTBot DecisionInterval |
| Looks for new cover after | 15 s | CTBot CoverBoredomTime |
| Burst | 3 shots, 0.1 s apart | BattleDroidBot BurstCount; BattleDroidBlasterTM ShotDelay |
| Likes to stay between | 800 and 1200 uu | StateAbstractAttack ReadyDistance, SafeDistance |
| Gives up a move after | 10 s | StateBattleDroidAttack MaxManuverTime |
| Weapon reach | 7000 uu | Weapon ValidRange |

Not stored anywhere we can read (so guessed, and marked so in the file):
the delay before the first shot (0.5 s), how far a shot can miss at
accuracy 0 (10°), the rest between bursts (1-2 s), how far it advances at a
time (`MoveIntervalDist` is declared but has no default), how far it hears
shots (2500 uu) and how far it goes for cover (1500 uu).

**Spawners.** `PawnFactory` (Engine) has `Prototype`, `Capacity`,
`MaxAlive`, `bGenerateForever`, `SpawnMode` (0 cyclic, 1 random, 2 at the
factory, 3 near the player), `SpawnInterval` and the start state given to
what it makes (6 = stationary attack). `SpawnPoint`s with the factory's
`Tag` are its spots. A `Trigger` whose `Event` is the factory's tag switches
it on when touched. geo_01a has 58 factories, 36 making battle droids; 9 of
those have such triggers and the rest are started by level scripts.

**Path network.** Each `NavigationPoint` lists its outgoing `ReachSpec`s
(start, end, `Distance`, `CollisionRadius`/`Height`, `reachFlags`). Flags:
walk 1, fly 2, jump 8, door 16, forced 256 (as in Unreal Engine 2), and
4096, seen on 7-9 links per map, Republic Commando's own (meaning unknown;
treated as walkable). `BlockedPath` has `bBlocked` true by default.

**The droid's look.** `BattleDroid` in `GameData/Animations/droid.ukx`: 32
bones, 848 points, 1,403 triangles, animation set `BattleDroidSet`
(ActionBreathe, WalkForward, RunForward, WalkBackward, StepLeft/Right, Fire,
CrouchBreathe, CrouchFire, Death and more, all 30 frames per second). Skin:
`DroidTextures.BattleDroid_Shader`, whose picture `BattleDroid` is 512 x 512
and averages R 79, G 74, B 70.

## Weapons and damage

Found on 2026-10-29 by reading the installed game's script packages, read
only, with `rc_probe --class NAME` (see "Player movement" for how class
default values are found). Only numbers are taken, and the rules are
described in our own words. The values are in `data/weapons/*.toml` and
`data/combat/player.toml`, each line naming the property it came from.

**Where the numbers really live.** Each class's own defaults in
`ctinventory.u`, `ctcharacters.u` and so on are only part of the story. The
game also has `GameData/Properties/properties.u`, with a "...Defaults" class
for most weapons, ammo, bolts and characters (`DC17mBlasterDefaults`,
`PlayerCommandoDefaults`, ...). Its values replace the base class's: some
values, like `MaxHealth`, exist only there, and the engine script sets health
to `MaxHealth` when a character appears. So every number below is the base
class's value with its "...Defaults" class on top.

| Weapon | Fire | Ammo | Bolt | Damage | Sound |
|---|---|---|---|---|---|
| DC-17m rifle (`DC17mBlaster`) | automatic, 0.1 s between shots, aim error 15 | magazine 60, carries 300, starts with 120; reload 1.5 s (the first-person animation: 45 frames at 30 fps) | starts at rest, speeds up at 60000 uu/s² to 10000 uu/s | 20 | `wep_GEN_dc17Fire_02` |
| DC-15s pistol (`DC15s`) | one shot per press, 0.2 s between shots, aim error 10 | 5 shots of charge, regains 1.5 a second, never reloads | 40000 uu/s² to 8000 uu/s | 55 | `wep_GEN_pistolFire_01` |
| E-5 droid blaster (`BattleDroidBlaster`) | 0.55 s between shots, aim error 0 | | to 5000 uu/s | 24 | `wep_GEN_sbdGun_01` |
| E-5, training mode (`BattleDroidBlasterTM`) | 0.1 s between shots | | to 5000 uu/s | 2 | |

| Character | Health | Shield |
|---|---|---|
| Player (`PlayerCommando`) | 200 | 100, refills at 20 a second, 3 s after the last hit |
| Battle droid (`BattleDroid`, `BattleDroidTM`) | 100 | none |

Difficulty (`System/user.ini`): damage to the player is multiplied by 0.25 on
Easy, 1 on Medium and 1.75 on Hard.

**Rules from the script text:**

- Fire modes: 0 is one shot per press, 1 is burst, 2 is automatic.
- Laser bolts start with no speed and accelerate along their direction up to
  their top speed (`Projectile`, when `bLaserBolt` is set).
- Aim error: the point the shot would hit is moved by up to `AimError` uu for
  every 1000 uu of distance, along the world's sideways and up axes (not the
  view's).
- The player's shots leave the muzzle (the weapon's `FPFireOffset` from the
  eye) and fly to the point under the crosshair. If that point is behind or
  far to the side of the muzzle, the shot just goes straight ahead.
- When something is between the eye and the muzzle (standing against a wall
  or a droid), the shot starts at the eye instead, so it still hits.
- Damage goes to the shield first, then health. Any hit on the shield
  restarts the wait before it refills.
- Recharging weapons (`CTAmmoRecharging`) build up charge at their
  `RechargeRate` and need a full 1 to fire.
- Characters are upright cylinders for collision and hits (40 uu radius,
  84 uu half-height for a commando).

**What we don't know yet:** how the view kick (`WeaponKick`,
`RecoilConstant`, `RecoilDampening`) settles back (it is in the compiled
engine), the burst mode's details and hit-location multipliers. Damage-type
multipliers, grenades and melee are in the next section.

## Grenades, attachments, melee, damage tables and more enemies

Read on 2026-10-29 with `rc_probe --class NAME` from the user's install
(read only; each class with its "...Defaults" class in
`Properties/properties.u` on top, as in "Weapons and damage"). The values
are in the data files, each marked `game:` with the property it came from.

**Grenades** (`data/grenades/*.toml`). All four hand grenades share
`GrenadeProj`: `Speed` 2200, `TossZ` 200, `BounceDampening` 0.3 (0.1 off a
character), `RestSpeed` 20, thrown from `FPGrenadeOffset` (57.4, -30,
-10.56); each carries `MaxAmmo` 5 and starts with `InitAmmoAmount` 2.
Gravity is 1100 (`LevelInfo`/`PhysicsVolume`). The explosive rules
(`CTProjExplosive`), in our words: the fuse runs `FuseTime` from the
throw; touching anything shortens what's left to `PostImpactFuseTime`;
`bActorDetonate`/`bWorldDetonate` make it go off on touch;
`bStickToWorld`/`bStickToActors` make it stick; a stuck proximity mine
checks every `ElapseTime` for a character within `ExplosionProximity`.
A character it goes off on takes the full `Damage`; everyone else in
`DamageRadius` takes the engine's `HurtRadius` falloff,
`1 - max(0, distance - their radius) / DamageRadius`.

| | Fuse | After a touch | Damage | Radius | Damage type | Status effect |
|---|---|---|---|---|---|---|
| Thermal | 12 s | 1 s | 400 | 600 | CTDamageExplosionThermalDet | burn (`StatusEffect_Immolate`, CTDamageFire): up to 12 s at 15/s, within 200 uu (radius -400) |
| Sonic | 10 s | 10 s | 400 | 800 | CTDamageExplosionSonic | none; sticks, proximity 400 uu every 0.5 s |
| EC (EMP) | 12 s | 1 s | 0 | 1024 | CTDamageElectricityEMPGrenade | electrocution: up to 9 s at 50/s, half as long on clone commandos |
| Flash | 12 s | 1 s | 0 | 4096 | CTDamageExplosionFlashDet | stun (`StatusEffect_Stun`, CTDamageStun): up to 18 s; the player only when facing it |

A status effect lasts `StatusEffectMaxDuration x (1 - distance / (radius -
their radius))`, only on characters that can see the blast. **Our
reading:** burning doesn't stop a character acting; stun and electrocution
do (for computer-controlled characters).

**Damage tables** (`data/combat/damage_types.toml`). Every character has a
`MyDamageVariance` class: a list of damage types with a multiplier
(`IncludeTypes`), and types it ignores (`ExcludeTypes`, written as 0). A
hit uses the most specific entry up the damage type's parent chain
(`CTDamageEnergyDC17` is a kind of `CTDamageEnergy`), else x1. The file
holds all 53 damage types' parents and all 41 tables. Examples: battle
droids take x3 from melee, x0.75 from explosions, x0.4 from ballistic
(the sniper), and nothing from stun, fire or droid blasters; the player
(`DV_OrganicCTPlayerDamage`) takes x0.5 from energy bolts and x0.7 from
explosions; super battle droids x0.15 from energy, x1.25 from the
anti-armour round and the EMP.

**DC-17m attachments.** Sniper (`DC17mSniper`): instant hit, range 10000,
`Damage` 175 (`CTDamageBallisticDC17Sniper`), 5 a magazine, 20 carried,
`ShotDelay` 1.5, `ZoomFOVs` 30 and 10, `ZoomDamageMultiplier` 1.72,
`WeaponKick` 2500, `RecoilConstant` 15, `RecoilDampening` 0.2, fire sound
`wep_GEN_sniperFire_01`. Anti-armour (`DC17mAntiArmor`): 1 a magazine, 4
carried, `ShotDelay` 0.6, kick 4000, sound `wep_GEN_antiArmorGun_03`; its
round (`AntiArmorProj`) flies at 3000, falls, goes off on touching
anything, `Damage` 400 within 800 (`CTDamageExplosionDC17AA`).

**Melee.** Each weapon has `MeleeRange` and `MeleeDamage`
(`CTDamageMelee`): DC-17m 150 uu / 220, DC-15s 175 / 100, E-5 100 / 15,
SBD blaster 200 / 100, Trandoshan repeater 150 / 220.

**Enemies.** Super battle droid (`SuperBattleDroid`): `MaxHealth` 500,
`GroundSpeed` 100, collision 65 x 115, eye 60, `Accuracy` 0.9,
`MaxAimYaw` 90, wrist blaster `SBDBlaster` (automatic, 0.2 s, bursts of
3, bolt `SBDProj` 75 damage at 5000 uu/s). Geonosian warrior
(`GeonosianWarrior`): health 100, speed 450, collision 50 x 115,
`Accuracy` 0.5, `MaxAimYaw` 80, fights with a `ForcePike` up close.
Trandoshan mercenary (`TrandoshanMercenary`): health 220, speed 450,
collision 40 x 84, `Accuracy` 0.65, `MaxAimYaw` 70, `SMG` repeater
(instant hit, 10 damage `CTDamageBallisticSMG`, 0.1 s, bursts of 9, 40 a
magazine, `AimError` 25, range 5000), plus thermal detonators.

**Enemy grenades** (read 2026-10-30). Who carries one
(`RequiredEquipment`): `TrandoshanMercenary` and `TrandoshanSlaver`
(`ThermalDetonatorEnemy`); battle droids, Geonosian warriors and the
squad's enemies otherwise carry none. `ThermalDetonatorEnemyDefaults`
`InitAmmoAmount` 2. Its projectile (`ThermalDetonatorEnemyProjDefaults`):
`FuseTime` 12, `Damage` 200, `DamageRadius` 600, the burn status
(`StatusEffect_Immolate`, 12 s at 15/s, radius -400), and `MyDamageType`
left at `CTDamageExplosion` (the player's version is
`CTDamageExplosionThermalDet` with 400 damage). The grenade weapon's
`ValidRange` is 600 to 2500 uu. The brain's settings: `CTBot`
`GrenadeProbability` 0.3, `MinGrenadeTime` 10;
`TrandoshanMercenaryBotDefaults` 0.05 and 20; `TrandoshanSlaverBotDefaults`
0.1 and 20; the "Grenadier" bots 1.0 / 0.85 and 5;
`SuperBattleDroidBot` 0.5 and 20, but its grenade slot is
`SBDCrouchWeapon`, a straight-flying 800-damage rocket (`Speed` 5000,
goes off on touch), not a thrown grenade; `GeonosianDroneBot` 0.7 with
`DroneSpit`. The throw animation `ThrowGrenade` in the Trandoshan sets is 26
frames at 30 a second, with an `AnimNotify_ReleaseGrenade` partway.
How the brain uses these numbers is compiled code.

**Still not read or not known:**
- The attachments' and repeater's reload times (in their animations).
- Grenade explosion sounds (they're in the explosion effect classes) and
  the droids' voice lines (in the large sound packages).
- How the original lets the player hurt droids with a picked-up E-5: its
  `CTDamageEnergyDroids` is ignored by battle droids' table. Our
  `e5_blaster_pickup.toml` uses plain `CTDamageEnergy` instead.
- The Trandoshan's mesh `TrandoshanMerc` isn't in `trandoshan.ukx`; we
  use the `Trandoshan` mesh there for now.
- Which class names the maps' spawners actually use for these enemies
  (log line `spawners no enemy file claims`).
- The other Geonosians (`GeonosianDrone`, `GeonosianElite` with its beam).
