//! Poses a skeleton from an animation and moves a mesh's points with it.
//!
//! - A **skeleton** is a tree of bones. Each bone has a position and a
//!   rotation relative to its parent. Walking down the tree from the root
//!   gives each bone's place in the whole model ("global" transform).
//! - The mesh was modelled in a **reference pose**. An animation gives each
//!   bone a new local position and rotation per frame.
//! - **Skinning** moves each point of the mesh with the bones that influence
//!   it: undo the bone's reference placement, apply its animated placement,
//!   and mix the results by the influence weights.
//!
//! Everything here is in the game's own units and axes (Unreal: Z is up).
//! The Bevy viewer converts at the very end.
//!
//! The rotation convention follows UE Viewer (github.com/gildor2/UEViewer,
//! MIT, Konstantin Nosov), `MeshInstance/SkelMeshInstance.cpp` and the SWRC
//! branches of `UnMesh2.cpp` / `UnAnim2.cpp`, written from that description:
//! Republic Commando stores bone and key quaternions mirrored compared with
//! other Unreal Engine 2 games, and Unreal Engine 2 itself applies them
//! inverted except on the root bone. The two flips cancel, so a stored
//! quaternion is used as-is for every bone except the root, whose
//! quaternion is inverted. `Convention` lets the probe try the other
//! choices and measure which one keeps the mesh in one piece.

pub mod math;

use math::{length, lerp, sub, Quat, Transform, Vec3};
use rc_package::animation::{MeshAnimation, Sequence};
use rc_package::skeletal::SkeletalMesh;

/// How to turn a stored quaternion into a rotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Convention {
    /// As stored, except the root bone which is inverted. What UE Viewer does
    /// for Republic Commando, and the default.
    InvertRoot,
    /// Every bone as stored.
    AsStored,
    /// Every bone inverted.
    InvertAll,
    /// Every bone inverted except the root (the usual Unreal Engine 2 rule
    /// without Republic Commando's extra mirror).
    InvertNonRoot,
}

impl Convention {
    pub const ALL: [Convention; 4] = [
        Convention::InvertRoot,
        Convention::AsStored,
        Convention::InvertAll,
        Convention::InvertNonRoot,
    ];

    pub fn rotation(self, stored: [f32; 4], is_root: bool) -> Quat {
        let q = Quat(stored).normalize();
        let invert = match self {
            Convention::InvertRoot => is_root,
            Convention::AsStored => false,
            Convention::InvertAll => true,
            Convention::InvertNonRoot => !is_root,
        };
        if invert {
            q.conjugate()
        } else {
            q
        }
    }
}

/// The mesh's bones in the reference pose.
#[derive(Debug, Clone)]
pub struct Skeleton {
    pub names: Vec<String>,
    pub parents: Vec<usize>,
    /// Each bone relative to its parent.
    pub local: Vec<Transform>,
    /// Each bone in the whole model.
    pub global: Vec<Transform>,
    /// The opposite of `global`: takes a model point into the bone's own space.
    pub inverse_global: Vec<Transform>,
    pub convention: Convention,
}

impl Skeleton {
    pub fn from_mesh(mesh: &SkeletalMesh, convention: Convention) -> Skeleton {
        let local: Vec<Transform> = mesh
            .bones
            .iter()
            .enumerate()
            .map(|(i, b)| Transform {
                rot: convention.rotation(b.orientation, i == 0),
                pos: b.position,
            })
            .collect();
        let parents: Vec<usize> = mesh
            .bones
            .iter()
            .map(|b| b.parent.max(0) as usize)
            .collect();
        let global = globals(&parents, &local);
        let inverse_global = global.iter().map(|t| t.inverse()).collect();
        Skeleton {
            names: mesh.bones.iter().map(|b| b.name.clone()).collect(),
            parents,
            local,
            global,
            inverse_global,
            convention,
        }
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n.eq_ignore_ascii_case(name))
    }
}

/// Walk down the tree: parents always come before their children.
pub fn globals(parents: &[usize], local: &[Transform]) -> Vec<Transform> {
    let mut out: Vec<Transform> = Vec::with_capacity(local.len());
    for (i, l) in local.iter().enumerate() {
        let g = if i == 0 || parents[i] >= i {
            *l
        } else {
            out[parents[i]].then(l)
        };
        out.push(g);
    }
    out
}

/// One bone's keys, unpacked.
#[derive(Debug, Clone)]
struct Keys {
    times: Vec<f32>,
    positions: Vec<Vec3>,
    rotations: Vec<Quat>,
}

/// One animation sequence, ready to sample at any frame.
#[derive(Debug, Clone)]
pub struct Clip {
    pub name: String,
    pub num_frames: f32,
    /// Frames per second.
    pub rate: f32,
    /// One entry per mesh bone; `None` means the animation doesn't move
    /// that bone, so it keeps its reference pose.
    tracks: Vec<Option<Keys>>,
    /// How many mesh bones found a track by name.
    pub matched_bones: usize,
}

impl Clip {
    pub fn new(anim: &MeshAnimation, seq: &Sequence, skeleton: &Skeleton) -> Clip {
        let mut tracks: Vec<Option<Keys>> = vec![None; skeleton.names.len()];
        let mut matched = 0;
        for (ai, bone) in anim.bones.iter().enumerate() {
            let (Some(mi), Some(t)) = (skeleton.find(&bone.name), seq.tracks.get(ai)) else {
                continue;
            };
            matched += 1;
            tracks[mi] = Some(Keys {
                times: t.key_times(),
                positions: t.key_positions(),
                rotations: t
                    .key_rotations(anim.new_rotation_format)
                    .into_iter()
                    .map(|q| skeleton.convention.rotation(q, mi == 0))
                    .collect(),
            });
        }
        Clip {
            name: seq.name.clone(),
            num_frames: seq.num_frames.max(1) as f32,
            rate: seq.rate,
            tracks,
            matched_bones: matched,
        }
    }

    /// Length in seconds.
    pub fn duration(&self) -> f32 {
        if self.rate > 0.0 {
            self.num_frames / self.rate
        } else {
            0.0
        }
    }

    /// Each bone's local transform at `frame` (0 to `num_frames`, looping).
    pub fn sample(&self, skeleton: &Skeleton, frame: f32) -> Vec<Transform> {
        let frame = frame.rem_euclid(self.num_frames);
        self.tracks
            .iter()
            .zip(&skeleton.local)
            .map(|(track, rest)| match track {
                None => *rest,
                Some(k) => {
                    let (a, b, t) = key_pair(&k.times, frame, self.num_frames);
                    let pos = match k.positions.len() {
                        0 => rest.pos,
                        1 => k.positions[0],
                        n => lerp(k.positions[a.min(n - 1)], k.positions[b.min(n - 1)], t),
                    };
                    let rot = match k.rotations.len() {
                        0 => rest.rot,
                        1 => k.rotations[0],
                        n => k.rotations[a.min(n - 1)].slerp(k.rotations[b.min(n - 1)], t),
                    };
                    Transform { rot, pos }
                }
            })
            .collect()
    }
}

/// Which two keys surround `frame`, and how far between them (0 to 1).
/// After the last key the animation blends back to the first (looping).
fn key_pair(times: &[f32], frame: f32, num_frames: f32) -> (usize, usize, f32) {
    let n = times.len();
    if n <= 1 {
        return (0, 0, 0.0);
    }
    let next = times.iter().position(|&t| t > frame).unwrap_or(n);
    if next == 0 {
        return (0, 0, 0.0);
    }
    let a = next - 1;
    let (b, end) = if next < n {
        (next, times[next])
    } else {
        (0, num_frames.max(times[a]))
    };
    let span = end - times[a];
    let t = if span > 0.0 {
        (frame - times[a]) / span
    } else {
        0.0
    };
    (a, b, t.clamp(0.0, 1.0))
}

/// Mix two poses (each bone's local transform), `t` = 0 is all `a`, 1 is
/// all `b`. Positions are blended in a straight line and rotations the
/// short way round.
pub fn blend_poses(a: &[Transform], b: &[Transform], t: f32) -> Vec<Transform> {
    let t = t.clamp(0.0, 1.0);
    a.iter()
        .zip(b)
        .map(|(x, y)| Transform {
            rot: x.rot.slerp(y.rot, t).normalize(),
            pos: lerp(x.pos, y.pos, t),
        })
        .collect()
}

/// Smooth switching from one animation to the next ("cross-fade").
///
/// Without it a character jumps from the last frame shown to the first
/// frame of the new animation in one step. With it, when the animation
/// changes the pose on screen is remembered, and over the next
/// `duration` seconds the new animation is mixed in on top of it, slow at
/// first and last ("ease in and out"). Switching again halfway through
/// starts from whatever mix was on screen, so there is never a jump.
///
/// The original engine does this inside its compiled code; the times used
/// are ours, set per character in its data file.
#[derive(Debug, Clone, Default)]
pub struct Crossfade {
    /// The pose shown last.
    shown: Vec<Transform>,
    /// The pose shown when the change started.
    from: Vec<Transform>,
    elapsed: f32,
    duration: f32,
}

impl Crossfade {
    /// The animation just changed: blend into it over `duration` seconds
    /// (0 switches at once).
    pub fn start(&mut self, duration: f32) {
        if duration > 0.0 && !self.shown.is_empty() {
            self.from = self.shown.clone();
            self.elapsed = 0.0;
            self.duration = duration;
        } else {
            self.duration = 0.0;
        }
    }

    pub fn advance(&mut self, dt: f32) {
        self.elapsed += dt.max(0.0);
    }

    /// How much of the new animation shows, 0 to 1 (1 when not blending).
    pub fn weight(&self) -> f32 {
        if self.duration <= 0.0 {
            return 1.0;
        }
        let x = (self.elapsed / self.duration).clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    }

    /// Still mixing the old pose in.
    pub fn blending(&self) -> bool {
        self.duration > 0.0 && self.elapsed < self.duration
    }

    /// The pose to show, given the new animation's pose this frame.
    pub fn apply(&mut self, target: Vec<Transform>) -> &[Transform] {
        self.shown = if self.blending() && self.from.len() == target.len() {
            blend_poses(&self.from, &target, self.weight())
        } else {
            self.duration = 0.0;
            target
        };
        &self.shown
    }
}

/// Per point, the bones that move it and how much (weights add up to 1).
pub fn weights_per_point(mesh: &SkeletalMesh) -> Vec<Vec<(usize, f32)>> {
    let mut out = vec![Vec::new(); mesh.points.len()];
    for inf in &mesh.influences {
        if let Some(list) = out.get_mut(inf.point as usize) {
            list.push((inf.bone as usize, inf.weight));
        }
    }
    for list in &mut out {
        let sum: f32 = list.iter().map(|w| w.1).sum();
        if sum > 0.0 {
            for w in list.iter_mut() {
                w.1 /= sum;
            }
        } else {
            list.push((0, 1.0));
        }
    }
    out
}

/// Move the mesh's points with posed bones (`pose_global` from `globals`).
pub fn skin(
    points: &[Vec3],
    weights: &[Vec<(usize, f32)>],
    skeleton: &Skeleton,
    pose_global: &[Transform],
) -> Vec<Vec3> {
    // For each bone: from the reference pose to the animated pose.
    let moves: Vec<Transform> = pose_global
        .iter()
        .zip(&skeleton.inverse_global)
        .map(|(g, inv)| g.then(inv))
        .collect();
    points
        .iter()
        .zip(weights)
        .map(|(p, ws)| {
            let mut out = [0f32; 3];
            for &(b, w) in ws {
                let q = moves[b].apply(*p);
                for k in 0..3 {
                    out[k] += q[k] * w;
                }
            }
            out
        })
        .collect()
}

/// Every distinct edge of the mesh's triangles, as pairs of point numbers.
pub fn edges(mesh: &SkeletalMesh) -> Vec<(usize, usize)> {
    let mut e: Vec<(usize, usize)> = Vec::new();
    for t in &mesh.triangles {
        let p = t
            .wedges
            .map(|w| mesh.wedges.get(w as usize).map_or(0, |w| w.point as usize));
        for (a, b) in [(p[0], p[1]), (p[1], p[2]), (p[2], p[0])] {
            if a != b {
                e.push((a.min(b), a.max(b)));
            }
        }
    }
    e.sort_unstable();
    e.dedup();
    e
}

/// How much the mesh's edges changed length compared with the reference
/// pose. A correct animation bends joints but barely stretches the surface;
/// a wrong rotation convention tears limbs apart and stretches edges a lot.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stretch {
    /// Average of |new length / old length - 1| over all edges.
    pub mean: f32,
    /// Share of edges that changed length by more than 50%.
    pub over_half: f32,
    /// Largest new / old length.
    pub max_ratio: f32,
}

pub fn stretch(rest: &[Vec3], posed: &[Vec3], edges: &[(usize, usize)]) -> Stretch {
    let mut s = Stretch::default();
    let mut counted = 0usize;
    let mut over = 0usize;
    for &(a, b) in edges {
        let l0 = length(sub(rest[a], rest[b]));
        if l0 < 1e-4 {
            continue;
        }
        let r = length(sub(posed[a], posed[b])) / l0;
        s.mean += (r - 1.0).abs();
        s.max_ratio = s.max_ratio.max(r);
        if (r - 1.0).abs() > 0.5 {
            over += 1;
        }
        counted += 1;
    }
    if counted > 0 {
        s.mean /= counted as f32;
        s.over_half = over as f32 / counted as f32;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_pair_walks_and_loops() {
        let times = [0.0, 2.0, 5.0];
        assert_eq!(key_pair(&times, 0.0, 6.0), (0, 1, 0.0));
        assert_eq!(key_pair(&times, 1.0, 6.0), (0, 1, 0.5));
        assert_eq!(key_pair(&times, 2.0, 6.0), (1, 2, 0.0));
        // After the last key: blend back to the first over the last frame.
        assert_eq!(key_pair(&times, 5.5, 6.0), (2, 0, 0.5));
        assert_eq!(key_pair(&[0.0], 3.0, 6.0), (0, 0, 0.0));
    }

    #[test]
    fn crossfade_moves_smoothly() {
        let at = |x: f32| {
            vec![Transform {
                rot: Quat::IDENTITY,
                pos: [x, 0.0, 0.0],
            }]
        };
        let mut c = Crossfade::default();
        // Nothing shown yet: the first pose shows at once.
        c.start(0.2);
        assert_eq!(c.apply(at(0.0))[0].pos[0], 0.0);
        // Switch to a pose 10 units away over 0.2 s, 0.05 s a step.
        c.start(0.2);
        let mut shown = vec![c.apply(at(10.0))[0].pos[0]];
        for _ in 0..5 {
            c.advance(0.05);
            shown.push(c.apply(at(10.0))[0].pos[0]);
        }
        assert_eq!(shown[0], 0.0);
        assert!((shown[2] - 5.0).abs() < 1e-4, "halfway is half: {shown:?}");
        assert_eq!(shown[4], 10.0);
        assert!(!c.blending());
        let steps: Vec<f32> = shown.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(steps.iter().all(|d| (0.0..4.0).contains(d)), "{steps:?}");
        // Switch back halfway through another change: starts from what was
        // on screen, not from either animation.
        c.start(0.2);
        c.advance(0.1);
        let mid = c.apply(at(0.0))[0].pos[0];
        c.start(0.2);
        assert_eq!(c.apply(at(10.0))[0].pos[0], mid);
        // Zero time switches at once.
        c.start(0.0);
        assert_eq!(c.apply(at(3.0))[0].pos[0], 3.0);
    }

    #[test]
    fn two_bone_chain() {
        // Root at the origin, child 10 units along x. Turning the root a
        // quarter turn about z moves the child to 10 units along y.
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let parents = [0, 0];
        let local = [
            Transform {
                rot: Quat([0.0, 0.0, h, h]),
                pos: [0.0; 3],
            },
            Transform {
                rot: Quat::IDENTITY,
                pos: [10.0, 0.0, 0.0],
            },
        ];
        let g = globals(&parents, &local);
        assert!(length(sub(g[1].pos, [0.0, 10.0, 0.0])) < 1e-4);
    }
}
