# Using the original game's menus and HUD

The menus and on-screen display in `rc_game` today (amber text on dark
glass) are **placeholders** we drew ourselves so the game could be started
and tested. The goal is to use the original game's own menus and HUD as
much as possible. This page records how the original builds them, what we
checked, and the plan. It is the next step after PR #11, for its own thread.

Everything below is read from the player's own install **at run time**.
None of the original's files, textures, text or code go into this
repository.

## Done so far: the main menu (2026-10-29)

`rc_game` now draws the original main menu (`XInterfaceCTMenus.CTMenuMain`)
and its quit question (`XInterface.MenuQuestionYesNo`) from the install,
with the original's pictures, fonts, colours, positions and focus and
select sounds. The amber placeholder pages are still used for everything
the main menu opens (campaign list, options, pause) and whenever the
install's menu packages can't be read.

How it works:

- `rc_package::class` reads a class's default values (tagged values at the
  end of the class's data), Republic Commando's enums (their value names)
  and which category each property is in. Checked on all 601 classes in
  `Core`, `Engine`, `XInterface`, `XInterfaceCommon`, `XInterfaceCTMenus`
  and `CTGame`: every one read, every enum read.
- `rc_package::font` reads Unreal fonts (letter rectangles on texture
  pages). `OrbitFonts` read fully; some `WarfareFonts` use a different
  layout and aren't needed yet.
- New crate `rc_menus` (no Bevy) turns a menu class into a list of
  widgets, filled in the way the original's menu code describes in its own
  comments: positions interpolated down arrays, other values copied down
  arrays, a widget's `Style` filled from its template, and the
  `DefaultMenu...` blocks filling the rest, never overwriting a value that's
  set. Then it turns the widgets into textured rectangles for a 640x480
  menu area (checked: the planets only fit inside their rings at that
  size). Pictures with the "stretch" scale modes are drawn in nine pieces.
  Text supports `MaxSizeX` with "..." or word wrap.
- `rc_menus::screens` has what the main menu's script does on a PC,
  rewritten by hand: the border grows by one row for QUIT, Xbox Live and
  controller items are hidden, NEW GAME starts `PRO`, QUIT asks "are you
  sure?" (starting on NO), and so on. No script text is copied.
- `rc_game/src/original_menu.rs` draws the rectangles with Bevy UI in the
  largest centred 4:3 area of the window (black at the sides), and moves
  focus with the menu actions, so keyboard, gamepad and mouse all work.
- Colours: the original multiplied and blended colours in sRGB numbers,
  Bevy does it in linear light. `rc_menus::gpu` bakes each tint into its
  own copy of the picture and adjusts transparency so the result matches
  the original over a black background.

Not done yet: the background video (`Movies/menu_bg.avi`, an MPEG-4 v2
video with no Rust decoder), LOAD GAME, PROFILES and EXTRAS (greyed out
until built), MULTIPLAYER (greyed out for good: offline only), the
campaign, options and pause menus, button prompt pictures, and filling the
sides of wide screens. Mouse use was not tested by the script.

How to look at a menu without the game: `rc_probe --menu` (main menu) or
`rc_probe --menu XInterface.MenuQuestionYesNo` paints it to
`extracted/menus/<Class>.png` and logs every widget to `logs/rc_menu.log`.
`rc_probe --classes <package.u> [Class]` logs a package's class defaults.

## Pages behind the main menu (2026-10-29, NOT TESTED with the game files)

The pause, campaign and options pages can each use their original menu.
`data/ui/original_menus.toml` lists, for each page, its class and what each
button's function does. When the game starts, each page's class is read
and checked (`rc_menus::pages::check`): a page uses the original only if
every widget on it is a kind we can draw (picture, text, text button) and
every button a PC player can reach has a command. Otherwise the amber
placeholder stays, and the log lists every reason, plus every button the
class has and the function it calls. So one run with the install shows
exactly what to add.

Built the night of 2026-10-29 without access to the game files (the PC
connection wasn't approved yet), so:

- The button function names in the file are guesses, except
  `RestartMissionSelected`, which was seen in `CTPausePCMenu` earlier.
- Whether `CTPausePCMenu` and `CTCampaignMenu` have widgets we can't draw
  (sliders, lists, scroll areas) is not known. The options classes almost
  certainly do, so they aren't listed yet.
- The pause menu is drawn over the game (no black backdrop), assuming its
  own `Background` widget darkens the view as the original does.
- `rc_game --script` presses Down, Down, Accept on the pause page for
  "Quit to main menu", which matches the placeholder. If the original
  pause page turns on, that step needs updating to its button order.

To check on the PC without playing: `rc_probe --pages` probes every page
in `data/ui/original_menus.toml` as `--menu` does (widgets and each
button's function in `logs/rc_menu.log`, a painting in `extracted/menus/`),
then ends with a summary saying which pages would use the original and
why the others wouldn't. In the game, the same reasons are in
`logs\rc_game.log` on lines starting `original menus: page`.

## What the original's menus are made of (checked 2026-10-29)

Checked by reading the install's packages with our own `rc_package` reader
(a throwaway tool in the cloud sandbox, not committed).

- **Menu classes.** Every menu is an UnrealScript class built on the
  `Menu` system from `XInterface.u` (`MenuBase`, `MenuTemplate`,
  `MenuTemplateTitled`, `...TitledBA` and so on, which add the title and
  the A/B/X button prompts). The game's own menus are in
  `XInterfaceCTMenus.u` (53 classes): `CTMenuMain`, `CTCampaignMenu`,
  `CTPausePCMenu`, `CTLoading`, `CTLoadingInfo`, `CTTitleCardMenu`,
  `CTDebriefingInfoGEO/RAS/YYY`, `CTGraphicsOptionsPCMenu`,
  `CTSoundOptionsPCMenu`, `CTControlsOptionsPCMenu`, `CTMapKeyMenu`,
  `CTLoadGameMenu`, `CTSaveGameMenu`, `CTCreditsMenu`, `CTExtrasMenu`, and
  Xbox versions of several. Shared ones (message boxes, profiles, warnings)
  are in `XInterfaceCommon.u` (28 classes).
- **Layouts are data.** Each menu's layout is in its class's default
  values, which our property reader already understands. For example
  `CTMenuMain` has 34 default values: the planet pictures (`Kamino`,
  `Geonosis`, `AssaultShip`, `Kashyyyk` and their borders), `GameTitle`,
  `Label` ("MAIN MENU"), `Border`, and the eight `MenuOptions` buttons (New
  Game, Load Game, Campaign Map, Multiplayer, Options, Profiles, Extras,
  Quit), plus the quit question's text. Each item is a struct:
  - `MenuSprite`: `WidgetTexture`, `TextureCoords`, `RenderStyle`,
    `DrawColor`, `DrawPivot`, `PosX`, `PosY`, `ScaleX`, `ScaleY`,
    `ScaleMode`, `bHidden`, `Platform`, `Style`.
  - `MenuText`: `MenuFont`, `Text`, `DrawColor`, `DrawPivot`, `PosX`,
    `PosY`, `ScaleX`, `ScaleY`, `Kerning`, `MaxSizeX`, `bWordWrap`,
    `TabStops`, `Style`.
  - `MenuButtonText`: a `MenuText` plus its button behaviour (focus and
    select colours, sounds, what it calls when chosen).
  - Positions are fractions of the screen (0 to 1). For example, "MAIN
    MENU" is at 0.24 across and 0.575 down, and the Kamino picture is at
    0.1375 and 0.16. `Style` names a shared look defined in
    `MenuTemplate`'s defaults (fonts, colours, sounds).
- **Behaviour is script.** What a button does (for example
  `CTMenuMain.NewGameSelected`, `CTPausePCMenu.RestartMissionSelected`) is
  UnrealScript. The packages include the script's text, so the behaviour
  can be read and rewritten by hand in Rust. Each menu has only a handful
  of short functions. The text is never copied into the repository, as
  with all of the original's code.
- **Pictures** are in `Textures/GUIContent.utx` (162 textures, in the
  groups `Menu`, `LoadingScreens`, `LoadInfoScreens`, `Multiplayer`,
  `Shader`), which our texture reader can already read.
- **Fonts** are in `Textures/WarfareFonts.utx` (43 `Font` objects:
  `FontEurostile8` to `37`, `FontNeuzeit8` to `37`, `FontSmall`,
  `FontMedium`, `FontLarge`, `FontMono`, with 800x600 and 1024x768
  versions) and `Textures/OrbitFonts.utx` (`OrbitBold8` to `24`). An Unreal
  font is a set of texture pages plus a rectangle for each letter. Reading
  that table is new work, but small.
- **Mission names and loading text** come from
  `System/LevelLoadingInfo.int`, which `rc_game` already reads.

## The original's HUD

The HUD is different: `CTHUD` in `CTGame.u` draws almost everything in
native code (`PostRender`, `RenderToScale`, `DisplayFOV`), not script, and
the visor frame is a 3D helmet drawn close to the camera. So there is no
layout to read. Its pictures are all there to reuse:

- `Textures/HUDTextures.utx`: `Reticles` (one crosshair per weapon, such as
  `HUD_WeaponDefault`, `HUD_WeaponPistol`, `HUD_WeaponSniper`),
  `Icons` (`HUD_Ammo`, `HUD_ClipFrame`, squad visor icons, button icons,
  compass), `VisorDamage` (cracks), `VisorBlood`, `Helmet` (sniper and night
  vision glass, flashlight).
- `Textures/HUDArmsTextures.utx`: the helmet and arms.

**Started (2026-10-29, NOT TESTED with the game files):** the crosshair.
`rc_game/src/original_hud.rs` draws the reticle picture named in
`data/ui/hud.toml` (one per weapon, by the weapon file's `name`, else the
default), centred and sized by screen height (one picture pixel per line
of a 480-line screen, a guess), red over an enemy. If the picture can't be
read, the placeholder's bars stay and the log says why. The paths
(`HUDTextures.Reticles.HUD_WeaponDefault`, `...HUD_WeaponPistol`) come
from the earlier look at the package and still need checking. Not done:
spread (the bars move apart; the picture doesn't grow yet), shield,
health, ammo and squad pictures.

The plan for the HUD is to place these original pictures where the original
shows them, matched by eye and by measuring against screenshots of the
real game, with the helmet model drawn the way the original does once the
character work can load it.

## Plan (a separate thread after PR #11)

1. **Read class defaults.** Add to `rc_package` a reader for a class's
   default values (the same tagged values we already read for textures and
   meshes, found at the end of the class's data), and for nested structs.
2. **Read Unreal fonts.** Letter rectangles plus texture pages, drawn as
   images in Bevy UI.
3. **A menu renderer.** A Bevy plugin that draws `MenuSprite`,
   `MenuText` and `MenuButtonText` items from a menu class by name, with
   focus, selection and the original's sounds.
4. **Menu behaviour.** Rewrite each menu's few functions in Rust, behind
   the same page and command logic `rc_game` already has (so settings,
   level select and pause keep working). Start with the main menu, the
   campaign menu, pause, the loading screen and the options pages.
5. **Screen shapes.** The original's positions assume a 4:3 screen. Draw
   each menu in a centred 4:3 area and extend its background to fill wider
   or taller screens, so nothing is stretched.
6. **Prompts.** The original's button pictures (A, B, X, Y) change to key
   names when the last input was keyboard and mouse
   (`ActionState::last_device`).
7. **The HUD** as above, after the menus.
8. **Modding.** A mod can replace any layout value or picture through the
   mods folder, since layouts are read as data.

The placeholder menus stay as the fallback when the install's menu
packages can't be read.
