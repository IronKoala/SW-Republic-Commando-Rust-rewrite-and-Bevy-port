# How the project works

Plain-language notes on the design. Updated when the shape of something changes.

## Layout

The code is a Cargo "workspace": several small Rust libraries and programs
("crates") in `crates/`, built together. The idea comes from IW4L and benilla.

| Crate | What it does |
|---|---|
| `rc_package` | Reads Unreal Engine 2 packages from bytes it's given. Opens no files. No Bevy. |
| `rc_image` | Turns raw pixel data (P8, DXT1/3/5, RGBA8, G16) into plain RGBA pictures and writes PNG files. Knows nothing about Unreal. No Bevy. |
| `rc_probe` | Small command-line tool that opens packages from the game folder and logs what it read. |
| `rc_audio` | Reads `.wav` files into samples and measures them (length, peak, loudness, checksum). Knows nothing about Unreal. No Bevy. |
| `rc_viewer` | The Bevy program: opens a window and shows one texture, or one 3D model with `--mesh`. |
| `rc_anim` | Poses a skeleton from an animation and moves a mesh's points with it ("skinning"). Plain maths, no Bevy. |
| `rc_character` | 3D Bevy program: shows one character from the game with its skin, playing one animation. |
| `rc_soundtest` | Bevy program with no window: plays one weapon sound and one ambient loop and logs proof that they played. |

Format readers stay plain Rust with no Bevy in them, so they can be tested alone.
Only `rc_viewer`, `rc_character` and `rc_soundtest` use Bevy (version 0.20.1,
MIT or Apache-2.0). Each turns on only the parts of Bevy it needs, which keeps
the build smaller: `rc_viewer` and `rc_character` a 2D window plus `bevy_pbr`
for 3D models and their materials (the same list, so they share one build);
`rc_soundtest` only audio and WAV decoding, with no window.

## Finding the game

The game folder comes from the `RC_GAME_DIR` environment variable, or defaults to
`C:\Games\Steam\steamapps\common\Star Wars Republic Commando`. Programs only ever
read from it. Logs go to `logs/` in the folder you run from, which `.gitignore`
never tracks.

## Unreal packages

Every game data file (`.u` scripts, `.ctm` maps, `.utx` textures, `.uax` sounds,
`.usx` static meshes, `.ukx` animations) is an Unreal "package":

1. **Header** (64 bytes in this game): the magic number `C1 83 2A 9E`, the version,
   and where the three tables are.
2. **Name table**: every name used in the file (like `Core`, `Vector`, `ScriptText`).
   Everything else refers to names by their number in this list.
3. **Import table**: objects this package uses from other packages.
4. **Export table**: objects stored in this package, each with its class, name and
   the byte range of its data.

Object references are one number: positive means "export number N-1", negative
means "import number -N-1", zero means none.

Many numbers are stored as a "compact index": a variable-length integer of 1 to 5
bytes, small numbers take 1 byte.

### What's special in Republic Commando

The stored version has two halves: the package version (low 16 bits) and a
"licensee" version that the game studio sets (high 16 bits). Republic Commando
uses licensee version 1. A full scan of the Steam install (273 packages) found
package versions 130 to 159 with licensee 1, mostly 157-159. Three odd files
use other engines' versions (99 and 106 with licensee 0, and 121 with licensee
28); they still read fine with the standard UE2 layout.

From package version 151, the export table layout differs from standard UE2
(found by UE Viewer):

- Data size and offset are plain 4-byte numbers, always present, instead of
  compact indexes.
- From version 159 there is one extra compact index after the package index.
  Its meaning is unknown; in `Core.u` it is 0 for the exports we looked at.

Older packages (135-145) use the standard UE2 layout. Both are handled, and both
were tested on real files.

### How we know the reading is right

`Package::check()` checks every name is readable text, every reference points
inside the tables, every export's data lies inside the file, and the header's
"generation" counts match. In `Core.u` the first export's data starts at exactly
the byte where the export table ends, which is what we expect if the table was
read correctly.

## Object data: properties

Most objects' data starts with a list of "properties", the settings you'd see in
the Unreal editor (for a texture: `Format`, `USize`, `VSize`, `Palette`, ...).
Each one is the property's name, one info byte (type, size, array flag) and the
value. The list ends with the name `None`. `rc_package::properties` reads it and
decodes the simple types (byte, int, bool, float, object, name); the rest keep
their raw bytes for now.

## Textures

After the properties, a `Texture` stores its mipmaps: the picture at full size,
then half size, quarter size, and so on. Each mipmap is a "skip position" (the
file offset just after its pixels), the pixel bytes, the width and height, and
two small numbers (log2 of width and height). `rc_package::texture` reads them.

Three checks show the reading is right, and all pass on real files:
- The skip position equals where the pixels really end.
- The byte count matches what the format needs (DXT1 uses 8 bytes per 4x4
  block, DXT3/DXT5 16, P8 one byte per pixel, RGBA8 four).
- Nothing is left over at the end of the object.

Formats are UE2's numbers: 0 P8 (8-bit with a separate palette), 3 DXT1,
7 DXT3, 8 DXT5, 5 RGBA8, 10 G16 (terrain heightmaps). Republic Commando also
uses 12 and 14, both 2 bytes per pixel (measured on every mipmap). UE Viewer
guesses 14 is CxV8U8, a two-channel format used for bump maps. We found no name
for 12.

Known quirk: `MP_Trando_Light_Cluster` (in `multiplayer_textures.utx`) stores
its last mipmap, 1x1 pixels, with 0 bytes. The file is otherwise consistent, so
this seems to be how the game shipped. The check reports it rather than hiding
it.

## From texture bytes to pictures

`rc_image` turns a mipmap's bytes into RGBA (red, green, blue, alpha; 4 bytes
per pixel):

- **P8**: each byte picks one of 256 colours from the texture's `Palette`
  object (256 colours stored red, green, blue, alpha). If `bMasked` is set,
  colour 0 is see-through.
- **RGBA8**: stored blue, green, red, alpha, so the first and third bytes swap.
- **DXT1/3/5**: the picture is cut into 4x4 squares. Each square stores two
  colours and picks between them and two in-between colours per pixel. DXT3
  adds 4 bits of alpha per pixel; DXT5 adds an alpha pair with 8 steps.
- **G16**: 16-bit grey; we keep the top 8 bits.

PNG files are written uncompressed (simplest valid PNG), so they're large but
open anywhere. They go to `extracted/textures/<package>/<texture>.png`.

**How we check pictures without looking at them.** Each texture stores its own
half-size copy (mip 1), made by the game's tools. We shrink our decoded full-
size picture (mip 0) by averaging 2x2 squares and compare it with our decoded
mip 1. Correct decoding gives a small difference; unrelated pictures of the same
size differ by about 57 out of 255 on average (measured on 524 pairs). Results
across the install: 1,916 of 2,003 textures differ by under 10; 17 by over 20.
The 17 are mostly fine patterns (grates, noise, flickering screens). Their
smaller copies fade toward flat grey, which suggests the artists made those
mipmaps differently. That explanation is a guess and hasn't been checked by eye.

## Showing a texture in Bevy (`rc_viewer`)

Bevy is the game engine. A Bevy program is an "app" made of:

- **Resources**: single pieces of shared data. `rc_viewer` keeps its log file,
  the decoded texture and the frame counters as resources.
- **Entities**: the things in the scene. Here there are two: a 2D camera, and a
  "sprite" (a flat picture) using the texture.
- **Systems**: plain Rust functions Bevy calls for us. `setup` runs once at the
  start and creates the camera and the sprite. `count_frames` runs every frame.

The order of a run:

1. Before Bevy starts, `main` reads the package from the game folder and
   decodes the texture's full-size mipmap with `rc_package` and `rc_image`
   (the same code `rc_probe` uses). If that fails, it logs why and exits with
   code 1 without opening a window.
2. Bevy opens an 800x800 window with a dark grey background.
3. `setup` hands the RGBA pixels to Bevy as an `Image` (format
   `Rgba8UnormSrgb`: 4 bytes per pixel, colours in the usual sRGB scale) and
   draws it as a sprite scaled to 90% of the window (720x720), keeping its shape.
4. `count_frames` logs the window size on the first frame and the frame rate
   about once a second. With `--frames N` it closes the window after N frames.

### How we check the window without looking at it

On frame 10, `rc_viewer` asks Bevy for a screenshot of the window, kept in
memory, never saved. It then compares each screen pixel inside the sprite's
square with the texture pixel it should show. See-through texture pixels are
first mixed with the background colour, the same way the graphics card mixes
them. The result is the average difference per colour channel, from 0 to 255.
It also checks a corner pixel is the background colour.

Results in the cloud sandbox (software rendering, no graphics card):
- `DC15Blaster` (256x256 DXT1): 3.77. `WookieeMelee` (128x128 DXT1): 4.32.
  `TDetonator` (128x128 DXT5, mostly see-through): 0.55. Under 10 passes.
- The same check with the texture deliberately compared upside down gives
  22.56, so the check does notice a wrong picture. The small leftover
  difference on a correct picture comes from scaling: the screen blends
  neighbouring texture pixels, the check doesn't.

Known quirk: in the sandbox, the first screenshot is often completely black,
because the window hadn't been drawn yet. The check then tries again 10 frames
later, up to 5 times. On the user's Windows PC the first capture was fine
(3.77, PASS).

## Static meshes (3D models)

A "static mesh" is a model that doesn't bend or animate: weapons, props, and
most pieces of the levels. They live in `.usx` packages. The DC-15 blaster is
`DC15Blaster` (group `CloneWeapons`) in `GameData/StaticMeshes/weapons.usx`.
Animated models (characters, first-person arms) are a different class in
`.ukx` packages and aren't read yet.

`rc_package::static_mesh::read_static_mesh` reads one, in this order (layout
from UE Viewer, MIT):

1. The property list. `Materials` is an array of small structs, each its own
   property list. We keep each one's `Material` reference: one per section.
2. A bounding box (smallest and largest x, y, z, plus a "valid" byte) and a
   bounding sphere (centre and radius).
3. Sections: each is a run of triangles drawn with one material (first index,
   first and last vertex used, triangle count).
4. The bounding box again.
5. Vertices: position and normal, three 4-byte decimals each. A "normal" is
   the direction the surface faces at that corner, used for lighting.
6. Republic Commando adds two 4-byte numbers here (one from package version
   149, another from 155). Their meaning is unknown.
7. Vertex colours and vertex alphas (often one per vertex, sometimes empty).
8. Texture coordinate sets ("UVs"): for each vertex, where on the texture it
   sits, from 0 to 1 across and down.
9. The triangle list: three vertex numbers per triangle (2 bytes each), then
   a second index list we don't use.
10. Collision data, which we don't read yet.

Every list is a compact-index count followed by the items, and most streams
end with a 4-byte "revision" number.

**How we know the reading is right.** `StaticMesh::check` tests that every
triangle points at real vertices, the sections add up to the triangle count,
each section stays inside its vertex range, there is one UV per vertex, every
vertex is inside the stored bounding box, and normals are 1 long. If any field
were read at the wrong spot, the later lists would be garbage and these would
fail. `rc_probe --meshes` ran this on all 58 `.usx` files: all 2,447 meshes
pass. 11 of them have a few normals that are exactly (0, 0, 0), probably from
triangles with no area in the artists' tools. The check allows exactly zero.

**Axes.** Unreal uses x forward, y right, z up (a "left-handed" system).
Bevy uses x right, y up, z toward the viewer ("right-handed"). Swapping y and z
converts one to the other without mirroring the model. After the swap, all
342 of the DC-15's triangles have their corners counter-clockwise when seen
from the side their stored normal points to, which is the order Bevy treats as
the front. So the triangle list goes to Bevy unchanged.

**Materials.** A mesh's material reference is an import into a texture
package: for the DC-15 it's `WeaponTextures.CloneWeapons.DC15Blaster`, so we
open `GameData/Textures/weapontextures.utx` and decode that texture. Many
weapons use a `Shader` instead, which combines several textures. For now we
take the shader's `Diffuse` property (the main colour texture) and ignore the
rest (bump maps, shine, transparency).

## Showing a model in Bevy (`rc_viewer --mesh`)

`rc_viewer/src/mesh.rs` reads the mesh and its texture before Bevy starts,
moves the model's centre to (0, 0, 0), and spawns:

- the model, as a Bevy `Mesh` with positions, normals and UVs, and an
  "unlit" `StandardMaterial` (no lights: every pixel is just the texture
  colour);
- an orthographic camera (no perspective: far parts are drawn the same size
  as near ones) looking at the side of the model, sized so the model fills
  90% of the window. Colour adjustment ("tonemapping") and edge smoothing
  ("MSAA") are off, so screen pixels are exactly texture colours.

Once the on-screen check is done, the model turns slowly (6 turns a minute)
so you can see all sides.

### How we check the model without looking at it

On frame 10 the window is captured, as in the texture viewer. Then
`check_screen` draws the same model itself in plain Rust: it projects each
corner to the screen the same way the camera does, skips triangles facing
away, and for each pixel keeps the nearest triangle and its texture
coordinate. It compares this with the capture in two ways:

1. **Outline**: which pixels show the model at all. The score is overlap
   divided by the area either drawing covers; 1.0 is identical. Over 0.97
   passes.
2. **Colour**: on pixels well inside the model, the average difference from
   the texture colour expected there. Under 10 of 255 passes.

As a control it also scores the texture flipped upside down.

Results in the cloud sandbox (software rendering): DC15Blaster outline 0.996,
colour 1.04 (flipped: 22.88), PASS in 3 of 3 runs. DC17mBlaster 0.86,
DC15Pistol 0.32, ThermalDetonator 2.12 (flipped: 26 to 69).

Known quirk: a capture can arrive before the model is drawn, showing only the
background. That's retried like an all-black capture.
## Characters: skeletal meshes and animations

Characters (clone troopers, droids, creatures) live in the `.ukx` packages in
`GameData\Animations`. Each holds `SkeletalMesh` objects (the bodies) and
`MeshAnimation` objects (sets of animations). Readers:
`rc_package::skeletal` and `rc_package::animation`, learned from UE Viewer
(MIT); the Republic Commando differences are listed at the top of each file.

**A skeletal mesh** is:
- **Points**: corners of the surface, in the "reference pose" (the clone
  trooper stands in a T shape, arms out).
- **Wedges**: a point plus a texture position. Two triangles can share a
  point but use different spots of the texture, so they get separate wedges.
- **Triangles**: three wedges and a material slot.
- **Bones**: a tree. Each bone stores its position and rotation relative to
  its parent. The root bone's parent is stored as -1. The stored "number of
  children" counts all bones below a bone, not just direct children (found
  by measuring; the root of a 65-bone skeleton says 64).
- **Influences**: "bone B moves point P with weight W". A point can follow up
  to 3 bones (clone trooper). Weights add up to 1, except a few points in 17
  meshes that add up to 0.98-0.99; we spread the difference evenly.
- The skin: an object reference to a material in a texture package. For the
  clone that's a `Shader` whose `Diffuse` setting is a 512x512 DXT1 picture.

**An animation set** has its own bone list (matched to the mesh's bones by
name) and a list of sequences (`WalkForward`, `Reload`, ...). Each sequence
has a frame count, a speed (30 frames per second in every file seen), and
one track per bone. A track is packed small: positions are three 16-bit
numbers times a scale, rotations are three 16-bit numbers with flag bits
that rebuild a quaternion (a compact way to store a rotation), and each key
lasts a whole number of frames (one byte).

**Rotations.** Republic Commando stores bone rotations mirrored compared with
other Unreal Engine 2 games, and the engine applies them inverted except on
the root bone. Together: use the stored rotation as-is, except invert the
root's. We checked this by measurement instead of trusting it: playing every
sequence on every mesh, the surface stretches by 3.5% on average with this
rule and by 86% with the opposite one (limbs fly apart).

**Playing an animation** (`rc_anim`):
1. `Clip::sample` picks the two keys around the current frame for each bone
   and blends between them (straight line for positions, "slerp", the smooth
   rotation blend, for rotations). After the last key it blends back to the
   first, so the animation loops.
2. `globals` walks down the bone tree to get each bone's place in the model.
3. `skin` moves each point: for every bone that influences it, undo the
   bone's reference placement, apply its animated placement, and mix by the
   weights. This runs on the CPU, about 1,500 points for the clone.

The pose is a plain list of per-bone transforms, so anything can produce
one, not only the game's animations: a mod's new animation, a physics
ragdoll, or tracked VR hands could set some bones and leave the rest to an
animation.

**How we check without looking:**
- `rc_probe --characters` reads every mesh and animation set, checks the bone tree,
  the index ranges and the weights, and plays every sequence of each mesh's
  animation sets, logging the surface stretch.
- In the reference pose, each bone sits close to the points it moves (median
  1.5% of the clone's height).

### Showing a character in Bevy (`rc_character`)

1. Before Bevy starts, `load.rs` reads the mesh, the animation set and the
   sequence, plays the whole sequence once on the CPU and logs the
   character's height and surface stretch on every frame, then follows the
   material to the skin picture and decodes it.
2. Axes: the game has Z up and is "left-handed"; Bevy has Y up and is
   "right-handed". Bevy X = game Y, Bevy Y = game Z, Bevy Z = minus game X.
   Changing handedness mirrors the model, which also flips which side of a
   triangle is its front. We decide the triangle order by measuring: the
   "signed volume" of the closed surface is positive when fronts face out.
3. One Bevy vertex per wedge. Every frame `animate` samples the pose, skins
   the points, recomputes normals (the direction each bit of surface faces,
   for lighting) and copies them into the Bevy mesh. The root bone's
   sideways movement is removed so walks stay in place.
4. The game's "forward" is +X (checked: `WalkForward` moves the root bone
   +364.7 units along X, `StepLeft` moves it along -Y, and the toes point
   +X). The camera sits in front of the character, slightly to one side.

**On-screen check:** on frame 30 the animation pauses and the window is
captured. We project every triangle of our own skinned points with Bevy's
camera, fill them into a yes/no grid, and compare with the captured pixels
that aren't the background colour. The score is the overlap divided by the
combined area (1.0 = identical). Results in the cloud sandbox: 0.984-0.994
for the clone trooper, battle droid and super battle droid; comparing
against a pose 22 frames off scores 0.77-0.86, so the 0.95 pass mark means
something. In the sandbox the first capture is often black or shows only
the background (the mesh hadn't reached the graphics card yet); the check
retries up to 5 times.

**Fit with the project goals.** Readers and animation maths have no Bevy and
no platform-specific code, so they build for Windows, Linux and Android
alike (only `rc_character` needs Bevy). Skinning ~1,500 points on the CPU is
cheap, but on a phone we will want to hand it to the graphics card through
Bevy's own skinned meshes; the pose code stays the same. For mods: the
viewer takes any package, mesh, set and sequence on its command line, and
nothing assumes the clone trooper's bone names except the defaults.


## Sounds

### Where the game keeps its sounds

- `GameData\Sounds\*.uax`: 23 packages. Three classes of object matter:
  - `Sound`: holds a complete `.wav` file. After the usual property list comes
    the file type as a name (always `WAV`), then the file's bytes stored the
    same way as texture mipmaps (a "skip position" telling where the bytes
    end, then a length, then the bytes). All 7,974 are plain uncompressed PCM,
    16-bit: 3,794 at 11,025 Hz mono (mostly voices), 4,039 at 22,050 Hz mono,
    34 at 44,100 Hz mono, 106 at 22,050 Hz stereo (ambient loops), 1 at
    44,100 Hz stereo. Source for the layout: UE Viewer's `UnSound.cpp` (MIT).
  - `SoundStreamed`: holds no audio, only a file name, played from disk while
    the game runs. Layout (worked out from the game's own bytes): the property
    list (`Looping`, `Streaming`, fades), then the file name as a string.
    272 of the 285 names match an `.ogg` file in `GameData\Music`
    (case doesn't matter). The other 13 are `amb_RAS_*` ambient loops whose
    files aren't in the PC install.
  - `SoundMultiple`: a group of sounds the game picks from (for example the
    13 "whisper" music pieces). Layout (worked out from the bytes): the
    property list, a list of references to sounds, then a second list that was
    empty in all 1,208 found. Its meaning is not known yet.
- `GameData\Music\*.ogg`: 296 music files (Ogg Vorbis). Not decoded yet.

### Checking sounds without listening

`rc_audio` turns a `.wav` file into numbers: sample rate, channels, length,
peak (loudest sample, 0 to 1, also in decibels where 0 dB is the loudest
possible), RMS (average loudness), how many samples hit full scale, and a
checksum that changes if any sample changes. `rc_probe --sounds` logs these
for every sound. The only silent sound found is called `fol_GEN_silence_01`.

### Playing sounds in Bevy (`rc_soundtest`)

Bevy plays sounds from an `AudioSource`, which is just the bytes of a sound
file. We hand it the `.wav` bytes straight out of the package, so nothing is
written to disk. The program:

1. Reads `gen_sound.uax` and finds `wep_GEN_dc17Fire_02` (DC-17 rifle,
   44,100 Hz mono, 1.317 s) and `amb_KAM_siege_lp_01` (battle ambience,
   22,050 Hz stereo, 9.977 s).
2. Decodes each one twice: with `rc_audio`, and with Bevy's own decoder. It
   compares every sample. They must match exactly (they do: largest
   difference 0, same checksum).
3. Runs Bevy without a window: the ambience loops, the rifle fires every 2
   seconds. When Bevy actually hands a sound to the sound card it adds an
   `AudioSink` to it; the program logs that moment, and the ambience's play
   position each second.
4. At the end it reports `PLAYED` or `NOT PLAYED (Bevy found no sound card)`
   and the ambience speed: play position divided by time. On a real sound card
   that should be about 1.00. In the cloud sandbox there is no sound card, so
   a fake "null" one was set up; it swallows sound as fast as it can, about
   60x.

### Fitting the long-term goals

- **Easy modding.** Sounds are found by name, and Bevy is given plain `.wav`
  bytes, the same thing Bevy loads from a normal `.wav` file. So a later step
  can let a mod folder replace a game sound by dropping in a `.wav` (or `.ogg`)
  file with the same name, without touching the game's packages.
- **VR.** This test plays sounds "flat" (not placed in the world). Bevy also
  has positional sound (`SpatialListener` on the player's head, sounds placed
  on entities). The game's sounds are mostly mono, which is what positional
  sound needs. Placing sounds in the world comes with the level and player
  steps.
- **Windows, Linux and Android.** `rc_audio` and `rc_package` are plain Rust
  with no operating-system code. Bevy's audio goes through rodio and cpal,
  which talk to WASAPI on Windows, ALSA on Linux and AAudio on Android, and
  WAV decoding (hound) is pure Rust. No Windows-only audio API or codec is
  used. The sandbox runs are on Linux, so Linux is tested; building there
  needs the ALSA development package (`libasound2-dev` on Ubuntu). Windows
  and Android are not tested yet.
