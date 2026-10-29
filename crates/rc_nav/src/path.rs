//! Shortest routes along the network (the A* search).
//!
//! A* explores nodes in order of "distance walked so far + straight-line
//! distance still to go", which finds the shortest route without looking at
//! the whole level. The cost of a link is the editor's stored `Distance`
//! (or the straight line if that is 0).

use crate::graph::{reach, NavGraph};
use rc_movement::Vec3;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// A route between two positions.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    /// Nodes visited, first to last.
    pub nodes: Vec<usize>,
    /// Points to walk to, in order: each node's location, then the goal.
    /// The starting position isn't included.
    pub points: Vec<Vec3>,
    /// Total length, Unreal units, from the start position to the goal.
    pub length: f32,
}

#[derive(PartialEq)]
struct Open {
    f: f32,
    node: usize,
}

impl Eq for Open {}

impl Ord for Open {
    fn cmp(&self, o: &Self) -> Ordering {
        // BinaryHeap is a max-heap; reverse for smallest f first.
        o.f.total_cmp(&self.f).then(o.node.cmp(&self.node))
    }
}

impl PartialOrd for Open {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl NavGraph {
    /// Shortest list of nodes from node `a` to node `b` using walkable
    /// links and no blocked nodes. `None` if `b` can't be reached.
    pub fn node_path(&self, a: usize, b: usize) -> Option<Vec<usize>> {
        self.node_path_avoiding(a, b, |_| 0.0)
    }

    /// Like `node_path`, with an extra cost for passing through a node
    /// (for example near an enemy's line of fire). `extra(node)` is added
    /// to every link that arrives at `node`.
    pub fn node_path_avoiding(
        &self,
        a: usize,
        b: usize,
        extra: impl Fn(usize) -> f32,
    ) -> Option<Vec<usize>> {
        let n = self.nodes.len();
        if a >= n || b >= n {
            return None;
        }
        let goal = self.nodes[b].location;
        let h = |i: usize| (self.nodes[i].location - goal).length();
        let mut g = vec![f32::INFINITY; n];
        let mut came = vec![usize::MAX; n];
        let mut heap = BinaryHeap::new();
        g[a] = 0.0;
        heap.push(Open { f: h(a), node: a });
        while let Some(Open { node, f }) = heap.pop() {
            if node == b {
                let mut out = vec![b];
                let mut c = b;
                while c != a {
                    c = came[c];
                    out.push(c);
                }
                out.reverse();
                return Some(out);
            }
            if f > g[node] + h(node) + 1e-3 {
                continue; // stale heap entry
            }
            for l in &self.links[node] {
                if !reach::walkable(l.flags) || self.blocked.contains(&l.to) {
                    continue;
                }
                let step = if l.distance > 0.0 {
                    l.distance
                } else {
                    (self.nodes[l.to].location - self.nodes[node].location).length()
                };
                let cand = g[node] + step + extra(l.to).max(0.0);
                if cand < g[l.to] {
                    g[l.to] = cand;
                    came[l.to] = node;
                    heap.push(Open {
                        f: cand + h(l.to),
                        node: l.to,
                    });
                }
            }
        }
        None
    }

    /// A route from `from` to `to`: walk to the node nearest `from`, along
    /// the network to the node nearest `to`, then straight to `to`.
    pub fn route(&self, from: Vec3, to: Vec3) -> Option<Route> {
        let a = self.nearest_node(from)?;
        let b = self.nearest_node(to)?;
        self.route_between(from, a, b, to)
    }

    /// The same, with the first and last node chosen by the caller (for
    /// example the nearest node the character can actually see).
    pub fn route_between(&self, from: Vec3, a: usize, b: usize, to: Vec3) -> Option<Route> {
        let nodes = self.node_path(a, b)?;
        let mut points: Vec<Vec3> = nodes.iter().map(|&i| self.nodes[i].location).collect();
        points.push(to);
        let mut length = 0.0;
        let mut prev = from;
        for &p in &points {
            length += (p - prev).length();
            prev = p;
        }
        Some(Route {
            nodes,
            points,
            length,
        })
    }

    /// Points to walk through from `from` to `to`, ending at `to` and not
    /// including `from`. The interface agreed with the squad thread.
    pub fn find_path(&self, from: Vec3, to: Vec3) -> Option<Vec<Vec3>> {
        self.route(from, to).map(|r| r.points)
    }
}
