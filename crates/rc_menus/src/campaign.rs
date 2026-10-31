//! The original campaign page (`XInterfaceCTMenus.CTCampaignMenu`), what
//! its script does on a PC, rewritten by hand from reading it (never
//! copied).
//!
//! The page shows three planets: Geonosis, the assault ship (the
//! Prosecutor) and Kashyyyk. Choosing one lights it, its ring and its
//! mission blips, shows its name, and fills the list with its missions.
//! Choosing a mission and SELECT opens that mission's first map. Left and
//! right (or the small arrows between the planets) move between planets.
//!
//! Everything shown comes from the class's own values: the planet names
//! (`Campaigns`), each planet's missions (`GEO_SubMissions` and the names in
//! `GEO_SubMissionDescs`, and the same for `RAS_` and `YYY_`), and which
//! blip belongs to which missions (`GEO_MissionSubEnd`...). The class also
//! has a developer list with every map (`DEV_GEO_SubMissions`...), which
//! the original only shows in a debug mode; we offer it as "every map".
//!
//! The original only lists missions the player's profile has reached. This
//! port doesn't keep that progress yet, so every mission is listed.

use crate::layout::{Kind, MenuLayout};
use crate::library::ClassData;
use crate::lists;
use crate::value::V;

/// The class of the original campaign page.
pub const CAMPAIGN_MENU: &str = "XInterfaceCTMenus.CTCampaignMenu";

/// The list on the page.
pub const LEVEL_LIST: &str = "LevelList";

/// One planet: its widgets and missions.
#[derive(Debug, Clone, PartialEq)]
pub struct Planet {
    /// The planet button (`Geonosis`, `AssaultShip`, `Kashyyyk`).
    pub button: &'static str,
    /// Its blips (`CompletedGeonosis`...).
    pub blips: &'static str,
    /// The arrows shown while it's chosen.
    pub arrows: &'static [&'static str],
    /// Its name, shown beside "CAMPAIGN MISSIONS".
    pub title: String,
    /// The first map of each mission, and the mission's name.
    pub missions: Vec<(String, String)>,
    /// Every map, for the "every map" list.
    pub every_map: Vec<(String, String)>,
    /// The last mission (index) each blip stands for, in order.
    pub blip_ends: Vec<usize>,
    /// The first mission the blips count from (1 on Geonosis: the
    /// prologue has no blip).
    pub blip_start: usize,
}

/// The page's state.
#[derive(Debug, Clone, PartialEq)]
pub struct Campaign {
    pub planets: Vec<Planet>,
    /// The planet chosen now.
    pub chosen: usize,
    /// Show every map instead of the missions.
    pub every_map: bool,
}

const PLANETS: [(&str, &str, &str, &[&str]); 3] = [
    (
        "GEO",
        "Geonosis",
        "CompletedGeonosis",
        &["GEONavigateRight"],
    ),
    (
        "RAS",
        "AssaultShip",
        "CompletedAssaultShip",
        &["RASNavigateLeft", "RASNavigateRight"],
    ),
    ("YYY", "Kashyyyk", "CompletedKashyyyk", &["YYYNavigateLeft"]),
];

/// Every element of a fixed array of strings.
fn strings(class: &ClassData, name: &str) -> Vec<String> {
    (0..class.array_size(name))
        .map(|i| match class.get_at(name, i) {
            Some(V::Str(s)) => s.clone(),
            Some(V::Name(s)) => s.clone(),
            _ => String::new(),
        })
        .collect()
}

fn ints(class: &ClassData, name: &str) -> Vec<i32> {
    (0..class.array_size(name))
        .map(|i| match class.get_at(name, i) {
            Some(V::Int(n)) => *n,
            _ => 0,
        })
        .collect()
}

fn pairs(maps: Vec<String>, names: Vec<String>) -> Vec<(String, String)> {
    maps.into_iter()
        .zip(names)
        .filter(|(m, _)| !m.is_empty())
        .collect()
}

/// Read the planets from the class. `None` if it has none of them.
pub fn read(class: &ClassData) -> Option<Campaign> {
    let titles = strings(class, "Campaigns");
    let mut planets = Vec::new();
    for (i, (prefix, button, blips, arrows)) in PLANETS.iter().enumerate() {
        let missions = pairs(
            strings(class, &format!("{prefix}_SubMissions")),
            strings(class, &format!("{prefix}_SubMissionDescs")),
        );
        if missions.is_empty() {
            continue;
        }
        let blip_ends = ints(class, &format!("{prefix}_MissionSubEnd"))
            .into_iter()
            .map(|n| n.max(0) as usize)
            .collect();
        planets.push(Planet {
            button,
            blips,
            arrows,
            title: titles.get(i).cloned().unwrap_or_default(),
            missions,
            every_map: pairs(
                strings(class, &format!("DEV_{prefix}_SubMissions")),
                strings(class, &format!("DEV_{prefix}_SubMissionDescs")),
            ),
            blip_ends,
            blip_start: usize::from(*prefix == "GEO"),
        });
    }
    (!planets.is_empty()).then_some(Campaign {
        planets,
        chosen: 0,
        every_map: false,
    })
}

impl Campaign {
    /// The list shown for planet `p`: (map, name).
    pub fn entries(&self, p: usize) -> &[(String, String)] {
        let planet = &self.planets[p];
        if self.every_map && !planet.every_map.is_empty() {
            &planet.every_map
        } else {
            &planet.missions
        }
    }

    /// The map to open for item `item` of the chosen planet's list.
    pub fn map(&self, item: usize) -> Option<&str> {
        self.entries(self.chosen).get(item).map(|(m, _)| m.as_str())
    }

    /// The planet left (-1) or right (1) of the chosen one, if any.
    pub fn neighbour(&self, step: i32) -> Option<usize> {
        let n = self.chosen as i32 + step;
        (0..self.planets.len() as i32)
            .contains(&n)
            .then_some(n as usize)
    }

    /// The planet whose button is called `on_select` (`GeonosisSelected`).
    pub fn planet_for(&self, on_select: &str) -> Option<usize> {
        self.planets
            .iter()
            .position(|p| on_select.eq_ignore_ascii_case(&format!("{}Selected", p.button)))
    }
}

fn set_rgba(c: &mut [u8; 4], v: [u8; 4]) {
    *c = v;
}

/// What the page's script does when it opens on a PC: the planets and
/// arrows are for the mouse (keys and controllers move along the list and
/// use left and right for planets), and the list rows don't take focus
/// from a still pointer.
pub fn setup(layout: &mut MenuLayout, c: &Campaign) {
    for p in &c.planets {
        for name in std::iter::once(p.button).chain(p.arrows.iter().copied()) {
            if let Some(Kind::Button(b)) = layout.widget_mut(name).map(|w| &mut w.kind) {
                b.mouse_only = true;
            }
        }
    }
}

/// Choose planet `p`: dim the others (`UnSelectCampaign`), light this one
/// and its blips, show its name and arrows, and fill the list.
pub fn choose(layout: &mut MenuLayout, c: &mut Campaign, p: usize) {
    let p = p.min(c.planets.len() - 1);
    c.chosen = p;
    for (i, planet) in c.planets.iter().enumerate() {
        let lit = i == p;
        let (rgb, alpha) = if lit { (255, 255) } else { (96, 192) };
        if let Some(Kind::Button(b)) = layout.widget_mut(planet.button).map(|w| &mut w.kind) {
            set_rgba(&mut b.under_blurred.color, [rgb, rgb, rgb, 255]);
            set_rgba(&mut b.under_focused.color, [rgb, rgb, rgb, 255]);
            set_rgba(&mut b.background_blurred.color, [rgb, rgb, rgb, alpha]);
            // The chosen planet can't be chosen again; the others light up
            // under the pointer.
            b.background_focused.color = [255, 255, 255, 255];
            b.disabled = lit;
        }
        for k in 0.. {
            let Some(w) = layout.widget_mut(&format!("{}[{k}]", planet.blips)) else {
                break;
            };
            w.hidden = false;
            if let Kind::Sprite(s) = &mut w.kind {
                s.color = [rgb, rgb, rgb, alpha];
            }
        }
        for a in planet.arrows {
            if let Some(w) = layout.widget_mut(a) {
                w.hidden = !lit;
            }
        }
    }
    let title = c.planets[p].title.clone();
    if let Some(Kind::Text(t)) = layout.widget_mut("Campaign").map(|w| &mut w.kind) {
        t.text = title;
    }
    let items: Vec<String> = c.entries(p).iter().map(|(_, n)| n.clone()).collect();
    if let Some(l) = layout.lists.lists.iter().position(|l| l.name == LEVEL_LIST) {
        lists::set_items(layout, l, items);
    }
}

/// Light the blip of the mission at `item` green and the others white
/// (`HighlightMissionBlip`). Only for the missions list; the every-map
/// list has its own numbering.
pub fn highlight(layout: &mut MenuLayout, c: &Campaign, item: Option<usize>) {
    let planet = &c.planets[c.chosen];
    let mut start = planet.blip_start;
    for (k, &end) in planet.blip_ends.iter().enumerate() {
        let on = !c.every_map && item.is_some_and(|i| i >= start && i <= end);
        if let Some(Kind::Sprite(s)) = layout
            .widget_mut(&format!("{}[{k}]", planet.blips))
            .map(|w| &mut w.kind)
        {
            s.color = if on {
                [128, 255, 128, 255]
            } else {
                [255, 255, 255, 255]
            };
        }
        start = end + 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn campaign() -> Campaign {
        let planet = |button, blips, missions: &[&str], ends: Vec<usize>, start| Planet {
            button,
            blips,
            arrows: &[],
            title: button.to_uppercase(),
            missions: missions
                .iter()
                .map(|m| (m.to_string(), format!("NAME {m}")))
                .collect(),
            every_map: vec![],
            blip_ends: ends,
            blip_start: start,
        };
        Campaign {
            planets: vec![
                planet(
                    "Geonosis",
                    "CompletedGeonosis",
                    &[
                        "PRO",
                        "GEO_01Briefing",
                        "GEO_03A",
                        "GEO_03D",
                        "GEO_04A",
                        "GEO_05A",
                    ],
                    vec![1, 3, 4, 5],
                    1,
                ),
                planet(
                    "AssaultShip",
                    "CompletedAssaultShip",
                    &["RAS_01Briefing"],
                    vec![0],
                    0,
                ),
            ],
            chosen: 0,
            every_map: false,
        }
    }

    #[test]
    fn planets_and_maps() {
        let mut c = campaign();
        assert_eq!(c.planet_for("assaultshipselected"), Some(1));
        assert_eq!(c.neighbour(-1), None);
        assert_eq!(c.neighbour(1), Some(1));
        assert_eq!(c.map(2), Some("GEO_03A"));
        c.chosen = 1;
        assert_eq!(c.map(0), Some("RAS_01Briefing"));
        assert_eq!(c.map(1), None);
    }

    #[test]
    fn blips_follow_the_mission_ranges() {
        use crate::layout::{Sounds, Sprite, Widget};
        let mut l = MenuLayout {
            class: "T".into(),
            widgets: (0..4)
                .map(|k| Widget {
                    name: format!("CompletedGeonosis[{k}]"),
                    pass: 0,
                    hidden: false,
                    platform: 0,
                    kind: Kind::Sprite(Sprite::default()),
                })
                .collect(),
            skipped: vec![],
            sounds: Sounds::default(),
            background_movie: String::new(),
            music: None,
            lists: Default::default(),
        };
        let c = campaign();
        let green = |l: &MenuLayout| -> Vec<bool> {
            l.widgets
                .iter()
                .map(|w| matches!(&w.kind, Kind::Sprite(s) if s.color == [128, 255, 128, 255]))
                .collect()
        };
        // The prologue (item 0) has no blip; items 2 and 3 share blip 1.
        highlight(&mut l, &c, Some(0));
        assert_eq!(green(&l), [false, false, false, false]);
        highlight(&mut l, &c, Some(1));
        assert_eq!(green(&l), [true, false, false, false]);
        highlight(&mut l, &c, Some(3));
        assert_eq!(green(&l), [false, true, false, false]);
        highlight(&mut l, &c, Some(5));
        assert_eq!(green(&l), [false, false, false, true]);
    }
}
