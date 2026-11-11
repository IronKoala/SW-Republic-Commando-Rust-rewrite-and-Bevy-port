//! The menus' moving background: the original game's video, read from
//! the install and played in a loop behind the menu pictures.
//!
//! What the original does, from its own files:
//! - Each menu class names its video in `BackgroundMovieName` (an Xbox
//!   `.xmv` name, `Menu_BG.xmv` for the main menu, options, campaign,
//!   load and question pages; empty for the pause menu) and draws it with
//!   its `BackgroundMovie` widget (a `MenuMovie`: colour 192,192,192, so
//!   at three quarters brightness, sized to the whole 4:3 menu screen,
//!   declared before every other widget, so under all of them).
//! - `Engine.dll` swaps `.XMV` for `.AVI` and looks in `..\Movies\`, so
//!   the PC plays `GameData/Movies/menu_bg.avi` (Microsoft MPEG-4 v2,
//!   512x512, 29.97 frames a second, 475 frames).
//! - `MenuBase.StartBackgroundMovie` (script in `XInterface.u`) plays it
//!   looping at the file's own rate (`MaxFrameRate = 0`). It only plays on
//!   the front-end (the "entry" level), not in a mission. A new menu with
//!   the same video takes over the playing one
//!   (`TransferTransientElements`), so it carries on without restarting.
//! - Matching the user's screenshots of the original against our decoded
//!   frames shows the 512x512 picture stretched over the whole 4:3 screen.
//!
//! On screens wider (or narrower) than 4:3 the original has nothing to
//! copy; we keep the 4:3 look and cut off the top and bottom (or sides)
//! to fill the screen, see [`MenuVideo::crop`].
//!
//! `rc_video` decodes the file; this keeps a Bevy picture up to date with
//! the frame that should show now. `original_menu::draw` places it.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use rc_game_core::{GameLog, GamePaths};

/// The video playing behind the menus.
#[derive(Resource, Default)]
pub struct MenuVideo {
    /// The `BackgroundMovieName` this was opened for.
    pub name: String,
    pub player: Option<rc_video::VideoPlayer>,
    pub image: Option<Handle<Image>>,
    /// The widget's colour, multiplied into every pixel (as the
    /// original multiplies 8-bit colours).
    color: [u8; 4],
    /// Real time when frame 0 showed.
    started: Option<f64>,
    /// Names that failed to open (so they are tried and logged once).
    failed: Vec<String>,
    rgba: Vec<u8>,
    /// Frames put on screen since the video (re)started, for the log and
    /// the script check.
    pub frames_shown: u64,
    pub last_frame: Option<usize>,
    logged_loops: u32,
    /// Frames counted from the start (not wrapped) of the picture on
    /// screen, to know how far behind the clock the video is.
    decoded_to: Option<u64>,
    /// Frames let go late because the computer couldn't decode them in
    /// time (see [`MAX_DECODES_PER_UPDATE`]); logged once.
    pub frames_late: u64,
    /// True while a menu that shows the video is drawn.
    pub wanted: bool,
}

/// Most video frames decoded in one game frame. On a computer that can't
/// decode 30 pictures a second (an unoptimised build on the cloud's
/// software renderer), decoding every missed frame to catch up made each
/// game frame longer than the last, until the whole game ran at under one
/// frame a second. Past this many, the video runs late instead: it still
/// shows every picture in order, just slower. A computer that keeps up
/// (the user's PC, any release build) never reaches it.
pub const MAX_DECODES_PER_UPDATE: u64 = 3;

/// How many frames to skip in time (shown late, not dropped) when the
/// clock says `frame` and the picture on screen is `decoded_to`.
fn frames_to_let_go(frame: u64, decoded_to: Option<u64>) -> u64 {
    match decoded_to {
        Some(done) => frame.saturating_sub(done + MAX_DECODES_PER_UPDATE),
        None => 0,
    }
}

/// The `.avi` the PC game plays for an Xbox `.xmv` name: the same name
/// with `.avi` in `GameData/Movies` (case doesn't matter). Folders in the
/// name are dropped.
pub fn avi_for(paths: &GamePaths, xmv: &str) -> std::path::PathBuf {
    let base = xmv.rsplit(['/', '\\']).next().unwrap_or(xmv);
    let stem = base.rsplit_once('.').map_or(base, |(s, _)| s);
    paths.game_file(&format!("GameData/Movies/{stem}.avi"))
}

/// Multiply 8-bit colours as the original's `DrawColor` does.
fn tint(rgba: &mut [u8], color: [u8; 4]) {
    if color[..3] == [255, 255, 255] {
        return;
    }
    for p in rgba.chunks_exact_mut(4) {
        for k in 0..3 {
            p[k] = ((p[k] as u32 * color[k] as u32 + 127) / 255) as u8;
        }
    }
}

impl MenuVideo {
    /// Open the video for `name` if it isn't open yet. Returns the picture
    /// to draw, or `None` when there is no video (logged once).
    pub fn picture_for(
        &mut self,
        name: &str,
        color: [u8; 4],
        paths: &GamePaths,
        images: &mut Assets<Image>,
        log: &GameLog,
    ) -> Option<Handle<Image>> {
        if name.is_empty() {
            return None;
        }
        if self.name.eq_ignore_ascii_case(name) && self.player.is_some() {
            self.color = color;
            return self.image.clone();
        }
        if self.failed.iter().any(|f| f.eq_ignore_ascii_case(name)) {
            return None;
        }
        let path = avi_for(paths, name);
        let opened = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(rc_video::VideoPlayer::open);
        let mut player = match opened {
            Ok(p) => p,
            Err(e) => {
                log.line(format!(
                    "menu video: {name:?} -> {}: can't play: {e}",
                    path.display()
                ));
                self.failed.push(name.to_string());
                return None;
            }
        };
        let info = player.info.clone();
        if let Err(e) = player.step() {
            log.line(format!("menu video: {}: first frame: {e}", path.display()));
            self.failed.push(name.to_string());
            return None;
        }
        self.color = color;
        player.rgba(&mut self.rgba);
        tint(&mut self.rgba, color);
        let image = Image::new(
            Extent3d {
                width: info.width,
                height: info.height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            self.rgba.clone(),
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
        );
        log.line(format!(
            "menu video: playing {name:?} from {}: {}x{} {} at {:.3} frames a second, {} frames ({:.2} s), looping, colour {color:?}",
            path.display(),
            info.width,
            info.height,
            info.codec_name(),
            info.fps(),
            info.frames.len(),
            info.seconds()
        ));
        let handle = images.add(image);
        self.name = name.to_string();
        self.player = Some(player);
        self.image = Some(handle.clone());
        self.started = None;
        self.frames_shown = 1;
        self.last_frame = Some(0);
        self.decoded_to = Some(0);
        self.logged_loops = 0;
        Some(handle)
    }

    /// The part of the video picture to show in a play area `aspect`
    /// wide, in video pixels. The original fills its 4:3 screen with the
    /// video (stretching the square picture); on another shape we keep
    /// that 4:3 look and fill the screen by cutting off the top and bottom
    /// (or the sides), so nothing is stretched further. That choice is
    /// ours: the original only drew its menus at 4:3.
    pub fn crop(&self, aspect: f32) -> Option<Rect> {
        let info = &self.player.as_ref()?.info;
        let (w, h) = (info.width as f32, info.height as f32);
        const SHOWN_AT: f32 = 4.0 / 3.0;
        Some(if aspect >= SHOWN_AT {
            let keep = h * SHOWN_AT / aspect;
            Rect::new(0.0, (h - keep) / 2.0, w, (h + keep) / 2.0)
        } else {
            let keep = w * aspect / SHOWN_AT;
            Rect::new((w - keep) / 2.0, 0.0, (w + keep) / 2.0, h)
        })
    }

    /// The frame on screen now, cut like [`Self::crop`] for a box
    /// `aspect` wide and sized to `width` x `height` (bilinear), for the
    /// script's software copy of the menu.
    pub fn software_frame(
        &self,
        width: usize,
        height: usize,
        aspect: f32,
    ) -> Option<rc_image::Rgba> {
        let player = self.player.as_ref()?;
        let crop = self.crop(aspect)?;
        let (vw, vh) = (player.info.width as usize, player.info.height as usize);
        if self.rgba.len() != vw * vh * 4 {
            return None;
        }
        let mut pixels = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for x in 0..width {
                let u = crop.min.x + (x as f32 + 0.5) / width as f32 * crop.width() - 0.5;
                let v = crop.min.y + (y as f32 + 0.5) / height as f32 * crop.height() - 0.5;
                let x0 = u.floor().clamp(0.0, (vw - 1) as f32);
                let y0 = v.floor().clamp(0.0, (vh - 1) as f32);
                let (fx, fy) = ((u - x0).clamp(0.0, 1.0), (v - y0).clamp(0.0, 1.0));
                let (x0, y0) = (x0 as usize, y0 as usize);
                let (x1, y1) = ((x0 + 1).min(vw - 1), (y0 + 1).min(vh - 1));
                let at = |x: usize, y: usize, k: usize| self.rgba[(y * vw + x) * 4 + k] as f32;
                for k in 0..3 {
                    let top = at(x0, y0, k) * (1.0 - fx) + at(x1, y0, k) * fx;
                    let bottom = at(x0, y1, k) * (1.0 - fx) + at(x1, y1, k) * fx;
                    pixels.push((top * (1.0 - fy) + bottom * fy).round() as u8);
                }
                pixels.push(255);
            }
        }
        Some(rc_image::Rgba {
            width,
            height,
            pixels,
        })
    }
}

/// Show the frame that belongs to the time since the video started.
pub fn play(
    mut video: ResMut<MenuVideo>,
    time: Res<Time<Real>>,
    mut images: ResMut<Assets<Image>>,
    log: Res<GameLog>,
) {
    let video = &mut *video;
    if !video.wanted {
        // No menu shows it (in a mission, or no menu): next time one
        // does, it starts again from its first frame.
        if video.started.take().is_some() {
            if let Some(p) = video.player.as_mut() {
                p.restart();
            }
            video.last_frame = None;
            video.decoded_to = None;
            video.frames_shown = 0;
            video.logged_loops = 0;
        }
        return;
    }
    let Some(player) = video.player.as_mut() else {
        return;
    };
    let now = time.elapsed_secs_f64();
    let started = *video.started.get_or_insert(now);
    let fps = player.info.fps();
    if fps <= 0.0 {
        return;
    }
    let mut frame = ((now - started) * fps).floor().max(0.0) as u64;
    let late = frames_to_let_go(frame, video.decoded_to);
    if late > 0 {
        // Too far behind: move the start later so the video carries on
        // from here, instead of decoding every frame it missed.
        video.started = Some(started + late as f64 / fps);
        if video.frames_late == 0 {
            log.line(format!(
                "menu video: this computer can't decode {fps:.1} frames a second; the video runs late instead of slowing the game ({late} frames let go)"
            ));
        }
        video.frames_late += late;
        frame -= late;
    }
    let target = (frame % player.frame_count() as u64) as usize;
    if Some(target) == video.last_frame {
        return;
    }
    if let Err(e) = player.advance_to(frame) {
        log.line(format!("menu video: {e}; stopped"));
        video.player = None;
        return;
    }
    video.last_frame = Some(target);
    video.decoded_to = Some(frame);
    video.frames_shown += 1;
    player.rgba(&mut video.rgba);
    tint(&mut video.rgba, video.color);
    if let Some(mut image) = video.image.as_ref().and_then(|h| images.get_mut(h)) {
        if let Some(data) = image.data.as_mut() {
            if data.len() == video.rgba.len() {
                data.copy_from_slice(&video.rgba);
            }
        }
    }
    if player.loops > video.logged_loops {
        video.logged_loops = player.loops;
        if video.logged_loops <= 3 {
            log.line(format!(
                "menu video: looped back to frame 0 (loop {}, {:.1} s since start, {} frames shown); {:?}",
                video.logged_loops,
                now - started,
                video.frames_shown,
                player.stats()
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xmv_names_become_avi_files_in_movies() {
        let paths = GamePaths::new("/nowhere", Vec::new());
        let p = avi_for(&paths, "Menu_BG.xmv");
        assert!(
            p.ends_with("GameData/Movies/Menu_BG.avi"),
            "{}",
            p.display()
        );
    }

    #[test]
    fn a_slow_computer_lets_the_video_run_late_instead_of_catching_up() {
        // Keeping up, or a little behind: decode every frame, as before.
        assert_eq!(frames_to_let_go(10, Some(9)), 0);
        assert_eq!(frames_to_let_go(12, Some(9)), 0);
        // 100 frames behind: decode only the next 3, let the rest run late.
        assert_eq!(frames_to_let_go(109, Some(9)), 97);
        // Nothing on screen yet (just started): nothing to let go.
        assert_eq!(frames_to_let_go(50, None), 0);
    }

    #[test]
    fn the_widget_colour_darkens_like_the_original() {
        let mut px = vec![200, 100, 0, 255];
        tint(&mut px, [192, 192, 192, 255]);
        assert_eq!(px, vec![151, 75, 0, 255]);
    }
}
