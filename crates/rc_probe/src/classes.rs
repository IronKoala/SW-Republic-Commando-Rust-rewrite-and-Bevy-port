//! `rc_probe --classes <package> [ClassName]`: reads the default values of
//! every class in a script package (`.u`) and the names of every enum, and
//! checks they read cleanly. With a class name, writes out that class's
//! default values in full, including the values inside structs. This is how
//! the original menus' layouts were found (see `docs/ORIGINAL_MENUS.md`).
//!
//! `rc_probe --fonts <package>`: reads every font in a texture package and
//! checks its letters and pages.

use crate::Log;
use rc_package::class::{self, object_class, object_path};
use rc_package::font;
use rc_package::properties::{Property, PropertyType, Value};
use rc_package::Package;
use std::fs;
use std::path::Path;

pub(crate) fn scan(log: &mut Log, target: &Path, only: Option<&str>) -> bool {
    let data = match fs::read(target) {
        Ok(d) => d,
        Err(e) => {
            log.line(format!("FAILED: can't read {}: {e}", target.display()));
            return false;
        }
    };
    let pkg = match Package::parse(&data) {
        Ok(p) => p,
        Err(e) => {
            log.line(format!("FAILED: {e}"));
            return false;
        }
    };
    let (mut classes, mut failed, mut enums, mut enum_failed, mut unchecked) = (0, 0, 0, 0, 0);
    for e in &pkg.exports {
        let name = pkg.name(e.object_name);
        let kind = pkg.export_class_name(e);
        if kind == "Class" {
            if only.is_some_and(|o| !o.eq_ignore_ascii_case(name)) {
                continue;
            }
            classes += 1;
            match class::read_class_defaults(&pkg, &data, e) {
                Ok(d) => {
                    log.line(format!(
                        "class {name} (parent {}): {} default values from offset {} of {}{}",
                        pkg.object_name(d.super_class),
                        d.properties.len(),
                        d.start,
                        e.serial_size,
                        if d.layout_checked {
                            ""
                        } else {
                            " (layout NOT checked)"
                        }
                    ));
                    if !d.layout_checked {
                        unchecked += 1;
                    }
                    if only.is_some() {
                        if let Ok(b) = class::export_data(&data, e) {
                            log.line(format!(
                                "  first bytes {:02X?}",
                                &b[..(d.start + 8).min(b.len())]
                            ));
                        }
                        for p in &d.properties {
                            dump(log, &pkg, p, 1);
                        }
                    }
                }
                Err(err) => {
                    failed += 1;
                    log.line(format!("class {name}: FAILED: {err}"));
                }
            }
        } else if kind == "Enum" && only.is_none() {
            enums += 1;
            match class::read_enum(&pkg, &data, e) {
                Ok(names) => log.file_only(format!("enum {name}: {}", names.join(", "))),
                Err(err) => {
                    enum_failed += 1;
                    log.line(format!("enum {name}: FAILED: {err}"));
                    if let Ok(b) = class::export_data(&data, e) {
                        log.file_only(format!("  bytes {:02X?}", &b[..b.len().min(64)]));
                    }
                }
            }
        }
    }
    log.line(format!(
        "SUMMARY: {classes} classes ({failed} failed, {unchecked} layout not checked), {enums} enums ({enum_failed} failed)"
    ));
    failed == 0 && unchecked == 0 && enum_failed == 0 && classes > 0
}

fn dump(log: &mut Log, pkg: &Package, p: &Property, depth: usize) {
    let pad = "  ".repeat(depth);
    let index = if p.array_index > 0 {
        format!("[{}]", p.array_index)
    } else {
        String::new()
    };
    let name = format!("{}{index}", pkg.name(p.name));
    if let Some(inner) = class::struct_properties(pkg, p) {
        log.line(format!(
            "{pad}{name}: struct {} ({} values)",
            p.struct_name.map_or("?", |s| pkg.name(s)),
            inner.len()
        ));
        for q in &inner {
            dump(log, pkg, q, depth + 1);
        }
        return;
    }
    let value = match (&p.value, p.kind) {
        (Value::Object(i), _) => format!(
            "{} {}",
            object_class(pkg, *i),
            if *i == 0 {
                "None".into()
            } else {
                object_path(pkg, *i)
            }
        ),
        (Value::Name(i), _) => format!("name {}", pkg.name(*i)),
        (Value::Raw(_), PropertyType::Str | PropertyType::String) => {
            format!("{:?}", class::string_value(p).unwrap_or_default())
        }
        (Value::Raw(b), PropertyType::Struct) if b.len() == 4 => {
            let c = class::color_value(p).unwrap();
            format!(
                "struct {} bytes as colour rgba({}, {}, {}, {})",
                p.struct_name.map_or("?", |s| pkg.name(s)),
                c[0],
                c[1],
                c[2],
                c[3]
            )
        }
        (Value::Raw(b), _) => format!(
            "{:?} {} {} bytes {:02X?}",
            p.kind,
            p.struct_name.map_or("", |s| pkg.name(s)),
            b.len(),
            &b[..b.len().min(24)]
        ),
        (v, _) => format!("{v:?}"),
    };
    log.line(format!("{pad}{name} = {value}"));
}

pub(crate) fn fonts(log: &mut Log, target: &Path) -> bool {
    let data = match fs::read(target) {
        Ok(d) => d,
        Err(e) => {
            log.line(format!("FAILED: can't read {}: {e}", target.display()));
            return false;
        }
    };
    let pkg = match Package::parse(&data) {
        Ok(p) => p,
        Err(e) => {
            log.line(format!("FAILED: {e}"));
            return false;
        }
    };
    let (mut count, mut failed) = (0, 0);
    for e in &pkg.exports {
        if pkg.export_class_name(e) != "Font" {
            continue;
        }
        count += 1;
        let name = pkg.name(e.object_name);
        match font::read_font(&pkg, &data, e) {
            Ok(f) => {
                let used = f.glyphs.iter().filter(|g| g.width > 0).count();
                let a = f.glyph('A');
                log.line(format!(
                    "font {name}: {} letters ({used} drawn), {} pages [{}], kerning {}, line height {}, 'A' {:?}, {} bytes left",
                    f.glyphs.len(),
                    f.pages.len(),
                    f.pages
                        .iter()
                        .map(|p| object_path(&pkg, *p))
                        .collect::<Vec<_>>()
                        .join(", "),
                    f.kerning,
                    f.line_height(),
                    a,
                    f.leftover
                ));
            }
            Err(err) => {
                failed += 1;
                log.line(format!("font {name}: FAILED: {err}"));
            }
        }
    }
    log.line(format!("SUMMARY: {count} fonts ({failed} failed)"));
    failed == 0 && count > 0
}
