//! SCREEN SHOT: the key (F12 unless changed) saves a picture of the screen
//! as a `.png` file in the `screenshots` folder next to the settings file.

use crate::settings::SettingsPath;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use rc_game_core::{Action, ActionState, GameLog};
use std::path::PathBuf;

pub struct ScreenshotPlugin;

impl Plugin for ScreenshotPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, take);
    }
}

/// The folder screenshots go to.
fn folder(settings: &SettingsPath) -> PathBuf {
    settings
        .0
        .parent()
        .map(|p| p.join("screenshots"))
        .unwrap_or_else(|| PathBuf::from("screenshots"))
}

fn take(
    mut commands: Commands,
    actions: Res<ActionState>,
    settings: Res<SettingsPath>,
    mut count: Local<u32>,
) {
    if !actions.just_pressed(Action::Screenshot) {
        return;
    }
    *count += 1;
    let dir = folder(&settings);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("rc_game_{stamp}_{:03}.png", *count));
    commands.spawn(Screenshot::primary_window()).observe(
        move |shot: On<ScreenshotCaptured>, log: Res<GameLog>| {
            let img = &shot.image;
            let (w, h) = (img.width() as usize, img.height() as usize);
            let Some(data) = img.data.clone() else {
                log.line("screenshot: the picture had no pixels");
                return;
            };
            let bgra = matches!(
                img.texture_descriptor.format,
                TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb
            );
            // Packing and writing the file takes a moment; doing it beside
            // the game keeps the game from pausing.
            let (path, log) = (path.clone(), log.clone());
            std::thread::spawn(move || save(data, w, h, bgra, &path, &log));
        },
    );
}

fn save(data: Vec<u8>, w: usize, h: usize, bgra: bool, path: &std::path::Path, log: &GameLog) {
    let mut pixels = Vec::with_capacity(w * h * 4);
    for p in data.as_chunks::<4>().0.iter().take(w * h) {
        if bgra {
            pixels.extend([p[2], p[1], p[0], 255]);
        } else {
            pixels.extend([p[0], p[1], p[2], 255]);
        }
    }
    let bytes = rc_image::png::encode(&rc_image::Rgba {
        width: w,
        height: h,
        pixels,
    });
    let result = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|_| std::fs::write(path, bytes));
    match result {
        Ok(()) => log.line(format!("screenshot: saved {} ({w}x{h})", path.display())),
        Err(e) => log.line(format!("screenshot: can't save {}: {e}", path.display())),
    }
}
