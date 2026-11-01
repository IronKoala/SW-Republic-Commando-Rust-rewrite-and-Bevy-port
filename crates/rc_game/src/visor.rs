//! Plugs the visor (rc_visor) into the game: the GAME page's TACTICAL
//! VISOR MODE and TACTICAL MODE INTENSITY settings go to it, and squad
//! mates and pickups are marked to glow in tactical mode.

use crate::settings::Settings;
use bevy::prelude::*;
use rc_combat::pickup::WeaponPickup;
use rc_squad_plugin::SquadMate;
use rc_visor::{TacticalGlow, VisorPlugin, VisorSettings};

pub struct GameVisorPlugin;

impl Plugin for GameVisorPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(VisorPlugin)
            .add_systems(Update, (copy_settings, mark_glowing));
    }
}

fn copy_settings(settings: Res<Settings>, mut visor: ResMut<VisorSettings>) {
    let want = VisorSettings {
        choice: settings.game.tactical_visor.choice(),
        intensity: settings.game.tactical_intensity.clamp(0.0, 1.0),
    };
    if *visor != want {
        *visor = want;
    }
}

/// Squad mates glow in their own colour (by name: "sev", "fixer",
/// "scorch"), weapons on the ground in the pickups' colour.
fn mark_glowing(
    mut commands: Commands,
    mates: Query<(Entity, &SquadMate), Without<TacticalGlow>>,
    pickups: Query<Entity, (With<WeaponPickup>, Without<TacticalGlow>)>,
) {
    for (e, m) in &mates {
        commands
            .entity(e)
            .insert(TacticalGlow(m.name.to_ascii_lowercase()));
    }
    for e in &pickups {
        commands.entity(e).insert(TacticalGlow("pickup".into()));
    }
}
