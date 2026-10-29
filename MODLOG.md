# MODLOG

Running record of changes to the Star Wars: Republic Commando Rust and Bevy
rewrite. Newest entries first. Format and rules are in `MODLOG-template.md`.

## 2026-10-29: Lights read from maps; Bevy lighting module with a settings file

- **Changed:** New `crates/rc_package/src/light.rs` (lights and zone ambients from an actor's properties, Unreal hue/saturation to colour). New `crates/rc_viewer/src/lighting.rs` (spawns Bevy point, spot and directional lights and the ambient light; every factor in `LightingSettings`, loadable from a `key = value` text file for mods). `lighting.rs` is compiled into the viewer (using the 3D parts PR #4 added) but no viewer mode uses it yet. DESIGN ("Lights in maps", "Lighting in Bevy") and README updated.
- **Why:** Roadmap step 2: lights. Prepare them so they can be switched on as soon as a level loads.
- **Tested how:** Wrote throwaway test programs (kept outside the repo) and ran them on `geo_01a`, `pro`, `ras_01a` and `yyy_01b`, copied read-only from the install. 4 unit tests for colours, light classes, axes, facing and the settings file (23 in the workspace, all pass). A scratch Bevy app placed `ras_01a`'s lights over a white floor, captured the screen and measured it (results in the next line).
- **Result:** Measured the actor state-frame layout (2 bytes where UE2 has 4): all 600 light/zone/level actors read with 0 bytes left over; shared with the level thread. Lights per map: 31, 211, 286, 37; only 2-9% dynamic. Radius is in plain Unreal units (typical radius 350-1,200 vs 230-840 to the nearest light). Static meshes carry baked light as one colour per vertex: read on all 1,267 instances. Bevy test on `ras_01a`: 282 point lights and 4 spotlights created; with all lights on, the floor's brightest pixel is 225 of 255, with lights off (ambient only) 47, so the lights do light the scene. The first two captures were black (the known sandbox quirk).
- **Still broken / not tested:** Brightness factors are placeholders, not matched to the game. Spotlight cone angle is a guess. Light defaults are UE2's, not read from the game's `Engine.u`. Where the BSP walls' baked light is stored: not found. Not connected to a level, not tested on Windows.
- **Next:** When the level thread's loader lands, show its lights; then show the baked vertex colours and tune the factors against them.

## 2026-10-29: Why 17 textures' small copies don't match

- **Changed:** DESIGN "How we check pictures" rewritten with the findings. No code changes.
- **Why:** 17 textures failed the mip check; we needed to know whether our decoder was wrong.
- **Tested how:** Throwaway program comparing every mip with the one before, measuring how varied each mip is and its average colour, and trying the small mips sideways and upside down.
- **Result:** The decoder is fine: the same formats give under 5 on 2,000 other textures, including 159 other non-square ones, and other orientations made things worse. The game's files store different pictures in these mips: 5 arena textures turn exactly flat grey (127, 125, 127) from mip 3 down, one blast door has a different picture on every level, 7 tiny glow sprites are likely just compression error (a guess), and the rest are screens, noise and two menu buttons.
- **Still broken / not tested:** Not checked by eye.
- **Next:** Use the game's own mips in Bevy rather than generating new ones.

## 2026-10-29: Decode the two bump-map formats (12 = V8U8, 14 = L6V5U5)

- **Changed:** `crates/rc_image/src/lib.rs`: new `PixelFormat::V8U8` and `PixelFormat::L6V5U5`, decoded to normal-map colours (L in alpha), 2 new unit tests. `rc_package::texture::format_name` names them. `rc_probe` and `rc_viewer` decode them. DESIGN gains a "Bump maps" section; README updated.
- **Why:** 133 textures (the game's bump maps) couldn't be decoded.
- **Tested how:** Tried six candidate layouts on every bump map and compared each decoded mip 0, shrunk, against mip 1. Then ran `rc_probe --textures` on all 57 texture packages (copied read-only into the cloud sandbox). All 21 unit tests in the workspace pass (after bringing in PR #4).
- **Result:** Clear winners, as a fraction of the difference between unrelated pictures: format 12 as V8U8 0.06 (next best 0.36), format 14 as L6V5U5 0.08 (next best 0.28). UE Viewer's unverified guess (CxV8U8 for 14) scored 0.34. Scan: `decoded: 2312  failed: 0`, formats `29 V8U8`, `104 L6V5U5`. Bump-map mip difference median 1.8, one over 10 (11.2). `rc_viewer` shows them and its on-screen check passes: `Wookiee_Bump` (L6V5U5) 3.53, `Wookiee_HairBump` (V8U8) 7.20, and `DC15Blaster` still 3.77.
- **Still broken / not tested:** Which way V ("up") points for Bevy: not tested. What L means in format 14: unknown (maybe shininess). Not run on Windows.
- **Next:** Use them for lighting in Bevy.

## 2026-10-29: rc_soundtest tested on Windows

- **Changed:** README "what works" updated. No code changes.
- **Why:** The sound test had only run in the cloud sandbox with a fake sound card.
- **Tested how:** The user ran `cargo run --release -p rc_soundtest` in `C:\Users\matth\Projects\sw-rc` and listened. Claude read `logs\rc_soundtest.log` from the PC.
- **Result:** The user heard it ("it worked"). Log: both sounds `MATCH` Bevy's decoder, the ambience started 0.017 s in, 4 of 4 rifle shots started, `PLAYED`, ambience speed `1.00x` real time. The package read took 22 ms.
- **Still broken / not tested:** Linux was tested only with a fake sound card. Android is not tested. Music (`.ogg`) is still not played.
- **Next:** Merge PR #5. Then play music, and later 3D sound with the level work.

## 2026-10-29: Read and play the game's sounds

- **Changed:** New crate `crates/rc_audio` (reads `.wav` files, measures them). New file `crates/rc_package/src/sound.rs` (reads `Sound`, `SoundStreamed` and `SoundMultiple` objects). New `rc_probe --sounds` and `--export-sounds` (`crates/rc_probe/src/sounds.rs`). New Bevy program `crates/rc_soundtest` (no window) that plays a weapon sound over an ambient loop. Updated `README.md` and `docs/DESIGN.md`.
- **Why:** Roadmap step 4: play the game's sounds.
- **Tested how:** 4 new unit tests on made-up `.wav` files (20 in total, all passing). Copied all 23 `.uax` packages and 2 music files from the install into the cloud sandbox (outside the repo) and ran `rc_probe --sounds` and `rc_soundtest`. The sandbox has no sound card, so `rc_soundtest` was run once with none (to check it notices) and then with a fake ALSA "null" sound card.
- **Result:** `Sound objects: 7974  decoded: 7974  with problems: 0  failed: 0  silent: 1` (the silent one is `fol_GEN_silence_01`), 15,068 s of audio, 31.7 s scan. All plain 16-bit PCM WAV. 291 `SoundStreamed` and 1,208 `SoundMultiple` read with nothing left over. `rc_soundtest`: rifle `wep_GEN_dc17Fire_02` 44100 Hz, 1 ch, 58069 frames, 1.317 s, peak 0.977, checksum ac6be1b553909337; ambience `amb_KAM_siege_lp_01` 22050 Hz, 2 ch, 219982 frames, 9.977 s, peak 0.534, checksum 8beac96a49db50df. Bevy's decoder matched both sample for sample (largest difference 0). With the fake sound card: ambience started, 3 of 3 shots started, `PLAYED`. With no sound card: `NOT PLAYED`, exit code 1, as intended.
- **Still broken / not tested:** Not tested on Windows or on a real sound card, so nobody has heard it yet, and the "1.00x real time" check is untested. Music (`.ogg`) is not played. 13 `amb_RAS_*` streamed loops point at files missing from the PC install. The second list inside `SoundMultiple` is always empty and its meaning is unknown. No positional (3D) sound yet.
- **Next:** Run `rc_soundtest` on the user's PC. Then play an `.ogg` music track, and later place sounds in 3D with the level work.

## 2026-10-29: Static meshes: read them, and show the DC-15 blaster in 3D

- **Changed:** New `crates/rc_package/src/static_mesh.rs` (reads `StaticMesh` objects: vertices, normals, UVs, triangles, sections, materials, and checks them), plus `Reader::f32`. New `rc_probe --meshes` (`crates/rc_probe/src/meshes.rs`, logs to `logs/rc_meshes.log`). `rc_viewer --mesh` (`crates/rc_viewer/src/mesh.rs`) shows one model with its texture, by default `DC15Blaster` from `weapons.usx`. It follows `Shader` materials to their `Diffuse` texture. Shared screenshot code in `rc_viewer/src/main.rs` was moved into `read_capture`. Bevy feature `bevy_pbr` added. 1 new unit test (one triangle built byte by byte). README and DESIGN updated.
- **Why:** Next milestone: show a game model in 3D, starting with the DC-15 because its texture is already known to be good.
- **Tested how:** Copied all 58 `.usx` packages plus `weapontextures.utx` and `hudarmstextures.utx` read-only from the install into the cloud sandbox. Ran `rc_probe --meshes` over them. Ran `rc_viewer --mesh` 3 times on DC15Blaster and once each on DC17mBlaster, DC15Pistol and ThermalDetonator, with Xvfb and software rendering. The viewer's new on-screen check draws the model itself in plain Rust and compares the outline and the colours with Bevy's screenshot. Texture mode re-run as a regression check. All 17 unit tests pass.
- **Result:** All 2,447 static meshes read, 0 failures, 0 check problems (2.09 million vertices, 1.57 million triangles, 3 s). DC15Blaster: 470 vertices, 342 triangles, 1 section, size 72.5 x 13.3 x 22.2 Unreal units, UVs 0.006 to 0.998, material `WeaponTextures.CloneWeapons.DC15Blaster` (a plain texture). 342 of 342 triangles face the same way as their stored normals after the axis swap. On-screen check: outline match 0.996, colour difference 1.04 of 255 (flipped texture: 22.88), PASS in 3 of 3 runs. DC17mBlaster 0.86, DC15Pistol 0.32, ThermalDetonator 2.12, all PASS.
- **Still broken / not tested:** Dead end: on one run the first capture showed only the background (the model wasn't drawn yet), which failed the check. Fixed by retrying, the same way as an all-black capture. 11 meshes have a few normals that are exactly zero, which the check now allows. Not tested: running on Windows. The collision data after the mesh streams isn't read. Only a Shader's `Diffuse` texture is used: no bump maps, transparency or lighting yet. The model is drawn unlit.
- **Next:** User runs `cargo run --release -p rc_viewer -- --mesh` on their PC. The level thread will reuse `rc_package::static_mesh`.

## 2026-10-29: rc_viewer tested on Windows

- **Changed:** README "what works" updated. No code changes.
- **Why:** The Bevy viewer had only been tested in the cloud sandbox.
- **Tested how:** The user ran `cargo run --release -p rc_viewer` on their Windows PC. I read `logs\rc_viewer.log` from the project folder through folder access.
- **Result:** `on-screen check PASS: average difference from the texture 3.77 of 255`, on the first capture (no black frame). Window 800x800, texture 720x720. Texture read and decoded in 1.8 ms. Steady 144 frames per second, probably held to the monitor's refresh rate.
- **Still broken / not tested:** Other textures not tried on Windows.
- **Next:** Show a mesh or a level's geometry in 3D.

## 2026-10-29: First Bevy window showing one game texture (rc_viewer)

- **Changed:** New crate `crates/rc_viewer` (Bevy 0.20.1, only the 2D parts turned on), added to the workspace in `Cargo.toml`; `Cargo.lock` updated. Started from the unbuilt draft written in the data loader thread. README and DESIGN updated.
- **Why:** First Bevy step: prove a texture read from the user's install can be shown in a game window.
- **Tested how:** Built and ran in the cloud sandbox with a virtual screen (Xvfb) and software rendering (llvmpipe), using a read-only copy of `weapontextures.utx` staged from the user's PC. Added an on-screen check: on frame 10 the program captures the window in memory and compares it pixel by pixel with the decoded texture. Ran it 12 times on `DC15Blaster`, `WookieeMelee` and `TDetonator`, plus once with the comparison deliberately flipped upside down, and once with a texture name that doesn't exist. Existing 16 unit tests still pass.
- **Result:** Window 800x800, texture drawn at 720x720. Average difference from the texture (0-255): DC15Blaster 3.77, WookieeMelee 4.32, TDetonator 0.55; all 12 runs PASS. Upside-down comparison: 22.56 (FAIL), so the check catches a wrong picture. Missing texture: `FAILED: no texture called NoSuchTexture in this package`, exit code 1, no window. Texture read and decode: 8 to 28 ms. Release build: compiled in 10 min 16 s in the sandbox, same PASS (3.77), about 21 frames per second with software rendering; a real graphics card should be much faster (not tested).
- **Still broken / not tested:** Dead end found: the first screenshot in the sandbox is often completely black (window not drawn yet), which made 8 of 12 early runs fail. Fixed by retrying up to 5 times, 10 frames apart. Not tested: running on Windows, and whether the black first capture happens there. A `cargo check` for Windows couldn't run in the sandbox (no Windows C compiler), so the Windows build is not tested.
- **Next:** User runs `cargo run --release -p rc_viewer` on their PC. Then show a mesh or a level's geometry in 3D.

## 2026-10-29: Texture pixels to pictures (PNG)

- **Changed:** New crate `crates/rc_image` (decodes P8, DXT1, DXT3, DXT5, RGBA8 and G16 to RGBA; minimal PNG writer). `rc_package::texture` gains `read_palette`, `palette_ref` and `masked`. `rc_probe --textures` now also decodes pixels and checks them. New `rc_probe --export-textures <path>` writes PNGs to the gitignored `extracted/textures/`. README and DESIGN updated.
- **Why:** The next step after reading texture bytes is turning them into real pictures.
- **Tested how:** 7 new unit tests in `rc_image`, including known PNG checksums (16 in the workspace, all passing). Ran `rc_probe --textures` on all 57 texture packages plus 46 others, copied read-only from the install into the cloud sandbox. Exported `weapontextures.utx` and checked every PNG's structure, checksums and size with Python's zlib.
- **Result:** 2,358 textures decoded, 0 failed. Comparing each decoded mip 0, shrunk to half size, with the decoded mip 1 gives these average differences (0-255): under 2 for 658, 2-5 for 884, 5-10 for 374, 10-20 for 70, over 20 for 17. Unrelated pictures differ by about 57. An experiment decoding non-square textures with width and height swapped made the median worse (12.9 vs 2.6), so the right orientation is in use. The 40 PNGs from `weapontextures.utx` are all valid.
- **Still broken / not tested:** Not checked by eye: nobody has looked at a PNG yet. 17 textures have large mip differences. Probably artist-made mipmaps, but not verified. Formats 12 and 14 (133 textures) aren't decoded. Palettes stored in another package aren't followed (none came up). Not run on Windows.
- **Next:** Start the Bevy side: open a window and show one decoded texture.

## 2026-10-29: Texture scan on every texture package; format 12 measured

- **Changed:** `crates/rc_package/src/texture.rs`: format 12 is now checked as 2 bytes per pixel (measured, not documented anywhere we found). README and DESIGN updated.
- **Why:** The first texture test only covered 50 packages. The user asked me to run the tests myself.
- **Tested how:** Copied all 57 `.utx` packages from the install into the cloud sandbox (read-only copy, outside the repo) and ran `rc_probe --textures` over them plus the 46 packages copied earlier.
- **Result:** `textures: 2491  OK: 2490  with problems: 1  failed: 0`, 404 ms, 355 MB of pixel data. Formats: 782 DXT3, 688 DXT1, 545 DXT5, 194 P8, 138 RGBA8, 104 format 14, 29 format 12, 11 G16. The one problem is `MP_Trando_Light_Cluster`, whose 1x1 mip is stored with 0 bytes. Its skip position matches and nothing is left over, so this looks like the game's own data, not a misread.
- **Still broken / not tested:** Not tested: textures inside the 76 maps not copied, and running the scan on Windows. Format 12's name and how formats 12 and 14 store colour are unknown. Pixels aren't converted into images yet.
- **Next:** Convert one texture to a PNG in the gitignored `extracted/` folder.

## 2026-10-29: Read textures (properties and mipmaps)

- **Changed:** Added `crates/rc_package/src/properties.rs` (reads an object's property list) and `texture.rs` (reads a `Texture`'s mipmaps and checks them). Added `rc_probe --textures` (`crates/rc_probe/src/textures.rs`), which logs to `logs/rc_textures.log`. Updated README and DESIGN.
- **Why:** Next step after reading the package tables: decode a real object. Textures were chosen because their sizes can be checked exactly.
- **Tested how:** 2 new unit tests (9 in total, all passing). Ran `rc_probe --textures` on the 50 packages copied from the install earlier.
- **Result:** `textures: 192  OK: 192  with problems: 0  failed: 0`. Formats: 143 P8, 24 DXT1, 18 RGBA8, 3 DXT5, 2 DXT3, 2 G16. Largest: GeonosianTank, 1024x1024 DXT1, 11 mips. Every skip position matched, every byte count matched its format, no bytes left over.
- **Still broken / not tested:** Not tested: the 54 texture packages that weren't in the sample, and running on Windows. I couldn't copy more files from the PC in this session. Pixels aren't converted into images. Other texture-like classes (Cubemap, Combiner, ...) are skipped.
- **Next:** Run the full texture scan on the user's PC, then convert one texture to a PNG in the gitignored `extracted/` folder.

## 2026-10-29: Full scan of the install on Windows

- **Changed:** No code changes. Updated `README.md` and `docs/DESIGN.md` with the result.
- **Why:** The first test only covered 50 packages, in the cloud sandbox.
- **Tested how:** The user ran `cargo run --release -p rc_probe -- "C:\Games\Steam\steamapps\common\Star Wars Republic Commando\GameData"` on their Windows PC.
- **Result:** `packages: 273  parsed and passed checks: 273  parsed with problems: 0  failed to parse: 0`, 962 ms. Versions: 130-159 with licensee 1 (170 files at 159), plus one file each at 99/0, 106/0 and 121/28.
- **Still broken / not tested:** Object contents are still not decoded. Not tested: which three files carry the odd versions 99, 106 and 121.
- **Next:** Decode one object type.

## 2026-10-29: First data loader, reads game packages (rc_probe)

- **Changed:** Added a Cargo workspace (`Cargo.toml`) with two crates: `crates/rc_package` (reads Unreal Engine 2 package headers and tables, no Bevy) and `crates/rc_probe` (command-line tool that opens packages read-only and logs what it read to `logs/rc_probe.log`). Added `README.md`, `docs/DESIGN.md` and `docs/RESEARCH.md` (what exists for this game: format docs, UE Viewer, the CT headers, licenses).
- **Why:** First step of the rewrite: prove the game's data can be read from the user's install.
- **Tested how:** 7 unit tests on made-up packages built in memory (`cargo test`). Then copied 50 real packages from the Steam install into the cloud sandbox (read-only copy, outside the repo, never committed) and ran `rc_probe` on them: all 36 `.u` script packages, 3 maps, 3 texture, 3 sound, 3 static mesh and 4 animation packages.
- **Result:** 50 of 50 parsed and passed every consistency check, in 33 ms total. `core.u`: package version 159, licensee 1, 408 names, 17 imports, 809 exports; the first export's data starts exactly where the export table ends (byte 23692). Versions seen: 135, 138, 140, 145, 154, 156, 157, 158, 159, so both the standard and the Republic Commando export layouts were exercised. `vehicles.ukx`, which an old UModel forum thread said failed, reads fine.
- **Still broken / not tested:** Not tested on the user's Windows PC (run there by the cloud sandbox only). Not tested: the other 223 packages in the install. Object contents (textures, meshes, sounds, levels) are not decoded yet. The extra export field from version 159 has unknown meaning.
- **Next:** Run the full scan on the user's PC, then decode one object type (probably textures or the script text).

## 2026-10-29: Switch .gitignore to a whitelist

- **Changed:** Replaced `.gitignore` (GitHub's standard Rust list) with a whitelist. It ignores everything, then allows only `.gitignore`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `*.rs` and `*.md`. It never looks inside `target/`, `extracted/` or `game-data/`.
- **Why:** Project rule 2 says `.gitignore` must be a whitelist, so game files can never be committed by accident.
- **Tested how:** Made a throwaway git repo, copied in this `.gitignore`, created fake files of each kind, ran `git add -A` and checked what git picked up.
- **Result:** Tracked only `.gitignore`, `Cargo.toml`, `Cargo.lock`, `README.md`, `MODLOG.md`, `docs/DESIGN.md`, `src/main.rs`. Ignored `target/debug/build/gen.rs`, `extracted/level.rs`, `extracted/notes.md`, `game-data/Map.ctm`, `assets/clone.utx`, `tex.dds`, `save.sav`, `run.log`, `secrets.env`.
- **Still broken / not tested:** Not tested in a real Rust project yet. New kinds of source files (for example shader files) will need their own line added.
- **Next:** Set up the empty Rust and Bevy project.

## 2026-10-29: Project setup and MODLOG started

- **Changed:** Created `MODLOG.md` (this file) and `MODLOG-template.md`. No code yet.
- **Why:** The project rules require a log entry after every change, so the log has to exist before any work starts.
- **Tested how:** Not applicable. These are text files only. Nothing was run.
- **Result:** Both files added to the GitHub repository `IronKoala/SW-Republic-Commando-Rust-rewrite-and-Bevy-port` on the branch `add-modlog`. Not committed yet.
- **Still broken / not tested:** Not tested: nothing to run yet. `README.md` and `docs/DESIGN.md` do not exist yet. The repository's `.gitignore` is GitHub's standard Rust ignore list, not the whitelist the project rules require.
- **Next:** Replace `.gitignore` with a whitelist, then set up the empty Rust and Bevy project.
