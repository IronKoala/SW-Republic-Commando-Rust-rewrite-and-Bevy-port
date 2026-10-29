# Running on Linux and Android

## Linux

`rc_game` builds and runs on Linux (tested in the cloud sandbox, under X11
with software rendering). Bevy's Wayland support is switched on as well as
X11, so it opens a native window on either; Wayland itself is **not tested**
yet. Nothing in the game uses a Windows-only path: the game folder comes
from `settings.toml` or the usual Steam folders on Linux
(`~/.steam/steam/steamapps/common/...`, `~/.local/share/Steam/...`, and the
Flatpak Steam folder), and file names are matched without caring about
upper and lower case, because Linux file systems care and the game's own
references don't.

Libraries Bevy needs on Linux (Debian/Ubuntu names): `libasound2-dev`
(sound), `libudev-dev` (gamepads), `libwayland-dev` and `libxkbcommon-dev`
(windows), `pkg-config`.

## Android: how far it gets

**Checked (cloud sandbox, 2026-10-29):** `cargo check --target
aarch64-linux-android -p rc_game` passes. That means all of our code and all
of Bevy's compile for a 64-bit Android phone. It does **not** mean there is
an app yet: nothing has been linked, packaged or run on a phone.

Two settings in `crates/rc_game/Cargo.toml` made that work, and only apply
when building for Android:

- `blake3` with its `pure` feature, so it doesn't need a C compiler for ARM.
- Bevy's `android-native-activity`, which is how Bevy starts on Android
  without Java or C++ code of our own.

## Android: what's still needed

1. **Make the game a library.** Android loads games as a shared library
   (`.so`), not a program. Move the body of `main()` in
   `crates/rc_game/src/main.rs` into `lib.rs` as `pub fn run()`, mark it
   `#[bevy_main]`, and add `crate-type = ["lib", "cdylib"]`. The desktop
   `main()` then just calls `run()`.
2. **Install the Android tools:** the Android NDK (C compiler and linker for
   phones) and SDK, and a packager such as `cargo-apk` or `xbuild`. The cloud
   sandbox doesn't have them; they're large downloads.
3. **Pick the game folder.** The player copies their own `GameData` folder to
   the phone; the game must ask where with Android's folder picker (the
   "Storage Access Framework") the first time, and remember the answer in
   `settings.toml`. Our code already takes the game folder from the settings
   file and never assumes a drive letter, so only the picker itself is
   missing. Until then, setting `folder` by hand in the settings file works.
4. **Where settings and logs live.** On a phone the program's start folder
   isn't writable. `settings.toml` and `logs/` need to go in the app's own
   storage folder (Android gives each app one). This means turning the
   fixed `settings.toml` and `logs/rc_game.log` paths into "the app folder
   plus that name", only on Android.
5. **Our data files.** `data/` (controls, theme, music lists, levels) has to
   be packed into the app as "assets" and copied to the app folder on first
   start, or read through Android's asset manager.
6. **Touch controls on screen.** Touch already works without drawings:
   drag on the left half to move, drag on the right half to look, tap the
   right half to fire (`[touch]` in `data/input/bindings.toml`). Drawn
   buttons for jump, crouch, reload, grenade and the squad orders still
   need to be added to the on-screen display.
7. **Graphics limits.** The level drawing uses only plain textured materials
   and lightmaps, which phones handle. Not tested on a phone GPU. Keep the
   real-time lights and shadow maps few; mobile GPUs struggle with many.
