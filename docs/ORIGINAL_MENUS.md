# Using the original game's menus and HUD

The menus and on-screen display in `rc_game` today (amber text on dark
glass) are **placeholders** we drew ourselves so the game could be started
and tested. The goal is to use the original game's own menus and HUD as
much as possible. This page records how the original builds them, what we
checked, and the plan. It is the next step after PR #11, for its own thread.

Everything below is read from the player's own install **at run time**.
None of the original's files, textures, text or code go into this
repository.

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
