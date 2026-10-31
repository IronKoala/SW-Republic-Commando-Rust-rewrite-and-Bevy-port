//! How the game covers the screen (`video.display_mode`): in a window, full
//! screen in a borderless window, or true full screen.
//!
//! - **Windowed:** the window is the chosen screen size.
//! - **Borderless:** a window without edges covering the whole monitor at
//!   the monitor's own size. Choosing a screen size of another shape than
//!   the monitor's switches to a window of that size (`option_settings`).
//! - **True full screen:** the monitor switches to the chosen size when it
//!   has that size and the same shape. For a size of another shape the
//!   monitor stays at its own size and the game is drawn in the largest
//!   centred box of the chosen shape, with black bars filling the rest
//!   (the "play area"). Every camera draws into that box, so the 3D view,
//!   the menus and the display all take the chosen shape.
//!
//! Everything else that needs the shape of the picture (the field of view,
//! the menus) reads [`PlayArea`] rather than the window's size.

use crate::option_settings::same_shape;
use crate::settings::{DisplayMode, Settings, VideoSection};
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{RenderTarget, Viewport};
use bevy::prelude::*;
use bevy::render::renderer::RenderAdapterInfo;
use bevy::window::{
    Monitor, MonitorSelection, PresentMode, PrimaryMonitor, PrimaryWindow, VideoMode,
    VideoModeSelection, WindowMode, WindowRef,
};
use bevy::window::{WindowFocused, WindowOccluded, WindowResized};
use rc_game_core::GameLog;
use std::time::{Duration, Instant};

pub struct DisplayPlugin;

impl Plugin for DisplayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Desktop>()
            .init_resource::<PlayArea>()
            .add_systems(
                Update,
                (read_desktop, apply_video, find_play_area, fit_cameras).chain(),
            )
            .add_systems(Startup, start_watchdog)
            .add_systems(Update, log_window_events)
            .add_systems(Last, (tick_frame_clock, limit_frame_rate));
    }
}

/// The main monitor: its own size (physical pixels) and the sizes it can
/// switch to for true full screen.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct Desktop {
    pub size: Option<(u32, u32)>,
    pub modes: Vec<VideoMode>,
}

/// The part of the window the game draws in, in physical pixels: the whole
/// window, except in true full screen at a size of another shape than the
/// monitor's (then a centred box of that shape, black around it).
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct PlayArea {
    pub window: UVec2,
    pub min: UVec2,
    pub size: UVec2,
    /// Physical pixels per logical pixel (the operating system's scaling).
    pub scale: f32,
}

impl Default for PlayArea {
    fn default() -> Self {
        PlayArea {
            window: UVec2::new(1280, 720),
            min: UVec2::ZERO,
            size: UVec2::new(1280, 720),
            scale: 1.0,
        }
    }
}

impl PlayArea {
    /// Width over height of the picture.
    pub fn aspect(&self) -> f32 {
        self.size.x as f32 / self.size.y.max(1) as f32
    }

    /// True when there are black bars.
    pub fn letterboxed(&self) -> bool {
        self.size != self.window
    }

    /// Size in logical pixels (what Bevy UI works in).
    pub fn logical_size(&self) -> Vec2 {
        self.size.as_vec2() / self.scale.max(0.01)
    }
}

/// The box the game draws in, inside a window `window` big: the whole
/// window, or in true full screen with a chosen size of another shape, the
/// largest centred box of that shape. (min, size), physical pixels.
pub fn play_area(window: UVec2, v: &VideoSection) -> (UVec2, UVec2) {
    let chosen = (v.width.max(1), v.height.max(1));
    if v.display_mode != DisplayMode::Fullscreen
        || window.x == 0
        || window.y == 0
        || same_shape(chosen, (window.x, window.y))
    {
        return (UVec2::ZERO, window);
    }
    let shape = chosen.0 as f32 / chosen.1 as f32;
    let size = if window.x as f32 / window.y as f32 > shape {
        // Wider window: bars at the sides.
        UVec2::new((window.y as f32 * shape).round() as u32, window.y)
    } else {
        // Taller window: bars above and below.
        UVec2::new(window.x, (window.x as f32 / shape).round() as u32)
    };
    let size = size.min(window).max(UVec2::ONE);
    ((window - size) / 2, size)
}

/// The window mode and size for the settings. `desktop` is None until the
/// monitor has been read.
pub fn window_plan(v: &VideoSection, desktop: &Desktop) -> (WindowMode, Option<(u32, u32)>) {
    let at = MonitorSelection::Current;
    match v.display_mode {
        DisplayMode::Windowed => (WindowMode::Windowed, Some((v.width, v.height))),
        DisplayMode::Borderless => (WindowMode::BorderlessFullscreen(at), None),
        DisplayMode::Fullscreen => {
            // Switch the monitor to the chosen size only when that keeps its
            // shape; other shapes are drawn with black bars at the
            // monitor's own size. Highest refresh rate of that size.
            let keeps_shape = desktop
                .size
                .is_some_and(|d| same_shape((v.width, v.height), d));
            let mode = desktop
                .modes
                .iter()
                .filter(|m| m.physical_size == UVec2::new(v.width, v.height))
                .max_by_key(|m| (m.refresh_rate_millihertz, m.bit_depth))
                .filter(|_| keeps_shape && desktop.size != Some((v.width, v.height)));
            let pick = match mode {
                Some(m) => VideoModeSelection::Specific(*m),
                None => VideoModeSelection::Current,
            };
            (WindowMode::Fullscreen(at, pick), None)
        }
    }
}

fn read_desktop(monitors: Query<&Monitor, With<PrimaryMonitor>>, mut desktop: ResMut<Desktop>) {
    let Ok(m) = monitors.single() else {
        return;
    };
    // The monitor's own size: the largest of its modes (its current size
    // changes while true full screen has switched it to a smaller one).
    let largest = m
        .video_modes
        .iter()
        .map(|v| (v.physical_size.x, v.physical_size.y))
        .max_by_key(|(w, h)| w * h);
    let now = Desktop {
        size: largest.or(Some((m.physical_width, m.physical_height))),
        modes: m.video_modes.clone(),
    };
    if *desktop != now {
        *desktop = now;
    }
}

/// Screen size, display mode and vsync changed in the options take effect
/// at once. The window keeps a size the player dragged it to until the
/// setting itself changes.
#[allow(clippy::type_complexity)]
pub fn apply_video(
    settings: Res<Settings>,
    desktop: Res<Desktop>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut projections: Query<&mut Projection>,
    mut last: Local<Option<((WindowMode, Option<(u32, u32)>), bool)>>,
    log: Res<GameLog>,
) {
    let v = &settings.video;
    let plan = window_plan(v, &desktop);
    let Some((before, vsync_before)) = last.replace((plan, v.vsync)) else {
        // The window was made from these settings.
        return;
    };
    let Ok(mut w) = windows.single_mut() else {
        return;
    };
    if before != plan {
        let (mode, size) = plan;
        if let Some((width, height)) = size {
            w.resolution.set(width as f32, height as f32);
        }
        w.mode = mode;
        log.line(format!(
            "display: {:?} ({}x{} chosen), window mode {mode:?}",
            v.display_mode, v.width, v.height
        ));
        // Every camera must take the new size in the same frame. Bevy
        // only re-reads a camera's size when its window reports a resize
        // or its projection changes; the player's camera changes its
        // projection at once (the view widens), so without this the arms'
        // and sky's cameras keep the old size for a frame and the GPU
        // stops the game ("attachments have differing sizes").
        for mut p in &mut projections {
            p.set_changed();
        }
    }
    if vsync_before != v.vsync {
        w.present_mode = if v.vsync {
            PresentMode::AutoVsync
        } else {
            PresentMode::AutoNoVsync
        };
        log.line(format!("display: vsync {}", v.vsync));
    }
}

fn find_play_area(
    settings: Res<Settings>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut area: ResMut<PlayArea>,
    log: Res<GameLog>,
) {
    let Ok(w) = windows.single() else {
        return;
    };
    let window = UVec2::new(w.physical_width(), w.physical_height());
    let (min, size) = play_area(window, &settings.video);
    let now = PlayArea {
        window,
        min,
        size,
        scale: w.scale_factor(),
    };
    if *area != now {
        if now.letterboxed() != area.letterboxed() || now.letterboxed() {
            log.line(format!(
                "display: drawing {}x{} at ({}, {}) in a {}x{} window{}",
                size.x,
                size.y,
                min.x,
                min.y,
                window.x,
                window.y,
                if now.letterboxed() {
                    ", black bars around it"
                } else {
                    ""
                }
            ));
        }
        *area = now;
    }
}

/// Fills the window with black before the game draws its box (only while
/// there are black bars).
#[derive(Component)]
pub struct BarsCamera;

/// The layer nothing is drawn on: the bars camera only clears.
const BARS_LAYER: usize = 31;

#[allow(clippy::type_complexity)]
fn fit_cameras(
    mut commands: Commands,
    area: Res<PlayArea>,
    mut cameras: Query<(&mut Camera, Option<&RenderTarget>), Without<BarsCamera>>,
    bars: Query<Entity, With<BarsCamera>>,
) {
    let want = area.letterboxed().then(|| Viewport {
        physical_position: area.min,
        physical_size: area.size,
        ..default()
    });
    for (mut cam, target) in &mut cameras {
        let to_window = match target {
            None | Some(RenderTarget::Window(WindowRef::Primary)) => true,
            _ => false,
        };
        if !to_window {
            continue;
        }
        let same = match (&cam.viewport, &want) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                a.physical_position == b.physical_position && a.physical_size == b.physical_size
            }
            _ => false,
        };
        if !same {
            cam.viewport = want.clone();
        }
    }
    match (area.letterboxed(), bars.single()) {
        (true, Err(_)) => {
            commands.spawn((
                BarsCamera,
                Camera2d,
                Camera {
                    order: -100,
                    clear_color: ClearColorConfig::Custom(Color::BLACK),
                    ..default()
                },
                RenderLayers::layer(BARS_LAYER),
            ));
        }
        (false, Ok(e)) => {
            commands.entity(e).despawn();
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(mode: DisplayMode, w: u32, h: u32) -> VideoSection {
        let mut v = Settings::default().video;
        v.display_mode = mode;
        v.width = w;
        v.height = h;
        v
    }

    #[test]
    fn black_bars_only_in_true_full_screen_at_another_shape() {
        let screen = UVec2::new(3440, 1440);
        // Same shape, or not true full screen: the whole window.
        for v in [
            video(DisplayMode::Fullscreen, 3440, 1440),
            video(DisplayMode::Borderless, 1920, 1080),
            video(DisplayMode::Windowed, 1920, 1080),
        ] {
            assert_eq!(play_area(screen, &v), (UVec2::ZERO, screen));
        }
        // 16:9 on an ultrawide: bars at the sides.
        let (min, size) = play_area(screen, &video(DisplayMode::Fullscreen, 1920, 1080));
        assert_eq!(size, UVec2::new(2560, 1440));
        assert_eq!(min, UVec2::new(440, 0));
        // 21:9 on a 16:9 TV: bars above and below.
        let (min, size) = play_area(
            UVec2::new(1920, 1080),
            &video(DisplayMode::Fullscreen, 2560, 1080),
        );
        assert_eq!(size, UVec2::new(1920, 810));
        assert_eq!(min, UVec2::new(0, 135));
        // 4:3 on 16:9.
        let (min, size) = play_area(
            UVec2::new(1920, 1080),
            &video(DisplayMode::Fullscreen, 1024, 768),
        );
        assert_eq!((min, size), (UVec2::new(240, 0), UVec2::new(1440, 1080)));
    }

    #[test]
    fn true_full_screen_switches_the_monitor_only_at_its_shape() {
        let mode = |w, h, hz| VideoMode {
            physical_size: UVec2::new(w, h),
            bit_depth: 32,
            refresh_rate_millihertz: hz,
        };
        let desktop = Desktop {
            size: Some((1920, 1080)),
            modes: vec![
                mode(1920, 1080, 60000),
                mode(1280, 720, 60000),
                mode(1280, 720, 144000),
                mode(1024, 768, 60000),
            ],
        };
        let fs = |w, h| window_plan(&video(DisplayMode::Fullscreen, w, h), &desktop).0;
        let at = MonitorSelection::Current;
        assert_eq!(
            fs(1280, 720),
            WindowMode::Fullscreen(at, VideoModeSelection::Specific(mode(1280, 720, 144000)))
        );
        // Another shape: the monitor keeps its size (black bars instead).
        assert_eq!(
            fs(1024, 768),
            WindowMode::Fullscreen(at, VideoModeSelection::Current)
        );
        assert_eq!(
            fs(1920, 1080),
            WindowMode::Fullscreen(at, VideoModeSelection::Current)
        );
        // A window has the chosen size; borderless doesn't change size.
        assert_eq!(
            window_plan(&video(DisplayMode::Windowed, 1600, 900), &desktop),
            (WindowMode::Windowed, Some((1600, 900)))
        );
        assert_eq!(
            window_plan(&video(DisplayMode::Borderless, 1600, 900), &desktop),
            (WindowMode::BorderlessFullscreen(at), None)
        );
    }
}

/// Writes the graphics card in use and every time the window gains or loses
/// focus, is hidden or changes size, so a log shows what the window was
/// doing when something went wrong with the picture.
fn log_window_events(
    adapter: Option<Res<RenderAdapterInfo>>,
    mut told_adapter: Local<bool>,
    mut focused: MessageReader<WindowFocused>,
    mut occluded: MessageReader<WindowOccluded>,
    mut resized: MessageReader<WindowResized>,
    log: Res<GameLog>,
) {
    if !*told_adapter {
        if let Some(a) = adapter {
            *told_adapter = true;
            log.line(format!(
                "display: graphics card {} ({:?}, {:?}), driver {} {}",
                a.name, a.device_type, a.backend, a.driver, a.driver_info
            ));
        }
    }
    for e in focused.read() {
        log.line(format!(
            "display: window {}",
            if e.focused { "focused" } else { "lost focus" }
        ));
    }
    for e in occluded.read() {
        log.line(format!(
            "display: window {}",
            if e.occluded { "hidden" } else { "shown again" }
        ));
    }
    if let Some(e) = resized.read().last() {
        log.line(format!(
            "display: window resized to {}x{}",
            e.width, e.height
        ));
    }
}

/// With vsync off, waits at the end of each frame so the game draws at most
/// `video.max_fps` pictures a second.
fn limit_frame_rate(settings: Res<Settings>, mut last: Local<Option<Instant>>) {
    let v = &settings.video;
    if v.vsync || v.max_fps == 0 {
        *last = None;
        return;
    }
    let frame = Duration::from_secs_f64(1.0 / v.max_fps as f64);
    if let Some(start) = *last {
        let next = start + frame;
        // Sleep most of the wait (sleeping can overshoot by a millisecond
        // or more), then spin for the rest.
        let now = Instant::now();
        if next > now + Duration::from_millis(2) {
            std::thread::sleep(next - now - Duration::from_millis(2));
        }
        while Instant::now() < next {
            std::hint::spin_loop();
        }
    }
    // Counting from now, a slow frame isn't made up for by rushing the next.
    *last = Some(Instant::now());
}

/// The graphics settings for `video.graphics_api`. `auto` also lets the
/// `WGPU_BACKEND` environment variable choose.
pub fn wgpu_settings(v: &VideoSection, log: &GameLog) -> bevy::render::settings::WgpuSettings {
    use bevy::render::settings::{Backends, WgpuSettings};
    let mut w = WgpuSettings::default();
    let chosen = match v.graphics_api.to_ascii_lowercase().as_str() {
        "auto" | "" => None,
        "vulkan" => Some(Backends::VULKAN),
        "dx12" | "directx" | "directx12" => Some(Backends::DX12),
        "gl" | "opengl" => Some(Backends::GL),
        other => {
            log.line(format!(
                "display: WARNING: video.graphics_api '{other}' isn't auto, vulkan, dx12 or gl; using auto"
            ));
            None
        }
    };
    if let Some(b) = chosen {
        w.backends = Some(b);
        log.line(format!(
            "display: graphics API {} (video.graphics_api)",
            v.graphics_api
        ));
    }
    w
}

/// When the last frame finished, in milliseconds since the game started.
#[derive(Resource, Clone)]
struct FrameClock {
    start: Instant,
    last_ms: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

/// Watches from its own thread for the game stopping between frames, and
/// says so in the log: when the picture freezes, the log then shows whether
/// the whole game stopped or only the picture.
fn start_watchdog(mut commands: Commands, log: Res<GameLog>) {
    use std::sync::atomic::Ordering;
    let clock = FrameClock {
        start: Instant::now(),
        last_ms: Default::default(),
    };
    let (start, last_ms, log) = (clock.start, clock.last_ms.clone(), log.clone());
    let spawned = std::thread::Builder::new()
        .name("frame watchdog".into())
        .spawn(move || {
            let mut stuck_since = None;
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let now = start.elapsed().as_millis() as u64;
                let last = last_ms.load(Ordering::Relaxed);
                let gap = now.saturating_sub(last);
                // Before the first frame the game is still starting up.
                if last > 0 && gap >= 5000 {
                    if stuck_since != Some(last) {
                        stuck_since = Some(last);
                        log.line(format!(
                            "display: WARNING: no new frame for {:.0} s, the game is stuck (last frame at {:.1} s)",
                            gap as f64 / 1000.0,
                            last as f64 / 1000.0
                        ));
                    }
                } else if let Some(at) = stuck_since.take() {
                    log.line(format!(
                        "display: frames again after being stuck from {:.1} s",
                        at as f64 / 1000.0
                    ));
                }
            }
        });
    if spawned.is_ok() {
        commands.insert_resource(clock);
    }
}

fn tick_frame_clock(clock: Option<Res<FrameClock>>) {
    if let Some(c) = clock {
        c.last_ms.store(
            c.start.elapsed().as_millis() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}
