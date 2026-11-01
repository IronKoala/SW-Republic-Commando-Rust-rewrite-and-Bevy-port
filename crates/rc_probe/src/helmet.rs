//! `rc_probe --helmet [file]`: reads the original helmet frame (see
//! `rc_menus::helmet`), checks how it fits every landscape screen shape and
//! paints it without a GPU to `extracted/menus/helmet_*.png` (a gitignored
//! folder), on grey so the see-through middle shows.

use crate::Log;
use rc_menus::helmet::{self, HelmetPlan};
use rc_menus::Library;
use std::path::Path;

pub(crate) fn probe(log: &mut Log, game_data: &Path, plan_file: &Path, out_dir: &Path) -> bool {
    let plan = match std::fs::read_to_string(plan_file) {
        Ok(t) => match HelmetPlan::parse(&t) {
            Ok(p) => p,
            Err(e) => {
                log.line(format!("FAILED: {}: {e}", plan_file.display()));
                return false;
            }
        },
        Err(e) => {
            log.line(format!("{}: {e}; using the defaults", plan_file.display()));
            HelmetPlan::default()
        }
    };
    let mut lib = Library::new(game_data);
    let model = match helmet::load(&mut lib, &plan) {
        Ok(m) => m,
        Err(e) => {
            log.line(format!("FAILED: {e}"));
            return false;
        }
    };
    log.line(format!(
        "helmet {}: {} points, DisplayFOV {}",
        plan.mesh,
        model.points.len(),
        model.display_fov
    ));
    for n in &model.notes {
        log.line(format!("  {n}"));
    }
    let (lines, problems) = helmet::check_shapes(&model, &plan);
    for l in lines {
        log.line(l);
    }
    if std::fs::create_dir_all(out_dir).is_ok() {
        for (name, aspect) in [
            ("4x3", 4.0 / 3.0),
            ("16x9", 16.0 / 9.0),
            ("21x9", 21.0 / 9.0),
            ("32x9", 32.0 / 9.0),
            ("5x4", 1.25),
        ] {
            let h = 360;
            let w = (h as f32 * aspect).round() as usize;
            let mut p = helmet::paint(
                &model,
                &helmet::fit(&model, &plan, aspect),
                plan.brightness,
                w,
                h,
            );
            for px in p.pixels.chunks_exact_mut(4) {
                if px[3] == 0 {
                    px.copy_from_slice(&[90, 110, 140, 255]);
                }
            }
            let path = out_dir.join(format!("helmet_{name}.png"));
            match std::fs::File::create(&path).map(|f| rc_image::png::write(&p, f)) {
                Ok(Ok(())) => log.line(format!("painted {}", path.display())),
                _ => log.line(format!("couldn't write {}", path.display())),
            }
        }
    }
    if problems.is_empty() {
        log.line("HELMET CHECK: fits every shape");
        true
    } else {
        for p in &problems {
            log.line(format!("PROBLEM: {p}"));
        }
        log.line(format!("HELMET CHECK: {} problems", problems.len()));
        false
    }
}
