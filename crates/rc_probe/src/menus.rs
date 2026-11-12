//! `rc_probe --menu [Package.Class]`: builds one of the original menus from
//! the install (default: the main menu), logs every widget with where it
//! goes, and paints it without a GPU to `extracted/menus/<class>.png`
//! (a gitignored folder) so it can be looked at and compared.

use crate::Log;
use rc_menus::layout::Kind;
use rc_menus::quads::{self, Sizes};
use rc_menus::{campaign, render, screens, Assets, Library};
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
    if rc_menus::debriefing::is_debriefing(&data.family) {
        // As a level script fills it in. The words are this probe's own
        // sample (the real ones are in the maps); the picture is the
        // Geonosis one, named without its group as the map names it.
        let filled = rc_menus::debriefing::setup(
            &mut layout,
            &mut assets.library,
            "SAMPLE DEBRIEFING TITLE",
            "Sample text written by rc_probe to check the layout: a long \
             paragraph has to wrap onto several lines inside the frame, the \
             way a real debriefing's text does, without running into the \
             picture or the CONTINUE button below it.",
            "GUIContent.GEO_Debrief",
        );
        let widened = rc_menus::debriefing::fit_continue(&mut layout, &mut assets);
        let size = rc_menus::debriefing::fit_text(&mut layout, &mut assets);
        log.line(format!(
            "debriefing filled in: picture {:?}, problems {:?}, CONTINUE widened: {widened}, text size {size:.2}",
            filled.picture, filled.problems
        ));
    }
    if class.eq_ignore_ascii_case(campaign::CAMPAIGN_MENU) {
        // As the game shows it: Geonosis chosen, its third mission picked.
        if let Some(mut c) = campaign::read(&data) {
            campaign::setup(&mut layout, &c);
            campaign::choose(&mut layout, &mut c, 0);
            for p in &c.planets {
                log.line(format!(
                    "planet {} {:?}: {} missions {:?}, {} maps in the every-map list",
                    p.button,
                    p.title,
                    p.missions.len(),
                    p.missions,
                    p.every_map.len()
                ));
            }
            campaign::highlight(&mut layout, &c, Some(2));
            focus = rc_menus::lists::widget_of(&layout, 0, 2);
        }
    }
    if rc_menus::pause::is_pause_menu(&layout) {
        // The pause menu sets its rows up each time it opens
        // (rc_menus::pause): each case painted on its own, and the
        // pictures below show it playing with a quicksave.
        use rc_menus::pause::PauseState;
        let cases = [
            ("playing", PauseState::default()),
            (
                "playing_quicksave",
                PauseState {
                    has_quick_save: true,
                    ..Default::default()
                },
            ),
            (
                "playing_quicksave_hints_off",
                PauseState {
                    has_quick_save: true,
                    can_enable_hints: true,
                    ..Default::default()
                },
            ),
            (
                "down",
                PauseState {
                    no_health: true,
                    incapacitated: true,
                    has_quick_save: true,
                    ..Default::default()
                },
            ),
            (
                "dead",
                PauseState {
                    no_health: true,
                    has_quick_save: true,
                    ..Default::default()
                },
            ),
            (
                "briefing",
                PauseState {
                    briefing: true,
                    ..Default::default()
                },
            ),
        ];
        let name = class.rsplit('.').next().unwrap_or(class);
        let _ = std::fs::create_dir_all(out_dir);
        let mut chosen = None;
        for (case, s) in cases {
            let mut l = layout.clone();
            let Some(rows) = rc_menus::pause::init(&mut l, &s) else {
                continue;
            };
            let f = rc_menus::pause::row(&l, rows.focus);
            let names: Vec<String> = rows
                .shown
                .iter()
                .filter_map(|&i| rc_menus::pause::row(&l, i))
                .map(|w| match &l.widgets[w].kind {
                    Kind::Button(b) => format!("{} at {:.4}", b.blurred.text, b.blurred.pos[1]),
                    _ => String::new(),
                })
                .collect();
            log.line(format!(
                "pause menu {case}: top {:.4}, focus {}, rows: {}",
                rows.top,
                rows.focus,
                names.join(", ")
            ));
            quads::set_pixels_high(480.0);
            let q = quads::menu_quads(&l, f, &mut assets);
            let picture = render::render(&q, &mut assets, 640, 480, [0, 0, 0, 255]);
            let path = out_dir.join(format!("{name}_{case}.png"));
            let _ = std::fs::File::create(&path).and_then(|f| rc_image::png::write(&picture, f));
            if case == "playing_quicksave" {
                chosen = Some((f, l));
            }
        }
        if let Some((f, l)) = chosen {
            focus = f;
            layout = l;
        }
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
                "button {:?} -> {} {}{}box {:?} text colour {:?} focused {:?}{}",
                b.blurred.text,
                b.on_select,
                if b.disabled { "(disabled) " } else { "" },
                if b.mouse_only { "(mouse only) " } else { "" },
                quads::sprite_box(&b.background_blurred, &mut assets).map(round),
                b.blurred.color,
                b.focused.color,
                if b.is_chooser() {
                    format!(" choices {:?}", b.items)
                } else {
                    String::new()
                }
            ),
        };
        log.line(format!("  [{shown}] pass {} {}: {line}", w.pass, w.name));
    }
    let focus = focus.or_else(|| layout.buttons_on_pc().first().copied());
    // Painted 480 pixels high, or RC_PROBE_MENU_HIGH (to compare with a
    // screenshot of the original). Stretched pictures' corners are one
    // texel to one pixel, so the quads depend on it.
    let high: usize = std::env::var("RC_PROBE_MENU_HIGH")
        .ok()
        .and_then(|h| h.parse().ok())
        .filter(|&h| h >= 120)
        .unwrap_or(480);
    quads::set_pixels_high(high as f32);
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
    let picture = render::render(&quads, &mut assets, high * 4 / 3, high, [0, 0, 0, 255]);
    let name = class.rsplit('.').next().unwrap_or(class);
    let _ = std::fs::create_dir_all(out_dir);
    let path = out_dir.join(format!("{name}.png"));
    match std::fs::File::create(&path).and_then(|f| rc_image::png::write(&picture, f)) {
        Ok(()) => log.line(format!("painted to {}", path.display())),
        Err(e) => log.line(format!("couldn't write {}: {e}", path.display())),
    }
    // The same menu spread over wider screens (rc_menus::fit), painted
    // 480 pixels high.
    for (shape, aspect) in [("16x9", 16.0 / 9.0), ("21x9", 2.39), ("32x9", 32.0 / 9.0)] {
        let mut wide = layout.clone();
        let across = rc_menus::fit::across_for(aspect, rc_menus::pages::DEFAULT_WIDEST_SHAPE);
        let spread = rc_menus::fit::spread(&mut wide, &mut assets, across);
        let screen = aspect / (4.0 / 3.0);
        let left = 0.5 - screen / 2.0;
        let mut q = quads::menu_quads(&wide, focus, &mut assets);
        for quad in &mut q {
            quad.dst[0] = (quad.dst[0] - left) / screen;
            quad.dst[2] = (quad.dst[2] - left) / screen;
        }
        let picture = render::render(
            &q,
            &mut assets,
            (high as f32 * aspect).round() as usize,
            high,
            [0, 0, 0, 255],
        );
        let path = out_dir.join(format!("{name}_{shape}.png"));
        let _ = std::fs::File::create(&path).and_then(|f| rc_image::png::write(&picture, f));
        log.line(format!(
            "{shape}: {} groups spread over {across:.2} 4:3 widths, widened {:?}, bridged {:?}; still overlapping: {:?}; painted to {}",
            spread.groups,
            spread.widened,
            spread.bridged,
            rc_menus::fit::text_overlaps(&wide, &mut assets),
            path.display()
        ));
    }
    for p in &assets.problems {
        log.line(format!("  PROBLEM {p}"));
    }
    assets.problems.is_empty()
}

fn round(b: [f32; 4]) -> [f32; 4] {
    b.map(|v| (v * 1000.0).round() / 1000.0)
}

/// `rc_probe --pages [file]`: for every page in `data/ui/original_menus.toml`
/// (or the file given), probe and paint its menu as `--menu` does, then say
/// whether the game would use it or keep the placeholder, and why.
pub(crate) fn pages(log: &mut Log, game_data: &Path, file: &Path, out_dir: &Path) -> bool {
    let plans = match std::fs::read_to_string(file)
        .map_err(|e| e.to_string())
        .and_then(|t| rc_menus::pages::parse_plans(&t))
    {
        Ok(p) => p,
        Err(e) => {
            log.line(format!("FAILED: {}: {e}", file.display()));
            return false;
        }
    };
    let mut verdicts = Vec::new();
    for plan in &plans {
        log.line(format!("=== page {}: {} ===", plan.page, plan.class));
        probe(log, game_data, &plan.class, out_dir);
        let mut assets = Assets::new(Library::new(game_data));
        let verdict = match assets.library.class(&plan.class) {
            Ok(c) => {
                let mut layout = rc_menus::build(&c);
                if rc_menus::debriefing::is_debriefing(&c.family) {
                    // As the game sets it up (CONTINUE shows on a PC).
                    rc_menus::debriefing::show_continue(&mut layout);
                }
                match rc_menus::pages::check(&mut layout, plan) {
                    Ok(b) => {
                        // Fill the campaign page's list as the game does,
                        // so its rows are checked too.
                        if let Some(mut camp) = campaign::read(&c) {
                            campaign::setup(&mut layout, &camp);
                            campaign::choose(&mut layout, &mut camp, 0);
                        }
                        let overlaps = quads::text_overlaps(&layout, &mut assets);
                        for (a, b, by) in &overlaps {
                            log.line(format!(
                                "  OVERLAP page {}: {a} and {b} cross by {:.1}% of the width",
                                plan.page,
                                by * 100.0
                            ));
                        }
                        format!(
                            "USES THE ORIGINAL ({} buttons), {} overlapping texts",
                            b.commands.len(),
                            overlaps.len()
                        )
                    }
                    Err(why) => format!("keeps the placeholder: {}", why.join("; ")),
                }
            }
            Err(e) => format!("keeps the placeholder: can't read the class: {e}"),
        };
        verdicts.push(format!("page {}: {verdict}", plan.page));
    }
    log.line("=== summary ===");
    for v in verdicts {
        log.line(v);
    }
    true
}

/// `rc_probe --hud [hud.toml]`: paints the HUD from the original's
/// pictures and fonts, with made-up numbers, at several screen shapes, to
/// `extracted/menus/hud_<width>x<height>.png`, over a grey background
/// standing in for the game. Logs any overlap `check_plan` finds.
pub(crate) fn hud(log: &mut Log, game_data: &Path, file: &Path, out_dir: &Path) -> bool {
    use rc_menus::hud::{check_plan, hud_quads, HudLayout, HudValues, SquadLine};
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) => {
            log.line(format!("FAILED: can't read {}: {e}", file.display()));
            return false;
        }
    };
    let layout = match HudLayout::parse(&text) {
        Ok(l) => l,
        Err(e) => {
            log.line(format!("FAILED: {}: {e}", file.display()));
            return false;
        }
    };
    let mut assets = Assets::new(Library::new(game_data));
    let member = |number: &str, name: &str, health: f32, down: bool, order: &str| SquadLine {
        number: number.into(),
        name: name.into(),
        health,
        down,
        order: order.into(),
    };
    let values = HudValues {
        shield: 0.7,
        health: 0.45,
        weapon: "DC-17m Blaster".into(),
        clip: Some(42),
        clip_size: Some(60),
        reserve: Some(180),
        grenade: Some(("Thermal Detonator".into(), 2)),
        squad: vec![
            member("07", "Sev", 1.0, false, "Form up"),
            member("40", "Fixer", 0.6, false, "Search and destroy"),
            member("62", "Scorch", 0.0, true, ""),
        ],
        messages: vec![
            "Picked up 2 thermal detonators".into(),
            "Objective: find a way into the hive".into(),
        ],
        pickup: "DC-17m Blaster Clips".into(),
        pickup_fade: 1.0,
        prompt: Vec::new(),
        prompt_fade: 0.0,
        subtitle: "Clone Advisor 1: Proceed to the next bacta station and plug in. You'll see a tall device with a cross on it.".into(),
        subtitle_fade: 1.0,
        objective: String::new(),
        objective_fade: 0.0,
        compass: 0.37,
        shield_hit_steps: 0,
        shield_hit_red: 0,
        frame_pulse: None,
        blinking: vec![],
    };
    // The objective shows when nobody speaks and no message is up.
    let objective_only = HudValues {
        messages: Vec::new(),
        subtitle: String::new(),
        objective: "USE BACTA TO HEAL".into(),
        objective_fade: 1.0,
        ..values.clone()
    };
    // Through the helmet's glass, as in the game with HELMET on.
    let plan = rc_menus::helmet::HelmetPlan::default();
    let helmet = match rc_menus::helmet::load(&mut Library::new(game_data), &plan) {
        Ok(m) => Some(m),
        Err(e) => {
            log.line(format!("  no helmet glass ({e}); HUD not moved"));
            None
        }
    };
    let mut ok = true;
    for (w, h) in [
        (1024, 768),
        (1920, 1080),
        (3440, 1440),
        (3840, 1080),
        (2400, 1080),
        (2208, 1768),
    ] {
        let (wf, hf) = (w as f32, h as f32);
        for p in check_plan(&layout, wf, hf, 16.0 / 9.0, 3) {
            log.line(format!("  OVERLAP {p}"));
            ok = false;
        }
        let glass = helmet
            .as_ref()
            .map(|m| rc_menus::helmet::GlassMap::new(m, &plan, wf / hf));
        let place = |p: [f32; 2]| glass.as_ref().map_or(p, |g| g.place(p));
        let place: Option<&dyn Fn([f32; 2]) -> [f32; 2]> = glass.as_ref().map(|_| &place as _);
        // The shields just hit: the bars swell and go red for a moment.
        let shield_hit = HudValues {
            shield_hit_steps: 2,
            shield_hit_red: 150,
            ..values.clone()
        };
        for (name, v) in [
            ("hud", &values),
            ("hud_objective", &objective_only),
            ("hud_shield_hit", &shield_hit),
        ] {
            let quads = hud_quads(&layout, v, wf, hf, 16.0 / 9.0, &mut assets, place);
            let picture = render::render(&quads, &mut assets, w, h, [70, 70, 70, 255]);
            let _ = std::fs::create_dir_all(out_dir);
            let path = out_dir.join(format!("{name}_{w}x{h}.png"));
            match std::fs::File::create(&path).and_then(|f| rc_image::png::write(&picture, f)) {
                Ok(()) => log.line(format!(
                    "{w}x{h}: {} pictures, painted to {}",
                    quads.len(),
                    path.display()
                )),
                Err(e) => log.line(format!("couldn't write {}: {e}", path.display())),
            }
        }
    }
    for p in &assets.problems {
        log.line(format!("  PROBLEM {p}"));
    }
    ok && assets.problems.is_empty()
}
