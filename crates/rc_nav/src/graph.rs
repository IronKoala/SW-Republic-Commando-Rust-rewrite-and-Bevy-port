//! Reading the path network from a map.
//!
//! In Unreal Engine 2 maps, every "navigation point" actor (`PathNode`,
//! `CoverPoint`, `PlayerStart`, ...) has a `PathList`: references to
//! `ReachSpec` objects. Each `ReachSpec` is one one-way link: `Start` and
//! `End` (the two navigation points), `Distance` in Unreal units, and
//! `reachFlags` saying how it is travelled (walk, jump, fly...). They are
//! plain tagged property lists, so the shared property reader reads them.
//! The field names come from the game's own script source (Engine.u,
//! `ReachSpec`), read only; the flag meanings follow Unreal Engine 2's
//! public headers. Measured on geo_01a: 1,224 reach specs, all readable.

use rc_level::actors;
use rc_movement::Vec3;
use rc_package::properties::{self, read_properties, Value};
use rc_package::{ObjectRef, Package, Reader};
use std::collections::HashMap;

/// `reachFlags` bits (Unreal Engine 2's `R_*` values).
pub mod reach {
    pub const WALK: u32 = 1;
    pub const FLY: u32 = 2;
    pub const SWIM: u32 = 4;
    pub const JUMP: u32 = 8;
    pub const DOOR: u32 = 16;
    pub const SPECIAL: u32 = 32;
    pub const LADDER: u32 = 64;
    pub const PROSCRIBED: u32 = 128;
    pub const FORCED: u32 = 256;
    pub const PLAYER_ONLY: u32 = 512;
    /// What a walking character may use: plain walking links, or links
    /// with no flags at all (forced links the editor added by hand).
    pub fn walkable(flags: u32) -> bool {
        flags & (FLY | SWIM | LADDER | PROSCRIBED | PLAYER_ONLY | SPECIAL) == 0
    }
}

/// What a navigation point is for, with the extra numbers the AI uses.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    /// A plain point on the walking network.
    Path,
    /// A spot to hide behind something. The game stores how far a
    /// character can step out to each side to shoot (`StrafeDistLeft`,
    /// `StrafeDistRight`), and the distance to the thing it hides behind.
    /// `facing` is the actor's yaw: the direction the cover faces
    /// (towards the danger), radians.
    Cover {
        strafe_left: f32,
        strafe_right: f32,
        distance_to_cover: f32,
        facing: f32,
    },
    /// A good spot to fight from (`CombatPoint`). `radius` is how close
    /// counts as "there".
    Combat { radius: f32 },
    PlayerStart,
    /// Points only fliers use (`FlyPoint`).
    Fly,
    /// Anything else (`JumpDest`, `SquadMarkerSpot`, ...), by class name.
    Other(String),
}

#[derive(Debug, Clone)]
pub struct NavNode {
    pub name: String,
    pub class: String,
    /// The actor's `Tag`, which scripts use to name a point.
    pub tag: String,
    /// Centre of the node, Unreal units. Characters stand with their
    /// collision centre here (feet about one collision-height lower).
    pub location: Vec3,
    /// Export index (0-based) in the map.
    pub export: usize,
    pub kind: NodeKind,
}

impl NavNode {
    pub fn is_cover(&self) -> bool {
        matches!(self.kind, NodeKind::Cover { .. })
    }
}

/// One one-way link between two nodes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Link {
    pub to: usize,
    /// Unreal units (the editor's `Distance`).
    pub distance: f32,
    pub flags: u32,
}

/// The whole network: nodes, and for each node the links leaving it.
#[derive(Debug, Clone, Default)]
pub struct NavGraph {
    pub nodes: Vec<NavNode>,
    pub links: Vec<Vec<Link>>,
}

/// Counts from reading, for the log.
#[derive(Debug, Clone, Default)]
pub struct ReadStats {
    pub reach_specs: usize,
    pub unreadable: usize,
    /// Links whose end isn't a node we kept.
    pub dangling: usize,
    /// How many links had each flag value.
    pub flag_counts: Vec<(u32, usize)>,
}

fn raw_vec3(v: &Value) -> Option<Vec3> {
    match v {
        Value::Raw(b) if b.len() == 12 => {
            let f = |i: usize| f32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap());
            Some(Vec3::new(f(0), f(1), f(2)))
        }
        _ => None,
    }
}

fn float(pkg: &Package, props: &[properties::Property], name: &str) -> Option<f32> {
    match properties::find(pkg, props, name).map(|p| &p.value) {
        Some(Value::Float(f)) => Some(*f),
        Some(Value::Int(i)) => Some(*i as f32),
        _ => None,
    }
}

impl NavGraph {
    /// Read the network from a map package.
    pub fn from_map(pkg: &Package, data: &[u8]) -> Result<NavGraph, String> {
        Self::from_map_with_stats(pkg, data).map(|(g, _)| g)
    }

    pub fn from_map_with_stats(pkg: &Package, data: &[u8]) -> Result<(NavGraph, ReadStats), String> {
        let level = actors::read_level(pkg, data).map_err(|e| format!("level: {e}"))?;
        let mut stats = ReadStats::default();

        // 1. Every reach spec: (start export, end export, distance, flags).
        let mut specs = Vec::new();
        for e in pkg.exports.iter() {
            if pkg.export_class_name(e) != "ReachSpec" {
                continue;
            }
            stats.reach_specs += 1;
            let start = e.serial_offset as usize;
            let Some(bytes) = data.get(start..start + e.serial_size as usize) else {
                stats.unreadable += 1;
                continue;
            };
            let Ok(props) = read_properties(pkg, &mut Reader::new(bytes)) else {
                stats.unreadable += 1;
                continue;
            };
            let obj = |n: &str| match properties::find(pkg, &props, n).map(|p| &p.value) {
                Some(Value::Object(o)) => match ObjectRef::from_index(*o) {
                    ObjectRef::Export(k) => Some(k),
                    _ => None,
                },
                _ => None,
            };
            let int = |n: &str| match properties::find(pkg, &props, n).map(|p| &p.value) {
                Some(Value::Int(i)) => *i,
                _ => 0,
            };
            match (obj("Start"), obj("End")) {
                (Some(s), Some(t)) => specs.push((s, t, int("Distance") as f32, int("reachFlags") as u32)),
                _ => stats.unreadable += 1,
            }
        }

        // 2. Nodes: every level actor that has a PathList or is a link end.
        let mut wanted: std::collections::HashSet<usize> = specs.iter().flat_map(|s| [s.0, s.1]).collect();
        let mut nodes = Vec::new();
        let mut index_of = HashMap::new();
        for &a in level.actors.iter().flatten() {
            let Ok(props) = actors::read_actor_properties(pkg, data, a) else {
                continue;
            };
            let has_paths = properties::find(pkg, &props, "PathList").is_some();
            if !has_paths && !wanted.contains(&a) {
                continue;
            }
            wanted.remove(&a);
            let e = &pkg.exports[a];
            let class = pkg.export_class_name(e).to_string();
            let location = properties::find(pkg, &props, "Location")
                .and_then(|p| raw_vec3(&p.value))
                .unwrap_or(Vec3::ZERO);
            let yaw = match properties::find(pkg, &props, "Rotation").map(|p| &p.value) {
                Some(Value::Raw(b)) if b.len() == 12 => {
                    i32::from_le_bytes(b[4..8].try_into().unwrap()) as f32 / 65536.0 * std::f32::consts::TAU
                }
                _ => 0.0,
            };
            let tag = match properties::find(pkg, &props, "Tag").map(|p| &p.value) {
                Some(Value::Name(n)) => pkg.name(*n).to_string(),
                _ => class.clone(),
            };
            let kind = match class.as_str() {
                "PathNode" => NodeKind::Path,
                "CoverPoint" => NodeKind::Cover {
                    strafe_left: float(pkg, &props, "StrafeDistLeft").unwrap_or(0.0),
                    strafe_right: float(pkg, &props, "StrafeDistRight").unwrap_or(0.0),
                    distance_to_cover: float(pkg, &props, "DistanceToCover").unwrap_or(0.0),
                    facing: yaw,
                },
                "CombatPoint" => NodeKind::Combat {
                    radius: float(pkg, &props, "RadiusOverride")
                        .or(float(pkg, &props, "CollisionRadius"))
                        .unwrap_or(150.0),
                },
                "PlayerStart" => NodeKind::PlayerStart,
                "FlyPoint" => NodeKind::Fly,
                other => NodeKind::Other(other.to_string()),
            };
            index_of.insert(a, nodes.len());
            nodes.push(NavNode {
                name: pkg.name(e.object_name).to_string(),
                class,
                tag,
                location,
                export: a,
                kind,
            });
        }

        // 3. Links.
        let mut links = vec![Vec::new(); nodes.len()];
        let mut flag_counts: HashMap<u32, usize> = HashMap::new();
        for (s, t, distance, flags) in specs {
            match (index_of.get(&s), index_of.get(&t)) {
                (Some(&a), Some(&b)) => {
                    *flag_counts.entry(flags).or_default() += 1;
                    links[a].push(Link { to: b, distance, flags });
                }
                _ => stats.dangling += 1,
            }
        }
        let mut fc: Vec<(u32, usize)> = flag_counts.into_iter().collect();
        fc.sort();
        stats.flag_counts = fc;
        Ok((NavGraph { nodes, links }, stats))
    }

    pub fn link_count(&self) -> usize {
        self.links.iter().map(Vec::len).sum()
    }

    /// The node closest to `p` (straight-line distance), counting height
    /// differences double so a node on the floor above isn't chosen over
    /// one on this floor.
    pub fn nearest_node(&self, p: Vec3) -> Option<usize> {
        self.nearest_node_where(p, |_| true)
    }

    pub fn nearest_node_where(&self, p: Vec3, keep: impl Fn(&NavNode) -> bool) -> Option<usize> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| !matches!(n.kind, NodeKind::Fly) && keep(n))
            .map(|(i, n)| {
                let d = n.location - p;
                (i, d.length_2d().powi(2) + (2.0 * d.z).powi(2))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// Nodes within `radius` of `p` (straight-line), nearest first.
    pub fn nodes_near(&self, p: Vec3, radius: f32) -> Vec<usize> {
        let mut v: Vec<(usize, f32)> = self
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (i, (n.location - p).length()))
            .filter(|(_, d)| *d <= radius)
            .collect();
        v.sort_by(|a, b| a.1.total_cmp(&b.1));
        v.into_iter().map(|(i, _)| i).collect()
    }

    /// Find a node by its actor name (`PathNode3`) or tag.
    pub fn find(&self, name: &str) -> Option<usize> {
        self.nodes
            .iter()
            .position(|n| n.name.eq_ignore_ascii_case(name) || n.tag.eq_ignore_ascii_case(name))
    }

    /// How many separate walkable islands the network has, and the size of
    /// the biggest. A well-read map has one big island and a few small ones
    /// (fly paths, closed-off rooms).
    pub fn islands(&self) -> (usize, usize) {
        let n = self.nodes.len();
        // Treat links as two-way for this count.
        let mut adj = vec![Vec::new(); n];
        for (a, ls) in self.links.iter().enumerate() {
            for l in ls.iter().filter(|l| reach::walkable(l.flags)) {
                adj[a].push(l.to);
                adj[l.to].push(a);
            }
        }
        let mut seen = vec![false; n];
        let (mut count, mut biggest) = (0, 0);
        for s in 0..n {
            if seen[s] {
                continue;
            }
            count += 1;
            let mut size = 0;
            let mut stack = vec![s];
            seen[s] = true;
            while let Some(a) = stack.pop() {
                size += 1;
                for &b in &adj[a] {
                    if !seen[b] {
                        seen[b] = true;
                        stack.push(b);
                    }
                }
            }
            biggest = biggest.max(size);
        }
        (count, biggest)
    }
}
