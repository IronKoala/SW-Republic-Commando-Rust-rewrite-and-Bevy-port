# Star Wars: Republic Commando, rewritten in Rust

A from-scratch Rust rewrite of Star Wars: Republic Commando (2005), aiming to run on
the Bevy game engine. It reads the game's data from **your own installed copy** at
runtime. Single-player and offline only.

**This repository contains no game content.** No textures, maps, sounds, models,
scripts or decompiled code are included, and none ever will be. You need your own
copy of the game.

## What works

- **Reading game packages (tested).** `rc_probe` opens any of the game's package
  files (`.u .ctm .utx .uax .usx .ukx`) read-only and reads the header, the name
  table, the import table and the export table. Tested on Windows against a full
  Steam install: all 273 packages read and passed every consistency check, in
  under one second.

- **Reading textures (partly tested).** `rc_probe --textures` reads every `Texture`
  object: its settings (format, size) and the raw pixel data of each mipmap.
  Tested on all 57 texture packages from the install plus the script packages and
  three maps: 2,491 textures, 2,490 pass every check. The one exception is a
  texture whose smallest (1x1) mipmap is stored with no pixels, which looks like
  a quirk in the game's own file. Formats seen: P8, DXT1, DXT3, DXT5, RGBA8,
  G16, and the bump-map formats V8U8 (12) and L6V5U5 (14).

- **Turning textures into pictures (tested by numbers, not by eye).** The
  `rc_image` crate decodes P8 (with its palette), DXT1, DXT3, DXT5, RGBA8, G16
  and the two bump-map formats into normal colours, and `rc_probe --export-textures` saves them as PNG files
  in the gitignored `extracted/textures/` folder. Of 2,491 textures, 2,358 were
  decoded with no failures. For 2,003 of them we could compare against the
  game's own half-size copy, and 1,916 match within 10 out of 255.

- **Showing a texture in a window (tested on Windows).** `rc_viewer`
  opens an 800x800 Bevy window showing one texture from the game, by default
  `DC15Blaster` from `weapontextures.utx`. It checks itself: it captures the
  window and compares it pixel by pixel with the texture. In the cloud sandbox
  this passed 12 of 12 runs on three textures. On the user's Windows PC it
  passed first time (difference 3.77 of 255) and ran at a steady 144 frames
  per second.

- **Reading and playing sounds (tested on Windows).** `rc_probe --sounds` reads every sound in the game's 23 sound
  packages (`.uax`). Every sound inside them is a normal `.wav` file
  (uncompressed 16-bit): 7,974 sounds, about 4 hours 11 minutes in total, all
  decoded with no failures. `rc_probe --export-sounds` saves them as `.wav`
  files in the gitignored `extracted/sounds/` folder. `rc_soundtest` plays the
  DC-17 rifle firing over a looping battle ambience through Bevy's audio and
  checks that Bevy decodes every sample exactly as our own reader does. On the
  user's Windows PC it played and was heard, at exactly real-time speed.

- **Reading 3D models (tested on every static mesh).** `rc_probe --meshes`
  reads every static mesh (the fixed models used for weapons, props and level
  pieces, in `.usx` files): corner points, triangles and texture coordinates.
  All 2,447 static meshes in the install read and pass every check.

- **Showing a model in 3D (tested in the cloud sandbox, not yet on Windows).**
  `rc_viewer --mesh` shows the DC-15 blaster with its texture, turning slowly.
  It checks itself by drawing the model on its own and comparing it with the
  screen: outline match 0.996 and colour difference 1.04 of 255, PASS. It also
  works for the DC-17m, the DC-15s pistol and the thermal detonator.

- **Walking in first person (tested in the cloud sandbox, not yet on
  Windows).** `rc_walk` puts you in a test room with steps, stairs, ramps, a
  low ceiling and a tower, and moves you with the original game's numbers
  (run 450, walk and crouch half speed, jump 103 units high, 35-unit steps,
  gravity 1100). Controls are the game's: W A S D, mouse, Space, hold C to
  crouch, hold Shift to walk. The numbers are in
  `data/movement/player_commando.toml`, which mods can edit. `rc_walk
  --script` drives itself through 21 checks and passed all of them.
  `rc_walk --map geo_01a` puts you in the first Geonosis level instead, with
  collision against its walls, floors and props.

- **Delta Squad (tested in the cloud sandbox by numbers, not yet on Windows
  or with eyes).** Sev, Fixer and Scorch follow you and obey the original's
  orders: F1 search and destroy, F2 form up, F3 secure the spot you aim at,
  F4 cancel, and aiming at a squad marker (like a bacta dispenser) and
  pressing the squad order key sends one of them to do it. Distances are
  the game's own; every number is in `data/squad/squad.toml`. The
  windowless test run `rc_squad_sim` passes 33 checks on geo_01a. They
  have the original's health and shields, and at 0 health one goes down
  and stays put until revived. They are in the game program (`rc_game`):
  start a level and they stand with you.

- **Sev, Fixer and Scorch look like themselves (tested in the cloud
  sandbox and on Windows).** Each squad mate is drawn with the
  original commando model, their own armour (Sev's red, Fixer's green,
  Scorch's yellow) and the original's animations: standing, walking,
  running, backing up, side-stepping, firing, falling when knocked out, and
  kneeling to revive. Everything is read from the install when the level
  starts; which model and animations are used is in
  `data/squad/looks.toml`. A squad mate whose model can't be read is drawn
  as a coloured capsule (a placeholder) and the log says why.
  Squad mates and enemies blend smoothly from one animation into the next
  instead of snapping (checked from logged numbers with
  `rc_blend_check`, not looked at). They don't
  hold a weapon yet, and the helmet extras the original adds
  (`AvailableAccessories`) aren't drawn.

- **The squad fights and revives (tested in the cloud sandbox by numbers,
  not yet on Windows or with eyes).** Sev, Fixer and Scorch shoot enemies
  they can see with the DC-17m, in bursts, and hold fire when you or
  another squad mate is in the way. When one goes down, the nearest other
  runs over and revives them in 3 s; they revive you the same way, and you
  revive them by holding Use (E) next to them for 3 s. While you're down
  you can look around but not move. The rules are our own (the original's
  are compiled code we don't read) and every number is in the `[fight]`
  and `[revive]` parts of `data/squad/squad.toml`; the ones marked "guess"
  still have to be read from the game. Droids still only shoot at you, not
  at the squad.

- **Squad context orders (tested in the cloud sandbox by numbers, not yet
  on Windows, in a real level, or with eyes).** Each kind of squad marker
  does its own job: bacta heals the squad mate who uses it; demolition and
  door breach set a charge that blows after a few seconds and hurts
  anything near it (a breach first sends a squad mate to every spot at the
  door); slicing takes longer and makes no bang; sniper, grenade and
  anti-armour positions are held until you press F4, the sniper firing
  single long-range shots, the grenadier throwing a grenade every few
  seconds. Every job is a `[job.*]` part of `data/squad/squad.toml`, so
  mods can change them or add new ones. Doors don't open yet: the squad
  reports the job done, and the level scripts part will act on it. The
  markers are read the way the original's maps lay them out (checked on
  all 50 campaign maps), and blast sizes, fuses and who does which job
  are the game's; how long the work takes is our guess. Traps, mines and
  turrets aren't done yet. With "auto pull maneuvers" on (the default,
  as in the original), ordering a new job when everyone is busy takes
  the squad mate who has held a sniper, grenade or anti-armour position
  longest off it; switch it off in `settings.toml`
  (`auto_pull_maneuvers = false`) and the order is refused until you
  recall the squad with F4. Tested by numbers only.

- **Firing the DC-17m and the DC-15s pistol (tested in the cloud sandbox,
  not yet on Windows).** `rc_shoot` is a firing range: you stand in the test
  room holding the game's own DC-17m model, and shoot battle droids. The
  weapons use the original game's numbers: the rifle fires 10 bolts a second
  from a 60-shot magazine and reloads in 1.5 s, the pistol fires once per
  press and recharges itself, and a droid with 100 health goes down after 5
  rifle hits or 2 pistol hits. One droid fires back, and its hits take your
  shield before your health. The firing and reload sounds are the game's.
  Every weapon is a text file in `data/weapons`, which mods can edit.
  `rc_shoot --script` drives itself through 17 checks and passed all of them.

- **First-person arms and weapon animations (tested in the cloud sandbox,
  not yet on Windows or by playing).** The player holds the original
  first-person arms and gun for the DC-17m, its sniper and anti-armour
  attachments, the DC-15s pistol and the picked-up Trandoshan repeater,
  read from the game's `hudarms.ukx` at run time. They breathe when you
  stand still, sway when you walk or run, kick on each shot, play the full
  reload, holster the old weapon and bring up the new one when you switch
  (with the attachment swap between DC-17m attachments), and play the
  grenade throw and melee hit. They're drawn by their own camera, so the gun
  never pokes into walls. The sniper's arms hide while zoomed, as in the
  game. The little screen on the DC-17m and its sniper shows the ammo you
  carry (three digits, like "120") and a bar that empties with the
  magazine and fills during a reload (tested in the cloud with
  `rc_shoot --script`; not yet seen on Windows).

- **Loading a whole level (tested on Windows).**
  `rc_level_viewer` loads a map with its walls, floors and every placed prop,
  textured, and lets you fly around it. Tested on `geo_01a` (the first Geonosis
  level): 141,477 triangles, all 56 materials textured, loaded in a quarter of a
  second and drawn at a steady 144 frames per second on the user's PC. It checks itself by
  numbers: props sit under the AI's path points where they should, and the
  window is covered by the level. The level's geometry and actors were read
  from 5 maps with no errors.

- **Characters with animation (tested on Windows).**
  `rc_probe --characters` reads every character body ("skeletal mesh") and
  animation set in the game: all 131 meshes and 165 animation sets (1,414
  animations) in the 20 animation packages read with no failures.
  `rc_character` opens a 3D window with a textured clone trooper walking on
  the spot. It checks itself by capturing the window and comparing the
  character's outline with the one it expects (score 0.99 of 1.0; 0.95
  passes) and checking its colours match the skin. Battle droids work too.
  On the user's Windows PC the clone scored 0.993 with the right colours.

- **Bump maps (tested by numbers, not by eye).** All 133 bump maps (formats
  12 and 14) now decode, as the usual light-blue "normal map" pictures. The
  two layouts were found by measurement, since no documentation names them
  (see `docs/DESIGN.md`, "Bump maps"). Their own half-size copies match with
  a median difference of 1.8 of 255.

- **Reading a map's lights (tested in the cloud on 4 maps, not on Windows).**
  `rc_package::light` turns light and zone actors into position, colour,
  brightness and reach. Read 600 lights and zones from `geo_01a`, `pro`,
  `ras_01a` and `yyy_01b` with nothing left over.

- **Levels lit with the game's own baked light (tested on Windows with
  `geo_01a`, and in the cloud on 4 maps).** The level viewer now draws walls with the
  lightmaps the game saved inside each map, and props with the light saved
  at each of their corners, plus the few lights that change during play.
  The lightmaps read on all 4 maps with every check passing (every wall
  corner lands inside its own patch of light). On the user's PC it still
  draws at a steady 144 frames per second. `lighting.cfg` lets anyone
  change the brightness without touching code (see `docs/DESIGN.md`,
  "Lighting in Bevy").

- **Every level loads (tested in the cloud on all 79 maps, not on Windows).**
  `rc_level_check` loads every map in the game without a window and logs
  each one: `79 of 79 maps loaded, 0 failed`. Lightmaps now read on 78 of
  them (12 failed before a fix). Levels now have their sky (67 maps have
  one), invisible walls (blocking volumes) are solid when you walk, and
  about 3,460 props and double doors that take their model from their
  type are now drawn and solid.

- **Doors and lifts move.**
  Doors, lifts and hatches open, close and travel with the timings, keys
  and triggers stored in each map, and their collision moves with them, so
  you can ride a lift in `rc_walk`. `rc_mover_check` runs every one in every
  map and logs the numbers: all 863 that could be timed opened and closed
  on time, and all 33 rideable lifts carried the player. They also run
  in the game itself (`rc_game`, tested in the cloud sandbox on geo_01a):
  doors are solid where they stand and move when set off. Doors that the
  game's scripts open (472 of them, plus the `Locked` ones) stay shut for
  now. Our own settings are in `data/movers/movers.toml`.

- **Levels run their own scripts (tested in the cloud sandbox, not on
  Windows).** The game reads each map's own scripting (79 of 79 maps, 2,842
  scripts, 19,041 actions, nothing unreadable) and runs it: scripts wait
  for events and timers, touch triggers, counters and conditions send
  events on, doors open by event, checkpoints ask for a save, and
  barricades come down. On geo_01a the landing zone now plays like the
  original: after the intro you are hit by a shell, the advisor tells you
  to heal, the bacta dispenser switches on, and holding Use at it heals you
  and blows the barricade so you can reach the fight. `rc_game --script`
  checks that whole chain (37 of 37 checks). `rc_script_check` runs every
  map's scripts without a window.
  Enemy spawners now follow the scripts too (tested in the cloud sandbox
  with the real files, not yet on Windows or by playing): the level's
  events switch them on and off, scripts can stop or refill them, and
  each droid tells the scripts when it dies, so "kill these droids, then
  the next thing happens" works. They follow the original's spawning
  rules (where, how many, how often). `rc_game --script` checks that on
  geo_01a's small bunker (52 of 52 checks).
  Levels now talk (tested in the cloud sandbox on a silent sound card, not
  yet listened to or run on Windows): the lines of dialogue and sounds the
  scripts play (squad and advisor radio lines, explosions on cue) come
  from the game's own sound files, and a script waiting for a line waits
  exactly as long as the line lasts. Every one of the 1,629 sounds the
  campaign maps' scripts name is found. `rc_game --script` checks that
  geo_01a's intro plays its lines (57 of 57 checks with level changes).
  Finishing a level now takes you to the next one (tested in the cloud
  sandbox with the real files, not yet on Windows or by playing through):
  walking into the exit arrows, or a script's "change level", loads the
  next map in the campaign, and you keep your health, shield, weapons and
  ammo. You arrive where the original puts you, and the new map's start is
  saved as your checkpoint. Following the exits from the prologue goes
  through all 56 campaign maps to the epilogue. `rc_game --script` walks
  into geo_01a's exit and checks geo_01b (56 of 56 checks).
  Scripts now direct the enemies too (tested in the cloud sandbox with the
  real files, not yet on Windows or by playing): they walk where the
  level sends them, turn to you, play their scripted animations, wait for
  you, appear, vanish, and can be made unkillable or hurt. Geonosians leap
  up to ledges they can't walk to (a stand-in for flying). Scripts can also
  set Delta Squad's stance. On geo_01a the lone Geonosian in the trench
  leaps up, turns to you, plays his "seen an enemy" animation and runs off.
  Squad mates' own scripted moves aren't done yet. `rc_game --script`
  checks the Geonosian's whole scene (60 of 60 checks).

- **The game program, with menus (tested in the cloud sandbox, not yet on
  Windows).** `rc_game` is the one program the whole game is built into. It
  opens on a main menu in the visor's amber, lists the campaigns and missions
  with the names from the game's own files (`Extreme Prejudice`, `Into The
  Hive`, ...), shows a loading screen with the mission text, and drops you
  into the level with an on-screen display (shield, health, crosshair, ammo,
  squad, messages). Escape pauses; the pause menu resumes, opens options or
  quits to the main menu. It plays the game's own music (the main menu
  theme, then each campaign's calm tracks). It reads a settings file
  (`settings.toml`: game folder, mods, screen, volumes, mouse) and a controls
  file (`data/input/bindings.toml`: the original PC keys, plus gamepad and
  touch). `rc_game --script` drives itself through the menus and a level and
  passed 20 of 20 checks, including three screenshots checked by colour.
  No weapons, enemies or squad until those parts plug in.

- **Levels in the game, with sky, invisible walls and doors (tested in the
  cloud sandbox on 7 maps, not yet on Windows).** The level part
  (`rc_level_plugin`) loads the mission you pick in the menu: the map drawn
  and lit, its sky, and solid walls, props, double doors and invisible walls
  (blocking volumes). You start at the map's own player start. Checked on
  geo_01a, pro, ras_01a, yyy_01b, geo_04c, ras_04c and ctf_siege.

- **Every level loads (tested in the cloud on all 79 maps, not on Windows).**
  `rc_level_check` loads every map in the game without a window and logs
  each one: `79 of 79 maps loaded, 0 failed`. Lightmaps now read on 78 of
  them (12 failed before a fix). Levels now have their sky (67 maps have
  one), invisible walls (blocking volumes) are solid when you walk, and
  about 3,460 props and double doors that take their model from their
  type are now drawn and solid.

  **The main menu is the original's (tested in the cloud sandbox, not yet
  on Windows).** `rc_game` opens on the game's own main menu, built at run
  time from your install: the planets in their rings, the title, the eight
  buttons in their display box, the original fonts, colours and focus and
  select sounds. QUIT asks the original's "are you sure?" question. Keyboard,
  gamepad and mouse move the focus. A screenshot of it matches a picture
  painted straight from the original's textures (mean difference 3.4 of
  255). LOAD GAME, PROFILES and EXTRAS are greyed out until they're built;
  MULTIPLAYER is greyed out for good (offline only).

  **The other menus and the on-screen display are still placeholders.**
  The campaign list, options, pause, loading screen and HUD are our own
  amber stand-ins, and the amber main menu comes back if the install's menu
  files can't be read. The plan is to replace them with the original's in
  turn; see `docs/ORIGINAL_MENUS.md`. **The pause menu is now the
  original's** (tested in the cloud sandbox with the install, not yet on
  Windows): its eleven buttons, with saving, loading and hints greyed out
  until built. The campaign page keeps the placeholder until its planet
  buttons and level list can be drawn; `logs/rc_game.log` says why
  ("original menus: page campaigns kept the placeholder: ..."). Which
  original menu draws which page is in `data/ui/original_menus.toml`. The
  crosshair is the original's reticle picture (`data/ui/hud.toml`); its
  size is our guess.

  Any screen shape works: the view is "Hor+" (wider screens see more at the
  sides, the view up and down stays the original's; upright phones see more
  above and below), until the screen is twice as long as it is short,
  so a maximized window or an ultrawide doesn't look fish-eyed
  (`widest_shape` in `settings.toml`; the field of view setting still
  works on every screen). The HUD stays in a centred 16:9 area on ultrawides
  (`hud_max_aspect`). The angles are checked by a test at 14 shapes, from
  4:3 to 32:9 and from upright and flip phones to foldables; in the game
  itself the script run was checked at 4:3, 16:9, 21:9, 32:9 and an upright
  phone shape before this limit was added (not re-run since). Keyboard and
  mouse and a gamepad work at the same time.

- **Controllers and gyro aiming (tested with pretend controllers only, no
  real hardware yet).** SDL3 reads Xbox, PlayStation, Switch Pro, Joy-Con,
  GameCube and Steam Deck controllers and says which kind each is. Button
  names for prompts follow the device you used last (`InputPrompts`,
  names in `data/input/button_names.toml`). Separate dead zones and curves
  for each stick, a trigger threshold, and gyro aiming with sensitivity,
  drift calibration, a ratchet button and on/off (`[gyro]` in
  `settings.toml`). Changing a control in the game is saved to
  `my_bindings.toml` (the screen to do it from comes with the original
  options menus). `rc_padtest` checks a real controller. See
  `docs/DISPLAY_AND_INPUT.md`.
- **Panini view for ultrawide screens (tested in a check program, not yet
  in a level).** `video.panini` in `settings.toml` bends the view so the
  sides of a 21:9 or 32:9 screen aren't stretched, keeping the middle's
  size.

- **Battle droids that fight back (tested in the cloud sandbox on
  geo_01a, not yet on Windows).** Droids appear from the map's own
  spawners when the level's scripts or triggers start them, walk the level's own path
  network, spot you (sight range, view cone, line of sight), shoot in
  bursts of three with the game's accuracy, take cover when shot in the
  open, and die to your shots through the weapons part's health and
  damage. They are the game's own animated droid model with its skin.
  Their numbers come from the game's script files and are in
  `data/enemies/battle_droid.toml`, where anyone can change them.
  `rc_droidview --check` watched a fight on geo_01a: 5 droids, 180 shots,
  2 droids destroyed by a stand-in player, no droid shooting another, and
  the droids on screen had the skin's colours (all 12 checks passed).

- **Grenades, the DC-17m's attachments, melee, picked-up weapons and
  more enemy types, with the original's numbers (tested in the cloud
  sandbox with automatic tests and the game's script files, not yet played
  in the real game).** You can throw thermal, sonic, EMP and flash
  grenades: the thermal bounces and goes off a second after it lands (and
  sets people close to it alight), the sonic sticks and waits for someone to
  come near, the EMP electrocutes droids, and the flash stuns anyone looking
  at it. Switch to the DC-17m's sniper (two zoom steps; zoomed shots hit
  1.72 times harder) and anti-armour attachments, hit with melee, and pick
  up a fallen enemy's weapon. Every character takes more or less damage by
  type, from the game's own damage tables (battle droids shrug off sniper
  rounds but fall to melee). Super battle droids, Geonosian warriors and
  Trandoshan mercenaries each have their own settings file in
  `data/enemies` with the game's health, speed, size and aim.
- **Enemies throw grenades (tested in the cloud sandbox with automatic
  tests, not yet in the real game).** Trandoshan mercenaries carry two of
  the enemies' thermal detonators, as in the original, and now and then lob
  one at you when you are 600 to 2500 uu away, never onto a friend of
  theirs and never more often than every 20 seconds. Any enemy type can be
  given grenades in its `[grenades]` section. Battle droids carry none, as
  in the original.
- **Geonosian warriors fight with their force pike (tested in the cloud
  sandbox with automatic tests, not yet in the real game).** Like the
  original, they carry no gun: they rush at you and hit up close (350 uu
  reach, 125 damage, about one hit every 1.6 s). Any enemy type can get a
  melee weapon in its `[combat]` section.

- **Saving and loading (tested in the cloud sandbox on geo_01a; not yet on Windows).**
  Quicksave with F5 and quickload with F9. The start of each level and each
  checkpoint the level reports are saved automatically to the checkpoint
  save. A save holds where you are and where you look, your health, shield,
  weapons and ammo, and the droids: which spawners have started, which
  droids are still alive, where they are and their health, and the squad:
  where each mate is, what they're doing, who is down, and the level:
  which doors and lifts are open or moving, and where each of the level's
  scripts has got to (after a load they carry on instead of starting the
  level over). Saves are
  readable text files in `%APPDATA%\rc_game\saves` on Windows or
  `~/.local/share/rc_game/saves` on Linux (change it with `[save] folder` in
  `settings.toml`), never in the game's folder. The scripted run saves,
  walks 422 units away and quickloads, and the player comes back 0.00
  units from the saved spot with the ammo as it was.

## What doesn't work yet

- Not tested yet: textures stored inside the 76 maps that weren't sampled, and the
  texture scan on Windows (run in the cloud sandbox on copies of the files).
- Level scripts run, but only part of what they ask is carried out:
  doors, barricades, damage to the player, bacta dispensers, prompts,
  objectives (kept, not shown), checkpoints, enemy spawners, level
  changes, and lines of dialogue and sounds (the game's own, read from its
  sound files; a script waiting for a line waits for its real length). No
  subtitles yet, and every scripted sound plays at full loudness wherever
  its speaker is. Music cues, characters walking or animating, the squad
  joining and leaving and camera moves are logged as "not done yet". The
  end-of-campaign debriefing screen isn't shown: the game goes straight
  on. Briefings and cutscene maps may wait for events nothing sends yet,
  so you may get stuck in one. Not tested on Windows.
- Music: the menu and level tracks play, but the switch to battle music
  waits for enemies that can say a fight has started. 13 "streamed" ambient loops for the RAS level name files that don't
  exist in the PC install. `SoundMultiple` groups (a set of takes the game
  picks from) are played for weapons, grenades and enemy voices. No 3D
  (positional) sound yet: enemy sounds just get quieter with distance.
- Collision follows each prop's "can be walked through" settings and
  each mesh part's collision switch (since 2026-10-30; before, every prop
  was solid). Simplified collision shapes aren't read, so solid props
  block by their drawn shape. Not tested on Windows or by playing.
- Levels are drawn without characters or effects. Door and lift sounds are
  logged, not played, and doors opened by the game's scripts or squad
  orders don't open yet. The gunship isn't solid. 9 maps have windows onto a sky but no sky box, so they show a flat
  colour there. The level viewer's camera flies through walls (`rc_walk`
  has collision).
- Models are drawn without bump maps or transparency. Only a
  material's main colour texture is used.
- No levels yet: walking only works in the hand-made test room. No weapons,
  enemies, health or sound.
- Enemies: only battle droids so far. Level scripts don't run yet, so
  most droid spawners (27 of 36 on geo_01a) start when you come within
  3000 units instead of when the mission says, and barricades stay shut.
  On geo_01a the start area is closed in by such a barricade, so in
  `rc_game` you won't reach the droids yet; `rc_droidview` shows them.
  Some droid numbers are guesses (marked in the data file), including how
  far a shot can miss. No hit reactions or death effects beyond the
  animation. Droids don't use jump links or doors.

- Weapons: no muzzle flash, and switching weapons is instant for firing
  (the arms do play the holster and draw animations). The first-person
  arms have no blending between animations, aren't lit by the level (a
  fixed brightness), and the anti-armour attachment's grenade digit and
  the DC-15s pistol's charge bar on the gun don't move yet. How
  the view kick settles back is our own guess. Grenades have no models yet (a small glowing ball), and squad mates only
  throw grenades at grenade markers. Grenade explosions play the game's own
  explosion sounds (tested: the sounds are found; not listened to).
- Enemy types: not yet checked on the real maps. Geonosian warriors walk
  instead of flying, jumping and climbing. Trandoshan mercenaries use their own model, skin and animations from the game (checked by loading them; not yet seen on a real map).
- Enemy voices: battle droids, super battle droids, Trandoshans and
  Geonosian warriors say the game's own lines when they spot you, get hurt
  and die, with the hurt and death lines matched to explosions, EMP and
  falls where the game has them. Their other lines (heard something, saw a
  grenade, retreating, ...) aren't used yet. Not listened to in `rc_game`
  yet: the test checks every line is found in the game's files.
- Movement guesses: how the game blends the backwards and sideways speed
  limits, how fast it brakes, and how fast the mouse turns live in the game's
  compiled engine, so those follow Unreal Engine 2's usual behaviour or our
  own choice. See `docs/DESIGN.md`.
- Characters: only the first material slot (`rc_character` viewer), no
  sounds or effects from animation events. Blending between animations
  uses our own times; the original's are in its compiled engine.
- Android: the game compiles for Android phones but is not an app yet;
  see `docs/ANDROID.md` for what's missing. Wayland: built, not tested.
- Original menus: the main menu's moving background video isn't played
  (it uses a video format with no Rust decoder yet), and the sides of
  screens wider than 4:3 stay black. Only the main menu and its quit
  question are the original's so far. Mouse use wasn't tested by the
  script.
- The game (`rc_game`): no weapons, enemies or squad yet (other parts are
  being built to plug into it). No briefings or cut scenes. Saving: there's no Load Game screen yet (the
  menu's LOAD GAME stays greyed out). Options are a short list. Not tested on Windows,
  with a real controller, or with a touch screen.
- Controllers: no button pictures yet (words only), no rebinding screen
  yet, no rumble or aim assist. Gyro on phones isn't started. Building with
  SDL needs CMake installed (`cargo build --no-default-features` builds
  without SDL and without gyro).
- Lighting: the overall brightness follows the convention of the game's era
  (baked light drawn at double strength) and hasn't been compared with the
  real game side by side. Real-time lights use placeholder strengths and
  don't light props yet. Shadows from moving lights aren't drawn. Bump maps
  aren't used for lighting yet, and which way their "up" points isn't
  checked.
- 17 textures have small copies (mipmaps) that differ from the full picture.
  This is in the game's own files, not a decoding fault (see `docs/DESIGN.md`).

## How to run it

You need Rust (install from <https://rustup.rs>) and CMake, which builds
SDL, the controller and gyro reader (Windows: `winget install
Kitware.CMake`, then open a new PowerShell; Linux: your system's `cmake`
package). From this folder:

```
cargo run --release -p rc_game
```

This is the game. The first time, it writes `settings.toml` next to where
you run it from; if your game isn't in a usual Steam folder, open that file
and set `folder` under `[game]`. `--map geo_01a` skips the menus.
`--script` plays itself through the menus and a level and checks each step
(the log is `logs/rc_game.log`).

To check a controller without the game (lists every controller, which kind
it is taken for and whether it has a gyro, then logs its buttons, sticks
and gyro to `logs\rc_padtest.log` for 20 seconds):

```
cargo run --release -p rc_pads --bin rc_padtest
```

To look at one of the original menus without starting the game:

```
cargo run --release -p rc_probe -- --menu
```

This paints the main menu to `extracted\menus\CTMenuMain.png` and logs every
picture, text and button to `logs\rc_menu.log`. Add a class name, such as
`--menu XInterface.MenuQuestionYesNo`, for another menu. `rc_probe --pages` does
this for every page in `data/ui/original_menus.toml` and ends with a
summary of which would use their original menu in the game, and why not.

The other programs test one piece each:

```
cargo run --release -p rc_probe
```

This opens `GameData\System\Core.u` from the game folder and logs what's inside.
To check every package in the game:

```
cargo run --release -p rc_probe -- "C:\Games\Steam\steamapps\common\Star Wars Republic Commando\GameData"
```

To read every texture in the game:

```
cargo run --release -p rc_probe -- --textures
```

This logs to `logs\rc_textures.log` and ends with a `--- texture summary ---` line.
To save one package's textures as pictures you can open:

```
cargo run --release -p rc_probe -- --export-textures "C:\Games\Steam\steamapps\common\Star Wars Republic Commando\GameData\Textures\weapontextures.utx"
```

The PNGs go to `extracted\textures\weapontextures\`, which git never tracks.

To open a window showing one texture (the first build downloads and compiles
Bevy, which takes several minutes):

```
cargo run --release -p rc_viewer
```

You should see a window titled `Republic Commando texture: DC15Blaster` with the
texture in the middle of a dark grey square. The log goes to
`logs\rc_viewer.log`. Look for the line starting `on-screen check PASS`.
To show a different texture, give the package and the texture's name, and add
`--frames 300` to close the window by itself after 300 frames:

```
cargo run --release -p rc_viewer -- "C:\Games\Steam\steamapps\common\Star Wars Republic Commando\GameData\Textures\weapontextures.utx" WookieeMelee --frames 300
```

To show the DC-15 blaster in 3D:

```
cargo run --release -p rc_viewer -- --mesh
```

You should see a window titled `Republic Commando model: DC15Blaster` with the
blaster seen from the side. After a few seconds it starts turning slowly. The
log goes to `logs\rc_viewer_mesh.log`. Look for `on-screen check PASS`. Other
models work too, for example
`cargo run --release -p rc_viewer -- --mesh "C:\Games\Steam\steamapps\common\Star Wars Republic Commando\GameData\StaticMeshes\weapons.usx" DC17mBlaster`.
To check every static mesh: `cargo run --release -p rc_probe -- --meshes`
(log in `logs\rc_meshes.log`).

To walk around the test room in first person (click the window to look
around with the mouse, Escape to free the mouse):

```
cargo run --release -p rc_walk
```

To let it test itself instead (it moves on its own and closes when done):

```
cargo run --release -p rc_walk -- --script
```

The log goes to `logs\rc_walk.log`. The last line should be
`SCRIPT DONE: 21 checks passed, 0 failed`. To try different movement
numbers, copy `data\movement\player_commando.toml`, edit it and add
`--settings yourfile.toml`.

To walk a real level (the game folder is found the same way as the level
viewer's; set `RC_GAME_DIR` if yours isn't the default Steam folder):

```
cargo run --release -p rc_walk -- --map geo_01a
```

The level is drawn exactly like the level viewer draws it: textured, with
its baked light (and `lighting.cfg`, if you made one). Add `--script` to let
it walk a square on its own and check itself. You start at the level's
PlayerStart, which is in the gunship's hold. The gunship isn't solid yet, so
you drop to the floor below first.

To try the weapons on a firing range (click the window to aim with the
mouse, left button fires, R reloads, Q or the mouse wheel switches weapon,
Escape frees the mouse):

```
cargo run --release -p rc_combat --bin rc_shoot
```

The window title shows your weapon, ammo, health and shield. To let it test
itself instead (it shoots on its own and closes when done):

```
cargo run --release -p rc_combat --bin rc_shoot -- --script
```

The log goes to `logs\rc_shoot.log`. The last line should be
`script done: 17 of 17 checks passed`. With the game folder found, you hold
the original first-person arms and gun: the log shows lines like
`arms: dc17m_blaster plays Reload` as the animations change.

To load a level and fly around it:

```
cargo run --release -p rc_level_viewer
```

A window titled `Republic Commando level: geo_01a` opens at the player's start
point. Controls: W A S D to move, E or Space up, Q or Ctrl down, hold Shift to
go faster, hold the right mouse button and move the mouse to look around, the
mouse wheel changes speed. The log goes to `logs\rc_level_viewer.log`. Look for
`BSP checks: all passed`, `lightmap checks: all passed`, the `floor test`
lines and `on-screen check PASS`. To compare with the old full-brightness
look, make a file called `lighting.cfg` in this folder containing the line
`baked_light = false` (delete it to go back).
For another map, give its name, for example
`cargo run --release -p rc_level_viewer -- ras_01a`.
`--pitch 60` starts the camera looking 60 degrees up (to see the sky) and
`--no-sky` leaves the sky out.

To check every level at once:

```
cargo run --release -p rc_level_viewer --bin rc_level_check
```

It takes about 15 seconds after building, writes `logs\rc_level_check.log`,
and ends with one line per map and `79 of 79 maps loaded, 0 failed`.

To check every door and lift in every level:

```
cargo run --release -p rc_movers --bin rc_mover_check
```

It writes `logs\rc_mover_check.log`: every mover and trigger with its values,
each mover's position over time as it opens and closes, and a ride on every
lift. The last lines are totals, ending with `timing: N of N movers opened
and closed in their MoveTimes` and `lift rides: N of N passed`. Give map
names to check only those, for example `... rc_mover_check -- geo_01a`.

To ride a lift in first person (stands you on the map's first lift and sets
it going; add `--script` to have it check the ride by itself):

```
cargo run --release -p rc_walk -- --map MAP --ride
```

To check every character and animation in the game:

```
cargo run --release -p rc_probe -- --characters
```

This logs to `logs\rc_characters.log` and ends with a `--- character summary ---` part.

To see an animated clone trooper (the first build compiles Bevy's 3D parts,
which takes several minutes):

```
cargo run --release -p rc_character
```

You should see a window titled `Republic Commando: CloneTrooper playing
WalkForward (CloneCommandoSet)` with a white-armoured clone walking on the
spot. The log goes to `logs\rc_character.log`; look for the line starting
`on-screen check PASS`. Give a different animation by name, or add `--list`
to log all of them, for example `cargo run --release -p rc_character -- Reload`.
Other characters: `cargo run --release -p rc_character -- --package
"C:\Games\Steam\steamapps\common\Star Wars Republic Commando\GameData\Animations\droid.ukx" --mesh BattleDroid WalkForward`.

To watch battle droids fight a stand-in player in geo_01a (a window; it
closes by itself after 40 seconds of fighting):

```
cargo run --release -p rc_enemies --bin rc_droidview -- --check
```

The log goes to `logs\rc_droidview.log` and ends with CHECK lines and
`DONE: 0 checks failed`. Screenshots go to `logs\droidview_with.png` (the
droids) and `logs\droidview_droids.png` (only the droids' pixels). The same
fight with no window, faster, is `cargo run --release -p rc_ai --bin
rc_droidsim -- geo_01a`, and `cargo run --release -p rc_nav --bin
rc_navcheck -- geo_01a` checks a map's path network.

To read every sound in the game:

```
cargo run --release -p rc_probe -- --sounds
```

This logs to `logs\rc_sounds.log` (one line per sound) and ends with a
`--- summary ---` block. Expect `Sound objects: 7974  decoded: 7974  with
problems: 0  failed: 0`. To save one package's sounds as `.wav` files you can
play:

```
cargo run --release -p rc_probe -- --export-sounds "C:\Games\Steam\steamapps\common\Star Wars Republic Commando\GameData\Sounds\ui_sound.uax"
```

To hear a rifle firing over a looping battle ambience (no window; it stops by
itself after 8 seconds):

```
cargo run --release -p rc_soundtest
```

The log goes to `logs\rc_soundtest.log`. Look for `numbers check ...: PASS`,
`-> PLAYED` and an `ambience speed` close to `1.00x`. Pick other sounds with
`--weapon NAME`, `--ambient NAME` and `--seconds N` (names are in
`logs\rc_sounds.log`).

If your game is somewhere else, set `RC_GAME_DIR` first, for example
`set RC_GAME_DIR=D:\Games\Star Wars Republic Commando`.

Results go to the screen and to `logs\rc_probe.log`. The program only reads the
game folder and refuses to run from inside it.

To run the squad's test on the first level (no window; about a minute):

```
cargo run --release -p rc_squad_plugin --example rc_squad_sim
```

It logs to `logs\rc_squad_sim.log` and should end with `SIM DONE: 33 checks
passed, 0 failed`. Lines starting `squad: t=` show each squad mate's
position and distance to you once a second; `ORDER`, `sets off`,
`arrived`, `STUCK` and `working at` lines show what they decided;
`targets`, `fires at`, `KILLED`, `goes to revive`, `revived` and the
`fight:` lines every 5 s show their fight.

The squad's fight in the test room, with no game files needed (a few
seconds):

```
cargo test --release -p rc_squad_plugin --test fight_room
```

It should say `test result: ok. 1 passed`. The log is `logs\fight_room.log`.

The squad's context orders in the test room (bacta heals, a demolition
charge blows up a stand-in droid), no game files needed:

```
cargo test --release -p rc_squad_plugin --test jobs_room
```

It should say `test result: ok. 1 passed`. In `logs\jobs_room.log` look
for `Fixer healed to 400`, `Scorch set the charge ... 10.0 s`, then ten
seconds later `BLAST (Demolition, Scorch)` and `blast hurt 1 bodies`.

To check that Sev, Fixer and Scorch are drawn with their original models
and armour (opens a window for a few seconds):

```
cargo run -p rc_squad_plugin --example rc_squad_look_check
```

It stands the three side by side, checks every animation, checks each
armour is the one the original gives that commando, and compares the
outline and colours on screen with what the model and its pictures say. It
should end with `LOOK CHECK DONE: 48 checks passed, 0 failed`. The log is
`logs\rc_squad_look_check.log`, and `logs\rc_squad_look_check.ppm` is the
captured picture. To list what a model package holds (for editing
`looks.toml`):

```
cargo run -p rc_model --example rc_model_list -- "C:\Games\Steam\steamapps\common\Star Wars Republic Commando\GameData\Animations\clone.ukx" --sequences
```

## Credits

- [Bevy](https://bevy.org) game engine, MIT or Apache-2.0 license. Its audio
  uses [rodio](https://github.com/RustAudio/rodio) and
  [hound](https://github.com/ruuda/hound) (both MIT or Apache-2.0).

The package and sound formats were learned from these. No code was copied from them.

- [UE Viewer (UModel)](https://github.com/gildor2/UEViewer) by Konstantin Nosov, MIT
  license. Source of the Republic Commando-specific package details.
- [Unreal Wiki: Package File Format](https://unrealarchive.org/wikis/unreal-wiki/Legacy:Package_File_Format.html).
- [SWRC Wiki](https://wiki.swrc-modding.net) for game versions and folder layout.
- [SWRC-Modding/CT](https://github.com/SWRC-Modding/CT), read for reference only (it
  has no license, so nothing is copied from it).
- Project structure ideas from [IW4L](https://github.com/vladtrc/iw4L) (Apache-2.0),
  [gang-beasts-rust](https://github.com/muffinmxn/gang-beasts-rust) (MIT) and
  [benilla](https://github.com/samwhosung/benilla) (MIT or Apache-2.0).

See [docs/RESEARCH.md](docs/RESEARCH.md) for everything found about the game's formats.

Star Wars: Republic Commando is © Lucasfilm / LucasArts. This project is not
affiliated with or endorsed by them.
