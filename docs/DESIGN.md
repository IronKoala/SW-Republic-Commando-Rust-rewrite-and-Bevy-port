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
| `rc_movement` | The player's walking, jumping, crouching and collision, with the game's own numbers. No Bevy. |
| `rc_walk` | Bevy program: walk around a test room or a real map (`--map`) in first person using `rc_movement`. |
| `rc_level` | Reads maps: the level's own geometry (the BSP), its baked lightmaps, where every actor is placed, and each prop's baked corner colours. No Bevy. |
| `rc_level_viewer` | Bevy program: loads one map, lit with its baked light and real-time lights, and lets you fly around it. Also a library (`scene`, `draw`, `lighting`, `materials`) that `rc_walk` uses to draw maps. |
| `rc_anim` | Poses a skeleton from an animation and moves a mesh's points with it ("skinning"). Plain maths, no Bevy. |
| `rc_character` | 3D Bevy program: shows one character from the game with its skin, playing one animation. |
| `rc_soundtest` | Bevy program with no window: plays one weapon sound and one ambient loop and logs proof that they played. |
| `rc_squad` | Delta Squad: Sev, Fixer and Scorch following the player and obeying squad orders, with the game's numbers. No Bevy. |
| `rc_squad_plugin` | `SquadPlugin`, the squad in the Bevy game, and the `rc_squad_sim` example (a windowless squad test on a real map). |

Format readers stay plain Rust with no Bevy in them, so they can be tested alone.
Only `rc_viewer`, `rc_character`, `rc_level_viewer` and `rc_soundtest` use Bevy
(version 0.20.1, MIT or Apache-2.0). Each turns on only the parts of Bevy it
needs, which keeps the build smaller: `rc_viewer` and `rc_character` a 2D window
plus `bevy_pbr` for 3D models and their materials (the same list, so they share
one build); `rc_level_viewer` Bevy's full 3D renderer (`3d_bevy_render`);
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
uses 12 (V8U8) and 14 (L6V5U5), two bump-map formats, both 2 bytes per pixel.
How we know which is which is under "Bump maps" below.

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
- **V8U8 and L6V5U5** (bump maps): see the next section.

PNG files are written uncompressed (simplest valid PNG), so they're large but
open anywhere. They go to `extracted/textures/<package>/<texture>.png`.

**How we check pictures without looking at them.** Each texture stores its own
half-size copy (mip 1), made by the game's tools. We shrink our decoded full-
size picture (mip 0) by averaging 2x2 squares and compare it with our decoded
mip 1. Correct decoding gives a small difference; unrelated pictures of the same
size differ by about 57 out of 255 on average (measured on 524 pairs). Results
across the install's 57 texture packages: 2,026 of 2,114 textures with a usable
mip 1 differ by under 10; 17 by 20 or more.

**The 17 that don't match.** We looked at them closely with numbers (each mip
against the one before it, how varied each mip is, its average colour). Our
decoding is not the problem: the same decoder gives under 5 on the other 2,000
textures in the same formats and packages, including 159 other non-square ones,
and reading their small mips sideways or upside down only made things worse.
The game's own files store different pictures in their small mips:
- 5 in `arena_textures.utx` (`Propeller`, `HEX_grate`, `Main_Spire`,
  `Generic_Cave_Opening`, `Generic_Strips`): mip 1 is a detailed picture but not
  a shrunk copy of mip 0 (differences 20-60), and from mip 3 down every pixel is
  exactly the same grey (127, 125, 127). That exact flat grey can't come from
  shrinking a picture, so these mips were made by hand or by a tool setting.
- `grndDSBlastDoor02` (`assaultship_textures.utx`): every mip differs from the
  one before by 48-73, as if each level were a separate picture.
- 7 tiny glow sprites in `effecttex.utx` (8x8 and 16x16, like `eng_glowblue`):
  differences of 21-26. At those sizes a DXT block covers a quarter of the
  picture, so compression error alone may explain it. That part is a guess.
- `console_display_05`, `PowerBar` and `DigiNoise`: screens and noise. For
  `DigiNoise`, mips 1 and below agree with each other (about 5) but not with
  mip 0, so mip 0 is a different noise pattern.
- `CT_ButtonFocus` and `CT_ButtonDefault` (`guicontent.utx`, 16x32 menu
  buttons): mip 1 is noticeably brighter than mip 0.
What this means for the port: use the game's own stored mips rather than
making new ones, so things look the way the game made them look at a distance.

## Bump maps (formats 12 and 14)

A bump map (also "normal map") stores, for each pixel, which way the surface
tilts, so flat walls catch light as if they had dents and rivets. 133 of the
game's textures are bump maps, in two layouts that Direct3D (the Windows
graphics system the game ran on) names:

- **Format 12 = V8U8** (29 textures): two bytes per pixel, the sideways tilt U
  and the up/down tilt V, each a signed number from -128 to 127.
- **Format 14 = L6V5U5** (104 textures): one 16-bit number per pixel. Bits 0-4
  are U and bits 5-9 are V (signed, -16 to 15), and bits 10-15 are a brightness
  value L (0-63). What the game used L for isn't known; some of these textures
  are named `..._spec`, so it may be shininess. We keep it in the alpha channel.

`rc_image` turns both into the usual normal-map colours: red = U, green = V,
blue = how much the surface faces straight out, all mapped from -1..1 to
0..255. A flat pixel becomes (128, 128, 255), the familiar light blue.

**How we found out, without documentation.** UE Viewer guesses format 14 is
CxV8U8 and marks it "not verified"; nothing names 12. So we tried six possible
layouts on every bump map and used the mipmap test from above: decode mip 0 and
mip 1 with each layout, shrink mip 0, and compare. The right layout gives a
small difference compared with a deliberately mismatched pair. Results
(difference divided by the mismatched difference, smaller is better):

| Layout tried | Format 12 | Format 14 |
|---|---|---|
| V8U8, signed bytes | **0.06** | 0.34 |
| two unsigned bytes | 0.36 | 0.28 |
| L6V5U5 | 0.76 | **0.08** (L alone: 0.01) |
| RGB565, ARGB4444, other bit orders | 0.47-0.76 | 0.37-0.51 |

Each format has one clear winner. With these layouts all 133 bump maps decode;
for the 121 that have a mip 1, the median mip difference is 1.8 and only one is
over 10 (11.2). Not checked: which way "up" points for V. Bevy may need its
`flip_normal_map_y` setting, which can only be told by lighting a bump-mapped
wall and comparing with the game.

## Lights in maps

A map (`.ctm`) stores its lights as actors, objects placed in the level:
`Light` (most), `Spotlight`, `Sunlight`, `TriggerLight` (can be switched on and
off) and a few game-specific ones such as `K_POWSpotLight`. Each `ZoneInfo`
(one per room-like area) adds a flat "ambient" light inside it.

**Actors have an extra header.** Before their property list, actors store a
"state frame" (where their script was running). We measured its layout in
Republic Commando: a compact number, another compact number, 8 bytes, 2 bytes,
and one more compact number if the first one isn't 0. This read all 600
light, zone and level actors in four maps with 0 bytes left over; the usual UE2
layout (4 bytes in place of the 2) did not fit. The level thread's actor reader
(`rc_level`) uses the same layout.

**What a light stores** (`rc_package::light`):
- `Location` (Unreal units: X forward, Y right, Z up) and `Rotation`
  (pitch, yaw, roll, 65536 per full turn) for spotlights and sunlight.
- `LightBrightness`: a float here (UE2 used 0-255; values up to 500 were seen).
- `LightHue` and `LightSaturation`, 0-255. Hue goes round the colour wheel.
  Saturation works backwards: 255 is white, 0 is the pure colour.
- `LightRadius`: how far the light reaches. Also a float here, and in plain
  Unreal units: the typical radius (350-1,200) is about one to three times the
  distance to the nearest other light (230-840). UE2's rule of 25 x (radius +
  1) would make lights reach 20 times further than their spacing, so Republic
  Commando must have dropped it.
- `LightRadiusInner`: Republic Commando only. Meaning not known yet.
- `bDynamicLight`: whether it lights moving things while playing.

Properties the editor left at their default aren't stored. We use UE2's
documented defaults for those (brightness 64, saturation 255, radius 64); the
game's own defaults in `Engine.u` haven't been read yet.

| Map | Lights (point / spot / sun) | Dynamic | Zones with ambient |
|---|---|---|---|
| `geo_01a` | 29 / 0 / 2 | 2 | 4 |
| `pro` | 211 / 0 / 0 | 19 | 7 |
| `ras_01a` | 282 / 4 / 0 | 7 | 7 |
| `yyy_01b` | 29 / 1 / 0 (7 switched off) | 2 | 7 |

**Most of the lighting is baked.** Only 2-9% of lights are dynamic. The rest
were worked out when the level was built and saved into the map:
- **Static meshes** (props, most walls): each `StaticMeshActor` has a
  `StaticMeshInstance` holding one colour per mesh corner (vertex). Layout: an
  empty property list, a count, then that many 4-byte colours (blue, green,
  red, alpha). It read correctly on all 1,267 instances in the four maps. A
  few bytes after the colours (4 + 6 x n) are not understood yet. Many of the
  colours are black (60-80%), so the game probably brightens them when
  drawing (UE2 often doubled vertex light). The actor points to its instance
  with a `StaticMeshInstance` property. In `geo_01a` all 355 drawn props have
  one, with exactly one colour per mesh corner.
- **BSP geometry** (the level's brush-built walls and floors): lightmaps
  stored at the end of the level's `Model` object. See "Baked light on the
  walls: lightmaps" under Levels.

## Lighting in Bevy (`rc_level_viewer/src/lighting.rs`)

Bevy lights are physically based (lumens, lux), the game's are not, so the
conversion has tunable factors. All of them sit in one `LightingSettings`
resource, and a plain text file can change them (`key = value` lines, `#`
comments) so modders can tweak lighting without touching code. The level
viewer reads `lighting.cfg` from the folder it runs in, or the file given
with `--lighting FILE`; every key is optional:

```
# lighting.cfg
baked_light = true              # false draws everything at full brightness
lightmap_gain = 2               # walls: texture x lightmap x this
vertex_colour_gain = 2          # props: texture x corner colour x this
units_per_bevy = 50             # Unreal units per Bevy unit (project-wide rule)
live_lights = dynamic           # all, dynamic, or none
lumens_per_range_squared = 2000 # point and spot light power
sun_lux_at_255 = 10000          # Sunlight brightness
ambient_at_255 = 2000           # zone ambient brightness
shadows = false
```

- Positions: Bevy (x, y, z) = Unreal (x, z, y) / 50. Swapping Y and Z also
  turns Unreal's left-handed axes into Bevy's right-handed ones. The other
  threads use the same rule.
- Point light power = factor x (brightness / 255) x range². Range is the
  game's radius in Bevy units. Scaling with range² means a light looks the
  same at the same fraction of its reach, big or small.
- Spotlight cone: we guess `LightCone` 0-255 spans 0-90 degrees. Not checked.
- Ambient: Bevy has one ambient light for the whole scene, the game has one
  per zone. For now we take the dimmest non-zero zone (the bright 192-255 ones
  are skies).

**How baked light is drawn.**
- Walls (BSP) use Bevy's own lightmap support: each lightmap page becomes
  an image, each wall gets a second set of texture coordinates pointing into
  its page (`ATTRIBUTE_UV_1`) and a `Lightmap` component. The material is
  lit, so real-time lights add on top. The scene-wide ambient light is off,
  because the baked light already contains each zone's ambient.
- Props use their per-corner colours as Bevy vertex colours on an unlit
  material, which multiplies texture and colour directly. Real-time lights
  don't reach them yet (see below).
- Surfaces without baked light (marked unlit by the level designer, skies,
  and the 33-327 BSP polygons per map without a lightmap) are drawn at full
  brightness, as before.

**Brightness.** The game multiplied texture by light on its own brightness
curve (roughly "gamma 2.2") and, like other engines of its time, drew baked
light at double strength, so a stored 128 means "normal". The pages fit
that: in all four maps almost every texel is below 128 (the brightest few
reach 133-255). Bevy multiplies in linear light, so doubling on the game's
curve becomes x2^2.2 (about 4.6) in linear light; `LightingSettings::
linear_gain` does that conversion, and the viewer also undoes the camera's
exposure for lightmaps, so gain 1 shows a texture exactly as stored. The
lightmaps and textures are both marked sRGB so Bevy's multiply on screen
equals the game's. Gain 2 is the era's convention, not yet compared with the
game running.

The real-time light factors are still placeholders. The plan to set them
properly: compute what our lights would give at each static-mesh vertex and
compare with the colours the game baked there, then adjust until they agree.

**Speed (and VR).** The viewer uses `live_lights = dynamic` by default: the
91-98% of lights that are baked cost nothing at run time, and only a handful
of real-time lights remain (2 in `geo_01a`, 19 in `pro`). That suits a later
VR version, which has to draw everything twice (once per eye) at 90 frames
per second or more.

**Known gaps.** Real-time lights don't light props yet (their baked colours
are on an unlit material). Shadow bitmaps for the dynamic lights are read
past but not used. What the lightmap pages' alpha channel holds is unknown
(it is ignored). Lightmaps are sampled with plain linear filtering.

**Phones and other platforms.** Textures, bump maps included, are decoded to
plain RGBA on the CPU before Bevy sees them, so no graphics card has to
understand the game's DXT compression. Android GPUs often can't, so this path
works on Windows, Linux and Android alike. The cost is memory (4 bytes per
pixel instead of 0.5-1); re-compressing into a phone-friendly format (ETC2 or
ASTC) can come later if memory gets tight. Baked lighting with few real-time
lights is also the cheap option for phone GPUs.

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
## Player movement (`rc_movement`, `rc_walk`)

### The numbers

All movement numbers are the original game's, read from its script packages
(see `docs/RESEARCH.md`, "Player movement"). They live in a settings file,
`data/movement/player_commando.toml`, so a mod can change them without
touching code. Each line is `name = number`; a misspelt name or a bad number
stops the program with the line number, so a mistake in a mod is never
silently ignored. If the file is missing, the built-in values (the same
numbers) are used. A unit test checks the file and the built-in values agree.

Distances are Unreal units (uu). The player's collision box is 80 uu wide and
168 uu tall, so 1 uu is roughly 1.1 cm.

### How a step works

`Pawn::step(dt, input, world)` moves the player by `dt` seconds. Long frames
are cut into steps of at most 0.05 s. Each step:

1. **Push.** The input's forward/strafe and facing give a direction. The push
   is always full acceleration (1024 uu/s²), because the game's keys give more
   than the maximum.
2. **Jump** if the jump key went down this step, the player is on the ground
   and not crouching: upward speed becomes 475.
3. **Crouch.** Holding crouch on the ground shrinks the box from 84 to 56
   half-height, keeping the feet in place. Letting go stands up only if there
   are 56 uu of free space above the head.
4. **Walking physics** (on the ground):
   - Top speed: 450, times 0.5 if walking, times 0.5 if crouched, times a
     direction ratio: 1 forwards, 0.75 sideways, 0.6 backwards. For diagonals
     we blend along an ellipse through those values. **Guess:** the game does
     this in its compiled engine and we don't know its blend.
   - With a push: friction (8) turns the current speed toward the push
     direction, then the push adds speed up to the top speed. Without a push:
     friction brakes the speed by 2 × 8 = 16 per second, so a 450 run slows
     below 10 uu/s in about a quarter second and slides about 28 uu. **Guess:**
     this is Unreal Engine 2's documented behaviour. The game also stores a
     DecelRate of 600 whose use we don't know, so it is not used.
   - Moving into something: on floor-like slopes (normal Z ≥ 0.7) the player
     follows the slope at the same speed across the ground. On a wall it tries
     to step up: go up 35, forward, back down onto floor. If that fails it
     slides along the wall.
   - After moving, it looks for floor up to 35 below and snaps onto it (so
     stairs and slopes going down keep the feet on the ground). No floor means
     falling.
   - The speed afterwards is the distance really moved, so running into a wall
     stops you, like in the engine.
5. **Falling physics:** gravity -1100 and 35% steering, which can't push you
   past running speed. The move uses the average of the old and new speed,
   which makes jump heights exact at any frame rate (103 uu high, 0.86 s in
   the air). Landing on floor-like ground switches back to walking and
   reports the game's falling damage (not applied to health yet).
6. **Eye height** eases toward 60 (44.8 crouched) at the game's rate and
   absorbs steps and crouching so the camera doesn't jump.

### Collision

Like Unreal Engine 2, the player collides with the world as an upright box.
The movement code only asks the world one question, through the `World`
trait: "slide this box from A to B: where does it first touch, and which way
does that surface face?". The test room (`rc_movement::room`) answers it with
convex solids: each solid is a set of flat planes, and each plane is pushed
outward by the box's size so the box can be treated as a point. Real levels
will answer the same question with the level's polygons and static mesh
triangles (from the level-loading work), without changing the movement code.

Real levels use `TriangleWorld`: loose triangles, each treated as a very
thin solid. To be exact for a box, each triangle's solid has its face (both
ways), one side per edge, its bounding box's six sides, and one side per
edge-and-axis pair. Triangles are filed in a grid of 256 uu columns, so a
sweep only checks the ones nearby. The test room cut into triangles gives the
same results as the box version, within 1 uu.

A 0.1 uu gap is kept between the box and surfaces, so rounding errors can't
leave it stuck inside a wall. A move that only grazes a surface (pushes into it by less than
1/10000 of its length) doesn't count as hitting it. Without that, sliding
along a wall kept "hitting" the same wall, and a jump while pressing into a
wall rose only 87 of its 103 units.

`rc_walk --map NAME` draws the map with the level viewer's own code
(`rc_level_viewer` is also a library: `scene::build` makes the draw batches,
`draw::spawn_level` turns them into Bevy meshes, materials and lights), so a
level looks the same in both programs. For collision it builds a
`TriangleWorld` from the same map (`rc_walk/src/level.rs`):
the BSP's polygons plus every visible static mesh prop, cut into triangles by
`rc_level::collision`, and puts the player at the first PlayerStart (its
centre is the start point, so feet = start z - 84). Not yet solid:
BlockingVolumes and movers and vehicles (the gunship you start in). The
game's per-prop collision settings are ignored: every visible prop is solid.

### The test room

`rc_movement::room::test_room()`: a floor with walls, and side-by-side lanes
starting at x = 1000: a low ceiling (130) to crouch under, steps of 30 and 40,
stairs (8 steps of 16), ramps of 30 and 50 degrees, and a 700-high tower to
fall from. The unit tests and `rc_walk --script` both run through it.

### `rc_walk`: the Bevy side

The program is built from separate pieces that only talk through shared data,
so any one can be replaced:

| Piece | Reads | Writes |
|---|---|---|
| Input (keyboard and mouse, or the `--script` list) | keys, mouse | `PlayerIntent`: move, look, jump, crouch, walk |
| Body | `PlayerIntent` | the `Body` (an `rc_movement::Pawn`) |
| Camera | `Body`, `PlayerIntent` look direction | camera position and rotation |
| Report | everything | the log |

`PlayerIntent` holds actions, not keys. A gamepad, touch screen (Android) or
VR controller only needs a new input piece that fills in the same actions. A
VR headset only needs a new camera piece. The movement code itself is plain
Rust with no Bevy and no platform code, so it builds for Windows, Linux and
Android alike.

Coordinates: `rc_movement` uses Unreal's (Z up). Bevy's are Y up and
mirrored, so positions convert as Bevy = (X, Z, Y) / 50, the same as the level
and weapon viewers. The camera's field of view is the game's 85 degrees
across the screen, converted to Bevy's up-and-down angle for the window's
shape. Mouse turning speed is our own choice (0.0025 radians per count); the
game's rule is in the compiled engine.

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
something. The check also compares the colour balance (each colour
channel's share of the total) of the character on screen with its skin
picture: lighting changes brightness but not hue. This was added after the
clone came out magenta on Windows while its outline still matched (Bevy's
default colour grading needs the `tonemapping_luts` feature, which our shared
Bevy feature list leaves out, so the camera uses `Tonemapping::None`). The
magenta picture scores 0.331, the right one 0.001-0.013; 0.06 passes. In the
sandbox the first capture is often black or shows only
the background (the mesh hadn't reached the graphics card yet); the check
retries up to 5 times.

**Fit with the project goals.** Readers and animation maths have no Bevy and
no platform-specific code, so they build for Windows, Linux and Android
alike (only `rc_character` needs Bevy). Skinning ~1,500 points on the CPU is
cheap, but on a phone we will want to hand it to the graphics card through
Bevy's own skinned meshes; the pose code stays the same. For mods: the
viewer takes any package, mesh, set and sequence on its command line, and
nothing assumes the clone trooper's bone names except the defaults.


## Levels (maps)

A map (`.ctm`) is a package like the others. Three kinds of object matter:

- **`Level`**: the list of every actor in the map (the "things": props,
  lights, path nodes, triggers, the player start...) and a link to the BSP.
- **`Model`**: the BSP. Every brush and volume has a small one; the level's
  own is the one `Level` points at.
- **Actors**: objects of classes like `StaticMeshActor`, `Light` or
  `PlayerStart`, each with its own settings.

`rc_level` reads all three. It keeps Unreal's units and axes; converting to
Bevy is the viewer's job.

### The BSP (the level's own walls and floors)

"BSP" (binary space partition) is how Unreal 2 stored the level's hand-built
geometry: a tree that splits space with flat planes. For drawing we only need
its polygons. Each tree node with corners is one flat, convex polygon. The
arrays, in file order:

1. **Vectors**: directions (surface normals and texture directions).
2. **Points**: corner positions.
3. **Nodes**: plane, zone mask, flags, then compact indexes (first corner in
   the vertex pool, surface, back/front/coplanar children, bounds), a
   bounding sphere, zones, corner count, leaves, and three numbers Republic
   Commando adds (they look like render section, first vertex in it, and
   lightmap). 
4. **Surfaces**: material, flags (invisible, two-sided, translucent, ...),
   texture origin point, normal and texture U/V directions, the brush it came
   from, its plane, and its lightmap scale.
5. **Vertex pool**: for each corner, a point index and a "side" index.

The field names come from the public Unreal headers (read in the
SWRC-Modding/CT repository, which has no licence, so nothing was copied); the
exact Republic Commando layout was worked out from `geo_01a.ctm` and checked
on 5 maps. Zones, leaves, render sections and lightmaps come after these
arrays (next section).

Texture coordinates come from the surface: U = (corner - origin) . U
direction, same for V, in texels; the viewer divides by the texture's size.

In Republic Commando most of a level is static meshes, not BSP: `geo_01a`
has 249 BSP polygons but 355 placed props with 141,000 triangles.

### Baked light on the walls: lightmaps

When a level was built, the editor worked out how much light reached each
spot on each BSP wall and saved it as a small picture per surface (a
"lightmap"), packed with others onto 512x512 pages. `rc_level::lightmap`
reads them from the end of the `Model` object, after the vertex pool:

1. `NumSharedSides`, then the zones (count, then an actor, 16 bytes and 4
   bytes each), the `Polys` object, `Bounds` (25 bytes each), `LeafHulls`
   (whole numbers), `Leaves` (two indexes and 8 bytes each), `Lights` (actor
   indexes), `RootOutside`, `Linked`.
2. Render sections: per section, its vertices (40 bytes each: position,
   texture position, lightmap position, normal), a node count, a material and
   12 bytes.
3. Lightmap rectangles: page, surface, zone, top-left corner and size on the
   page (compact indexes), a 4x4 "world to lightmap" matrix and 9 more
   decimals, then a "shadow bitmap" for each dynamic light that touches it
   (the light, a byte array, 7 numbers), then 5 more indexes.
4. Pages: the rectangles on it, 3 numbers, two sizes of pixels (each stored
   as a "lazy array": the file position of its end, a length, the bytes),
   then the pixel format (7 = DXT3 in every map so far), width, height and a
   revision number.

Each BSP node's last number (`light_map`) says which rectangle it uses. A
corner's place on the page: multiply its position by the matrix (Unreal's
row-vector order, translation in the last row) to get texels inside the
rectangle, add the rectangle's corner, divide by the page size.

How we know it is right, on `geo_01a`, `pro`, `ras_01a` and `yyy_01b`:
every object reads exactly to its last byte; every lazy array ends where it
says; every one of the polygon corners lands inside its own rectangle
(within 0.6 of a texel); every lit node's rectangle names the node's own
surface; every rectangle fits on its page. The names follow the public UE2
headers (read in SWRC-Modding/CT; no licence, nothing copied); the byte
layout was measured.

| Map | Rectangles | Pages | BSP polygons with / without a lightmap |
|---|---|---|---|
| `geo_01a` | 126 | 2 | 211 / 33 |
| `pro` | 81 | 2 | 142 / 94 |
| `ras_01a` | 361 | 1 | 812 / 327 |
| `yyy_01b` | 182 | 2 | 268 / 119 |

### Actors and where they are

Actors in a map start with a short "state frame" (script state) before their
property list: two compact indexes, 8 bytes, then 2 bytes (UE2 has 4 here;
found by the lighting thread), and one more compact index if the first was
not zero. After that come normal properties. `Location` is three decimals,
`Rotation` three whole numbers (pitch, yaw, roll; 65,536 = a full turn),
`DrawScale` and `DrawScale3D` scale the model, `StaticMesh` names the model,
and `Skins` replaces its materials.

To place a model: subtract `PrePivot`, scale, turn (yaw around the up axis,
pitch, then roll, Unreal's standard order), then move to `Location`.

The `Level` object holds one unknown byte (always 0 so far), the actor list,
the map's URL, and then the BSP model reference.

### Showing a level (`rc_level_viewer`)

Before Bevy starts, the viewer reads the map, then every material and static
mesh it uses from the other packages in `GameData` (it indexes every package
file by name once). Materials are followed to a plain texture: a `Shader` to
its `Diffuse`, Republic Commando's own `Hs...` materials to their
`DiffuseTexture`, panners and modifiers to their `Material`. `HsFalloff`
light shafts are drawn as glow (added on top), not as solid surfaces.

All triangles that share a material go into one Bevy mesh, so the GPU gets
one "draw call" per material (60 for `geo_01a`) instead of one per prop. This
matters for slower machines and phones.

**Coordinates.** Unreal: X forward, Y right, Z up. Bevy: Y up. We swap Y and
Z and divide by 50 (`UNREAL_UNITS_PER_BEVY_UNIT`, the one place the scale is
set, agreed with the other threads). Swapping two axes mirrors the world,
which also turns Unreal's left-handed coordinates into Bevy's right-handed
ones. Mirroring reverses the order of every polygon's corners, so the viewer
works out each BSP polygon's corner order from its stored normal and writes
the triangles so the right side faces out. In all 5 maps every BSP polygon
had the same order.

**Camera.** A free-flying camera in its own file (`camera.rs`), separate from
level loading, so it can later become a first-person player or a VR headset.
It starts at the first `PlayerStart`, 64 units above it, facing its yaw.

### How we check a level without looking at it

- **BSP checks**: every index points inside its array, every polygon's
  corners lie on its node's plane (within 1 unit), plane normals have length 1.
- **Floor test**: PathNodes (the AI's walking network) and PlayerStarts float
  at a character's half height above a floor. A ray straight down from each
  must hit level geometry soon, and never closer than 80 units (that would
  mean the point is inside a prop). In `geo_01a`, 292 of 293 have a floor
  within 150 units (median 104), none closer than 80. Turning props the wrong
  way put 14-20 points inside props, so the test notices wrong placements.
- **On-screen check**: from the spawn point, more than half the window must
  be level (not background) with more than 100 different colours.
  `--capture FILE.bmp` also saves that frame as a picture.
- **Lightmap checks**: the ones listed under "Baked light on the walls". If
  the lightmaps can't be read, the level still loads and draws unlit, and the
  log says why.

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

## Delta Squad (`rc_squad`, `rc_squad_plugin`)

Sev (07), Fixer (40) and Scorch (62) follow the player and take the four
squad orders of the original. Every number is in `data/squad/squad.toml`.

### Bodies

Each squad mate is an `rc_movement::Pawn`, the same body and physics as the
player (the commandos share the player's speeds and size in the game's
data). The squad steers each one the way a player would press keys: a
`MoveInput` that faces the next point and holds "forward", with "walk" held
when the spot is closer than 400 uu (the original's `AUTORUNDISTANCE`). A
body that barely moves for 0.6 s jumps once; after 3 s it's logged as stuck.

### Orders are stances, and stances are tethers

In the original each order switches the squad's "stance". A stance ties
("tethers") each member to something and gives two pairs of distances,
read from the stance classes' defaults:

| Order (PC key) | Stance class | Tied to | Set off beyond | Stop within | Urgent beyond / until |
|---|---|---|---|---|---|
| Search and destroy (F1) | StanceSearchAndDestroy | objective (we use the leader) | 512 | 256 | 2048 / 1280 |
| Form up (F2) | StanceFormUp | leader | 768 | 192 | 1280 / 768 |
| Secure position (F3, aimed) | StanceSecurePosition | the aimed spot | 768 | 128 | 1024 / 512 |
| Hold (no PC key) | StanceHold | where they stood | 512 | 256 | 768 / 384 |
| Cancel (F4) | (cancels all marked jobs, back to form up) | | | | |

How we use them (the original does this in compiled code, so the rules are
our reading of the numbers):

- Every 0.25 s (`DecisionInterval`) each member works out its spot and its
  distance to what it's tied to.
- Standing, it stays put until that distance passes "set off beyond". Then
  it walks (or runs) to its spot, and stops at the spot or once back within
  "stop within". So the squad doesn't shuffle after every step: on geo_01a
  they set off when the player was 837 uu away and stopped about 200 uu
  from them.
- Beyond "urgent" it always runs, until back within the second number.
- A direct order (pressing the key) sends everyone to their new spot at
  once, all the way, whatever the distances.
- If it gets stuck while already within "set off beyond", it stops where it
  is ("stopped short" in the log) instead of pushing at the obstacle.

Spots (our own, in `squad.toml`): form up uses three slots behind the
leader, measured along the way they face; search and destroy three slots
spread out ahead; secure position a circle of 110 uu around the spot. A
slot inside a wall is pulled back towards the leader (or the spot) and
dropped to the floor below.

Search and destroy in the original advances to the squad's next objective
(set by each map's scripts). Map objectives aren't read yet, so we tie it
to the leader. With enemies known (`SquadThreats`), each member closes to
half the search radius (768 uu, `Squad.DefaultSearchAndDestroyRadius`) of
the nearest one within reach.

### Aimed orders and marked jobs

F3 and `squad_command` trace a line from the player's eyes up to 4096 uu.
Where it meets the world, the spot is moved back a body's width and
dropped to the floor. If the aim is within 0.12 radians of a squad marker,
`squad_command` (and `use`) send the nearest free member to do that job
instead.

A marked job is two actors in the map: the thing (the "anchor", like
`BactaDispenserGEO4`) and a `SquadMarkerSpot` next to it whose `MyAnchor`
names it. The member walks to the spot, works for 3 s (our own; the
original plays an animation), and the marker is done. Stuck for 10 s on
the way, it gives the job up. geo_01a has 6: five bacta dispensers and a
demolition charge.

### Where the squad comes from

`spawn = "with_player"` puts all three in their form-up slots at the
start. The original brings them in through each map's scripts: on geo_01a
only Scorch, from `PawnFactory14` on the `levelstart` event. The log lists
these (`the map would bring in ...`) for later.

### Finding the way

Routes come from `rc_nav` (the enemies thread's crate: the map's path
nodes and links). The squad only asks for one when a straight walk is
blocked: it sweeps a slightly slimmer body from where it is to the spot,
lifted by a step's height so floors and small steps don't count.

### In Bevy

`SquadPlugin` follows `docs/PLUGINS.md`. Its systems run in
`GameSet::Simulate` inside `SquadSet` (order the player's movement before
it). It reads `ActionState`, the `Player` and `PlayerCamera` transforms and
`LevelCollision`, and writes `SquadReadout`. A `SquadOrder` message lets
anything else (a script, a touch menu, a VR gesture) give orders. Each
member is an entity with `SquadMate` and a `Transform` at its feet, facing
its way like the player's; with a renderer it gets a capsule in its own
colour (`EnhancedVisionColor` from the Properties package) and a dark
visor, until the real model is drawn.

### Health and going down

Each squad mate entity also carries `rc_combat`'s `Health`, `Shield`,
`Team::Republic` and `Hittable::COMMANDO`, so shots hit them like anyone
else. The numbers are per member in `squad.toml` (from the Properties
package: 400 health, 50 shield, recharging 15 per second after 4 s). Squad
mates can't die in the original (`bCannotDie`): when health reaches 0 (or
combat marks them `Dead`), `check_down` calls `Squad::set_down`. A downed
member stops where it is, takes no orders, gives up any marked job (the
marker opens again), and shows as `Incapacitated` with "Down" on the squad
panel. Health above 0 again brings them back (`Revived`); nothing heals
them yet, as reviving isn't built.

### How we check it without looking

`rc_squad_sim` runs the real plugin on a real map with no window, in fixed
1/60 s steps, with a scripted player. It logs every order, set-off,
arrival, stuck, job and a once-a-second line with every member's position,
state and distances, and checks 23 numbers (see `README.md`).
