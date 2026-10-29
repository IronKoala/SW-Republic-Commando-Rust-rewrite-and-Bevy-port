# MODLOG

Running record of changes to the Star Wars: Republic Commando Rust and Bevy
rewrite. Newest entries first. Format and rules are in `MODLOG-template.md`.

## 2026-10-29: Walking a real level, geo_01a (rc_walk)

- **Changed:** New `crates/rc_walk/src/level.rs` loads a map's collision (BSP polygons plus every visible static mesh prop, via `rc_level::collision`) and its PlayerStart. `rc_walk` gained `--map NAME|path.ctm`, and `--script` with a map walks a square (with a jump and a crouch) and checks it stays on the ground. Fix in `rc_movement/src/world.rs`: a move that only grazes a surface no longer counts as hitting it, with a new unit test (`full_jump_while_pushing_into_a_wall`). README and DESIGN updated.
- **Why:** Roadmap step 5: walk a real level in first person.
- **Tested how:** 21 unit tests in rc_movement. In the cloud sandbox (Xvfb, software rendering, debug build), `rc_walk --script` (test room) and `rc_walk --map geo_01a --script` with `geo_01a.ctm` and its static mesh packages copied read-only from the user's install.
- **Result:** Room: `SCRIPT DONE: 21 checks passed, 0 failed`. geo_01a: loaded in 152 ms (249 BSP nodes, 355 props, 0 unreadable), collision grid built in 43 ms, `SCRIPT DONE: 3 checks passed, 0 failed`. Physics took 0.37 ms per frame on average, 5.2 ms at worst. The first walk found a bug: jumping while pressing into a wall rose only 87 of 103 units, because each slide along the wall "hit" the wall again. After the fix the jump on the map peaks about 101 units up in the log's samples (taken every 6 frames; the unit test gets 102.6). The PlayerStart is about 690 units up, inside the gunship, which isn't solid yet, so the player first drops to the floor (lands at 1154 uu/s, no damage).
- **Still broken / not tested:** Not tested on Windows or by hand with keyboard and mouse. Not solid yet: BlockingVolumes (27 in geo_01a), movers and vehicles. Per-prop collision settings are ignored. The map is drawn plain grey (no textures or lights in rc_walk). Only geo_01a tried.
- **Next:** BlockingVolumes, then sharing the textured, lit level drawing with `rc_level_viewer`, then the blaster.

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

## 2026-10-29: Level viewer passes on Windows

- **Changed:** No code changes. Updated `README.md` with the result.
- **Why:** The level viewer had only been tested in the cloud sandbox.
- **Tested how:** The user ran `cargo run --release -p rc_level_viewer` on their Windows PC and flew around; Claude read `logs\rc_level_viewer.log` from the PC.
- **Result:** 249 package files found; BSP checks all passed; 1,209 actors, 355 props, 141,477 triangles; 56 of 56 materials textured; floor test 292 of 293 (0 too close); level built in 246 ms; on-screen check PASS on the first capture (580 colours); steady 144 frames per second while the camera moved about 30 Bevy units.
- **Still broken / not tested:** Not tested: maps other than `geo_01a`. Still unlit, no collision.
- **Next:** Lights and lightmaps (roadmap step 2).

## 2026-10-29: Load a whole level and fly around it (rc_level, rc_level_viewer)

- **Changed:** New crate `crates/rc_level` (no Bevy): `bsp.rs` reads a map's BSP model (the level's own walls and floors: vectors, points, nodes, surfaces, vertex pool) and turns each node into a polygon with texture coordinates; `actors.rs` reads the `Level` object (actor list, map URL, BSP model) and each actor's placement (Location, Rotation, DrawScale, DrawScale3D, PrePivot, StaticMesh, Skins, bHidden), plus the Unreal rotation maths; `collision.rs` gives world-space collision triangles (solid BSP plus placed static meshes, each with its source) for the first-person thread. 3 unit tests. New program `crates/rc_level_viewer`: loads one map (default `geo_01a`), draws the BSP and every placed static mesh with their textures, batched by material, with a free-fly camera starting at the PlayerStart. Logs to `logs/rc_level_viewer.log`. README and DESIGN updated.
- **Why:** Roadmap step 1: load a whole level and fly around it.
- **Tested how:** Copied 5 maps and the 15 packages `geo_01a` uses (read-only) into the cloud sandbox. Read the BSP and all actors of all 5 maps. Ran the viewer on `geo_01a` 4 times with Xvfb and software rendering. New checks: every BSP index in range, every polygon corner on its plane, the arrays end where the next begins; a "floor test" (PathNodes and PlayerStarts must have a floor a little below them and nothing inside them); an on-screen check (how much of the window is level, how many colours).
- **Result:** 5 of 5 maps: BSP checks all pass, all 3,042 actors read. `geo_01a`: 249 BSP polygons, 355 static mesh actors (60 different meshes), 141,477 triangles in 60 draw batches, 55 of 56 materials textured (the missing one's package wasn't copied). Floor test 292 of 293 points have a floor 0-150 units below (median 104), 0 closer than 80. Deliberately turning props the wrong way made 14-20 points land inside props, so the test notices. On-screen check PASS 4 of 4: 100% of the window covered, 576 colours.
- **Still broken / not tested:** Not tested on Windows, or on maps other than `geo_01a` in the viewer. Not looked at by eye: the scene is only checked by numbers. Drawn unlit (no lights or lightmaps), no bump maps, no sky box treatment, no movers, characters, emitters or terrain. Two dead ends logged: a first state-frame layout that only fitted StaticMeshActors (the lighting thread found the right one: a 2-byte field, not 4), and on-screen captures that showed only the background colour (now retried like black ones). The real-world scale (50 Unreal units per Bevy unit) is still to be measured against a character.
- **Next:** Test on the user's PC; then lights and lightmaps (roadmap step 2).

## 2026-10-29: Fix the pink clone trooper in rc_character

- **Changed:** `crates/rc_character/src/main.rs`: the camera now uses `Tonemapping::None` (and `DebandDither::Disabled`). `crates/rc_character/src/check.rs`: the on-screen check now also compares the colour balance on screen with the skin picture's.
- **Why:** The user ran `rc_character` on Windows: the clone moved correctly but was pink. Cause (my mistake): when I switched `rc_character` to the same Bevy feature list as `rc_viewer` to share one build, I dropped `tonemapping_luts`. Bevy's default colour grading needs those tables and draws every lit pixel magenta without them. The outline check only compared shapes, so it still passed.
- **Tested how:** Read `logs\rc_character.log` from the user's PC: `character pixels' average colour R 253 G 1 B 254`, outline match 0.989 PASS. Reproduced it in the cloud sandbox (R 253 G 1 B 253, plus Bevy's log error "TonyMcMapFace tonemapping requires the `tonemapping_luts` feature"). Then fixed it and re-ran.
- **Result:** Clone R 167 G 166 B 165 on screen vs skin R 142 G 139 B 137; colour balance difference 0.005 (BattleDroid 0.013, SuperBattleDroid 0.001), all PASS. With the fix removed on purpose, the new check reports 0.331, `colour WRONG`, FAIL, exit code 1.
- **Windows re-run:** The user re-ran it on Windows ("he's beautiful"). Their log: `colour balance differs from the skin's by 0.005`, `on-screen check PASS: outline match 0.993 ..., colour ok`.
- **Still broken / not tested:** Nothing new.
- **Next:** Merge PR #9.

## 2026-10-29: Characters: skeletal meshes, animations, and an animated clone trooper in 3D

- **Changed:** New readers `crates/rc_package/src/skeletal.rs` (SkeletalMesh: bones, points, triangles, texture coordinates, bone weights) and `crates/rc_package/src/animation.rs` (MeshAnimation: sequences, packed key tracks). New crate `crates/rc_anim` (no Bevy): poses a skeleton from a sequence and moves the mesh's points with it (skinning). New `rc_probe --characters` mode (`crates/rc_probe/src/characters.rs`, logs to `logs/rc_characters.log`; `--meshes` is the static-mesh mode from PR #4). New 3D Bevy program `crates/rc_character`: shows one character with its skin texture, playing one animation, with an on-screen self-check. Small shared edits: `Reader::i16` in `reader.rs`, two `pub mod` lines in `rc_package/src/lib.rs`, workspace members in `Cargo.toml`.
- **Why:** Roadmap step 3: load a character with animation.
- **Tested how:** 6 new unit tests (all 23 unit tests in the workspace pass). Copied the 20 animation packages plus `clonetextures.utx` and `droidtextures.utx` from the install into the cloud sandbox and ran `rc_probe --characters` on them and `rc_character` under a virtual screen (software rendering). Not run on the user's PC yet.
- **Result:** `skeletal meshes: 131 read, 0 with problems, 0 failed`; `animation sets: 165 read, 2 with problems, 0 failed; 1414 sequences, 127873 frames`. Rotation convention checked by playing every sequence of every mesh (226 mesh/set pairs, 8 frames each): average edge stretch 0.035 with the convention UE Viewer describes, 0.857 with the opposite one. CloneTrooper: 65 bones, 1556 points, 3008 triangles; WalkForward 48 frames at 30 fps, height 169.1-174.2 units throughout, root moves 364.7 units per loop (removed for display). `rc_character`: `on-screen check PASS: outline match 0.991` for the clone (0.993 on 6 of 6 runs of RunForward), 0.988 for BattleDroid, 0.994 for SuperBattleDroid; comparing against a pose 22 frames off gives 0.773 and 0.858, so the 0.95 pass mark catches a wrong pose.
- **Still broken / not tested:** Not tested on Windows (needs one run on the user's PC). The 2 "problem" animation sets are real data: an empty sequence named `None` with 0 frames (`SecCameraASet`, `TranTurretSet`). 29 bytes at the end of every mesh are not understood (UE Viewer skips them too). Only the skin's colour picture is drawn: bump maps, shininess and the material's other layers are ignored. Only the first material slot is used (the clone has one; some hud arms have up to 7). Animation notifies (footstep sounds, gun fire) are read but not used. Skinning runs on the CPU. Pitch and roll of the mesh's stored rotation are ignored (all zero in the files tested).
- **Next:** Run `rc_character` on the user's PC. Later: GPU skinning through Bevy's own skinned meshes, more material slots, blending between animations.

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
