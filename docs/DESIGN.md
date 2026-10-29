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
| `rc_viewer` | The Bevy program: opens a window and shows one texture, or one 3D model with `--mesh`. |

Format readers stay plain Rust with no Bevy in them, so they can be tested alone.
Only `rc_viewer` uses Bevy (version 0.20.1, MIT or Apache-2.0). It turns on
only the parts of Bevy it needs: a 2D window, plus `bevy_pbr` for 3D models
and their materials (no audio, gamepads or UI yet), which keeps the build
smaller.

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
  drawing (UE2 often doubled vertex light). Not checked.
- **BSP geometry** (the level's brush-built walls and floors): not found yet.
  The maps hold no separate lightmap texture objects, so it must be inside the
  level's `Model` data, which the level thread is reading.

## Lighting in Bevy (`rc_viewer/src/lighting.rs`)

Bevy lights are physically based (lumens, lux), the game's are not, so the
conversion has tunable factors. All of them sit in one `LightingSettings`
resource, and a plain text file can change them (`key = value` lines, `#`
comments) so modders can tweak lighting without touching code:

```
# lighting.cfg
units_per_bevy = 50             # Unreal units per Bevy unit (project-wide rule)
live_lights = all               # all, dynamic, or none
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

The factors are placeholders. The plan to set them properly: compute what our
lights would give at each static-mesh vertex and compare with the colours the
game baked there, then adjust until they agree.

**Plan for speed (and VR).** Once the baked light is shown, set `live_lights =
dynamic`: the 91-98% of lights that are baked cost nothing at run time, and
only a handful of real-time lights remain. That suits a later VR version,
which has to draw everything twice (once per eye) at 90 frames per second or
more.

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
