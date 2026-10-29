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

- **Reading and playing sounds (tested in the cloud sandbox, not yet on
  Windows).** `rc_probe --sounds` reads every sound in the game's 23 sound
  packages (`.uax`). Every sound inside them is a normal `.wav` file
  (uncompressed 16-bit): 7,974 sounds, about 4 hours 11 minutes in total, all
  decoded with no failures. `rc_probe --export-sounds` saves them as `.wav`
  files in the gitignored `extracted/sounds/` folder. `rc_soundtest` plays the
  DC-17 rifle firing over a looping battle ambience through Bevy's audio and
  checks that Bevy decodes every sample exactly as our own reader does.

## What doesn't work yet

- Formats 12 and 14 (133 textures, probably bump maps) aren't decoded.
- Not tested yet: textures stored inside the 76 maps that weren't sampled, and the
  texture scan on Windows (run in the cloud sandbox on copies of the files).
- Other objects (meshes, level geometry, scripts) are not decoded yet.
- Sound: music (`.ogg` files in `GameData\Music`) isn't played yet, only
  found. 13 "streamed" ambient loops for the RAS level name files that don't
  exist in the PC install. `SoundMultiple` groups are read but not used yet.
  No 3D (positional) sound yet. Not tested on Windows.
- No 3D, no levels, no game yet. The Bevy window shows one flat picture.

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
