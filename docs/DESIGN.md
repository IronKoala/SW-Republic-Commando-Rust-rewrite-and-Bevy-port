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
| `rc_level` | Reads maps: the level's own geometry (the BSP), its baked lightmaps, where every actor is placed, each prop's baked corner colours, blocking volumes, models that come from a prop's class, and a map's walking collision (`map_collision`). No Bevy. |
| `rc_level_viewer` | Bevy program: loads one map, lit with its baked light and real-time lights, with its sky, and lets you fly around it. Also a library (`scene`, `draw`, `lighting`, `materials`, `sky`) that `rc_walk` uses to draw maps. `rc_level_check` loads every map without a window and logs the results. |
| `rc_movers` | Doors, lifts and hatches: runs a map's movers through their keys, makes their collision move with them, carries the player on lifts, and runs triggers. No Bevy. `rc_mover_check` runs every mover in every map without a window and logs the numbers. |
| `rc_anim` | Poses a skeleton from an animation and moves a mesh's points with it ("skinning"). Plain maths, no Bevy. |
| `rc_character` | 3D Bevy program: shows one character from the game with its skin, playing one animation. |
| `rc_soundtest` | Bevy program with no window: plays one weapon sound and one ambient loop and logs proof that they played. |
| `rc_squad` | Delta Squad: Sev, Fixer and Scorch following the player and obeying squad orders, with the game's numbers. No Bevy. |
| `rc_squad_plugin` | `SquadPlugin`, the squad in the Bevy game, and the `rc_squad_sim` example (a windowless squad test on a real map). |
| `rc_game_core` | The shared pieces every part of the game plugs into: screens, folders, input actions, display readouts, loading, collision, player markers, units, the log, and the save interface (`save`). Bevy, but no window. |
| `rc_save` | Saving, loading and checkpoints: `SavePlugin` writes and reads the save files, quicksave and quickload, and saves at checkpoints. Bevy, but no window. |
| `rc_level_plugin` | The level part of the game: `LevelPlugin` loads the chosen map into `rc_game` (its look, sky, collision with props, doors and invisible walls, and the player start). |
| `rc_game` | The game: one program with the menus, settings, controls, player, on-screen display and music. The other parts (levels, weapons, enemies, squad) plug into it. |
| `rc_nav` | The level's path network (path nodes and the editor's links between them), shortest routes, and walking a route with the player's physics. Shared by enemies and the squad. No Bevy. Tool: `rc_navcheck`. |
| `rc_ai` | What enemies decide: settings from `data/enemies/*.toml`, the map's spawners, one fighter's brain, and a whole fight on a map (`Battle`). No Bevy. Tool: `rc_droidsim` (droids against a stand-in player, no window). |
| `rc_pads` | Controllers: which kind each one is, button names for prompts, gyro clean-up, and an SDL3 reader (built from source) for controllers and gyros. Bevy, no window. Tool: `rc_padtest`. |
| `rc_panini` | The optional Panini view for very wide screens: a full-screen pass that bends the 3D view. Check: `panini_check` example. |
| `rc_enemies` | Bevy plugin `EnemiesPlugin`: battle droids in the game, shooting and taking damage through `rc_combat`, animated with the game's mesh. Program: `rc_droidview` (watch a fight in a real level, with a picture check). |

Format readers stay plain Rust with no Bevy in them, so they can be tested alone.
Only `rc_viewer`, `rc_character`, `rc_level_viewer` and `rc_soundtest` use Bevy
(version 0.20.1, MIT or Apache-2.0). Each turns on only the parts of Bevy it
needs, which keeps the build smaller: `rc_viewer` and `rc_character` a 2D window
plus `bevy_pbr` for 3D models and their materials (the same list, so they share
one build); `rc_level_viewer` Bevy's full 3D renderer (`3d_bevy_render`);
`rc_soundtest` only audio and WAV decoding, with no window.

## Finding the game

In the game program (`rc_game`), the folder comes from `settings.toml`
(`[game] folder`), then the `RC_GAME_DIR` environment variable, then the
usual Steam folders for the system (Windows, Linux, Linux Flatpak). A folder
counts if it has `GameData/Maps`. File names are matched without caring
about upper or lower case, because Linux and Android care and Windows
doesn't. On Android the folder will be one the player picks.

The older test programs use `RC_GAME_DIR` or default to
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
   (the light, a byte array, 7 numbers), then one compact index and one
   4-byte number. (We first read 5 compact indexes here; that happened to
   work on the first maps and broke 12 others.)
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

**Values that come from the class.** A map only stores the settings a
designer changed. Most props (barrels, lamps, bacta dispensers, wall
pieces) and the double-door movers (`MoverDB_*`) have no `StaticMesh` in
the map at all: their model is the class's default, stored at the end of
the class in a script package (`Properties/StaticPropProperties.u`,
`KarmaPropProperties.u`, `CTInventory.u` and others). `rc_level::class_look`
walks the class and its parents across packages (nearest class wins) using
`rc_package::class_defaults`, and fills in `StaticMesh`, `DrawScale`,
`DrawScale3D` and `bHidden` where the map leaves them out. A class whose
`DrawType` isn't "static mesh" (8) is left alone, and markers are hidden in
the game, so they stay hidden. Across all 79 maps this adds 3,460 props.

**Blocking volumes** (`rc_level::volumes`). Invisible walls the designers
used to keep the player in bounds. Each has a small BSP model (its brush)
in the brush's own space: subtract `PrePivot`, turn by `Rotation`, then move
to `Location` (`MainScale` and `PostScale` were 1 in every map). Flags say
whether it stops players (`bBlockPlayers`), the AI (`bBlockActors`) and
shots (`bBlockZeroExtentTraces`). The ones that stop players are added to
the walking collision. Check: only 11 of 8,942 AI path points are inside a
volume (107 with `PrePivot` ignored, 115 turned a quarter wrong).

**The sky.** Unreal 2 draws the sky from a small sky box somewhere in the
map, marked by a `SkyZoneInfo`. Windows onto it are BSP surfaces flagged
"fake backdrop". `rc_level_viewer/src/sky.rs` puts a second camera at the
`SkyZoneInfo` on its own render layer; it turns with the player but never
moves, and draws first. The player's camera then draws the level without
clearing, leaving out the fake backdrop polygons, so the sky shows through.
Which polygons and props are in the sky box comes from the BSP zone tree
(`Bsp::point_zone`). 67 of 79 maps have one.

**Collision without Bevy** (`rc_level::map_collision`). Loads a map's BSP,
placed props (with class models) and player-blocking volumes as triangles,
plus the first `PlayerStart`. `rc_walk` uses it, and other programs can.

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
- **Every map** (`rc_level_check`): builds all maps in `GameData/Maps` without
  a window and writes one block per map to `logs/rc_level_check.log` (BSP,
  lightmaps, props, materials, volumes, sky, floor test), then a summary
  line per map. A crash in one map is logged as that map failing. Exits
  with 1 if any map failed.

## Doors and lifts (movers)

A "mover" is anything in a map that slides or turns between fixed
positions: doors, lifts, hatches, bridges. The reading is in
`rc_level::movers`, the running in the `rc_movers` crate, and the drawing in
`rc_walk`.

### What a mover stores

Read from the map, falling back to the mover's class defaults (the same
class walk as props, `rc_level::class_look`), then to the engine's own
defaults. The property names come from Unreal Engine 2's public `Mover`
and `Trigger` classes; the values are the game's.

- **Keys**: up to 24 positions (`KeyPos`) and turns (`KeyRot`), counted
  from a base (`BasePos`, `BaseRot`). `NumKeys` of them are used. Key 0 is
  "closed", the last key is "open". `KeyNum` is where it starts.
- **Timing**: Republic Commando keeps one time per step, `MoveTimes[k]`,
  the seconds from key k to key k+1 (UE2 had one `MoveTime` for all). A
  step with no entry takes the one before it; with none at all, 1 s.
  `StayOpenTime` is how long it waits before going back. `DelayTime` is a
  wait before it starts.
- **Loops that go round**: when the last key is the same pose as key 0 (a
  turntable, a conveyor), a looping mover carries on round from key 0
  instead of going back the way it came.
- **Easing** (`MoverGlideType`): constant speed, or (the default) a gentle
  start and stop. We use the smooth curve 3t² - 2t³ for the gentle one. That
  is our reading of how UE2 describes it; the exact curve is in the compiled
  engine and hasn't been measured against the real game.
- **What sets it off** (`InitialState`): see below.
- **`Tag` and events**: a trigger whose `Event` matches a mover's `Tag`
  sets it off. RC movers send four events of their own, `OpeningEvent`,
  `OpenedEvent` (falls back to `Event`), `ClosingEvent` and `ClosedEvent`,
  so doors and lifts can be chained.
- **`bAutoDoor`** is read and logged, not acted on yet.
- **Double doors** (`MoverDB_*` and `MoverDBSlave_*`) take their values from
  `Properties/MarkerProperties.u`: they start `Locked`, open in 0.5 s after
  a 0.2 s delay.
- **When blocked** (`MoverEncroachType`): stop, go back the way it came (the
  default), keep going (crush), or pass through.
- **Sounds**: opening, opened, closing, closed and while moving, kept as the
  sound's name (like `Package.Group.Name`). They are logged where they would
  play; `rc_walk` has no audio yet.

### What sets a mover off

`InitialState` names a behaviour. `data/movers/movers.toml` maps each name
to one of ours, so the game's own state names (or a mod's) can be added
without code:

| Behaviour | What it does |
|---|---|
| `trigger_open_timed` | A trigger opens it; it waits `StayOpenTime`, then closes. |
| `trigger_toggle` | Each trigger opens it if closed, closes it if open. |
| `trigger_control` | Open while the trigger is held, closes when let go. |
| `trigger_pound` | Opens and closes over and over while the trigger is held. |
| `bump_open_timed` | Opens when the player walks into it (the UE2 default). |
| `stand_open_timed` | Opens when the player stands on it (a lift). |
| `bump_button` | Moves when bumped, comes back after a wait. |
| `trigger_loop` | A trigger starts it opening and closing over and over; the next trigger stops it where it rests. |
| `loop` | Opens and closes for ever from the start. |
| `static` | Never moves. |

Republic Commando's own state names, found by listing all 941 movers in the
79 maps: `TriggerControl` 335, `TriggerLoop` 186, `BumpOpenTimed` 122,
`TriggerToggle` 109, `None` 84, `Locked` 58, `TriggerOpenTimed` 42,
`Looping` 5. `Locked` and `None` run as `static` (scripts move them, which
we don't run yet); `Looping` runs as `loop`.

Engine `Trigger`s default to `bTriggerOnceOnly = true` in RC's packages,
so most fire once.

Touching only counts when it starts: standing on a lift that has come back
down doesn't send it up again until you step off and on. A door that is
open keeps to its timer even if you stand in it; if it then closes on you,
its "when blocked" rule decides.

Triggers (`Trigger` and its subclasses) run when the player's box overlaps
their cylinder (`CollisionRadius`, `CollisionHeight`) and their
`TriggerType` includes players. Only triggers in the normal state run;
others (switched on and off by other triggers) are listed in the log as not
handled yet. Doors opened by the game's scripts (scripted sequences, squad
orders) are not handled yet either.

### Moving collision

Each mover's collision triangles are taken out of the level's fixed
collision and kept on their own, where the map placed the mover. To test a
move against a mover, the move is shifted into the mover's starting pose
instead of moving the triangles: exact for sliding movers, and for turning
ones the player's box keeps its world-aligned shape, which is close enough
for doors.

Each frame, before the player moves:

1. Triggers and touches are checked.
2. Each mover works out its next position.
3. If the player stands on it (a short sweep down hits it as floor), the
   player is moved by the same amount first, so a lift carries them.
4. If the mover would then overlap the player, it tries to push them along.
   If they can't be pushed (a wall behind, or the floor under a closing
   door), the "when blocked" rule applies: it stops, or turns round.

Then the player moves against the fixed level plus the movers where they
now are.

### Drawing

`rc_level_viewer::scene` gives each mover its own draw batches, and
`draw::spawn_level` tags them with `MoverPart(export)`. Their corners are
where the map placed the mover; `rc_walk` sets their `Transform` each
frame to the mover's move from there (Unreal to Bevy swaps Y and Z, so the
turn's rows and columns swap the same way).

### How we check movers without looking

- **Unit tests** (`cargo test -p rc_movers`): a triggered door opens in its
  `MoveTime`, waits `StayOpenTime` and closes; the smooth curve is at 15.6%
  a quarter of the way through and 50% halfway; a 3-key mover takes 2 x
  `MoveTime`; a lift carries the player up 400 units and back down without
  them sinking in or floating off; a closing door turns back when the
  player is in the way; walking into a bump door opens it and lets you
  through.
- **Every map** (`rc_mover_check`): lists every mover and trigger with
  their values and logs them to `logs/rc_mover_check.log`. Then it runs
  each mover through a full open and close with the player far away,
  logging its position every 0.1 s and checking that opening and closing
  each take `MoveTime` per key (within 2 frames). Then it stands a player on
  top of every lift (moves mostly up or down by at least 64 units, wide
  enough to stand on), sets it going, and checks the player's feet move as
  far as the lift (within 4 units) and never more than 16 units from it on
  the way.
  Doors that rise into a ceiling look like lifts by shape, so a lift is
  only ridden when a player's box can travel its whole path through the
  fixed level (a sweep from bottom to top); the rest are counted as
  "level in the way".
- **In the walker**: `rc_walk --map MAP --ride --script` starts the player
  on the map's first rideable lift, sets it going, and checks the same
  three things.

Results on all 79 maps (2026-10-29): 941 movers (877 with collision), 1,675
triggers, 1,405 sounds named, no unknown states. 863 of 863 movers opened
and closed in their `MoveTimes` (72 take over 60 s and weren't run). 33 of
33 lift rides passed (25 skipped, level in the way). 472 trigger-driven
movers have nothing in the map sending their tag: most likely the game's
scripts. In the walker, geo_05a's `Mover37` carried the player 12,176
units up in 18 s with a gap of 0.0 units.

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
`GameSet::Simulate` inside `SquadSet`. `rc_game` orders its
`player::PlayerMoveSet` before `SquadSet`, so the squad sees this frame's
leader. It reads `ActionState`, the `Player` and `PlayerCamera` transforms and
`LevelCollision`, and writes `SquadReadout`. A `SquadOrder` message lets
anything else (a script, a touch menu, a VR gesture) give orders. Each
member is an entity with `SquadMate` and a `Transform` at its feet, facing
its way like the player's; with a renderer it gets the original model (see
"How the squad looks" below), or, if that can't be read, a capsule in its
own colour (`EnhancedVisionColor` from the Properties package) with a dark
visor, as a placeholder.

### How the squad looks (`rc_model`, `rc_squad_plugin/src/look.rs`)

All three squad mates use the same mesh, `CloneCommando` in
`GameData/Animations/clone.ukx` (the `Mesh` default of the `CloneCommando`
class in CTCharacters.u). The mesh's own material is a stand-in
(`CloneCommandoProxy`); each class's `Skins[0]` (in the Properties package,
`CommandoNNDeltaDefaults`) replaces it with that commando's armour:
`CloneTextures.CloneTextures.CloneCommando07_Shader` for Sev, `..40..` for
Fixer and `..62..` for Scorch. A Shader's `Diffuse` leads to the 1024x1024
picture. The animations are spread over two sets the mesh links to:
everyday moves in `CloneCommandoSet`, wounded and healing ones in
`CloneCommandoSPSet`.

`data/squad/looks.toml` names the package, mesh, replacement materials
(`skins`, by material slot) and which sequence plays for each pose. The
poses (`look::Pose`) are picked from the squad's state each frame: knocked
out plays `WoundedFallDown` once and holds it; reviving plays
`HealImmobileLoop`; moving picks walk, run, back or side-step from the
speed and the direction against the facing (as the droids do); a shot in
the last 0.4 s plays `Fire`; otherwise `ActionBreathe`. Which animations the
original plays for knocked-out and reviving is compiled code, so those two
are our picks.

`rc_model` is a new crate with no Bevy. It reads a skeletal mesh, the named
sequences (looking in each animation set the mesh links to, first set
first), and a picture for every material. A mesh's triangles are split into
one "section" per material slot, each drawn as its own Bevy mesh with its
own picture, so a model made of several materials (like `TrooperNoHelmet`:
face plus armour) is drawn whole. It follows Shader -> `Diffuse`, Combiner
-> `Material1`, FinalBlend and texture modifiers -> `Material`, including
steps into another package (looked up in Textures, Animations and
StaticMeshes). A material counts as see-through-in-places ("cut-out") when
the texture is masked, a FinalBlend alpha-tests, or a Shader has an
`Opacity`; other alpha is left alone, because Unreal often keeps
shininess there. Posing is as for the droids: CPU skinning every frame,
copied into each section's mesh. The model faces +X in the game's axes,
the squad mate's body faces its local -Z, so the meshes hang under a holder
turned a quarter turn (`look::model_turn`).

The droids (`rc_enemies/src/look.rs`) and the `rc_character` viewer still
have their own one-material readers; they can switch to `rc_model` later.

Each mate reads its model once (about 0.7 s each in a debug build in the
cloud). The three share one mesh, so reading it once and swapping only the
picture would be quicker; not done yet.

How it's checked: `rc_squad_look_check` (an example program in
`rc_squad_plugin`) stands the three in front of a camera using the game's
own `spawn_pieces`, plays every animation on the CPU and checks heights
stay between 20 and 260 uu, checks each `skins` entry against the class's
`Skins` read from the Properties package, then captures the window. For
each squad mate and each material it projects our own posed triangles
(nearest in front) and compares: outline overlap (0.93 or more passes; got
0.986-0.988), and colour balance on screen against the material's picture
sampled at each triangle's middle, weighted by size (0.04 or less passes;
got 0.001-0.003). A capture where textures were lost (magenta or plain
grey) fails the colour part even with a perfect outline. Finally each mate's
colour must be nearest its own armour's (own 0.001-0.003, nearest other
0.020-0.023). With Sev's and Scorch's armour swapped in `looks.toml` the
original-armour check fails for both, as it should. In `rc_game --script`
the player turns round, `logs/squad.png` is captured, and a check confirms
all three mates have their model and pictures.

### Health and going down

Each squad mate entity also carries `rc_combat`'s `Health`, `Shield`,
`Team::Republic` and `Hittable::COMMANDO`, so shots hit them like anyone
else. The numbers are per member in `squad.toml` (from the Properties
package: 400 health, 50 shield, recharging 15 per second after 4 s). Squad
mates can't die in the original (`bCannotDie`): when health reaches 0 (or
combat marks them `Dead`), `check_down` calls `Squad::set_down`. A downed
member stops where it is, takes no orders, gives up any marked job (the
marker opens again), and shows as `Incapacitated` with "Down" on the squad
panel. Health above 0 again brings them back (`Revived`).

### Squad fighting and reviving (`rc_squad/src/fight.rs`)

The original decides how its squad fights in compiled code (`ctgame.dll`)
that we don't read, so these rules are **our own**. Every number is in
`[fight]` and `[revive]` of `squad.toml`, and each one says whether it is
the game's, a guess, or our own.

*Seeing and picking a target.* The plugin turns every living
`Team::Separatist` character with a `Hittable` body into a `Threat` each
frame (`SquadThreats`; the id is the entity's bits). On every think (0.25
s) a member who is up and not busy with a job or a revive takes the
nearest threat within `sight_radius` whose eyes it can see (a thin line
from its eyes to theirs that hits no level geometry). Nothing in sight
drops the target.

*Shooting.* After `reaction_time` on a new target it fires bursts:
`burst_count` shots `shot_interval` apart (the DC-17m's fire rate), then a
pause between `burst_pause_min` and `burst_pause_max`. Each shot aims at
the target's middle, off by up to `(1 - accuracy) x max_aim_error`; the
weapons part then adds the gun's own spread, as it does for the player and
the droids. A standing member turns to face its target and fires only
within `max_aim_yaw` of its facing; a moving one fires on the move. A
member holds fire while the player or another squad mate (standing or
lying) is within `friendly_clearance` of the line of fire, nearer than the
target. Shots go out as `rc_combat::FireShot` with the squad mate as the
shooter, so the weapons part flies the bolts, applies the damage, and its
`ShotLanded` and `Died` messages count hits and kills per member
(`SquadFightStats`).

*Reviving.* Every frame, each downed commando nobody is seeing to gets the
nearest member who is up, not on a marked job and not already reviving,
if one is within `range`. The player counts too when they have `Dead`
(and `revive_player = "yes"`). The reviver runs to a spot beside the body
and, once within `reach`, kneels (`MemberState::Reviving`) for `time`
seconds. Then `ReviveDone` goes out and the plugin sets the patient's
health to `health` times their maximum and removes `Dead`; next frame
`check_down` sees them up. The player revives a squad mate by holding
`Use` within `reach` of the body for `player_time` seconds
(`Squad::player_use`); letting go starts it over. A revive job ends early
if the patient is already up (someone else got there) or the reviver goes
down. While the player is down, `rc_game` keeps their body still (they
can look around).

Not done yet: droids only shoot at the player (`rc_ai::Battle` takes one
target), so squad mates are only hurt by stray bolts and the player. The
original also has the downed player call the squad over with an order;
here they come on their own.

### How we check it without looking

`rc_squad_sim` runs the real squad and combat plugins on a real map with no
window, in fixed 1/60 s steps, with a scripted player. It logs every
order, set-off, arrival, stuck, job, target picked, first shot of each
burst, kill and revive, a once-a-second line with every member's position,
state and distances, and every 5 s a fight line with each member's shots,
hits, kills and revives. For the fight it puts stand-in droids (real
combat targets with the game's droid size and health that stand still and
don't shoot back) in sight of the squad. The checks are listed in
`README.md`.

## The game program (`rc_game`)

`rc_game` is built from Bevy plugins. Parts don't call each other; they
share the types in `rc_game_core`. How a part plugs in is in
`docs/PLUGINS.md`.

**Screens.** `GameState` is `Boot` (read settings) then `MainMenu`, `Loading`
and `InGame`. While in `InGame` a second state, `Play`, is `Running` or
`Paused`, so pausing never counts as leaving the level. Gameplay systems run
only while `Running`. Everything spawned for a level carries
`DespawnOnExit(GameState::InGame)` and is cleared on the way back to the
menu (the script checks that no player, camera or level mesh is left).

**Loading.** The menu sets `CurrentLevel` and enters `Loading`. Any part with
slow work calls `LoadingGate::wait("name")` and later `done("name")` or
`fail("name", reason)`. The shell moves to `InGame` when nothing is waiting
(and at least one frame has passed, so every part has had its turn); on a
failure it goes back to the menu and shows the reason.

**Input.** Devices are read in one place (`input.rs`) into `ActionState`:
which actions are held this frame, a movement direction and a turn amount.
Gameplay reads only that. The bindings file maps keys, mouse buttons and
wheel, gamepad buttons and sticks to actions; touch screens move with a drag
on the left half, look with a drag on the right half, and fire with a tap.
The keyboard defaults are the original's `DefaultCtrl.ini`. A scripted run
or a VR mod writes `ActionState` itself instead.

**The player.** `player.rs` moves a body with `rc_movement` (as `rc_walk`
does) from `ActionState`, and puts a separate camera at its eyes. The
camera is not a child of the body, so a VR headset can replace it.

**Menus.** *Placeholders:* these menus and the display below are our own
stand-ins until the original's menus and HUD are read from the install
(`docs/ORIGINAL_MENUS.md`). Each menu is a page: a title and a list of items, drawn with
Bevy's UI and the theme file's colours. Pages: main, campaign list, a
campaign's levels, options, pause, and "game files not found". The mission
names, order and loading text come from the game's own
`GameData/System/LevelLoadingInfo.int` (plain text with numbered entries
like `MapName[3]="GEO_03A"`), read at start-up, never copied into the repo.
`data/levels/campaigns.toml` only groups them by name prefix. Sizes are a
share of the screen height, or of its width when that is smaller, so the
menus fit a phone or a monitor.

**Screen shapes and controls.** The view is "Hor+": `video.fov` is the
angle across a 4:3 screen, as in the original, and wider screens see more
at the sides. Keyboard, mouse, gamepads and touch all work at once. Details,
numbers and what is still to do (edge stretching on ultrawides, controller
pictures, gyro) are in `docs/DISPLAY_AND_INPUT.md`.

**On-screen display.** Corner brackets like the commando's visor, a
four-bar crosshair whose gap follows the weapon's spread and turns red on a
target, shield and health bars, the ammo counter, a squad list and short
messages. It reads only the readout resources, and logs its numbers every
two seconds.

**Music.** `music.rs` plays the game's Ogg Vorbis files straight from
`GameData/Music` with Bevy's audio. `data/music/music.toml` picks the menu
track and, per map name prefix, a calm and a battle list; `MusicMood`
chooses between them. Each track is despawned when it ends and the next one
starts.

**Mods.** Everything we ship as data (controls, theme, music, campaigns,
movement) is found through `GamePaths::data_file`, which looks in each
enabled mod's folder (`mods/<name>/`) before `data/`. Lists that add up (the
campaigns file) read every copy.

**The level (`rc_level_plugin`).** `LevelPlugin` is the only part that
reads the map. When the game enters `Loading` it tells the loading screen to
wait (`LoadingGate`, part name `level`), waits three frames so the loading
screen is drawn, then in one go:

1. loads the collision with `rc_level::map_collision::load`: the BSP, every
   placed prop, props and double doors whose model comes from their class,
   and the blocking volumes that stop players. It becomes `LevelCollision`
   (a `rc_movement::TriangleWorld`, in Unreal units);
2. puts the player's feet one collision height (the commando's, from
   `MoveParams`) under the map's first `PlayerStart` and inserts
   `PlayerStart`. A map without one starts above its middle, and says so in
   the log;
3. draws the map with `rc_level_viewer::scene::build` and `draw::spawn_level`,
   with brightness from an optional `levels/lighting.cfg` in a data folder
   (so a mod can change it);
4. if the map has a `SkyZoneInfo`, spawns the sky camera
   (`rc_level_viewer::sky`) at camera order -1, under the player's camera
   (order 0) and the menus' camera (order 1). Once the player's camera
   exists, the plugin gives it `SkyViewer` (the sky camera copies its turn
   and field of view every frame, so it lines up at any screen shape) and
   stops it clearing the picture, so the sky shows where the level has
   nothing.

Everything it spawns carries `DespawnOnExit(InGame)`, and `LevelInfo` (the
counts, for the log and the self-test) is removed on the way out. The
plugin never reads input and never moves the camera, so a VR camera that
carries `PlayerCamera` gets the sky the same way.

Loading blocks the game while it runs (about 2 s for geo_01a in a debug
build in the cloud). Moving it to a background thread is future work; the
loading screen stays up meanwhile.

**Checking without looking.** `rc_game --script` presses the same actions a
player would and checks each step: menu pages, loading, the player moving
430 units in a second of walking, nothing moving while paused, the level
being cleared, options changing. It also captures the window twice and
measures colours: the main menu must contain amber text, and in the level
at least 30% of the screen must be lit with amber crosshair pixels at the
centre. With the level part plugged in it also checks the level's counts
(collision triangles, props, props with their class's model, a player
start), that the sky camera follows the player's camera, and looks up 60
degrees in geo_01a: more than 45% of the screen must be lit (72% with the
sky; 17% in a trial run with the sky camera switched off, because the game
clears to black).
centre.

## Enemies (`rc_nav`, `rc_ai`, `rc_enemies`)

Three layers, so the rules can be tested without a window and reused by the
squad:

1. **Paths (`rc_nav`).** Every map stores a network of path points
   (`PathNode`, cover points, `PlayerStart`, ...) and, for each, the links
   the editor worked out ("reach specs": start, end, length and flags such
   as walk, fly, jump, door). `NavGraph::from_map` reads them; links whose
   flags allow walking are used. `BlockedPath` points (barricades) start
   blocked, as their class default `bBlocked` says, and
   `set_blocked_by_tag` opens one when its event fires. Routes are found
   with A* ("walked so far + straight line still to go"), and
   `PathFollower` turns a route into the same `MoveInput` the player's
   keys make, so droids move with the player's physics (`rc_movement`).
   `rc_navcheck MAP` checks a map: on geo_01a 358 points, 1,224 links,
   every stored length within 1 uu of the straight line.

2. **Decisions (`rc_ai`).** The original's AI is compiled engine code we
   don't read. What we can read are its settings and the names of its
   states, so the rules are ours, built on those settings
   (`data/enemies/battle_droid.toml`, each value marked "game" or "guess"):
   - *Noticing*: within sight radius, within the view cone
     (`PeripheralVision`), with a clear line from eye to eye; or anything
     within `AutoDetectRadius`; or a shot heard nearby. Once fighting, it
     keeps track by line of sight alone (as Unreal Engine 2 bots do).
   - *Reacting*: waits `reaction_time`, turns at its turn rate, and shoots
     only within `MaxAimYaw` of its facing.
   - *Shooting*: bursts of `BurstCount`, `ShotDelay` apart, then a rest.
     Each shot's aim is off by up to `(1 - Accuracy) x max_aim_error`
     degrees. Aims at the body, or the head if only that is visible.
   - *Moving*: spawners set a start state; "stationary attack" droids hold
     their spot unless they lose sight of the enemy. Others keep between
     `ReadyDistance` and `SafeDistance`, move to a path point with a clear
     shot when they lose one, and give up a move after `MaxManuverTime`.
     Shot at in the open, a droid runs to a free cover point that faces
     the enemy and crouches between bursts; after `CoverBoredomTime` it
     looks for another.
   - *Friends*: characters only collide with the level, so `Battle`
     pushes apart droids that overlap, and a droid holds fire while a
     friend stands within 50 uu of its line of fire (it moves if that
     lasts).
   - *Forgetting*: `EnemyMemoryTime` without seeing the enemy, then it
     goes to look where it was last seen.
   - *Spawning*: `PawnFactory` actors (with their `SpawnPoint`s) make the
     droids. One switches on when the player walks into a `Trigger` whose
     event is the factory's tag. Factories only level scripts can start
     switch on when the player comes within `activate_unscripted_within`
     (a stand-in until scripts run).
   `Battle` runs all of this for a map; the game plugin and `rc_droidsim`
   both use it.

3. **The game (`rc_enemies`).** `EnemiesPlugin` (see `docs/PLUGINS.md`):
   - On `OnEnter(Loading)` it reads the map's paths and spawners and the
     settings file through `GamePaths::data_file`, so a mod's file wins.
   - Each frame (`GameSet::Simulate`, before `CombatSet::Fire`) it steps
     the `Battle` with the `Player`'s position (eyes from `PlayerCamera`)
     and `LevelCollision`, spawns droid entities with `Health`,
     `Team::Separatist` and `Hittable` (35 x 80 uu, 50 crouched), and
     turns each shot into `rc_combat::FireShot` with the weapon named in
     the settings. The weapons part flies the bolts and applies damage.
   - It reads `Damage` (the droid reacts, maybe runs for cover) and
     `Died` (the droid stops and plays its death animation, and its body
     stays), and hears the player's `FireShot`s.
   - `MusicMood::Battle` while any droid is alerted or attacking, `Calm`
     otherwise and on leaving the level.
   - Looks: the `[look]` section names the mesh (`BattleDroid` in
     `droid.ukx`) and which animation plays for idle, walk, run, back,
     sidesteps, fire, crouch, crouched fire and death. Every droid's points
     are posed on the CPU each frame with `rc_anim` (as `rc_character`
     does) and copied into its own Bevy mesh; 848 points each, so a dozen
     droids cost little. The level's light is baked into its surfaces and
     the scene has no ambient light, so droids get a steady share of their
     own colour (`ambient`, a guess standing in for each zone's ambient
     light, which we don't read yet).
   - VR and other platforms: nothing here reads input or the screen; the
     player is found by its `Player` and `PlayerCamera` markers, so a VR
     camera works the same. No platform-specific code.

**Checking without looking.** `rc_droidsim` (no window) and
`rc_droidview --check` (a window) run a stand-in player into a spawner's
trigger on a real map and log every spawn, sighting, state change, route,
shot and death, then print CHECK lines. `rc_droidview --check` also
captures the window with the droids shown and hidden, counts the pixels
that changed and compares their colour with the skin picture (it must not
be black, washed out, magenta or a different tint). `cargo test -p
rc_enemies` runs the plugin with the real combat plugin in a test room: a
droid spawns, spots the player in under a second, fires bursts that
sometimes hit, and dies to the player's DC-17.

## Weapons, shots and damage (`rc_combat`, `rc_shoot`)

`rc_combat` is the game's weapons and health, as a Bevy plugin
(`CombatPlugin`) that the game program adds. It follows `docs/PLUGINS.md`:
it reads input only from `ActionState` (fire, reload, next and previous
weapon), the player from `Player` and `PlayerCamera`, the level from
`LevelCollision`, and writes the display's `PlayerVitals` and
`WeaponReadout`. The numbers are the original game's (see
`docs/RESEARCH.md`, "Weapons and damage").

### The pieces

- `data.rs` reads our settings files (a small part of TOML). A misspelt
  setting is an error that names the line, so a mod's typo is reported.
- `weapon.rs`: a weapon's numbers (`WeaponParams`, read from
  `data/weapons/<id>.toml`) and its state (`WeaponState`: magazine, spare
  ammo, time to the next shot, reload). Firing adds the time between shots
  instead of setting it, so 0.1 s is exactly 10 shots a second at any frame
  rate.
- `shot.rs`: where a shot starts, the aim error, how a bolt flies (exact
  for any frame time), and whether a line hits a character's cylinder. This
  is in the game's own units and axes.
- `health.rs`: `Health`, `Shield`, `Team`, and the `Damage` and `Died`
  messages. Damage goes to the shield first.
- `fire.rs`: the player's trigger, `FireShot` from anyone, bolts in flight,
  and the sounds. `damage.rs` applies `Damage`.
- `view.rs`: the gun in the player's view (the game's own model and
  textures) and the bolts' glow.

### How other parts use it

- To damage anything: send `Damage { target, amount, kind, instigator,
  point, direction }`. Anything with `Health` (and optionally `Shield`) can
  take it. `Died` is sent once at 0 health, and the entity gets `Dead`.
- To fire a weapon (droids, squad mates): send `FireShot { shooter,
  origin, direction, weapon }`. `weapon` is a file name from
  `data/weapons`. The plugin adds the weapon's aim error, plays its sound,
  flies the bolt and sends `ShotLanded` when it ends.
- To be hit: add `Hittable` (the cylinder) and `Team`.
- Recoil: each player shot sends `ViewKick { pitch, yaw }` (radians). The
  player's view applies it; a VR mod can ignore it.

### Modding

Every weapon is one file in `data/weapons`. A mod folder's file with the
same name replaces it, and a new file is a new weapon. The player's health,
shield and starting weapons are in `data/combat/player.toml`.

### `rc_shoot`: the firing range

`rc_shoot` puts the player in the movement test room with three battle
droids: one straight ahead, one to the right, and one far away that fires
back every 2 seconds. Downed droids tip over and stand up again after 4
seconds. The window title shows the weapon, ammo, health and shield. The
keys are mapped to actions in `rc_shoot` itself; the real game reads them
from `data/input/bindings.toml`.

`rc_shoot --script` plays a fixed list of actions and checks 12 things: the
files read, 5 rifle hits down the droid ahead, the magazine and spare counts
before and after a 1.5 s reload, the pistol's two shots, the far droid's
shots taking shield before health, and the picture. For the picture it
captures the window and checks that the gun's corner has many colours and
no magenta (magenta means a missing material), and that the middle of the
view is the droid's colour.

### Our choices (not from the game)

- The DC-17m has no instant-hit setting in the game, so it fires bolts with
  its bolt's damage.
- How the view kick settles back is in the game's compiled engine. In
  `rc_shoot` it fades at exp(-8 t).
- The fire sound is full volume for the player and quieter with distance for
  others; the game uses its 3D sound system.
- Weapon switching is instant; the game plays holster and draw animations
  (0.37 s and 0.67 s for the DC-17m).
- The gun's place in view (`[model] offset`) and the bolt colour are ours.
  The game draws the gun with first-person arms, which aren't done yet.

### Fitting the long-term goals

- **Modding:** all numbers are in text files, each line naming where in the
  game the value came from.
- **VR:** a shot is a start point and a direction, not a screen position, so
  a tracked controller can be the gun later. The kick is a message the view
  can ignore.
- **Windows, Linux and Android:** plain Rust and Bevy, no operating-system
  code. Tested on Linux in the cloud only.

## The game program (`rc_game`)

`rc_game` is built from Bevy plugins. Parts don't call each other; they
share the types in `rc_game_core`. How a part plugs in is in
`docs/PLUGINS.md`.

**Screens.** `GameState` is `Boot` (read settings) then `MainMenu`, `Loading`
and `InGame`. While in `InGame` a second state, `Play`, is `Running` or
`Paused`, so pausing never counts as leaving the level. Gameplay systems run
only while `Running`. Everything spawned for a level carries
`DespawnOnExit(GameState::InGame)` and is cleared on the way back to the
menu (the script checks that no player, camera or level mesh is left).

**Loading.** The menu sets `CurrentLevel` and enters `Loading`. Any part with
slow work calls `LoadingGate::wait("name")` and later `done("name")` or
`fail("name", reason)`. The shell moves to `InGame` when nothing is waiting
(and at least one frame has passed, so every part has had its turn); on a
failure it goes back to the menu and shows the reason.

**Input.** Devices are read in one place (`input.rs`) into `ActionState`:
which actions are held this frame, a movement direction and a turn amount.
Gameplay reads only that. The bindings file maps keys, mouse buttons and
wheel, gamepad buttons and sticks to actions; touch screens move with a drag
on the left half, look with a drag on the right half, and fire with a tap.
The keyboard defaults are the original's `DefaultCtrl.ini`. A scripted run
or a VR mod writes `ActionState` itself instead.

**The player.** `player.rs` moves a body with `rc_movement` (as `rc_walk`
does) from `ActionState`, and puts a separate camera at its eyes. The
camera is not a child of the body, so a VR headset can replace it.

**Menus.** *Placeholders:* these menus and the display below are our own
stand-ins until the original's menus and HUD are read from the install
(`docs/ORIGINAL_MENUS.md`). Each menu is a page: a title and a list of items, drawn with
Bevy's UI and the theme file's colours. Pages: main, campaign list, a
campaign's levels, options, pause, and "game files not found". The mission
names, order and loading text come from the game's own
`GameData/System/LevelLoadingInfo.int` (plain text with numbered entries
like `MapName[3]="GEO_03A"`), read at start-up, never copied into the repo.
`data/levels/campaigns.toml` only groups them by name prefix. Sizes are a
share of the screen height, or of its width when that is smaller, so the
menus fit a phone or a monitor.

**The original main menu.** When the install's menu packages can be read,
`original_menu.rs` draws the original main menu instead of the placeholder
main page (the placeholder pages stay for everything else). The layout comes
from the menu class's default values, read by `rc_package::class` and
filled in by the new `rc_menus` crate, which knows nothing about Bevy:

- `rc_menus::library` finds packages and classes in `GameData` and merges
  a class's defaults with its parents' (child values win). It keeps the
  order properties were declared in, because that is the drawing order
  (a child that redeclares a widget moves it to its own place).
- `rc_menus::layout::build` turns the defaults into widgets (pictures,
  texts, buttons), applying the menu system's rules from the comments in
  `MenuBase`: positions are interpolated down arrays, other values are
  copied, `Style` fills from a template, sub-widgets fill from each other
  (blurred to focused and so on) and the `DefaultMenu...` blocks fill the
  rest. A value that is already set is never overwritten. Empty colours
  mean opaque white, empty scales mean 1.
- `rc_menus::quads` turns widgets into textured rectangles in a 0..1 menu
  area laid out for 640x480. `rc_menus::render` paints the same rectangles
  in software; the script compares that picture with a screenshot.
- `rc_menus::gpu` makes copies of pictures for Bevy: each render style
  (normal, masked, translucent or additive, alpha) and each tint gets its
  own copy, with transparency adjusted so Bevy's linear-light blending
  gives the original's sRGB result over black.
- `rc_menus::screens` holds what each menu's script does, rewritten by
  hand (the main menu and the yes/no question so far).

The menu area is the largest centred 4:3 box in the window; the original's
positions assume 4:3, so nothing is stretched on wide or tall screens.
Focus moves with the menu actions (`MenuUp`, `MenuDown`, `MenuAccept`,
`MenuBack`), so gamepads and keyboards work the same; the mouse moves focus
only when it actually moves. Details and what is left are in
`docs/ORIGINAL_MENUS.md`.

**Screen shapes and controls.** The view is "Hor+": `video.fov` is the
angle across a 4:3 screen, as in the original, and wider screens see more
at the sides. Keyboard, mouse, gamepads and touch all work at once. Details,
numbers and what is still to do (edge stretching on ultrawides, controller
pictures, gyro) are in `docs/DISPLAY_AND_INPUT.md`.

**On-screen display.** Corner brackets like the commando's visor, a
four-bar crosshair whose gap follows the weapon's spread and turns red on a
target, shield and health bars, the ammo counter, a squad list and short
messages. It reads only the readout resources, and logs its numbers every
two seconds.

**Music.** `music.rs` plays the game's Ogg Vorbis files straight from
`GameData/Music` with Bevy's audio. `data/music/music.toml` picks the menu
track and, per map name prefix, a calm and a battle list; `MusicMood`
chooses between them. Each track is despawned when it ends and the next one
starts.

**Mods.** Everything we ship as data (controls, theme, music, campaigns,
movement) is found through `GamePaths::data_file`, which looks in each
enabled mod's folder (`mods/<name>/`) before `data/`. Lists that add up (the
campaigns file) read every copy.

**The stand-in level.** Until the levels part plugs in its own plugin,
`stand_in_level.rs` loads a map the way `rc_walk --map` does (its collision
code is a copy of `rc_walk/src/level.rs`). It is meant to be deleted then.

**Checking without looking.** `rc_game --script` presses the same actions a
player would and checks each step: menu pages, loading, the player moving
430 units in a second of walking, nothing moving while paused, the level
being cleared, options changing. It also captures the window twice and
measures colours: the main menu must contain amber text, and in the level
at least 30% of the screen must be lit with amber crosshair pixels at the
centre. With the original menus, it also paints the main menu and the quit
question in software from the original's pictures, and compares each with
a screenshot of the game shrunk to 320x240: the mean difference must stay
under 12 of 255 and fewer than 5% of pixels may differ by more than 40.
centre.

## Controllers, gyro and the Panini view (`rc_pads`, `rc_panini`)

The details and numbers are in `docs/DISPLAY_AND_INPUT.md`. The shape:

- **One reader for controllers.** `rc_pads::sdl` starts SDL3 with no
  window and turns its controller events into the same Bevy gamepad
  messages gilrs sends, so Bevy keeps each controller's `Gamepad`
  component up to date and the game's input code doesn't care which reader
  ran. `rc_game` disables Bevy's `GilrsPlugin` only when SDL started.
  SDL lives in a non-send resource, so it stays on the main thread.
- **Gyro readings are messages.** `GyroSample { source, rate, accel, dt }`
  comes from the SDL reader, and later from Android's sensor service.
  `PadsPlugin` runs each source through its own `GyroFilter` and puts this
  frame's turn for the controller used last (or the built-in gyro) in
  `GyroLook`. `rc_game`'s input code decides whether to use it (mode,
  ratchet, toggle) and adds it to `ActionState::look`. So a VR mod or
  another sensor only needs to send `GyroSample`s or write
  `ActionState`.
- **Prompts are data.** `InputPrompts` (in `rc_game_core`, so every part
  can read it) is rebuilt when the last-used device, the controller's kind
  or the bindings change, from `bindings.toml` and `button_names.toml`.
- **Rebinding saves differences.** `Bindings::changes_from(base)` writes
  only the actions that differ from the shipped (and modded) bindings, and
  `with_changes` lays them back on top at start-up.
- **Panini** is a `FullscreenMaterial` on the player's camera. After
  `place_camera` sets the normal field of view, `rc_game/src/panini.rs`
  widens it with `Fit::new` and updates the camera's `PaniniView`. The
  shader and `Fit::source` do the same maths, which the tests check.
## Saving, loading and checkpoints (`rc_game_core::save`, `rc_save`)

**One interface, one section per part.** A save file is a plain TOML text
file: a `[save]` header (format number, map, last checkpoint, when, time
played) and then one `[section]` per part of the game. Each part writes its
own section by implementing `SavePart` (`rc_game_core/src/save.rs`): a
struct with `Serialize`/`Deserialize`, a `save` that reads it out of the
game and a `load` that puts it back, registered with
`app.add_save_part::<T>()`. The save plugin never knows what's inside a
section. Sections so far: `[player]` (position, speed, on the ground,
crouched, facing; in `rc_game/src/player.rs`), `[combat]` (health, shield,
weapons and ammo; `rc_combat/src/save.rs`), `[enemies]` (the battle clock,
each spawner's state and each living droid; `rc_enemies/src/save.rs`, using
`rc_ai::Battle::snapshot` and `restore`), `[squad]` (stance, orders, each
mate's place, job, revive, health and shield; `rc_squad_plugin/src/save.rs`,
applied right after the squad appears on its first frame), `[checkpoints]`
(checkpoints reached on this map; `rc_save`).

**Loading reloads the map.** Loading a save sets `CurrentLevel` to the
save's map and goes through `Loading` exactly like the menu does, so the
level is always built fresh. On the first frame of play, after every
`OnEnter(InGame)` has run and before input is read, each part's `load` gets
its section, in the order the parts were registered. So parts only put back
what changed since the map started, and never have to undo half a level.
While this happens the resource `RestoringSave` exists; level-start things
(the level-start autosave, opening scripts) check for it and stay quiet. A
section the file doesn't have calls `load_missing` (the level stays as it
starts), and a section no part claims is logged and ignored, so older and
newer saves still load. A file with a higher format number than ours is
refused with a message.

**Destroyed droids stay destroyed** because each spawner's count of droids
made is saved; only living droids are listed. A droid that was fighting
comes back alerted toward where it last saw the player. Its plan (route,
cover spot, burst) starts over: our choice, since those plans are rebuilt
every few seconds anyway.

**Checkpoints.** Entering a map from the menu saves `checkpoint.toml`
under the name "level start". Level scripts send `CheckpointReached { name
}`; the first time each name arrives it is added to `[checkpoints]` and
`checkpoint.toml` is written again. Repeats are ignored, also after a load.
The original does this with placeable `CTGame.Checkpoint` actors. When an
event matching a checkpoint's `Tag` fires, it waits until the player is up
(not incapacitated) and autosaves. `rc_level::checkpoints::read` lists a
map's checkpoints and the actors whose `Event` names each tag. The 50
campaign maps with checkpoints hold 144: 56 fired by a walk-in `Trigger`
and 88 by level scripts. The level scripts part fires them and sends
`CheckpointReached` with the checkpoint actor's name (like `checkpoint1`).
`DefaultCtrl.ini` binds quicksave to F5 and quickload to F9, as we do.

**Files and folder.** `quicksave.toml`, `checkpoint.toml` and
`save_NAME.toml` (named saves, for a save menu; only letters, digits, `-`
and `_` reach the file name). The folder is `[save] folder` in
`settings.toml`, else `%APPDATA%\rc_game\saves` on Windows,
`$XDG_DATA_HOME/rc_game/saves` or `~/.local/share/rc_game/saves` on Linux,
and `saves` beside the settings file elsewhere (the Android app will set
its own folder). Never the game's install. Each file is written to
`NAME.toml.writing` and then swapped in, so a crash can't leave half a save.
`--script` runs save into `logs/script_saves`, emptied at the start.

**Keys and menus.** `Action::QuickSave` (F5) and `Action::QuickLoad` (F9),
the original's keys.
`SaveList` lists the saves newest first for a Load Game screen; a menu
sends `LoadGame { slot }` or `SaveGame { slot }` and gets `SaveResult` back.
Every save and load is logged with the prefix `save:` and a one-line summary
of each section, so a round trip can be checked from the log.

## Automatic checks on GitHub (`.github/workflows/ci.yml`)

Every pull request, and every push to `main`, is built and tested by GitHub's
own machines on Windows and Linux. A pull request shows a green tick or a red
cross for each. The steps are `cargo build --workspace --all-targets --locked`
and `cargo test --workspace --locked`. `--locked` means the exact library
versions in `Cargo.lock` are used. Linux also installs the system libraries
Bevy needs (sound, gamepads, windows). The build folder is cached between
runs, so a run that changes little is much quicker than the first.

The game is never on GitHub, so `RC_GAME_DIR` is not set there. Unit tests
must not need game files. A future test that does should check for
`RC_GAME_DIR`, print "skipped: no game folder" and pass when it isn't set.

A third job runs `cargo fmt --check` (code layout) and `cargo clippy` (common
mistakes). It only reports: its problems show as warnings and never turn the
run red. The checks do not open windows or draw anything, so the Bevy
programs' own `--script` and screenshot checks are not run there, and Android
is not built yet.
