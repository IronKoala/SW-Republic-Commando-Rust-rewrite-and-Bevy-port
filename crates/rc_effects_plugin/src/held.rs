//! The weapon a character holds in its hand: the weapon class's
//! `AttachmentClass` and that class's `StaticMesh`, read from the game's
//! files, hung on the character's weapon bone as the original's
//! `AttachToBone` does. Shared by the level's cast (`rc_cast`) and the
//! fighters the map's spawners make (`rc_enemies`).

use bevy::prelude::*;
use rc_level_viewer::materials::{Materials, Picture};

/// The bone a character's weapon hangs on (engine.u's
/// `Pawn.GetWeaponBoneFor` returns it for every weapon).
pub const WEAPON_BONE: &str = "weaponAttach_R";

/// The model a character shows for the weapon it holds.
pub struct HeldModel {
    pub info: rc_effects::projectile::HeldWeapon,
    pub sections: Vec<(Mesh, Option<Picture>)>,
}

impl HeldModel {
    /// Read the model for weapon class `weapon` (like `DC17mBlaster`).
    pub fn load(
        classes: &rc_effects::projectile::ClassPackages,
        game_data: &std::path::Path,
        weapon: &str,
    ) -> Result<HeldModel, String> {
        let info = classes.held_weapon(weapon)?;
        let mut mats = Materials::new(game_data);
        let sections = crate::static_mesh(&mut mats, &info.mesh)?;
        Ok(HeldModel { info, sections })
    }

    /// One line for the log about what was read.
    pub fn describe(&self) -> String {
        let mut s = format!(
            "held weapon {}: {} with model {} ({} parts), on bone {WEAPON_BONE}",
            self.info.weapon,
            self.info.attachment,
            self.info.mesh.join("."),
            self.sections.len()
        );
        if self.info.relative_location != [0.0; 3] || self.info.relative_rotation != [0; 3] {
            s.push_str(&format!(
                "; its RelativeLocation/Rotation {:?} {:?} aren't applied yet",
                self.info.relative_location, self.info.relative_rotation
            ));
        }
        s
    }

    /// Where the model goes for a weapon bone at `frame` (the bone's origin
    /// and its X, Y and Z directions in Bevy units, in the body's own
    /// space). The static mesh's points are Unreal's X, Z, Y (see
    /// [`crate::static_mesh`]), in Unreal units.
    pub fn on_bone(&self, frame: [[f32; 3]; 4]) -> Transform {
        let [o, x, y, z] = frame;
        let s = self.info.draw_scale;
        let col = |v: [f32; 3]| Vec4::new(v[0] * s, v[1] * s, v[2] * s, 0.0);
        let m = Mat4::from_cols(col(x), col(z), col(y), Vec4::new(o[0], o[1], o[2], 1.0));
        Transform::from_matrix(m)
    }

    /// Spawn the model as a child of `parent`, placed at `at` (hidden when
    /// `None`), lit by where it stands like its holder: `ambient` of its
    /// own colour, and the level's light there times `light_scale`.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        &self,
        parent: &mut ChildSpawnerCommands,
        at: Option<Transform>,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
        images: &mut Assets<Image>,
        ambient: f32,
        light_scale: f32,
    ) -> Entity {
        let a = ambient.max(0.0);
        let mut w = parent.spawn((at.unwrap_or_default(), Visibility::default()));
        w.with_children(|g| {
            for (mesh, pic) in &self.sections {
                let texture = pic
                    .as_ref()
                    .map(|p| images.add(rc_level_viewer::draw::to_image(p)));
                let material = materials.add(StandardMaterial {
                    base_color: Color::WHITE,
                    emissive: LinearRgba::rgb(a, a, a),
                    emissive_texture: texture.clone(),
                    base_color_texture: texture,
                    perceptual_roughness: 0.7,
                    // The mesh is mirrored into Bevy's axes.
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                });
                g.spawn((
                    Mesh3d(meshes.add(mesh.clone())),
                    MeshMaterial3d(material),
                    rc_game_core::LitByPlace {
                        target: rc_game_core::LightTarget::Emissive,
                        own: 0.0,
                        scale: light_scale,
                        height: 1.0,
                        made_at: a.powf(1.0 / 2.2),
                    },
                ));
            }
        });
        if at.is_none() {
            w.insert(Visibility::Hidden);
        }
        w.id()
    }
}
