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
  G16, and two 2-byte-per-pixel formats numbered 12 and 14.

- **Turning textures into pictures (tested by numbers, not by eye).** The
  `rc_image` crate decodes P8 (with its palette), DXT1, DXT3, DXT5, RGBA8 and G16
  into normal colours, and `rc_probe --export-textures` saves them as PNG files
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

- **Reading 3D models (tested on every static mesh).** `rc_probe --meshes`
  reads every static mesh (the fixed models used for weapons, props and level
  pieces, in `.usx` files): corner points, triangles and texture coordinates.
  All 2,447 static meshes in the install read and pass every check.

- **Showing a model in 3D (tested in the cloud sandbox, not yet on Windows).**
  `rc_viewer --mesh` shows the DC-15 blaster with its texture, turning slowly.
  It checks itself by drawing the model on its own and comparing it with the
  screen: outline match 0.996 and colour difference 1.04 of 255, PASS. It also
  works for the DC-17m, the DC-15s pistol and the thermal detonator.

- **Loading a whole level (tested on Windows).**
  `rc_level_viewer` loads a map with its walls, floors and every placed prop,
  textured, and lets you fly around it. Tested on `geo_01a` (the first Geonosis
  level): 141,477 triangles, all 56 materials textured, loaded in a quarter of a
  second and drawn at a steady 144 frames per second on the user's PC. It checks itself by
  numbers: props sit under the AI's path points where they should, and the
  window is covered by the level. The level's geometry and actors were read
  from 5 maps with no errors.

## What doesn't work yet

- Formats 12 and 14 (133 textures, probably bump maps) aren't decoded.
- Not tested yet: textures stored inside the 76 maps that weren't sampled, and the
  texture scan on Windows (run in the cloud sandbox on copies of the files).
- Animated models (`.ukx`), sounds and scripts are not decoded yet.
- Levels are drawn without lights, lightmaps, sky, characters, effects or
  moving parts. No collision: the camera flies through walls.
- Models are drawn without lighting, bump maps or transparency. Only a
  material's main colour texture is used.
- No game yet.

## How to run it

You need Rust (install from <https://rustup.rs>). From this folder:

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

To load a level and fly around it:

```
cargo run --release -p rc_level_viewer
```

A window titled `Republic Commando level: geo_01a` opens at the player's start
point. Controls: W A S D to move, E or Space up, Q or Ctrl down, hold Shift to
go faster, hold the right mouse button and move the mouse to look around, the
mouse wheel changes speed. The log goes to `logs\rc_level_viewer.log`. Look for
`BSP checks: all passed`, the `floor test` lines and `on-screen check PASS`.
For another map, give its name, for example
`cargo run --release -p rc_level_viewer -- ras_01a`.

If your game is somewhere else, set `RC_GAME_DIR` first, for example
`set RC_GAME_DIR=D:\Games\Star Wars Republic Commando`.

Results go to the screen and to `logs\rc_probe.log`. The program only reads the
game folder and refuses to run from inside it.

## Credits

- [Bevy](https://bevy.org) game engine, MIT or Apache-2.0 license.

The package format was learned from these. No code was copied from them.

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
