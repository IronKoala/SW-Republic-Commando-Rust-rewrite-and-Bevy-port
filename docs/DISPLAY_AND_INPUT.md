# Screens and controls

Project goals (from the user, 2026-10-29):

- any resolution and screen shape, with proper ultrawide and no fisheye
  or fishbowl stretching;
- mouse and keyboard and a controller at the same time;
- wide controller support (Xbox, PlayStation, Switch, Steam Deck) with
  rebinding, dead zones and response curves;
- gyro aiming on handheld PCs, phones and gyro controllers.

This page says what `rc_game` does now and what is still to do.

Controllers, button names and the gyro are in `crates/rc_pads`; the Panini
view is in `crates/rc_panini`; the game's input code (bindings, prompts,
rebinding) is in `crates/rc_game/src/input.rs` and `rebind.rs`.

## Screen shapes (done)

**Field of view is "Hor+".** The original's resolutions were all 4:3
(`System.ini` lists 640x480, 800x600 and 1024x768), and its field of view
of 85 degrees is the angle across that 4:3 screen. `video.fov` in
`settings.toml` keeps that meaning. The game turns it into a fixed angle up
and down (69.0 degrees for 85), and a wider screen shows more at the sides:

| Screen | Across | Up and down |
|---|---|---|
| 4:3 (1024x768) | 85.0 | 69.0 |
| 16:9 (1280x720) | 101.4 | 69.0 |
| 21:9 (1720x720) | 117.3 | 69.0 |
| 32:9 (2560x720) | 135.5 | 69.0 |
| Phone held upright (540x960) | 42.3 | 69.0 |

These are the numbers the game logs (`player: view ...`) at each size; the
script run passed 17 of 17 checks at all five.

**The HUD and menus fit any shape.** Everything is placed from the screen's
edges and sized by the screen's height, or its width when that is smaller
(a phone held upright), so text never runs off the side. On screens wider
than 16:9 the HUD stays in a centred 16:9 area (`video.hud_max_aspect`;
0 lets it fill the screen), so health and ammo aren't in the far corners.

**Panini view for very wide screens (done, optional).** A normal 3D view
stretches things near the left and right edges, and the wider the angle,
the stronger it is: at 32:9 a droid at the edge looks almost five times as
wide as the same droid in the middle. `video.panini` in `settings.toml`
(0 is off, the default; up to 1) bends the view into a "Panini" projection,
as ultrawide-friendly games do. The 3D view is drawn as usual but a little
wider, then a full-screen pass (`crates/rc_panini`, shader `panini.wgsl`)
moves each pixel to its Panini place. The width is chosen so the middle of
the screen keeps exactly its normal size, so aiming feels the same, and you
see a little more at the sides.

The cost is that the middle is softer, because the wider view spends fewer
pixels on it. The game logs it (`panini: ... middle keeps N% of its
detail`). The drawn view never goes past 150 degrees across. Past that, the
middle is enlarged a little instead.

| Screen | Strength | Seen across | Middle size | Middle detail |
|---|---|---|---|---|
| 16:9 | 0.25 | 108.8 (was 101.4) | same | 87% |
| 21:9 | 0.25 | 126.9 (was 116.1) | same | 80% |
| 32:9 | 0.2 | 148.4 (was 135.5) | same | 65% |
| 32:9 | 0.5 | 150.0 (widest) | 1.3 times bigger | 65% |

Checked with `cargo run -p rc_panini --example panini_check`, which draws
seven equal cubes in a ring on a 2560x720 window and measures them in
screenshots. Plain view: widths 252, 133, 73, **54**, 73, 133, 252 pixels.
Panini 0.2: 161, 104, 67, **54**, 67, 104, 161. The middle cube keeps its
54 pixels and the edge cube's stretch drops from 4.7 to 3.0 times. The
pictures are `logs/panini_plain.png` and `logs/panini_bent.png`. **Not
tested** inside a level yet (the cloud sandbox had no game files this
time), only in that check program.

## Dual input (done)

Every device is read every frame and added together: keys, mouse, every
connected controller, gyros and touch. A player can walk with the stick and
aim with the mouse at the same moment. `ActionState::last_device` says
which kind of device was used last (keyboard and mouse, gamepad or touch).

**Button prompts follow it.** `rc_game_core::InputPrompts` names the
button for every action on the device used last: "F" and "Left Mouse"
after the keyboard, "X" and "RT" after an Xbox controller, "Square" and
"R2" after a PlayStation one, "Y" and "ZR" on Switch, "L4" on a Steam Deck.
The names are in `data/input/button_names.toml` (one section per kind of
controller, then per family, then generic), so a mod can change them. The
log says when prompts switch (`input: prompts now for Gamepad (ps5): fire
is R2, use is Square`). Nothing draws prompts yet: menus, the HUD and
level scripts read `InputPrompts` when they need "Press F to use".
**Placeholder:** prompts are words. The original PC game has no
controller button pictures, so pictures need new art.

## Controllers (done; not tested with real hardware)

- **Reading controllers: SDL3.** `crates/rc_pads` builds SDL 3.4 from its
  source code with the game, with only its controller, HID, sensor and
  rumble parts (no window, sound or graphics). SDL reads Xbox,
  PlayStation 3/4/5, Switch Pro, Joy-Cons (single or as a pair), GameCube
  adapters, the Steam Deck's own controls and most others, and says
  exactly which kind each is. SDL's controllers become ordinary Bevy
  `Gamepad`s, so the rest of the game reads them as before. Bevy's own
  reader (gilrs) is switched off when SDL starts, so no button counts
  twice. If SDL can't start, or on Android, or with
  `--no-default-features`, gilrs is used as before, with no gyro, and the
  kind is guessed from the controller's USB ids and name.
- **Building SDL needs CMake** (and the C compiler Rust already uses). The
  GitHub checks build it on Windows and Linux. On Linux nothing else is
  needed; it finds controllers through the system at run time.
- **Extra buttons through SDL:** the Steam Deck's four back buttons (L4,
  R4, L5, R5) and Xbox Elite or DualSense Edge paddles (`LeftPaddle1`,
  `RightPaddle1`, `LeftPaddle2`, `RightPaddle2`), the PlayStation
  touchpad click (`Touchpad`) and Share, Capture or Mute (`Misc1`).
- **Rebinding (engine done; no screen yet):** every action's keys and
  buttons are in `data/input/bindings.toml`. In the game, a menu sends
  `rebind::StartRebind { action, device }`. The next key, mouse button or
  wheel turn (or controller button or trigger) becomes that action's
  button, Escape or five seconds cancels, and `RebindDone` says what
  happened and which other actions share the button. The change is saved at
  once to `my_bindings.toml` next to `settings.toml`. That file holds only
  your changes, so a game update or mod that changes the defaults still
  reaches everything you didn't change. Delete it to get the defaults back.
  The controls page that sends these belongs with the original options
  menus (`CTMapKeyMenu`, `CTControllerOptionsRemapXboxMenu`), which the
  menus part is bringing over.
- **Dead zones, curves, sensitivity:** in `[controls]`: `stick_dead_zone`
  (move stick) and `look_stick_dead_zone`, each stretching the rest back to
  the full range so the stick never jumps; `stick_move_curve` and
  `stick_look_curve` (1 is straight, higher is gentler near the middle);
  `stick_look_speed` and `stick_look_vertical` (up and down speed compared
  with across); `trigger_threshold` (how far to pull a trigger to fire);
  `mouse_sensitivity` and `invert_y`.
- **Checking a real controller:** `cargo run -p rc_pads --bin rc_padtest`
  lists every controller SDL finds, which kind it is taken for and whether
  it has a gyro, then four times a second logs its buttons, sticks,
  triggers and gyro turn to `logs/rc_padtest.log`. `rc_game --log-input`
  writes every pressed action and the move, look and gyro numbers to
  `logs/rc_game.log`.
- **How it was tested without hardware:** the game's input code runs in a
  small Bevy app fed with pretend controllers (the same messages SDL and
  gilrs send): buttons, sticks, half-pulled triggers, keyboard and
  controller at once, prompts switching between Xbox, PlayStation, Switch
  and Steam Deck names, the gyro with ratchet, toggle and "aiming" mode, and
  rebinding a key and a Deck back button with the saved file read back.
  SDL's own pretend controller (a "virtual joystick" with PlayStation 5 ids,
  a gyro and an accelerometer) is read through the real SDL reader: it is
  taken for a PS5 controller, Cross, the stick, the trigger and a gyro turn
  of 0.024 radians all come through, and unplugging removes it. These run
  on Windows and Linux in the GitHub checks.
- **Still to do:** aim assist (the original has some on Xbox), rumble
  (SDL's rumble part is built in, not used yet), and button pictures.

## Gyro aiming (done; not tested with real hardware)

SDL reads the gyro and accelerometer of DualShock 4, DualSense, Switch Pro,
Joy-Cons and the Steam Deck, and a handheld PC's built-in gyro where the
system offers one through its sensor service (on Windows). The gyro of the
controller used last is used, or else the built-in one.

Every reading goes through `rc_pads::GyroFilter`:

1. **Calibration.** Every gyro reads a little turn when still ("drift").
   When the device lies still for 1.5 seconds, its average reading is taken
   as the drift and taken off from then on. The log says so (`pads: gyro
   ... lay still; drift measured as ...`).
2. **Player space** (the default; `space = "local"` turns it off). The
   accelerometer feels gravity, so the game knows which way is really up,
   and turning around that axis turns the view, however the controller or
   handheld is held.
3. **Smoothing** of very small movements only (`smoothing_threshold`,
   degrees per second), so hand shake doesn't wobble the view while real
   movements have no delay. `tightening` can damp them further.
4. **Sensitivity** (`sensitivity`: 2 turns the view twice as far as the
   controller), `vertical_scale`, `invert_x`, `invert_y`.

All of these are in `[gyro]` in `settings.toml`. `mode` is `always`,
`aiming` (only while zoom is held) or `off`. The `gyro_ratchet` action,
held, pauses the gyro so you can re-centre the controller, like lifting a
mouse. By default it is the back buttons. `gyro_toggle` (the PlayStation
touchpad by default) turns it on and off. The gyro adds to the look like
the mouse does, so sticks, mouse and gyro all work together.

**Steam Deck and Steam Input:** when the game runs under Steam with Steam
Input on, Steam hides the real controller and offers an Xbox-style one
with no gyro. Either turn Steam Input off for the game (then SDL reads the
Deck's gyro directly), or let Steam Input turn the gyro into mouse
movement, which also works.

**Phones (planned, not started):** SDL isn't used on Android (it needs its
own Java start-up code there). The plan is to read the phone's gyro and
accelerometer through Android's sensor service (the `ndk` crate's
`ASensorManager`) and send the same `rc_pads::GyroSample` messages with
`GyroSource::BuiltIn`. Everything after that (calibration, player space,
ratchet, settings) is shared. The axes match: SDL uses the same ones as
Android (X right, Y up the screen, Z out of it).
