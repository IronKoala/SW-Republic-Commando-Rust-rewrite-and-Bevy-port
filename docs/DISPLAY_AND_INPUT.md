# Screens and controls

Project goals (from the user, 2026-10-29):

- any resolution and screen shape, with proper ultrawide and no fisheye
  or fishbowl stretching;
- mouse and keyboard and a controller at the same time;
- wide controller support (Xbox, PlayStation, Switch, Steam Deck) with
  rebinding, dead zones and response curves;
- gyro aiming on handheld PCs, phones and gyro controllers.

This page says what `rc_game` does now and what is still to do.

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

**Still to do: edge stretching on very wide screens.** A normal 3D view
stretches things near the left and right edges, and the wider the angle,
the stronger it is. At 32:9 (135 degrees across) it is clearly visible. The
fix used by ultrawide-friendly games is an optional "Panini" or cylindrical
view: the 3D view is drawn as usual, then bent so the sides look natural.
In Bevy this is a full-screen pass after drawing. It needs its own small
piece of work and should be a setting, because it softens the edges.

## Dual input (done)

Every device is read every frame and added together: keys, mouse, every
connected gamepad, and touch. A player can walk with the stick and aim with
the mouse at the same moment. `ActionState::last_device` says which kind of
device was used last (keyboard and mouse, gamepad or touch), so button
prompts can follow it. Nothing shows prompts yet; the original's menus will
(see `docs/ORIGINAL_MENUS.md`).

## Controllers (partly done)

- **Reading controllers:** Bevy uses the `gilrs` library (0.11). On Windows
  it reads XInput (Xbox and anything that acts like one) and other
  controllers through Windows' gaming input. On Linux it reads any
  controller the system sees, using SDL's community list of controller
  layouts, which covers PlayStation, Switch Pro and most others. The Steam
  Deck's controls appear as an Xbox-style controller when the game runs
  under Steam. **Not tested** with a real controller yet: the cloud sandbox
  has none.
- **Rebinding:** all buttons are set in `data/input/bindings.toml`
  (`[gamepad]`, `[gamepad_sticks]`); a mod can replace that file. There is
  no in-game rebinding screen yet. The original has one (`CTMapKeyMenu`,
  `CTControllerOptionsRemapXboxMenu`) that will come with the original's
  menus.
- **Dead zones and response curves:** done. `controls.stick_dead_zone`
  ignores small stick movement and then stretches the rest back to the full
  range, so the stick doesn't jump. `controls.stick_look_curve` shapes the
  look stick: 1 is straight; the default 1.5 makes small pushes gentler for
  fine aiming while full tilt still turns at full speed.
- **Still to do:** separate dead zones per stick and for the triggers,
  per-controller button pictures (Xbox letters, PlayStation shapes, Switch
  letters), aim assist (the original has some on Xbox), and rumble (Bevy
  supports it through gilrs).
- **Android controllers:** not checked yet.

## Gyro aiming (not started)

**gilrs cannot read gyros.** Its 0.11 axes are only sticks, triggers and
the d-pad; there is no motion sensor support, so Bevy has none either.

Options, for its own thread:

1. **SDL3** (through the `sdl3` crate) reads the gyro and accelerometer on
   DualShock 4, DualSense, Switch Pro and Joy-Cons, and the Steam Deck's
   built-in gyro. It would sit beside gilrs, used only for motion, or
   replace it for controllers altogether. This is the most complete choice
   but adds a C library to the build on every system.
2. **Phones:** Android's own sensor service (through the `ndk` crate),
   read only on Android.
3. Steam Input can also turn the gyro into mouse movement outside the game
   with no work from us, which already works for Steam Deck players today.

However it is read, the gyro becomes extra `look` in `ActionState`, like
the mouse, with its own sensitivity, a "ratchet" button that pauses it, and
an option to only use it while aiming down the sights.
