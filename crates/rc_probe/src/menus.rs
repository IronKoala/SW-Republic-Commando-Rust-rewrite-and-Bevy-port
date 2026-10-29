//! `rc_probe --menu [Package.Class]`: builds one of the original menus from
//! the install (default: the main menu), logs every widget with where it
//! goes, and paints it without a GPU to `extracted/menus/<class>.png`
//! (a gitignored folder) so it can be looked at and compared.

use crate::Log;
use rc_menus::layout::Kind;
use rc_menus::quads::{self, Sizes};
use rc_menus::{render, screens, Assets, Library};
use std::path::Path;

pub(crate) fn probe(log: &mut Log, game_data: &Path, class: &str, out_dir: &Path) -> bool {
    let mut assets = Assets::new(Library::new(game_data));
    log.line(format!(
        "packages found: {}",
        assets.library.package_count()
    ));
    let data = match assets.library.class(class) {
        Ok(d) => d,
        Err(e) => {
            log.line(format!("FAILED: {e}"));
            return false;
        }
    };
    log.line(format!("class family: {}", data.family.join(" <- ")));
    let mut layout = rc_menus::build(&data);
    let mut focus = None;
    if class.eq_ignore_ascii_case(screens::MAIN_MENU) {
        for l in screens::setup_main_menu(&mut layout, &[screens::MainAction::Multiplayer]) {
            log.line(l);
        }
    } else if class.eq_ignore_ascii_case(screens::QUESTION) {
        // As the main menu's QUIT button asks it.
        let main = assets.library.class(screens::MAIN_MENU).ok();
        let question = main.as_ref().and_then(|m| match m.get("QuitGameConfirm") {
            Some(rc_menus::value::V::Str(s)) => Some(s.clone()),
            _ => None,
        });
        let under = main.map(|m| {
            let mut l = rc_menus::build(&m);
            screens::setup_main_menu(&mut l, &[]);
            l
        });
        focus = screens::setup_question(&mut layout, &question.unwrap_or_default(), under.as_ref());
    } else if data
        .family
        .iter()
        .any(|c| c.ends_with(".MenuTemplateTitled"))
    {
        screens::setup_titled(&mut layout);
    }
    log.line(format!(
        "{} widgets ({} shown on PC), skipped: {:?}",
        layout.widgets.len(),
        layout.widgets.iter().filter(|w| w.shown_on_pc()).count(),
        layout.skipped
    ));
    log.line(format!(
        "sounds: {:?}; background video: {:?}; music: {:?}",
        layout.sounds, layout.background_movie, layout.music
    ));
    for w in &layout.widgets {
        let shown = if w.shown_on_pc() { "shown " } else { "hidden" };
        let line = match &w.kind {
            Kind::Sprite(s) => format!(
                "sprite {:?} box {:?} colour {:?} mode {}",
                s.texture,
                quads::sprite_box(s, &mut assets).map(round),
                s.color,
                s.scale_mode
            ),
            Kind::Text(t) => format!(
                "text {:?} font {:?} box {:?} colour {:?}",
                t.text,
                t.font,
                quads::text_box(t, &mut assets).map(round),
                t.color
            ),
            Kind::Button(b) => format!(
                "button {:?} -> {} {}box {:?} text colour {:?} focused {:?}",
                b.blurred.text,
                b.on_select,
                if b.disabled { "(disabled) " } else { "" },
                quads::sprite_box(&b.background_blurred, &mut assets).map(round),
                b.blurred.color,
                b.focused.color
            ),
        };
        log.line(format!("  [{shown}] pass {} {}: {line}", w.pass, w.name));
    }
    let focus = focus.or_else(|| layout.buttons_on_pc().first().copied());
    let quads = quads::menu_quads(&layout, focus, &mut assets);
    log.line(format!("{} quads to draw", quads.len()));
    for q in &quads {
        if !q.widget.starts_with("MenuOptions") && q.widget != "Border" {
            continue;
        }
        if q.texture.contains("Font") {
            continue;
        }
        log.line(format!(
            "  quad {} {} style {} src {:?} dst {:?} colour {:?}",
            q.widget,
            q.texture,
            q.render_style,
            q.src,
            round(q.dst),
            q.color
        ));
    }
    // Sizes are cached by now, so this only checks that every font loaded.
    for w in &layout.widgets {
        if let Kind::Text(t) = &w.kind {
            if let Some(f) = &t.font {
                if assets.font(f).is_none() {
                    log.line(format!("  font missing: {f}"));
                }
            }
        }
    }
    let picture = render::render(&quads, &mut assets, 640, 480, [0, 0, 0, 255]);
    let name = class.rsplit('.').next().unwrap_or(class);
    let _ = std::fs::create_dir_all(out_dir);
    let path = out_dir.join(format!("{name}.png"));
    match std::fs::File::create(&path).and_then(|f| rc_image::png::write(&picture, f)) {
        Ok(()) => log.line(format!("painted to {}", path.display())),
        Err(e) => log.line(format!("couldn't write {}: {e}", path.display())),
    }
    for p in &assets.problems {
        log.line(format!("  PROBLEM {p}"));
    }
    assets.problems.is_empty()
}

fn round(b: [f32; 4]) -> [f32; 4] {
    b.map(|v| (v * 1000.0).round() / 1000.0)
}
