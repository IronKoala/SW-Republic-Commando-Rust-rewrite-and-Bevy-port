# What already exists for Republic Commando

Researched 2026-10-29. These are references only. Nothing from them is copied into
this repository. When code here follows something we learned from one of them, the
code comment names the source.

## The game itself

- Unreal Engine 2, engine version 2226, build 139 for the Steam, GOG and retail PC
  versions (all the same build). Source: [SWRC Wiki, Game Versions](https://wiki.swrc-modding.net/index.php?title=Game_Versions).
- Files live under `GameData\`: `Maps` (`.ctm` maps), `Textures` (`.utx`),
  `Sounds` (`.uax`), `StaticMeshes` (`.usx`), `Animations` (`.ukx`), `System`
  (`.u` script packages, `.ini` settings, the `.exe`), `Music` (`.ogg`),
  `Movies`, `KarmaData` (physics). Source: [SWRC Wiki, Directory Layout](https://wiki.swrc-modding.net/index.php?title=Directory_Layout).
- Every `.ctm .utx .uax .usx .ukx .u` file is an "Unreal package": one container
  format with a header, a name table, an import table and an export table.

## File format documentation

| Source | What it gives us | License |
|---|---|---|
| [Unreal Wiki: Package File Format](https://unrealarchive.org/wikis/unreal-wiki/Legacy:Package_File_Format.html) and [Data Details](https://unrealarchive.org/wikis/unreal-wiki/Legacy:Package_File_Format/Data_Details.html) | The generic UE1/UE2 package layout: header, "compact index" numbers, name/import/export tables | Wiki text, used as description only |
| [UE Viewer / UModel](https://github.com/gildor2/UEViewer) by Konstantin Nosov (Gildor) | Reads Republic Commando packages. Has the Republic Commando quirks: package versions 133-148 and 154-159 with licensee version 1, a changed export-table layout from version 151, and mesh/texture differences | MIT |
| [SWRC Wiki](https://wiki.swrc-modding.net) | Versions, folder layout, ini files, class reference, UnrealScript reference | Community wiki |

## Decompilation and reverse-engineering projects

| Project | What it is | License |
|---|---|---|
| [SWRC-Modding/CT](https://github.com/SWRC-Modding/CT) (mirror: [doublej472/CT](https://github.com/doublej472/CT)) | C++ headers for the game's engine, rebuilt from Unreal Tournament 432 public sources plus reverse engineering of the game's DLLs. Includes an OpenGL renderer, a custom launcher and the "Republic Commando Fix". The wiki says it also holds decompiled pseudo-C. Not a full decompilation: the README says parts are missing | **No license stated.** Treat as read-only reference; never copy |
| The game's own UnrealScript | The `.u` packages in `System` contain the game's script source text for most classes. UCC or UModel can export it to a gitignored folder | Game content, never committed |

No complete decompilation of the game exists that we found. No existing Rust
reader for this game was found.

## Reference projects for structure (picked by the project owner)

| Project | What we take from it | License |
|---|---|---|
| [vladtrc/iw4L](https://github.com/vladtrc/iw4L) | Many small crates, one per format or system. Points at the user's own install, ships no assets | Apache-2.0 |
| [muffinmxn/gang-beasts-rust](https://github.com/muffinmxn/gang-beasts-rust) | Game folder set by an environment variable, clear "no game content" statement | MIT |
| [samwhosung/benilla](https://github.com/samwhosung/benilla) | Format readers are plain Rust with no Bevy in them, so they can be tested alone | MIT OR Apache-2.0 |

## Community

- SWRC Modding Discord, linked from the wiki.
- [Gildor's forums](https://www.gildor.org/smf/) for UModel support. An old thread
  reported UModel failing on `vehicles.ukx`; newer UModel versions list the game
  as supported.
