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
| `rc_movers` | Doors, lifts and hatches: runs a map's movers through their keys, makes their collision move with them, carries the player and squad mates on lifts, and runs triggers. No Bevy. `rc_mover_check` runs every mover in every map without a window and logs the numbers. |
| `rc_anim` | Poses a skeleton from an animation and moves a mesh's points with it ("skinning"). Plain maths, no Bevy. |
| `rc_character` | 3D Bevy program: shows one character from the game with its skin, playing one animation. |
| `rc_soundtest` | Bevy program with no window: plays one weapon sound and one ambient loop and logs proof that they played. |
| `rc_squad` | Delta Squad: Sev, Fixer and Scorch following the player and obeying squad orders, with the game's numbers. No Bevy. |
| `rc_squad_plugin` | `SquadPlugin`, the squad in the Bevy game, and the `rc_squad_sim` example (a windowless squad test on a real map). |
| `rc_hologram` | `HologramPlugin`: holograms shimmer and flicker like the original's `DynamicHologram` shader, with the game's own glow and noise pictures. Check: `hologram_check` example in `rc_level_viewer`. |
| `rc_cast` | `CastPlugin`, the characters placed in a map that no other part owns (the Kaminoans, young clones, troopers and commandos of the cutscenes, animated props, troopers and vehicles standing about in missions): drawn with the game's models and moved by the level scripts. |
| `rc_shadows` | `ShadowsPlugin`, character shadows the way the original draws them: each marked character's shape laid on the floor from a light high above (`CastsShadow`, `data/shadows/shadows.toml`). The `shadow_check` example checks it without the game. |
| `rc_game_core` | The shared pieces every part of the game plugs into: screens, folders, input actions, display readouts, loading, collision, player markers, units, the log, and the save interface (`save`). Bevy, but no window. |
| `rc_movers_plugin` | `MoversPlugin`, doors and lifts in `rc_game`: runs `rc_movers` on the level part's movers, puts them in `LevelCollision`, moves their drawn parts, and opens doors by event name (`LevelMovers::send`). |
| `rc_scripts` | Level scripts: runs a map's own scripting (scripted triggers and sequences, touch triggers, counters, true/false conditions, checkpoints, destructible props) as events and tags. No Bevy. `rc_script_check` reads every map's scripts and runs them without a window. |
| `rc_scripts_plugin` | `ScriptsPlugin`, level scripts in `rc_game`: carries out what the scripts ask (doors, barricades, the player's health, bacta dispensers, prompts, objectives, checkpoints) and sends every event as a `ScriptEvent`. |
| `rc_save` | Saving, loading and checkpoints: `SavePlugin` writes and reads the save files, quicksave and quickload, and saves at checkpoints. Bevy, but no window. |
| `rc_level_plugin` | The level part of the game: `LevelPlugin` loads the chosen map into `rc_game` (its look, sky, collision with props, doors and invisible walls, and the player start). |
| `rc_game` | The game: one program with the menus, settings, controls, player, on-screen display and music. The other parts (levels, weapons, enemies, squad) plug into it. |
| `rc_nav` | The level's path network (path nodes and the editor's links between them), shortest routes, and walking a route with the player's physics. Shared by enemies and the squad. No Bevy. Tool: `rc_navcheck`. |
| `rc_ai` | What enemies decide: settings from `data/enemies/*.toml`, the map's spawners, one fighter's brain, and a whole fight on a map (`Battle`). No Bevy. Tool: `rc_droidsim` (droids against a stand-in player, no window). |
| `rc_pads` | Controllers: which kind each one is, button names for prompts, gyro clean-up, and an SDL3 reader (built from source) for controllers and gyros. Bevy, no window. Tool: `rc_padtest`. |
| `rc_panini` | The optional Panini view for very wide screens: a full-screen pass that bends the 3D view. Check: `panini_check` example. |
| `rc_visor` | The visor's modes (normal, tactical, low light): `VisorPlugin` makes squad mates and pickups glow in tactical mode, in the colours the install gives them, and switches modes with CYCLE VISOR MODE. Check: `visor_check` example. |
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

**When alpha means see-through.** Low alpha does not always mean a hole.
Many Republic Commando textures (DXT3/DXT5 walls, floors and props) keep a
shine (specular) mask in their alpha channel. Like the original engine, the
level viewer only cuts holes where the material asks for it: the texture has
`bMasked` or `bAlphaTexture`, a `FinalBlend` around it has `AlphaTest` or
alpha blending (`FrameBufferBlending` 2), a `Shader` has an `Opacity` map or
masked output (`OutputBlending` 1), or a BSP surface has the "masked" flag
(0x2). Everything else is drawn solid (`materials.rs`, `Picture::has_alpha`
and `alpha_pixels`). Before 2026-11-01 any low-alpha pixel cut a hole, which
made whole floors and walls vanish in-game.

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
over 10 (11.2). Which way each channel points was measured later, see "Drawing bump maps":
in these two formats the two tilts are the other way round from their
Direct3D names, so `materials.rs` swaps them before drawing.

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
  empty property list, a count, then that many 4-byte colours (red, green,
  blue, alpha; see "How baked light is drawn"). It read correctly on all
  1,267 instances in the four maps. After the colours come a 4-byte number
  (2 when the next list is filled, else 0), the lights baked in (a count,
  then per light an object reference and a 4-byte 1), a second per-corner
  list (count, 4 bytes each), and a 4-byte number; all 556 instances on
  yyy_06a read to their last byte (`actors::read_baked_light`). The second
  list is empty except on props with bump-mapped (`HsBump...`) materials,
  where it holds the light's direction for the bump map: red, green, blue
  are x, y, z in the mesh's own axes (0-255 for -1..1; length 0.99 median;
  it points at the lights, 0.96 on unturned props), and alpha is the light's
  strength before facing (the colour's brightest channel is 0.75 x alpha x
  how squarely the corner faces that direction, middle half 0.69-0.77, over
  62,507 corners). Many of the
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
live_light_on_props = 1         # real-time light on props and terrain (0 = off)
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
- Props (and terrain) use their per-corner colours as Bevy vertex colours on
  our `PropMaterial` (`prop_light.rs` and `prop_light.wesl`): Bevy's
  standard material with a small shader of ours that draws
  `texture x corner colour + texture x real-time lights x live_light_on_props`.
  Bevy's own material would treat vertex colours as paint, so a lamp's light
  would be multiplied by the dark baked colour; the shader keeps the two
  apart. With no real-time light near, a prop is drawn exactly as the old
  unlit material drew it (geo_01a and yyy_06a pictures identical to the
  pixel). Sky props and `live_light_on_props = 0` still use the plain unlit
  material. Cost: the same as any lit surface, one pass, no extra textures,
  so fine for phones and VR. Every app that draws a level adds
  `PropLightPlugin` (the game's `LevelPlugin` does). The colours are stored red, green,
  blue, alpha (checked against the colour of the lights each prop lists;
  they were read as blue, green, red until 2026-11-01, which drew geo_04a's
  orange canyon blue-grey).
- Zone ambient light is **not** in the props' stored colours: corners that
  face away from every light store 0. The game adds it when drawing, so
  `scene.rs` (`zone_ambients`) adds it per prop, by the zone at the prop's
  position (a zone's settings sit on the `ZoneInfo` standing in it; zones
  without one use the `LevelInfo`). It adds half the ambient, because the
  lightmaps hold half (their darkest texels are half the zone's ambient,
  e.g. 21, 21, 27 for 36, 40, 50 on yyy_06a, with the sun on top at the same
  scale as on props); both are then drawn at double strength. The ambient's
  colour is the zone's own stored `AmbientVector` (Republic Commando only),
  which equals Unreal's hue and saturation (`hue_saturation_to_rgb`) times
  `brightness_curve(AmbientBrightness)` on every zone checked (6 maps).
  A prop whose position is inside solid rock or wall (BSP zone 0, outside
  every zone: 245 of 556 on yyy_06a, the whole start tunnel, which is built
  from props) takes the zone most of its corners face instead: each corner
  is moved 8 units off the surface along its normal and looked up in the
  BSP (`prop_zone`). Before that they got no ambient at all and the tunnel
  was black away from the lamps.
- Bump-mapped props (those with the light-direction list above) get the
  zone's whole ambient instead of half (`BUMP_PROP_AMBIENT`). The original
  draws them with its bump shader, and against the user's original-game
  screenshot of yyy_06a's start tunnel, half left the parts lit only by the
  ambient at about half the original's brightness while the lamp-lit floor
  was close. With the whole ambient, six areas of the view (near and middle
  floor, far tunnel, both walls, ceiling) are within about 20% of the
  original (middle values 19-47 against 18-44). One screenshot, so it is a
  calibration, not a proof. The bump maps themselves (the pixel-by-pixel
  shading the direction list is for) are drawn since 2026-11-09, see
  "Drawing bump maps".
- Colours from hue and saturation follow Unreal's model: hue fades in a
  straight line red to green to blue (so hue 30 is 65% red, 35% green, not
  orange-yellow), checked against the light baked into props on geo_04a,
  ras_01a and yyy_06a.
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

**Light by place (characters, the gun, the helmet).** Squad mates, droids,
the first-person gun and the helmet frame carry no baked light, and the
original draws them by where they are: dim in a dark tunnel, bright in the
sun. Until 2026-11-09 ours used fixed guesses (`ambient = 0.5`,
`arms.brightness = 0.75`, helmet `brightness = 0.5`), so in YYY_06A's tunnel
the gun was 2 to 3.5 times brighter than in the user's original screenshot
and the squad about twice as bright.

- `rc_level_viewer::probe::LightProbe` (no Bevy) is built once per level
  from `scene::Scene`: points spread over every lit surface (about every 3
  Bevy units on big triangles, one per small triangle) with the surface's
  facing and its light, from the prop corner colours (zone ambient
  included), the terrain's corner colours and the lightmaps, on the game's
  scale (stored value x drawing gain / 255, so 1.0 shows a texture as
  stored). They sit in a grid of 6-unit cells. A question ("how much light
  at this point?") averages the samples within 6 units that face the point,
  weighted by how squarely they face it and 1 / (1 + distance²); with none
  it looks 12, then 24 units away. Packed to 20 bytes a sample: 0.3 to 1.1
  million samples, 6 to 22 MB, built in 30 to 140 ms per map (yyy_06a,
  geo_04a, ras_01a, geo_01a). `cargo run -p rc_level_viewer --example
  probe_check -- MAP` prints the light at a map's start.
- `rc_level_plugin` puts it in `rc_game_core::PlaceLight` while a level is
  on. Any part tags an entity `rc_game_core::LitByPlace` (`own`, `scale`,
  `height`, `made_at`, and whether it sets the glow or the base colour);
  `rc_level_plugin::place_light` gives the entity its own copies of its
  materials and every 0.1 s sets them to `own + scale x light`, eased over
  about a quarter of a second, measured `height` above the entity. The
  log says the light at the player's eyes every 5 s (`light: at the
  player's eyes ...`).
- Who uses it, and the numbers (all in data files): squad mates
  (`squad/looks.toml` `light_scale = 1.1`) and enemies (`light_scale` in
  `enemies/*.toml`, default 1.1) as their glow, so real-time lights still
  add; the gun (`arms.light_scale = 0.85` in each weapon file) and the
  helmet frame (`ui/helmet.toml`: its own `brightness = 0.5` plus
  `light_scale = 0.8`) as their colour. The old fixed values are kept for
  when there's no level light (and for `arms.light_scale = 0`).
- Matched to two original screenshots, YYY_06A's start tunnel (light 0.35)
  and GEO_04A's canyon (about 0.45-0.5, spot not exactly known). Middle
  values (0-255), original / before / after: tunnel gun body 22 / 76 / 25,
  gun side 56 / 113 / 43, helmet top 58 / 35 / 59, helmet lower left 66 /
  40 / 68; canyon gun 43 / 76 / 34 and 63 / 113 / 55, helmet 69 / 35 / 65
  and 110 / 40 / 74. Squad mates in the tunnel: brightest 2% of their
  pixels 67-95 in the original, 153 before, about 105 after (one mate,
  another spot).
- It is a fill light only: no direction, so a character in the sun isn't
  brighter on the sunny side, and the original's per-light shading isn't
  copied. The scales come from two screenshots and are a calibration, not
  the original's formula.

**Speed (and VR).** The viewer uses `live_lights = dynamic` by default: the
91-98% of lights that are baked cost nothing at run time, and only a handful
of real-time lights remain (2 in `geo_01a`, 19 in `pro`). That suits a later
VR version, which has to draw everything twice (once per eye) at 90 frames
per second or more.

**Known gaps.** Real-time light on props uses the same placeholder strength
as on walls, and whether the original lit props per corner or per pixel is
not known (we light per pixel, which looks smoother). Shadow bitmaps for the dynamic lights are read
past but not used. What the lightmap pages' alpha channel holds is unknown
(it is ignored). Lightmaps are sampled with plain linear filtering.

**Phones and other platforms.** Textures, bump maps included, are decoded to
plain RGBA on the CPU before Bevy sees them, so no graphics card has to
understand the game's DXT compression. Android GPUs often can't, so this path
works on Windows, Linux and Android alike. The cost is memory (4 bytes per
pixel instead of 0.5-1); re-compressing into a phone-friendly format (ETC2 or
ASTC) can come later if memory gets tight. Baked lighting with few real-time
lights is also the cheap option for phone GPUs.

## Drawing bump maps (`rc_level_viewer/src/bump.rs`, `bump_lit.wesl`)

Props whose material has a bump map (`HsBump...` materials name it
`NormalMap`, with `BumpUVScale` for how often it repeats; a `Shader` names
it `Bumpmap`) are drawn with it, so walls show their rivets, panels and
grooves. `materials.rs` finds and decodes the bump map next to the main
picture (`Picture::bump`).

**What the game gives us.** For these props the map stores, per corner,
the baked light colour (as for every prop) and the direction the main
light comes from, with its strength (see "Lights in maps"). The corner's
colour is 0.75 x strength x how squarely the corner faces that direction.
So per pixel we draw:

```
light = stored corner colour (with the zone's ambient)
      + light colour x (pixel facing - corner facing)
```

"Facing" is how squarely the surface faces the light (0 to 1): the bump
map's tilt for the pixel, the plain surface for the corner. The light's
colour is the corner's own colour where it is lit enough to tell (16 of
255 or more), else the prop's average lit colour, else white; its
strength is 0.75 x the stored strength. A pixel the bump map leaves flat
gets exactly the stored colour, so a surface is on average as bright as
before and the brightness calibrated against the original game holds.
How the original's shader mixes these exactly is compiled code we
haven't read; this rule is ours, built to fit the stored numbers.

**Which way the bump maps point (measured).** Nothing documents it.
A tilt map is the slope of some height map, and slopes of a height map
must fit together (how the tilt along the width changes going down equals
how the tilt along the height changes going across). Trying the four ways
of reading the two tilt channels shows which is right:

| Bump maps | Best reading | How clear |
|---|---|---|
| Colour normal maps on level props (16 used by yyy_06a, geo_01a, ras_01a) | red along the width, green down the picture, as stored | all 16, error 0.05-0.79 of the next best |
| L6V5U5 (format 14, 104 maps, mostly characters and weapons) | the two swapped | 59 best, 49 clearly (half the error or less); 2 clearly another way |
| V8U8 (format 12, 29 maps) | mixed: 9 clearly swapped, 4 clearly not | read like L6V5U5 |

Whether a tilt means a bump or a dent comes from the main picture: grooves
are painted darker, so where tilts point inwards (the height map dips) the
picture should be darker. For the 13 level bump maps that repeat like their
picture, 11 agree with "red/green lean towards where the surface faces"
(correlation +0.2 to +0.6), 1 is about 0, 1 the other way (-0.4). That is
the usual normal-map convention, with green pointing down the picture
(Direct3D's way). None of the bump maps on level props in these three maps
are in the two-byte formats.

**The shader.** Our own, `bump_lit.wesl`, a Bevy `Material`
(`BumpLitMaterial`). Each corner carries its stored colour, the direction
along the picture's width (Bevy's tangent attribute, worked out on the CPU
from positions and picture coordinates, `bump::surface_tangents`), the
light direction and the light colour (two attributes of our own). The
stored colours are turned into linear light per corner and blended across
the triangle exactly as flat props are, so `bump_strength = 0` draws the
same picture as the flat props (checked, below). Only opaque, one-sided
props use it; see-through, glowing and two-sided ones stay flat.
Real-time lights (moving or switched lights, the helmet lamp) are added on
top with Bevy's own `apply_pbr_lighting`, using the bump-tilted surface, so
a lamp shows the bumps too; `BumpParams::live_light` carries
`live_light_on_props` (0 in the sky). With no real-time light near, that
adds nothing (yyy_06a start view identical to the pixel with it on or off).

**Cost.** One extra texture read, a few dozen sums and three powers per
pixel, no loops, plus Bevy's usual work for the few real-time lights near:
within what phone GPUs and VR
headsets manage. Bump maps are decoded to plain RGBA on the CPU like every
other texture. Settings in `lighting.cfg`:

```
bump_maps = true      # false draws bump-mapped props flat
bump_strength = 1     # 0 flat (through the bump shader), up to 2
```

**How it is checked (colours, not outlines).** `rc_level_viewer MAP
--view-bump` puts the camera in front of the bump-mapped surface where the
stored light is strongest. `rc_capture_compare A.bmp B.bmp --same|--bumped`
compares two captures by the average colour of each square of a 4 x 4
grid, by how many pixels changed and by "detail" (how much neighbouring
pixels differ):

- flat (`bump_maps = false`) vs `bump_strength = 0` must be the same
  (squares within 2 of 255, 99% of pixels within 3): proves the shader's
  light maths matches the flat props';
- flat vs bump maps on must keep every square's colour within 4, change at
  least 5% of pixels and add at least 3% detail.

Results are in the MODLOG entry of 2026-11-09.

**Not done.** BSP walls with a bump-mapped material (1-50 polygons per map)
store no light direction, so they stay flat. The bump maps' extra channel
(alpha, sometimes named "spec") and the materials' shine (`Specularity`,
`Specular`) are not drawn. `DiffUVScale` and `BlendTexture` on
`HsBumpDiffBlend...` materials aren't used (that was already so). Characters
and weapons don't use bump maps yet. Not compared side by side with the
original game.

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
     follows the slope keeping the same direction and speed across the
     ground, only rising with it. On a wall it tries to step up: go up 35,
     forward (at least 8 uu, see below), back down onto floor. If that fails
     it slides along the wall.
   - After moving, it looks for floor up to 35 below and snaps onto it (so
     stairs and slopes going down keep the feet on the ground), always 0.1
     above it; a box that has crept closer is lifted back. No floor means
     falling.
   - Every move stops 0.1 uu short of what it hits, measured straight out
     from the surface, and the rest of the move carries on from there.

   **Same at any frame rate (2026-11-09).** The player used to slow to a stop
   on some ramps and lips at high frame rates (the user's PC runs 240 fps),
   because a frame's move there is only a unit or two, and four things lost a
   fixed amount per frame or per hit: the 0.1 gap was taken off the move and
   never given back; that gap was measured along the move, so a move that
   grazed a slope ended up touching it and the next move "hit" it at once;
   following a slope slid along its surface, which turned each move a little
   downhill; and a step up only went forward as far as that frame's move,
   which at 240 fps ended on the sloped front of a low lip, too steep to
   stand on. `rc_walkcheck` (below) and the `high_frame_rates_*`,
   `slope_at_an_angle_keeps_speed_at_240` and
   `low_lip_with_sloped_front_at_240` tests check it. The 8 uu step-up
   minimum is our own number (about one 60 fps frame of running).
   - The speed afterwards is the distance really moved, so running into a wall
     stops you, like in the engine.
5. **Falling physics:** gravity -1100 and 35% steering, which can't push you
   past running speed. The move uses the average of the old and new speed,
   which makes jump heights exact at any frame rate (103 uu high, 0.86 s in
   the air). Landing on floor-like ground switches back to walking and
   reports the game's falling damage (not applied to health yet).
6. **Eye height** eases toward 60 (44.8 crouched) at the game's rate and
   absorbs steps and crouching so the camera doesn't jump.

### Checking a level for places the player gets stuck (`rc_walkcheck`)

`rc_walkcheck MAP [--fps N] [--near X,Y,Z RADIUS] [--link FROM TO]` (in
`rc_nav`, no window) loads a map's collision and its AI path network. The
level designers linked path points that an AI can walk straight between, so
the player should manage it too. For every walking link (not jumps, doors or
links blocked until a script opens them) a player starts on one end and runs
at the other, steering at it every frame, for twice the time it should take.
It writes `logs/rc_walkcheck.log`: `FAIL` for runs that never arrive, `SLOW`
for runs that took over 1.5 times the straight run or started inside the
level, with where the player was slowest. Some failures are real: paths made
for smaller characters (a commando is 168 uu tall) under low pipes and
through crawl spaces.

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

Keeping sweeps cheap (2026-10-30). A sweep first checks the box its whole
move covers against the box around all the world's triangles and stops
there if they don't touch. That matters because the level's collision is
many `TriangleWorld`s asked one after another (the fixed level, each door,
each prop a script can switch off): before this check a sweep far from a
door still looked at the triangles in the door's nearest grid column. Then
it only visits, row by row, the grid columns the move actually passes over
(a long diagonal line of sight no longer visits the whole square around
it), skips triangles whose own box is outside the move's box, and marks
triangles already seen with a per-thread stamp list instead of a new hash
set per sweep. A unit test compares the result with trying every triangle
on 3,000 random moves. On geo_01b this took collision from about 33 ms of
work per frame (1,000 sweeps looking at 550,000 triangles, with nobody
fighting) to about 0.5 ms. `rc_game` logs a `speed:` line every 5 s with
frames per second, the slowest frame, the number of entities, and the
collision work per frame (`rc_movement::triangles::take_counts`).

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
vehicles (the gunship you start in).

Which props are solid follows the game's settings (2026-10-30): a prop
with `bCollideActors` or `bBlockPlayers` off (from the map, or else its
class's defaults; on if neither says) is left out of collision
(`rc_level::class_look::blocks_players`), and so are the triangles of mesh
sections whose `EnableCollision` is off (`StaticMesh::section_collision`,
read from the mesh's `Materials` list). They are still drawn. Before, every
visible prop was solid, so you bumped into light shafts, cables and other
decorations. geo_01b: 32 walk-through props and about 50,000 section
triangles; its solid triangles went from 227,043 to 190,205. geo_01a: 68
walk-through props and about 15,000 section triangles.

Simple collision shapes (2026-11-10). After its vertex and index lists a
static mesh stores a reference to a `Model` (the same kind of object as a
map's BSP, read by `rc_level::bsp`): a few blocky boxes the artists put
around it. 1,720 of the game's 2,447 meshes have one. Then comes the
mesh's collision triangle list, which holds exactly the triangles of the
sections with collision on (checked on 2,295 meshes), so we don't read it.
For the 660 meshes with collision off on every section (explosive barrels,
floor and ceiling pipes on Raxus Prime, the jamming device's wrecked
model) that list is empty: the simple shape is all the game can collide
with, so `rc_level::collision::solid_local_triangles` uses it
(`read_simple_shape` fills `StaticMesh::simple_shape`). Meshes with
collision on keep their own triangles; the original may use the simple
shape for walking against those too (UE2's `UseSimpleBoxCollision`), but
its default lives in the engine's C++ code, so we can't read it and left
them as they were. Six Karma test meshes' shapes don't read (an older
layout); they keep no shape.

Shots and people (2026-11-10): `rc_level::class_look::blocks` reads what an
actor stops. People: `bCollideActors` and `bBlockPlayers`. Shots:
`bCollideActors` and `bBlockZeroExtentTraces`, and it is a shot target
(`bProjTarget`, on for every `Prop`, or it blocks actors and players). The
Geonosian shield wall pieces of geo_04c (`K_GEO04_WallComponent`, 89 of
them) have `bBlockPlayers`, `bBlockActors` and `bBlockNonZeroExtentTraces`
off in their class: in the game you walk through them but shots hit them.
`map_collision` keeps such props in `shot_triangles` (or
`shot_part_triangles` when they are parts, such as breakable ones), and
`rc_level_plugin::LevelParts::with_shots_only` puts them in
`LevelCollision` so that only sweeps with no size (shots, looks, blast
lines) stop at them; a moving body (any size) passes. A broken piece stops
nothing. Numbers on 2026-11-10: geo_04c 98 props now collide through their
simple shape and 89 stop shots only; geo_01a 10 and 5; ras_01a 315 and 0.

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

**Changing animation smoothly** (`rc_anim::Crossfade`). When a character
switches animation (standing to running, say) the new animation's first
frame can be far from the pose on screen, so without help the body jumps
in one frame. Each squad mate and enemy keeps a `Crossfade`: when the
animation changes it remembers the pose on screen, and for the next
fraction of a second mixes that pose with the new animation, bone by bone
(`blend_poses`: straight line for positions, slerp for rotations). The mix
goes from 0 to 1 along an S curve ("ease in and out"), so it starts and
ends gently. If the animation changes again halfway, the new blend starts
from whatever mix was showing, so there is never a jump. The original
engine blends inside its compiled code (its `PlayAnim` and `LoopAnim` in
`Engine.u` have no blend-time setting, and no script passes one), so the
times are ours, kept in data files for mods: `[blend]` in
`data/squad/looks.toml` and `blend_time*` in each enemy's `[look]`.
Defaults: 0.2 s, 0.05 s into the very short firing animation, 0.25 s into
a squad mate falling down wounded, 0.1 s into a droid's death. It costs one
extra pose mix per character per frame, nothing on the GPU.

`rc_model --example rc_blend_check` measures it from the numbers: for 18
animation changes of the commando and the battle droid it plays half a
second of each at 30 frames a second and records how far the fastest
moving point travels each frame. Walk to run snapped by 130 uu (about
2.6 m) in one frame before; blended, the biggest step is 25 uu, less than
the run animation's own fastest step (28.5 uu). The surface stretch while
two poses are mixed stays at the animations' own level (commando 0.06,
droid 0.005), so limbs don't fold or tear.

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
Other volumes block too when the map turns their `bBlockPlayers` or
`bBlockActors` on (engine.u's `Actor` leaves both off; `BlockingVolume`
turns them on). geo_01a's `PhysicsVolume1`, tag `PlayerStand`, is the
floor you stand on inside the gunship; the intro script's `SetCollision`
takes it away after the rope order, and you drop about 590 units to the
ground (gravity -1100, landing at about 1,140 units a second, under
`MaxFallSpeed` 1200, so no fall damage). The original's `Briefing` state
only turns the view (`PlayerMove`: `UpdateRotation`), so the body still
falls. These volumes don't stop shots here.

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

### Outdoor ground: terrain (`rc_level::terrain`)

Outdoor maps (geo_03a, geo_04a, yyy_01b, yyy_35a among those checked) put
their ground in `TerrainInfo` actors, not the BSP. Republic Commando saves
the grid's points already placed in the world, right after the actor's
properties: a list of `TerrainSector` objects, then a point count and that
many points (three floats each), row by row with X fastest. Grids are
square (16x16 to 128x128 points, 64 to 768 units apart). Each square is
two triangles; `EdgeTurnBitmap` picks the diagonal and
`QuadVisibilityBitmap` leaves out squares (holes for caves).

Painted layers (`Layers`): layer 0 covers everything; each later layer is
drawn over it, see-through by its `AlphaMap` (often smaller than the grid,
so it is blended between pixels), as a separate batch pulled slightly
towards the camera (`depth_bias`). A texture repeats every `UScale` grid
squares (checked against the layers' stored `TerrainMatrix`). Layers
painted with the editor's checkerboard (`Chex`) mark where decorations
(bushes, palms) go, so they are left out; the decorations themselves aren't
placed yet.

Light: the game's own terrain light wasn't found in the map, so each
corner gets its zone's ambient light (halved, as for props, see "How baked
light is drawn") plus the `Sunlight`'s colour x brightness by how squarely
it faces it, on the same 0-255 scale the props store: on geo_04a the
props' light rises in a straight line with how squarely they face the sun.
That made geo_04a's ground 118, 84, 54 on screen against 110, 82, 56 in the
user's original-game screenshot at the same spot. Maps without a sun
(geo_03a, yyy_01b, yyy_35a) are lit by ordinary lights, not worked out for
terrain yet, so their ground gets the props' average light, flat. The terrain's
triangles also go into collision (`collision::add_terrains`, used by
`map_collision` and so by `rc_game` and `rc_walk`).

Falling rocks (`rc_level::settle`): physics props (`KarmaProp`) whose
`KarmaParams` say `KStartEnabled` (and whose `Physics` the map doesn't
switch off) drop onto the ground as the game's level starts. The port has
no physics for them, so `scene.rs` and `map_collision.rs` place them first
and then move them straight down onto the ground (BSP, other props,
terrain) until their lowest corner touches it, up to 512 units. They don't
turn to lie flat. Props whose physics starts asleep stay where the map puts
them.

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

### In the game (`rc_movers_plugin`)

The level part hands over the map's movers and their triangles
(`LevelMoverData`) while loading. `MoversPlugin` builds the same
`MoverSet` the walker uses, then swaps `LevelCollision` for "the level plus
the movers where they are now", so everything that moves or shoots (the
player, shots, later droids and the squad) runs into doors and stands on
lifts without knowing about movers. The set sits behind a read-write lock
inside the `LevelMovers` resource: sweeps read it, and stepping writes it.

Each frame the player's movement calls `LevelMovers::step` just before its
own step, as in the walker: movers move first (UE2 ticks movers before
pawns), carrying or pushing the player, then the player moves among them.
A `MoverSystems` set (in `GameSet::View`) then logs what happened
(`movers: t=... MoverDB_GEO2 starts opening`), sends each as a
`MoverHappened` message for sounds and level scripts, and moves the drawn
`MoverPart`s.

Other parts open a door the way the original's scripts do: by event name.
`LevelMovers::send("D_Exit")` sets off every mover whose `Tag` is `D_Exit`.

### Squad mates (and other bodies) ride too

In the original every pawn standing on a mover is carried by it, not only
the player, so the squad rides lifts with you (geo_05a, b and c start on
one). Here the player's movement steps the movers, and `MoverSet::step`
remembers which movers moved that step and from where. Any other body is
then carried with `MoverSet::carry` (in the game: `LevelMovers::carry`, or
the `MovingFloors` resource in `rc_game_core`, which hides the movers
behind a small `CarryBodies` trait), called once per body before that
body's own move:

1. What it stands on is judged where the movers were before this step's
   move (where its feet still are), with the same short sweep down as for
   the player.
2. A mover it stands on moves it by the same amount.
3. A mover that now overlaps it pushes it along. If it can't be pushed,
   the mover's "when blocked" rule applies as for the player: it is put
   back where it was and stops or turns round. It is not put back if it
   carried the player this step (the player was already moved with it).

Triggers and bump-opened doors still only answer the player. A frame
where the movers don't step (a cutscene holds the player) carries nobody:
`MoversPlugin` forgets the last step's moves at the start of every frame.

The squad part calls it in `ride_movers`, just before the squad moves, and
logs `squad: t=... Sev stands on Mover37` and `... steps off ...`. Test:
`cargo test -p rc_movers` (a squad mate rides a lift up and down, is left
behind without `carry`, and a closing door goes back for them) and
`cargo test -p rc_squad_plugin --test lift_room` (the real squad and
lifts plugins: all three ride a 400 uu lift with the player).

Where every mover is goes into save files as `[movers]` (see "Saving,
loading and checkpoints").



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

## Level scripts (`rc_scripts`, `rc_scripts_plugin`)

A Republic Commando level is a mission because of its scripting, and the
scripting is data in the map, not code. It works with **events** and
**tags**: every actor has a `Tag` (a name), and many have an `Event`
(another name). "Triggering" an event wakes every actor whose `Tag` is that
name. The map's own scripts are `ScriptedTrigger` and `ScriptedSequence`
actors, each holding an `Actions` list of small objects with values:

```
ScriptedTrigger15:
  0 WaitForEvent   ExternalEvent=BactaUsed
  1 SetCollision   CollideActorTag=BactaBarricadeBlockingVolume bCollideActors=false
  ...
 11 HurtActor      TargetTag=PathToGunshipBarricade amount=50000
```

### Reading (`rc_level::scripts`)

`read` takes every actor in the level (except plain meshes, path nodes and
lights) with all its values by name: numbers, true/false, names, strings,
lists of objects, and structs. Republic Commando writes most structs as
named values too (a dispenser's `BactaRecharging = {Event=BactaUsed, ...}`,
a random wait's `PauseTime = {Min=2, Max=4}`), so those are read the same
way. A script's `Actions` list is resolved to the action objects, in
order; empty slots are skipped, as the original does. New action kinds need
no code in the reader. All 79 maps read with nothing unreadable: 2,842
scripts with 19,041 actions of 87 kinds.

`tagged_actors` lists the props and blocking volumes with a `Tag` of their
own. Those are what scripts switch off or blow up (barricades, bunker
walls), so the collision loader keeps their triangles apart
(`MapCollision::part_triangles`) and the level drawing gives them batches
of their own (`MoverPart`), like movers.

### Running (`rc_scripts::Director`)

The director runs every script one action at a time until it has to wait:

- **Own actions** (run inside the director): `WaitForEvent`,
  `TriggerEvent`, `WaitForTimer`, `WaitForRandomTimer`, `Label`,
  `GotoLabel`, `GotoAction`, `IfCondition`/`IfRandomPct` ... `EndSection`
  (an `If` runs its section only when true, otherwise skips to the matching
  `EndSection`; sections nest).
- **Everything else** comes out as an `Effect::Action` for the game. Some
  also take time: a line of dialogue with `bWaitForFinish`, an animation
  that waits for its end, a character told to walk. A line waits for the
  sound's real length, which the game measures while loading and hands
  over with `Director::set_sound_length`; animations, walks and sounds whose
  length can't be read use our settings.
- `HurtActor` and `DestroyActor` on a **prop** are the director's: it keeps
  each prop's `Health` and reports `PropDestroyed` (and sends the prop's
  `DestroyedState.Event`).

An event, when sent, goes to: scripts waiting for it; `ScriptedSequence`s
handed out by a `PawnFactory` with that tag (`AIScriptFromFactory`; a
sequence also runs from the start if a placed character names it in
`AIScriptTag`); touch triggers it switches on or off (`OtherTriggerTurnsOn`,
`...TurnsOff`, `...Toggles`); `Counter`s (count down `NumToCount`, then
send their own event); `TriggeredCondition`s (a `bToggled` one flips, others
take the opposite of how they started, `bTriggerControlled` ones go back
when untriggered); `Checkpoint`s (`Effect::Checkpoint`). Every event also
comes out as `Effect::Event`, so the game can pass it to doors, factories
and anything else. All of this is our reading of Unreal Engine 2's public
descriptions of these classes and of what the maps store; no game code was
copied.

Scripts run before events are handed out each frame, so a script that has
just reached a `WaitForEvent` hears an event sent in the same frame. A
chain settles within the frame (up to 8 rounds); a script that loops
without waiting stops after 64 actions in a frame and carries on the next
(`max_actions_per_frame`), so no map can freeze the game.

The level starts by sending the first `PlayerStart`'s `Event` (`levelstart`
in most campaign maps; `Startup`, `SetupLevel` and others in some).

### Line-of-sight triggers (`LineOfSightTrigger`)

Some scripted moments wait for the player to *see* something: Grievous on
the tower (yyy_01e `GreivousVerified`), the Trandoshan dragging a clone
away (ras_03c), the Wookiee about to be killed (yyy_01e `Wookie_Die`), a
hack terminal ("we need to slice that"). The map places a
`LineOfSightTrigger` with an `Event`, and usually a `SeenActorTag` naming
what to look at; with none (or no actor with that tag) it watches its own
spot. 45 of them in 25 campaign maps
(`cargo run -p rc_scripts --example sight_triggers` lists them all, what
each watches, what waits for its event, and whether the line to it is clear
through the real walls).

The rule, from Unreal Engine 2's public description of the class and the
values the game's engine.u stores (`rc_probe --class LineOfSightTrigger`:
`MaxViewDist` 3000, `MaxViewAngle` 15, `bEnabled` true): while switched on
and not used yet, if the watched thing is within `MaxViewDist` of the
player's eye, within `MaxViewAngle` degrees of where the player looks, and
no level geometry is in between, the trigger sends its event, once. An
event with the trigger's tag switches it on (`bEnabled=False` ones wait for
that: ras_01a, ras_01b, yyy_05f).

- `rc_scripts::Director::sight_candidates_live` does the distance and
  angle test; `seen` sends the event; `look` does both with a line check
  passed in. Switched on and used are kept in saves (`[scripts] sight`).
- `rc_scripts::sight::clear_line` checks the line through the level's
  collision (walls, solid props, closed doors), stopping 32 uu short of
  the watched thing so its own solid shape doesn't hide it.
- `rc_scripts_plugin::sight::look` runs each frame before the scripts: eye
  and view from the `PlayerCamera` (so a VR camera works the same), the
  line through `LevelCollision`. Many triggers watch a character (a
  Trandoshan, a Wookiee, a Delta mate): where a character with a matching
  `ScriptTags` tag is now is used (its middle: feet plus `Hittable`'s
  `center_up`, or 84 uu), else where the map placed it. A dead player sees
  nothing. Log line: `scripts: ... the player sees what LineOfSightTrigger0
  watches for`.
- Data: `[sight]` in `data/scripts/scripts.toml` holds the class defaults
  for a trigger that gives none.

Not the same as the original: its engine also checks the watched thing was
drawn recently; ours uses the line check instead. A watched thing hidden
inside a big solid prop may be missed (the 32 uu allowance is a guess).

### In the game (`rc_scripts_plugin`)

While loading, the plugin reads the map's scripts and builds the director
once the level part has read the triggers (loading gate part `scripts`).
It inserts `TriggersRunElsewhere`, so the movers part leaves touch triggers
to the director and one place sends every trigger's event. Each frame of
play (after the player moves) it steps the director with the player's body
and carries out the effects:

| Effect | What happens |
|---|---|
| any event | `LevelMovers::send`: doors and lifts with that tag; then a `ScriptEvent` message for other parts |
| a door's own event (`OpenedEvent`, ...) | back into the director, so scripts can wait for doors |
| player leaves a trigger | `LevelMovers::release` (doors that open only while set off close) |
| `SetCollision` | `LevelParts::set_solid(tag, bCollideActors)` |
| prop destroyed, `DestroyActor` | part no longer solid; its drawn `MoverPart`s hidden |
| `SetHidden` | drawn parts hidden |
| `HurtActor` on the player (`PlayerCommando`) | `rc_combat` `Damage` to the player |
| `GotoState` on a bacta dispenser | switched off (`Disabled`) or on |
| `ShowPrompt` | `HudMessage` (`@`, the button, shows as USE for now) |
| `ObjectiveUpdate` | kept in `Objectives` (what the numbers mean isn't worked out) |
| checkpoint | `CheckpointReached { name }`, held while the player is down |
| walking into an open level exit, `ChangeLevel`, a debriefing's `NewLevel` | `ChangeLevel { map, portal }` (see "Going on to the next map") |
| `StopFactorySpawning`, `ResetFactory` | `FactoryOrder::Stop { tag, for_good, remove_offspring }` / `FactoryOrder::Reset { tag }` for the enemy spawners |
| a `RaiseEvent` from another part (a droid destroyed, a spawner done) | into the director, like a door's event, and so out again as a `ScriptEvent` to everyone |
| `AddAmmo` | `rc_game_core::pickups::AddAmmo` to `rc_combat::script_ammo`: the player's grenade of that class (the grenade file's `game_class`) gets the amount added, kept between 0 and the most carried, as the original's `Ammunition.AddAmmo`; nothing if not carried. Only grenades: no map names a gun |
| `BlinkHUDHealth`, `BlinkHUDSquad`, `BlinkHUDGrenade`, `BlinkHUDShields`, `BlinkHUDSquadCommandPrompt` | `LevelScript::hud_blink` (`hud_blink.rs`), read by `rc_game`'s original HUD: elements with that `blink` in `data/ui/hud.toml` are drawn with `rc_menus::hud::blink_alpha` (Engine.dll `UHelmet::DrawHUD`: sin(5 t) * 200 + 55, kept as a byte) until `BlinkTime` has passed. The squad command prompt's blink shows nothing in the original, so nothing here |
| `PlaySound`, `PlayLocalSound` | the game's own sound plays (see "Script sounds" below); `bDisabled` ones are skipped |
| actions on characters (below) | `characters::cast`: a `CharacterOrder`, invulnerability, damage or a squad order |
| anything else | logged with the action's values and the script's own comment, "not done yet" |

**Script sounds** (`rc_scripts_plugin/src/sounds.rs`). A script names a
sound only by its name (`Sound=CA1G1701`). The map's import table also
says which package holds it (`GEO_Voice.CA1G1701`);
`rc_level::scripts::sound_refs` keeps that as `MapScripts::sounds`. While
the level loads, every sound its scripts can play is read from
`GameData/Sounds/<package>.uax` with `rc_combat::read_sounds` (groups too,
also members kept in another package, such as geo_sound's
`arena_cloneDeaths` whose takes are in character_voice), measured with
`rc_audio` and kept in the `ScriptSounds` resource. `SoundStreamed` ones
(a few music stingers) play their `.ogg` from `GameData/Music`. Across the
57 campaign maps the scripts name 1,629 sounds; `rc_script_check` finds
every one in its package. When a script plays one, the sound starts at
`[sound] volume` (`data/scripts/scripts.toml`); the action's own `Volume`
is ignored, as the game's `PlayOwnedSound` ignores it. It is logged as `scripts: t=4.94 ScriptedTrigger52 #13:
ScriptedTrigger52 says CA1G1701 (2.0 s, volume 1.00)` and again when it
reaches the sound card. Which line stops which is the sound engine's
(below): a speaker's lines share a number, and `bDontOverrideOtherSounds`
makes a line one that can't be cut. A group waits for its longest take.
Lines pause with the
game and stop when the level ends. Not done: subtitles (the game's
`System/subtitles_*.int` files hold them, by sound name) and sounds stored
inside a map.

**Where a script sound comes from.** Every `Sound` in the game's packages
names its `Params`: an `AudioParams` object in `Sounds/params_vox.uax`
(speech) or `params_sfx.uax` (effects). Its PC values that matter here are
`NonPositional` (heard the same everywhere: the Clone Advisor, most squad
radio, briefings), `Radius` (Unreal units) and `bDisable3DFalloff` (big
explosions: from their side, never quieter). The class default `Radius` is
2000 (engine.u, `rc_probe --class AudioParams`), which is also
`[sound] default_radius`. `rc_package::sound` reads them,
`rc_combat::read_sounds` now records each sound's `params` (a group's own,
else its first take's) and `rc_combat::read_audio_params` reads the
params packages. At play time `sounds.rs` decides:
- heard everywhere: `ACTION_PlayLocalSound`, `NonPositional` sounds,
  `SoundSourceTag=PlayerCommando` (a tag nothing in the level has plays
  nothing, as `ACTION_PlaySound` logs "non-existant SoundSourceTag");
- else at the character whose `ScriptTags.tag` is the `SoundSourceTag`
  (followed as it moves), or the map actor with that tag or name (fixed);
- with no tag: the character running the `ScriptedSequence`, else the
  script actor's own spot in the map (UE2 plays a script's sound from the
  script when it has no pawn).

A placed sound gets `rc_game_core::SoundAt` (3D SOUND makes it heard from
its side) and a `Placed` component; `sounds::follow` moves it with its
character and sets its loudness every frame: full out to `Radius`, then
`Radius / distance` (our reading: OpenAL's reference distance; some radii
are tiny, enemy footsteps 20 uu, so `Radius` can't be where a sound
stops). Over the 56 campaign maps
(`rc_script_check`) the scripts' 1,946 enabled sound actions split as:
1,404 radio, 58 aimed at the player, 23 local, 391 at the script's spot
(mostly explosions), 48 at a sequence's character, 9 at a tagged map
actor, 13 at a tagged character if one is there.

Bevy's own 3D sound (rodio) also divides a placed sound's loudness by the
distance squared in Bevy units (metres): a droid 10 m away played at 1/100.
`rc_game/src/sound_options.rs` now shrinks placed sounds' positions by
1000 (`SpatialScale`) before Bevy sees them, which leaves left and right
alone and pushes that fading out past a kilometre, so only each part's
own distance rule applies.

**The sound engine** (`rc_scripts_plugin/src/engine.rs`, the game's
`UAudioSubsystem`, read from Engine.dll and ALAudio.dll by disassembly).
Every part still plays a sound by spawning a Bevy `AudioPlayer`, adding
`SoundAt` for a place and `rc_game_core::SoundBy` for who plays it (an
entity or a number, whether it is a character's own cue that may be cut,
a map loop, heard everywhere). In `PostUpdate` the engine:
- finds the sound's facts (`rc_combat::sound_meta`, registered when the
  game's packages are read: its `AudioParams`, category, flags, length);
- rolls its volume and pitch between `MinVolume`/`MaxVolume` and
  `MinPitch`/`MaxPitch` (`PlayUSound`; under 0.0001 it isn't played),
  checks it can be heard (`CheckHearSound`), gives it the game's id and
  priority (`UpdatePriority`);
- sorts queued and playing sounds with the C library's `qsort` (equal
  priorities land in the game's order), stops playing ones under the cut,
  drops impacts when no channel is free and hands out the AUDIO CHANNELS
  in channel order (`FindSource`);
- ducks music to 0.5 and effects and other voices to 0.25 over 0.5 s
  while a `bDisableVoiceCues` line plays (`UpdateDuckVolume`), published
  as `rc_game_core::Ducking` for the music part and squad chatter;
- sets each sink's volume: rolled volume × duck × `SoundFade` × the
  category slider (`SoundMix`, from the SOUND options), kept within
  0.0001 to 0.9999, times the distance share of `Placed`;
- works out the EAX occlusion (`rc_game_core::EaxOcclusion`) when EAX is on.
Sound groups (`USoundMultiple`) pick in order, `FirstSound` once, or by
`Likelihood` skipping members for the wrong battle state or campaign
(`LevelInfo.Campaign`, read by `rc_game/src/music.rs`).

**Animation sounds** (`rc_scripts_plugin/src/anim_sounds.rs`). Animation
sequences carry `AnimNotify_Sound` notifies: a moment (0 to 1 through the
sequence) and a `Sound` (`rc_package::animation::notify_sound`). The cast
(`rc_cast`, from `rc_model::Model::sounds`, every kept set's sequences by
their own name) and the enemies' level-script animations (`rc_enemies`,
`DroidLook::sounds`) send `rc_game_core::AnimSound` for each notify the
frame passes, with the character's entity. While a level loads both add
every sound their models can play to `rc_game_core::AnimSoundsWanted`;
`anim_sounds::load` reads the new ones, one read per package, and
`anim_sounds::play` plays each from the character with the placed-sound
rule (`Placed`, following the entity). The game does the same: its
`Engine.dll` `UAnimNotify_Sound::Notify` asks the audio code to play the
sound from the animating actor (read by disassembly, nothing copied).
Not yet: the enemies' own (not scripted) animations, `AnimNotify_Footstep`
and `AnimNotify_Effect`.

**The map's looping sounds** (`rc_level::ambient`,
`rc_scripts_plugin/src/ambient.rs`). Any Unreal actor can carry an
`AmbientSound` (engine.u: `var(Sound) sound AmbientSound;`) that loops
while the actor is there. Maps use it on `AmbientSound` actors (geo_01a's
battle in the distance, `GEO_Sound.ambPS_GEO_battle_lp_01`) and through
class defaults (`Fire_Medium` crackles `ambPS_GEN_fireMd_lp_01`,
`WhiteSmoke` hisses `ambPS_GEN_steam_lp_05`, both in CTEffects.u). While
loading, `rc_level::ambient::read` lists every actor with one (the map's
value first, then its class chain via `ClassLooks`), with its
`SoundOcclusion`, leaving out movers (theirs is `MoveAmbientSound`, while
moving) and characters. They reach the game as
`rc_level_plugin::LevelAmbientSounds`. On the first frame of play
`ambient::start` reads the sounds and their `AudioParams`
(`AmbientSources`); after that `ambient::update` follows the game's
`UAudioSubsystem::UpdateAmbientList`/`UpdateAmbients` (Engine.dll, read by
disassembly, never committed): a loop starts when the listener is within
`Radius × 100` (`GAudioMaxRadiusMultiplier`), times `0.85 ^ (rooms
apart²)` when a level wall is between, fading in over 0.4 s; it stops past
that distance plus 10, fading out over 0.3 s, and starts from its
beginning next time. While playing it is a `Placed` sound like the script
sounds. geo_01a's six loops all use `Ambience_PS_01`, `Radius` 10 on PC
(1000 on Xbox), so they can be heard only within 1000 units. geo_01a has 6
(`cargo run -p rc_level --example ambient_sounds -- <game folder>
geo_01a`). The placed emitters' particles are drawn by `placed_emitters`
through the effects part, which leaves their `AmbientSound` to this
(`SpawnEffect::skip_ambient_sound`), so it plays once, from its place.
Switching an emitter off with a level event stops its particles only: the
sound belongs to the actor, not its particle emitters.

**Walls, rooms and placed sounds** (`rc_level::hearing`,
`rc_scripts_plugin::sounds::{Placed, Reach}`). Every placed sound (script
sounds, animation sounds, map loops) gets louder and quieter by the game's
own rule. OpenAL's inverse-distance-clamped model, roll-off 1, with the
reference distance ("reach") set each frame by
`FAbstractSource::UpdateRadii`: the sound's `Radius` when the straight
line from sound to listener crosses no solid part of the level's BSP
(`ULevel::IsAudibleAt` → `UModel::FastLineCheck`; placed meshes don't
count, a mover's sound and `OCCLUSION_None` actors are never blocked),
otherwise `Radius × 0.85^(steps²) × 0.35`, where `steps` comes from the
64 × 64 zone table `UGameEngine::LoadMap` builds from each zone's
`Connectivity`. The zones are the sound's spot and the listener's spot
(`UModel::PointRegion`, the same walk as `Bsp::point_zone`). The reach
eases toward its target as the game does: by `(now − when the target last
changed)` of the way each frame, jumping straight there before the first
change. OpenAL refuses a negative distance, so a negative overshoot keeps
the last good one. `now` is the level scripts' clock (the game uses its
level time, `GEngineTime`). Loudness is `reach / distance` beyond the
reach, times the loop's fade.

**Bacta dispensers.** Hold Use within 256 units of one that's switched on
(the game's `PlayerRange`) to heal at 50 health per second (our choice).
Starting sends the dispenser's `BactaActive.Event`; full health sends its
`BactaRecharging.Event` and it rests 10 s (our choice). In geo_01a those
are `BactaDispenserActive` (a condition the reminder lines check) and
`BactaUsed`, which is what opens the landing zone.

The rest of the dispenser (`rc_scripts_plugin/src/bacta.rs`, numbers and
names in `[bacta]` of `data/scripts/scripts.toml`), read from the game's
class defaults with `rc_probe --classes`:
- **Pull and hold.** `BactaDispenser` has `bLockPlayerLocation` and
  `BactaDispenserDefaults` `PlayerOffset` 90. While healing, the bacta
  system fills in `rc_game_core::UseItem::lock` (a spot 90 units in front
  of the dispenser, along its own facing, and the way back to it); the
  player part (`rc_game/src/player.rs`) pulls the body there and turns it
  to face the dispenser, then holds it (`ActionState::hold_for_use`: like
  a cutscene hold, but Use still counts, so letting go stops). After the
  turn, looking around is free. The front was checked on geo_01a: each
  dispenser's squad spot (`NavPts`) lies along its facing, 135 units out.
- **Hologram.** `MarkerGetBacta` (CTGame.u) names its participant's
  `IconMesh` `MarkerIcons.Revive.reviveSM1BactaDispenser`: a see-through
  commando, 183 units tall, origin at the feet, drawn with a hologram
  shader (`HsHologram` HoloMarkerClone). One is placed at each dispenser's
  squad spot, on the dispenser's floor, facing it. When it shows comes
  from the game's own scripts (engine.u `SquadMarker`, ctgame.u
  `BactaDispenser`, `CTPlayerController`): the player's controller tells
  the marker it is targeted, and `UpdateIcons` shows the icons unless the
  marker is irrelevant (`MarkerGetBacta::IsRelevantFor`: a squad mate
  below `HL_Green`, read here as below full health). The dispenser makes
  its marker obsolete while in use and while recharging. So the hologram
  fades in only while the crosshair is on the squad spot within
  `TargetingDistance` (3000), a squad mate is hurt and the dispenser is
  free, and fades out otherwise (fade time our guess). Tinted
  `EnabledColor`. Drawn as a flat additive tint; the flickering hologram
  shader is `rc_hologram`'s job.
- **Prompt.** BactaDispenser's `ActivatePromptText` ("PRESS AND HOLD @
  TO HEAL"), read from ctgame.u at load (`[bacta] prompt` overrides it),
  with `@` as the Use button, drawn where the HUD's pickup message goes
  while the player stands at a usable dispenser. As in
  `CTPlayerController::Use`, Use does nothing at full health.
- **Sounds.** The HUD's own (`CTHUDDefaults`): `TargetMarker`
  (int_GEN_retCrossZoom_01) when the crosshair moves onto a usable
  dispenser's collision cylinder, `HealthRecharging`
  (int_GEN_hudHlthChrgng_02) looping while healing, `HealthCharged`
  (int_GEN_hudHlthChrgd_01) at full. Heard from the helmet (no `SoundAt`).
- **Use panel** (how it is drawn was read from the original's engine
  code, `Engine.dll`, 2026-11-11: the helmet HUD's `DrawTimerRing` and the
  part of `DrawHelmet` that calls it when the player's `TargetType` is
  `TGT_Panel`; only behaviour and numbers were taken, no code). In place
  of the crosshair while aiming at a dispenser, centred on the screen on
  the original's 600-line HUD:
  - The ring `HUDTextures.Reticles.HUD_EventTimer` (the helmet's
    `EventTimer`) at its own size. It is cut into a 4 by 4 grid of
    pieces. The whole ring is drawn first in (200,240,255) at opacity
    92/255, then the lit pieces on top in the same colour, fully opaque:
    one more step for each full twelfth of the value, from just left of
    the top, down the left, along the bottom, up the right; the piece
    just right of the top only lights at full, when the whole picture is
    drawn (`original_hud::ring_pixels`, `RING_STEPS`).
  - For a bacta dispenser (the panel kind the engine checks for) the
    value is the player's `Health / HealthMax`. At full health the ring
    is left out and only the icon shows (`UsePanel::Icon`).
  - The icon: square `HUDIconX` 1, `HUDIconY` 7 of the helmet's
    `HUDTextures.Icons.LargeVisorIcons`, 40-pixel squares counted from
    the bottom right of a 3 by 10 grid, drawn 40x40. Its colour follows
    the same value: red (255 - 115v), green (140 + 115v), blue 180,
    opacity 240.
  - The prompt (`DrawActivateText`) is (164,200,208), fading in to
    opacity 220. The fade-in is copied from `DrawActivateText`: opacity
    = whole part of (seconds up / `PromptFadeInTime` x 220), so 0 on the
    first frame and 220 from 0.5 s on (`PromptFadeInTime` is 0.5 in
    engine.u's Helmet defaults, read with `rc_probe --classes`). The clock
    starts again when the prompt was not drawn the frame before
    (`bActivateTextLastFrame`, also cleared when the advisor's words take
    the box) or its words differ from `LastActivateText`. In the port:
    `LayoutSettings::prompt_faded_in`, `fade = "prompt"` on the prompt
    elements, see-through (`blend = "alpha"`) like the advisor's words,
    which go through the same text call. Its place, near the top in two lines ("PRESS
    AND HOLD 'F' TO" / "HEAL"), is still matched to the user's
    screenshots: the `{prompt1}`/`{prompt2}` elements of
    `data/ui/hud.toml`.
  - Not read yet: other panels show a "%02i:%02i" countdown under the
    ring when the panel has time left; the healing sound is the
    `HealingTarget` HUD sound (int_GEN_hudTimer_01), but when the engine
    plays it was not found in this part of the code.
- **Our readings, not checked against the original** (it doesn't run on
  the user's PC right now): when each sound plays, where the prompt sits
  (matched by eye to screenshots), the
  pull's speed, the hologram's brightness and fade. Not done: the
  body effect and `HUDIconX/Y` (an icon over the dispenser).
- **The zap and hum** (`bacta::zap`), read from the game's
  `BactaDispenser` script (ctgame.u, `state Active`): after `Load` has
  finished (`FinishAnim`) the script sets `bActivated` (healing starts only
  then; `heal` waits for it too) and does `SpawnAttachment('Eye',
  DispenserEffectClass)`; every dispenser class in
  `Properties/MarkerProperties.u` sets `BactaZap_Dispenser`. A small
  carrier entity (`BactaZap`) stands on the model's `Eye` bone
  (`AnimProp::bone`, from `rc_model::Model::bone_from_origin`) and the
  effect follows it, turned by the bone's axes (`SpawnEffect::axes`). The
  hum is that effect's own `AmbientSound`
  (`GEO_Sound.GEO_03.ele_GEO_genEnrgy_lp_01`), which the effects plugin
  loops while the effect is there. `EndState` does
  `DestroyAttachment('Eye')`, so when healing stops the carrier goes and
  the effect with its hum vanishes at once (`vanish_with_carrier`), not a
  fade. Our choices: the hum's loudness follows the effects plugin's
  distance rule (worked out when the zap starts), and it isn't placed in
  3D. Not done: `PawnEffectClass` (`BactaZap_Dispenser_Body`), whose
  particles start at the bones of the player's own body model
  (`UseSkeletalLocationAs`), which first person here doesn't have.

### Breakable props (`rc_level::breakables`, `rc_scripts_plugin/src/breakables.rs`)

Explosive barrels, geo_05b's fuel dispensers and their clamps, holo
machines, consoles, rock cover, drone eggs and wall panels are all `Prop`s
(or `KarmaProp`, `StaticProp`, `ExplosiveBarrel` ...) with health. The
values live in the map, or as the class's defaults in the script packages
(`Properties/StaticPropProperties.u`, `Properties/MarkerProperties.u`,
`CTMarkers.u`); `rc_level::breakables::read` takes the map's value first
and then walks the class and its parents (`class_look`). The Geonosis maps
have 838 of them; 53 send an event the level scripts wait for.

How a prop breaks (our reading of the game, in our words):

- It starts with `Health`. With none (0 or less) nothing can hurt it; only
  a script can remove it.
- Damage counts after the prop's damage table, as for characters.
- `DamagedRatio` above 0: when health falls to that share of the start,
  it goes to its damaged step (`DamagedState`): sends that step's event,
  and if the step has no model of its own it simply disappears, *without*
  its broken event (geo_04d's rock cover works this way). A damaged step
  with a time limit (`StateLifeSpan`) breaks by itself when it runs out.
- At no health it breaks (`DestroyedState`): it sends its own `Event`, or
  if it has none `DestroyedState.Event`. Broken props take no more damage.
- If it has an explosion (`ExplosionDamage` and `ExplosionRadius`) it goes
  off 0.1 to 0.2 s later, hurting everything it can see within the radius,
  falling off with distance (the same rule as grenades).

Who does what: `rc_level::map_collision` keeps every breakable prop's
triangles apart (as a `LevelParts` part), and the level viewer draws each
as its own `MoverPart`, so it can go when it breaks. Some models have
collision switched off on every part (the Geonosian explosive barrels):
those collide through the model's simple collision shape (see "Simple
collision shapes"). A breakable prop with neither gets its drawn
triangles, so it can still be shot. `rc_scripts::Director`
keeps health and steps (`add_breakables`, `hurt_prop`) and sends the
events, so a script waiting for `Clamp00` carries on. `rc_scripts_plugin`
turns weapons into damage: a `ShotLanded` on the level hurts the prop whose
box it landed on (`ShotLanded` now carries the shot's damage and type), a
grenade `Blast` hurts the props around it, and a prop's own explosion sends
`rc_combat::Explode` (characters) and hurts the props around it (so barrels
set each other off). Broken and damaged state are saved with the scripts.

When a step starts, `rc_scripts_plugin/src/breakables.rs` (`show`) sends
its effect (`Effect`, at `EffectOffset`) and sound (`TransitionSound`) to
the effects plugin, and for a broken step with a model (`Mesh`) puts that
model where the prop stood, for the step's `StateLifeSpan`. A map that
writes its own `DestroyedState` replaces the class's one whole: geo_01b's
barrels say `Mesh=None`, so they leave no broken model there (geo_03a's
and others' do: `MarkerIcons.SetTrap.GEO_EplBarrel_Dest`). The broken
model is unlit and tinted by the average vertex colour of the prop it
replaces (our choice).

Not done yet: the damaged step's model, the broken look in save files,
the burning or electric after-effect some barrels leave,
the push on loose (Karma) props, and a prop's `DeathSpawnActorClass`. Breakable props players can walk
through (geo_04c's wall components) aren't in the collision, so shots pass
through them.

### Characters in scripts (`rc_scripts_plugin/src/characters.rs`)

A `ScriptedSequence` belongs to the characters that run it: a placed pawn
names it in `AIScriptTag`, and a `PawnFactory` gives it to everyone it makes
through `AIScriptFromFactory`. An action with no target of its own acts on
those characters. Each spawned character carries `ScriptTags { tag, script,
follows_orders }` (in `rc_game_core`): its tag (the factory's
`PawnTagFromFactory`, or a squad mate's class name) and the sequence it runs.

The director marks these actions "the game runs them"
(`Director::game_runs_characters`). It hands them out as `Effect::Action`
with the sequence's tag as `owner`; the plugin turns each into a `Call` and
`cast` finds its characters, by tag (the action's own `Tag`, `PawnTag`,
`DestroyTag`, ...; a player tag means the player) or by owner. A character
made by a factory appears a frame after the event that starts its script,
so a call with nobody to act on waits `character_grace_seconds` (1 s) before
it is logged "no character" and skipped; later calls of the same script
wait behind it.

| Action | What happens |
|---|---|
| `MoveToPoint`, `MoveToPlayer` | `CharacterAct::MoveTo` / `MoveToPlayer`: the enemy walks (or runs) there along the level's paths |
| `TurnTowardPlayer` | turns to face the player |
| `PlayAnim`, `WaitForAnimEnd` | stands still and plays the named animation from its own animation set |
| `WaitForPlayer` | holds until the player is within the distance |
| `TeleportToPoint` | put there at once, facing the point's way |
| `SetHidden`, `DestroyActor`, `DestroyPawn` | hidden / removed (removed without a death event) |
| `MakeInvulnerable`, `SetCannotDie` | `rc_combat::Invulnerable` (no damage) / `CannotDie` (health stops at 1) |
| `HurtActor` on a character | `rc_combat` `Damage` |
| `ChangeSquadStance` | `SquadStanceOrder` to the squad (form up, search and destroy, secure a point) |

Places are every actor in the map but static meshes and lights
(`rc_level::scripts::places`), found by tag and then by name; a missing one
is logged. Enemies (`rc_enemies/src/scripted.rs`) carry the orders out
through `rc_ai::Fighter::follow_script` (`ScriptGoal`): while it has one it
doesn't fight, being hurt ends a hold, and it answers `AiEvent::ScriptDone`,
which comes back as `CharacterOrderDone`. A walk that gets stuck for 2 s
ends there. An enemy with `leap_speed` above 0 in its `[combat]` section
(the Geonosian warrior, 900) leaps straight to a point it can't walk to or
that's more than 200 uu above it, a stand-in for the original's flying.

Walks, turns and animations hold the script until the game says they're
done, with a timeout each (`move_timeout_seconds` 30, `turn_timeout_seconds`
2, `anim_timeout_seconds` 10, in `data/scripts/scripts.toml`), so a stuck
character can't stop a level. A save made while a script waits on one keeps
which action it was, and after a load the action is given again.

Not done yet: squad mates' own scripted moves (`ChangeSquadMember`), moving
the player, `SetTeamIndex`, `SetThreat`, `SetSightRadius`, `ShootTarget` and
`GotoState` on characters, characters the game doesn't spawn yet (placed
clone troopers and the like are logged and skipped), one sequence run
separately per character, and `ScriptPriority`.

### The objective tracker (`rc_scripts_plugin/src/tracker.rs`, `rc_game/src/objective_tracker.rs`)

Level scripts point the player at the next goal with
`ACTION_ShowActorObjectiveTracker` (178 steps in 36 maps). Everything below
was read from the original's files on 2026-11-12: the script step's
UnrealScript text in ctgame.u, the `Helmet` defaults in engine.u, and the
native helmet drawing (`UHelmet::DrawHelmet`, `FCanvasUtil::DrawRotatedTile`
and `DrawRotatedIcon`, `UHelmet::ActorDestroyed`) in Engine.dll,
disassembled and partly run in an emulator locally for reference. Only
numbers and rules were taken; no code from the game is in the repository.
All numbers are in `data/ui/objective_tracker.toml`.

- **The step.** Finds the first actor whose `Tag` is `ActorTag`, in the
  map's actor order (`AllActors`), and hands it to the helmet; when none
  has it, or the tag is None, it hides the tracker. When the map is read,
  `tracker::read_targets` finds the first placed actor for every tag the
  map's tracker steps name (an actor with no `Tag` is tagged with its class
  name, as in UE2) and writes each to the log. A placed character (its
  class comes from `Pawn`) is followed while the game's character with that
  tag lives; a tag no placed actor has is matched against characters the
  game spawned; with neither, the tracker hides, as the original's does.
- **The goal.** Its actor's `Location`, followed each frame. Its size is
  the bounding sphere radius of the actor's static mesh (the map's own
  setting, else its class default, through `ClassLibrary::object_path`), 0
  without one.
- **Ring or arrows.** If the view is within 45 degrees of the line from the
  eye to the goal, the line is checked through the level and its movers to
  250 units short of the goal: at once when the goal comes ahead, then
  every 0.25 s (`VisCheckFrequency`) while it stays clear. Clear: the four
  corner arrows. Otherwise the ring.
- **The ring.** `HUDTextures.Icons.ObjectiveRing` (256x256), centred at
  (400, 463) on the 800x600 screen, turned clockwise by 65535 minus (the
  view's yaw minus the goal's yaw seen from the pawn), then flattened by
  cos(12288) (tipped back 67.5 degrees). Colour (200, 240, 255): a shadow 5
  units lower at opacity 63, then the ring. The engine's sine table (16384
  steps) is used for every turn.
- **The distance.** `"%.1f"` of the distance from the pawn's `Location`
  (the centre of its collision box, `rc_game_core::PlayerPawn`) to the
  goal, in hundreds of units, in `OrbitFonts.OrbitBold8`, (200, 240, 255),
  centred on the ring's centre, each letter its size times the screen
  scale in whole pixels.
- **The arrows.** Four corners half the goal's radius out to the sides and
  up and down, square to the line from the eye, put on the screen with the
  camera; the box is pushed out to at least 78 pixels each way. At each
  corner the triangle of `HUDTextures.Icons.LargeVisorIcons` (texels 0,360
  to 39,399) is drawn 39 units across, turned to point inward (24575, 8191,
  -8193, -24577), in (200, 240, 255) with opacity = the low byte of
  int(sin(time x 2.5) x 200 + 55), then again at 1.25 and 0.75 times the
  size at a quarter of that opacity.
- **Wide screens.** The original scales its 800x600 units to the screen
  both ways; the port uses the screen height over 600 both ways and keeps
  the middle in the middle (the same as the original on a 4:3 screen).
- **Blocking volumes.** The original's sight line (`SingleLineCheck`
  flags 0x286: movers, level, level geometry) doesn't stop at blocking
  volumes. The port's `LevelCollision` holds their faces, so the map's
  reader keeps the faces of volumes that lines pass
  (`bBlockZeroExtentTraces` off) apart as well
  (`MapCollision::line_through_triangles`, resource
  `rc_level_plugin::LineThroughVolumes`), and the tracker's line goes on
  past a hit on one of them.
- **Moving goals.** A door or lift as the goal is followed where it has
  moved to (the mover's pose), as the original's `Location` is.
- **While the player is down.** The original keeps drawing it: the helmet
  is drawn (`FPlayerSceneNode` calls `UHelmet::DrawHelmet`) unless the view
  isn't the normal render mode, there is no pawn or one player-controller
  flag is set, and `DrawHelmet` only checks `bDraw` (changed only by the
  `DrawHUD` console command) and `bShowObjectiveTracker`. The down state
  (`CTPlayer` state `CtrlIncapacitated`) changes none of these, so the port
  keeps it too.
- **Saves.** The original doesn't keep the tracker in save files. The
  player's helmet is held in `var transient Helmet Helmet` (engine.u
  `PlayerController`), and a transient value isn't written to a save.
  After a load `PlayerController.PostLoadBeginPlay` calls
  `SpawnDefaultHUD`, and ctgame.u's `CTPlayer.SpawnDefaultHUD` makes a
  `new class'Helmet'` from the class defaults: nothing followed,
  `bShowObjectiveTracker` off (it has no default, so false), and the sight
  check's time and result at zero. So the tracker stays off until a
  script next runs a tracker step. The port does the same: it saves
  nothing for the tracker, and leaving the level for a load
  (`tracker::forget`, `ObjectiveTracker::reset`) puts it back to those
  defaults.
- **Ours, not the original's:** the port's view yaw is in radians and is
  rounded to the engine's units; and the sight check runs in the game's
  update rather than while drawing.

### Squad marker jobs reach the level (`markers.rs`, `rc_scripts::markers`)

Every squad marker in a map (`MarkerDemolition`, `MarkerHackTerminal`,
`MarkerHackDoor`, `MarkerDoorBreach`, `MarkerSnipe`, ...) names an
**anchor** (`Anchor`): the map actor the job is for. The anchor holds the
level's side of the job as map settings, and the level's scripts wait for
the events it sends:

| Anchor class | Started | Armed | Done | Also |
|---|---|---|---|---|
| `DemolitionCharge` | `EventUnderway` | `ChargeArmed.Event` | `ChargeExplode.Event` | blows away `PropToDestroy` |
| `HackTerminal*` (console) | `EventUnderway`, `ActiveState.Event` | | `HackedState.Event` | |
| `HackDoor*` (lock, sliced) | `EventUnderway` | | `DoorUnlockedState.Event` | opens its door |
| `BreachDoor*` (lock, blown) | `EventUnderway` | | `DoorDamagedState.Event`, `DoorUnlockedState.Event` | opens `DoorLeft`, `DoorRight` |
| anything else (a sniper post's anchor) | `EventUnderway` | | | |

The order marker can have its own `EventUnderway` too (geo_01a's
`MarkerDemolition2` sends `SpawnDemoDroids`), sent with the anchor's.
A door can have both a slice lock and a breach lock; they name each other
(`MatchingComponent`) and either one opens the door. The doors are the
`MoverDB_*` double doors, whose class default state is `Locked`: they
never open on their own, so the movers part treats them as still until
`LevelMovers::open_named` opens them. They then stay open (the original
lets a sliced door work as an automatic door again; not done).

How it flows: the squad part (`rc_squad_plugin`) sends
`rc_game_core::MarkerJob { anchor, step }` when a squad mate starts a job
or takes up a post (`Started`), lights a fuse (`Armed`) and finishes
(`Done`, with the job's result). `markers::run` in the scripts part sends
the anchor's events into the director, once per step per anchor (a
`Done` with no `Started` before it sends both), blows away the prop
(`Director::destroy_prop_named`, which also sends that prop's own
destroyed event) and opens the doors. Another part can send `MarkerJob`
too (the player doing a job himself, later).

`rc_script_check` lists every map's anchors and checks that each door they
open is a door the movers part moves. On the 13 Geonosis maps: 57 anchors,
61 of their events are ones scripts wait for, and 38 of 38 doors found.

Not done: the anchor's own look changing (a lit charge, a console's
screen, the blown lock's mesh), its sounds, the player doing a job with
Use, and saving which steps were sent (after a load a step can be sent
again if the squad repeats it).

### geo_01a, worked through

From the map: `levelstart` starts the gunship intro (`ScriptedTrigger52`),
which sends `Rappel`; `ScriptedTrigger7` plays the advisor's lines, sends
`CheckpointLevelStart` (checkpoint0) and `StartBactaTraining`;
`ScriptedTrigger3` hurts the player 175 (an "anti-infantry round"), plays
the "heal yourself" lines and sends `StartBactaReminderTimer`;
`ScriptedTrigger86` switches the dispenser `BactaDispenserLZ` on. Using it
sends `BactaUsed`, and `ScriptedTrigger15` switches off
`BactaBarricadeBlockingVolume`, sends `RetreatToGammaSector`, and blows up
the prop `PathToGunshipBarricade` (`Blockade01`, health 49,000, hurt
50,000). The barricade's other piece (`Blockade02`, a plain mesh) stays: the
way through is the gap where `Blockade01` stood.

### How we check it without looking

- **Unit tests** (`cargo test -p rc_scripts`, 7): an event and timer chain
  in order and on time; `IfCondition` skipping a section only when its
  condition is on; a counter sending on its last count and a checkpoint
  asking for a save; touch triggers firing once, and one switched on by
  another; a prop destroyed when its health runs out; a loop with no wait
  bounded, and a sequence starting with its factory; the settings file.
- **Every map** (`rc_script_check`): reads all scripts, lists every action
  with its values, counts action kinds, sends the level start and runs 30 s
  (or `--secs`) with no player, logging every event and effect. `--fire
  BactaUsed@60` stands in for the game.
- **Saving** (`cargo test -p rc_scripts -p rc_scripts_plugin`): a save
  made while a script waits on a timer with a counter half counted loads
  into a fresh director that carries on the same way, without the level
  start; a small geo_01a (barricade blown, volume off, checkpoint held)
  goes through the file and back, and after the load the parts stay off,
  the level counts as started and the script finishes on `BactaUsed`.
  After the game's quickload, `rc_game --script` checks the level start
  wasn't sent again and the barricade is still gone.
- **In the game** (`rc_game --script`): on geo_01a it checks the scripts
  run and `levelstart` was sent, that a box swept through the barricade at
  `Blockade01` is stopped, waits for the scripts to switch the dispenser
  on, stands the player at the dispenser's marker, holds Use, waits for the
  barricade to go, then checks the same sweep now passes, the volume isn't
  solid, both drawn parts of the prop are hidden, and that the chain of
  events happened in order with the player hurt, healed and checkpoint0
  saved. Then it sends `SpawnTrench02` and `LoneRangerJump` and checks that
  the lone Geonosian from `PawnFactory50` carries his tag and sequence,
  reaches `JumpDestForLoneRanger` and `KillPointForLoneRanger`, turns,
  plays `EmoteSeeEnemy` and is removed, each answered back to the script.
- **Characters** (`cargo test -p rc_scripts -p rc_ai`): a walk holds its
  script until the game answers, a timeout moves it on, and a save made
  mid-walk gives the walk again after loading; an enemy follows walk, turn
  and hold orders without shooting, then fights; one that can leap leaps
  to a high point and a battle droid doesn't. `rc_script_check` also counts
  the places scripts send characters to that it can find.

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
names it. Stuck for 10 s on the way, a member gives the job up. geo_01a
has 6: five bacta dispensers and a demolition charge.

### Context orders (`rc_squad/src/jobs.rs`, `[job.*]` in `squad.toml`)

What a marked job *does* depends on its kind. Each `[job.<name>]` section
of `squad.toml` describes one; a marker gets the first job whose `match`
words appear in its order marker's class (`MarkerDoorBreach` matches
"breach"), or, with none, its anchor's class (`BactaDispenserGEO` matches
"bacta"). How a map's anchors, order markers and spots fit together is
in docs/RESEARCH.md, "Squad markers"; `rc_squad::map::read` turns them
into one `MarkerSpot` per place a squad mate stands (a firing position
with no spot puts them at the anchor). A marker no job matches is a plain
"walk there, work `marker_work_time` seconds, done". Blast sizes, fuses,
grenades and who prefers which job are the game's numbers; how long the
work takes and the posts' firing rhythm are compiled code in the
original, so those are our own, marked in the file. A job's `preferred`
squad mate takes it when free (Scorch demolition and anti-armour, Fixer
slicing, Sev sniping, as in the original). Mods can change a job or add a new one.
`[job.turret]` ("Man turret") is a post with `mount = "yes"`: see
"Squad mates on turrets" under "Mounted turrets".

- **Once** jobs (bacta, demolition, breach, slice): walk to the spot,
  work `work_time` seconds, done. The member is free again at once. Then,
  after `fuse` seconds (the charge burning), there is a blast if
  `blast_damage` is set, and the `result`:
  - `heal` (bacta): the squad mate's health and shield are filled.
  - `trigger` (breach, slice, demolition): the plugin sends a
    `SquadJobDone` message naming the anchor. Whoever owns that actor (the
    level's scripts and doors, still being built in their own part)
    opens the door or destroys the target. Until then it is logged:
    `Demolition done: sets off DemolitionCharge DemolitionCharge2`.
- **Group** jobs (`group = "yes"`, the door breach): ordering one spot
  sends a free member to every spot sharing the same anchor (the ordered
  spot first, nearest member each). Members wait at their spots (`Ready`)
  until all are in place; then the one at the ordered spot does the work
  and the others stand guard and shoot. When it's done, every spot is
  done. If someone gives up (stuck or down), the rest go ahead without.
- **Posts** (sniper, grenade, anti-armour): walk to the spot and stay
  (`Posted`) until F4 recalls the squad, even if the player walks away.
  While posted, the job's own shooting numbers replace `[fight]`'s:
  the sniper sees 8000 uu and fires single aimed shots with
  `dc17m_sniper`. A grenade post throws a grenade every `throw_interval`
  seconds; it goes off `throw_fuse` seconds later where the target stood.
  A grenade marker names a target area (`WeaponTarget`): the grenadier
  throws at the enemy nearest it within `target_radius`, seen or not.
  Without one, at its own target within `throw_range`.
- **Auto pull maneuvers** (the original's GAME option, on by default;
  `auto_pull_maneuvers` in `settings.toml`, passed to the squad as
  `rc_squad_plugin::SquadOptions`): when an order needs more squad mates
  than are free (one per open spot of its group), the missing ones are
  taken off `pullable` posts (sniper, grenade, anti-armour in the shipped
  file), the one who has held a post longest first. The log says
  `Sev left S0 (pulled for S3)`. Off, the order is refused while
  everyone is busy, and the log says to recall the squad. Once jobs are
  never pulled. How the original does it is in docs/RESEARCH.md, "Auto
  pull maneuvers".
- **Blasts** (charges and grenades) hurt everyone near: `rc_combat::Damage`
  of kind `Explosion`, `blast_damage` at the middle falling to nothing at
  the radius, measured to the edge of each body. Friend or foe, as in the
  original, so stand clear of a demolition charge.
- **Guns from other parts.** `dc17m_sniper` and `dc17m_antiarmor` come with
  the weapons part's attachments. If a post's weapon file isn't there, the
  squad mate uses the usual DC-17m and the log says so once.

### Where the squad comes from, leaving and rejoining (`rc_squad/src/presence.rs`, `rc_squad_plugin/src/membership.rs`)

As in the original, each map has its own squad (`spawn = "from_map"`, the
default). Most mission maps place `Commando07Delta`, `Commando40Delta` and
`Commando62Delta` actors; those squad mates start where they are placed,
in the squad. A few come later from a `PawnFactory` whose `Prototype` is
one of those classes, when its event is sent, out of the squad (Scorch on
geo_01a at `levelstart`, Fixer on ras_02b at `PlayerAtFinalDoor`, Sev and
Scorch on yyy_01d). A map with neither has no squad mates: the player
starts ras_01a, ras_01c, ras_02a, ras_02b and geo_05a alone. Mates are not
carried from map to map; the original only carries their health
(`SaveSquadTravelInfo`). `spawn = "with_player"` (all three around the
player, the old way) is still there, and is what a map that can't be read
gets. `rc_game --script` and `rc_squad_sim` use it through
`SquadSpawnOverride`, because their checks are about all three.

Level scripts take mates out and put them back with
`ACTION_ChangeSquadMember` (`MemberTag`, `bAddMember`, default add; the
class's own script calls `RemoveFromSquad` / `AddToSquad` for every pawn
with that tag). Scripts name a mate by its `Tag`: the class name unless
the map gives another (`Seven`, `Delta07`, `Delta40` from a factory's
`PawnTagFromFactory`). The scripts part resolves the tag to the mate's
entity (`ScriptTags`) and sends `rc_game_core::SquadMembership`; the squad
part carries it out.

Each `rc_squad::Member` has a `Presence`: `InSquad`, `Away` (in the level,
out of the squad) or `Gone` (not in the level). Away and gone members are
skipped by orders, marked jobs, cover, fighting and reviving
(`Member::available`). An away member stands where they are unless a
script sends them somewhere: they follow `CharacterOrder`s (walk or run
to a point or to the player, turn to the player, wait for the player, be
put somewhere, hide, leave the level) using the squad's own path finding
and steering, and the script hears back (`CharacterOrderDone`) when they
get there. In the squad, orders that would walk or turn them are answered
at once (the squad leads them). Bodies come and go with presence
(`sync_bodies`). A save keeps each mate's presence and which spawners went
off (`presence`, `spawners_used` in `[squad]`).

yyy_06d's `NewSquad` spawners make second copies of Fixer and Scorch while
the first ones are still in the level. We have one body per squad mate,
so the one in the level moves to the spawner and takes the new name.

Not done: scripted animations on squad mates (`PlayAnim` is logged and
skipped), scripted walks for mates who are in the squad (the original
lets a high-priority script take over a squad member), and squad mates
other than Delta (clone troopers, "Fred" on ras_01c) joining.

Checked by `cargo test -p rc_squad_plugin --test rejoin_room` (no game
files) and `cargo run --release -p rc_squad_plugin --example rejoin_survey
-- <Maps folder>`, which lists every map's mates and checks every
`ChangeSquadMember` names one of them.

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
member holds fire while the line of fire, nearer than the target, passes
within `friendly_clearance` of the player's or another squad mate's whole
body (the 40-wide, 168-tall commando cylinder, or a low wide shape for one
lying down), widened by how far the shot can stray (our aim error plus
about 2 uu per 100 for the gun's spread). Until 2026-10-30 it measured from
the body's middle point only, so lines passing at shoulder or head height
went through friends (`fight::shot_passes_friend`). Shots go out as `rc_combat::FireShot` with the squad mate as the
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

### Taking cover (`rc_squad/src/cover.rs`, `[cover]` in `squad.toml`)

In the original, squad mates in a fight run to the map's cover points and
crouch behind them between bursts. How they pick one is compiled code, so
the rules are **our own**, but they lean on numbers the game stores:

| What | Value | From |
|---|---|---|
| Seconds of fighting before looking for cover | form up 2, secure area 0, search and destroy 0, hold 6 | each stance's `TimeBeforeSeekingCover` (SquadStance 6) |
| How far from what the order ties them to | the stance's `prevent_range` (1024 / 640 / 1024 / 256) | `PreventRange` |
| Cover nearest the tether, not nearest themselves | form up, secure area | `FavorCombatNearestTether` |
| A cover point shields enemies within | 140° around the way it faces | CoverPoint `FOVdegrees` |
| Look for another after | 15 s at one point | CTBot `CoverBoredomTime` |
| Look within | 1500 uu of themselves | guess (the droids' value) |
| Enemy at least this far from the point | 256 uu | our own |
| Leave cover after no enemy in sight for | 4 s | our own |

*Cover points.* The path network (`rc_nav::NavGraph`) already reads every
`CoverPoint` actor with the way it faces; `Navigator::cover_near` lists
them for the squad (`StraightWithCover` in tests).

*Taking one.* On each think, a member who is up, not on a job or a
revive, has had an enemy in sight for the order's wait, and holds no cover
yet, checks the cover points near it, nearest first: not taken by another
squad mate, within the order's prevent range of the tether, the enemy
inside the point's 140° cone and between 256 uu and its sight radius, and
a clear line from standing eye height at the point to the enemy's eyes (at
most 12 such line tests per look). With `cover_near_tether` the cheapest is
the one nearest the tether, otherwise the one nearest the member. The goal
becomes the floor under the point (`TakenCover::feet`), and the member
goes there ordered, all the way. Finding none, it tries again in 1 s.

*In cover.* Standing still at the point (`Squad::in_cover`), the member
crouches whenever it isn't about to fire: between bursts, and with no
enemy in sight. When the next shot is due it stands up first (one frame or
more until the body has risen), then fires. It sees from standing eye
height while in cover, so ducking doesn't lose the target. Crouched, its
`Hittable` body is the crouched height, so it's harder to hit, and it plays
the original's `CrouchBreathe` animation (`crouch` in `looks.toml`).

*Leaving.* No enemy in sight for `calm_time`; the point is further than the
prevent range from the tether (the player walked on); a new squad order
(every non-marker order calls everyone out, and the order's wait starts
again); a marked job or a revive; going down. After `boredom_time` at one
point with an enemy still in sight, it moves to another point if there is
one. Each take and leave is logged (`takes cover at ...`, `leaves cover:
why`), the map's cover point count is in the `path network:` line, and the
5-second `fight:` line counts covers taken.

### Specialities (`[member.*] speciality`, `auto` in `[job.*]`)

In the original each squad mate uses their own speciality without being
told: Sev (Commando07) snipes, Fixer (Commando40) throws grenades and
Scorch (Commando62) fires the anti-armour attachment. When and at what is
compiled code, so the rules are **our own**. They reuse the gun, sight and
throw numbers of the matching marked post (`[job.sniper]`,
`[job.grenade]`, `[job.anti_armour]`), so a mod changes a speciality the
same way it changes the post.

*Who.* Each `[member.*]` has `speciality`, the key of a `[job.*]`. The job's
`auto` says how it's used on their own: `far`, `armoured`, `group` or
`no` (only from a marked post). `[fight] use_specialities = "no"` turns
them all off. A member on a marked job or a revive doesn't use it.

| Rule | Who | When | Numbers |
|---|---|---|---|
| `far` | Sev | target at least `auto_range` away (1500 uu); sees out to the job's `sight_radius` (8000) instead of 5000 | sniper post's gun, accuracy 1, one shot every 1.5-2.5 s |
| `armoured` | Scorch | target armoured, at least `auto_range` (600 uu) away, no friend within `auto_clearance` (400 uu) of it; he picks the nearest armoured enemy before nearer unarmoured ones | anti-armour post's gun, one round every 2-3 s |
| `group` | Fixer | his target 600 to 1500 uu away, at least 2 enemies within the blast's 512 uu, no friend within 512 uu of it, at most every 10 s | grenade post's throw: 400 damage, 2 s fuse |

*Armoured.* An enemy counts as armoured when the squad's usual gun does
no more than `[fight] armoured_below` (0.5) of its damage to it, read
from its damage table (`data/combat/damage_types.toml`): a super battle
droid takes 0.15 from the DC-17m, a battle droid 1. The plugin works this
out for each enemy (`Threat::blaster_share`), so a new enemy only needs
its damage table to be treated right.

*Changing guns.* Sev and Scorch decide on every think which gun suits
their target. Changing either way logs `uses their speciality (...)` or
`puts away the ...` with the reason, and waits `switch_time` (0.5 s)
before the next shot. The gun's `Fire` goes to the combat code like any
squad shot, so the anti-armour round flies and explodes like the
player's. Fixer's grenade is a `Throw` event (`throws a grenade on their
own ...`) and a blast after the fuse, the same as a grenade post's. The
5-second `fight:` line counts speciality shots and grenades.

### How we check it without looking

`rc_squad_sim` runs the real squad and combat plugins on a real map with no
window, in fixed 1/60 s steps, with a scripted player. It logs every
order, set-off, arrival, stuck, job, target picked, first shot of each
burst, kill and revive, a once-a-second line with every member's position,
state and distances, and every 5 s a fight line with each member's shots,
hits, kills and revives. For the fight it puts stand-in droids (real
combat targets with the game's droid size and health that stand still and
don't shoot back) in sight of the squad. The checks are listed in
`README.md`. `rc_squad_sim MAP --jobs` is a second run, a marker-job tour:
for each job kind the map has it puts the player near the open marker of
that kind nearest the squad, orders it, and checks after 45 s that the job
was done (or the post taken), that the player asked for it once and that a
squad mate answered.

### Optional squad ammo (`rc_squad/src/ammo.rs`, `[ammo]` in `squad.toml`)

Off by default, because the original's squad never runs out and never
takes boxes (docs/RESEARCH.md, "Squad ammo and ammo boxes"). With
`[ammo] limited = "no"` every squad mate's ammo list is empty, which means
"never runs out", and none of the code below does anything.

A mod switches it on with a `squad/squad.toml` holding only the lines it
changes (every other value keeps its built-in default):

```toml
[ammo]
limited = "yes"
```

Then, all our own rules:

- **Counting.** Each squad mate starts with the `start` of each
  `carry_<id> = most, start` line (the DC-17m 120 of 300, sniper 20 of 20,
  anti-armour 4 of 4, thermal detonators 2 of 5, from the squad's own guns
  in the Properties package). Guns or grenades not listed never run out.
  Every shot (`Squad::shoot`) takes one from the gun fired, every grenade
  (Fixer's own and grenade posts) one from `throws`. `SquadEvent::OutOfAmmo`
  when one reaches 0.
- **Running out.** An empty speciality gun isn't picked (Sev keeps to the
  rifle with no sniper rounds); with the usual gun empty they hold fire.
- **Fetching** (`Squad::think_fetch`, before the usual think). A squad mate
  with no job (no marker, not reviving, not on a post) whose lowest gun is
  at or below `fetch_below` (25%) of the most walks to the nearest box that
  fills it, within `fetch_range` (1500 uu) of them and of the spot they're
  tied to. In a fight (a target, or an enemy within sight) they wait until
  that gun is empty, unless `fetch_in_fights`. Two squad mates never go for
  the same box. After `fetch_give_up` (15 s) they leave a box they can't
  reach alone, and wait as long again (back at their place) before trying
  another. Events `FetchAmmo` and `FetchStopped` (box gone, full again,
  busy, enemy to fight first, can't get to it).
- **Taking** (`rc_squad_plugin/src/ammo.rs`). `gather_boxes` copies the
  untaken `rc_combat::ammo_box::AmmoBox`es into `Squad::boxes` each frame;
  `take_boxes` runs after the squad moves and uses the player's touch test
  (`ammo_box::touches`). `Squad::take_box` fills only what isn't full, up to
  the most carried, like the player's rule; the box is then marked taken,
  so it's hidden and saved as taken. A squad mate walking over a box they
  have room for takes it even if they weren't fetching it.
- **Saving.** Each squad mate's counts are in the `[squad]` section of a save
  (`ammo`, left out when empty).

Tests: `cargo test -p rc_squad` (7 ammo tests) and
`cargo test -p rc_squad_plugin --test ammo_room` (switched off nobody goes
near the boxes; switched on Sev, low on rifle shots, and Fixer, out of
grenades, each fetch theirs and come back).

### Squad voices (`rc_squad/src/voice.rs`, `data/squad/voice.toml`)

The lines are the game's: each member's `audio_table` (squad.toml, like
`Delta07Audio`) and the player's `leader_table` (`Delta38Audio`) are
classes in `System/CTAudio.u`. Their `Table` holds one sound group per
event of `EPawnAudioEvent` (`System/Engine.u`), entry *i* = event *i*.
`rc_combat::audio_table` reads both at run time and `read_sounds` loads the
groups from `character_voice.uax`.

When to speak is compiled code in the original, so it is ours:
`rc_squad::Chatter` turns `SquadEvent`s (orders, first sight of an enemy,
cover, held fire, down, revive, jobs) into lines by the `[line.*]` rules in
voice.toml: who says it (`self`, `other`, `any`, `leader`), chance,
cooldown, `urgent` (said even while someone talks) and `wait` (wait for a
quiet moment). One squad line at a time; a line keeps the squad quiet for
its WAV length plus `gap`. The player's order lines are urgent and answers
wait for them; a new order drops answers to older ones. If the chosen
squad mate has no recording of a line, another standing one says it.
A rule marked `optional` may find no recording at all (Sev has no
GrenadeReady in the game); that is not counted as a missing line. Squad
events name jobs as the squad panel does ("Breach door"); `Chatter` turns
that into the job's key ("breach") for `job_assigned:breach` and the like.
A player (`leader`) rule speaks at most once per frame, because a breach
gives every squad mate a spot at once.
`rc_squad_plugin::voice` gathers events in `SquadHeard`, plays lines as
`Voice` sounds and logs every line said or skipped (`squad: ... voice:`).

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
at the sides (upright phones see more above and below). The long side stops
widening past the screen shape `video.widest_shape` (2.0) so wide windows
don't get a fish-eye look (`player::view_angle`). Unknown options in
`settings.toml` are logged as a warning and skipped. Keyboard, mouse, gamepads and touch all work at once. Details,
numbers and what is still to do (edge stretching on ultrawides, controller
pictures, gyro) are in `docs/DISPLAY_AND_INPUT.md`.

**On-screen display.** Corner brackets like the commando's visor, a
four-bar crosshair whose gap follows the weapon's spread and turns red on a
target, shield and health bars, the ammo counter, a squad list and short
messages. It reads only the readout resources, and logs its numbers every
two seconds.

**Music.** The original's music system ("iMuse") is rebuilt from what the
game's own files say, with nothing hard-coded per level:

- `rc_music::library` reads every piece of music from the `*_music.uax`
  packages (`SoundStreamed`: file, looping, fade-in and fade-out time and
  shape, loudness from `params_mus.uax`, likelihood) and every group
  (`SoundMultiple`: members picked at random by likelihood, an optional
  first piece, and for battle groups the "won" flourish and the "lost"
  music). Room music comes from the `AudioEnvironment` classes in
  `Properties/audioenv.u` (`iMuseState`, `iMuseOverlays`, overlay start
  delay and spacing, once only).
- `rc_level::music` reads what a map says: the `LevelInfo` battle settings
  (battle music, how many enemies, inner and outer radius, criteria) and
  every volume with an `AudioClass`.
- `rc_music::imuse` is the director, without Bevy. Inputs: script music
  actions (`SetImuseState`, `PlayMusic`, `EnableBattleEngine`,
  `SetBattleEngineParams`, `SetBattleMusicMulti`, `ForceBattleMusic`,
  `ForceVictoryFlourish`), rooms entered, where the enemies are and whether
  they are fighting, and whether the player is down. Output: play and stop
  commands for three layers (main, overlay, episode).
- `rc_game`'s `music.rs` carries the commands out with Bevy's audio and
  fades. Menus play what their menu class says (`BackgroundMusic`: the main
  menu's sung theme; the loading and pause menus have none, so the level's
  music pauses).
- Script actions reach it as `rc_game_core::MusicCue` messages; the enemies
  part lists its enemies in `MusicThreats`.
- `data/music/music.toml` holds only our own timings (how often the battle
  check runs, how long after the last enemy the fight ends, the
  player-down music) and an optional menu override.

Our reading of the battle rules (the original does this in compiled code,
so these are guesses checked against the settings' names and defaults): a
fight starts when the number of counted enemies inside the inner radius
reaches the threshold; it ends when no counted enemy has been inside the
outer radius for 3 seconds, then the "won" flourish plays and the room's
music comes back. "Proximity" counts any living enemy, "Attack" counts
fighting enemies anywhere, "Attack and proximity" counts fighting enemies
in range. A state set by a script during a fight waits until the fight
ends. The fade curves (linear, sine, log, inverse log) are our guess at
the shapes. Not done: `BattleVolume`, the death stinger, debriefing,
credits and title-card music, music in save games.

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
   (order 0), the first-person arms' camera (order 1) and the menus'
   camera (order 2). Once the player's camera
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

**The loading screens (`rc_game/src/loading_screen.rs`).** On entering
`Loading`, `LoadingScreenPlugin` picks the original's screen for the map
(`rc_menus::loading::choose`, the original's `LoadMap` rule over
`LevelLoadingInfo.int` and `System.ini`), resets that page's layout and fills
it in (`rc_menus::loading::setup`), and sets
`OriginalMenus::loading_page`. While that is set, the original page is
drawn instead of the placeholder loading screen (`menu.rs`, system set
`PlaceholderLoadingScreen`, which steps aside). The bar's value is kept
by the original engine's rules and `LoadMap`'s stages
(`rc_menus::loading::Progress`): the map's own `LoadingGate` part is the
first stage, the other parts count through the second, and the bar is
full when all are done. On leaving `Loading`, a `CTLoadingInfo` page
switches to its finished look (`rc_menus::loading::finish`) and
`LoadingDone::waiting` is set; when play starts it is paused at once with
`Page::LoadingDone`, which maps to the same original page, and its START
calls `resume`. Leaving `Play::Paused` clears the wait. A `CTLoading` page
just closes.

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
     droids, following the original's rules (read from CTGame.u's class
     defaults and script source, described in `rc_ai::spawns`): an event
     named like the factory's tag switches it on, the same event again
     off; it makes one at once and then one every `SpawnInterval` (to
     `SpawnIntervalMax`) while fewer than `MaxAlive` of its own live, until
     `Capacity` is used (unless `bGenerateForever`); at the factory itself
     (`SpawnMode` 2, the class default) or at its spawn points in turn (0),
     at random (1) or nearest the player (3). Each droid gets a tag and a
     death event (and a first-hurt event) from the factory, or from the
     spawn point when it is made at one. A factory sends `SpawnEvent` for
     each droid and `ExhaustedEvent` once its capacity is used and all it
     made are dead. Scripts can stop a factory (for good, and removing
     what it made: no death events) or refill it.
     With the level scripts running (`Battle::scripted`, set when the
     `rc_game_core::LevelScriptsRun` resource exists) only events switch
     factories on; the scripts run the walk-in `Trigger`s. Without them
     (the headless tools) a factory switches on when the player walks into
     its `Trigger`, or comes within `activate_unscripted_within` of one
     that has none.
     Not done: `bCovert` (only spawn out of the player's sight) is not
     checked; `bStartTimerAfterDeath` and `bAlwaysUseFactorySettings` are
     ignored; `SpawnEvent` is sent only when a droid is actually made (the
     original also sends it when the player is out of the factory's range).
   - *Placed in the map*: characters can also stand in the map with no
     factory. The original gives every placed pawn its AI as the level
     starts (Engine.u `Pawn.PostBeginPlay`, read only: through its
     `AIScriptTag`'s script when it names one, unless `bDontPossess` is
     set), at full health. Its own `Tag`, `Event` (sent when it dies),
     `FirstDamagedEvent`, `StartState`, `TeamIndex` and `AIScriptTag` are
     the same values a factory copies onto what it makes (CTGame.u
     `PawnFactory.SetupNewActor`), and a placed actor with no stored
     `Tag` has its class name as its tag. `rc_ai::read_placed` turns each
     one into a one-character factory (`Factory::placed`) at its own
     place: `Battle` makes them all on its first step (no "one per step"
     and no `max_alive_total` limit, since the original has them all from
     the start) and never again; events, `StopFactorySpawning`,
     `ResetFactory` and walk-in triggers leave them alone. They go after
     the real factories, so saves from before they were made still load
     (they then start as on a fresh level).
   `Battle` runs all of this for a map; the game plugin and `rc_droidsim`
   both use it.

3. **The game (`rc_enemies`).** `EnemiesPlugin` (see `docs/PLUGINS.md`):
   - On `OnEnter(Loading)` it reads the map's paths and spawners and the
     settings file through `GamePaths::data_file`, so a mod's file wins.
     It logs each spawner it runs (tag, type, capacity, where, its tags
     and events). It also reads the characters placed straight in the map
     (`rc_enemies::placed_enemies`): an actor counts when the game's
     classes (`GameData/System` and `Properties`, read once per run into
     `PawnClasses`) say it is a `Pawn` and an enemy file claims its class.
     Placed clone troopers are left to the cast (`rc_cast`), and placed
     kinds no file claims are logged and left out. Each one made is
     logged as "placed ... is a ...", and its body as "placed in the map
     as ...".
   - It hears the level's events (`rc_game_core::ScriptEvent`) and the
     scripts' `FactoryOrder`s, and sends what its spawners and droids
     send (`BattleEvent::Event`) back as `rc_game_core::RaiseEvent`; the
     scripts pass those on, so counters and scripts waiting for "all the
     droids are dead" carry on.
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
     otherwise and on leaving the level. Every living droid and whether it
     is fighting goes into `MusicThreats`, which the music's battle rules
     read.
   - Looks: the `[look]` section names the mesh (`BattleDroid` in
     `droid.ukx`) and which animation plays for idle, walk, run, back,
     sidesteps, fire, crouch, crouched fire and death. Every droid's points
     are posed on the CPU each frame with `rc_anim` (as `rc_character`
     does) and copied into its own Bevy mesh; 848 points each, so a dozen
     droids cost little. The level's light is baked into its surfaces and
     the scene has no ambient light, so droids get a steady share of their
     own colour (`ambient`, a guess standing in for each zone's ambient
     light, which we don't read yet).
     Each animation comes from the first of the mesh's linked animation
     sets that has it, in the mesh's order, so a character whose own set
     holds only cutscenes still finds its everyday moves (the Trandoshan
     mercenary borrows the commandos' `CloneCommandoSet`). `[look] skin`
     names a material to wear instead of the mesh's own, as the original
     does with a class's `Skins[0]` (the mercenary's mesh carries only a
     grey stand-in). `rc_enemy_look_check` (an example program in
     rc_enemies) loads every enemy file's look and checks it.
   - VR and other platforms: nothing here reads input or the screen; the
     player is found by its `Player` and `PlayerCamera` markers, so a VR
     camera works the same. No platform-specific code.

### Enemy types

Each enemy type is one file in `data/enemies/`, read on top of the battle
droid's built-in values, so a file only lists what differs. Its `[type]`
section names the game classes it stands for (`classes`, a trailing `*`
matches any ending) and its `body` (droid or organic). When a map loads,
each spawner's `Prototype` class is matched to a type, and that spawner's
enemies get the type's health, size, speed, weapon, accuracy, look and
sounds. A spawner whose class no file claims (squad mates, turrets) is
left to other parts and logged. `combat.uses_cover = false` makes an enemy
walk at you instead of taking cover (super battle droids);
`combat.drops` is the weapon it leaves when it falls. `[sound]` lists
sound names played when it spots you, is hurt and is destroyed.

To see which characters are placed straight in each map and which file
makes each one, run `rc_placed_enemies` (an example in `rc_enemies`, same
setup; writes `logs/placed_enemies.log`).
To see which file claims each map's spawners, and which classes no file
claims yet, run `rc_enemy_spawners` (an example in `rc_enemies`, needs the
game folder in `RC_GAME_DIR`; writes `logs/enemy_spawners.log`). A new
enemy type that fights like the existing ones (walks, shoots, hits up
close) is only a new file: the Geonosian elite (`geonosian_elite.toml`
with its weapon `data/weapons/elite_beam_enemy.toml`) needed no code. A
beam weapon is an `instant` shot fired as a burst of quick hits.

When a class could match two files, the first file in name order wins, so
give each file exact class names where they overlap (the Trandoshan
files list `TrandoshanMercenary`, `TrandoshanMercenaryGrenadier` and so on
one by one; a test checks each class has exactly one file).

**Shields.** `combat.shields` gives an enemy an energy shield on top of its
health (the Kashyyyk Trandoshan mercenaries: 100, regaining 10 a second).
Damage goes to the shield first, as for the player (`rc_combat::Shield`).

**Shotguns.** A weapon's `shot.pellets` fires that many instant hits at
once, each up to `shot.spread` rotation units off the aim (the slavers'
shotgun: 8 pellets, 1700, the game's `ProjPerFire` and `Spread`). How the
original scatters them is our reading.

**The Trandoshans.** Classes that the game builds in
`Properties/properties.u` straight from a class in `ctcharacters.u` (like
`TrandoshanMercYYYUp` from `TrandoshanMercenary`) keep that class's own
values, not its "...Defaults" class's, so the Kashyyyk mercenaries have
100 health and a shield while the Prosecutor's have 220 and none. The
evidence: those classes restate every value the Defaults class changes
that they want (the grenadier repeats Health 220, Shields 0, its damage
table and sight). The files: `trandoshan` (Prosecutor mercenaries),
`trandoshan_kashyyyk`, `trandoshan_grenadier`, `trandoshan_slaver`,
`trandoshan_slaver_berserker`, `trandoshan_slaver_grenadier`,
`trandoshan_heavy` and `trandoshan_scavenger`, with weapons
`trandoshan_shotgun`, `trandoshan_rifle` and `scav_beam`.

Grievous's bodyguards have 250 shield, which never recharges.
**Missiles.** An enemy weapon with a
`lobbed` shot and a grenade file with `flight.gravity = 0` flies straight
and goes off on touching anything: the bodyguards' missile rack
(`grievous_missile_rack.toml` with `data/grenades/grievous_missile.toml`).
To check that a level's own scripts and its spawners work together without
a window, `rc_guard_check` (an example in `rc_enemies`) runs yyy_01e's
scripts and spawners with a stand-in player that kills each bodyguard 3 s
after it appears, and checks the level ends.

**Grenades.** `[grenades]` gives an enemy grenades: which file in
`data/grenades/` and how many (the Trandoshan mercenary: two of
`thermal_detonator_enemy`, the original's `ThermalDetonatorEnemy`). The
rules for when are ours, since the original decides in its compiled
engine; the numbers are its settings. At each decision while fighting, an
enemy that still has one, threw its last at least `min_time` ago
(`MinGrenadeTime`), saw its enemy within `seen_within` seconds, and is
`min_range` to `max_range` away from it (the grenade's `ValidRange`)
throws with chance `probability` (`GrenadeProbability`), unless a friend
stands within the blast radius of the landing spot. It aims at the floor
by the enemy, off by the same aim error as its shots, and works out the
arc itself (`rc_ai::brain::throw_arc`): first the gentlest lob that
reaches (the slowest throw, which comes down steeply and rolls on little),
then the flattest arc at full speed for under a low ceiling; if the level
blocks both, it tries again 3 s later. It doesn't shoot for `throw_time`
(the throw animation, `[look] throw`). The AI sends `AiEvent::Throw`;
`rc_enemies` turns it into the same `rc_combat::ThrowGrenade` the player's
button sends, so the grenade flies, bounces and goes off with the
grenade's own rules. To aim, `rc_enemies` reads the grenade file's speed,
gravity and radius. Saves keep how many each enemy has left.

**Melee (the Geonosian's force pike).** `combat.melee_range` above 0
gives an enemy a close-up hit: `melee_damage` of `melee_damage_type`
every `melee_delay` seconds. An enemy with no gun (`weapon = ""`) always
rushes in; one with a gun only when its enemy is within
`lunge_distance` (`LungeDist`). Rushing in (`Maneuver::Charge`) heads
straight at the enemy when the level leaves a clear path (checked with
its body lifted a step off the floor), else along the paths, and is
re-planned at every decision as the enemy moves. Once within 70% of its
reach it stops; it swings when its enemy's centre is within reach, within
30 degrees of straight ahead, with a clear line. **Our rules**: the
original's are compiled. The AI sends `AiEvent::Melee`; `rc_enemies`
turns it into `rc_combat::MeleeAttack`, which `rc_combat` resolves with
the same rule as the player's melee (a line that long against the
characters' cylinders, walls block it, the attacker's own team is
skipped) and the target's damage table. `[look] melee` names the
animation.

**Several enemies to fight.** Droids fight the player and the squad mates
(every living `Team::Republic` character). Each keeps fighting the one it
fought last while it can still see it, otherwise picks the nearest one it
can see, otherwise hunts the player. **Our choice:** the original's target
choice is in its compiled engine. A stunned droid (flash or EMP grenade)
stands still and does nothing until the stun wears off, and grenade blasts
are heard like shots.

**Two sides: clone troopers fight the droids.** A type's `[type] side`
is `"separatist"` (the default) or `"republic"` (the original's
`TeamIndex`: droids 5, clone troopers 2). The clone troopers
(`clone_trooper.toml` for `CloneTrooper`, `clone_trooper_tm.toml` for
`CloneTrooperTM`, 16 spawners on geo_01a) come from the map's spawners
like the droids and run in the same `Battle`, with the same brain. Each
step, separatist fighters choose among the player, the squad and the
republic fighters; republic fighters choose among the separatist ones
only, and with none in sight they don't hunt anyone. Each side keeps clear
of its own side's line of fire (`ally_in_line`, which widens with the
shooter's aim error so a burst doesn't stray into a friend), counts
against its own `max_alive_total`, and gets `rc_combat::Team::Republic`
or `Separatist`, so the player's crosshair and the squad treat troopers as
friends. The music's battle rules count only separatists. `[combat]
can_crouch = false` (the original's `bCanCrouch`) keeps troopers standing.
What the files don't say is listed in `clone_trooper_tm.toml`: the
troopers' attack state (`StateTrandoshanAttack`) works in compiled code,
so they fight the way our droids do. A type's `[look] held_weapon` names
the weapon class whose model it holds (the original's `RequiredEquipment`:
`DC15BlasterClone` for `CloneTrooper`, `DC15BlasterCloneTM` for
`CloneTrooperTM`). `CloneTrooper`'s own value in ctcharacters.u is the
commandos' `CTInventory.DC17mBlaster`, but the game lays
`CloneTrooperDefaults` (Properties/properties.u) over the class (core.dll
builds the name `Properties.%sDefaults`), and that sets
`Properties.DC15BlasterClone`; its numbers are in
`data/weapons/dc15_blaster_clone.toml`. `rc_enemies` reads it once per type with
`rc_effects_plugin::held::HeldModel` (as the level's cast does), hangs it
as a child of each new fighter (`HeldWeapon`), and moves it onto the bone
`weaponAttach_R` each time the body is posed (`DroidLook::bone_frame`).

**Droid dispensers.** The maps' `BDDispenser` and `SBDDispenser` actors
(26 in 14 maps; the 3 `DroidekaDispenser`s name no prototype and make
nothing yet) are factories too. `rc_ai::read_factories` reads them with
their class defaults (`Dispenser::class_defaults`, numbers from
ctgame.u's `DroidDispenser`, `BDDispenser(Defaults)` and
`SBDDispenser(Defaults)`) under the map's own values, and the battle runs
their script (`rc_ai/src/dispense.rs`, after `DroidDispenser`, `Factory`
and `PawnFactory` in ctgame.u): offline until an event with its tag
plays its intro (`IntroAnim`, or `ActionBreathe`); when the intro ends it
warms up (`WarmUpInfo`, `WarmUpTime`) or, with an `OnlineTag`, waits for
that event; online it makes one droid at once and then one per
`SpawnInterval` (random up to `SpawnIntervalMax`), its first
`NumInitDroids` as a wave and then up to `MaxAlive`, waiting for a death
when full (`bStartTimerAfterDeath`), within `MaxInWorld`. Each droid
rides slot `NextSpawnPoint` (bone `droidAttach<n>`, its centre its own
collision height out along the bone's Z axis, turned like the bone) in
`DispBreathe` while the machine plays `Dispense<n>`; at the slot
animation's `AnimNotify_ReleaseDroid` it is let go (`SpawnEvent`), plays
`DispUnfold` once while that animation's root bone carries it
(`PHYS_RootMotion`), then walks, can be hit and fights. Health steps at
66 % and 33 % show `SlightlyDamagedInfo` and `HeavilyDamagedInfo`; at 0 it
shows `DamagedInfo` (the maps put their "dispenser destroyed" event
here), the droid still on its slot is destroyed with it, and once its
droids are dead it sends `ExhaustedEvent`. Each look step (`TransitionInfo`)
swaps the model's first material, replaces its one effect (centre plus
offset, moving with it), plays its animation and sends its event. The
machine's own `Physics` is `PHYS_RootMotion`: its intro's root bone
(`Move`) carries it, which is why the maps place `entranceSky` dispensers
3000 uu above the floor (it drops in), `EntranceSky704` ones 704 uu up and
the slavers' `SlaversPushDispencer` 1105 uu back (it is pushed in).
`rc_enemies/src/dispensers.rs` draws it from `Dispensers.ukx` with
`rc_model`, holding the root bone still (`Model::posed_rooted`) and
moving the entity instead (`Battle::dispenser_location`), gives the
battle its animation numbers (`DispenserMotion`: intro length and travel,
each slot's length, release moment and bone path), plays its
animations' sounds, and passes its `Health` changes (its damage table
`DV_DroidDispenser`: blaster bolts do nothing) to the battle. Saves keep
each dispenser's step, health, damage looks and droids
(`SpawnerSave::dispenser`). A droid on a slot takes the slot bone's
rotation (Engine.dll's `UpdateAttachments`), keeps it when let go
(`DetachFromBone` is only `SetBase(None)`) and while unfolding (its `Move`
bone doesn't turn). The intro animations' `AnimNotify_Effect`s
(`Dispenser_Landing_Dirt`, `Dispenser_Landing_Metal`, at the `root` bone)
start their effects as Engine.dll's `UAnimNotify_Effect::Notify` places
them (`rc_model::NotifyEffect`). `TransitionSound` and `DispenseSound`
play when a map gives one (no class default does).
`rc_enemies/src/dispenser_world.rs` puts the machines in the world: one
with an `AttachTag` rides the mover tagged or named so (`UGameEngine::LoadMap`
bases it; `ULevel::MoveActor` carries it rigidly: `rc_ai::Carry`, set each
frame from `LevelMovers`), which flies ras_04a's and ras_04b's dispensers
in; its cylinder is solid for bodies (it wraps `LevelCollision`; shots
still hit it as a target, and a body that starts inside may walk out);
and while it plays `entranceSky`, `EntranceSky704` or `EntranceSky384`
whoever it touches takes their full health as `Crushed` damage each frame
(`DroidDispenser.Bump`; the engine bumps both ways on a blocked move).
Not done: the falling machine isn't held up by who it lands on, sight
lines pass through it, and `Crushed` kills the player outright in the
original (`bKillsNotIncapacitatesPlayer`) where the port treats it like
any death. `rc_dispenser_check` (no window) runs every map's dispensers
and checks their steps, timing and where the droids come out;
`rc_dispenser_view` shows one in a window and takes pictures.

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
- To throw a grenade (squad mates, enemies): send `ThrowGrenade { thrower,
  origin, velocity, grenade }` (Bevy space, units per second). When it goes
  off, `Blast { grenade, point, radius, instigator, hit }` is sent.
- `DamageTable(name)` names a character's damage table (the original's
  `DamageVariance`, in `data/combat/damage_types.toml`). Every hit by a
  shot, melee or blast is multiplied by the table's entry for the hit's
  damage type (a weapon's `shot.damage_type`, a grenade's
  `blast.damage_type`). The player gets one from `player.toml`, enemies
  from `combat.damage_table`; squad mates should use
  `DV_OrganicCTSquadDamage`. No table means x1.
- `Body` (droid or organic) is for logs and AI. `Stunned { left,
  damage_per_sec, kind, stops, .. }` is a grenade's lasting effect: when
  `stops` is set the character can't act (AI should check it); it hurts
  `damage_per_sec` while it lasts.
- `read_sounds(paths, package, names, audio)` reads sounds by name from a
  game sound package, for any part.

### Grenades (`grenade.rs`, `throw.rs`)

A grenade kind is a file in `data/grenades`. `grenade.rs` holds the plain
rules: a thrown grenade starts at the hand (`throw.offset` from the eye)
with the view's direction times `throw.speed`, plus `throw.toss_up`
upwards. Gravity pulls it down every frame. When it touches a wall, floor
or character it bounces off the surface, keeping `flight.bounce` of its
speed (`flight.actor_bounce` off a character); on a floor, once slower
than `flight.rest_speed`, it lies still. The fuse is the original's
(`CTProjExplosive`): it goes off `fuse.time` seconds after the throw, or
`fuse.after_impact` seconds after it first touches anything if that is
sooner; `fuse.on_actor` / `fuse.on_world` make it go off on touch.
`flight.stick_to_world` / `stick_to_actors` make it stick where it lands
(the sonic detonator), and `fuse.proximity` makes a stuck one go off when a
character it can see comes that close (checked every
`fuse.proximity_interval` seconds). The thrower never sets it off.

The blast is the engine's `HurtRadius` rule: a character it went off on
takes the full damage; every other character within `blast.radius` (plus
its own radius) whose centre the blast can see takes `damage x (1 -
(distance - its radius) / radius)`, so full damage up close and nothing at
the edge. Each hit is then multiplied by the target's damage table for
`blast.damage_type`. A `[status]` section adds a lasting effect
(`Stunned`) to every character that can see the blast within the radius
(plus `status.radius_change`): it lasts `status.max_time` at the centre,
falling to nothing at the edge, and hurts `status.damage_per_sec`. A
character whose table gives `status.damage_type` x0 is immune (droids
aren't stunned or set alight). `half_on_commandos` halves it on clone
commandos (the EMP), `player_must_face` spares a player looking away (the
flash), and `stops = false` lets characters keep acting (the thermal's
burn).

`throw.rs` is the Bevy side: the throw and next-grenade buttons, the
`ThrowGrenade` message (anyone can throw), grenades in flight, the `Blast`
message when one goes off, stuns wearing off, and the grenade line on the
display. The player's kinds and their order are `grenades` in
`data/combat/player.toml`; each starts with its file's `start_count`.

### Attachments, zoom, melee and pickups

- The DC-17m's sniper and anti-armour attachments are their own weapon
  files and their own entries in the weapon cycle (rifle, sniper,
  anti-armour, pistol), each with its own ammo. **Our choice:** the
  original swaps the attachment onto the rifle with an animation.
- `fire.zoom_fovs` gives a weapon zoom steps: each press of the zoom
  button narrows the view to the next one (degrees across a 4:3 screen),
  and after the last it zooms back out. Combat writes it to
  `WeaponReadout.zoom_fov` and the game's camera uses it; a VR mod can
  ignore it. Switching weapon or reloading leaves the zoom. While zoomed,
  the player's shots do `fire.zoom_damage` times the damage (1.72 for the
  sniper).
- `shot.kind = "lobbed"` fires a grenade file (`shot.grenade`) along the
  aim at `shot.speed`: the anti-armour round is an impact grenade.
- Melee (`melee.rs`): a line `[melee] range` long from the eye along the
  view; the first character on it takes `[melee] damage` of type
  `CTDamageMelee` (times its table: battle droids take x3). The weapon in
  hand's `[melee]` section wins; `player.toml`'s is the fallback. Walls
  block it.
- Pickups (`pickup.rs`): see "Weapons lying in the levels" below. Enemies
  drop the weapon file named by `combat.drops` in their settings file.
- The ACP array gun uses `shot.pellets` (8) like the Trandoshan shotgun:
  see "Shotguns" above.

### Ammo and grenade boxes (`rc_level::pickups`, `rc_combat/src/ammo_box.rs`)

The campaign maps place 477 boxes of seven kinds (`DC17mBlasterBox` 225,
`DC17mAntiArmorBox` 64, `ThermalDetonatorBox` 62, `EMPGrenadeBox` 50,
`DC17mSniperBox` 42, `SonicDetonatorBox` 17, `FlashBangBox` 17). All come
from the engine's `Ammo` pickup class. A map stores only where each box
is; everything else is the class's defaults with its "...Defaults" class in
`Properties/properties.u` on top, as for the weapons:

| Box | Gives | Touch cylinder (radius, half-height) | Model |
|---|---|---|---|
| DC-17m | 300 shots (`DC17mBlasterBoxDefaults`; 120 in `CTInventory.u`) | 40, 15 | `PickupMeshes.ClonePickups.DC17mBox` |
| Sniper | 20 | 50, 30 | `SniperBox` |
| Anti-armour | 4 (`DC17mAntiArmorBoxDefaults`; 6 in `CTInventory.u`) | 50, 31 | `AntiArmorBox` |
| Thermal, EMP, flash | 5 | 45, 44 | `ThermalDetonatorBox`, `EMPBox`, `FlashGrenadeBox` |
| Sonic | 5 | 45, 44 | `GeoGrenadeBox` (the Defaults class swaps it) |

No campaign map changes a box's amount; one box sets `bForceRespawn`
(geo_01a's grenade crate by the start, back 0.1 s after it's taken).

**Reading** (`rc_level::pickups`, plain Rust): `read_boxes` finds every
actor whose class comes from `Ammo` and reads its amount, the ammo class it
fills (`InventoryType`), its cylinder and whether it comes back.
`map_collision::load` does this, leaves every pickup out of collision (the
game's pickups don't block: touching one is how you take it; before, the
boxes were solid), and drops each box onto the ground under it, as the game
drops pickups at the start. **Our choice:** a box more than 40 uu above
the ground found stays where the map put it, because it rests on
something the port doesn't count as ground (ten boxes, 50 to 745 uu up).
The level viewer draws each box on its own (`MoverPart` with its export
index) with its "...Defaults" model, dropped the same way.

**Taking them** (`ammo_box.rs`): `rc_level_plugin` hands the boxes over as
`rc_game_core::pickups::LevelAmmoBoxes`; combat makes an `AmmoBox` entity
for each. `data/pickups/ammo_boxes.toml` says which weapon or grenade file
each of the original's ammo classes fills, and can change a box class's
amount. The rules, from the game's `Pickup` and `Ammunition` scripts:

- A box is tried when the player starts touching it (the two cylinders
  overlap: the player is 40 wide and 168 tall from the feet). Standing on
  it doesn't keep trying.
- It only fills ammo the player already carries, and only if it isn't
  full; otherwise it's left lying.
- It adds its whole amount, capped at the most carried, and is used up.
- If the weapon in hand was empty, it reloads at once.
- Taken boxes are gone for good in single player, unless the map marks
  them to come back. Their model is hidden.

The `[pickups]` section of a save file lists the boxes taken, by name.
`AmmoBoxTaken` is sent for each box taken.

Squad mates don't take boxes, on purpose: in the original only the player
can (`bCanPickupInventory`), and anyone else whose weapon runs dry gets a
fresh magazine for free (`bInfiniteClipsForNonPlayers`). The port's squad
already fires without running out, which matches. Evidence in
`docs/RESEARCH.md`, "Squad ammo and ammo boxes". A mod can switch on squad
ammo and box fetching: see "Optional squad ammo" under Delta Squad.

#### Pickup sound and message

Each box class names its own `PickupSound` and `PickupMessage`
(`CTInventory.u`, with the `...Defaults` class in `properties.u` on top,
read by `rc_level::pickups::read_box` like the amount):

| Box | Message | Sound |
| --- | --- | --- |
| DC-17m | DC-17m Blaster Clips (`...Defaults`; "DC-17m Blaster" in the class) | `GEN_Sound.Weapons.GenericWeaponPickup` |
| Sniper | DC-17m Sniper | same |
| Anti-armour | DC-17m Anti-Armor Charge Packs (`...Defaults`) | same |
| Thermal, sonic, flash | Thermal Detonators, Sonic Detonators, Flashbang Detonators | same |
| EMP | EMP Grenades | `GEN_Sound.Interface.int_GEN_wepPickup_01` |

What the original does with them (engine.u `Pickup.AnnouncePickup`,
`PlayerController.HandlePickup`, `PickupMessagePlus`, `LocalMessage`): the
sound is played for the player only, from where they are
(`ClientPlaySoundLocally`), and the message is a `PickupMessagePlus`:
centred, 35% of the way down the screen, white, one at a time (a new one
replaces the last), 3 seconds, fading. The real drawing is in native code
(`CTHUD` keeps `LastPickupText` and `fPickupStartTime`), so the exact font
size and fade are **our choice**.

In this port: `take_boxes` sends `rc_game_core::pickups::PickedUp { text,
sound }` for each box taken (a full player's touch sends nothing).
Anything else that can be picked up (weapons on the floor, health) can send
the same message. `rc_combat::pickup_sound` reads the sounds a level's
boxes name when the level loads (`PickupSounds`) and plays them, at full
volume scaled by EFFECTS VOLUME, not placed in the level. The original HUD
(`rc_game/src/original_hud.rs`) shows the text in the `pickup` element of
`data/ui/hud.toml` (`{pickup}`, `when = "pickup"`, `fade = "pickup"`); how
long it stays and fades is `[pickup_message]` there. The placeholder HUD
adds it to its message list instead. The log says `pickups: played sound
GEN_Sound.GenericWeaponPickup` and `original hud` redraws as it fades.

### Weapons lying in the levels (`rc_level::pickups`, `rc_combat/src/pickup.rs`)

The campaign maps place 40 weapons: 14 bowcasters, 9 ACP repeaters
(`SMG`), 6 rocket launchers, 3 ACP array guns (`Shotgun`), 3 concussion
rifles, 2 anti-armour and 1 sniper attachment, 1 DC-17m, 1 Elite Beam
(geo_03d) and 1 LS-150 heavy repeater (`TrandoshanRifleMP`, ras_04b). Each
is a `CTPickup` whose `InventoryType` is a weapon class. As for the boxes,
the map stores only the place: the weapon, size (`CollisionRadius`,
`CollisionHeight`), model and `PickupMessage` come from the pickup class,
and the shots it holds from the weapon's `PickupAmmoCount` with its
"...Defaults" class on top (24 for the bowcaster, 160 for the repeater).

**Reading:** `rc_level::pickups::read_weapons` (`PlacedWeapon`).
`map_collision::load` drops them onto the ground like the boxes; the level
viewer draws each on its own (`MoverPart`), so it can be hidden.
`rc_level_plugin` hands them over as
`rc_game_core::pickups::LevelWeaponPickups`. A weapon class named without a
package (`Bowcaster` in `CTInventory`) is looked up in the pickup class's
own package.

**Taking them** (`pickup.rs`): combat makes a `WeaponPickup` entity for
each one `data/pickups/weapon_pickups.toml` knows (the original's weapon
class to this port's weapon file). The rules, from the game's `Pickup`,
`Weapon`, `PlayerController` and `EliteBeamPickup` scripts:

- Touching a weapon you carry takes its ammo when touching starts, up to
  the most you carry; full, it's left lying. The DC-15s (it recharges)
  never takes ammo.
- A weapon you don't carry needs Use while touching it. Meanwhile the
  display shows the game's "PRESS @ TO PICKUP" (the prompt file's text,
  `@` replaced by the Use button) and the weapon's name.
- Weapons in `[carry] one_at_a_time` (the game's secondary slot,
  `InventoryGroup` 5: everything taken from the enemy) are carried one at
  a time. Taking another throws the one carried down in front of you with
  the shots it has left (a dropped `WeaponPickup`).
- `swap_when_empty` weapons (the Elite Beam) never give ammo; Use swaps one
  in only once yours is empty.
- A new weapon goes into your hands. Taking it or its ammo sends
  `PickedUp` (see "Pickup sound and message" above) with its name
  (`PickupMessage`) and its own `PickupSound`: `GEN_Sound.GenericWeaponPickup`
  for the bowcaster, `GEN_Sound.int_GEN_wepPickup_01` for the rest. A
  dropped weapon uses `[sound]` in `weapon_pickups.toml`.

Weapons an enemy drops, and ones you throw down, follow the same rules,
with the game's `CTPickup` width (84) and a half-height of 15 (our
choice). `WeaponTaken` is sent for each. The `[pickups]` section of a save
lists the map's weapons taken (`weapons_taken`); weapons on the ground
that the map didn't place are not saved.

The new weapon files (`bowcaster`, `acp_array_gun`, `concussion_rifle`,
`rocket_launcher`, `elite_beam`, `heavy_repeater_mp`) use the game's
numbers. Not built: the bowcaster's charged shot, rocket homing, the speed
penalty of heavy weapons, and the concussion rifle's instant hit (it is a
very fast round that blasts where it lands, `grenades/concussion_rifle_blast.toml`).
The sound package names are guesses from the sound names (`wep_GEN_` in
`gen_sound`, `wep_YYY_` in `yyy_sound`).

### Modding

Every weapon is one file in `data/weapons`. A mod folder's file with the
same name replaces it, and a new file is a new weapon. The player's health,
shield and starting weapons are in `data/combat/player.toml`. What each
ammo box fills, and its amount if a mod wants another, is in
`data/pickups/ammo_boxes.toml`; which weapon file each weapon lying in the
maps is, which weapons are carried one at a time, the pickup prompt and
the sound of taking a dropped weapon are in
`data/pickups/weapon_pickups.toml`. Where and how long the pickup message
shows is in `data/ui/hud.toml` (the `pickup` element and
`[pickup_message]`); any element can fade with `fade = "pickup"`.

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
- Weapon switching is instant for firing; the arms play the holster and
  draw animations (0.37 s and 0.67 s for the DC-17m) but you can shoot the
  new weapon before they finish. In the game you can't.
- The bolt colour is ours. Weapons without `[arms]` (the picked-up E-5)
  show their plain model at our `[model] offset`.

### First-person arms (`rc_combat/src/arms.rs`, `arms_view.rs`)

The game draws each player weapon as one animated model: the commando's
arms and the gun together, from `GameData/Animations/hudarms.ukx`
(`HudArmsDC17m`, `HudArmsSniper`, `HudArmsAntiArmor`, `HudArmsDC15s`,
`HudArmsUzi` for the Trandoshan repeater, and more for turrets and the
other pick-ups). The weapon class's `Mesh` names it, and the textures are
in `HUDArmsTextures.utx`. A weapon file's `[arms]` section names the mesh;
it's read at run time like the squad's models (`rc_model`), and posed on the
CPU each frame.

**Where it goes.** The game's `Weapon.RenderOverlays` puts the model at the
eye, turned with the view, and draws it after the world with the depth
cleared and its own field of view (`DisplayFOV`, 85 degrees across 4:3).
The mesh's own origin is the eye: forward +X, right +Y, up +Z. Measured
(`rc_model --example rc_arms_probe`): the DC-17m rests 5 to 68 uu in front
of the eye, 13 left to 37 right, 4 to 50 below; `Holster` ends with
nothing in view and `Load` starts with nothing in view; `Aim` brings the
sights to the middle. We copy this with a second camera, a child of the
player's camera, that sees only render layer 2 (the arms) and draws over
the world's picture with a fresh depth buffer, so the gun never pokes into
walls and a wider view setting or the Panini view doesn't bend it. Camera
orders: sky -1, player 0, arms 1, menus and display 2.

**Which animation** (the game's `Engine.Weapon` script, our words):

| What | Animation |
| --- | --- |
| standing still | `ActionBreathe`, over and over |
| still for 15 s (`PlayIdleTime`) | `Idle` once, then breathing again |
| moving under 226 uu/s / faster | `Walk` / `Run` |
| zoomed in | `AimBreathe`, `Aim` when moving, `AimFire` per shot (if the set has them) |
| each shot | `Fire`, from the start |
| reload | `Reload` |
| grenade thrown / melee | `ThrowGrenade` / `MeleeAttack` |
| switching | old weapon `Holster`, then new one `Load` |
| switching between DC-17m attachments | `AttachHolster`, then `AttachLoad` |

The game holds the weapon in its `Reloading` state until the `Reload`
animation ends, so the animation length *is* the reload time. That gave
the real reload times of the sniper (78 frames, 2.6 s), anti-armour
(42 frames, 1.4 s) and the Trandoshan repeater (65 frames, 2.17 s), which
were stand-ins before.

**Using things: the hands** (`rc_combat/src/hands.rs`,
`data/combat/hand_signal.toml`). The game's `HandSignal` is a "weapon"
(mesh `HudArmsHands`) that using something switches to
(`CTPlayer.StartHandSignal`, `SwitchWeapon(11)`). For a bacta dispenser
(`HandSignal.SetupBactaHealer`) its moves are `BactaLoad` (up),
`BactaLoop` (fired again each time it goes idle while Use is held,
`CTPlayer` state `UsingItem`) and `BactaHolster` (down); then
`Pawn.SwitchToLastWeapon` brings the gun back with `Load`. We follow the
same order: the scripts set `UseItem::hands = "bacta"` while healing, the
arms put the gun away with `Holster`, play the hands with their moves
under the usual names (`hold_fire` keeps `Fire` going), and bring the gun
back when it's cleared. Measured in the shipped files: the three bacta
moves are 3-frame poses (0.1 s) with every point of the hands below and
behind the eye, outside the view, so on screen the gun goes down and
comes back and no hands show. The keypad and squad-order moves in the
same set do come into view, so the measurement is sound.

The sniper attachment has `bZoomedUsesNoHUDArms`: its arms are hidden
while zoomed (`hide_zoomed`). The arms are also hidden while the player is
down.

**Our choices:** the game changes between breathing, walking and running
only when the current animation ends; we change at once. No blending
between animations yet. The arms are drawn unlit at `brightness` 0.75:
the game lights them by the player's zone, which we don't read yet.

**The ammo counter on the gun** (`counter.rs`, plain rules, 5 tests). The
DC-17m and sniper models have a little screen made of model pieces whose
materials are `TexPanner2D`s: they slide one picture (`DC17mAmmo`) sideways.
The ammo class names them (`DC17mBlasterAmmo`: `PannerDigit100`,
`PannerDigit10`, `PannerDigit1`, `PannerCharge`, `PannerClip`); the weapon
file lists them as `counter_digits` and `counter_bar` in `[arms]`.

| Piece | Shows | Slide (picture widths, sideways) |
| --- | --- | --- |
| three digits | the ammo carried, magazine included ("120" at the start) | strip of 11 cells (blank, 0-9): `(digit - 9) / 11` |
| bar | how full the magazine is; refills over the reload | `-0.5 * (1 - shots / magazine size)` |

The numbers come from the game's engine (`Engine.dll`, read for its
numbers: 1/11 and 2/11 per digit, -0.5 for the bar, hundreds = count / 100,
no blank leading zero). The game slides by `digit / 11 + 2 / 11` on a
repeating picture; `(digit - 9) / 11` is the same cell without needing the
repeat. `arms_view` sets each piece's `uv_transform` when it changes and
reads the slides back into `ArmsReadout::counter`, which `rc_shoot` checks.
Not done: the anti-armour's grenade digit (`PannerClip`, shown when the
ammo has `DigitalClipDisplay`) and the DC-15s charge bar (its recharge
uses other numbers we haven't read); they show their pictures unslid.

**The arms' sounds** (`arms_sound.rs`). The reload clatter, the click of
bringing a weapon up, putting it away, the melee swing and the grenade
throw aren't in the weapon classes. They are `AnimNotify_Sound` events in
the arms animations: each names a sound and a moment (0 = start, 1 = end).
`rc_model` reads them into `Model::sounds`; when a weapon's arms are read,
`ArmsSounds` reads every sound they name (all in `GEN_Sound`). Each frame
`spans` works out which stretch of which animation the arms went through
(including the end of a reload or holster that finished this frame, the
old weapon's holster when switching, and both ends when breathing or
running goes round), and `play` plays every event in it. List them with
`cargo run -p rc_combat --example rc_arms_sounds -- <game folder>`. What
the five arms make (2026-11-10): reload (DC-17m 1, sniper 5, anti-armour 1,
repeater 2, pistol none, as its charge refills by itself), holster and
load 1 each, attachment swaps 1 each, idle 1-3, run 2 armour footsteps,
throw 2-3, melee 1-6. **Our choices:** played for the player only at full
volume, not placed in the level; quiet while the player is down or a
cutscene gives them other hands; still played when the sniper's zoom hides
the arms. A weapon file's `sound.reload` is still read (for a weapon with
no arms), but the DC-17m's was taken out, since its animation plays it.

**Checks:** 8 unit tests in `arms.rs` (with the DC-17m's real lengths), 9
in `arms_sound.rs`;
`rc_shoot --script` checks the arms play `ActionBreathe`, `Reload`,
`Holster`, the pistol's `Fire`, and the rifle again after switching back,
that the reload played the game's own reload sound, and that the gun corner of the picture is textured; `rc_game --script`
checks the arms show the weapon in hand after loading a level.

**VR:** the arms are a child of the player camera with their own camera,
so a VR mod can hang the same posed model off a tracked hand instead.

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
  area whose pictures and letters are sized for 800x600 (the original's
  `XInterface.dll` ResScale), with stretched pictures' corners one texel
  to one screen pixel (`quads::set_pixels_high`). `rc_menus::render` paints the same rectangles
  in software; the script compares that picture with a screenshot.
- `rc_menus::gpu` makes copies of pictures for Bevy: each render style
  (normal, masked, translucent or additive, alpha) and each tint gets its
  own copy, with transparency adjusted so Bevy's linear-light blending
  gives the original's sRGB result over black.
- `rc_menus::screens` holds what each menu's script does, rewritten by
  hand (the main menu and the yes/no question so far).
- `rc_video` (no Bevy) plays the original's AVI videos: `avi::parse`
  finds the size, rate and frame chunks; `msmpeg4v2::Decoder` decodes
  Microsoft MPEG-4 v2 with the oxideav crates' tables and block decoding
  plus our own picture loop; `VideoPlayer` steps, loops and gives YUV or
  RGBA. `rc_game::menu_video` (resource `MenuVideo`, system `play`) opens
  the video a menu's `BackgroundMovie` widget names
  (`rc_menus::layout::MovieWidget`) and keeps a Bevy image on the frame
  for the real time since it started; `original_menu::draw` places it
  under the menu, cut to the screen's shape by `MenuVideo::crop`.
  Decoding is on the CPU: about 5 ms a 512x512 frame in a release build,
  so 30 frames a second cost little. Two guards keep a slow computer from
  dragging the game down: the decoder and the oxideav crates are built
  optimised even in debug builds (`[profile.dev.package.*]` in the root
  `Cargo.toml`; unoptimised they took about 70 ms a frame), and `play`
  decodes at most `MAX_DECODES_PER_UPDATE` frames per game frame. When
  the clock is further ahead, the video's start moves later, so it shows
  every picture in order but runs late, instead of each game frame
  decoding more catch-up frames than the last (logged once,
  `MenuVideo::frames_late` counts them).
- `rc_menus::pages` handles the pages behind the main menu (pause,
  campaign, options) as data. `data/ui/original_menus.toml` names each
  page's menu class and maps each button's function name (the original's
  `OnSelect`) to one of a few commands (`resume`, `restart`,
  `open:<page>`, `back`, `quit_to_menu`, `quit_game`, `play:<map>`,
  `unavailable`). `pages::check` lets a page use its original menu only
  if every widget is a kind we draw and every button has a command;
  otherwise the placeholder page stays and every reason is logged. So a
  mod can switch a page to another class, or teach it a button, without
  code, and an unfinished page never shows half-working.
- The GRAPHICS page follows its own script, `CTGraphicsOptionsPCMenu` in
  `XInterfaceCTMenus.u`, read from the install. Its presets aren't in the
  menu: it names five `System.ini` sections (`GraphicsQualityLowest` to
  `GraphicsQualityHighest`), and `rc_game/src/graphics_presets.rs` reads
  them at start from `GameData/System/System.ini` with
  `GameData/Save/System.ini` on top (only its lines that aren't commented
  out), plus `[VideoSettings] DefaultConfig`, the preset SET RECOMMENDED
  uses. GRAPHICS QUALITY isn't stored: as the original's
  `MatchSettingsToPreset`, it shows the first preset every row matches, or
  CUSTOM. The arrows never stop on CUSTOM (GRAPHICS QUALITY goes to
  LOWEST; TEXTURE QUALITY to HIGHEST going left, LOWEST going right).
  PROJECTORS off turns SQUAD SHADOWS off and greys it out; VSYNC off sets
  FSAA to NONE and greys it out (`option_settings::row_enabled`,
  `screens::disable_row` / `enable_row`). BRIGHTNESS is the original's
  `Gamma`: row = int((Gamma - 0.5) / 2 x 10 + 0.5), set as row / 10 x 2 + 0.5
  (`Engine.dll`); its screen curve is x ^ (1 / Gamma) (`D3DDrv.dll`), which
  the port matches at mid grey with Bevy's exposure on the 3D view. FSAA's
  list is NONE plus the sample counts the graphics card supports for
  every format the cameras use. Presets leave the screen size alone
  (`PRESETS_SET_SCREEN_SIZE`), since the original's hold only 4:3 sizes up
  to 1024x768. The page opens with nothing lit, as in the user's
  screenshots of the original.
- The options pages (GAME, SOUND, GRAPHICS, CONTROLS) are original too.
  `rc_menus::layout` draws the original's choosers (`MenuButtonEnum`: a
  row with a list of values and arrows) and its picture buttons
  (`MenuButtonSprite`; the arrows are mouse only, as in the original), and
  fills fixed-size arrays to their full length. In
  `data/ui/original_menus.toml` each page's `choosers` table names the
  setting behind each row (`rc_game/src/option_settings.rs` turns a
  setting into "which value" and back), `keys` names the action behind
  each key row of CONTROLS, and `extra_buttons` adds a copy of an original
  button (here the "PORT +" tab). The port's own settings page
  (`more_settings`: FOV, Panini, full screen, master volume, controller
  look and gyro) borrows the original GAME page with `texts`, `moves` and
  `hidden_rows`. The screen size opens a drop-down list made from the
  page's own pieces (`screens::drop_down`); other values change only with
  their arrows. The size list holds only sizes that fit the primary monitor
  (`option_settings::items`, monitor in `display::Desktop`). On screens
  wider than 4:3 `rc_menus::fit::spread` moves the menus apart rather than
  shrinking anything (`docs/ORIGINAL_MENUS.md`, "Wide screens");
  `screens::fit_row_labels` shrinks row labels that would reach their row's
  first button only where that isn't enough (4:3). Changing the size in a
  level marks every camera's `Projection` changed (`display::apply_video`): Bevy only re-reads a
  camera's target size on a resize event or a projection change, so before
  this the player camera resized while the arms and sky cameras kept the old
  size, and the GPU refused to draw ("Attachments have differing sizes").
  A row naming `unavailable` is greyed
  out and skipped, as the original greys options it can't offer. Key rows
  open the original's REASSIGN BUTTON box (`CTMapKeyMenu`), which uses
  `rebind.rs` for the actual capture. The effects volume now scales every
  sound that isn't music (`music.rs`, `effects_volume`).
- The campaign page is original too (`CTCampaignMenu`). `rc_menus::lists`
  draws the original's scrolling list (`MenuStringList`: its template row
  copied once per visible row, spaced between the list's two corners), its
  scroll bar's handle (`MenuScrollBar`, sized by how much of the list
  shows), and its wheel area (`MenuScrollArea`). `rc_menus::campaign` is
  the page's script rewritten by hand: the three planets with their rings
  and mission blips, dimming the others, the planet name, and the missions
  read from the class's own `GEO_SubMissions`/`GEO_SubMissionDescs` (and
  `RAS_`, `YYY_`). `rc_game/src/original_lists.rs` does the keys (up and
  down along the list, left and right change planet), the wheel, double
  clicks and SELECT. New page commands: `scroll_up`, `scroll_down`,
  `list_pick` and `act:<name>` (page-specific actions). An "EVERY MAP +"
  button (an `extra_buttons` copy of BACK, moved right with `move_right`)
  swaps the list for the class's developer list of every map. List rows
  that are too long are squeezed narrower (`Text::squeeze`) rather than cut.
  `quads::text_overlaps` finds text touching other text or picture buttons;
  `rc_probe --pages` prints every overlap per page.
- The load and save pages are original too (`CTLoadGameMenu`,
  `CTSaveGameMenu`), using the same list. `rc_game/src/original_saves.rs`
  fills them from `SaveList` and turns their buttons (`act:load_save`,
  `delete_save`, `new_save`, `overwrite_save`, and the pause menu's
  `quick_save`, `quick_load`, `load_last`) into `SaveGame`, `LoadGame` and
  the new `DeleteSave` messages, asking first with the original question
  box where the original does (`OriginalMenus::ask`, `Asking`). A page
  button that opens a load or save page that can't be drawn is greyed out.
- The PROFILES pages are the original's too (`MenuSelectProfile`,
  `MenuPCNewProfileName`, `MenuLoadingProfile`, `MenuWarning` in
  `XInterfaceCommon.u`). `rc_game/src/original_profiles.rs` follows their
  scripts: the list (`< NEW >` then the profile folders, capitalised),
  must-pick-one when there is no profile (BACK hidden and ignored), DELETE
  hidden on `< NEW >`, the name checks and messages, the one-second
  "CREATING/LOADING PROFILE" box. `rc_save::profiles` keeps each profile as
  a `Profile_<name>` folder in the save folder with its saves inside; the
  profile in use is `settings.save.profile`, and `SaveFolder` points into
  its folder. Typing boxes are `rc_menus::edit::EditBox` (the original's
  `MenuEditBox`): its filter, length limit, backspace and blinking cursor
  follow `XInterface.dll`'s native code, read by disassembly; the cursor is
  drawn from the layout's `WhiteTexture`. `OriginalMenus::show_box` shows a
  warning or wait box with the question box's machinery (`Asking::Warning`,
  `Asking::Wait`).
- The pause menu's rows follow the original's own rules each time it
  opens (`rc_menus::pause`, rewritten by hand from the script in
  `XInterfaceCTMenus.u`): which of the eleven rows show depends on health,
  being down, a briefing (`Cutscene.held`), the prologue, the quicksave
  and whether hints can be switched back on (`settings.game.hint_screens`
  off and the map's LevelInfo `bLevelContainsHints`, read by
  `rc_game/src/original_pause.rs` when the level starts). The rows that
  show are stacked one step apart (the step between the first two rows),
  centred on the screen. The menu starts from the layout as first read on
  each opening (`PageScreen.base`), as the original makes a new menu each
  time; coming back from its own pages keeps the rows, except that closing
  the load or save page hides LOAD LAST SAVE or QUICK LOAD when their saves
  are gone. `rc_probe --menu XInterfaceCTMenus.CTPausePCMenu` paints each
  case.
- `rc_game/src/original_hud.rs` swaps the placeholder HUD's drawn shapes
  for the original's pictures, named in `data/ui/hud.toml` (the original
  places them in native code, so placement is ours). It reads its own
  `rc_menus::Library`, so it works when `--map` skips the menus. So far
  only the crosshair; the bars stay whenever a picture can't be read.

The menu area is the largest centred 4:3 box in the window; the original's
positions assume 4:3, so nothing is stretched on wide or tall screens.
Focus moves with the menu actions (`MenuUp`, `MenuDown`, `MenuAccept`,
`MenuBack`), so gamepads and keyboards work the same; the mouse moves focus
only when it actually moves. Details and what is left are in
`docs/ORIGINAL_MENUS.md`.

**Screen shapes and controls.** The view is "Hor+": `video.fov` is the
angle across a 4:3 screen, as in the original, and wider screens see more
at the sides (upright phones see more above and below). The long side stops
widening past the screen shape `video.widest_shape` (2.0) so wide windows
don't get a fish-eye look (`player::view_angle`). Unknown options in
`settings.toml` are logged as a warning and skipped. Keyboard, mouse, gamepads and touch all work at once. Details,
numbers and what is still to do (edge stretching on ultrawides, controller
pictures, gyro) are in `docs/DISPLAY_AND_INPUT.md`.

**On-screen display.** Corner brackets like the commando's visor, a
four-bar crosshair whose gap follows the weapon's spread and turns red on a
target, shield and health bars, the ammo counter, a squad list and short
messages. It reads only the readout resources, and logs its numbers every
two seconds.

**Music.** The original's music system ("iMuse") is rebuilt from what the
game's own files say, with nothing hard-coded per level:

- `rc_music::library` reads every piece of music from the `*_music.uax`
  packages (`SoundStreamed`: file, looping, fade-in and fade-out time and
  shape, loudness from `params_mus.uax`, likelihood) and every group
  (`SoundMultiple`: members picked at random by likelihood, an optional
  first piece, and for battle groups the "won" flourish and the "lost"
  music). Room music comes from the `AudioEnvironment` classes in
  `Properties/audioenv.u` (`iMuseState`, `iMuseOverlays`, overlay start
  delay and spacing, once only).
- `rc_level::music` reads what a map says: the `LevelInfo` battle settings
  (battle music, how many enemies, inner and outer radius, criteria) and
  every volume with an `AudioClass`.
- `rc_music::imuse` is the director, without Bevy. Inputs: script music
  actions (`SetImuseState`, `PlayMusic`, `EnableBattleEngine`,
  `SetBattleEngineParams`, `SetBattleMusicMulti`, `ForceBattleMusic`,
  `ForceVictoryFlourish`), rooms entered, where the enemies are and whether
  they are fighting, and whether the player is down. Output: play and stop
  commands for three layers (main, overlay, episode).
- `rc_game`'s `music.rs` carries the commands out with Bevy's audio and
  fades. Menus play what their menu class says (`BackgroundMusic`: the main
  menu's sung theme; the loading and pause menus have none, so the level's
  music pauses).
- Script actions reach it as `rc_game_core::MusicCue` messages; the enemies
  part lists its enemies in `MusicThreats`.
- `data/music/music.toml` holds only our own timings (how often the battle
  check runs, how long after the last enemy the fight ends, the
  player-down music) and an optional menu override.

Our reading of the battle rules (the original does this in compiled code,
so these are guesses checked against the settings' names and defaults): a
fight starts when the number of counted enemies inside the inner radius
reaches the threshold; it ends when no counted enemy has been inside the
outer radius for 3 seconds, then the "won" flourish plays and the room's
music comes back. "Proximity" counts any living enemy, "Attack" counts
fighting enemies anywhere, "Attack and proximity" counts fighting enemies
in range. A state set by a script during a fight waits until the fight
ends. The fade curves (linear, sine, log, inverse log) are our guess at
the shapes. Not done: `BattleVolume`, the death stinger, debriefing,
credits and title-card music, music in save games.

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

## The helmet frame (`rc_menus::helmet`, `rc_game/src/helmet.rs`)

**What the original does.** The HUD class `CTGame.CTHUD` has
`StaticMesh = Helmet` and `DisplayFOV = 84`: the frame round the view is a
3D model, `HUDMeshes.HelmetSurface.Helmet` in
`StaticMeshes/hudmeshes.usx`, drawn at the eye with its own 84-degree view
(degrees across a 4:3 screen). It has two parts: the metal
(`HUDTextures.Helmet.HelmetMetal_Shader`, picture `Helmet_HUD`, 1,008
triangles) and a glass sheet (`HelmetFinalBlend`, whose shader names no
picture, so there is nothing of it to draw). Its points lie 9.6 to 14.9
Unreal units in front of the eye. The options menu's HELMET row sets the
player's `bVisor`. All read from the install with `rc_probe --class
CTHUD`, `rc_probe --meshes` and `rc_probe --helmet`; nothing is copied.

**Fitting every screen shape.** The model was made for 4:3. Seen from the
eye, a point's place across the screen is `y / x`. On a wider screen,
points further from the middle than `sides_from` (half of a 4:3 screen's
half width) move out by exactly the extra width, keeping their shape;
points nearer the middle (the bars across the top) spread evenly so
nothing tears. Up and down never change. Narrower screens (5:4) move the
sides in the same way. `rc_menus::helmet::check_shapes` paints the frame in
software at 14 landscape shapes (5:4 to 32:9, phones, flip and fold
phones, the user's 3440x1369 window) and checks that the sides still
reach the screen edges as at 4:3, the top bar still spans the screen, the
bottom corner pieces keep their size in screen heights, and the middle
stays open.

**Drawing.** `HelmetPlugin` reads the model once when a level first shows,
then gives the player's camera a child camera (order 2, render layer 3,
its own depth) that sees only the helmet, so it draws over the world and
the first-person arms (order 1) and under the menus and display (now order
3). Unlit at `brightness` (0.5, a guess) from `data/ui/helmet.toml`. The
mesh is refitted when the window changes shape. Settings `[game] helmet`
and the plan's `enabled` show or hide it; `HelmetReadout` says what is on
screen for tests and mods. For VR later, the helmet is its own camera and
model, so it can be put in front of each eye instead.

**Checks.** `rc_probe --helmet` (software, every shape);
`cargo run -p rc_game --example helmet_check` (the graphics card's
picture against the software painting at six shapes, and the setting
off and on); the game script's "the original helmet frame is drawn round
the view" (95% of its pixels match in geo_01a) and "HELMET OFF takes the
frame away".

**Not done.** The original's lighting of the frame (lights inside the
helmet, `AmbientGlow` 20, its bump map); the visor's effects that the
`Helmet` class also runs (blood, cracks, wipe, rain); the frame shows a few
seconds late on the cloud's software graphics while its shaders are
prepared.

**The HUD through the glass.** The model's other part (`HelmetFinalBlend`,
24 triangles) has no picture of its own: the original (Engine.dll,
`FPlayerSceneNode::Render` and `UHelmet::DrawHelmet`) draws the whole HUD
into a screen-sized picture and then draws this glass with that picture
on it. Each glass corner's texture place is a place in the HUD, but the
corner sits a little lower on the screen, so the original's HUD comes out
lower and slightly curved: text drawn 45 down on its 600-line HUD shows
about 1/7 of the way down. `helmet::GlassMap` works this move out from the
model for any screen shape (on wide screens the glass moves like the
frame and each corner keeps its move measured in screen heights) and
`rc_menus::hud` applies it to elements marked `glass = true`, moving each
picture's corners (the pictures themselves aren't bent). The crosshair and
the markers are drawn after the glass in the original, so they aren't
moved. Without the helmet (HELMET off) nothing is moved; the original
then bends its flat HUD a different way, which we don't copy yet.

**The advisor's words and the objective** (Engine.dll
`UHelmet::DrawTextInfo`): one text box near the top (218, 45, 364 wide
on the 800 x 600 HUD) in `OrbitFonts.OrbitBold15`. The words of whoever
speaks (`subtitles.rs` keeps the newest in `Spoken`) are left aligned at
0.8 of the font's size in (200, 200, 64) and win over everything else in
the box. Otherwise the current objective is centred at full size in
(164, 200, 208), when no short message or use prompt is in the box. Both
are at opacity 220, fading in over the helmet's `PromptFadeInTime`
(0.5 s) when they change. The objectives themselves are the map's
`MissionObjectives` list, kept as the original's script keeps them
(`rc_scripts_plugin::objectives`): its tag event reveals the next one,
`ACTION_ObjectiveUpdate` completes, fails or changes one, and the first
one not yet done shows, in capitals. Not done: Ctrl (`ShowGameStats`)
turning the objective off and on, objectives in other languages (the
words are the map's English ones).

**The bottom corners and meters** (Engine.dll `UHelmet::DrawHUD`, the
helmet-on drawing; `data/ui/hud.toml` has every number, times 0.8 of the
800 x 600 page, all `glass = true`):

- *Squad*, bottom left: one panel per squad mate who is in the squad
  (`SquadMember::in_squad`; mates away or not yet arrived aren't shown),
  40 apart. Their number above it in `OrbitBold15`, (164, 200, 208) at
  220; the panel picture (148, 354 to 192, 396 in `CloneVisorIcons`) at
  240; the order picture inside, tinted by how hurt they are.
- *Health meters* either side of the middle, mirrored: a white back at
  130, the top piece and a bar in the health colour. The bar drops in
  elevenths, and by at least 5/55 while hurt at all (`fill_steps`,
  `fill_least_gap`).
- *How hurt* is the original's `APawn::CalcHealthLevel`: 0 at no health,
  1 below half, 2 below 0.91, 3 above (`rc_menus::hud::health_level`, texts
  `{health_level}` and `{squad_health_level}`, used with `colour_by` and
  `colours`). Player meter colours: red (255, 94, 72), orange (246, 139,
  31), yellow (255, 250, 110), green (72, 255, 72); squad pictures: (255,
  73, 43), (246, 139, 31), (255, 251, 44), (81, 213, 67), at 200.
- *Grenades*, bottom right: the panel mirrored, the grenade's picture by
  its class, the count centred above, (200, 240, 255) at 192, red (255,
  128, 128) when none are left.
- *No ammo numbers*: the gun's own counter shows them. The original writes
  "clip / spare" only zoomed in with the sniper rifle or bowcaster, and in
  split screen.
- Through the glass, a point past the glass's edge moves as the nearest
  piece of glass carries it on (`GlassMap::place`), so pictures lying
  across the edge keep their shape. (The game itself now uses the page
  below; `GlassMap::place` still says where things land, for checks and
  `rc_probe --hud`.)

**The visor frame and compass** (Engine.dll `UHelmet::DrawHUD`): the arcs
down both sides (5, 22, 112 x 432 from `CloneVisorIcons` 0, 0 to 142,
442, mirrored at 795), the short pieces at the top (106, 22, 86 x 19,
mirrored at 694) and the compass strip between them (194, 25, 414 x 11,
`CompassGraphic` repeated). White at 130 while health is high, else the
health colour (165 at level 2, 150 below; the original pulses that, not
done). The compass slides with the way you face: the picture's offset is
the yaw (0..65535) times 0.0001, followed at 3 per second
(`original_hud::compass_follow`); `scroll = "compass"` on an element
slides its picture, which repeats (`rc_menus::hud` `tiles`). Group
`visor` may overlap the other groups.

**The HUD page on the glass** (`helmet::HudPage`): as the original does,
the HUD's `glass = true` parts are drawn on a 4:3 picture (1600 x 1200,
its own 2D camera, order -10), and that picture is laid on the helmet
model's glass (the part without a picture of its own) as an unlit,
premultiplied see-through material. So the HUD bends with the glass and
what lies past the glass's edge isn't seen. The parts not on the glass
(crosshair, pickup line) are drawn on the screen as before.

Not done yet: the pulsing (squad mates down, no grenades, low health),
the arrow round each squad panel pointing to that mate, the timer ring
while a mate works on something, which order uses which picture (ours),
the shield bars.

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
(checkpoints reached on this map; `rc_save`), `[movers]` (every door and
lift; `rc_movers_plugin/src/save.rs`, using `rc_movers::MoverSet::snapshot`
and `restore`) and `[scripts]` (the level scripts;
`rc_scripts_plugin/src/save.rs`, using `rc_scripts::Director::snapshot`
and `restore`).

**Doors, lifts and scripts carry on.** `[movers]` lists each mover by its
map name (`Mover37`) with the key it is at, what it is doing (`rest`,
`delay`, `moving` from one key to the next and how far, or `wait` with the
seconds left before it closes), and whether it has been used, plus each
touch trigger's state. Its position isn't written: it follows from the key
and the move. `[scripts]` holds the director's clock and random-number
state, each script's next action and what it waits for (an event, or a time
on the scripts' clock), each trigger, counter, condition and prop, and
events not handed out yet; and what the scripts changed in the game: level
parts no longer solid or hidden (by map export number), bacta dispensers,
objectives, and checkpoints waiting for the player to get up. Everything is
matched by the map's own actor names, so a name the map hasn't got is
logged and skipped. Loading `[scripts]` marks the level as started, so the
map's level-start events (`levelstart`) are not sent again, and while
`RestoringSave` exists the scripts don't run at all. An older save without
these sections loads as before: doors shut and scripts from the start.

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

## Going on to the next map (`rc_scripts`, `rc_save`, `rc_level_plugin`)

**How the original's maps end.** We read every campaign map with
`rc_script_check`. Three ways, all from the map itself:

- Most maps (37 of 56) have a `Teleporter` actor with a `URL`, like
  `GEO_01B.ctm#playerstart`: walking into it (the arrows; geo_01a's hint
  says "Walking through the arrows will load the next level") loads that
  map. Many are closed at first (`bCollideActors=false`) until the
  mission's last script runs `ACTION_SetCollision` on their tag; geo_04c's
  starts switched off (`bEnabled=false`) and an event with its tag
  switches it on, as UE2 teleporters do. Their size is the map's
  `CollisionRadius`/`CollisionHeight`, or the game's own defaults (80 and
  100, from engine.u's NavigationPoint; `data/scripts/scripts.toml`
  `[exits]`).
- Briefings, cutscenes and a few levels end with a script action,
  `ACTION_ChangeLevel` (`URL`).
- The last level of each campaign shows the debriefing,
  `ACTION_ShowDebriefingScreen` with a `NewLevel` (the same action with
  only `Text` is a hint screen, and doesn't leave). The game shows the
  screen first and goes on to `NewLevel` at CONTINUE (see "The debriefing
  screen" below).

**The debriefing screen.** geo_05c, ras_04d and yyy_06d each end with
`ACTION_ShowDebriefingScreen`: a menu class (`CTDebriefingInfoGEO`, `RAS`
or `YYY`), a `Title`, a `Text`, a picture (`PicName`) and `NewLevel`. The
director puts what the screen shows on its `Effect::ChangeLevel`
(`debriefing`), and the scripts part sends
`rc_game_core::ShowDebriefing` with the `ChangeLevel` held inside instead
of sending the `ChangeLevel` itself. In `rc_game`, `debriefing.rs` pauses
play and opens the `debriefing` page (`[pages.debriefing]` in
`data/ui/original_menus.toml`, class `CTDebriefingInfo`).
`rc_menus::debriefing` fills it in as the original's `SetInfoOptions` and
`Init` do: title and text into `InfoTitle`/`InfoText`, the picture into
`InfoPic` (whole picture), CONTINUE (`AButton`) shown on a PC, "YOU HAVE
UNLOCKED / EXTRAS CONTENT" shown (English). Two maps name the picture
without its group (`GUIContent.GEO_Debrief` is really
`GUIContent.LoadInfoScreens.Geo_Debrief`), so `Library::resolve_loose`
finds a picture by package and name when the full path doesn't match.
CONTINUE (`act:continue`) sets `Menu::go_on`; `debriefing.rs` then sends
the held `ChangeLevel` and the save part travels as for any exit. Back
does nothing on this page (`back = "unavailable"`), as in the original. Our wrapped text came out
longer than the frame, and CONTINUE wider than its bar, so
`debriefing::fit_continue` widens the bar and `debriefing::fit_text`
makes the text smaller in steps (not below 60%) until it fits. The real
texts end up at 85-90% of their size. If
the original page can't be drawn, a placeholder page marked as one shows
the title and text with Continue. Not done: the GEO/RAS/YYY classes differ
only in the music they play (`musGEO_cadence_lp` and so on); that music
isn't played yet. The same action without a `NewLevel` is a hint screen
(see "The hint screens" below).

**The hint screens.** 29 script steps in geo_01a, geo_01b, geo_01c, geo_03a
and geo_04d run `ACTION_ShowDebriefingScreen` with only a `Text`,
`MenuClass=XInterfaceCTMenus.CTPauseHintText` and `bIsForHintText=true`
(the tutorial pop-ups: crouching, the bacta dispenser, the objective
tracker, the squad, the sniper and anti-armour attachments...). What the
original does with them is in its own script in the install:
`ACTION_ShowDebriefingScreen.InitActionFor` (CTGame.u) shows the page only
if the step isn't for hint text or `bKeepHintMenusAwfulHack` is on (a
`globalconfig` of `PlayerController`, `true` in the install's `User.ini`:
despite its name it is simply "hints on"), and the script goes straight on.
`PlayerController.ShowMenu` (Engine.u) pauses and opens the page with the
text. The scripts part sends `rc_game_core::ShowHint`; `rc_game/src/hints.rs`
checks our `hint_screens` setting, pauses and opens the `hint` page
(`[pages.hint]` in `data/ui/original_menus.toml`). `rc_menus::hint` fills
the page in as the class's `Init` and `SetInfoOptions` do: the text in
`InfoText`, the title and picture hidden, the empty picture frame shown,
CONTINUE WITH HINTS shown and focused, and with the helmet off the frames
and text moved up to the class's numbers. CONTINUE WITH HINTS
(`act:hint_on`) goes on playing; CONTINUE WITHOUT HINTS (`act:hint_off`)
switches `hint_screens` off, saves the settings and goes on; back does
nothing, as `CTInfoBase.HandleInputBack` is empty. The pause menu's
ENABLE HINTS switches them back on. If the page can't be drawn, a
placeholder page (marked) shows the text with the same two choices.
With a gamepad, B is CONTINUE WITHOUT HINTS and A is CONTINUE WITH HINTS
wherever the focus is: the original's console (`KeyEvent` in Engine.u)
hands `IK_GameA`/`IK_GameB` to the menu's `HandleInputGamePad` before
anything else, the hint classes take them and return true, and on a PC
WinDrv.dll sends DirectInput joystick button n as key 0xC8 + n (checked
in its machine code; 0xC8 is `IK_GameA`, so button 1 is `IK_GameB`). A
hint asked for while a page shows takes that page's place, as in the
original: `PlayerController.ShowMenu` only skips `SetPause` when play is
already paused and still opens the page, and the console's `MenuOpen`
(Engine.u) destroys the page that was showing. Level scripts don't run
while paused, so this can only happen within one frame (none of the 29
does it); a debriefing it replaces doesn't go on to its next map. The HUD (prompt text, objective tracker, meters) stays drawn under the
hint box, as in the original: Engine.dll's `FPlayerSceneNode::Render`
calls `UHelmet::DrawHUD` whenever the level is drawn, skipping it only
for split screen or the console's `NOHUD` debug command (checked in its
machine code; nothing kept). Every widget's place, size, font, wrap width
(`InfoText.MaxSizeX` 0.38) and shown/hidden state is the class's own
default, changed only by its `Init` and `SetInfoOptions`. Not
done: the other languages' moves (the port reads only English).

Following the first exit of each map from `pro` visits all 56 campaign
maps in order and ends at `epilogue`; no exit names a missing map.

**Script actions switched off in the map.** 371 actions in the 57 campaign maps carry
`bDisabled=true` (designers switched them off: debug messages, an
`ACTION_SetCollision EndPort` at the start of ras_01c that would open the
exit at once). The director skips them, logged as "switched off in the
map, skipped". This is our reading of the flag's name; the class that
uses it is compiled.

**Leaving.** `rc_scripts::Director` sends `Effect::ChangeLevel { url }`
once and then stops running the map's scripts. `rc_scripts::level_url`
reads the address into a map name (lower case, no `.ctm`) and a portal
(after `#`). The scripts part sends `rc_game_core::ChangeLevel { map,
portal, by }`.

**Carrying the player over.** `rc_save` handles `ChangeLevel` like a load
of a save, but with only some sections: the ones `data/levels/travel.toml`
lists (`carry = ["combat", "squad_carry"]`) are read out of the game with
each part's own `SavePart::save`, the map is loaded fresh (`CurrentLevel`,
`GameState::Loading`), and on its first frame each carried section goes
back in with that part's `SavePart::load`, before the level-start
checkpoint is written, so that checkpoint has them. While those sections
are written, the resource `rc_game_core::save::CarryingOver` exists, so a
part can carry something different from what a save file keeps. Not
carried: the player's and squad's places (they arrive), doors, scripts and
enemies. A mod can change the list.

**Carrying the squad over** (`rc_squad_plugin::carry`, section
`[squad_carry]`). What the original keeps, read from the clone player's
`SaveSquadTravelInfo` and `TravelPostAccept` in CTCharacters.u: each squad
mate's health, matched by class (we match by name), and the squad's last
order. A mate who is down when the player leaves gets up on the next map
with a quarter of their full health (`DOWN_HEALTH_SHARE`). The order is
kept except "secure position", whose spot was on the old map. Shields come
back full (the original doesn't carry them either). Squad mates' weapons
and ammo aren't carried: they have no ammo limit. The section is only
written while `CarryingOver` exists, or while a carried section is still
waiting for the squad to appear (so a save made in between keeps it);
ordinary saves keep the full `[squad]` section instead. A mate on the new
map who wasn't carried starts at full health; one carried but missing is
dropped (the original keeps their old entry for a later map; we don't).
Checked by `rc_squad_plugin`'s `carry_over` test, through the real save
plugin's level exit.

**Arriving.** `rc_save` inserts `ArrivalPortal(portal)`. The level part
looks for a `Teleporter` with that `Tag` in the new map
(`MapCollision::arrivals`) and starts the player there, as UE2 does;
otherwise (`#playerstart`, and 12 of the original's addresses like
`Geo_04B.ctm#FromA`, whose map has no teleporter by that name) at the map's
first `PlayerStart`. `rc_script_check` lists which is which: 5 exits
arrive at a teleporter, 38 at the start.

**Checked by** `rc_scripts` tests (an exit a script opens; one switched on
by its tag; `ChangeLevel` and the debriefing; disabled actions; an opened
exit through a save; address reading), `rc_save`'s
`level_exit_carries_sections_to_the_next_map`, and `rc_game --script`,
which puts the player into geo_01a's exit with 13 health taken off and
checks that geo_01b loads with the same health and magazine, the player at
its start, and its start saved as a checkpoint with that health.

## Cutscenes (`rc_scripts` Dispatchers, `rc_scripts_plugin/src/cutscene.rs`, `rc_game/src/cutscene.rs`)

**What the original's cutscenes are.** Republic Commando has almost no
films. Its story scenes are the game's own world seen through the player's
eyes, run by the same level scripts as the missions: the prologue (`pro`,
the clone growing up on Kamino), the briefings on the gunship
(`geo_01briefing`, `ras_01briefing`, `yyy_01briefing`, `yyy07briefing`),
the `epilogue`, and short moments inside missions (geo_01a opens with the
player held in the gunship while the squad rappels down). The only video
files are the start-up logos (`Movies/lecintro.avi`, `nvidiaintro.avi`),
the main menu's background (`menu_bg.avi`) and the EXTRAS menu's
making-of films (`extras*.mpg`). The `.avi` files are MPEG-4 part 2
(`MP42`, `MPG4` four-character codes), which no Rust library decodes yet.

**Dispatchers.** Cutscene maps are timed by `Dispatcher` actors: when an
event with the dispatcher's tag is sent, it sends a list of events one
after another, each after its own `Delay` (a random time up to `DelayMax`
when that is longer), optionally playing a `Sound` first (and waiting for
it with `bWaitForSound`). Its `Events` list is an array of structs written
as named values; `rc_level::scripts` now reads such arrays into
`Val::Structs`. Before this, nothing ran them, so the prologue and the
briefings stopped at their first dispatcher and the player was stuck.
`rc_scripts::dispatcher_actions` turns each dispatcher into a small script
of our own making (wait for the tag; for each entry wait, play, send; go
back to waiting), so its waits are saved like any script's and its tag is
not heard while it runs, as UE2's Dispatcher ignores triggers while
dispatching. `rc_script_check pro geo_01briefing epilogue --secs 400`
shows each map now runs to its end: the prologue leaves for
geo_01briefing at 166 s, the briefing for geo_01a at 53 s, the epilogue
reaches its credits (`ShowMenuScreen CTCreditsMenu`) at 75 s.

**What the scripts ask of the player and the screen.** Shared in
`rc_game_core::cutscene`: the `Cutscene` resource (held view, frozen body,
screen tint, HUD and helmet frame hidden) and the `TeleportPlayer`
message. `rc_scripts_plugin/src/cutscene.rs` (`Scene`) fills them in from
the actions:

- `ACTION_Briefing` holds the player: no walking, jumping, crouching,
  fighting, using or orders (`ActionState::hold_still`); looking stays,
  within `MinYaw`..`MaxYaw` and `MinPitch`..`MaxPitch`. The limits are
  the world's own angles, not angles from where the player faced: in the
  prologue each room's range is centred on the way that room's arrival
  point faces (GymTeleporter faces 32640, its range is 28500..38000).
  Class defaults (read with `rc_probe --class ACTION_Briefing`): the whole
  circle both ways, `bDrawHUD` on. `bDrawHUD` off hides the HUD;
  `bEndBriefing` lets go; `bOkToChangeWeapon` keeps weapon changes.
- `ACTION_TeleportToPoint` on the player moves the middle of the body to
  the named point, facing the way it faces, with no speed.
- `ACTION_SetPhysics` on the player: mode 0 (none) freezes the body where
  it is (the newborn clone doesn't fall); any other mode lets it move.
- `ACTION_ScreenFade`: a positive `FadeTime` fades to `FadeColor` (default
  black, 3 s), 0 goes there at once, a negative one fades back from it.
- `ACTION_AnimateVisorFX`: `AddColor` shown as that colour over the
  screen, `MultiplyColor` as black over it as strong as the colour is
  dark, rising over `InTime`, holding `SustainTime`, falling over
  `OutTime`. This is a stand-in: the original adds and multiplies, we lay
  a colour over. Bloom is logged as not done.
- `ACTION_ShowVisor` hides or shows the helmet frame (`bVisor`).
- `ACTION_Briefing` with `bSetBriefingWeapon` gives the player other
  hands (`BriefingWeapon`: the prologue's `CTInventory.Baby` and
  `CTInventory.Boy`). Those hands aren't drawn yet, so the grown
  commando's arms and gun are hidden while it is set
  (`Cutscene::briefing_weapon`, read by `rc_combat`'s `arms_view` and
  `view`); `bEndBriefing` gives them back.

**No squad in cutscene maps.** The playable missions each have a `CTSquad`
actor; the prologue, the briefings and the epilogue don't. Our squad used
to come along everywhere (the user saw Delta standing behind a tank in the
prologue). `rc_squad::map::MapSquadInfo::wants_squad` is true when the map
has a `CTSquad` actor, a squad marker or a squad-mate spawner; when the
map was read and none is there, `rc_squad_plugin` brings in no squad mates
and logs "this map has no squad".

`rc_game/src/cutscene.rs` carries them out: it holds the actions between
`GameSet::Input` and `GameSet::Simulate`, hides every `HudRoot` (the
placeholder HUD and the original HUD pictures), and draws a full-screen
colour layer over the HUD and under the menus (`GlobalZIndex(9)`).
`player.rs` applies teleports, the look limits and the frozen body;
`helmet.rs` hides the frame. Everything is cleared when a level loads or
the game is left. A VR mod reads the same `Cutscene` and shows the tint
and limits its own way; nothing here assumes a flat screen.

**Our reading, not the game's code.** What each action does comes from
its name, its values in the maps and its class defaults; the classes
themselves are compiled code we don't read. Not done yet: the player
riding the gunship (`ACTION_AttachToGunship` to a bone of the
`BriefingGunship`, whose `GEO01fly` animation carries the camera through
the sky), drawing what the player holds (`BriefingWeapon`: the baby's and
boy's hands, the briefing helmet; for now nothing is drawn), characters brought in by factories during
a scene (placed ones are the cast, see `rc_cast` below), hiding and
showing actors that aren't level parts (the hologram text), view shakes,
the title card, the credits, and the cutscene state in save files (the
cutscene maps have no checkpoints, so only a quick save mid-scene would
lose the hold).

## The cast: characters no other part owns (`rc_cast`)

**Why.** The first playtest of the prologue showed an empty Kamino: the
level scripts ran, but the characters they talk to didn't exist, so the
log had 158 "no character ... in the game" lines. The droids belong to
the enemies part and Delta to the squad part; everything else a map
places was never spawned.

**Who is in the cast.** When a map loads, `rc_cast::read::find_cast`
looks at every placed actor whose class comes from `Pawn` (characters
and vehicles) or `AnimProp` (animated props, like the young clones
sitting in the nursery). Class parents come from the game's script
packages (`GameData/System/*.u` and `GameData/Properties/*.u`, read with
`rc_package::class_library`). Left out:

- classes `data/cast/cast.toml` leaves to other parts (`leave_out`: the
  droids, Geonosians, Trandoshans and turrets the enemies part fights);
- in a map with a squad (a `CTSquad` actor), commandos that can join the
  squad (`bCanBeSquadMember`, the squad part's). In the prologue every
  commando is cast.

Each wears the map's `Mesh` value or its class's default `Mesh`, found by
name among the skeletal meshes of `GameData/Animations/*.ukx`, and read
with `rc_model` with every animation set kept (`ModelSpec::keep_sets`,
`Model::named_clip`), so scripts can play any of its animations by name.
`[class.*]` in the data file can change a class's mesh or materials: the
commandos' armour colours come from there (the original swaps them in
compiled code). Characters stand with their feet `CollisionHeight` below
their `Location`; props are drawn at theirs, and so are characters whose
class turns `bAlignBottom` off (engine.u: "Actor aligned to bottom of
cylinder", true on `Pawn`; ctcharacters.u's `Gunship` sets it false), so
geo_01a's landing gunship hovers from its map place over the player
start instead of being stood on the ground. Cast members are drawn by
the level's light where they stand (`rc_game_core::LitByPlace`, the
data file's `light_scale`, 1.1 as the squad and droids), so a trooper in
shade is dark; holograms keep their own glow. A character whose class names a weapon
(`RequiredEquipment`, taken from the class's "...Defaults" class in
Properties/properties.u when that sets it, as the game does:
"Properties.DC15BlasterClone" from `CloneTrooperDefaults` on `CloneTrooper`)
holds its model: the weapon's `AttachmentClass` (engine.u's `Weapon`;
`DC17mBlasterAttachment`) gives a `StaticMesh`, read with
`rc_effects::projectile::ClassPackages::held_weapon` and drawn on the
bone `weaponAttach_R` (engine.u's `Pawn.GetWeaponBoneFor` returns it for
every weapon; `AttachToBone`), placed again each time the body is posed
(`rc_model::Model::bone_frame`). A class without a static mesh (the
gunship's chin gun) draws nothing and says so in the log. The reading,
drawing and placing are `rc_effects_plugin::held::HeldModel`, shared with
the fighters the map's spawners make (`rc_enemies`, see "Two sides").

**The Move bone isn't drawn (from engine.dll).** Many of the game's
skeletons have a first bone named `Move` that every other bone hangs
from, and animations use it to carry the whole body (geo_01a's gunship
clips hold it 757 to 850 Unreal units up). engine.dll's
`USkeletalMesh::PostLoad` marks bone 0 as the move bone when its name is
`Move`, and `USkeletalMeshInstance::ApplyAnimation`, outside the editor,
places a bone whose parent is the move bone without the move bone's own
travel (found by decompiling engine.dll for reference only; nothing is
copied into this repository). The travel is used only for actors with
`Physics` 12 (`PHYS_RootMotion`), whose place follows it
(`AActor::physRootMotion`). So the cast poses with
`rc_model::Model::posed_in_game` and `bone_frame_in_game`
(`rc_anim::globals_in_game`), unless the actor's `Physics` is 12, which
keeps the old `posed_local` way. Before this the landing gunship
(`Physics` 4, flying) was drawn 757 units too high. One effect: the
gunship's `GunshipAmbushSpiderdroid` clip, which flies it off at the end
of the level, also only turns it in place now, as the engine rule says
for a flying actor; nothing found in the files switches it to root
motion.

**What they do.** Each cast member gets `ScriptTags` (its `Tag`, which
is the class name when the map gives none, as in Unreal, and its
`AIScriptTag`) with `follows_orders` on, so the level scripts part finds
it and sends `CharacterOrder`s:

- `MoveTo`, `MoveToPlayer`: a straight line at `GroundSpeed` (walking at
  `walk_speed_ratio` of it), turning to face the way it goes, playing the
  first walk or run animation its sets have. No collision and no falling:
  cutscene walks are short and on clear floors. Arriving puts it on the
  point.
- `TurnToPlayer`, `Teleport`, `Hidden`, `Remove`.
- `PlayAnim` by name from its own sets, looping or once; `WaitForAnimEnd`
  waits for the one playing. A name its sets don't have is logged and the
  script carries on.
- `PlayPropAnim` (gameplay.u's `ACTION_PlayPropAnim`, a plain
  `ScriptedAction`): every actor whose `Tag` is `AnimPropTag` plays
  `BaseAnim` once, and the script goes straight on without waiting. Its
  most common target is `PlayerCommando`, the player's own body (the
  player pawn's tag, as ctcharacters.u's `ACTION_AttachToGunship` notes):
  the original plays it there, but the first-person view never shows that
  body (engine.u's `Pawn` sets `bOwnerNoSee`; the view is the pawn's place
  plus its eye height), so the port logs it and draws nothing. geo_01a's
  rope descent `EntranceRope600` is one of these. A tag no actor has does
  nothing, as the original's `ForEach` finds nobody (geo_01a's
  `CloneRope62` and `CloneRope40`).
- `WaitForPlayer` waits for the player to come within the distance, but
  only `wait_for_player_limit` seconds (our choice, 20): our player may
  stand where the original's wouldn't, and a stuck cutscene is worse.

Between orders a character plays the first of `idle` its sets have (an
animated prop plays its own `AnimHealthy`). Models are posed on the CPU
each frame like the squad's; hidden ones aren't posed.

**Animated props that are triggered, and events from animations.** As in
the original's `AnimProp` class, a level event named by a prop's `Tag`
triggers it: it plays its `AnimTriggered` (once or looping, as the map
says), or stops it if it is already playing (`trigger_props`). An
animation can send events itself (`AnimNotify_Trigger`: `EventName`, or
`TriggerSelf` to trigger its own prop again). `rc_model::Model::triggers`
reads them, and `animate_cast` sends each one to the level scripts as a
`RaiseEvent` when the animation passes its moment. This is how the
hologram briefings start (yyy_04b, ras_03a, geo_03a): the script holds
the player, sends `HoloProjector`, the projector plays
`ThrowHoloProjector`, about 2 s in that animation sends `StartBriefing`,
and the briefing's `Dispatcher` plays and in the end sends the event that
lets the player go. Without it nothing sent `StartBriefing` and the player
was held for good.

**Not done.** Characters brought in by `PawnFactory`s during a cutscene
(the prologue's walking young clones), riding vehicles, lighting by zone (a steady
`ambient` share of their own colour, as the squad), and saving their
places (the cutscene maps have no checkpoints). In missions, the placed
troopers, walkers and the scripted spider droid now stand and move as
their scripts say but don't fight.

**Damage and death** (since 2026-11-12). A cast character whose class has
`Health` (read with the class's "...Defaults" class laid on top, as the
game does) gets `rc_combat`'s `Health` and a `Hittable` cylinder from its
`CollisionRadius` and `CollisionHeight`, so bolts, blasts and a script's
`HurtActor` hurt it. Vehicles (`Vehicles` subclasses: the gunship,
walkers, spider droids) are left unhurt for now. When health runs out,
`rc_cast::die` does what engine.u's `Pawn.Died` and `PlayDying` do for
a character on the ground: it triggers the map's `Event` for the
character (`RaiseEvent`, geo_01a's `GunshipTrooper02Death`), stops it,
and plays its `Death` animation, holding the last frame. From then on it
ignores the scripts' orders apart from hiding and removal
(`Controller.PawnDied` destroys an AI's controller). The level's script
sequence for it carries on, as the map expects: Mikey's own sequence
kills him and then sends MikeyMegaForce, and the ramp troopers'
sequences wait for their own death event and then play a line.

geo_01a's two ramp troopers (CloneTrooper7 and 13, since 2026-11-12):
Trigger42 sends RunLZTroopers, their sequences run them to
WaitToBeBlownUp and WaitToBeBlownUp02, and Trigger5's bombardment
(ProjectileFactory11, `GEO_01AProj`, 400 damage within 512 uu) lands
next to them and kills them; their deaths send GunshipTrooper02Death and
03Death, which make the sequences play CT3R1001 and CT3R1002. Nothing
here was written for them: it comes from the map once the cast can die.
Not done: their `SuggestEnemy Rockey` and `SetScriptPriority` (the AI's
part).
The body is taken away `RagdollLifeSpan` seconds later (60 for a clone
trooper) unless the map or class sets `bSaveMyCorpse`. Not done: the
original's ragdolls (`TryToRagdoll`: always for explosions, whose
`RagdollOnDeathProbability` is 1, half the time by default, and whenever
there is no `Death` animation), the electric and fire death animations,
death sounds (`PlayDeathCue`), death effects (`DeathEffectContainerClass`)
and dropping the weapon. The rifle stays in the dead trooper's hand.

## Character shadows (`rc_shadows`)

**What the original does** (read from its install: the script text and
class defaults in `Engine.u`, `CTCharacters.u` and
`XInterfaceCTMenus.u`; nothing copied). Every `Pawn` checks each frame:
if its `bActorShadows` is set and the "Shadows" option is on (the
GRAPHICS page's SQUAD SHADOWS row, `EnableShadows`), it makes a
`ShadowProjector`: a 128x128 picture of the character's shape seen from
`Normal(vect(1,1,3))` (high above and a little to one side, the same for
every character), laid on the floors and walls around it, fading over
`MaxTraceDistance` 350 units and not drawn beyond `CullDistance` 1200
units from the camera, at full `ShadowDarkness`. Only `CloneCommando`
(Delta) sets `bActorShadows` in its class; maps set it on chosen actors.
The prologue marks 7 troopers, 3 young clones and 1 Kaminoan (Kaminoan2,
in the gym); the two Kaminoans in the opening nursery are not marked, so
they cast no shadow in the original either. `AnimProp`s carry the flag in
the prologue but aren't pawns, so they never make a projector.

**What we do.** `rc_cast` reads `bActorShadows` (the map's value, else
the class's) and puts `rc_shadows::CastsShadow` on each marked character.
`ShadowsPlugin` gives every mesh under it a "shadow twin": the same mesh
(same pose, updated with it), drawn with `ShadowMaterial`, whose small
shader (`shadow.wesl`) slides each point down along the light onto the
floor at the character's feet and draws it black and see-through by
`strength`: `darkness x (1 - distance along the light from the middle of
the body / max_trace_distance)`, about 0.75 for a standing trooper. The
twin writes depth with a strict "nearer than" test, and rounds its depth,
so the body's many overlapping triangles darken each floor spot once. No
extra camera or picture is drawn: one more draw of each body with a
very small shader, which phones and VR headsets can afford.
`CharacterShadows` switches all of them; `rc_game` sets it from SQUAD
SHADOWS (`video.squad_shadows`). Shadows farther than `cull_distance`
from `PlayerCamera` are hidden. All numbers are in
`data/shadows/shadows.toml`.

**Differences from the original.** The floor is taken as flat at the
feet: the shadow doesn't bend onto steps, slopes or walls, and it shows
under (not on) a prop standing on the floor next to the character. The
edges are sharp where the original's small picture softens them. How the
original's blending turns `ShadowDarkness` 255 into a final darkness
isn't known for sure. The squad and enemies don't use it yet (the squad
has the old flat round patch under each commando).

**Checking it.** `xvfb-run -a -s "-screen 0 1280x1024x24" cargo run -p
rc_shadows --example shadow_check`: two overlapping boxes on a white
floor, seen from above; checks where the shadow falls, its darkness, that
overlaps aren't darker, the off switch and the far-away cut-off. In the
game the log says `shadows: N more shadow copies (C characters, ...)`
and `shadows: character shadows on/off`.

## Multiplying materials and zone fog (geo_01a's arena)

**Multiplying `Combiner`s.** Most materials are one picture, perhaps
wrapped in shaders and modifiers, and we follow the wrapper links down to
it (`materials.rs`, `links_for`). A `Combiner` mixes two materials, and
for most of them we still show only `Material1`. The exception that
matters: `CombineOperation` 2, which the game's Engine.u enum
`EColorOperation` names `CO_Multiply`. geo_01a's big unlit backdrop meshes
(`EntireArena`, `RezSpires`, `UpperRock`, `upperStair`, all `bUnlit`) use
such materials (`Arena_Textures.Unique.TierWallLM` and 23 more in
arena_textures.utx): `Material1` is a painted light picture laid once over
the whole mesh (texture coordinate set 0), `Material2` a repeating stone
texture moved to set 1 by a `TexCoordSource` (`TexCoordSource` =
`TCS_Stream1`). Showing `Material1` alone drew the light picture: the
grey-white arena tiers. Now `Picture::multiply` carries the second picture
(`materials::Layer`) with its own coordinate set, the static mesh batch
gets those coordinates as `layer_uvs` (Bevy's `UV_1`), and `PropMaterial`
multiplies (`prop_light.wesl`, binding 101/102, scale in `settings.y`: 1,
or 2 / 4 for the Combiner's `Modulate2X` / `Modulate4X`). Multiplying in
linear light equals multiplying the stored values, as the game does, on a
pure power curve. A `TexCoordSource` (or any texture modifier storing
`TexCoordSource` 0-7) now also picks which coordinate set a plain picture
uses. `CO_Use_Color_From_Material2` (1) shows `Material2`. Not done: the
other operations (`CO_AlphaBlend_With_Mask` 5 with a `VertexColor` mask
blends rock and plaster on `RezSpires` and `RubbleBlend`; we show the
first), Combiners on walls (BSP) and on terrain.
Check: `cargo run -p rc_level_viewer --example multiply_check`.

**Zone fog (`fog.rs`).** Each `ZoneInfo` (and the `LevelInfo` for the rest)
can set `bDistanceFog`, `DistanceFogColor` (stored blue, green, red),
`DistanceFogStart`, `DistanceFogEnd` and `DistanceFogBlendTime`; values a
map leaves out are the `ZoneInfo` class defaults, read from Engine.u at
load (3000, 8000, grey 128, 1 s). geo_01a's arena zone: fog on, colour
142,119,94, start 3000, end 64000. The game's D3DDrv.dll sets Direct3D's
fog mode to `D3DFOG_LINEAR`, so a surface keeps `(end - d) / (end - start)`
of its colour; Bevy's `FogFalloff::Linear` is the same formula (units / 50).
`scene::build` reads every zone's fog into `Scene::fog` with a copy of the
BSP's zone tree; `spawn_level` puts it in the `LevelFog` resource, and
`ZoneFogPlugin`'s `follow_zone_fog` looks up the zone of every camera with
`ZoneFogView` each frame and sets its `DistanceFog`. The game's level
plugin puts `ZoneFogView` on the player's camera and takes it (and the
fog) away when the level ends; the viewer puts it on its camera unless
`--no-fog`. The visor's low light mode brings its own fog (the original's
vision mode `OverideFog`): it marks the camera `rc_game_core::OwnFog`
while on, and the level plugin's copy of `follow_zone_fog` skips such
cameras, then puts the zone's fog back when the mark goes. Our choices, not read from the game: the camera's zone picks
the fog, blended in a straight line over the new zone's blend time; no fog
in the sky box (geo_01a's `SkyZoneInfo` doesn't turn it on); `bClearToFogColor`
unused; fog mixed in linear light (Direct3D mixes on screen values), and
by distance rather than depth.

## Light shafts and god rays (`HsFalloff`, `rc_level_viewer::falloff`)

geo_01a's sun rays are one mesh (`Arena_HR.ArenaWalls.SunBeam`, material
`Arena_Textures.Unique.rayfalloff`), and its tunnels have light cones
(`Arena_GM.DecapTunels.VolumetricLight`, `Shader.VolumnLightTex`). Both
materials are `HsFalloff`. We drew them as their picture added on top,
darkened by the baked light and covered by the zone fog, so the rays were
a flat block in the haze or not seen at all.

What the game does, all read from its files:

| Read from | What it says |
| --- | --- |
| Engine.u, `HsFalloff` | settings `DiffuseTexture`, `FalloffGradient`, `UPanRate`, `VPanRate`, `UOscilationRate`, `VOscilationRate`, `DiffuseTint`, `GradientTint`, `SrcBlend`, `DestBlend`; defaults: tints white, `SrcBlend` 3, `DestBlend` 2 (`ED3DBLEND`: `SRCCOLOR`, `ONE`); shader `HardwareShaders.Falloff.FalloffImpl` |
| hardwareshaders.utx, `FalloffImpl` | vertex shader: picture place = texture place + c0 x sin(time) + c1 x time; facing = max(0, eye direction . normal) picks the gradient's U; vertex colour from the `FVF_Specular` stream; no fog value written. Pixel shader: picture x c0 x gradient x c1 x vertex colour |
| Engine.dll, `UHsFalloff`'s setup | copies the oscillation rates to vertex constant 0, the pan rates to 1, `DiffuseTint` and `GradientTint` (each byte / 255) to pixel constants 0 and 1, and the blend settings to the shader |

So the frame buffer gets colour x colour + what was there with the
defaults (the sun rays), or colour + what was there with `SrcBlend` `ONE`
(the light cones, which also pan down at 0.1 a second).

`materials::Materials::falloff` reads these into `falloff::Falloff` on
the picture; `draw::spawn_level` gives such batches the mesh's painted
colours and a `falloff::view::FalloffSurface`; `FalloffPlugin` (added by
`PropLightPlugin`) swaps in `FalloffMaterial` (`falloff.wesl`).

Our choices, not confirmed in the files:
- The vertex colour is the mesh's own painted colour stream (the sun
  beams are painted white at the bright end and black at the far end). We
  did not find which stream the game feeds to `FVF_Specular`.
- No distance fog on these surfaces: the shader writes no fog value (the
  game's projector shaders do), and in the user's screenshot the rays
  stand out from the haze.
- The game adds on its brightness curve, we add in linear light. The
  shader's result is added as it is: over mid-grey ground (0.45 on the
  curve) that is close to what adding on the curve does (0.84 x); over
  dark ground ours is brighter, over bright ground fainter. Turned into
  linear light first it was about five times too faint to see.
- The gradient is read at V 0 (the shader sets only U); the game's
  gradients are the same down every column.

Check: `cargo run -p rc_level_viewer --example falloff_check` with the
game's files (11 checks on geo_01a). The level viewer and the game log
`light shafts ... N prop parts, M draw batches`.

## Holograms, tank liquid and props with no stored light

**Holograms (`HsHologram`).** Republic Commando's own hologram material
names a `DiffuseTexture` and a `HologramColor`. The game draws it with
its `DynamicHologram` hardware shader, whose assembly text is stored in
`GameData/Textures/hardwareshaders.utx` (read to learn what it does,
not copied): the base picture's brightness x a glow by how squarely the
surface faces the eye x a scrolling noise picture x a flicker x the
colour. `rc_image::hologram` turns the picture into its brightness
(0.5 x red + 0.6 x green + 0.5 x blue, capped, as the shader does) in
the hologram colour; the rest is done while drawing by `rc_hologram`
(next section). Without that part the hologram is drawn as a steady
glow: unlit, added to what is behind it, from both sides. Used by the level's
static meshes (`rc_level_viewer::materials`, `Picture::hologram`) and by
models (`rc_model`, `Skin::hologram`; `rc_cast` draws them): the
prologue's hologram rifles (`Holo_DC17m`) and advisor (`Holo_Advisor`, a
clone trooper with a hologram skin). Without a picture (`Holo_Blank`) it
is plain glow. The default colour (Engine.u class defaults) is a light
blue.

**Shimmer and flicker (`rc_hologram`).** `HologramPlugin` swaps the steady
glow for `HologramMaterial` on every mesh with a `Hologram` component
(`draw::spawn_level` and `rc_cast` add it to hologram meshes). Its
shader, `hologram.wesl`, does what `DynamicHologram` does, read from the
shader's assembly text and its constants (`VSConstants`, `PSConstants`,
each a constant type from Engine.u's `SConstant` enum and four numbers
stored W, X, Y, Z):

| The original | Ours |
| --- | --- |
| Glow picture `GreyHoloFade` looked up at 0.11 x (eye . surface) | the same, read from `hardwareshaders.utx`; edges seen side-on fade out |
| Noise picture `NoiseDigital` across the model's X and Z at 0.0171 per unit, moved by -flicker x (0.2, 0.5) | the same; the noise jumps somewhere new each step (shimmer) |
| Vertex colour = facing x flicker (c22) | brightness x facing x small flicker |
| Height x flicker (c21, "big rare flicker") | the model's height squashes towards its pivot now and then |
| Position + flicker (c22, "small frequent flicker") | the model is nudged by about 1 unit |
| Additive, source alpha = glow picture's alpha | added, x the glow's alpha |
| A "distort" and "scan" swelling | not done: its switch heights (c18, 123,000 units) are never reached, so it's off in the game too |

The flicker values (`EVC_Flicker`) are filled by game code we can't see.
Their settings come in pairs (0.99/0.3, 0.1/0.4, 0/1); we read them as
"chance a step stays at 1" and "most it drops", the one reading that fits
all three. Steps come 30 times a second whatever the frame rate (the
original changed them every frame); the values come from a hash of the
step number in the shader, so there's no per-frame work on the CPU.
`HologramLook` holds every number for mods. The game's pictures are read
when `HologramGameData` names the `GameData` folder (`rc_level_plugin`
sets it from `GamePaths`); until then a plain fade and no noise stand in.

Open question: whether the game also multiplies by `HologramColor`'s
alpha (50 of 255 in every hologram, which would make them about five
times dimmer). We don't, so holograms stay as bright as the steady glow
the user has seen; `HologramLook::brightness` can turn them down.

Check: `cargo run -p rc_level_viewer --example hologram_check` (under
`xvfb-run` in the cloud; set `RC_GAME_DIR` to test the noise too)
measures a hologram ball on screen over 40 pictures: it got the new
material, it is light blue, its rim is darker than its middle (0.17 of
it), its brightness flickers (160 to 224) and its speckle moves.

**Cast skins.** A cast member's materials come from `data/cast/cast.toml`
if it names any, else the `Skins` the map sets on the actor
(`rc_cast::read::map_skins`), else its class's default `Skins`
(`ClassLibrary::object_list`). The briefing holograms rely on the last:
`Holo_Advisor` is a clone trooper mesh with the `CloneAdvisor`
hologram skin, `Holo_BattleDroid` a battle droid with
`Holo_BattleDroidRedShader`.

**Colour settings are stored blue first.** A material's `Color` setting
(`HologramColor`, `DiffuseTint`) is 4 bytes in the order blue, green,
red, alpha (`rc_image::unreal_color`). Read the other way the hologram
default would be orange and the "red team" holograms blue.

**Blended pictures.** A `FinalBlend` with `FrameBufferBlending` 2
(`FB_AlphaBlend`) now blends by its alpha and is seen from both sides
(`Picture::blend`), instead of being cut into solid and see-through
parts. The briefing holograms' grids and text use it.

**Tinted falloff (`HsFalloff`).** Light shafts and the prologue's tank
liquid multiply their picture by `DiffuseTint` and `GradientTint`
(white, no change, by default). Now drawn with the game's own falloff
shader: see "Light shafts and god rays" below. The tinted picture is
still the fallback when `HsFalloff`'s class defaults can't be read.

**Props with no stored light.** The map stores light only for props that
stay put (their `StaticMeshInstance`). Props a script moves or shows
later, like the clone babies in the prologue's tanks (`bStatic` off),
have none, and were drawn at full brightness: flat shapes with no
shading. Unreal lights such props from the lights around them as it
draws them, so `scene::unbaked_prop_light` does that once, when the
level loads: zone ambient (as other props) plus each light that doesn't
move: colour x brightness x facing x a straight fade to nothing at the
light's radius, x 1.6 for point and spot lights and x 0.8 for suns. Not
done: shadows, and lighting them again when a script moves them.

How the numbers were found: for the props that do have stored light,
work out the light this way too and compare corner by corner. On pro
(lit by lamps) the straight fade lit about as many corners as the
stored light (58,894 against 57,057) and was 1.58 times too dim with
little spread (middle half 1.46-1.61); a squared fade or a shorter reach
lit far too few corners. On geo_01briefing (lit by a sun) ours was 1.23
times too bright (0.80-0.81), which fits the terrain, lit the same way,
being 20% brighter than the props. geo_01a's canyon is mostly shadow,
which this doesn't do, so it wasn't used.

## The visor: tactical and low light modes (`rc_visor`)

The commando's visor has three modes, stepped through with CYCLE VISOR
MODE (X on PC, the original's `ToggleHeadlamp`): **normal** (the plain
view), **tactical** (squad mates glow in their own colour, things to pick
up glow pale blue) and **low light** (the helmet lamp). The rules come from
the original's scripts (`PlayerController` in Engine.u, `CTPlayer` in
CTGame.u, read, rewritten by hand in `rc_visor/src/rules.rs`):

- TACTICAL VISOR MODE (`VisorModeDefault`): ON (0) skips normal, OFF (1)
  skips tactical, CYCLE (2, the default) steps through all three.
- The visor starts in tactical mode on every level start and load, unless
  the option is OFF. Changing the option during play moves off a mode it no
  longer allows.
- TACTICAL MODE INTENSITY (`TacticalModeIntensity`, 0 to 1, default 0.2)
  is the glow's strength; the menu shows tenths (0 to 10).
- Each thing's glow colour is its class's `EnhancedVisionColor`, stored
  blue, green, red. Black means no glow, which is why enemies, walls and
  props don't glow. Sev (`Commando07DeltaDefaults`) is red, Fixer (40)
  green, Scorch (62) yellow, and every pickup (`CTPickupDefaults` and the
  ammo, weapon and bacta pickups) pale blue. These are read at run time from
  the install; `data/visor/tactical.toml` only names the classes, and can
  give colours of its own for mods.
- Switching to tactical or low light shows the mode's name ("TACTICAL
  MODE", "LOW LIGHT MODE", from the `Helmet` class's defaults) on the
  display for `VisorModeTextTime` (3 s).

**How the glow is drawn.** A `TacticalGlow("sev")` component marks a body;
every mesh under it (now or added later, such as a pickup's model) gets a
child copy (a "shell") with the same mesh and `TacticalGlowMaterial`
(`tactical_glow.wesl`), drawn additively on top. The shader makes edges
(where the surface turns away from the eye) glow most, so the body gets a
coloured outline, plus a small fill over the middle. Because the squad's
meshes are posed on the CPU, the shells share the same mesh and follow
every animation for free. Shells are hidden (cost nothing) outside tactical
mode or at intensity 0. The cost is one extra, very cheap draw per glowing
mesh: no textures, no extra screen passes, so it suits phones and VR.

**Our guess, not the original's.** The original draws the glow in its
compiled renderer (`UFrameFX::GetTacticalSkelMeshShader`), which we can't
read, so the glow's shape (`[glow]` in the data file: strength scale, edge
sharpness, fill) is ours, to be compared with the real game. Also not done:
the faint film grain the original's tactical mode adds over the whole
picture (`VisionShaderEnhance`) and its `EnhancedVisionFade` flag on clones
(meaning unknown).

**Low light mode (night vision).** The original's low light view is the
vision mode object `FrameFX.VisionModes.VisionNight` (class
`VisionModeSniper`, in `Textures/FrameFX.utx`). Its settings, read at run
time by `night_original.rs`: `VisionShader` = `VisionShaderNight` (a
"hardware shader": old Direct3D shader text plus constants and pictures),
`Noise` 30 (grain strength 30/255), `MotionBlur` 51 (keep 51/255 of the last
picture), `OverideFog` with `DistanceFogStart` 555 and `DistanceFogEnd` 4500
(Unreal units, black fog), `BlurPasses` 1. The `VisionMode` class default
`FlashlightOn` is true, so the lamp stays on. The shader's settings say what
it does: it reads a blurred copy of the picture, takes red, green and blue
squared and added up as one brightness (so black and white, darks darker,
lights brighter), fades in from black, multiplies by the transparency of
its overlay picture `HUDTextures.Helmet.HUD_GlassNight` (512x512, a
visor-shaped window: open in the middle, black outside, with notches at the
middle of each edge) and adds the overlay's colour (a very faint blue,
10/255), then adds film grain from `FrameFX.FBFX.FilmGrain` (128x128, grey),
repeated 9.2 times across and 6.5 down a 4:3 screen (its vertex constant 1)
and moved every frame. We read those pictures and numbers; the shader
(`night_vision.wesl`) is our own, written from that description, not a copy.

How ours is drawn: in low light mode a `NightVisionView` goes on the
player's camera, and one full-screen pass runs after tone mapping (the
original worked on the shown colours, so ours turns the colours back into
screen values first). The plumbing follows Bevy's `FullscreenMaterial` with
two pictures added (`night_vision.rs`). The HUD, arms and helmet frame are
drawn by other cameras afterwards and stay sharp. A `DistanceFog` (marked
`NightFog`) is added with the mode's fog and taken off again after. Cost:
six texture reads per pixel, one pass, no extra pictures kept, fine for
phones.

Wide screens: the window picture is made for 4:3. Stretched across 21:9 or
32:9 its curved edges and notches would be pulled wide. Instead
(`rules::window_u`, the same sum in the shader) the middle 0.25 screen
heights each side of the centre and the outer parts keep their shape, and
the plain part between them is drawn longer. Narrower than 4:3 it is
squeezed. The film grain gets more repeats across on wider screens so its
specks stay square.

`[low_light]` in `data/visor/tactical.toml`: `picture` (off: lamp only),
`mode` (which vision mode object to read), `boost`, `tint` (white is the
original), `window` (off for a full-screen picture, which VR will want),
`window_middle`, `grain_scale`, `fog`, `fade_seconds`, and stand-in values
used when the install can't be read (then the window is fully open and the
grain flat).

Not done: the trail (the original blends 80% of the new picture over 20%
of the last one, `MotionBlur` 51); that needs the last picture kept, a
second texture per camera. The original's `BloomOverideValue` 100 isn't
used (its `BloomOveride` switch isn't set). The fade out when leaving the
mode (`FadeOutTime` 0.2) isn't done; the picture goes off at once. Not
compared with the real game, which doesn't run on the user's PC right now.

**Plugging in.** `rc_game/src/visor.rs` copies the settings into
`VisorSettings` and puts `TacticalGlow` on squad mates (by name) and weapon
pickups. The visor never moves a camera or draws on the screen itself: the
glow is part of the 3D world (a VR headset sees it in both eyes, each from
its own eye position), the lamp hangs off whatever `PlayerCamera` is, and
the mode's name goes to the display as a `HudMessage`.

**Checked by** `rc_visor`'s tests (the rules, colour bytes, the data file,
and the install's colours when `RC_GAME_DIR` is set), the `visor_check`
example (a stand-in ball and box drawn in every mode, colours measured: no
red in normal mode, a red edge at 0.2, stronger at 1.0, edges redder than
the middle, a blue box edge, no glow and a lit floor in low light, the same
picture after cycling round), and `rc_game --script` (starts in tactical
with the squad glowing, X goes to low light with the lamp and back, the two
GAME page rows change the settings).

## Particle effects (`rc_effects`, `rc_effects_plugin`)

The game's fireballs, smoke, sparks and flying debris are `Emitter`
classes in the script packages (575 of them, mostly in
`Properties/CTEffects.u`). Each class's defaults list its parts in
`Emitters`: `SpriteEmitter`s (camera-facing squares with a picture) and
`MeshEmitter`s (copies of a model), each a block of named values
(`MaxParticles`, `Lifetime`, `StartVelocityRange`, `SizeScale`,
`ColorScale`, `Texture`, `TextureUSubdivisions`, `DrawStyle` ...). The
class also names a `SpawnSound`. `rc_effects` reads these with the engine's
defaults (`engine.u`'s `ParticleEmitter`) under the class's own values
(`read.rs`, `tagged.rs`), and runs them with no Bevy (`sim.rs`): spawning
at the emitter's rate, moving with speed, gravity and drag, growing and
fading by the scale lists, picking picture frames, bouncing debris off the
floor. `rc_effect_check` reads every effect in the install.

`rc_effects_plugin` draws them. Any part of the game sends `SpawnEffect`
(effect class, place, optional floor height and sound); `PreloadEffect`
reads one ahead of time (the level does this for every breakable prop's
effect, so the first blast doesn't stall). Sprites become one mesh per
emitter rebuilt each frame, with the emitter's draw style turned into a
blend mode (Translucent and Brighten add light, AlphaBlend blends,
Modulated and Darken multiply). Mesh debris is one entity per piece. The
sound plays with its loudness falling off with distance
(`sound_half_distance`). Settings are in `data/effects/effects.toml`
(on/off, size, a particle cap, volume).

Our choices (not checked against the original): a sprite's size is its
half-width; both the prop's `TransitionSound` and the effect's
`SpawnSound` play; added-light sprites ignore the colour list's alpha;
debris bounces off a flat floor at the prop's base, not the level;
`UseDirectionAs`, `MaxAbsVelocity` and blending between frames are
ignored; beam and trail emitters aren't drawn; sounds aren't placed in 3D
yet, only made quieter with distance (the level scripts' 3D sound in
`rc_scripts_plugin/src/sounds.rs` could be reused).

### Effect spawners (`FXFactory`, `rc_effects::factory`)

**What the original does** (ctgame.u's `Factory` and `FXFactory`
scripts). A map places `FXFactory` actors that make an effect class
(`Prototype`) again and again, at `SpawnPoint`s that share its `Tag`
(`SpawnMode` 0 in turn, 1 at random, 3 the one nearest the player) or at
the factory itself (2). It starts off; the event named by its `Tag` turns
it on and the same event again turns it off. While on, it makes one at
once, then one every `SpawnInterval` seconds (a random time up to
`SpawnIntervalMax` when that is bigger), as long as fewer than `MaxAlive`
of its effects are still going and it has some of its `Capacity` left
(`bGenerateForever` never runs out). A spawn point outside
`MinPlayerRange..MaxPlayerRange` of the player is skipped; with `bCovert`
one the player can see (in front within the field of view and nothing in
the way) is skipped. Each spawn sends `SpawnEvent`; running out sends
`ExhaustedEvent` and turns it off. Scripts' `ACTION_ResetFactory` fills
it back to `Capacity`. `FXFactory` adds a random `EmitterDrawScale` and
`SpawnOffset` to each effect. geo_01a's five `OverheadVehicles`
factories (gunships and Geonosian fighters, effects `cFX_Gunships` and
`cFX_GEOfighters` whose mesh emitters fly models across the sky) are
turned on by `ScriptedTrigger52` after the briefing.

**How we split it.** `rc_effects::factory::Spawners` holds the rules with
no Bevy: `trigger`, `reset`, `stop`, `ended` and `step`, with a
`PlayerView` the game gives it (where the player is, whether a point is in
view). `rc_scripts_plugin/src/fx_factories.rs` reads the factories and
spawn points from the map, passes level script events and
`ResetFactory`/`StopFactory` orders to them, sends `SpawnEffect` with an
id and the factory's turn and scale, and hears back `EffectEnded` from
`rc_effects_plugin` when an effect finishes. On/off and what is left go
in save files (`effect_spawners`).

For the flying models, mesh emitters now use `CoordinateSystem`
(`PTCS_Relative` turns a part's start place, speed and pull with the
effect), `UseMeshBlendMode` (the class default true: the model's own
material, drawn solid) and the effect's `bUnlit` (false on both flyover
effects: drawn with the level's light at that place, as droids are,
starting from `lit_model_light`).

**Our reading (not checked against the original's engine code):**
`EmitterDrawScale` multiplies every particle's size; the relative turn is
the effect's full rotation; only the X part of `StartSpinRange` and
`SpinsPerSecondRange` is used, so the flyovers' small Z spin (0.1, 0.01)
is not applied (which way each part turns a model is not known); a lit model starts from the same light level the droids use
(0.5); fades are not applied to models; the field of view for `bCovert`
is the camera's 4:3 one.

### Projectile spawners (`ProjectileFactory`, `rc_effects::projectile`)

**What the original does** (ctgame.u's `Factory` and `ProjectileFactory`,
engine.u's `Projectile`, ctinventory.u's `CTProjExplosive`). A
`ProjectileFactory` runs by the same rules as an effect spawner but makes
a projectile class. With `SpawnMode` 2 the projectile starts with the
factory's `InitialPhysicsFromFactory`/`InitialSpeedFromFactory` and turn;
otherwise with its spawn point's `InitialPhysics`/`InitialSpeed` (default
2, falling, and 0) and turn. Physics 6 flies straight; 2 falls with the
zone's gravity (`PhysicsVolume` default Z -1100, `TerminalVelocity`
12000). The class's `ProjectileEffect` is spawned and carried along; when
the projectile goes, the trail fades out (`Emitter.FadeOut`: no new
particles, the old ones finish). An explosive projectile blows up when
its `FuseTime` runs out, on hitting the world with `bWorldDetonate`
(the class's hit effect is spawned first, then `Explode` spawns it
again) or on touching an actor with `bActorDetonate`; `Explode` spawns the
`HitEffectsByMaterial` effect and hurts everything within
`DamageRadius` (`HurtRadius`). `ACTION_StopFactorySpawning` turns a
factory off (for good with `PermanentlyDisableFactory`) and can destroy
what it made (`DestroyFactoryOffspring`). geo_01a has six: air
bombardment (`GEO_01AProj`, Speed 8000, from Z 21244 at 10000 straight
down), missiles from the bunkers (`GEO_01A_BunkerMissile`, falling, 4000)
and the end-of-level bombardment; 52 on 15 maps in all.

**How we split it.** `rc_effects::projectile` (no Bevy) reads a class
and its parents from properties.u, physicalmaterials.u, ctinventory.u and
engine.u (`ClassPackages`), and moves a `Flight` with a hit test the game
passes in. `rc_effects::factory` gained `Kind::Projectile`, spawn-point
physics and speed, and `stop(tag, for_good, remove_offspring)`.
`rc_scripts_plugin/src/projectiles.rs` draws the model, carries the trail
(`SpawnEffect.follow`, moved each frame by `rc_effects_plugin`, faded when
the carrier goes), sweeps against `LevelCollision`, and on a blast spawns
the effect and sends `rc_combat::Explode`.

**Not done yet:** projectiles that aren't explosive or are laser bolts
(logged "isn't flown yet"), bouncing and sticking, touching actors
(`bActorDetonate`), zone gravity other than the default, decals and
per-surface effects (the default material effect is used), and the trails'
ribbon emitters (not drawn).

### Emitters placed in the map (`rc_scripts_plugin/src/placed_emitters.rs`)

**From the files.** A map places effects as actors whose class leads,
through its parents, to engine.u's `Emitter` (`rc_effects::read::Library::is_emitter`).
geo_01a has 11: the dust the gunship blows up (`REP_Gunship_GroundDust0`),
fires (`Fire_Medium`), smoke (`SpireBigSmoke`, `WhiteSmoke`), blowing
dust and floor fog. Each one runs its class's effect (from
`Properties/cteffects.u`) at its map place and turn from the start of the
level, for as long as the level lasts (`SpawnEffect::endless`, carried by
an `EmitterCarrier` entity). None of geo_01a's starts switched off.

**What a level event does to it (from engine.dll).** A level event that
names an emitter's `Tag` calls engine.u's `Emitter.Trigger`, which calls
each particle emitter's native `Trigger()`. engine.dll (decompiled for
reference only, nothing copied) shows `UParticleEmitter::execTrigger`
flipping the emitter's `Disabled` when its `TriggerDisabled` is set
(engine.u's default, kept by every geo_01a emitter and by the map's own
copies), and calling `Reset` when `ResetOnTrigger` is set (default off).
`AEmitter::Tick` and `AEmitter::Render` skip a disabled emitter: its
particles stop and vanish at once, and come back where they were when a
second event switches it on. `rc_effects::sim::EffectState::trigger` does
this, reached through `rc_effects_plugin::TriggerEffect`; the bool order
that maps the engine's bits to these names is engine.u's own
`ParticleEmitter` property list. geo_01a's gunship script sends
`REP_Gunship_GroundDust` as the gunship leaves for the spider droid at
the end of the level. A placed emitter's effect stays while the level
lasts, even with every emitter off, as the map's actor does.

**Not done.** The map's own copies of an emitter's settings (the class's
are drawn), the actor's `DrawScale` (logged), and keeping which are off
in save files.

## Mounted turrets (`rc_turrets`, `rc_turrets_plugin`)

**What the original does.** A `Turret` (the Wookiee heavy turret on
Kashyyyk, the Trandoshan missile turrets on the ship, the E-Web) holds one
gunner at a time. The player walks up, looks at it and presses Use
("PRESS @ TO MOUNT TURRET"); the view moves onto the gun, looking turns it
(at most `Speed` rotation units a second, inside `MinYaw..MaxYaw` and
`MinPitch..MaxPitch`, or all the way round with `bUnlimitedYaw`), Fire
fires the turret's own first-person weapon (`HeavyTurret`,
`MissileTurret`, `EWebTurret`), and Use again gets off at the map's exit
spot (`NavPts`, else `NavPtLocs` turned with the turret). Getting on sends
`PlayerControlEvent` (player only) then `ControlEvent`; getting off sends
`ReleaseEvent`. Scripts put characters on turrets with
`ACTION_AttachToTurret` (the first turret with that tag, in map order) and
change who may use one with `ACTION_SetTurretUsable` (`bPlayerUsable`).

**How we split it.**

- `rc_turrets` (no Bevy): the rules. `Turret::mount`/`leave` return the
  events to send; `turn` clamps by speed and limits and wraps the yaw to
  -32768..32768 after the limits, as the original does, so yyy_06d's
  limits of a whole turn mean no limit; `turn_toward` and `settle` for a
  character gunner and an empty turret. `map::read` reads each placed
  turret's tag, events, limits and exit spot. `params` reads
  `data/turrets/<id>.toml`.
- `data/turrets/*.toml`: one file per kind, claiming map classes
  (`classes = ["WookieeHeavyTurret"]`). Model and bones, aim limits and
  speed, gunner height, reach, exit offset, the player's weapon
  (`data/weapons/<id>.toml`) and, under `[auto]`, what a character gunner
  fires. A turret class no file claims is left out and logged
  (`MarkerTurret`, `RASCeilingTurret`). A mod adds a turret with a new file.
- `rc_game_core::Manning` (component on the player): which turret, where
  the body is held, where the gun points. The player's body and camera
  read it, so the view code doesn't know about turrets (a VR camera reads
  the same thing). `rc_game_core::TurretOrder` messages carry the script
  steps from `rc_scripts_plugin` without a dependency.
- `rc_turrets_plugin` (Bevy): draws the turrets (their `turrets.ukx` model,
  the yaw and pitch bones turned in the model's own axes), offers the
  prompt, reads `Action::Use` and `ActionState::look` (so mouse, pad, gyro
  and touch all aim it), swaps the turret's weapon into the player's
  `Arsenal` while on (and back after), sends the turret's events as
  `RaiseEvent`s, moves the player to the exit spot with `TeleportPlayer`,
  holds character gunners on the gun and fires `FireShot`s at the nearest
  enemy in range. Saves aim, gunner and usable flag (`turrets` save part).
- `rc_combat`: `fire.mirror = true` in a weapon file fires a second shot
  from the mirrored offset (the Wookiee turret's two barrels). While on a
  turret the player can't switch weapons or reload by hand.

**Checked against the maps** with `rc_turret_check` (reads only):
yyy_06d has four Wookiee turrets; `WookieeHeavyTurret0` is the player's
(sends `MountedTurret4`, `MaxPitch 16384`), and getting on it moves
`ScriptedTrigger11` (the ending) on to waiting for the targets to die.
The other three are tagged for `ACTION_AttachToTurret` (two share
`WookieeCannon1`; the first in the map, `WookieeHeavyTurret4`, wins, as in
the original). ras_03b's missile turret sends `TurretResponseDudes` and
`SpawnPerch3`; ras_03c's two missile turrets and E-Web send theirs.
`rc_turrets_plugin`'s test turns each model and checks the muzzles move
the way the gun aims (needs the game files).

**Squad mates on turrets ("Man turret").** The maps mark turrets the
squad can man with squad spots whose anchor is the turret (yyy_06d's
`SquadMarkerSpot161`..`164` at the Wookiee turrets) or with a
`MarkerTurret` order marker. `[job.turret]` in `data/squad/squad.toml`
matches them: a pullable post with `mount = "yes"`. Once the squad mate
is posted at the spot, `rc_squad_plugin` (`man_turrets`) sends
`TurretOrder::Squad { anchor, near, gunner, member, on }`. The turret part
puts them on the turret the anchor names (else the nearest within 512 uu
of the spot, our choice), sends its `ControlEvent`, and puts
`rc_game_core::OnTurret` (where to hold the body, where the gun points)
on the squad mate. The squad part owns where squad mates stand, so it
holds the body there itself (`Squad::hold_on_turret`): while mounted the
member doesn't think, move or fire their own gun. The turret part aims
and fires the turret's `[auto]` weapon at the nearest enemy in range,
exactly as for a script's character. Recall (F4), a new order, an auto
pull or going down takes them off the post: the squad part asks them off
(`on: false`), the `ReleaseEvent` goes, `OnTurret` is removed and
`Squad::off_turret` puts them back on their feet at the spot. A turret
that's taken (the player is on it) leaves them fighting from the spot;
they ask again every 2 s (`TURRET_ASK_EVERY`), so they get on once it's
free. Saves write the gunner as `squad:<name>`; after loading, the
squad mate is still posted, asks again, and the turret, already theirs,
takes them back without sending its event twice. Test:
`rc_turrets_plugin/tests/squad_turret.rs` (no game files). Check the maps
with `cargo run --release -p rc_turrets_plugin --example
squad_turret_survey -- <Maps folder> [map ...]`.

**Turrets that run themselves (`[automatic]`).** The core ship turrets
(geo_04b, geo_05a, geo_05c) are `AutoTurret`s in the original: folded
(`ClosedBreathe`) until a `Trigger` with their tag, then they play
`Deploy`, look for the nearest enemy they can see from `BaseEyeHeight`,
turn toward it and fire (`ShotDelay` 0.15 s), barrels in turn, straight
from the barrel at the target. After 4 s of firing they stop for 1 s
(`TimeUntilCooldown`, `CooldownDuration`). Another `Trigger` with their tag
switches them off. They have health (256), a damage table
(`DV_TurretsCoreshipAuto`) and send `FirstDamagedEvent` on the first hit and
`Event` when destroyed; nobody can get on them. In the port a turret file's
`[automatic]` section turns this on (`deployed`, `closed_pose`, `deploy`,
`fire_for`, `cool_down`, `side`, `health`, `damage_table`, `eye_height`).
Each automatic turret gets a hidden target entity (`LevelTurrets::hits`)
that shots and blasts hit and enemies aim at; the turret fires as that
entity. Turrets mounted on walls (geo_04b) use their full map rotation
(`Turret::axes`, `to_level`), and aim in their own frame
(`turn_toward_by`), allowing for barrels that hang below the pivot.

**Gunship cannons.** `GunshipCannon` (geo_04a, geo_04d) is a player turret
like the others: sit-down (`SitToUse`), yaw and pitch +-90 degrees, and an
instant-hit weapon (`GunshipTurret`: 50 damage, `CTDamageEnergyGunshipTurret`,
range 10000). Getting on sends its `ControlEvent`, which the scripts there
wait for. Our choice: the game fires it every frame (`FireRate` 0.01); we
fire every 0.05 s.

**Our choices.** Facing the turret within 1.2 radians counts as looking at
it (the original asks for the crosshair on it). The Trandoshan turret's
four barrels fire one after another for a character gunner. The turret
weapons' first-person offsets and the E-Web's reload time are guesses.

## Screen shake (`rc_game_core::shake`, `rc_game/src/player.rs`, `rc_scripts_plugin/src/shake.rs`)

Every shake in the original ends in one call on the player's controller,
`ShakeView(InTime, SustainTime, OutTime, X, Y, Z, YawMag, PitchMag,
Frequency)`. The port has the same single entry: a `ShakeView` message.
Anything that wants to shake the view (scripts now; explosions and weapons
later) sends one.

- **The rules** (`rc_game_core::shake::ShakeState`) follow the game's
  `Engine.u` script text: a new shake is ignored while a stronger one runs;
  the strength fades in over `InTime`, holds, and falls as
  `1 - elapsed / end`; each axis has a value that walks toward a new random
  target in -1..1 by at most `Frequency` a frame; the turn (Unreal rotation
  units, cut to a whole number) is added to where the player looks every
  frame and stays; the movement (Unreal units, world axes) is added to the
  eye position. A rotation's `Yaw` value shakes up and down and its `Pitch`
  value left and right, as the original swaps them.
- **When** (`rc_game/src/player.rs`, `shake_view`): once a frame, right
  after the player's own turning and its limits, also on a turret, as the
  engine's `UpdateRotation` does (read from Engine.dll). The camera
  (`place_camera`) adds `ViewShake::offset` to the eyes.
- **Where the numbers come from** (`rc_scripts_plugin/src/shake.rs`):
  `ACTION_ShakeView` steps and `ViewShaker` actors in the map; anything a
  map leaves out is the class default, read from `Gameplay.u` at level load.
  If those can't be read, those shakes are skipped and logged, not guessed.
  A `ViewShaker` shakes only if the player is closer than its
  `ShakeRadius`; the campaign's four set none, so (as in the original) they
  never do.
- **For a VR mod**: the shake is one resource, `ViewShake`, with two
  switches. `turn_view = false` stops it turning the player's view (a
  must in VR); `move_camera = false` stops the camera movement. Nothing
  else depends on them.
- **Log**: `scripts: ... ShakeView: shake the view: ...` when a script
  asks, `player: t=... shake from ...` when the view takes it (or
  ignores it), and `player: ... shake from ... over: N frames, camera moved
  up to X uu, turned up to P pitch / Y yaw a frame` when it ends.

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
