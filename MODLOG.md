# MODLOG

Running record of changes to the Star Wars: Republic Commando Rust and Bevy
rewrite. Newest entries first. Format and rules are in `MODLOG-template.md`.

## 2026-10-29: Collision against triangles, ready for real levels (rc_movement)

- **Changed:** New `crates/rc_movement/src/triangles.rs`: `TriangleWorld`, a `World` made of loose triangles (what the level reader produces for BSP polygons and static meshes), filed in a 256-unit grid so each sweep only looks at nearby triangles. The plane sweep in `world.rs` was split out so both worlds share it. DESIGN updated.
- **Why:** Real levels are triangles, not boxes. This lets the movement code walk a map once `rc_level` (draft PR #8) is merged, without changing the movement code.
- **Tested how:** 3 new unit tests (20 in rc_movement, all passing): a box dropped on a single triangle, and the test room cut into triangles run lane by lane next to the original box version (30 and 40 steps, stairs, both ramps, low ceiling standing and crouched).
- **Result:** Landing on a triangle hits at exactly the expected point (time 0.5). In every lane, the player ends within 1 uu of where the box version puts it.
- **Still broken / not tested:** Not tested on a real map yet (needs PR #8), and speed with 141,477 triangles (geo_01a) is not measured. Not tested on Windows.
- **Next:** After PR #8 merges: load geo_01a's collision triangles into `rc_walk` and walk it, logging speed per step.

## 2026-10-29: First-person movement with the game's own numbers (rc_movement, rc_walk)

- **Changed:** New crate `crates/rc_movement` (player walking, running, strafing, jumping, falling, crouching, step and slope handling, box collision; plain Rust, no Bevy) and `crates/rc_walk` (Bevy 3D program: first-person test room, keyboard/mouse or `--script` input). New settings file `data/movement/player_commando.toml`, and a `.gitignore` line allowing `data/**/*.toml`. Workspace `Cargo.toml` and `Cargo.lock` updated. Research written up in `docs/RESEARCH.md` ("Player movement"); design in `docs/DESIGN.md`; README updated.
- **Why:** Roadmap step 5: walk a level in first person. Built against a test room first, since level loading is being done in parallel.
- **Tested how:** Read the player's class defaults (PlayerCommando and its parents) from `ctcharacters.u`, `ctgame.u` and `engine.u`, copied read-only from the user's install into the cloud sandbox, with a throwaway reader outside the repo; read the controls from `user.ini`/`defaultctrl.ini` and the movement rules from the script text in the packages. 17 unit tests in `rc_movement`. Ran `rc_walk --script` in the sandbox (Xvfb, software rendering); it drives the player through the test room and checks 21 results. Also ran it with a misspelt settings file, and in keyboard mode for 30 frames.
- **Result:** Script: `SCRIPT DONE: 21 checks passed, 0 failed`. Run 450.00, walk 225.00, strafe 337.50, back 270.00, crouched 225.00 uu/s; stopped below 10 uu/s within 0.3 s (1.69); jump 102.56 high (expected 475²/2200 = 102.6); 30-high step climbed (feet 30.10), 40-high step blocked (0.10) and jumped onto (40.08); stairs 128.10; 30° ramp 200.10; 50° ramp not climbable; low ceiling blocks standing, crouching goes under and stays crouched; 700-high fall lands at 1241.02 uu/s (expected 1241) with 3.42 damage (expected 3.4). Camera matched the eye every logged frame. First version of the fall check failed because the small landing right after spawning was counted; fixed by taking the hardest landing, and landing speed is now measured at the moment of contact (it was 1246.7, off by one frame of gravity). A bad settings file stops with `line 2: unknown setting `jump``, exit code 1.
- **Still broken / not tested:** Not tested on Windows (not built there; the Windows build can't be checked from the sandbox). Not tested by hand: nobody has walked it with a real keyboard and mouse. Guesses, because the game does them in compiled engine code: diagonal blend of the back/side speed limits, braking (Unreal Engine 2 style; the game's DecelRate of 600 is unused), mouse turning speed. Fall damage is reported, not applied. Only test-room collision; no level collision yet.
- **Next:** Collision against real level polygons and static mesh triangles once `rc_level` lands, then walking a real map.

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
