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
| `rc_level` | Reads maps: the level's own geometry (the BSP) and where every actor is placed. No Bevy. |
| `rc_level_viewer` | Bevy program: loads one map and lets you fly around it. |

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
on 5 maps. Zones, leaves, lightmaps and render sections come after these
arrays and aren't read yet.

Texture coordinates come from the surface: U = (corner - origin) . U
direction, same for V, in texels; the viewer divides by the texture's size.

In Republic Commando most of a level is static meshes, not BSP: `geo_01a`
has 249 BSP polygons but 355 placed props with 141,000 triangles.

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
